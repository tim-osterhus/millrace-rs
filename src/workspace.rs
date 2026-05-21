//! Workspace paths and explicit initialization defaults.

use std::{
    collections::BTreeMap,
    env, fmt, fs, io,
    path::{Path, PathBuf},
};

use serde_json::json;

use crate::contracts::validate_safe_identifier;

#[path = "workspace/blueprint_state.rs"]
mod blueprint_state;
#[path = "workspace/doctor.rs"]
mod doctor;
#[path = "workspace/lineage_repair.rs"]
mod lineage_repair;
#[path = "workspace/managed_assets.rs"]
mod managed_assets;
#[path = "workspace/queue_claims.rs"]
mod queue_claims;
#[path = "workspace/queue_lifecycle.rs"]
mod queue_lifecycle;
#[path = "workspace/queue_store.rs"]
mod queue_store;
#[path = "workspace/runtime_control.rs"]
mod runtime_control;
#[path = "workspace/runtime_lock.rs"]
mod runtime_lock;
#[path = "workspace/schema_epoch.rs"]
mod schema_epoch;
#[path = "workspace/state_store.rs"]
mod state_store;
#[path = "workspace/task_lifecycle_integrity.rs"]
mod task_lifecycle_integrity;
#[path = "workspace/work_item_adapters.rs"]
mod work_item_adapters;

pub use blueprint_state::{
    approve_active_blueprint_draft, block_active_blueprint_draft, blueprint_artifact_ref,
    blueprint_manifest_path, cancel_blueprint_draft, claim_next_blueprint_draft,
    enqueue_blueprint_draft, list_blueprint_manifests, list_blueprint_manifests_for_root,
    list_open_blueprint_lineage_work_ids, list_open_blueprint_lineage_work_refs,
    move_candidate_blueprint_packet, persist_blueprint_critique, persist_blueprint_evaluation,
    persist_blueprint_packet, persist_blueprint_promotion, read_active_blueprint_draft,
    read_blueprint_draft, read_blueprint_manifest, requeue_active_blueprint_draft,
    resolve_blueprint_manifest_path, update_active_blueprint_draft, write_blueprint_manifest,
};
pub use doctor::{DoctorIssue, DoctorReport, run_workspace_doctor, run_workspace_doctor_for_paths};
pub use lineage_repair::{
    ClosureLineageRepairOutcome, LineageDiagnosticReason, LineageDriftDiagnostic,
    LineageDriftFinding, LineageRepairChange, LineageRepairError, LineageRepairPlan,
    LineageRepairResult, LineageWorkState, apply_lineage_repair_plan, build_lineage_repair_plan,
    build_lineage_repair_plan_at, closure_target_state_path, known_root_spec_aliases,
    lineage_drift_diagnostic_path, load_closure_target_state, refresh_lineage_repair_snapshot,
    repair_closure_lineage, save_closure_target_state, scan_closure_lineage_drift,
    scan_closure_lineage_drift_at, write_closure_lineage_repaired_event,
    write_lineage_drift_diagnostic, write_lineage_repair_report,
};
pub use managed_assets::{
    BaselineManifest, BaselineManifestEntry, BaselineUpgradeEntry, BaselineUpgradePreview,
    UpgradeDisposition, apply_baseline_upgrade, build_baseline_manifest,
    build_baseline_manifest_from_source, deploy_runtime_assets, deploy_runtime_assets_from_source,
    load_baseline_manifest, preview_baseline_upgrade, should_skip_runtime_asset_path,
    write_baseline_manifest,
};
pub use queue_claims::QueueClaim;
pub use queue_lifecycle::{
    QueueLifecycleInterpreter, SourceLifecycleAction, SourceLifecycleIntent,
    apply_source_lifecycle_intent, requeue_active_work_item, requeue_all_active_work_items,
};
pub use queue_store::{
    OperatorInterventionAction, OperatorInterventionContext, OperatorInterventionRecord,
    OperatorInterventionResult, QueueInspectionEntry, QueueStore, QueueStoreError,
    QueueStoreResult, StaleActiveState, archive_blocked_task, archive_invalid_incident_artifact,
    cancel_incident, cancel_work_item, claim_next_execution_task, claim_next_learning_request,
    claim_next_planning_item, detect_execution_stale_state, detect_learning_stale_state,
    detect_planning_stale_state, enqueue_incident, enqueue_learning_request, enqueue_probe,
    enqueue_spec, enqueue_task, find_queue_item, inspect_queue_items, list_deferred_root_spec_ids,
    mark_incident_blocked, mark_incident_resolved, mark_learning_request_blocked,
    mark_learning_request_done, mark_probe_blocked, mark_probe_done, mark_spec_blocked,
    mark_spec_done, mark_task_blocked, mark_task_done, requeue_blocked_task, requeue_incident,
    requeue_learning_request, requeue_probe, requeue_spec, requeue_task,
    resolve_incident_by_operator, retarget_queued_task_dependency, supersede_task,
};
pub use runtime_control::{
    RuntimeControl, RuntimeControlActionResult, RuntimeControlError, RuntimeControlMode,
    RuntimeControlResult, write_mailbox_command,
};
pub use runtime_lock::{
    ClearRuntimeOwnershipLockResult, RuntimeOwnershipLockError, RuntimeOwnershipLockOptions,
    RuntimeOwnershipLockResult, RuntimeOwnershipLockState, RuntimeOwnershipLockStatus,
    RuntimeOwnershipRecord, acquire_runtime_ownership_lock,
    acquire_runtime_ownership_lock_with_options, clear_stale_runtime_ownership_lock,
    clear_stale_runtime_ownership_lock_with_pid_checker, inspect_runtime_ownership_lock,
    inspect_runtime_ownership_lock_with_pid_checker, release_runtime_ownership_lock,
};
pub use schema_epoch::{
    CURRENT_WORKSPACE_SCHEMA_EPOCH, SchemaArchiveResetOptions, SchemaArchiveResetResult,
    SchemaEpochError, WorkspaceSchemaEpochMarker, archive_reset_workspace_schema,
    archive_reset_workspace_schema_with_options, default_workspace_schema_epoch_marker_payload,
    ensure_workspace_schema_epoch_current, load_workspace_schema_epoch_marker,
    workspace_schema_epoch_marker_path, write_workspace_schema_epoch_marker,
};
pub use state_store::{
    StateStore, StateStoreError, StateStoreResult, append_usage_governance_ledger_entry,
    atomic_write_text, increment_troubleshoot_attempt, load_execution_status, load_learning_status,
    load_planning_status, load_recovery_counters, load_snapshot, load_usage_governance_ledger,
    load_usage_governance_state, normalize_status_marker, reset_forward_progress_counters,
    save_recovery_counters, save_snapshot, save_usage_governance_state, set_execution_status,
    set_learning_status, set_planning_status,
};
pub use task_lifecycle_integrity::{
    TaskLifecycleDuplicate, find_duplicate_task_lifecycle_ids,
    retire_stale_blocked_task_duplicate_after_done,
};
pub use work_item_adapters::{
    AdapterParsedDocument, WorkItemDocumentAdapter, adapter_for_family_id, adapter_for_kind,
    builtin_work_item_adapters, enqueue_rendered_with_adapter, parse_with_adapter,
    validate_filename_with_adapter,
};

/// Result type for workspace filesystem operations.
pub type WorkspaceResult<T> = Result<T, WorkspaceError>;

/// Failures produced while resolving or initializing a workspace.
#[derive(Debug)]
pub enum WorkspaceError {
    /// The target does not have the canonical initialized Millrace baseline.
    Uninitialized {
        /// Workspace root that was checked.
        root: PathBuf,
    },
    /// A filesystem operation failed.
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Underlying IO error message.
        message: String,
    },
    /// A default JSON payload could not be rendered.
    Json {
        /// Payload being rendered.
        artifact: &'static str,
        /// Serde error message.
        message: String,
    },
    /// A workspace-managed relative path was malformed.
    InvalidPath {
        /// Path value involved in the failure.
        path: PathBuf,
        /// Validation error message.
        message: String,
    },
    /// A managed baseline upgrade could not be completed safely.
    Upgrade {
        /// Upgrade failure message.
        message: String,
    },
}

impl WorkspaceError {
    fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uninitialized { root } => {
                write!(
                    f,
                    "workspace is not initialized: {}. Run `millrace init --workspace {}` first.",
                    root.display(),
                    root.display()
                )
            }
            Self::Io { path, message } => {
                write!(
                    f,
                    "workspace filesystem error at {}: {message}",
                    path.display()
                )
            }
            Self::Json { artifact, message } => {
                write!(f, "failed to render default {artifact}: {message}")
            }
            Self::InvalidPath { path, message } => {
                write!(
                    f,
                    "invalid workspace-managed path {}: {message}",
                    path.display()
                )
            }
            Self::Upgrade { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for WorkspaceError {}

/// Resolved canonical workspace paths rooted at one workspace directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePaths {
    /// Requested workspace root.
    pub root: PathBuf,
    /// Runtime-owned Millrace workspace root, `<workspace>/millrace-agents`.
    pub runtime_root: PathBuf,

    /// Runtime state directory.
    pub state_dir: PathBuf,
    /// Mailbox root directory.
    pub mailbox_dir: PathBuf,
    /// Incoming mailbox command directory.
    pub mailbox_incoming_dir: PathBuf,
    /// Processed mailbox command directory.
    pub mailbox_processed_dir: PathBuf,
    /// Failed mailbox command directory.
    pub mailbox_failed_dir: PathBuf,

    /// Runtime run artifact directory.
    pub runs_dir: PathBuf,

    /// Execution capability approval root directory.
    pub approvals_dir: PathBuf,
    /// Pending execution capability approval directory.
    pub approvals_pending_dir: PathBuf,
    /// Resolved execution capability approval directory.
    pub approvals_resolved_dir: PathBuf,

    /// Execution tasks root directory.
    pub tasks_dir: PathBuf,
    /// Queued task directory.
    pub tasks_queue_dir: PathBuf,
    /// Active task directory.
    pub tasks_active_dir: PathBuf,
    /// Done task directory.
    pub tasks_done_dir: PathBuf,
    /// Blocked task directory.
    pub tasks_blocked_dir: PathBuf,

    /// Planning specs root directory.
    pub specs_dir: PathBuf,
    /// Queued spec directory.
    pub specs_queue_dir: PathBuf,
    /// Active spec directory.
    pub specs_active_dir: PathBuf,
    /// Done spec directory.
    pub specs_done_dir: PathBuf,
    /// Blocked spec directory.
    pub specs_blocked_dir: PathBuf,

    /// Incident root directory.
    pub incidents_dir: PathBuf,
    /// Incoming incident directory.
    pub incidents_incoming_dir: PathBuf,
    /// Active incident directory.
    pub incidents_active_dir: PathBuf,
    /// Resolved incident directory.
    pub incidents_resolved_dir: PathBuf,
    /// Blocked incident directory.
    pub incidents_blocked_dir: PathBuf,

    /// Probe root directory.
    pub probes_dir: PathBuf,
    /// Queued probe directory.
    pub probes_queue_dir: PathBuf,
    /// Active probe directory.
    pub probes_active_dir: PathBuf,
    /// Done probe directory.
    pub probes_done_dir: PathBuf,
    /// Blocked probe directory.
    pub probes_blocked_dir: PathBuf,

    /// Runtime-owned intake artifact root directory.
    pub intake_dir: PathBuf,
    /// Durable runtime-owned source copies for idea intake.
    pub intake_ideas_dir: PathBuf,

    /// Recon artifact root directory.
    pub recon_dir: PathBuf,
    /// Durable recon packet directory.
    pub recon_packets_dir: PathBuf,
    /// Recon report directory.
    pub recon_reports_dir: PathBuf,

    /// Learning plane root directory.
    pub learning_dir: PathBuf,
    /// Learning request root directory.
    pub learning_requests_dir: PathBuf,
    /// Queued learning request directory.
    pub learning_requests_queue_dir: PathBuf,
    /// Active learning request directory.
    pub learning_requests_active_dir: PathBuf,
    /// Done learning request directory.
    pub learning_requests_done_dir: PathBuf,
    /// Blocked learning request directory.
    pub learning_requests_blocked_dir: PathBuf,
    /// Research packet directory.
    pub learning_research_packets_dir: PathBuf,
    /// Skill candidate directory.
    pub learning_skill_candidates_dir: PathBuf,
    /// Skill update candidate directory.
    pub learning_update_candidates_dir: PathBuf,
    /// Learning event ledger.
    pub learning_events_file: PathBuf,

    /// Arbiter root directory.
    pub arbiter_dir: PathBuf,
    /// Arbiter contracts root directory.
    pub arbiter_contracts_dir: PathBuf,
    /// Arbiter idea contract directory.
    pub arbiter_idea_contracts_dir: PathBuf,
    /// Arbiter root-spec contract directory.
    pub arbiter_root_spec_contracts_dir: PathBuf,
    /// Arbiter target directory.
    pub arbiter_targets_dir: PathBuf,
    /// Arbiter rubric directory.
    pub arbiter_rubrics_dir: PathBuf,
    /// Arbiter verdict directory.
    pub arbiter_verdicts_dir: PathBuf,
    /// Arbiter report directory.
    pub arbiter_reports_dir: PathBuf,

    /// Compatibility loop asset root.
    pub loops_dir: PathBuf,
    /// Execution loop asset directory.
    pub execution_loops_dir: PathBuf,
    /// Planning loop asset directory.
    pub planning_loops_dir: PathBuf,
    /// Learning loop asset directory.
    pub learning_loops_dir: PathBuf,
    /// Graph asset root.
    pub graphs_dir: PathBuf,
    /// Execution graph asset directory.
    pub execution_graphs_dir: PathBuf,
    /// Planning graph asset directory.
    pub planning_graphs_dir: PathBuf,
    /// Learning graph asset directory.
    pub learning_graphs_dir: PathBuf,
    /// Registry asset root.
    pub registry_dir: PathBuf,
    /// Stage-kind registry root.
    pub stage_kind_registry_dir: PathBuf,
    /// Execution stage-kind registry directory.
    pub execution_stage_kind_registry_dir: PathBuf,
    /// Planning stage-kind registry directory.
    pub planning_stage_kind_registry_dir: PathBuf,
    /// Learning stage-kind registry directory.
    pub learning_stage_kind_registry_dir: PathBuf,

    /// Mode asset directory.
    pub modes_dir: PathBuf,
    /// Runtime log directory.
    pub logs_dir: PathBuf,
    /// Entrypoint asset directory.
    pub entrypoints_dir: PathBuf,
    /// Skill asset directory.
    pub skills_dir: PathBuf,

    /// Workspace outline file.
    pub outline_file: PathBuf,
    /// Runtime history log file.
    pub historylog_file: PathBuf,
    /// Runtime configuration file.
    pub runtime_config_file: PathBuf,
    /// Execution plane status file.
    pub execution_status_file: PathBuf,
    /// Planning plane status file.
    pub planning_status_file: PathBuf,
    /// Learning plane status file.
    pub learning_status_file: PathBuf,
    /// Managed baseline manifest file.
    pub baseline_manifest_file: PathBuf,
    /// Runtime snapshot state file.
    pub runtime_snapshot_file: PathBuf,
    /// Recovery counters state file.
    pub recovery_counters_file: PathBuf,
    /// Persisted compiler-authoritative run plan.
    pub compiled_plan_file: PathBuf,
    /// Persisted compiler diagnostics.
    pub compile_diagnostics_file: PathBuf,
    /// Runtime error context file.
    pub runtime_error_context_file: PathBuf,
    /// Usage-governance state file.
    pub usage_governance_state_file: PathBuf,
    /// Usage-governance ledger file.
    pub usage_governance_ledger_file: PathBuf,
    /// Runtime daemon lock file.
    pub runtime_lock_file: PathBuf,
}

impl WorkspacePaths {
    /// Resolves canonical workspace paths from a root workspace directory.
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        workspace_paths(root)
    }

    /// Returns all directories that must exist for a canonical workspace.
    #[must_use]
    pub fn directories(&self) -> Vec<&Path> {
        vec![
            &self.runtime_root,
            &self.state_dir,
            &self.mailbox_dir,
            &self.mailbox_incoming_dir,
            &self.mailbox_processed_dir,
            &self.mailbox_failed_dir,
            &self.runs_dir,
            &self.approvals_dir,
            &self.approvals_pending_dir,
            &self.approvals_resolved_dir,
            &self.tasks_dir,
            &self.tasks_queue_dir,
            &self.tasks_active_dir,
            &self.tasks_done_dir,
            &self.tasks_blocked_dir,
            &self.specs_dir,
            &self.specs_queue_dir,
            &self.specs_active_dir,
            &self.specs_done_dir,
            &self.specs_blocked_dir,
            &self.incidents_dir,
            &self.incidents_incoming_dir,
            &self.incidents_active_dir,
            &self.incidents_resolved_dir,
            &self.incidents_blocked_dir,
            &self.probes_dir,
            &self.probes_queue_dir,
            &self.probes_active_dir,
            &self.probes_done_dir,
            &self.probes_blocked_dir,
            &self.intake_dir,
            &self.intake_ideas_dir,
            &self.recon_dir,
            &self.recon_packets_dir,
            &self.recon_reports_dir,
            &self.learning_dir,
            &self.learning_requests_dir,
            &self.learning_requests_queue_dir,
            &self.learning_requests_active_dir,
            &self.learning_requests_done_dir,
            &self.learning_requests_blocked_dir,
            &self.learning_research_packets_dir,
            &self.learning_skill_candidates_dir,
            &self.learning_update_candidates_dir,
            &self.arbiter_dir,
            &self.arbiter_contracts_dir,
            &self.arbiter_idea_contracts_dir,
            &self.arbiter_root_spec_contracts_dir,
            &self.arbiter_targets_dir,
            &self.arbiter_rubrics_dir,
            &self.arbiter_verdicts_dir,
            &self.arbiter_reports_dir,
            &self.loops_dir,
            &self.execution_loops_dir,
            &self.planning_loops_dir,
            &self.learning_loops_dir,
            &self.graphs_dir,
            &self.execution_graphs_dir,
            &self.planning_graphs_dir,
            &self.learning_graphs_dir,
            &self.registry_dir,
            &self.stage_kind_registry_dir,
            &self.execution_stage_kind_registry_dir,
            &self.planning_stage_kind_registry_dir,
            &self.learning_stage_kind_registry_dir,
            &self.modes_dir,
            &self.logs_dir,
            &self.entrypoints_dir,
            &self.skills_dir,
        ]
    }
}

/// Resolves canonical workspace paths from a root workspace directory.
#[must_use]
pub fn workspace_paths(root: impl AsRef<Path>) -> WorkspacePaths {
    let root = absolute_workspace_root(root.as_ref());
    let runtime_root = root.join("millrace-agents");
    let state_dir = runtime_root.join("state");
    let mailbox_dir = state_dir.join("mailbox");
    let approvals_dir = runtime_root.join("approvals");
    let tasks_dir = runtime_root.join("tasks");
    let specs_dir = runtime_root.join("specs");
    let incidents_dir = runtime_root.join("incidents");
    let probes_dir = runtime_root.join("probes");
    let intake_dir = runtime_root.join("intake");
    let recon_dir = runtime_root.join("recon");
    let learning_dir = runtime_root.join("learning");
    let learning_requests_dir = learning_dir.join("requests");
    let arbiter_dir = runtime_root.join("arbiter");
    let arbiter_contracts_dir = arbiter_dir.join("contracts");
    let loops_dir = runtime_root.join("loops");
    let graphs_dir = runtime_root.join("graphs");
    let registry_dir = runtime_root.join("registry");
    let stage_kind_registry_dir = registry_dir.join("stage_kinds");

    WorkspacePaths {
        root,
        runtime_root: runtime_root.clone(),
        state_dir: state_dir.clone(),
        mailbox_dir: mailbox_dir.clone(),
        mailbox_incoming_dir: mailbox_dir.join("incoming"),
        mailbox_processed_dir: mailbox_dir.join("processed"),
        mailbox_failed_dir: mailbox_dir.join("failed"),
        runs_dir: runtime_root.join("runs"),
        approvals_dir: approvals_dir.clone(),
        approvals_pending_dir: approvals_dir.join("pending"),
        approvals_resolved_dir: approvals_dir.join("resolved"),
        tasks_dir: tasks_dir.clone(),
        tasks_queue_dir: tasks_dir.join("queue"),
        tasks_active_dir: tasks_dir.join("active"),
        tasks_done_dir: tasks_dir.join("done"),
        tasks_blocked_dir: tasks_dir.join("blocked"),
        specs_dir: specs_dir.clone(),
        specs_queue_dir: specs_dir.join("queue"),
        specs_active_dir: specs_dir.join("active"),
        specs_done_dir: specs_dir.join("done"),
        specs_blocked_dir: specs_dir.join("blocked"),
        incidents_dir: incidents_dir.clone(),
        incidents_incoming_dir: incidents_dir.join("incoming"),
        incidents_active_dir: incidents_dir.join("active"),
        incidents_resolved_dir: incidents_dir.join("resolved"),
        incidents_blocked_dir: incidents_dir.join("blocked"),
        probes_dir: probes_dir.clone(),
        probes_queue_dir: probes_dir.join("queue"),
        probes_active_dir: probes_dir.join("active"),
        probes_done_dir: probes_dir.join("done"),
        probes_blocked_dir: probes_dir.join("blocked"),
        intake_dir: intake_dir.clone(),
        intake_ideas_dir: intake_dir.join("ideas"),
        recon_dir: recon_dir.clone(),
        recon_packets_dir: recon_dir.join("packets"),
        recon_reports_dir: recon_dir.join("reports"),
        learning_dir: learning_dir.clone(),
        learning_requests_dir: learning_requests_dir.clone(),
        learning_requests_queue_dir: learning_requests_dir.join("queue"),
        learning_requests_active_dir: learning_requests_dir.join("active"),
        learning_requests_done_dir: learning_requests_dir.join("done"),
        learning_requests_blocked_dir: learning_requests_dir.join("blocked"),
        learning_research_packets_dir: learning_dir.join("research-packets"),
        learning_skill_candidates_dir: learning_dir.join("skill-candidates"),
        learning_update_candidates_dir: learning_dir.join("update-candidates"),
        learning_events_file: learning_dir.join("events.jsonl"),
        arbiter_dir: arbiter_dir.clone(),
        arbiter_contracts_dir: arbiter_contracts_dir.clone(),
        arbiter_idea_contracts_dir: arbiter_contracts_dir.join("ideas"),
        arbiter_root_spec_contracts_dir: arbiter_contracts_dir.join("root-specs"),
        arbiter_targets_dir: arbiter_dir.join("targets"),
        arbiter_rubrics_dir: arbiter_dir.join("rubrics"),
        arbiter_verdicts_dir: arbiter_dir.join("verdicts"),
        arbiter_reports_dir: arbiter_dir.join("reports"),
        loops_dir: loops_dir.clone(),
        execution_loops_dir: loops_dir.join("execution"),
        planning_loops_dir: loops_dir.join("planning"),
        learning_loops_dir: loops_dir.join("learning"),
        graphs_dir: graphs_dir.clone(),
        execution_graphs_dir: graphs_dir.join("execution"),
        planning_graphs_dir: graphs_dir.join("planning"),
        learning_graphs_dir: graphs_dir.join("learning"),
        registry_dir: registry_dir.clone(),
        stage_kind_registry_dir: stage_kind_registry_dir.clone(),
        execution_stage_kind_registry_dir: stage_kind_registry_dir.join("execution"),
        planning_stage_kind_registry_dir: stage_kind_registry_dir.join("planning"),
        learning_stage_kind_registry_dir: stage_kind_registry_dir.join("learning"),
        modes_dir: runtime_root.join("modes"),
        logs_dir: runtime_root.join("logs"),
        entrypoints_dir: runtime_root.join("entrypoints"),
        skills_dir: runtime_root.join("skills"),
        outline_file: runtime_root.join("outline.md"),
        historylog_file: runtime_root.join("historylog.md"),
        runtime_config_file: runtime_root.join("millrace.toml"),
        execution_status_file: state_dir.join("execution_status.md"),
        planning_status_file: state_dir.join("planning_status.md"),
        learning_status_file: state_dir.join("learning_status.md"),
        baseline_manifest_file: state_dir.join("baseline_manifest.json"),
        runtime_snapshot_file: state_dir.join("runtime_snapshot.json"),
        recovery_counters_file: state_dir.join("recovery_counters.json"),
        compiled_plan_file: state_dir.join("compiled_plan.json"),
        compile_diagnostics_file: state_dir.join("compile_diagnostics.json"),
        runtime_error_context_file: state_dir.join("runtime_error_context.json"),
        usage_governance_state_file: state_dir.join("usage_governance_state.json"),
        usage_governance_ledger_file: state_dir.join("usage_governance_ledger.jsonl"),
        runtime_lock_file: state_dir.join("runtime_daemon.lock.json"),
    }
}

/// Return the durable runtime-owned source artifact path for one root idea id.
pub fn idea_source_artifact_path(
    paths: &WorkspacePaths,
    root_idea_id: &str,
) -> state_store::StateStoreResult<PathBuf> {
    validate_safe_identifier(root_idea_id, "root_idea_id").map_err(|error| {
        state_store::StateStoreError::from(WorkspaceError::InvalidPath {
            path: PathBuf::from(root_idea_id),
            message: error.to_string(),
        })
    })?;
    Ok(paths.intake_ideas_dir.join(format!("{root_idea_id}.md")))
}

/// Persist original idea markdown under runtime-owned durable intake storage.
pub fn write_idea_source_artifact(
    paths: &WorkspacePaths,
    root_idea_id: &str,
    markdown: &str,
) -> state_store::StateStoreResult<PathBuf> {
    let path = idea_source_artifact_path(paths, root_idea_id)?;
    state_store::atomic_write_text(&path, markdown)?;
    Ok(path)
}

/// Resolve a workspace and reject targets missing the canonical initialized baseline.
pub fn require_initialized_workspace(root: impl AsRef<Path>) -> WorkspaceResult<WorkspacePaths> {
    let paths = workspace_paths(root);
    require_initialized_workspace_paths(&paths)?;
    Ok(paths)
}

/// Reject resolved workspace paths missing the canonical initialized baseline.
pub fn require_initialized_workspace_paths(paths: &WorkspacePaths) -> WorkspaceResult<()> {
    let required_paths = [
        &paths.runtime_root,
        &paths.state_dir,
        &paths.tasks_queue_dir,
        &paths.probes_queue_dir,
        &paths.intake_ideas_dir,
        &paths.specs_queue_dir,
        &paths.incidents_incoming_dir,
        &paths.recon_packets_dir,
        &paths.learning_requests_queue_dir,
        &paths.entrypoints_dir,
        &paths.skills_dir,
        &paths.outline_file,
        &paths.historylog_file,
        &paths.baseline_manifest_file,
        &paths.runtime_config_file,
    ];

    if required_paths.iter().any(|path| !path.exists()) {
        return Err(WorkspaceError::Uninitialized {
            root: paths.root.clone(),
        });
    }

    Ok(())
}

/// Create the canonical workspace directory tree and missing bootstrap defaults.
pub fn initialize_workspace(root: impl AsRef<Path>) -> WorkspaceResult<WorkspacePaths> {
    let paths = workspace_paths(root);
    initialize_workspace_paths(&paths)?;
    Ok(paths)
}

/// Compatibility alias for creating a canonical workspace baseline.
pub fn bootstrap_workspace(root: impl AsRef<Path>) -> WorkspaceResult<WorkspacePaths> {
    initialize_workspace(root)
}

/// Create the canonical workspace directory tree and missing bootstrap defaults.
pub fn initialize_workspace_paths(paths: &WorkspacePaths) -> WorkspaceResult<()> {
    for directory in paths.directories() {
        fs::create_dir_all(directory).map_err(|error| WorkspaceError::io(directory, error))?;
    }

    for (file_path, payload) in default_file_payloads(paths)? {
        write_if_missing(&file_path, &payload)?;
    }

    deploy_runtime_assets(paths)?;
    if !paths.baseline_manifest_file.exists() {
        let manifest = build_baseline_manifest();
        write_baseline_manifest(paths, &manifest)?;
    }

    Ok(())
}

/// Return bootstrap-created file payloads keyed by their canonical path.
pub fn default_file_payloads(paths: &WorkspacePaths) -> WorkspaceResult<BTreeMap<PathBuf, String>> {
    let mut payloads = BTreeMap::new();
    payloads.insert(paths.outline_file.clone(), String::new());
    payloads.insert(paths.historylog_file.clone(), String::new());
    payloads.insert(
        paths.runtime_config_file.clone(),
        bootstrap_runtime_config(),
    );
    payloads.insert(paths.execution_status_file.clone(), idle_status_payload());
    payloads.insert(paths.planning_status_file.clone(), idle_status_payload());
    payloads.insert(paths.learning_status_file.clone(), idle_status_payload());
    payloads.insert(paths.learning_events_file.clone(), String::new());
    payloads.insert(
        paths.runtime_snapshot_file.clone(),
        default_runtime_snapshot_payload(paths)?,
    );
    payloads.insert(
        paths.recovery_counters_file.clone(),
        default_recovery_counters_payload()?,
    );
    payloads.insert(
        workspace_schema_epoch_marker_path(paths),
        default_workspace_schema_epoch_marker_payload()?,
    );
    Ok(payloads)
}

fn write_if_missing(path: &Path, payload: &str) -> WorkspaceResult<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| WorkspaceError::io(parent, error))?;
    }
    fs::write(path, payload).map_err(|error| WorkspaceError::io(path, error))
}

fn absolute_workspace_root(root: &Path) -> PathBuf {
    let expanded = expand_user(root);
    if expanded.is_absolute() {
        return expanded;
    }
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(expanded)
}

fn expand_user(path: &Path) -> PathBuf {
    let raw = path.as_os_str().to_string_lossy();
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        return path.to_path_buf();
    };

    if raw == "~" {
        home
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else {
        path.to_path_buf()
    }
}

fn idle_status_payload() -> String {
    "### IDLE\n".to_owned()
}

fn bootstrap_runtime_config() -> String {
    [
        "[runtime]",
        "default_mode = \"default_codex\"",
        "run_style = \"daemon\"",
        "",
        "[runners.codex]",
        "permission_default = \"maximum\"",
        "",
    ]
    .join("\n")
}

fn default_runtime_snapshot_payload(paths: &WorkspacePaths) -> WorkspaceResult<String> {
    let compiled_plan_path = paths
        .state_dir
        .join("compiled_plan.json")
        .strip_prefix(&paths.root)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "millrace-agents/state/compiled_plan.json".to_owned());
    let updated_at = "1970-01-01T00:00:00Z";
    let payload = json!({
        "schema_version": "1.0",
        "kind": "runtime_snapshot",
        "runtime_mode": "daemon",
        "process_running": false,
        "paused": false,
        "pause_sources": [],
        "stop_requested": false,
        "active_mode_id": "default_codex",
        "execution_loop_id": "execution.standard",
        "planning_loop_id": "planning.standard",
        "learning_loop_id": null,
        "loop_ids_by_plane": {
            "execution": "execution.standard",
            "planning": "planning.standard"
        },
        "compiled_plan_id": "bootstrap",
        "compiled_plan_path": compiled_plan_path,
        "active_plane": null,
        "active_stage": null,
        "active_node_id": null,
        "active_stage_kind_id": null,
        "active_run_id": null,
        "active_work_item_kind": null,
        "active_work_item_id": null,
        "active_runs_by_plane": {},
        "execution_status_marker": "### IDLE",
        "planning_status_marker": "### IDLE",
        "learning_status_marker": "### IDLE",
        "status_markers_by_plane": {
            "execution": "### IDLE",
            "planning": "### IDLE",
            "learning": "### IDLE"
        },
        "queue_depth_execution": 0,
        "queue_depth_planning": 0,
        "queue_depth_learning": 0,
        "queue_depths_by_plane": {
            "execution": 0,
            "planning": 0,
            "learning": 0
        },
        "last_terminal_result": null,
        "last_stage_result_path": null,
        "current_failure_class": null,
        "troubleshoot_attempt_count": 0,
        "mechanic_attempt_count": 0,
        "fix_cycle_count": 0,
        "consultant_invocations": 0,
        "config_version": "bootstrap",
        "watcher_mode": "off",
        "last_reload_outcome": null,
        "last_reload_error": null,
        "started_at": null,
        "active_since": null,
        "updated_at": updated_at
    });

    serde_json::to_string_pretty(&payload)
        .map(|mut rendered| {
            rendered.push('\n');
            rendered
        })
        .map_err(|error| WorkspaceError::Json {
            artifact: "runtime_snapshot",
            message: error.to_string(),
        })
}

fn default_recovery_counters_payload() -> WorkspaceResult<String> {
    let payload = json!({
        "schema_version": "1.0",
        "kind": "recovery_counters",
        "entries": []
    });

    serde_json::to_string_pretty(&payload)
        .map(|mut rendered| {
            rendered.push('\n');
            rendered
        })
        .map_err(|error| WorkspaceError::Json {
            artifact: "recovery_counters",
            message: error.to_string(),
        })
}
