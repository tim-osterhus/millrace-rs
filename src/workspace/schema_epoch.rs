//! Workspace schema epoch markers and archive-reset helpers.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    process,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    compiler::WORKSPACE_SCHEMA_EPOCH_ID,
    contracts::{RuntimeJsonContract, Timestamp},
};

use super::{
    RuntimeOwnershipLockState, StateStoreError, WorkspaceError, WorkspacePaths,
    initialize_workspace_paths, inspect_runtime_ownership_lock, state_store::atomic_write_text,
    workspace_paths,
};

/// Current workspace schema epoch accepted by this runtime.
pub const CURRENT_WORKSPACE_SCHEMA_EPOCH: &str = WORKSPACE_SCHEMA_EPOCH_ID;

const MARKER_FILENAME: &str = "workspace_schema_epoch.json";
const RESET_LOCK_FILENAME: &str = ".schema-reset.lock";
const MUTABLE_RUNTIME_NAMES: &[&str] = &[
    "state",
    "runs",
    "tasks",
    "specs",
    "incidents",
    "probes",
    "recon",
    "learning",
    "arbiter",
];

/// Result type for schema epoch operations.
pub type SchemaEpochResult<T> = Result<T, SchemaEpochError>;

/// Workspace schema epoch marker persisted under runtime state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSchemaEpochMarker {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_marker_kind")]
    pub kind: String,
    pub epoch_id: String,
    pub written_at: Timestamp,
}

impl RuntimeJsonContract for WorkspaceSchemaEpochMarker {
    const ARTIFACT: &'static str = "workspace_schema_epoch_marker";

    fn validate_contract(&mut self) -> Result<(), crate::contracts::RuntimeJsonError> {
        crate::contracts::validate_safe_identifier(&self.epoch_id, "epoch_id")
            .map_err(crate::contracts::RuntimeJsonError::Contract)?;
        Ok(())
    }
}

/// Options for archive-resetting mutable workspace state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaArchiveResetOptions {
    /// Human-readable reason persisted in the archive manifest.
    pub reason: String,
    /// Deterministic reset timestamp for tests; defaults to current UTC time.
    pub now: Option<Timestamp>,
}

impl SchemaArchiveResetOptions {
    /// Build reset options with the required reason.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            now: None,
        }
    }

    /// Attach a deterministic timestamp.
    #[must_use]
    pub fn with_now(mut self, now: Timestamp) -> Self {
        self.now = Some(now);
        self
    }
}

/// Outcome from archiving old mutable runtime state and reinitializing the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaArchiveResetResult {
    pub epoch_id: String,
    pub archive_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub moved_paths: Vec<String>,
}

/// Failures produced by workspace schema epoch helpers.
#[derive(Debug)]
pub enum SchemaEpochError {
    /// Filesystem operation failed.
    Io { path: PathBuf, message: String },
    /// JSON syntax or rendering failed.
    Json { path: PathBuf, message: String },
    /// Marker payload was absent or invalid.
    InvalidMarker { path: PathBuf, message: String },
    /// Marker epoch is incompatible with this runtime.
    Incompatible {
        path: PathBuf,
        found: String,
        expected: String,
    },
    /// Archive reset was refused because a daemon owns the workspace.
    RuntimeOwned { detail: String },
    /// Archive reset lock already exists.
    ResetLockExists { path: PathBuf },
    /// Workspace initialization or state persistence failed.
    Workspace(WorkspaceError),
    /// State persistence failed.
    StateStore(StateStoreError),
}

impl SchemaEpochError {
    fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for SchemaEpochError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => {
                write!(
                    f,
                    "workspace schema epoch filesystem error at {}: {message}",
                    path.display()
                )
            }
            Self::Json { path, message } => {
                write!(
                    f,
                    "workspace schema epoch JSON error at {}: {message}",
                    path.display()
                )
            }
            Self::InvalidMarker { path, message } => {
                write!(
                    f,
                    "workspace schema epoch marker is invalid at {}: {message}",
                    path.display()
                )
            }
            Self::Incompatible {
                path,
                found,
                expected,
            } => {
                write!(
                    f,
                    "workspace schema epoch {found} is incompatible at {}; expected {expected}",
                    path.display()
                )
            }
            Self::RuntimeOwned { detail } => {
                write!(f, "workspace has an active daemon owner: {detail}")
            }
            Self::ResetLockExists { path } => {
                write!(
                    f,
                    "workspace schema reset lock already exists: {}",
                    path.display()
                )
            }
            Self::Workspace(error) => write!(f, "{error}"),
            Self::StateStore(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SchemaEpochError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::StateStore(error) => Some(error),
            Self::Io { .. }
            | Self::Json { .. }
            | Self::InvalidMarker { .. }
            | Self::Incompatible { .. }
            | Self::RuntimeOwned { .. }
            | Self::ResetLockExists { .. } => None,
        }
    }
}

impl From<WorkspaceError> for SchemaEpochError {
    fn from(value: WorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

impl From<StateStoreError> for SchemaEpochError {
    fn from(value: StateStoreError) -> Self {
        Self::StateStore(value)
    }
}

/// Return the marker path for a resolved workspace.
#[must_use]
pub fn workspace_schema_epoch_marker_path(paths: &WorkspacePaths) -> PathBuf {
    paths.state_dir.join(MARKER_FILENAME)
}

/// Load and validate the workspace schema epoch marker.
pub fn load_workspace_schema_epoch_marker(
    paths: &WorkspacePaths,
) -> SchemaEpochResult<WorkspaceSchemaEpochMarker> {
    let path = workspace_schema_epoch_marker_path(paths);
    let raw = fs::read_to_string(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            SchemaEpochError::InvalidMarker {
                path: path.clone(),
                message: "workspace schema epoch marker is missing".to_owned(),
            }
        } else {
            SchemaEpochError::io(&path, error)
        }
    })?;
    let value: Value = serde_json::from_str(&raw).map_err(|error| SchemaEpochError::Json {
        path: path.clone(),
        message: error.to_string(),
    })?;
    let mut marker = WorkspaceSchemaEpochMarker::from_json_value(value).map_err(|error| {
        SchemaEpochError::InvalidMarker {
            path: path.clone(),
            message: error.to_string(),
        }
    })?;
    marker
        .validate_contract()
        .map_err(|error| SchemaEpochError::InvalidMarker {
            path,
            message: error.to_string(),
        })?;
    Ok(marker)
}

/// Ensure the workspace marker is compatible with this runtime before state JSON is parsed.
pub fn ensure_workspace_schema_epoch_current(
    paths: &WorkspacePaths,
) -> SchemaEpochResult<WorkspaceSchemaEpochMarker> {
    let marker = load_workspace_schema_epoch_marker(paths)?;
    if marker.epoch_id != CURRENT_WORKSPACE_SCHEMA_EPOCH {
        return Err(SchemaEpochError::Incompatible {
            path: workspace_schema_epoch_marker_path(paths),
            found: marker.epoch_id,
            expected: CURRENT_WORKSPACE_SCHEMA_EPOCH.to_owned(),
        });
    }
    Ok(marker)
}

/// Persist a schema epoch marker.
pub fn write_workspace_schema_epoch_marker(
    paths: &WorkspacePaths,
    epoch_id: Option<&str>,
    now: Option<Timestamp>,
) -> SchemaEpochResult<PathBuf> {
    let path = workspace_schema_epoch_marker_path(paths);
    let marker = WorkspaceSchemaEpochMarker {
        schema_version: default_schema_version(),
        kind: default_marker_kind(),
        epoch_id: epoch_id
            .unwrap_or(CURRENT_WORKSPACE_SCHEMA_EPOCH)
            .to_owned(),
        written_at: now.unwrap_or_else(now_timestamp),
    };
    let mut payload =
        serde_json::to_string_pretty(&marker).map_err(|error| SchemaEpochError::Json {
            path: path.clone(),
            message: error.to_string(),
        })?;
    payload.push('\n');
    atomic_write_text(&path, &payload)?;
    Ok(path)
}

/// Render the default marker payload used by workspace initialization.
pub fn default_workspace_schema_epoch_marker_payload() -> Result<String, WorkspaceError> {
    let marker = WorkspaceSchemaEpochMarker {
        schema_version: default_schema_version(),
        kind: default_marker_kind(),
        epoch_id: CURRENT_WORKSPACE_SCHEMA_EPOCH.to_owned(),
        written_at: now_timestamp(),
    };
    serde_json::to_string_pretty(&marker)
        .map(|mut rendered| {
            rendered.push('\n');
            rendered
        })
        .map_err(|error| WorkspaceError::Json {
            artifact: "workspace_schema_epoch_marker",
            message: error.to_string(),
        })
}

/// Archive mutable runtime state and reinitialize a clean workspace baseline.
pub fn archive_reset_workspace_schema(
    root: impl AsRef<Path>,
    reason: &str,
) -> SchemaEpochResult<SchemaArchiveResetResult> {
    let paths = workspace_paths(root);
    archive_reset_workspace_schema_with_options(&paths, SchemaArchiveResetOptions::new(reason))
}

/// Archive mutable runtime state using already-resolved paths and explicit options.
pub fn archive_reset_workspace_schema_with_options(
    paths: &WorkspacePaths,
    options: SchemaArchiveResetOptions,
) -> SchemaEpochResult<SchemaArchiveResetResult> {
    let reason = options.reason.trim();
    if reason.is_empty() {
        return Err(SchemaEpochError::InvalidMarker {
            path: workspace_schema_epoch_marker_path(paths),
            message: "schema reset reason is required".to_owned(),
        });
    }
    let now = options.now.unwrap_or_else(now_timestamp);

    let daemon_status = inspect_runtime_ownership_lock(paths);
    if daemon_status.state == RuntimeOwnershipLockState::Active {
        return Err(SchemaEpochError::RuntimeOwned {
            detail: daemon_status.detail,
        });
    }

    let reset_lock = acquire_reset_lock(paths)?;
    let result = (|| {
        let archive_dir = allocate_archive_dir(paths, &now)?;
        let moved_paths = archive_mutable_runtime_state(paths, &archive_dir)?;
        initialize_workspace_paths(paths)?;
        write_workspace_schema_epoch_marker(paths, None, Some(now.clone()))?;
        let manifest_path = write_archive_manifest(&archive_dir, reason, &moved_paths, &now)?;
        Ok(SchemaArchiveResetResult {
            epoch_id: CURRENT_WORKSPACE_SCHEMA_EPOCH.to_owned(),
            archive_dir,
            manifest_path,
            moved_paths,
        })
    })();

    let _ = fs::remove_file(&reset_lock);
    result
}

fn archive_mutable_runtime_state(
    paths: &WorkspacePaths,
    archive_dir: &Path,
) -> SchemaEpochResult<Vec<String>> {
    let mut moved = Vec::new();
    for name in MUTABLE_RUNTIME_NAMES {
        let source = paths.runtime_root.join(name);
        if !source.exists() {
            continue;
        }
        let destination = archive_dir.join(name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| SchemaEpochError::io(parent, error))?;
        }
        fs::rename(&source, &destination).map_err(|error| SchemaEpochError::io(&source, error))?;
        collect_files_relative_to(archive_dir, &destination, &mut moved)?;
    }
    moved.sort();
    Ok(moved)
}

fn collect_files_relative_to(
    root: &Path,
    path: &Path,
    output: &mut Vec<String>,
) -> SchemaEpochResult<()> {
    if path.is_file() {
        output.push(
            path.strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/"),
        );
        return Ok(());
    }
    if !path.is_dir() {
        return Ok(());
    }
    let mut entries = fs::read_dir(path)
        .map_err(|error| SchemaEpochError::io(path, error))?
        .collect::<Result<Vec<_>, io::Error>>()
        .map_err(|error| SchemaEpochError::io(path, error))?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        collect_files_relative_to(root, &entry.path(), output)?;
    }
    Ok(())
}

fn write_archive_manifest(
    archive_dir: &Path,
    reason: &str,
    moved_paths: &[String],
    now: &Timestamp,
) -> SchemaEpochResult<PathBuf> {
    let path = archive_dir.join("schema_archive_manifest.json");
    let payload = json!({
        "schema_version": "1.0",
        "kind": "schema_archive_manifest",
        "epoch_id": CURRENT_WORKSPACE_SCHEMA_EPOCH,
        "reason": reason,
        "archived_at": now.as_str(),
        "moved_paths": moved_paths,
    });
    let mut rendered =
        serde_json::to_string_pretty(&payload).map_err(|error| SchemaEpochError::Json {
            path: path.clone(),
            message: error.to_string(),
        })?;
    rendered.push('\n');
    atomic_write_text(&path, &rendered)?;
    Ok(path)
}

fn allocate_archive_dir(paths: &WorkspacePaths, now: &Timestamp) -> SchemaEpochResult<PathBuf> {
    let archives_root = paths.runtime_root.join("archives");
    fs::create_dir_all(&archives_root)
        .map_err(|error| SchemaEpochError::io(&archives_root, error))?;
    let base_name = format!("schema-reset-{}", archive_timestamp_fragment(now));
    let mut candidate = archives_root.join(&base_name);
    let mut suffix = 1_u64;
    while candidate.exists() {
        candidate = archives_root.join(format!("{base_name}-{suffix}"));
        suffix += 1;
    }
    fs::create_dir(&candidate).map_err(|error| SchemaEpochError::io(&candidate, error))?;
    Ok(candidate)
}

fn acquire_reset_lock(paths: &WorkspacePaths) -> SchemaEpochResult<PathBuf> {
    let lock_path = paths.runtime_root.join(RESET_LOCK_FILENAME);
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).map_err(|error| SchemaEpochError::io(parent, error))?;
    }
    let payload = json!({ "owner_pid": process::id() }).to_string() + "\n";
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(SchemaEpochError::ResetLockExists { path: lock_path });
        }
        Err(error) => return Err(SchemaEpochError::io(&lock_path, error)),
    };
    use std::io::Write;
    file.write_all(payload.as_bytes())
        .map_err(|error| SchemaEpochError::io(&lock_path, error))?;
    file.flush()
        .map_err(|error| SchemaEpochError::io(&lock_path, error))?;
    Ok(lock_path)
}

fn archive_timestamp_fragment(timestamp: &Timestamp) -> String {
    timestamp
        .as_str()
        .chars()
        .filter(|value| value.is_ascii_alphanumeric())
        .collect()
}

fn now_timestamp() -> Timestamp {
    let rendered = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());
    Timestamp::parse("written_at", &rendered).unwrap_or_else(|_| {
        Timestamp::parse("written_at", "1970-01-01T00:00:00Z").expect("epoch timestamp is valid")
    })
}

fn default_schema_version() -> String {
    "1.0".to_owned()
}

fn default_marker_kind() -> String {
    "workspace_schema_epoch_marker".to_owned()
}
