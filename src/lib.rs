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

/// Compiler contract boundary and frozen-plan models.
pub mod compiler;

/// Serial runtime request contracts.
pub mod runtime;

/// Stage runner contracts and deterministic fake runner support.
pub mod runners;

/// Command-line parsing, dispatch, and rendering surface.
pub mod cli;

pub use runtime::{
    AllowedResultClassPolicy, AllowedResultClassesByOutcome, BasicMonitorRenderer,
    BasicTerminalMonitor, NullRuntimeMonitorSink, RequestKind, RouterAction, RouterDecision,
    RuntimeDaemonCycleOutcome, RuntimeDaemonLoopExitReason, RuntimeDaemonLoopOptions,
    RuntimeDaemonLoopOutcome, RuntimeDaemonSleeper, RuntimeDaemonSupervisor,
    RuntimeFileFingerprint, RuntimeMonitorEvent, RuntimeMonitorFanout, RuntimeMonitorSink,
    RuntimePollWatcherState, RuntimeReconciliationSignal, RuntimeRunnersConfig, RuntimeStageConfig,
    RuntimeStartupConfig, RuntimeStartupError, RuntimeStartupOptions, RuntimeStartupReconciliation,
    RuntimeStartupResult, RuntimeStartupSession, RuntimeTickDispatchOutcome, RuntimeTickError,
    RuntimeTickOptions, RuntimeTickOutcome, RuntimeTickOutcomeKind, RuntimeTickResult,
    RuntimeTokenRuleConfig, RuntimeTokenRulesConfig, RuntimeWatchEvent, RuntimeWatcherSession,
    RuntimeWatcherTarget, StageCompletionOutcome, StageRunRequest, StageRunRequestError,
    StageWorkerOutcome, StageWorkerResult, SubscriptionQuotaRuleConfig,
    SubscriptionQuotaRulesConfig, ThreadRuntimeDaemonSleeper, UsageGovernanceConfig,
    apply_stage_worker_outcome, build_runtime_runner_dispatcher,
    build_runtime_runner_dispatcher_for_paths, build_runtime_watcher_session, can_dispatch_plane,
    compiled_entry_node_for_work_item, disabled_usage_governance_state,
    evaluate_runtime_token_rules, evaluate_subscription_quota_rules, evaluate_usage_governance,
    healthy_subscription_quota_status, ledger_entry_from_stage_result, load_runtime_startup_config,
    next_auto_resume_at, reconcile_usage_ledger_from_stage_results, record_stage_result_usage,
    render_stage_request_context_lines, run_runtime_daemon_loop,
    run_runtime_daemon_loop_with_monitor, run_runtime_daemon_supervisor_loop_with_sleeper,
    run_runtime_daemon_supervisor_loop_with_sleeper_and_monitor, run_serial_runtime_tick,
    run_serial_runtime_tick_with_runner, run_stage_worker, runtime_monitor_events_from_jsonl,
    should_record_runtime_tokens, stage_result_dedupe_key, startup_runtime_daemon,
    startup_runtime_daemon_for_paths, startup_runtime_once, startup_runtime_once_for_paths,
    subscription_quota_status_unavailable,
};

pub use runners::{
    CodexCliArtifactPaths, CodexCliConfig, CodexCliRunnerAdapter, CodexPermissionLevel,
    CodexProcessError, CodexProcessExecutor, CodexProcessRequest, FakeRunner, FakeRunnerConfig,
    FakeRunnerOutput, FakeRunnerResult, PiEventLogPolicy, PiRpcArtifactPaths,
    PiRpcClientCreateRequest, PiRpcClientError, PiRpcClientFactory, PiRpcConfig, PiRpcJsonlClient,
    PiRpcPromptClient, PiRpcRunnerAdapter, PiRpcSessionResult, PiRpcStreamEvent, PiRpcTransport,
    ProcessExecutionResult, ProcessExitKind, RunnerCompletionArtifact, RunnerEnvironmentDelta,
    RunnerError, RunnerExitKind, RunnerInvocationArtifact, RunnerRawResult, RunnerRegistry,
    RunnerResult, StageRunnerAdapter, StageRunnerDispatcher, SubprocessCodexExecutor,
    SubprocessPiRpcClientFactory, SubprocessPiRpcTransport, build_codex_cli_command,
    build_pi_rpc_command, build_stage_prompt, codex_cli_artifact_paths,
    completion_artifact_from_raw_result, extract_token_usage, invocation_artifact_from_request,
    legal_terminal_markers, materialize_stdout_artifact, normalize_stage_result, permission_flags,
    persist_event_log, persistable_event_lines, pi_rpc_artifact_paths,
    reconciled_timeout_terminal_marker, resolve_permission_level, runner_prompt_path,
    should_persist_event_log, token_usage_from_line, token_usage_from_payload,
    token_usage_from_stats_payload, write_runner_completion, write_runner_invocation,
    write_stage_prompt_artifact,
};

pub use workspace::{
    BaselineManifest, BaselineManifestEntry, BaselineUpgradeEntry, BaselineUpgradePreview,
    ClearRuntimeOwnershipLockResult, ClosureLineageRepairOutcome, DoctorIssue, DoctorReport,
    LineageDiagnosticReason, LineageDriftDiagnostic, LineageDriftFinding, LineageRepairChange,
    LineageRepairError, LineageRepairPlan, LineageRepairResult, LineageWorkState, QueueClaim,
    QueueInspectionEntry, QueueStore, QueueStoreError, QueueStoreResult, RuntimeControl,
    RuntimeControlActionResult, RuntimeControlError, RuntimeControlMode, RuntimeControlResult,
    RuntimeOwnershipLockError, RuntimeOwnershipLockOptions, RuntimeOwnershipLockResult,
    RuntimeOwnershipLockState, RuntimeOwnershipLockStatus, RuntimeOwnershipRecord,
    StaleActiveState, StateStore, StateStoreError, StateStoreResult, UpgradeDisposition,
    WorkspaceError, WorkspacePaths, WorkspaceResult, acquire_runtime_ownership_lock,
    acquire_runtime_ownership_lock_with_options, append_usage_governance_ledger_entry,
    apply_baseline_upgrade, apply_lineage_repair_plan, atomic_write_text, bootstrap_workspace,
    build_baseline_manifest, build_lineage_repair_plan, build_lineage_repair_plan_at,
    claim_next_execution_task, claim_next_learning_request, claim_next_planning_item,
    clear_stale_runtime_ownership_lock, clear_stale_runtime_ownership_lock_with_pid_checker,
    closure_target_state_path, deploy_runtime_assets, detect_execution_stale_state,
    detect_learning_stale_state, detect_planning_stale_state, enqueue_incident,
    enqueue_learning_request, enqueue_spec, enqueue_task, find_queue_item,
    increment_troubleshoot_attempt, initialize_workspace, inspect_queue_items,
    inspect_runtime_ownership_lock, inspect_runtime_ownership_lock_with_pid_checker,
    known_root_spec_aliases, load_baseline_manifest, load_closure_target_state,
    load_execution_status, load_learning_status, load_planning_status, load_recovery_counters,
    load_snapshot, load_usage_governance_ledger, load_usage_governance_state,
    mark_incident_blocked, mark_incident_resolved, mark_learning_request_blocked,
    mark_learning_request_done, mark_spec_blocked, mark_spec_done, mark_task_blocked,
    mark_task_done, normalize_status_marker, preview_baseline_upgrade,
    refresh_lineage_repair_snapshot, release_runtime_ownership_lock, repair_closure_lineage,
    requeue_incident, requeue_learning_request, requeue_spec, requeue_task,
    require_initialized_workspace, require_initialized_workspace_paths,
    reset_forward_progress_counters, run_workspace_doctor, run_workspace_doctor_for_paths,
    save_closure_target_state, save_recovery_counters, save_snapshot, save_usage_governance_state,
    scan_closure_lineage_drift, scan_closure_lineage_drift_at, set_execution_status,
    set_learning_status, set_planning_status, workspace_paths, write_baseline_manifest,
    write_closure_lineage_repaired_event, write_lineage_drift_diagnostic,
    write_lineage_repair_report, write_mailbox_command,
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
