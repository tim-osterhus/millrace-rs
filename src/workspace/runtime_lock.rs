//! Workspace-scoped runtime daemon ownership lock helpers.

use std::{
    env, fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process,
};

use serde::Serialize;
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::WorkspacePaths;

/// Result type for runtime ownership lock operations.
pub type RuntimeOwnershipLockResult<T> = Result<T, RuntimeOwnershipLockError>;

/// Current workspace runtime ownership lock state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOwnershipLockState {
    /// No runtime ownership lock file exists.
    Absent,
    /// The lock file belongs to a live process.
    Active,
    /// The lock file is well-formed but its owner process is not live.
    Stale,
    /// The lock file is malformed or references a different workspace root.
    Invalid,
}

impl RuntimeOwnershipLockState {
    /// Returns the canonical serialized state label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Active => "active",
            Self::Stale => "stale",
            Self::Invalid => "invalid",
        }
    }
}

impl fmt::Display for RuntimeOwnershipLockState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Structured ownership metadata persisted in the runtime lock file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeOwnershipRecord {
    /// Workspace root the owner believes it owns.
    pub workspace_root: String,
    /// Owning process id.
    pub owner_pid: u32,
    /// Owning host name.
    pub owner_hostname: String,
    /// Owning runtime session id.
    pub owner_session_id: String,
    /// RFC 3339 timestamp for lock acquisition.
    pub acquired_at: String,
}

impl RuntimeOwnershipRecord {
    /// Build a validated runtime ownership record.
    pub fn new(
        workspace_root: impl Into<String>,
        owner_pid: u32,
        owner_hostname: impl Into<String>,
        owner_session_id: impl Into<String>,
        acquired_at: impl Into<String>,
    ) -> RuntimeOwnershipLockResult<Self> {
        let record = Self {
            workspace_root: workspace_root.into(),
            owner_pid,
            owner_hostname: owner_hostname.into(),
            owner_session_id: owner_session_id.into(),
            acquired_at: acquired_at.into(),
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> RuntimeOwnershipLockResult<()> {
        validate_non_empty_string("workspace_root", &self.workspace_root)?;
        if self.owner_pid == 0 {
            return Err(RuntimeOwnershipLockError::InvalidRecord {
                field_name: "owner_pid",
                message: "must be a positive integer".to_owned(),
            });
        }
        validate_non_empty_string("owner_hostname", &self.owner_hostname)?;
        validate_non_empty_string("owner_session_id", &self.owner_session_id)?;
        validate_acquired_at(&self.acquired_at)?;
        Ok(())
    }
}

/// Inputs used to acquire a deterministic runtime ownership lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOwnershipLockOptions {
    /// Owning process id to persist.
    pub owner_pid: u32,
    /// Owning host name to persist.
    pub owner_hostname: String,
    /// Owning runtime session id to persist.
    pub owner_session_id: String,
    /// RFC 3339 timestamp to persist.
    pub acquired_at: String,
}

impl RuntimeOwnershipLockOptions {
    /// Build validated lock acquisition inputs.
    pub fn new(
        owner_pid: u32,
        owner_hostname: impl Into<String>,
        owner_session_id: impl Into<String>,
        acquired_at: impl Into<String>,
    ) -> RuntimeOwnershipLockResult<Self> {
        let options = Self {
            owner_pid,
            owner_hostname: owner_hostname.into(),
            owner_session_id: owner_session_id.into(),
            acquired_at: acquired_at.into(),
        };
        options.validate()?;
        Ok(options)
    }

    /// Build lock acquisition inputs from the current process and host.
    #[must_use]
    pub fn current() -> Self {
        let acquired_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());
        let session_seed = OffsetDateTime::now_utc().unix_timestamp_nanos();
        Self {
            owner_pid: process::id(),
            owner_hostname: current_hostname(),
            owner_session_id: format!("session-{}-{session_seed}", process::id()),
            acquired_at,
        }
    }

    fn validate(&self) -> RuntimeOwnershipLockResult<()> {
        if self.owner_pid == 0 {
            return Err(RuntimeOwnershipLockError::InvalidRecord {
                field_name: "owner_pid",
                message: "must be a positive integer".to_owned(),
            });
        }
        validate_non_empty_string("owner_hostname", &self.owner_hostname)?;
        validate_non_empty_string("owner_session_id", &self.owner_session_id)?;
        validate_acquired_at(&self.acquired_at)
    }

    fn into_record(
        self,
        paths: &WorkspacePaths,
    ) -> RuntimeOwnershipLockResult<RuntimeOwnershipRecord> {
        RuntimeOwnershipRecord::new(
            path_string(&paths.root),
            self.owner_pid,
            self.owner_hostname,
            self.owner_session_id,
            self.acquired_at,
        )
    }
}

/// Current lock status with optional parsed ownership context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOwnershipLockStatus {
    /// Classified lock state.
    pub state: RuntimeOwnershipLockState,
    /// Canonical lock file path.
    pub lock_path: PathBuf,
    /// Parsed ownership record when the payload was structurally valid.
    pub record: Option<RuntimeOwnershipRecord>,
    /// Human-readable detail suitable for CLI and doctor surfaces.
    pub detail: String,
}

/// Result from clearing stale or invalid runtime ownership locks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearRuntimeOwnershipLockResult {
    /// True when the lock file was removed by this call.
    pub cleared: bool,
    /// Deterministic reason code.
    pub reason: String,
    /// Status observed before the clear attempt.
    pub status: RuntimeOwnershipLockStatus,
}

/// Failures produced while acquiring or mutating runtime ownership locks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeOwnershipLockError {
    /// A filesystem operation failed.
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying IO error message.
        message: String,
    },
    /// Lock payload serialization failed.
    JsonRender {
        /// Serde error message.
        message: String,
    },
    /// Caller-provided or parsed record content was invalid.
    InvalidRecord {
        /// Field involved in the failure.
        field_name: &'static str,
        /// Human-readable validation message.
        message: String,
    },
    /// Exclusive acquisition failed because a lock file already exists.
    AlreadyHeld {
        /// Status observed after the failed exclusive create.
        status: RuntimeOwnershipLockStatus,
    },
}

impl RuntimeOwnershipLockError {
    fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: error.to_string(),
        }
    }

    /// Returns the lock status for exclusive acquisition failures.
    #[must_use]
    pub fn status(&self) -> Option<&RuntimeOwnershipLockStatus> {
        match self {
            Self::AlreadyHeld { status } => Some(status),
            Self::Io { .. } | Self::JsonRender { .. } | Self::InvalidRecord { .. } => None,
        }
    }
}

impl fmt::Display for RuntimeOwnershipLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => {
                write!(
                    f,
                    "runtime ownership lock filesystem error at {}: {message}",
                    path.display()
                )
            }
            Self::JsonRender { message } => {
                write!(f, "failed to render runtime ownership lock: {message}")
            }
            Self::InvalidRecord {
                field_name,
                message,
            } => {
                write!(f, "{field_name} is invalid: {message}")
            }
            Self::AlreadyHeld { status } => f.write_str(&ownership_error_message(status)),
        }
    }
}

impl std::error::Error for RuntimeOwnershipLockError {}

/// Inspect lock metadata and classify active/stale/invalid states.
#[must_use]
pub fn inspect_runtime_ownership_lock(paths: &WorkspacePaths) -> RuntimeOwnershipLockStatus {
    inspect_runtime_ownership_lock_with_pid_checker(paths, default_pid_is_running)
}

/// Inspect lock metadata using an injected PID liveness checker.
#[must_use]
pub fn inspect_runtime_ownership_lock_with_pid_checker<F>(
    paths: &WorkspacePaths,
    pid_is_running: F,
) -> RuntimeOwnershipLockStatus
where
    F: Fn(u32) -> bool,
{
    let lock_path = paths.runtime_lock_file.clone();
    if !lock_path.exists() {
        return RuntimeOwnershipLockStatus {
            state: RuntimeOwnershipLockState::Absent,
            lock_path,
            record: None,
            detail: "runtime ownership lock is absent".to_owned(),
        };
    }

    let raw = match fs::read_to_string(&lock_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return RuntimeOwnershipLockStatus {
                state: RuntimeOwnershipLockState::Absent,
                lock_path,
                record: None,
                detail: "runtime ownership lock is absent".to_owned(),
            };
        }
        Err(error) => {
            return invalid_status(
                lock_path,
                None,
                format!("invalid runtime ownership lock payload: {error}"),
            );
        }
    };

    let record = match parse_lock_payload(&raw) {
        Ok(record) => record,
        Err(error) => {
            return invalid_status(
                lock_path,
                None,
                format!("invalid runtime ownership lock payload: {error}"),
            );
        }
    };

    let workspace_root = path_string(&paths.root);
    if record.workspace_root != workspace_root {
        return invalid_status(
            lock_path,
            Some(record.clone()),
            format!(
                "runtime ownership lock references a different workspace root ({})",
                record.workspace_root
            ),
        );
    }

    if pid_is_running(record.owner_pid) {
        return RuntimeOwnershipLockStatus {
            state: RuntimeOwnershipLockState::Active,
            lock_path,
            detail: format!(
                "workspace runtime ownership lock is active: pid={} host={} session={}",
                record.owner_pid, record.owner_hostname, record.owner_session_id
            ),
            record: Some(record),
        };
    }

    RuntimeOwnershipLockStatus {
        state: RuntimeOwnershipLockState::Stale,
        lock_path,
        detail: format!(
            "workspace runtime ownership lock is stale: pid={} is not running (session={})",
            record.owner_pid, record.owner_session_id
        ),
        record: Some(record),
    }
}

/// Acquire exclusive runtime ownership for one workspace.
pub fn acquire_runtime_ownership_lock(
    paths: &WorkspacePaths,
) -> RuntimeOwnershipLockResult<RuntimeOwnershipRecord> {
    acquire_runtime_ownership_lock_with_options(paths, RuntimeOwnershipLockOptions::current())
}

/// Acquire exclusive runtime ownership using deterministic caller-provided inputs.
pub fn acquire_runtime_ownership_lock_with_options(
    paths: &WorkspacePaths,
    options: RuntimeOwnershipLockOptions,
) -> RuntimeOwnershipLockResult<RuntimeOwnershipRecord> {
    let record = options.into_record(paths)?;
    let payload = serialize_record(&record)?;
    let lock_path = &paths.runtime_lock_file;
    let parent = lock_path
        .parent()
        .ok_or_else(|| RuntimeOwnershipLockError::InvalidRecord {
            field_name: "lock_path",
            message: "runtime lock path must have a parent directory".to_owned(),
        })?;
    fs::create_dir_all(parent).map_err(|error| RuntimeOwnershipLockError::io(parent, error))?;

    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(RuntimeOwnershipLockError::AlreadyHeld {
                status: inspect_runtime_ownership_lock(paths),
            });
        }
        Err(error) => return Err(RuntimeOwnershipLockError::io(lock_path, error)),
    };

    let write_result = (|| {
        file.write_all(payload.as_bytes())?;
        file.flush()?;
        file.sync_all()
    })();

    match write_result {
        Ok(()) => Ok(record),
        Err(error) => {
            let _ = fs::remove_file(lock_path);
            Err(RuntimeOwnershipLockError::io(lock_path, error))
        }
    }
}

/// Release workspace runtime ownership if caller owns it or force is enabled.
pub fn release_runtime_ownership_lock(
    paths: &WorkspacePaths,
    owner_session_id: Option<&str>,
    force: bool,
) -> RuntimeOwnershipLockResult<bool> {
    let lock_path = &paths.runtime_lock_file;
    if !lock_path.exists() {
        return Ok(false);
    }

    if force {
        return remove_lock_file(lock_path);
    }

    let status = inspect_runtime_ownership_lock(paths);
    let Some(record) = status.record.as_ref() else {
        return Ok(false);
    };
    if status.state == RuntimeOwnershipLockState::Invalid {
        return Ok(false);
    }
    if owner_session_id.is_some_and(|session_id| record.owner_session_id != session_id) {
        return Ok(false);
    }

    remove_lock_file(lock_path)
}

/// Remove stale/invalid lock files, preserving active daemon ownership.
pub fn clear_stale_runtime_ownership_lock(
    paths: &WorkspacePaths,
) -> RuntimeOwnershipLockResult<ClearRuntimeOwnershipLockResult> {
    clear_stale_runtime_ownership_lock_with_pid_checker(paths, default_pid_is_running)
}

/// Remove stale/invalid lock files using an injected PID liveness checker.
pub fn clear_stale_runtime_ownership_lock_with_pid_checker<F>(
    paths: &WorkspacePaths,
    pid_is_running: F,
) -> RuntimeOwnershipLockResult<ClearRuntimeOwnershipLockResult>
where
    F: Fn(u32) -> bool,
{
    let status = inspect_runtime_ownership_lock_with_pid_checker(paths, pid_is_running);

    match status.state {
        RuntimeOwnershipLockState::Absent => Ok(ClearRuntimeOwnershipLockResult {
            cleared: false,
            reason: "missing".to_owned(),
            status,
        }),
        RuntimeOwnershipLockState::Active => Ok(ClearRuntimeOwnershipLockResult {
            cleared: false,
            reason: "active_owner".to_owned(),
            status,
        }),
        RuntimeOwnershipLockState::Stale | RuntimeOwnershipLockState::Invalid => {
            let reason = if status.state == RuntimeOwnershipLockState::Stale {
                "cleared_stale"
            } else {
                "cleared_invalid"
            }
            .to_owned();
            let cleared = remove_lock_file(&status.lock_path)?;
            Ok(ClearRuntimeOwnershipLockResult {
                cleared,
                reason,
                status,
            })
        }
    }
}

fn invalid_status(
    lock_path: PathBuf,
    record: Option<RuntimeOwnershipRecord>,
    detail: String,
) -> RuntimeOwnershipLockStatus {
    RuntimeOwnershipLockStatus {
        state: RuntimeOwnershipLockState::Invalid,
        lock_path,
        record,
        detail,
    }
}

fn parse_lock_payload(payload: &str) -> Result<RuntimeOwnershipRecord, String> {
    let parsed: Value = serde_json::from_str(payload).map_err(|error| error.to_string())?;
    let object = parsed
        .as_object()
        .ok_or_else(|| "expected top-level object".to_owned())?;
    let workspace_root = required_string(object, "workspace_root")?;
    let owner_pid = required_positive_u32(object, "owner_pid")?;
    let owner_hostname = required_string(object, "owner_hostname")?;
    let owner_session_id = required_string(object, "owner_session_id")?;
    let acquired_at = required_string(object, "acquired_at")?;
    validate_acquired_at_for_parse(&acquired_at)?;

    Ok(RuntimeOwnershipRecord {
        workspace_root,
        owner_pid,
        owner_hostname,
        owner_session_id,
        acquired_at,
    })
}

fn required_string(
    object: &serde_json::Map<String, Value>,
    field_name: &'static str,
) -> Result<String, String> {
    let value = object
        .get(field_name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field_name} must be a non-empty string"))?;
    if value.trim().is_empty() {
        return Err(format!("{field_name} must be a non-empty string"));
    }
    Ok(value.to_owned())
}

fn required_positive_u32(
    object: &serde_json::Map<String, Value>,
    field_name: &'static str,
) -> Result<u32, String> {
    let Some(value) = object.get(field_name).and_then(Value::as_i64) else {
        return Err(format!("{field_name} must be a positive integer"));
    };
    if value <= 0 || value > u32::MAX as i64 {
        return Err(format!("{field_name} must be a positive integer"));
    }
    Ok(value as u32)
}

fn serialize_record(record: &RuntimeOwnershipRecord) -> RuntimeOwnershipLockResult<String> {
    let mut payload = serde_json::to_string_pretty(record).map_err(|error| {
        RuntimeOwnershipLockError::JsonRender {
            message: error.to_string(),
        }
    })?;
    payload.push('\n');
    Ok(payload)
}

fn remove_lock_file(lock_path: &Path) -> RuntimeOwnershipLockResult<bool> {
    match fs::remove_file(lock_path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(RuntimeOwnershipLockError::io(lock_path, error)),
    }
}

fn validate_non_empty_string(
    field_name: &'static str,
    value: &str,
) -> RuntimeOwnershipLockResult<()> {
    if value.trim().is_empty() {
        return Err(RuntimeOwnershipLockError::InvalidRecord {
            field_name,
            message: "must be a non-empty string".to_owned(),
        });
    }
    Ok(())
}

fn validate_acquired_at(value: &str) -> RuntimeOwnershipLockResult<()> {
    if value.trim().is_empty() {
        return Err(RuntimeOwnershipLockError::InvalidRecord {
            field_name: "acquired_at",
            message: "must be an ISO datetime string".to_owned(),
        });
    }
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| {
        RuntimeOwnershipLockError::InvalidRecord {
            field_name: "acquired_at",
            message: "must be an ISO datetime string".to_owned(),
        }
    })?;
    Ok(())
}

fn validate_acquired_at_for_parse(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("acquired_at must be an ISO datetime string".to_owned());
    }
    OffsetDateTime::parse(value, &Rfc3339)
        .map(|_| ())
        .map_err(|_| "acquired_at must be an ISO datetime string".to_owned())
}

fn ownership_error_message(status: &RuntimeOwnershipLockStatus) -> String {
    match status.state {
        RuntimeOwnershipLockState::Stale => {
            format!(
                "{}; run clear-stale-state to remove stale ownership before starting runtime again",
                status.detail
            )
        }
        RuntimeOwnershipLockState::Invalid => {
            format!(
                "{}; run clear-stale-state to repair ownership lock before starting runtime again",
                status.detail
            )
        }
        RuntimeOwnershipLockState::Active => {
            if let Some(record) = &status.record {
                format!(
                    "workspace runtime ownership lock is already held by pid={} host={} session={}",
                    record.owner_pid, record.owner_hostname, record.owner_session_id
                )
            } else {
                "workspace runtime ownership lock is already held".to_owned()
            }
        }
        RuntimeOwnershipLockState::Absent => {
            "workspace runtime ownership lock is already held".to_owned()
        }
    }
}

fn current_hostname() -> String {
    env::var("HOSTNAME")
        .or_else(|_| env::var("COMPUTERNAME"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown-host".to_owned())
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn default_pid_is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    if pid == process::id() {
        return true;
    }
    platform_pid_is_running(pid)
}

#[cfg(unix)]
fn platform_pid_is_running(pid: u32) -> bool {
    if pid > i32::MAX as u32 {
        return false;
    }
    // SAFETY: `libc::kill` is called with signal 0, which performs an OS liveness
    // check without sending a signal. The pid value is range-checked above.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }
    matches!(
        io::Error::last_os_error().raw_os_error(),
        Some(code) if code == libc::EPERM
    )
}

#[cfg(not(unix))]
fn platform_pid_is_running(_pid: u32) -> bool {
    false
}
