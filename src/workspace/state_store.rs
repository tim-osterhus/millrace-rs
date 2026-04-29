//! Runtime state and status-file persistence helpers.

use std::{
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Serialize;
use serde_json::Value;

use crate::contracts::{
    RecoveryCounterEntry, RecoveryCounters, RuntimeJsonContract, RuntimeJsonError, RuntimeSnapshot,
    SubscriptionQuotaStatus, Timestamp, UsageGovernanceLedgerEntry, UsageGovernanceState,
    WorkItemKind,
};

use super::{WorkspaceError, WorkspacePaths, initialize_workspace};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Result type for runtime state store operations.
pub type StateStoreResult<T> = Result<T, StateStoreError>;

/// Runtime state store failures.
#[derive(Debug)]
pub enum StateStoreError {
    /// Workspace initialization or path handling failed.
    Workspace(WorkspaceError),
    /// A filesystem operation failed.
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Underlying IO error message.
        message: String,
    },
    /// JSON syntax failed before typed contract validation.
    JsonSyntax {
        /// Path being decoded.
        path: PathBuf,
        /// Serde error message.
        message: String,
    },
    /// JSONL syntax failed on a specific line.
    JsonLineSyntax {
        /// Path being decoded.
        path: PathBuf,
        /// One-based line number.
        line_number: usize,
        /// Serde error message.
        message: String,
    },
    /// JSON serialization failed before persistence.
    JsonRender {
        /// Path being encoded.
        path: PathBuf,
        /// Serde error message.
        message: String,
    },
    /// Runtime JSON artifact was valid JSON but violated the typed contract.
    RuntimeJson {
        /// Path being decoded or encoded.
        path: PathBuf,
        /// Typed runtime JSON contract error.
        source: RuntimeJsonError,
    },
    /// A JSONL line was valid JSON but violated a typed contract.
    RuntimeJsonLine {
        /// Path being decoded.
        path: PathBuf,
        /// One-based line number.
        line_number: usize,
        /// Typed runtime JSON contract error.
        source: RuntimeJsonError,
    },
    /// JSON state file did not contain an object payload.
    NonObjectPayload {
        /// Path being decoded.
        path: PathBuf,
    },
    /// Status marker shape was invalid.
    StatusMarker {
        /// Human-readable status marker error.
        message: String,
    },
    /// A state path could not be addressed safely.
    InvalidPath {
        /// Path value involved in the failure.
        path: PathBuf,
        /// Validation error message.
        message: String,
    },
}

impl StateStoreError {
    fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: error.to_string(),
        }
    }

    fn runtime_json(path: impl Into<PathBuf>, source: RuntimeJsonError) -> Self {
        Self::RuntimeJson {
            path: path.into(),
            source,
        }
    }

    fn status_marker(message: impl Into<String>) -> Self {
        Self::StatusMarker {
            message: message.into(),
        }
    }
}

impl fmt::Display for StateStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(error) => write!(f, "{error}"),
            Self::Io { path, message } => {
                write!(
                    f,
                    "state store filesystem error at {}: {message}",
                    path.display()
                )
            }
            Self::JsonSyntax { path, message } => {
                write!(f, "failed to decode JSON at {}: {message}", path.display())
            }
            Self::JsonLineSyntax {
                path,
                line_number,
                message,
            } => {
                write!(
                    f,
                    "failed to decode JSON at {} line {}: {message}",
                    path.display(),
                    line_number
                )
            }
            Self::JsonRender { path, message } => {
                write!(f, "failed to encode JSON at {}: {message}", path.display())
            }
            Self::RuntimeJson { path, source } => {
                write!(
                    f,
                    "runtime JSON contract error at {}: {source}",
                    path.display()
                )
            }
            Self::RuntimeJsonLine {
                path,
                line_number,
                source,
            } => {
                write!(
                    f,
                    "runtime JSON contract error at {} line {}: {source}",
                    path.display(),
                    line_number
                )
            }
            Self::NonObjectPayload { path } => {
                write!(f, "expected object payload in {}", path.display())
            }
            Self::StatusMarker { message } => f.write_str(message),
            Self::InvalidPath { path, message } => {
                write!(f, "invalid state path {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for StateStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::RuntimeJson { source, .. } => Some(source),
            Self::RuntimeJsonLine { source, .. } => Some(source),
            Self::Io { .. }
            | Self::JsonSyntax { .. }
            | Self::JsonRender { .. }
            | Self::JsonLineSyntax { .. }
            | Self::NonObjectPayload { .. }
            | Self::StatusMarker { .. }
            | Self::InvalidPath { .. } => None,
        }
    }
}

impl From<WorkspaceError> for StateStoreError {
    fn from(value: WorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

/// Runtime state store facade rooted at one initialized workspace.
#[derive(Debug, Clone)]
pub struct StateStore {
    /// Resolved workspace paths.
    pub paths: WorkspacePaths,
}

impl StateStore {
    /// Initialize or open a runtime state store rooted at the provided workspace.
    pub fn new(root: impl AsRef<Path>) -> StateStoreResult<Self> {
        let paths = initialize_workspace(root)?;
        Ok(Self { paths })
    }

    /// Build a state store from already resolved workspace paths.
    #[must_use]
    pub fn from_paths(paths: WorkspacePaths) -> Self {
        Self { paths }
    }

    /// Load and validate the runtime snapshot.
    pub fn load_snapshot(&self) -> StateStoreResult<RuntimeSnapshot> {
        load_snapshot(&self.paths)
    }

    /// Validate and atomically save the runtime snapshot.
    pub fn save_snapshot(&self, snapshot: &RuntimeSnapshot) -> StateStoreResult<()> {
        save_snapshot(&self.paths, snapshot)
    }

    /// Load and validate recovery counters.
    pub fn load_recovery_counters(&self) -> StateStoreResult<RecoveryCounters> {
        load_recovery_counters(&self.paths)
    }

    /// Validate and atomically save recovery counters.
    pub fn save_recovery_counters(&self, counters: &RecoveryCounters) -> StateStoreResult<()> {
        save_recovery_counters(&self.paths, counters)
    }

    /// Load the execution status marker.
    pub fn load_execution_status(&self) -> StateStoreResult<String> {
        load_execution_status(&self.paths)
    }

    /// Load the planning status marker.
    pub fn load_planning_status(&self) -> StateStoreResult<String> {
        load_planning_status(&self.paths)
    }

    /// Load the learning status marker.
    pub fn load_learning_status(&self) -> StateStoreResult<String> {
        load_learning_status(&self.paths)
    }

    /// Normalize and atomically set the execution status marker.
    pub fn set_execution_status(&self, marker: &str) -> StateStoreResult<String> {
        set_execution_status(&self.paths, marker)
    }

    /// Normalize and atomically set the planning status marker.
    pub fn set_planning_status(&self, marker: &str) -> StateStoreResult<String> {
        set_planning_status(&self.paths, marker)
    }

    /// Normalize and atomically set the learning status marker.
    pub fn set_learning_status(&self, marker: &str) -> StateStoreResult<String> {
        set_learning_status(&self.paths, marker)
    }

    /// Increment and persist a troubleshoot attempt counter.
    pub fn increment_troubleshoot_attempt(
        &self,
        failure_class: &str,
        work_item_kind: WorkItemKind,
        work_item_id: &str,
        now: Timestamp,
    ) -> StateStoreResult<RecoveryCounterEntry> {
        increment_troubleshoot_attempt(
            &self.paths,
            failure_class,
            work_item_kind,
            work_item_id,
            now,
        )
    }

    /// Reset recovery counters for work that has made forward progress.
    pub fn reset_forward_progress_counters(
        &self,
        work_item_kind: WorkItemKind,
        work_item_id: &str,
    ) -> StateStoreResult<()> {
        reset_forward_progress_counters(&self.paths, work_item_kind, work_item_id)
    }

    /// Load and validate usage-governance state, or return the disabled default.
    pub fn load_usage_governance_state(&self) -> StateStoreResult<UsageGovernanceState> {
        load_usage_governance_state(&self.paths)
    }

    /// Validate and atomically save usage-governance state.
    pub fn save_usage_governance_state(
        &self,
        state: &UsageGovernanceState,
    ) -> StateStoreResult<()> {
        save_usage_governance_state(&self.paths, state)
    }

    /// Load and validate usage-governance ledger JSONL entries.
    pub fn load_usage_governance_ledger(
        &self,
    ) -> StateStoreResult<Vec<UsageGovernanceLedgerEntry>> {
        load_usage_governance_ledger(&self.paths)
    }
}

/// Load and validate the runtime snapshot.
pub fn load_snapshot(paths: &WorkspacePaths) -> StateStoreResult<RuntimeSnapshot> {
    load_runtime_json_contract(&paths.runtime_snapshot_file)
}

/// Validate and atomically save the runtime snapshot.
pub fn save_snapshot(paths: &WorkspacePaths, snapshot: &RuntimeSnapshot) -> StateStoreResult<()> {
    save_runtime_json_contract(&paths.runtime_snapshot_file, snapshot)
}

/// Load and validate recovery counters.
pub fn load_recovery_counters(paths: &WorkspacePaths) -> StateStoreResult<RecoveryCounters> {
    load_runtime_json_contract(&paths.recovery_counters_file)
}

/// Validate and atomically save recovery counters.
pub fn save_recovery_counters(
    paths: &WorkspacePaths,
    counters: &RecoveryCounters,
) -> StateStoreResult<()> {
    save_runtime_json_contract(&paths.recovery_counters_file, counters)
}

/// Load and validate usage-governance state.
///
/// A missing file is an inert disabled state and does not create the file.
pub fn load_usage_governance_state(
    paths: &WorkspacePaths,
) -> StateStoreResult<UsageGovernanceState> {
    if !paths.usage_governance_state_file.is_file() {
        return disabled_usage_governance_state();
    }
    load_runtime_json_contract(&paths.usage_governance_state_file)
}

/// Validate and atomically save usage-governance state.
pub fn save_usage_governance_state(
    paths: &WorkspacePaths,
    state: &UsageGovernanceState,
) -> StateStoreResult<()> {
    save_runtime_json_contract(&paths.usage_governance_state_file, state)
}

/// Load and validate usage-governance ledger JSONL entries.
pub fn load_usage_governance_ledger(
    paths: &WorkspacePaths,
) -> StateStoreResult<Vec<UsageGovernanceLedgerEntry>> {
    let path = &paths.usage_governance_ledger_file;
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path).map_err(|error| StateStoreError::io(path, error))?;
    let mut entries = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let line_number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(line).map_err(|error| StateStoreError::JsonLineSyntax {
                path: path.to_path_buf(),
                line_number,
                message: error.to_string(),
            })?;
        let entry = UsageGovernanceLedgerEntry::from_json_value(value).map_err(|source| {
            StateStoreError::RuntimeJsonLine {
                path: path.to_path_buf(),
                line_number,
                source,
            }
        })?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Validate and append one usage-governance ledger entry.
pub fn append_usage_governance_ledger_entry(
    paths: &WorkspacePaths,
    entry: &UsageGovernanceLedgerEntry,
) -> StateStoreResult<()> {
    let mut validated = entry.clone();
    validated.validate_contract().map_err(|source| {
        StateStoreError::runtime_json(&paths.usage_governance_ledger_file, source)
    })?;
    let mut payload =
        serde_json::to_string(&validated).map_err(|error| StateStoreError::JsonRender {
            path: paths.usage_governance_ledger_file.clone(),
            message: error.to_string(),
        })?;
    payload.push('\n');
    append_text(&paths.usage_governance_ledger_file, &payload)
}

/// Load and normalize the execution status marker.
pub fn load_execution_status(paths: &WorkspacePaths) -> StateStoreResult<String> {
    load_status_marker(&paths.execution_status_file)
}

/// Load and normalize the planning status marker.
pub fn load_planning_status(paths: &WorkspacePaths) -> StateStoreResult<String> {
    load_status_marker(&paths.planning_status_file)
}

/// Load and normalize the learning status marker.
pub fn load_learning_status(paths: &WorkspacePaths) -> StateStoreResult<String> {
    load_status_marker(&paths.learning_status_file)
}

/// Normalize and atomically set the execution status marker.
pub fn set_execution_status(paths: &WorkspacePaths, marker: &str) -> StateStoreResult<String> {
    set_status_marker(&paths.execution_status_file, marker)
}

/// Normalize and atomically set the planning status marker.
pub fn set_planning_status(paths: &WorkspacePaths, marker: &str) -> StateStoreResult<String> {
    set_status_marker(&paths.planning_status_file, marker)
}

/// Normalize and atomically set the learning status marker.
pub fn set_learning_status(paths: &WorkspacePaths, marker: &str) -> StateStoreResult<String> {
    set_status_marker(&paths.learning_status_file, marker)
}

/// Increment and persist a troubleshoot attempt counter.
pub fn increment_troubleshoot_attempt(
    paths: &WorkspacePaths,
    failure_class: &str,
    work_item_kind: WorkItemKind,
    work_item_id: &str,
    now: Timestamp,
) -> StateStoreResult<RecoveryCounterEntry> {
    if failure_class.trim().is_empty() {
        return Err(StateStoreError::StatusMarker {
            message: "failure_class is required".to_owned(),
        });
    }
    if work_item_id.trim().is_empty() {
        return Err(StateStoreError::StatusMarker {
            message: "work_item_id is required".to_owned(),
        });
    }

    let mut counters = load_recovery_counters(paths)?;
    let mut updated_entry = None;
    for entry in &mut counters.entries {
        if entry.failure_class == failure_class
            && entry.work_item_kind == work_item_kind
            && entry.work_item_id == work_item_id
        {
            entry.troubleshoot_attempt_count += 1;
            entry.last_updated_at = now.clone();
            updated_entry = Some(entry.clone());
            break;
        }
    }

    let updated_entry = match updated_entry {
        Some(entry) => entry,
        None => {
            let entry = RecoveryCounterEntry {
                failure_class: failure_class.to_owned(),
                work_item_kind,
                work_item_id: work_item_id.to_owned(),
                troubleshoot_attempt_count: 1,
                mechanic_attempt_count: 0,
                fix_cycle_count: 0,
                consultant_invocations: 0,
                last_updated_at: now,
            };
            counters.entries.push(entry.clone());
            entry
        }
    };

    save_recovery_counters(paths, &counters)?;
    Ok(updated_entry)
}

/// Remove recovery counters for work that has made forward progress.
pub fn reset_forward_progress_counters(
    paths: &WorkspacePaths,
    work_item_kind: WorkItemKind,
    work_item_id: &str,
) -> StateStoreResult<()> {
    let mut counters = load_recovery_counters(paths)?;
    counters.entries.retain(|entry| {
        entry.work_item_kind != work_item_kind || entry.work_item_id != work_item_id
    });
    save_recovery_counters(paths, &counters)
}

fn disabled_usage_governance_state() -> StateStoreResult<UsageGovernanceState> {
    Ok(UsageGovernanceState {
        version: "1.0".to_owned(),
        enabled: false,
        auto_resume: true,
        auto_resume_possible: true,
        evaluation_boundary: crate::contracts::UsageGovernanceEvaluationBoundary::BetweenStages,
        calendar_timezone: "UTC".to_owned(),
        daemon_session_id: None,
        last_evaluated_at: epoch_timestamp()?,
        active_blockers: Vec::new(),
        paused_by_governance: false,
        next_auto_resume_at: None,
        subscription_quota_status: SubscriptionQuotaStatus::default(),
    })
}

fn epoch_timestamp() -> StateStoreResult<Timestamp> {
    Timestamp::parse("last_evaluated_at", "1970-01-01T00:00:00Z").map_err(|error| {
        StateStoreError::StatusMarker {
            message: error.to_string(),
        }
    })
}

fn load_runtime_json_contract<T>(path: &Path) -> StateStoreResult<T>
where
    T: RuntimeJsonContract,
{
    let raw = fs::read_to_string(path).map_err(|error| StateStoreError::io(path, error))?;
    let value: Value = serde_json::from_str(&raw).map_err(|error| StateStoreError::JsonSyntax {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if !value.is_object() {
        return Err(StateStoreError::NonObjectPayload {
            path: path.to_path_buf(),
        });
    }
    T::from_json_value(value).map_err(|source| StateStoreError::runtime_json(path, source))
}

fn save_runtime_json_contract<T>(path: &Path, model: &T) -> StateStoreResult<()>
where
    T: RuntimeJsonContract + Clone + Serialize,
{
    let mut validated = model.clone();
    validated
        .validate_contract()
        .map_err(|source| StateStoreError::runtime_json(path, source))?;
    let mut payload =
        serde_json::to_string_pretty(&validated).map_err(|error| StateStoreError::JsonRender {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    payload.push('\n');
    atomic_write_text(path, &payload)
}

fn load_status_marker(path: &Path) -> StateStoreResult<String> {
    let raw = fs::read_to_string(path).map_err(|error| StateStoreError::io(path, error))?;
    normalize_status_marker(&raw)
}

fn set_status_marker(path: &Path, marker: &str) -> StateStoreResult<String> {
    let normalized = normalize_status_marker(marker)?;
    atomic_write_text(path, &(normalized.clone() + "\n"))?;
    Ok(normalized)
}

/// Normalize a status marker to one `### ...` line.
pub fn normalize_status_marker(marker: &str) -> StateStoreResult<String> {
    let normalized = marker.trim();
    if normalized.is_empty() {
        return Err(StateStoreError::status_marker(
            "status marker cannot be empty",
        ));
    }
    let mut lines = normalized.lines();
    let first = lines.next().unwrap_or_default();
    if lines.next().is_some() {
        return Err(StateStoreError::status_marker(
            "status marker must be a single line",
        ));
    }
    if !first.starts_with("### ") || first[4..].trim().is_empty() {
        return Err(StateStoreError::status_marker(
            "status marker must start with '### '",
        ));
    }
    Ok(first.to_owned())
}

/// Write text through a same-directory temporary file and atomic rename.
pub fn atomic_write_text(path: &Path, payload: &str) -> StateStoreResult<()> {
    atomic_write_text_with_replace(path, payload, |source, target| fs::rename(source, target))
}

fn append_text(path: &Path, payload: &str) -> StateStoreResult<()> {
    let parent = path.parent().ok_or_else(|| StateStoreError::InvalidPath {
        path: path.to_path_buf(),
        message: "state path must have a parent directory".to_owned(),
    })?;
    fs::create_dir_all(parent).map_err(|error| StateStoreError::io(parent, error))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| StateStoreError::io(path, error))?;
    file.write_all(payload.as_bytes())
        .map_err(|error| StateStoreError::io(path, error))?;
    file.flush()
        .map_err(|error| StateStoreError::io(path, error))?;
    file.sync_all()
        .map_err(|error| StateStoreError::io(path, error))
}

fn atomic_write_text_with_replace<F>(path: &Path, payload: &str, replace: F) -> StateStoreResult<()>
where
    F: Fn(&Path, &Path) -> io::Result<()>,
{
    let parent = path.parent().ok_or_else(|| StateStoreError::InvalidPath {
        path: path.to_path_buf(),
        message: "state path must have a parent directory".to_owned(),
    })?;
    fs::create_dir_all(parent).map_err(|error| StateStoreError::io(parent, error))?;
    let temp_path = temp_path_for(path)?;

    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(payload.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        replace(&temp_path, path)
    })();

    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            Err(StateStoreError::io(path, error))
        }
    }
}

fn temp_path_for(path: &Path) -> StateStoreResult<PathBuf> {
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| StateStoreError::InvalidPath {
            path: path.to_path_buf(),
            message: "state path must have a UTF-8 filename".to_owned(),
        })?;
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(path.with_file_name(format!(".{filename}.tmp-{}-{counter}", process::id())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_cleans_temp_and_preserves_destination_when_replace_fails() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = temp_dir.path().join("runtime_snapshot.json");
        fs::write(&path, "original\n").unwrap();

        let error = atomic_write_text_with_replace(&path, "replacement\n", |_source, _target| {
            Err(io::Error::other("replace failure"))
        })
        .unwrap_err();

        assert!(error.to_string().contains("replace failure"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
        let leftovers: Vec<_> = fs::read_dir(temp_dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
    }
}
