//! Runtime control facade that routes offline mutations or daemon mailbox commands.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Serialize;
use serde_json::{Map, Value};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::contracts::{
    ActiveRunState, MailboxAddIdeaPayload, MailboxAddProbePayload, MailboxAddSpecPayload,
    MailboxAddTaskPayload, MailboxCommand, MailboxCommandEnvelope, PauseSource, Plane,
    ProbeDocument, RecoveryCounters, RuntimeJsonContract, RuntimeJsonError, RuntimeSnapshot,
    SpecDocument, TaskDocument, Timestamp, WorkItemKind,
};

use super::{
    QueueStore, QueueStoreError, RuntimeOwnershipLockError, RuntimeOwnershipLockState,
    StateStoreError, WorkspaceError, WorkspacePaths, atomic_write_text,
    clear_stale_runtime_ownership_lock, inspect_runtime_ownership_lock, load_recovery_counters,
    load_snapshot, load_usage_governance_state, require_initialized_workspace,
    require_initialized_workspace_paths, reset_forward_progress_counters, save_recovery_counters,
    save_snapshot, set_execution_status, set_learning_status, set_planning_status,
};

static COMMAND_COUNTER: AtomicU64 = AtomicU64::new(0);

const IDLE_STATUS_MARKER: &str = "### IDLE";
const DEFAULT_ISSUER: &str = "operator";
const DEFAULT_RETRY_REASON: &str = "operator requested retry";
const DEFAULT_PLANNING_RETRY_REASON: &str = "operator requested planning retry";
const DEFAULT_CLEAR_STALE_REASON: &str = "operator requested stale-state clear";

/// Result type for runtime control routing and mutation operations.
pub type RuntimeControlResult<T> = Result<T, RuntimeControlError>;

/// Control routing mode selected for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeControlMode {
    /// No active daemon lock owns the workspace, so the facade applied local state changes.
    Direct,
    /// An active daemon lock owns the workspace, so the facade wrote a mailbox command.
    Mailbox,
}

impl RuntimeControlMode {
    /// Returns the Python-compatible rendered mode value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Mailbox => "mailbox",
        }
    }
}

impl fmt::Display for RuntimeControlMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Outcome for one runtime control action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeControlActionResult {
    /// Command requested by the caller.
    pub action: MailboxCommand,
    /// Whether this was applied directly or mailbox-routed.
    pub mode: RuntimeControlMode,
    /// Whether a direct mutation changed runtime state.
    pub applied: bool,
    /// Human-readable detail compatible with Python CLI rendering.
    pub detail: String,
    /// Mailbox command id when `mode` is mailbox.
    pub command_id: Option<String>,
    /// Mailbox envelope path when `mode` is mailbox.
    pub mailbox_path: Option<PathBuf>,
    /// Directly written queue or idea artifact path when applicable.
    pub artifact_path: Option<PathBuf>,
}

impl RuntimeControlActionResult {
    fn direct(action: MailboxCommand, applied: bool, detail: impl Into<String>) -> Self {
        Self {
            action,
            mode: RuntimeControlMode::Direct,
            applied,
            detail: detail.into(),
            command_id: None,
            mailbox_path: None,
            artifact_path: None,
        }
    }

    fn direct_artifact(
        action: MailboxCommand,
        detail: impl Into<String>,
        artifact_path: PathBuf,
    ) -> Self {
        Self {
            action,
            mode: RuntimeControlMode::Direct,
            applied: true,
            detail: detail.into(),
            command_id: None,
            mailbox_path: None,
            artifact_path: Some(artifact_path),
        }
    }

    fn mailbox(
        action: MailboxCommand,
        detail: impl Into<String>,
        command_id: String,
        mailbox_path: PathBuf,
    ) -> Self {
        Self {
            action,
            mode: RuntimeControlMode::Mailbox,
            applied: false,
            detail: detail.into(),
            command_id: Some(command_id),
            mailbox_path: Some(mailbox_path),
            artifact_path: None,
        }
    }
}

/// Failures produced by runtime-control routing or local mutations.
#[derive(Debug)]
pub enum RuntimeControlError {
    /// Workspace initialization or path validation failed.
    Workspace(WorkspaceError),
    /// Runtime state load or save failed.
    StateStore(StateStoreError),
    /// Queue document write or transition failed.
    QueueStore(QueueStoreError),
    /// Runtime lock mutation failed.
    RuntimeLock(RuntimeOwnershipLockError),
    /// Runtime JSON validation failed.
    RuntimeJson {
        /// Path being read or written when available.
        path: PathBuf,
        /// Typed runtime JSON contract error.
        source: RuntimeJsonError,
    },
    /// A filesystem operation failed.
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying IO error message.
        message: String,
    },
    /// JSON serialization failed before persistence.
    JsonRender {
        /// Path being rendered when available.
        path: PathBuf,
        /// Serde error message.
        message: String,
    },
    /// A caller-provided value was invalid.
    InvalidField {
        /// Field involved in the failure.
        field_name: &'static str,
        /// Human-readable validation message.
        message: String,
    },
    /// A mailbox command would overwrite an existing command file.
    MailboxCommandExists {
        /// Existing mailbox command path.
        path: PathBuf,
    },
}

impl RuntimeControlError {
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

    fn invalid_field(field_name: &'static str, message: impl Into<String>) -> Self {
        Self::InvalidField {
            field_name,
            message: message.into(),
        }
    }
}

impl fmt::Display for RuntimeControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(error) => write!(f, "{error}"),
            Self::StateStore(error) => write!(f, "{error}"),
            Self::QueueStore(error) => write!(f, "{error}"),
            Self::RuntimeLock(error) => write!(f, "{error}"),
            Self::RuntimeJson { path, source } => {
                write!(
                    f,
                    "runtime control JSON contract error at {}: {source}",
                    path.display()
                )
            }
            Self::Io { path, message } => {
                write!(
                    f,
                    "runtime control filesystem error at {}: {message}",
                    path.display()
                )
            }
            Self::JsonRender { path, message } => {
                write!(
                    f,
                    "failed to render runtime control JSON at {}: {message}",
                    path.display()
                )
            }
            Self::InvalidField {
                field_name,
                message,
            } => {
                write!(f, "{field_name} is invalid: {message}")
            }
            Self::MailboxCommandExists { path } => {
                write!(f, "mailbox command already exists: {}", path.display())
            }
        }
    }
}

impl std::error::Error for RuntimeControlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::StateStore(error) => Some(error),
            Self::QueueStore(error) => Some(error),
            Self::RuntimeLock(error) => Some(error),
            Self::RuntimeJson { source, .. } => Some(source),
            Self::Io { .. }
            | Self::JsonRender { .. }
            | Self::InvalidField { .. }
            | Self::MailboxCommandExists { .. } => None,
        }
    }
}

impl From<WorkspaceError> for RuntimeControlError {
    fn from(value: WorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

impl From<StateStoreError> for RuntimeControlError {
    fn from(value: StateStoreError) -> Self {
        Self::StateStore(value)
    }
}

impl From<QueueStoreError> for RuntimeControlError {
    fn from(value: QueueStoreError) -> Self {
        Self::QueueStore(value)
    }
}

impl From<RuntimeOwnershipLockError> for RuntimeControlError {
    fn from(value: RuntimeOwnershipLockError) -> Self {
        Self::RuntimeLock(value)
    }
}

/// Public runtime-control API rooted at one initialized workspace.
#[derive(Debug, Clone)]
pub struct RuntimeControl {
    /// Resolved workspace paths.
    pub paths: WorkspacePaths,
}

impl RuntimeControl {
    /// Open runtime control for an initialized workspace root.
    pub fn new(root: impl AsRef<Path>) -> RuntimeControlResult<Self> {
        let paths = require_initialized_workspace(root)?;
        Ok(Self { paths })
    }

    /// Open runtime control from pre-resolved initialized workspace paths.
    pub fn from_paths(paths: WorkspacePaths) -> RuntimeControlResult<Self> {
        require_initialized_workspace_paths(&paths)?;
        Ok(Self { paths })
    }

    /// Pause the runtime or enqueue a daemon-owned pause command.
    pub fn pause_runtime(&self) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.pause_runtime_with_issuer(DEFAULT_ISSUER)
    }

    /// Pause the runtime or enqueue a daemon-owned pause command with a caller-provided issuer.
    pub fn pause_runtime_with_issuer(
        &self,
        issuer: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.dispatch(MailboxCommand::Pause, issuer, None, |snapshot| {
            self.pause_direct(snapshot)
        })
    }

    /// Resume the runtime or enqueue a daemon-owned resume command.
    pub fn resume_runtime(&self) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.resume_runtime_with_issuer(DEFAULT_ISSUER)
    }

    /// Resume the runtime or enqueue a daemon-owned resume command with a caller-provided issuer.
    pub fn resume_runtime_with_issuer(
        &self,
        issuer: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.dispatch(MailboxCommand::Resume, issuer, None, |snapshot| {
            self.resume_direct(snapshot)
        })
    }

    /// Stop the runtime or enqueue a daemon-owned stop command.
    pub fn stop_runtime(&self) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.stop_runtime_with_issuer(DEFAULT_ISSUER)
    }

    /// Stop the runtime or enqueue a daemon-owned stop command with a caller-provided issuer.
    pub fn stop_runtime_with_issuer(
        &self,
        issuer: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.dispatch(MailboxCommand::Stop, issuer, None, |snapshot| {
            self.stop_direct(snapshot)
        })
    }

    /// Retry the only active work item or enqueue a daemon-owned retry command.
    pub fn retry_active(&self, reason: &str) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.retry_active_with_issuer(reason, DEFAULT_ISSUER)
    }

    /// Retry the only active work item or enqueue a daemon-owned retry command with an issuer.
    pub fn retry_active_with_issuer(
        &self,
        reason: &str,
        issuer: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let reason = normalized_reason(reason, DEFAULT_RETRY_REASON);
        let payload = reason_payload(&reason);
        self.dispatch(
            MailboxCommand::RetryActive,
            issuer,
            Some(payload),
            |snapshot| self.retry_active_direct(snapshot, reason, None),
        )
    }

    /// Retry active planning work or enqueue a daemon-owned plane-scoped retry command.
    pub fn retry_active_planning(
        &self,
        reason: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.retry_active_planning_with_issuer(reason, DEFAULT_ISSUER)
    }

    /// Retry active planning work or enqueue a daemon-owned plane-scoped retry command with an issuer.
    pub fn retry_active_planning_with_issuer(
        &self,
        reason: &str,
        issuer: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let snapshot = load_snapshot(&self.paths)?;
        if active_run_for_plane(&snapshot, Plane::Planning).is_none() {
            let active_planes = active_planes_label(&snapshot);
            return Ok(RuntimeControlActionResult::direct(
                MailboxCommand::RetryActive,
                false,
                format!(
                    "planning retry requires active planning work; current active planes are {active_planes}"
                ),
            ));
        }

        let reason = normalized_reason(reason, DEFAULT_PLANNING_RETRY_REASON);
        let mut payload = reason_payload(&reason);
        payload.insert(
            "scope".to_owned(),
            Value::String(Plane::Planning.as_str().to_owned()),
        );
        self.dispatch(
            MailboxCommand::RetryActive,
            issuer,
            Some(payload),
            |snapshot| self.retry_active_direct(snapshot, reason, Some(Plane::Planning)),
        )
    }

    /// Clear stale runtime state or enqueue a daemon-owned clear-stale-state command.
    pub fn clear_stale_state(
        &self,
        reason: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.clear_stale_state_with_issuer(reason, DEFAULT_ISSUER)
    }

    /// Clear stale runtime state or enqueue a daemon-owned clear-stale-state command with an issuer.
    pub fn clear_stale_state_with_issuer(
        &self,
        reason: &str,
        issuer: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let reason = normalized_reason(reason, DEFAULT_CLEAR_STALE_REASON);
        let payload = reason_payload(&reason);
        self.dispatch(
            MailboxCommand::ClearStaleState,
            issuer,
            Some(payload),
            |snapshot| self.clear_stale_direct(snapshot, reason),
        )
    }

    /// Reload runtime config or enqueue a daemon-owned reload command.
    pub fn reload_config(&self) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.reload_config_with_issuer(DEFAULT_ISSUER)
    }

    /// Reload runtime config or enqueue a daemon-owned reload command with an issuer.
    pub fn reload_config_with_issuer(
        &self,
        issuer: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.dispatch(MailboxCommand::ReloadConfig, issuer, None, |snapshot| {
            self.reload_config_direct(snapshot)
        })
    }

    /// Add a task to the execution queue or enqueue a daemon-owned add-task command.
    pub fn add_task(
        &self,
        document: &TaskDocument,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.add_task_with_issuer(document, DEFAULT_ISSUER)
    }

    /// Add a task to the execution queue or enqueue a daemon-owned add-task command with an issuer.
    pub fn add_task_with_issuer(
        &self,
        document: &TaskDocument,
        issuer: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let mut payload_model = MailboxAddTaskPayload {
            document: document.clone(),
        };
        payload_model.validate().map_err(|source| {
            RuntimeControlError::runtime_json(self.paths.mailbox_incoming_dir.clone(), source)
        })?;
        let payload = payload_map(&self.paths.mailbox_incoming_dir, &payload_model)?;
        self.dispatch(MailboxCommand::AddTask, issuer, Some(payload), |snapshot| {
            self.add_task_direct(snapshot, document)
        })
    }

    /// Add a probe to the planning probe queue or enqueue a daemon-owned add-probe command.
    pub fn add_probe(
        &self,
        document: &ProbeDocument,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.add_probe_with_issuer(document, DEFAULT_ISSUER)
    }

    /// Add a probe to the planning probe queue or enqueue a daemon-owned add-probe command with an issuer.
    pub fn add_probe_with_issuer(
        &self,
        document: &ProbeDocument,
        issuer: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let mut payload_model = MailboxAddProbePayload {
            document: document.clone(),
        };
        payload_model.validate().map_err(|source| {
            RuntimeControlError::runtime_json(self.paths.mailbox_incoming_dir.clone(), source)
        })?;
        let payload = payload_map(&self.paths.mailbox_incoming_dir, &payload_model)?;
        self.dispatch(
            MailboxCommand::AddProbe,
            issuer,
            Some(payload),
            |snapshot| self.add_probe_direct(snapshot, document),
        )
    }

    /// Add a spec to the planning queue or enqueue a daemon-owned add-spec command.
    pub fn add_spec(
        &self,
        document: &SpecDocument,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.add_spec_with_issuer(document, DEFAULT_ISSUER)
    }

    /// Add a spec to the planning queue or enqueue a daemon-owned add-spec command with an issuer.
    pub fn add_spec_with_issuer(
        &self,
        document: &SpecDocument,
        issuer: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let mut payload_model = MailboxAddSpecPayload {
            document: document.clone(),
        };
        payload_model.validate().map_err(|source| {
            RuntimeControlError::runtime_json(self.paths.mailbox_incoming_dir.clone(), source)
        })?;
        let payload = payload_map(&self.paths.mailbox_incoming_dir, &payload_model)?;
        self.dispatch(MailboxCommand::AddSpec, issuer, Some(payload), |snapshot| {
            self.add_spec_direct(snapshot, document)
        })
    }

    /// Stage idea markdown or enqueue a daemon-owned add-idea command.
    pub fn add_idea_markdown(
        &self,
        source_name: &str,
        markdown: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        self.add_idea_markdown_with_issuer(source_name, markdown, DEFAULT_ISSUER)
    }

    /// Stage idea markdown or enqueue a daemon-owned add-idea command with an issuer.
    pub fn add_idea_markdown_with_issuer(
        &self,
        source_name: &str,
        markdown: &str,
        issuer: &str,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let mut payload_model = MailboxAddIdeaPayload {
            source_name: source_name.to_owned(),
            markdown: markdown.to_owned(),
        };
        payload_model.validate().map_err(|source| {
            RuntimeControlError::runtime_json(self.paths.mailbox_incoming_dir.clone(), source)
        })?;
        let payload = payload_map(&self.paths.mailbox_incoming_dir, &payload_model)?;
        self.dispatch(MailboxCommand::AddIdea, issuer, Some(payload), |snapshot| {
            self.add_idea_direct(snapshot, &payload_model)
        })
    }

    fn dispatch<F>(
        &self,
        command: MailboxCommand,
        issuer: &str,
        payload: Option<Map<String, Value>>,
        direct_handler: F,
    ) -> RuntimeControlResult<RuntimeControlActionResult>
    where
        F: FnOnce(RuntimeSnapshot) -> RuntimeControlResult<RuntimeControlActionResult>,
    {
        validate_issuer(issuer)?;
        let snapshot = load_snapshot(&self.paths)?;
        let lock_status = inspect_runtime_ownership_lock(&self.paths);
        if lock_status.state == RuntimeOwnershipLockState::Active {
            return self.enqueue_mailbox_command(command, issuer, payload.unwrap_or_default());
        }

        direct_handler(snapshot)
    }

    fn enqueue_mailbox_command(
        &self,
        command: MailboxCommand,
        issuer: &str,
        payload: Map<String, Value>,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let (issued_at, timestamp_ms) = current_timestamp("issued_at")?;
        let command_id = command_id(command, timestamp_ms);
        let envelope = MailboxCommandEnvelope {
            schema_version: "1.0".to_owned(),
            kind: "mailbox_command".to_owned(),
            command_id: command_id.clone(),
            command,
            issued_at,
            issuer: issuer.to_owned(),
            payload,
        };
        let mailbox_path = write_mailbox_command(&self.paths, &envelope)?;
        Ok(RuntimeControlActionResult::mailbox(
            command,
            mailbox_detail(command),
            command_id,
            mailbox_path,
        ))
    }

    fn pause_direct(
        &self,
        mut snapshot: RuntimeSnapshot,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let changed = !has_pause_source(&snapshot, PauseSource::Operator);
        snapshot.pause_sources = ordered_pause_sources(
            snapshot
                .pause_sources
                .into_iter()
                .chain([PauseSource::Operator]),
        );
        snapshot.paused = true;
        snapshot.updated_at = now_timestamp("updated_at")?;
        save_snapshot(&self.paths, &snapshot)?;
        Ok(RuntimeControlActionResult::direct(
            MailboxCommand::Pause,
            changed,
            "runtime paused directly",
        ))
    }

    fn resume_direct(
        &self,
        mut snapshot: RuntimeSnapshot,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let changed = has_pause_source(&snapshot, PauseSource::Operator);
        let governance_state = load_usage_governance_state(&self.paths)?;
        let governance_blocked = has_pause_source(&snapshot, PauseSource::UsageGovernance)
            || !governance_state.active_blockers.is_empty();
        if governance_blocked {
            snapshot.pause_sources = ordered_pause_sources(
                snapshot
                    .pause_sources
                    .into_iter()
                    .chain([PauseSource::UsageGovernance]),
            );
            snapshot.paused = true;
        }
        snapshot.pause_sources = ordered_pause_sources(
            snapshot
                .pause_sources
                .into_iter()
                .filter(|source| *source != PauseSource::Operator),
        );
        snapshot.paused = !snapshot.pause_sources.is_empty();
        snapshot.updated_at = now_timestamp("updated_at")?;
        save_snapshot(&self.paths, &snapshot)?;
        if governance_blocked {
            return Ok(RuntimeControlActionResult::direct(
                MailboxCommand::Resume,
                false,
                "runtime resume blocked by usage governance",
            ));
        }
        Ok(RuntimeControlActionResult::direct(
            MailboxCommand::Resume,
            changed,
            "runtime resumed directly",
        ))
    }

    fn stop_direct(
        &self,
        snapshot: RuntimeSnapshot,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let changed = snapshot.process_running || !snapshot.stop_requested;
        self.reset_runtime_to_idle(snapshot, false, true, true)?;
        Ok(RuntimeControlActionResult::direct(
            MailboxCommand::Stop,
            changed,
            "runtime stopped directly",
        ))
    }

    fn retry_active_direct(
        &self,
        snapshot: RuntimeSnapshot,
        reason: String,
        scope: Option<Plane>,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let active_run = match retry_active_run(&snapshot, scope) {
            RetryActiveRunSelection::Selected(active_run) => active_run,
            RetryActiveRunSelection::Missing => {
                return Ok(RuntimeControlActionResult::direct(
                    MailboxCommand::RetryActive,
                    false,
                    retry_active_missing_detail(&snapshot, scope),
                ));
            }
            RetryActiveRunSelection::Multiple => {
                return Ok(RuntimeControlActionResult::direct(
                    MailboxCommand::RetryActive,
                    false,
                    "multiple active planes; retry-active requires a plane scope",
                ));
            }
        };

        let Some(work_item_kind) = active_run.work_item_kind else {
            return Ok(RuntimeControlActionResult::direct(
                MailboxCommand::RetryActive,
                false,
                format!(
                    "active {} run is not a retryable work item",
                    active_run.plane.as_str()
                ),
            ));
        };
        let Some(work_item_id) = active_run.work_item_id.clone() else {
            return Ok(RuntimeControlActionResult::direct(
                MailboxCommand::RetryActive,
                false,
                format!(
                    "active {} run is not a retryable work item",
                    active_run.plane.as_str()
                ),
            ));
        };

        let queue = QueueStore::from_paths(self.paths.clone());
        if let Err(error) = requeue_active_item(&queue, work_item_kind, &work_item_id, &reason) {
            if let QueueStoreError::InvalidState { message } = error {
                return Ok(RuntimeControlActionResult::direct(
                    MailboxCommand::RetryActive,
                    false,
                    message,
                ));
            }
            return Err(error.into());
        }

        self.clear_retry_active_run(snapshot, active_run.plane)?;
        reset_forward_progress_counters(&self.paths, work_item_kind, &work_item_id)?;
        Ok(RuntimeControlActionResult::direct(
            MailboxCommand::RetryActive,
            true,
            format!(
                "active {} {} requeued",
                work_item_kind.as_str(),
                work_item_id
            ),
        ))
    }

    fn clear_stale_direct(
        &self,
        snapshot: RuntimeSnapshot,
        reason: String,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let had_counters = !load_recovery_counters(&self.paths)?.entries.is_empty();
        let queue = QueueStore::from_paths(self.paths.clone());
        let requeued_count = self.requeue_all_active_items(&queue, &reason)?;
        let lock_clear_result = clear_stale_runtime_ownership_lock(&self.paths)?;

        let snapshot_had_state = snapshot.active_stage.is_some()
            || snapshot.process_running
            || snapshot.paused
            || snapshot.stop_requested;
        self.reset_runtime_to_idle(snapshot, false, true, true)?;
        save_recovery_counters(
            &self.paths,
            &RecoveryCounters {
                schema_version: "1.0".to_owned(),
                kind: "recovery_counters".to_owned(),
                entries: Vec::new(),
            },
        )?;

        let applied =
            requeued_count > 0 || had_counters || snapshot_had_state || lock_clear_result.cleared;
        Ok(RuntimeControlActionResult::direct(
            MailboxCommand::ClearStaleState,
            applied,
            format!(
                "cleared stale runtime state; requeued={requeued_count}; runtime_ownership_lock={}",
                lock_clear_result.reason
            ),
        ))
    }

    fn reload_config_direct(
        &self,
        _snapshot: RuntimeSnapshot,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        Ok(RuntimeControlActionResult::direct(
            MailboxCommand::ReloadConfig,
            false,
            "no daemon running; reload request not enqueued",
        ))
    }

    fn add_task_direct(
        &self,
        mut snapshot: RuntimeSnapshot,
        document: &TaskDocument,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let queue = QueueStore::from_paths(self.paths.clone());
        let destination = queue.enqueue_task(document)?;
        let depth = count_markdown_files(&self.paths.tasks_queue_dir)?;
        set_queue_depth(&mut snapshot, Plane::Execution, depth);
        snapshot.updated_at = now_timestamp("updated_at")?;
        save_snapshot(&self.paths, &snapshot)?;
        Ok(RuntimeControlActionResult::direct_artifact(
            MailboxCommand::AddTask,
            "task queued directly",
            destination,
        ))
    }

    fn add_spec_direct(
        &self,
        mut snapshot: RuntimeSnapshot,
        document: &SpecDocument,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let queue = QueueStore::from_paths(self.paths.clone());
        let destination = queue.enqueue_spec(document)?;
        let depth = planning_queue_depth(&self.paths)?;
        set_queue_depth(&mut snapshot, Plane::Planning, depth);
        snapshot.updated_at = now_timestamp("updated_at")?;
        save_snapshot(&self.paths, &snapshot)?;
        Ok(RuntimeControlActionResult::direct_artifact(
            MailboxCommand::AddSpec,
            "spec queued directly",
            destination,
        ))
    }

    fn add_probe_direct(
        &self,
        mut snapshot: RuntimeSnapshot,
        document: &ProbeDocument,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let queue = QueueStore::from_paths(self.paths.clone());
        let destination = queue.enqueue_probe(document)?;
        let depth = planning_queue_depth(&self.paths)?;
        set_queue_depth(&mut snapshot, Plane::Planning, depth);
        snapshot.updated_at = now_timestamp("updated_at")?;
        save_snapshot(&self.paths, &snapshot)?;
        Ok(RuntimeControlActionResult::direct_artifact(
            MailboxCommand::AddProbe,
            "probe queued directly",
            destination,
        ))
    }

    fn add_idea_direct(
        &self,
        mut snapshot: RuntimeSnapshot,
        payload: &MailboxAddIdeaPayload,
    ) -> RuntimeControlResult<RuntimeControlActionResult> {
        let destination = self
            .paths
            .root
            .join("ideas")
            .join("inbox")
            .join(&payload.source_name);
        if destination.exists() {
            return Err(RuntimeControlError::InvalidField {
                field_name: "source_name",
                message: format!("idea document already exists: {}", destination.display()),
            });
        }
        atomic_write_text(&destination, &payload.markdown)?;
        let depth = planning_queue_depth(&self.paths)?;
        set_queue_depth(&mut snapshot, Plane::Planning, depth);
        snapshot.updated_at = now_timestamp("updated_at")?;
        save_snapshot(&self.paths, &snapshot)?;
        Ok(RuntimeControlActionResult::direct_artifact(
            MailboxCommand::AddIdea,
            "idea staged directly",
            destination,
        ))
    }

    fn reset_runtime_to_idle(
        &self,
        mut snapshot: RuntimeSnapshot,
        process_running: bool,
        clear_stop_requested: bool,
        clear_paused: bool,
    ) -> RuntimeControlResult<()> {
        snapshot.process_running = process_running;
        clear_active_projection(&mut snapshot);
        snapshot.current_failure_class = None;
        snapshot.troubleshoot_attempt_count = 0;
        snapshot.mechanic_attempt_count = 0;
        snapshot.fix_cycle_count = 0;
        snapshot.consultant_invocations = 0;
        set_status_marker(&mut snapshot, Plane::Execution, IDLE_STATUS_MARKER);
        set_status_marker(&mut snapshot, Plane::Planning, IDLE_STATUS_MARKER);
        set_status_marker(&mut snapshot, Plane::Learning, IDLE_STATUS_MARKER);
        set_queue_depth(
            &mut snapshot,
            Plane::Execution,
            count_markdown_files(&self.paths.tasks_queue_dir)?,
        );
        set_queue_depth(
            &mut snapshot,
            Plane::Planning,
            planning_queue_depth(&self.paths)?,
        );
        set_queue_depth(
            &mut snapshot,
            Plane::Learning,
            count_markdown_files(&self.paths.learning_requests_queue_dir)?,
        );
        if clear_paused {
            snapshot.paused = false;
            snapshot.pause_sources.clear();
        }
        if clear_stop_requested {
            snapshot.stop_requested = false;
        }
        snapshot.updated_at = now_timestamp("updated_at")?;

        save_snapshot(&self.paths, &snapshot)?;
        set_execution_status(&self.paths, IDLE_STATUS_MARKER)?;
        set_planning_status(&self.paths, IDLE_STATUS_MARKER)?;
        set_learning_status(&self.paths, IDLE_STATUS_MARKER)?;
        Ok(())
    }

    fn clear_retry_active_run(
        &self,
        mut snapshot: RuntimeSnapshot,
        plane: Plane,
    ) -> RuntimeControlResult<()> {
        snapshot.active_runs_by_plane.remove(&plane);
        if snapshot.active_runs_by_plane.is_empty() {
            self.reset_runtime_to_idle(snapshot, false, false, false)?;
            return Ok(());
        }

        project_foreground_active_run(&mut snapshot)?;
        snapshot.current_failure_class = None;
        snapshot.updated_at = now_timestamp("updated_at")?;
        save_snapshot(&self.paths, &snapshot)?;
        match plane {
            Plane::Execution => {
                set_execution_status(&self.paths, IDLE_STATUS_MARKER)?;
            }
            Plane::Planning => {
                set_planning_status(&self.paths, IDLE_STATUS_MARKER)?;
            }
            Plane::Learning => {
                set_learning_status(&self.paths, IDLE_STATUS_MARKER)?;
            }
        }
        Ok(())
    }

    fn requeue_all_active_items(
        &self,
        queue: &QueueStore,
        reason: &str,
    ) -> RuntimeControlResult<usize> {
        let mut requeued_count = 0;
        for task_id in markdown_stems(&self.paths.tasks_active_dir)? {
            if ignore_invalid_state(queue.requeue_task(&task_id, reason))? {
                requeued_count += 1;
            }
        }
        for spec_id in markdown_stems(&self.paths.specs_active_dir)? {
            if ignore_invalid_state(queue.requeue_spec(&spec_id, reason))? {
                requeued_count += 1;
            }
        }
        for probe_id in markdown_stems(&self.paths.probes_active_dir)? {
            if ignore_invalid_state(queue.requeue_probe(&probe_id, reason))? {
                requeued_count += 1;
            }
        }
        for incident_id in markdown_stems(&self.paths.incidents_active_dir)? {
            if ignore_invalid_state(queue.requeue_incident(&incident_id, reason))? {
                requeued_count += 1;
            }
        }
        for learning_request_id in markdown_stems(&self.paths.learning_requests_active_dir)? {
            if ignore_invalid_state(queue.requeue_learning_request(&learning_request_id, reason))? {
                requeued_count += 1;
            }
        }
        Ok(requeued_count)
    }
}

/// Validate and write one mailbox command envelope into incoming mailbox storage.
pub fn write_mailbox_command(
    paths: &WorkspacePaths,
    envelope: &MailboxCommandEnvelope,
) -> RuntimeControlResult<PathBuf> {
    let mut validated = envelope.clone();
    validated.validate_contract().map_err(|source| {
        RuntimeControlError::runtime_json(paths.mailbox_incoming_dir.clone(), source)
    })?;
    let filename = mailbox_command_filename(&validated.command_id)?;
    let destination = paths.mailbox_incoming_dir.join(filename);
    if destination.exists() {
        return Err(RuntimeControlError::MailboxCommandExists { path: destination });
    }
    let mut payload = serde_json::to_string_pretty(&validated).map_err(|error| {
        RuntimeControlError::JsonRender {
            path: destination.clone(),
            message: error.to_string(),
        }
    })?;
    payload.push('\n');
    atomic_write_text(&destination, &payload)?;
    Ok(destination)
}

fn validate_issuer(issuer: &str) -> RuntimeControlResult<()> {
    if issuer.trim().is_empty() {
        return Err(RuntimeControlError::invalid_field(
            "issuer",
            "must be a non-empty string",
        ));
    }
    Ok(())
}

fn payload_map<T>(path: &Path, payload: &T) -> RuntimeControlResult<Map<String, Value>>
where
    T: Serialize,
{
    match serde_json::to_value(payload).map_err(|error| RuntimeControlError::JsonRender {
        path: path.to_path_buf(),
        message: error.to_string(),
    })? {
        Value::Object(map) => Ok(map),
        _ => Err(RuntimeControlError::InvalidField {
            field_name: "payload",
            message: "must serialize to a JSON object".to_owned(),
        }),
    }
}

fn reason_payload(reason: &str) -> Map<String, Value> {
    let mut payload = Map::new();
    payload.insert("reason".to_owned(), Value::String(reason.to_owned()));
    payload
}

fn normalized_reason(reason: &str, default: &str) -> String {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        default.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn mailbox_detail(command: MailboxCommand) -> &'static str {
    if command == MailboxCommand::ReloadConfig {
        "queued for daemon processing on the next runtime tick"
    } else {
        "queued for daemon processing"
    }
}

fn current_timestamp(field_name: &'static str) -> RuntimeControlResult<(Timestamp, i128)> {
    let now = OffsetDateTime::now_utc();
    let timestamp = timestamp_from_offset(field_name, now)?;
    Ok((timestamp, now.unix_timestamp_nanos() / 1_000_000))
}

fn now_timestamp(field_name: &'static str) -> RuntimeControlResult<Timestamp> {
    timestamp_from_offset(field_name, OffsetDateTime::now_utc())
}

fn timestamp_from_offset(
    field_name: &'static str,
    value: OffsetDateTime,
) -> RuntimeControlResult<Timestamp> {
    let rendered = value
        .format(&Rfc3339)
        .map_err(|error| RuntimeControlError::invalid_field(field_name, error.to_string()))?;
    Timestamp::parse(field_name, &rendered)
        .map_err(|error| RuntimeControlError::invalid_field(field_name, error.to_string()))
}

fn command_id(command: MailboxCommand, timestamp_ms: i128) -> String {
    let counter = COMMAND_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{timestamp_ms}-{}-{counter:08x}",
        command.as_str(),
        process::id()
    )
}

fn mailbox_command_filename(command_id: &str) -> RuntimeControlResult<String> {
    let safe = command_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let safe = safe.trim_matches('.');
    if safe.is_empty() {
        return Err(RuntimeControlError::invalid_field(
            "command_id",
            "must include at least one filename-safe character",
        ));
    }
    Ok(format!("{safe}.json"))
}

fn has_pause_source(snapshot: &RuntimeSnapshot, source: PauseSource) -> bool {
    snapshot.pause_sources.contains(&source)
}

fn ordered_pause_sources<I>(sources: I) -> Vec<PauseSource>
where
    I: IntoIterator<Item = PauseSource>,
{
    let mut has_operator = false;
    let mut has_usage = false;
    for source in sources {
        match source {
            PauseSource::Operator => has_operator = true,
            PauseSource::UsageGovernance => has_usage = true,
        }
    }

    let mut ordered = Vec::new();
    if has_operator {
        ordered.push(PauseSource::Operator);
    }
    if has_usage {
        ordered.push(PauseSource::UsageGovernance);
    }
    ordered
}

fn set_queue_depth(snapshot: &mut RuntimeSnapshot, plane: Plane, depth: u64) {
    match plane {
        Plane::Execution => snapshot.queue_depth_execution = depth,
        Plane::Planning => snapshot.queue_depth_planning = depth,
        Plane::Learning => snapshot.queue_depth_learning = depth,
    }
    snapshot.queue_depths_by_plane.insert(plane, depth);
}

fn set_status_marker(snapshot: &mut RuntimeSnapshot, plane: Plane, marker: &str) {
    match plane {
        Plane::Execution => snapshot.execution_status_marker = marker.to_owned(),
        Plane::Planning => snapshot.planning_status_marker = marker.to_owned(),
        Plane::Learning => snapshot.learning_status_marker = marker.to_owned(),
    }
    snapshot
        .status_markers_by_plane
        .insert(plane, marker.to_owned());
}

fn clear_active_projection(snapshot: &mut RuntimeSnapshot) {
    snapshot.active_plane = None;
    snapshot.active_stage = None;
    snapshot.active_node_id = None;
    snapshot.active_stage_kind_id = None;
    snapshot.active_run_id = None;
    snapshot.active_work_item_kind = None;
    snapshot.active_work_item_id = None;
    snapshot.active_runs_by_plane.clear();
    snapshot.active_since = None;
}

fn active_run_for_plane(snapshot: &RuntimeSnapshot, plane: Plane) -> Option<ActiveRunState> {
    snapshot
        .active_runs_by_plane
        .get(&plane)
        .cloned()
        .or_else(|| legacy_active_run_for_plane(snapshot, plane))
}

fn legacy_active_run_for_plane(snapshot: &RuntimeSnapshot, plane: Plane) -> Option<ActiveRunState> {
    if snapshot.active_plane != Some(plane) {
        return None;
    }
    let active_stage = snapshot.active_stage?;
    let active_run_id = snapshot.active_run_id.clone()?;
    let active_since = snapshot.active_since.clone()?;
    let active_work_item_kind = snapshot.active_work_item_kind?;
    let active_work_item_id = snapshot.active_work_item_id.clone()?;
    Some(ActiveRunState {
        plane,
        stage: active_stage,
        node_id: snapshot
            .active_node_id
            .clone()
            .unwrap_or_else(|| active_stage.as_str().to_owned()),
        stage_kind_id: snapshot
            .active_stage_kind_id
            .clone()
            .unwrap_or_else(|| active_stage.as_str().to_owned()),
        run_id: active_run_id,
        request_kind: if active_work_item_kind == WorkItemKind::LearningRequest {
            crate::contracts::ActiveRunRequestKind::LearningRequest
        } else {
            crate::contracts::ActiveRunRequestKind::ActiveWorkItem
        },
        work_item_kind: Some(active_work_item_kind),
        work_item_id: Some(active_work_item_id),
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        active_since,
        running_status_marker: None,
    })
}

enum RetryActiveRunSelection {
    Selected(ActiveRunState),
    Missing,
    Multiple,
}

fn retry_active_run(snapshot: &RuntimeSnapshot, scope: Option<Plane>) -> RetryActiveRunSelection {
    if let Some(scope) = scope {
        return active_run_for_plane(snapshot, scope)
            .map(RetryActiveRunSelection::Selected)
            .unwrap_or(RetryActiveRunSelection::Missing);
    }
    if snapshot.active_runs_by_plane.len() > 1 {
        return RetryActiveRunSelection::Multiple;
    }
    if snapshot.active_runs_by_plane.len() == 1 {
        return RetryActiveRunSelection::Selected(
            snapshot
                .active_runs_by_plane
                .values()
                .next()
                .expect("len checked")
                .clone(),
        );
    }
    if let Some(active_plane) = snapshot.active_plane {
        return active_run_for_plane(snapshot, active_plane)
            .map(RetryActiveRunSelection::Selected)
            .unwrap_or(RetryActiveRunSelection::Missing);
    }
    RetryActiveRunSelection::Missing
}

fn retry_active_missing_detail(snapshot: &RuntimeSnapshot, scope: Option<Plane>) -> String {
    if let Some(scope) = scope {
        return format!(
            "{} retry requires matching active plane; current active planes are {}",
            scope.as_str(),
            active_planes_label(snapshot)
        );
    }
    "no active work item to retry".to_owned()
}

fn active_planes_label(snapshot: &RuntimeSnapshot) -> String {
    let mut active_planes = Vec::new();
    for plane in [Plane::Execution, Plane::Planning, Plane::Learning] {
        if active_run_for_plane(snapshot, plane).is_some() {
            active_planes.push(plane.as_str());
        }
    }
    if active_planes.is_empty() {
        "none".to_owned()
    } else {
        active_planes.join(", ")
    }
}

fn project_foreground_active_run(snapshot: &mut RuntimeSnapshot) -> RuntimeControlResult<()> {
    let active_run = [Plane::Planning, Plane::Execution, Plane::Learning]
        .into_iter()
        .find_map(|plane| snapshot.active_runs_by_plane.get(&plane).cloned())
        .ok_or_else(|| {
            RuntimeControlError::invalid_field("active_runs_by_plane", "cannot be empty")
        })?;
    snapshot.active_plane = Some(active_run.plane);
    snapshot.active_stage = Some(active_run.stage);
    snapshot.active_node_id = Some(active_run.node_id);
    snapshot.active_stage_kind_id = Some(active_run.stage_kind_id);
    snapshot.active_run_id = Some(active_run.run_id);
    snapshot.active_work_item_kind = active_run.work_item_kind;
    snapshot.active_work_item_id = active_run.work_item_id;
    snapshot.active_since = Some(active_run.active_since);
    Ok(())
}

fn requeue_active_item(
    queue: &QueueStore,
    work_item_kind: WorkItemKind,
    work_item_id: &str,
    reason: &str,
) -> Result<PathBuf, QueueStoreError> {
    match work_item_kind {
        WorkItemKind::Task => queue.requeue_task(work_item_id, reason),
        WorkItemKind::Probe => queue.requeue_probe(work_item_id, reason),
        WorkItemKind::Spec => queue.requeue_spec(work_item_id, reason),
        WorkItemKind::Incident => queue.requeue_incident(work_item_id, reason),
        WorkItemKind::LearningRequest => queue.requeue_learning_request(work_item_id, reason),
    }
}

fn ignore_invalid_state(result: Result<PathBuf, QueueStoreError>) -> RuntimeControlResult<bool> {
    match result {
        Ok(_) => Ok(true),
        Err(QueueStoreError::InvalidState { .. }) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn count_markdown_files(directory: &Path) -> RuntimeControlResult<u64> {
    let entries =
        fs::read_dir(directory).map_err(|error| RuntimeControlError::io(directory, error))?;
    let mut count = 0;
    for entry in entries {
        let entry = entry.map_err(|error| RuntimeControlError::io(directory, error))?;
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "md") {
            let file_type = entry
                .file_type()
                .map_err(|error| RuntimeControlError::io(path.as_path(), error))?;
            if file_type.is_file() {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn planning_queue_depth(paths: &WorkspacePaths) -> RuntimeControlResult<u64> {
    Ok(count_markdown_files(&paths.probes_queue_dir)?
        + count_markdown_files(&paths.specs_queue_dir)?
        + count_markdown_files(&paths.incidents_incoming_dir)?)
}

fn markdown_stems(directory: &Path) -> RuntimeControlResult<Vec<String>> {
    let entries =
        fs::read_dir(directory).map_err(|error| RuntimeControlError::io(directory, error))?;
    let mut stems = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| RuntimeControlError::io(directory, error))?;
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "md") {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| RuntimeControlError::io(path.as_path(), error))?;
        if !file_type.is_file() {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                RuntimeControlError::invalid_field(
                    "active_item_path",
                    format!("path has no UTF-8 file stem: {}", path.display()),
                )
            })?;
        stems.push(stem.to_owned());
    }
    stems.sort();
    Ok(stems)
}
