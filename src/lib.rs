#![doc = include_str!("../README.md")]
#![recursion_limit = "256"]

/// The crates.io package name.
pub const PACKAGE_NAME: &str = "millrace-ai";

/// The Rust library crate name.
pub const CRATE_NAME: &str = "millrace_ai";

/// The command-line binary name installed by this package.
pub const CLI_NAME: &str = "millrace";

/// The current development status of the Rust runtime.
pub const STABILITY: &str = "experimental";

/// Typed runtime artifact contracts.
pub mod contracts;

/// Embedded managed runtime assets.
pub mod assets;

/// Workspace path resolution and initialization defaults.
pub mod workspace;

/// Headed markdown work-document parsing and rendering helpers.
pub mod work_documents;

/// Recon packet markdown parsing and rendering helpers.
pub mod recon_packets;

/// Compiler contract boundary and frozen-plan models.
pub mod compiler;

/// Serial runtime request contracts.
pub mod runtime;

/// Stage runner contracts and deterministic fake runner support.
pub mod runners;

/// Command-line parsing, dispatch, and rendering surface.
pub mod cli;

pub use runtime::{
    AllowedResultClassPolicy, AllowedResultClassesByOutcome, ApprovalStorageError,
    ApprovalStorageResult, AutoRecoveryConfig, BasicMonitorRenderer, BasicTerminalMonitor,
    CapabilityGateBlockedGrant, CapabilityGateResult, CapabilitySupportEvaluator,
    ExecutionCapabilitiesConfig, ExecutionCapabilityApproval, ExecutionCapabilityApprovalListing,
    ExecutionCapabilityApprovalRequest, ExecutionCapabilityApprovalStatus, NullRuntimeMonitorSink,
    RequestKind, RouterAction, RouterDecision, RunTraceError, RunTraceResult,
    RuntimeConfigApplyBoundary, RuntimeDaemonCycleOutcome, RuntimeDaemonLoopExitReason,
    RuntimeDaemonLoopOptions, RuntimeDaemonLoopOutcome, RuntimeDaemonSleeper,
    RuntimeDaemonSupervisor, RuntimeEffectApplication, RuntimeEffectDecision,
    RuntimeEffectFailurePolicyInput, RuntimeEffectResult, RuntimeFailurePolicyInterpretation,
    RuntimeFileFingerprint, RuntimeMonitorEvent, RuntimeMonitorFanout, RuntimeMonitorSink,
    RuntimePollWatcherState, RuntimeReconciliationSignal, RuntimeRunnersConfig, RuntimeStageConfig,
    RuntimeStartupConfig, RuntimeStartupError, RuntimeStartupOptions, RuntimeStartupReconciliation,
    RuntimeStartupResult, RuntimeStartupSession, RuntimeTickDispatchOutcome, RuntimeTickError,
    RuntimeTickOptions, RuntimeTickOutcome, RuntimeTickOutcomeKind, RuntimeTickResult,
    RuntimeTokenRuleConfig, RuntimeTokenRulesConfig, RuntimeWatchEvent, RuntimeWatcherSession,
    RuntimeWatcherTarget, StageCompletionOutcome, StageRunRequest, StageRunRequestError,
    StageWorkerOutcome, StageWorkerResult, SubscriptionQuotaRuleConfig,
    SubscriptionQuotaRulesConfig, ThreadRuntimeDaemonSleeper, UsageGovernanceConfig,
    apply_runtime_effect_for_stage_result, apply_stage_worker_outcome,
    approve_execution_capability_request, blocked_metadata_allows_auto_requeue,
    blocked_metadata_path, blocked_task_metadata_path, build_runtime_runner_dispatcher,
    build_runtime_runner_dispatcher_for_paths, build_runtime_watcher_session, can_dispatch_plane,
    capability_gate_failure_result, compiled_entry_node_for_work_item,
    deny_execution_capability_request, derive_run_trace_from_stage_results,
    disabled_usage_governance_state, ensure_execution_capability_approval,
    evaluate_runtime_token_rules, evaluate_stage_request_capabilities,
    evaluate_stage_request_capabilities_with_runner, evaluate_subscription_quota_rules,
    evaluate_usage_governance, find_approval_for_grant, find_stranded_blocked_dependency,
    healthy_subscription_quota_status, inspect_run_trace, inspect_run_trace_id,
    interpret_runtime_effect_failure_policy, ledger_entry_from_stage_result,
    list_execution_capability_approvals, load_blocked_item_metadata, load_blocked_task_metadata,
    load_runtime_startup_config, next_auto_resume_at, reconcile_usage_ledger_from_stage_results,
    record_capability_gate_result, record_router_decision_trace, record_stage_result_usage,
    render_stage_request_context_lines, run_runtime_daemon_loop,
    run_runtime_daemon_loop_with_monitor, run_runtime_daemon_supervisor_loop_with_sleeper,
    run_runtime_daemon_supervisor_loop_with_sleeper_and_monitor, run_serial_runtime_tick,
    run_serial_runtime_tick_with_runner, run_stage_worker, runtime_config_apply_boundary_for_field,
    runtime_monitor_events_from_jsonl, should_record_runtime_tokens, spawned_work_ref_from_path,
    stage_result_dedupe_key, startup_runtime_daemon, startup_runtime_daemon_for_paths,
    startup_runtime_once, startup_runtime_once_for_paths, subscription_quota_status_unavailable,
    trace_path_for_run_dir, upsert_stage_result_trace_node,
};

pub use runners::{
    CodexCliArtifactPaths, CodexCliConfig, CodexCliRunnerAdapter, CodexPermissionLevel,
    CodexProcessError, CodexProcessExecutor, CodexProcessRequest, FakeRunner, FakeRunnerConfig,
    FakeRunnerOutput, FakeRunnerResult, PiEventLogPolicy, PiRpcArtifactPaths,
    PiRpcClientCreateRequest, PiRpcClientError, PiRpcClientFactory, PiRpcConfig, PiRpcJsonlClient,
    PiRpcPromptClient, PiRpcRunnerAdapter, PiRpcSessionResult, PiRpcStreamEvent, PiRpcTransport,
    ProcessExecutionResult, ProcessExitKind, RunnerCompletionArtifact,
    RunnerCompletionArtifactContext, RunnerEnvironmentDelta, RunnerError, RunnerExitKind,
    RunnerInvocationArtifact, RunnerRawResult, RunnerRegistry, RunnerResult, StageRunnerAdapter,
    StageRunnerDispatcher, SubprocessCodexExecutor, SubprocessPiRpcClientFactory,
    SubprocessPiRpcTransport, build_codex_cli_command, build_pi_rpc_command, build_stage_prompt,
    codex_cli_artifact_paths, completion_artifact_from_raw_result, extract_token_usage,
    invocation_artifact_from_request, legal_terminal_markers, materialize_stdout_artifact,
    normalize_stage_result, permission_flags, persist_event_log, persistable_event_lines,
    pi_rpc_artifact_paths, reconciled_timeout_terminal_marker, resolve_permission_level,
    runner_prompt_path, should_persist_event_log, token_usage_from_line, token_usage_from_payload,
    token_usage_from_stats_payload, write_runner_completion, write_runner_invocation,
    write_stage_prompt_artifact,
};

pub use workspace::{
    AdapterParsedDocument, BaselineManifest, BaselineManifestEntry, BaselineUpgradeEntry,
    BaselineUpgradePreview, CURRENT_WORKSPACE_SCHEMA_EPOCH, ClearRuntimeOwnershipLockResult,
    ClosureLineageRepairOutcome, DoctorIssue, DoctorReport, LineageDiagnosticReason,
    LineageDriftDiagnostic, LineageDriftFinding, LineageRepairChange, LineageRepairError,
    LineageRepairPlan, LineageRepairResult, LineageWorkState, QueueClaim, QueueInspectionEntry,
    QueueLifecycleInterpreter, QueueStore, QueueStoreError, QueueStoreResult, RuntimeControl,
    RuntimeControlActionResult, RuntimeControlError, RuntimeControlMode, RuntimeControlResult,
    RuntimeOwnershipLockError, RuntimeOwnershipLockOptions, RuntimeOwnershipLockResult,
    RuntimeOwnershipLockState, RuntimeOwnershipLockStatus, RuntimeOwnershipRecord,
    SchemaArchiveResetOptions, SchemaArchiveResetResult, SchemaEpochError, SourceLifecycleAction,
    SourceLifecycleIntent, StaleActiveState, StateStore, StateStoreError, StateStoreResult,
    TaskLifecycleDuplicate, UpgradeDisposition, WorkItemDocumentAdapter, WorkspaceError,
    WorkspacePaths, WorkspaceResult, WorkspaceSchemaEpochMarker, acquire_runtime_ownership_lock,
    acquire_runtime_ownership_lock_with_options, adapter_for_family_id, adapter_for_kind,
    append_usage_governance_ledger_entry, apply_baseline_upgrade, apply_lineage_repair_plan,
    apply_source_lifecycle_intent, approve_active_blueprint_draft, archive_reset_workspace_schema,
    archive_reset_workspace_schema_with_options, atomic_write_text, block_active_blueprint_draft,
    blueprint_artifact_ref, blueprint_manifest_path, bootstrap_workspace, build_baseline_manifest,
    build_lineage_repair_plan, build_lineage_repair_plan_at, builtin_work_item_adapters,
    cancel_blueprint_draft, claim_next_blueprint_draft, claim_next_execution_task,
    claim_next_learning_request, claim_next_planning_item, clear_stale_runtime_ownership_lock,
    clear_stale_runtime_ownership_lock_with_pid_checker, closure_target_state_path,
    deploy_runtime_assets, detect_execution_stale_state, detect_learning_stale_state,
    detect_planning_stale_state, enqueue_blueprint_draft, enqueue_incident,
    enqueue_learning_request, enqueue_rendered_with_adapter, enqueue_spec, enqueue_task,
    ensure_workspace_schema_epoch_current, find_duplicate_task_lifecycle_ids, find_queue_item,
    idea_source_artifact_path, increment_troubleshoot_attempt, initialize_workspace,
    inspect_queue_items, inspect_runtime_ownership_lock,
    inspect_runtime_ownership_lock_with_pid_checker, known_root_spec_aliases,
    list_blueprint_manifests, list_blueprint_manifests_for_root,
    list_open_blueprint_lineage_work_ids, list_open_blueprint_lineage_work_refs,
    load_baseline_manifest, load_closure_target_state, load_execution_status, load_learning_status,
    load_planning_status, load_recovery_counters, load_snapshot, load_usage_governance_ledger,
    load_usage_governance_state, load_workspace_schema_epoch_marker, mark_incident_blocked,
    mark_incident_resolved, mark_learning_request_blocked, mark_learning_request_done,
    mark_spec_blocked, mark_spec_done, mark_task_blocked, mark_task_done,
    move_candidate_blueprint_packet, normalize_status_marker, parse_with_adapter,
    persist_blueprint_critique, persist_blueprint_evaluation, persist_blueprint_packet,
    persist_blueprint_promotion, preview_baseline_upgrade, read_active_blueprint_draft,
    read_blueprint_draft, read_blueprint_manifest, refresh_lineage_repair_snapshot,
    release_runtime_ownership_lock, repair_closure_lineage, requeue_active_blueprint_draft,
    requeue_active_work_item, requeue_all_active_work_items, requeue_blocked_task,
    requeue_incident, requeue_learning_request, requeue_spec, requeue_task,
    require_initialized_workspace, require_initialized_workspace_paths,
    reset_forward_progress_counters, resolve_blueprint_manifest_path,
    retire_stale_blocked_task_duplicate_after_done, run_workspace_doctor,
    run_workspace_doctor_for_paths, save_closure_target_state, save_recovery_counters,
    save_snapshot, save_usage_governance_state, scan_closure_lineage_drift,
    scan_closure_lineage_drift_at, set_execution_status, set_learning_status, set_planning_status,
    update_active_blueprint_draft, validate_filename_with_adapter, workspace_paths,
    workspace_schema_epoch_marker_path, write_baseline_manifest, write_blueprint_manifest,
    write_closure_lineage_repaired_event, write_idea_source_artifact,
    write_lineage_drift_diagnostic, write_lineage_repair_report, write_mailbox_command,
    write_workspace_schema_epoch_marker,
};

/// Basic metadata for the Rust implementation of Millrace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeStatus {
    /// The crates.io package name.
    pub package_name: &'static str,
    /// The Rust library crate name.
    pub crate_name: &'static str,
    /// The command-line binary name.
    pub cli_name: &'static str,
    /// The Cargo package version.
    pub version: &'static str,
    /// The implementation stability label.
    pub stability: &'static str,
}

impl RuntimeStatus {
    /// Returns metadata for the currently compiled package.
    #[must_use]
    pub const fn current() -> Self {
        Self {
            package_name: PACKAGE_NAME,
            crate_name: CRATE_NAME,
            cli_name: CLI_NAME,
            version: env!("CARGO_PKG_VERSION"),
            stability: STABILITY,
        }
    }
}

/// Returns metadata for the currently compiled package.
#[must_use]
pub const fn runtime_status() -> RuntimeStatus {
    RuntimeStatus::current()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_expected_names() {
        let status = runtime_status();

        assert_eq!(status.package_name, "millrace-ai");
        assert_eq!(status.crate_name, "millrace_ai");
        assert_eq!(status.cli_name, "millrace");
        assert_eq!(status.stability, "experimental");
    }
}
