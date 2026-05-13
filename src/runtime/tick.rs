//! Deterministic serial runtime tick activation.

use std::{
    collections::{BTreeSet, HashMap},
    fmt, fs, io,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    compiler::{
        CompileWorkspaceOptions, CompiledGraphCompletionEntryPlan,
        CompiledGraphThresholdPolicyPlan, CompiledRunPlan, FrozenGraphPlanePlan,
        GraphLoopCounterName, GraphLoopEntryKey, GraphLoopTerminalClass, MaterializedGraphNodePlan,
        compile_and_persist_workspace_plan_for_paths,
    },
    contracts::{
        ActiveRunRequestKind, ActiveRunState, ClosureTargetState, ContractError, IncidentDecision,
        IncidentDocument, IncidentSeverity, LearningRequestDocument, LearningTerminalResult,
        MailboxAddIdeaPayload, MailboxAddProbePayload, MailboxAddSpecPayload,
        MailboxAddTaskPayload, MailboxCommand, MailboxCommandEnvelope, PauseSource, Plane,
        PlanningTerminalResult, ReconDecision, ReconPacketDocument, RecoveryCounters,
        ReloadOutcome, ResultClass, RootIntakeKind, RuntimeErrorCode, RuntimeErrorContext,
        RuntimeJsonContract, RuntimeSnapshot, SpecDocument, SpecSourceType, StageName,
        StageResultEnvelope, SubscriptionQuotaTelemetryState, TaskDocument, TerminalResult,
        Timestamp, UsageGovernanceBlocker, WorkDocumentError, WorkItemKind,
        allowed_work_item_kinds, stage_allows_work_item_kind, stage_name_for_plane,
    },
    recon_packets::{read_recon_packet, render_recon_packet},
    runners::{
        RunnerCompletionArtifactContext, RunnerEnvironmentDelta, RunnerError, RunnerExitKind,
        RunnerRawResult, StageRunnerAdapter, completion_artifact_from_raw_result,
        invocation_artifact_from_request, normalize_stage_result, write_runner_completion,
        write_runner_invocation, write_stage_prompt_artifact,
    },
    work_documents::{
        parse_incident_document_with_source, parse_learning_request_document_with_source,
        parse_spec_document_with_source, parse_spec_json_import_with_source,
        parse_task_document_with_source, parse_task_json_import_with_source,
    },
    workspace::{
        LineageRepairError, QueueClaim, QueueStore, QueueStoreError, StateStoreError,
        WorkspacePaths, atomic_write_text, load_closure_target_state, load_recovery_counters,
        load_usage_governance_ledger, load_usage_governance_state, reset_forward_progress_counters,
        save_closure_target_state, save_recovery_counters, save_snapshot,
        save_usage_governance_state, scan_closure_lineage_drift, set_execution_status,
        set_learning_status, set_planning_status, write_lineage_drift_diagnostic,
    },
};

use super::{
    AllowedResultClassPolicy, AllowedResultClassesByOutcome, RequestKind, RuntimeStartupError,
    RuntimeStartupSession, RuntimeWatchEvent, StageRunRequest, StageRunRequestError,
    blocked_recovery::persist_blocked_item_metadata,
    build_runtime_watcher_session, evaluate_usage_governance, load_runtime_startup_config,
    run_traces::{
        record_router_decision_trace, spawned_work_ref_from_path, upsert_stage_result_trace_node,
    },
};

static RUN_COUNTER: AtomicU64 = AtomicU64::new(0);
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

const IDLE_STATUS_MARKER: &str = "### IDLE";
const STAGE_WORK_ITEM_OWNERSHIP_INVALID: &str = "stage_work_item_ownership_invalid";

/// Result type for serial runtime tick activation.
pub type RuntimeTickResult<T> = Result<T, RuntimeTickError>;

/// Deterministic test overrides for one serial tick.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeTickOptions {
    /// Deterministic runtime timestamp; defaults to current UTC time.
    pub now: Option<Timestamp>,
    /// Deterministic run id for newly activated claims or closure targets.
    pub run_id: Option<String>,
    /// Deterministic request id for the constructed stage request.
    pub request_id: Option<String>,
}

/// Typed serial tick outcome before runner dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeTickOutcomeKind {
    /// One stage request was constructed and marked running.
    StageRequestReady,
    /// One stage request was dispatched and routed after runner completion.
    StageDispatched,
    /// Runtime-owned recovery work advanced without dispatching a stage.
    Recovered,
    /// No eligible work or completion target was available.
    NoWork,
    /// The runtime was paused before stage dispatch.
    Paused,
    /// A stop request was consumed before stage dispatch.
    Stopped,
    /// A runtime-owned inconsistency prevented safe dispatch.
    Blocked,
}

impl RuntimeTickOutcomeKind {
    /// Returns the stable outcome token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StageRequestReady => "stage_request_ready",
            Self::StageDispatched => "stage_dispatched",
            Self::Recovered => "recovered",
            Self::NoWork => "no_work",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
            Self::Blocked => "blocked",
        }
    }
}

/// Outcome evidence for one serial tick.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeTickOutcome {
    /// Typed outcome class.
    pub kind: RuntimeTickOutcomeKind,
    /// Stable reason token.
    pub reason: String,
    /// Constructed request when a stage should be dispatched by a later slice.
    pub stage_request: Option<StageRunRequest>,
    /// Snapshot after tick activation/status writes.
    pub snapshot: RuntimeSnapshot,
    /// Runtime event log path when this tick appended an event.
    pub event_log_path: Option<PathBuf>,
}

/// Runtime action selected by graph-authoritative routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterAction {
    /// Continue the active run at another stage.
    RunStage,
    /// Hand off into a different plane or incident flow.
    Handoff,
    /// The active plane has reached an idle terminal state.
    Idle,
    /// The active work cannot progress within configured recovery limits.
    Blocked,
}

impl RouterAction {
    /// Returns the canonical serialized action value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RunStage => "run_stage",
            Self::Handoff => "handoff",
            Self::Idle => "idle",
            Self::Blocked => "blocked",
        }
    }
}

/// Graph-authoritative routing decision for one normalized stage result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouterDecision {
    pub action: RouterAction,
    pub next_plane: Option<Plane>,
    pub next_stage: Option<StageName>,
    pub reason: String,
    pub next_node_id: Option<String>,
    pub next_stage_kind_id: Option<String>,
    pub failure_class: Option<String>,
    pub counter_key: Option<String>,
    pub create_incident: bool,
}

/// Outcome of a serial tick that dispatched a runner when a stage was ready.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeTickDispatchOutcome {
    /// Typed outcome class.
    pub kind: RuntimeTickOutcomeKind,
    /// Stable reason token.
    pub reason: String,
    /// Constructed request when a stage was dispatched.
    pub stage_request: Option<StageRunRequest>,
    /// Persisted stage request artifact path.
    pub stage_request_path: Option<PathBuf>,
    /// Raw runner result when dispatch occurred.
    pub runner_raw_result: Option<RunnerRawResult>,
    /// Persisted raw runner result artifact path.
    pub runner_raw_result_path: Option<PathBuf>,
    /// Normalized stage result when dispatch occurred.
    pub stage_result: Option<StageResultEnvelope>,
    /// Persisted normalized stage result artifact path.
    pub stage_result_path: Option<PathBuf>,
    /// Persisted terminal marker artifact path.
    pub terminal_marker_path: Option<PathBuf>,
    /// Router decision selected from the compiled graph.
    pub router_decision: Option<RouterDecision>,
    /// Persisted router decision artifact path.
    pub router_decision_path: Option<PathBuf>,
    /// Persisted runtime error context path, when normalization produced a recovery failure.
    pub runtime_error_context_path: Option<PathBuf>,
    /// Snapshot after tick dispatch, result persistence, and routing evidence writes.
    pub snapshot: RuntimeSnapshot,
    /// Runtime event log path when this tick appended an event.
    pub event_log_path: Option<PathBuf>,
}

/// Failures produced while activating one serial runtime tick.
#[derive(Debug)]
pub enum RuntimeTickError {
    /// A startup-owned operation failed.
    Startup(RuntimeStartupError),
    /// Queue state or work-document transition failed.
    Queue(QueueStoreError),
    /// Runtime state persistence failed.
    StateStore(StateStoreError),
    /// Closure-target state failed to load or save.
    Lineage(LineageRepairError),
    /// Shared contract validation failed.
    Contract(ContractError),
    /// Stage request construction failed.
    StageRunRequest(StageRunRequestError),
    /// Runner dispatch or stage-result normalization failed.
    Runner(RunnerError),
    /// A headed work document failed parsing or validation.
    WorkDocument {
        /// Source path involved in the parse.
        path: PathBuf,
        /// Typed work-document failure.
        source: WorkDocumentError,
    },
    /// Filesystem access failed.
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying IO error.
        message: String,
    },
    /// The compiled plan or snapshot state cannot be activated safely.
    InvalidState {
        /// Human-readable failure reason.
        message: String,
    },
    /// A deterministic timestamp could not be produced.
    Time {
        /// Timestamp field being built.
        field_name: &'static str,
        /// Human-readable failure reason.
        message: String,
    },
}

impl fmt::Display for RuntimeTickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Startup(error) => write!(f, "{error}"),
            Self::Queue(error) => write!(f, "{error}"),
            Self::StateStore(error) => write!(f, "{error}"),
            Self::Lineage(error) => write!(f, "{error}"),
            Self::Contract(error) => write!(f, "{error}"),
            Self::StageRunRequest(error) => write!(f, "{error}"),
            Self::Runner(error) => write!(f, "{error}"),
            Self::WorkDocument { path, source } => {
                write!(f, "work document error at {}: {source}", path.display())
            }
            Self::Io { path, message } => {
                write!(
                    f,
                    "runtime tick filesystem error at {}: {message}",
                    path.display()
                )
            }
            Self::InvalidState { message } => f.write_str(message),
            Self::Time {
                field_name,
                message,
            } => write!(f, "failed to build timestamp {field_name}: {message}"),
        }
    }
}

impl std::error::Error for RuntimeTickError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Startup(error) => Some(error),
            Self::Queue(error) => Some(error),
            Self::StateStore(error) => Some(error),
            Self::Lineage(error) => Some(error),
            Self::Contract(error) => Some(error),
            Self::StageRunRequest(error) => Some(error),
            Self::Runner(error) => Some(error),
            Self::WorkDocument { source, .. } => Some(source),
            Self::Io { .. } | Self::InvalidState { .. } | Self::Time { .. } => None,
        }
    }
}

impl From<RuntimeStartupError> for RuntimeTickError {
    fn from(value: RuntimeStartupError) -> Self {
        Self::Startup(value)
    }
}

impl From<QueueStoreError> for RuntimeTickError {
    fn from(value: QueueStoreError) -> Self {
        Self::Queue(value)
    }
}

impl From<StateStoreError> for RuntimeTickError {
    fn from(value: StateStoreError) -> Self {
        Self::StateStore(value)
    }
}

impl From<LineageRepairError> for RuntimeTickError {
    fn from(value: LineageRepairError) -> Self {
        Self::Lineage(value)
    }
}

impl From<ContractError> for RuntimeTickError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}

impl From<StageRunRequestError> for RuntimeTickError {
    fn from(value: StageRunRequestError) -> Self {
        Self::StageRunRequest(value)
    }
}

impl From<RunnerError> for RuntimeTickError {
    fn from(value: RunnerError) -> Self {
        Self::Runner(value)
    }
}

/// Activate at most one stage request for an already-started once-mode session.
pub fn run_serial_runtime_tick(
    session: &mut RuntimeStartupSession,
    options: RuntimeTickOptions,
) -> RuntimeTickResult<RuntimeTickOutcome> {
    let now = options
        .now
        .clone()
        .map(Ok)
        .unwrap_or_else(|| utc_now_timestamp("updated_at"))?;

    ingest_runtime_cycle_inputs(session, &now)?;

    if session.snapshot.stop_requested {
        return stopped_outcome(session, &now);
    }

    evaluate_and_apply_usage_governance(session, &now, None)?;

    if session.snapshot.paused {
        session.snapshot.updated_at = now.clone();
        save_snapshot(&session.paths, &session.snapshot)?;
        let event_log_path =
            write_runtime_event(&session.paths, "runtime_tick_paused", Map::new(), &now)?;
        return Ok(RuntimeTickOutcome {
            kind: RuntimeTickOutcomeKind::Paused,
            reason: "paused".to_owned(),
            stage_request: None,
            snapshot: session.snapshot.clone(),
            event_log_path: Some(event_log_path),
        });
    }

    if stale_reconciliation_blocks_tick(session) {
        let event_log_path = write_runtime_event(
            &session.paths,
            "runtime_tick_blocked",
            json_object([("reason", Value::String("stale_active_state".to_owned()))]),
            &now,
        )?;
        session.snapshot.updated_at = now.clone();
        save_snapshot(&session.paths, &session.snapshot)?;
        return Ok(RuntimeTickOutcome {
            kind: RuntimeTickOutcomeKind::Blocked,
            reason: "stale_active_state".to_owned(),
            stage_request: None,
            snapshot: session.snapshot.clone(),
            event_log_path: Some(event_log_path),
        });
    }

    if session.snapshot.active_stage.is_none() {
        activate_next_work_or_completion(session, &options, &now)?;
    }

    evaluate_and_apply_usage_governance(session, &now, None)?;

    if session.snapshot.paused {
        session.snapshot.updated_at = now.clone();
        save_snapshot(&session.paths, &session.snapshot)?;
        let event_log_path =
            write_runtime_event(&session.paths, "runtime_tick_paused", Map::new(), &now)?;
        return Ok(RuntimeTickOutcome {
            kind: RuntimeTickOutcomeKind::Paused,
            reason: "paused".to_owned(),
            stage_request: None,
            snapshot: session.snapshot.clone(),
            event_log_path: Some(event_log_path),
        });
    }

    let Some(active_plane) = session.snapshot.active_plane else {
        return idle_outcome(session, &now);
    };
    let Some(active_stage) = session.snapshot.active_stage else {
        return idle_outcome(session, &now);
    };

    if let Some(event_log_path) =
        guard_stage_work_item_ownership_for_plane(session, active_plane, &now)?
    {
        return Ok(RuntimeTickOutcome {
            kind: RuntimeTickOutcomeKind::Blocked,
            reason: STAGE_WORK_ITEM_OWNERSHIP_INVALID.to_owned(),
            stage_request: None,
            snapshot: session.snapshot.clone(),
            event_log_path: Some(event_log_path),
        });
    }

    let stage_plan = stage_plan_for_active_state(
        &session.compiled_plan,
        active_plane,
        active_stage,
        session.snapshot.active_node_id.as_deref(),
    )?;

    if !is_completion_stage_active(&session.snapshot)
        && (session.snapshot.active_work_item_kind.is_none()
            || session.snapshot.active_work_item_id.is_none())
    {
        let event_log_path = write_runtime_event(
            &session.paths,
            "runtime_tick_invalid_active_state",
            json_object([(
                "reason",
                Value::String("missing_active_work_item_identity".to_owned()),
            )]),
            &now,
        )?;
        return Ok(RuntimeTickOutcome {
            kind: RuntimeTickOutcomeKind::Blocked,
            reason: "missing_active_work_item_identity".to_owned(),
            stage_request: None,
            snapshot: session.snapshot.clone(),
            event_log_path: Some(event_log_path),
        });
    }

    let request = if is_completion_stage_active(&session.snapshot) {
        let target = active_closure_target_for_snapshot(&session.paths, &session.snapshot)?;
        build_closure_target_stage_run_request(session, stage_plan, &target, &options, &now)?
    } else {
        build_stage_run_request(session, stage_plan, &options, &now)?
    };

    mark_active_stage_running(session, &request, &now)?;
    let event_log_path = write_runtime_event(
        &session.paths,
        "stage_started",
        stage_started_data(&request),
        &now,
    )?;

    Ok(RuntimeTickOutcome {
        kind: RuntimeTickOutcomeKind::StageRequestReady,
        reason: "stage_started".to_owned(),
        stage_request: Some(request),
        snapshot: session.snapshot.clone(),
        event_log_path: Some(event_log_path),
    })
}

/// Run one serial runtime tick and dispatch the constructed stage through a runner.
pub fn run_serial_runtime_tick_with_runner(
    session: &mut RuntimeStartupSession,
    options: RuntimeTickOptions,
    runner: &impl StageRunnerAdapter,
) -> RuntimeTickResult<RuntimeTickDispatchOutcome> {
    let activation = run_serial_runtime_tick(session, options)?;
    let Some(request) = activation.stage_request.clone() else {
        return Ok(RuntimeTickDispatchOutcome {
            kind: activation.kind,
            reason: activation.reason,
            stage_request: None,
            stage_request_path: None,
            runner_raw_result: None,
            runner_raw_result_path: None,
            stage_result: None,
            stage_result_path: None,
            terminal_marker_path: None,
            router_decision: None,
            router_decision_path: None,
            runtime_error_context_path: None,
            snapshot: activation.snapshot,
            event_log_path: activation.event_log_path,
        });
    };

    let started_at = utc_now_timestamp("started_at")?;
    let raw_result = match runner.run(&request) {
        Ok(raw_result) => raw_result,
        Err(error) => {
            let completed_at = utc_now_timestamp("ended_at")?;
            runner_exception_raw_result(
                &request,
                &started_at,
                &completed_at,
                "RunnerError",
                &error.to_string(),
                runner_name_from_error(&error).as_deref(),
            )?
        }
    };
    apply_stage_worker_raw_result(session, request, raw_result)
}

pub(crate) fn apply_stage_worker_raw_result(
    session: &mut RuntimeStartupSession,
    request: StageRunRequest,
    raw_result: RunnerRawResult,
) -> RuntimeTickResult<RuntimeTickDispatchOutcome> {
    let stage_request_path = write_stage_request_artifact(&request)?;
    let runner_raw_result_path = write_runner_raw_result_artifact(&request, &raw_result)?;
    let stage_result_path = stage_result_artifact_path(&request);
    let mut stage_result = normalize_stage_result(&request, &raw_result)?;
    enrich_stage_result_artifact_metadata(
        &mut stage_result,
        &stage_request_path,
        &runner_raw_result_path,
        &stage_result_path,
    );
    let dispatch_application = match persist_and_apply_dispatch_result(
        session,
        &request,
        &mut stage_result,
        &stage_result_path,
    ) {
        Ok(application) => application,
        Err(application_error) => {
            let recovery = schedule_post_stage_exception_recovery(
                session,
                &stage_result,
                application_error.source.as_ref(),
                application_error.router_decision.as_deref(),
                application_error.stage_result_path.as_deref(),
            )?;
            let recovery_router_decision_path = write_recovery_router_decision_artifact(
                &request,
                &recovery.router_decision,
                application_error.stage_result_path.as_deref(),
            )?;
            return Ok(RuntimeTickDispatchOutcome {
                kind: RuntimeTickOutcomeKind::StageDispatched,
                reason: "stage_dispatched".to_owned(),
                stage_request: Some(request),
                stage_request_path: Some(stage_request_path),
                runner_raw_result: Some(raw_result),
                runner_raw_result_path: Some(runner_raw_result_path),
                stage_result: Some(stage_result),
                stage_result_path: application_error.stage_result_path,
                terminal_marker_path: application_error.terminal_marker_path,
                router_decision: Some(recovery.router_decision),
                router_decision_path: Some(recovery_router_decision_path),
                runtime_error_context_path: Some(recovery.runtime_error_context_path),
                snapshot: session.snapshot.clone(),
                event_log_path: Some(recovery.event_log_path),
            });
        }
    };

    Ok(RuntimeTickDispatchOutcome {
        kind: RuntimeTickOutcomeKind::StageDispatched,
        reason: "stage_dispatched".to_owned(),
        stage_request: Some(request),
        stage_request_path: Some(stage_request_path),
        runner_raw_result: Some(raw_result),
        runner_raw_result_path: Some(runner_raw_result_path),
        stage_result: Some(stage_result),
        stage_result_path: Some(stage_result_path),
        terminal_marker_path: Some(dispatch_application.terminal_marker_path),
        router_decision: Some(dispatch_application.router_decision),
        router_decision_path: Some(dispatch_application.router_decision_path),
        runtime_error_context_path: dispatch_application.runtime_error_context_path,
        snapshot: session.snapshot.clone(),
        event_log_path: Some(dispatch_application.event_log_path),
    })
}

pub(crate) fn runner_exception_raw_result(
    request: &StageRunRequest,
    started_at: &Timestamp,
    completed_at: &Timestamp,
    exception_type: &str,
    exception_message: &str,
    runner_name_override: Option<&str>,
) -> RuntimeTickResult<RunnerRawResult> {
    let run_dir = Path::new(&request.run_dir);
    create_dir_all(run_dir)?;
    let prompt_path = write_stage_prompt_artifact(request)?;
    let runner_name = runner_name_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            request
                .runner_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "stage_worker".to_owned());
    let stdout_path = run_dir.join(format!("runner_stdout.{}.txt", request.request_id));
    let stderr_path = run_dir.join(format!("runner_stderr.{}.txt", request.request_id));
    let invocation_path = run_dir.join(format!("runner_invocation.{}.json", request.request_id));
    let completion_path = run_dir.join(format!("runner_completion.{}.json", request.request_id));
    write_runtime_text(&stdout_path, "")?;
    write_runtime_text(
        &stderr_path,
        &format!("{exception_type}: {exception_message}\n"),
    )?;

    let command = vec!["millrace-runner-dispatch".to_owned(), runner_name.clone()];
    let environment_delta = RunnerEnvironmentDelta::default();
    let mut invocation = invocation_artifact_from_request(
        request,
        runner_name.clone(),
        command.clone(),
        request.run_dir.clone(),
        environment_delta.clone(),
        prompt_path.display().to_string(),
        started_at.clone(),
    )?;
    invocation.stdout_path = Some(stdout_path.display().to_string());
    invocation.stderr_path = Some(stderr_path.display().to_string());
    invocation.validate()?;
    write_runner_invocation(&invocation_path, &invocation)?;

    let raw_result = RunnerRawResult {
        request_id: request.request_id.clone(),
        run_id: request.run_id.clone(),
        stage: request.stage,
        runner_name: runner_name.clone(),
        model_name: request.model_name.clone(),
        thinking_level: request.thinking_level.clone(),
        model_reasoning_effort: request.model_reasoning_effort.clone(),
        exit_kind: RunnerExitKind::RunnerError,
        exit_code: Some(1),
        observed_exit_kind: None,
        observed_exit_code: None,
        stdout_path: Some(stdout_path.display().to_string()),
        stderr_path: Some(stderr_path.display().to_string()),
        terminal_result_path: None,
        event_log_path: None,
        token_usage: None,
        started_at: started_at.clone(),
        ended_at: completed_at.clone(),
    };
    raw_result.validate()?;
    let completion_context = RunnerCompletionArtifactContext::new(
        runner_name,
        command,
        request.run_dir.clone(),
        environment_delta,
        Some(prompt_path.display().to_string()),
        completed_at.clone(),
    )
    .with_failure_class(Some("runner_transport_failure".to_owned()))
    .with_notes(vec![format!("{exception_type}: {exception_message}")]);
    let completion = completion_artifact_from_raw_result(request, &raw_result, completion_context)?;
    write_runner_completion(&completion_path, &completion)?;
    Ok(raw_result)
}

fn runner_name_from_error(error: &RunnerError) -> Option<String> {
    match error {
        RunnerError::UnknownRunner { requested, .. } => Some(requested.clone()),
        RunnerError::RunnerBinaryNotFound { binary } => Some(binary.clone()),
        _ => None,
    }
}

fn write_runtime_text(path: &Path, contents: &str) -> RuntimeTickResult<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    atomic_write_text(path, contents)?;
    Ok(())
}

pub(crate) fn ingest_runtime_cycle_inputs(
    session: &mut RuntimeStartupSession,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    drain_mailbox(session, now)?;
    consume_watcher_events(session, now)?;
    refresh_queue_depths(&session.paths, &mut session.snapshot)?;
    save_snapshot(&session.paths, &session.snapshot)?;
    Ok(())
}

pub(crate) fn runtime_stop_requested(session: &RuntimeStartupSession) -> bool {
    session.snapshot.stop_requested
}

pub(crate) fn runtime_dispatch_paused(session: &RuntimeStartupSession) -> bool {
    session.snapshot.paused
}

pub(crate) fn runtime_reconciliation_blocks_dispatch(session: &RuntimeStartupSession) -> bool {
    stale_reconciliation_blocks_tick(session)
}

pub(crate) fn record_runtime_paused_cycle(
    session: &mut RuntimeStartupSession,
    now: &Timestamp,
) -> RuntimeTickResult<PathBuf> {
    session.snapshot.updated_at = now.clone();
    save_snapshot(&session.paths, &session.snapshot)?;
    write_runtime_event(&session.paths, "runtime_tick_paused", Map::new(), now)
}

pub(crate) fn record_runtime_blocked_cycle(
    session: &mut RuntimeStartupSession,
    reason: &str,
    now: &Timestamp,
) -> RuntimeTickResult<PathBuf> {
    let event_log_path = write_runtime_event(
        &session.paths,
        "runtime_tick_blocked",
        json_object([("reason", Value::String(reason.to_owned()))]),
        now,
    )?;
    session.snapshot.updated_at = now.clone();
    save_snapshot(&session.paths, &session.snapshot)?;
    Ok(event_log_path)
}

pub(crate) fn record_runtime_idle_cycle(
    session: &mut RuntimeStartupSession,
    now: &Timestamp,
) -> RuntimeTickResult<PathBuf> {
    session.snapshot.updated_at = now.clone();
    save_snapshot(&session.paths, &session.snapshot)?;
    write_runtime_event(&session.paths, "runtime_tick_idle", Map::new(), now)
}

pub(crate) fn evaluate_and_apply_usage_governance(
    session: &mut RuntimeStartupSession,
    now: &Timestamp,
    stage_result: Option<(&StageResultEnvelope, &Path)>,
) -> RuntimeTickResult<Option<PathBuf>> {
    if !should_evaluate_usage_governance(session) {
        return Ok(None);
    }

    let previous_state = load_usage_governance_state(&session.paths)?;
    let previous_ledger_len = if stage_result.is_none() && session.config.usage_governance.enabled {
        Some(load_usage_governance_ledger(&session.paths)?.len())
    } else {
        None
    };
    let had_governance_pause = has_governance_pause_source(&session.snapshot);
    let mut state = evaluate_usage_governance(
        &session.paths,
        &session.config.usage_governance,
        now.clone(),
        Some(session.lock_record.owner_session_id.clone()),
        had_governance_pause,
        stage_result,
        None,
    )?;

    let before_snapshot = session.snapshot.clone();
    if !session.config.usage_governance.enabled {
        if had_governance_pause {
            remove_governance_pause_source(&mut session.snapshot);
        }
    } else if !state.active_blockers.is_empty() {
        add_governance_pause_source(&mut session.snapshot);
    } else if had_governance_pause && session.config.usage_governance.auto_resume {
        remove_governance_pause_source(&mut session.snapshot);
    }
    let has_governance_pause = has_governance_pause_source(&session.snapshot);
    if session.snapshot != before_snapshot {
        session.snapshot.updated_at = now.clone();
        save_snapshot(&session.paths, &session.snapshot)?;
    }

    if state.paused_by_governance != has_governance_pause {
        state.paused_by_governance = has_governance_pause;
        save_usage_governance_state(&session.paths, &state)?;
    }

    let mut event_paths = Vec::new();
    if state.subscription_quota_status.state == SubscriptionQuotaTelemetryState::Degraded {
        event_paths.push(write_runtime_event(
            &session.paths,
            "usage_governance_degraded",
            json_object([
                (
                    "source",
                    Value::String(state.subscription_quota_status.provider.as_str().to_owned()),
                ),
                (
                    "policy",
                    state
                        .subscription_quota_status
                        .degraded_policy
                        .map(|policy| Value::String(policy.as_str().to_owned()))
                        .unwrap_or(Value::Null),
                ),
                (
                    "detail",
                    state
                        .subscription_quota_status
                        .detail
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
            ]),
            now,
        )?);
    }

    if !had_governance_pause && has_governance_pause && !state.active_blockers.is_empty() {
        for blocker in &state.active_blockers {
            event_paths.push(write_runtime_event(
                &session.paths,
                "usage_governance_blocked",
                blocker_event_data(blocker)?,
                now,
            )?);
            event_paths.push(write_runtime_event(
                &session.paths,
                "usage_governance_paused",
                blocker_pause_event_data(blocker),
                now,
            )?);
        }
    } else if had_governance_pause && !has_governance_pause {
        let cleared_rules = previous_state
            .active_blockers
            .iter()
            .map(|blocker| blocker.rule_id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        event_paths.push(write_runtime_event(
            &session.paths,
            "usage_governance_resumed",
            json_object([("cleared_rules", Value::String(cleared_rules))]),
            now,
        )?);
    }

    if let Some(previous_ledger_len) = previous_ledger_len {
        let current_ledger_len = load_usage_governance_ledger(&session.paths)?.len();
        if current_ledger_len > previous_ledger_len {
            event_paths.push(write_runtime_event(
                &session.paths,
                "usage_governance_reconciled",
                json_object([
                    (
                        "repaired_count",
                        Value::Number((current_ledger_len - previous_ledger_len).into()),
                    ),
                    (
                        "ledger_entry_count",
                        Value::Number(current_ledger_len.into()),
                    ),
                ]),
                now,
            )?);
        }
    }

    Ok(event_paths.pop())
}

fn should_evaluate_usage_governance(session: &RuntimeStartupSession) -> bool {
    session.config.usage_governance.enabled
        || session.paths.usage_governance_state_file.exists()
        || has_governance_pause_source(&session.snapshot)
}

fn has_governance_pause_source(snapshot: &RuntimeSnapshot) -> bool {
    snapshot
        .pause_sources
        .contains(&PauseSource::UsageGovernance)
}

fn add_governance_pause_source(snapshot: &mut RuntimeSnapshot) {
    if !snapshot
        .pause_sources
        .contains(&PauseSource::UsageGovernance)
    {
        snapshot.pause_sources.push(PauseSource::UsageGovernance);
    }
    order_pause_sources(&mut snapshot.pause_sources);
    snapshot.paused = true;
}

fn remove_governance_pause_source(snapshot: &mut RuntimeSnapshot) {
    snapshot
        .pause_sources
        .retain(|source| *source != PauseSource::UsageGovernance);
    order_pause_sources(&mut snapshot.pause_sources);
    snapshot.paused = !snapshot.pause_sources.is_empty();
}

fn order_pause_sources(sources: &mut Vec<PauseSource>) {
    let has_operator = sources.contains(&PauseSource::Operator);
    let has_usage = sources.contains(&PauseSource::UsageGovernance);
    sources.clear();
    if has_operator {
        sources.push(PauseSource::Operator);
    }
    if has_usage {
        sources.push(PauseSource::UsageGovernance);
    }
}

fn blocker_event_data(blocker: &UsageGovernanceBlocker) -> RuntimeTickResult<Map<String, Value>> {
    match json_value(blocker)? {
        Value::Object(data) => Ok(data),
        _ => Err(invalid_state(
            "usage-governance blocker did not serialize to an object",
        )),
    }
}

fn blocker_pause_event_data(blocker: &UsageGovernanceBlocker) -> Map<String, Value> {
    json_object([
        ("source", Value::String(blocker.source.as_str().to_owned())),
        ("rule_id", Value::String(blocker.rule_id.clone())),
        ("window", Value::String(blocker.window.clone())),
        ("observed", json!(blocker.observed)),
        ("threshold", json!(blocker.threshold)),
        (
            "next_auto_resume_at",
            blocker
                .next_auto_resume_at
                .as_ref()
                .map(|timestamp| Value::String(timestamp.as_str().to_owned()))
                .unwrap_or(Value::Null),
        ),
    ])
}

fn persist_and_apply_dispatch_result(
    session: &mut RuntimeStartupSession,
    request: &StageRunRequest,
    stage_result: &mut StageResultEnvelope,
    stage_result_path: &Path,
) -> Result<DispatchApplicationOutput, DispatchApplicationError> {
    let mut partial = DispatchApplicationPartial::default();

    let counters = match load_recovery_counters(&session.paths) {
        Ok(counters) => counters,
        Err(error) => return Err(partial.fail(error.into())),
    };
    let router_decision = match route_stage_result_from_graph(
        &session.compiled_plan,
        &session.snapshot,
        stage_result,
        &counters,
    ) {
        Ok(router_decision) => router_decision,
        Err(error) => return Err(partial.fail(error)),
    };
    partial.router_decision = Some(router_decision.clone());

    let runtime_error_context_path = match maybe_persist_runtime_error_context(
        session,
        stage_result,
        &router_decision,
        stage_result_path,
    ) {
        Ok(path) => path,
        Err(error) => return Err(partial.fail(error)),
    };
    if let Some(path) = &runtime_error_context_path {
        stage_result.metadata.insert(
            "runtime_error_context_path".to_owned(),
            Value::String(path.display().to_string()),
        );
    }

    let terminal_marker_path = match write_terminal_marker_artifact(request, stage_result) {
        Ok(path) => path,
        Err(error) => return Err(partial.fail(error)),
    };
    partial.terminal_marker_path = Some(terminal_marker_path.clone());
    stage_result.metadata.insert(
        "terminal_marker_path".to_owned(),
        Value::String(terminal_marker_path.display().to_string()),
    );

    if let Err(error) = write_stage_result_artifact(stage_result_path, stage_result) {
        return Err(partial.fail(error));
    }
    let run_dir = Path::new(&request.run_dir);
    upsert_stage_result_trace_node(&session.paths, run_dir, stage_result, stage_result_path);
    partial.stage_result_path = Some(stage_result_path.to_path_buf());

    let learning_request_paths = match enqueue_learning_requests_for_stage_result(
        session,
        stage_result,
        stage_result_path,
    ) {
        Ok(paths) => paths,
        Err(error) => return Err(partial.fail(error)),
    };

    let router_decision_path =
        match write_router_decision_artifact(request, &router_decision, stage_result_path) {
            Ok(path) => path,
            Err(error) => return Err(partial.fail(error)),
        };

    if let Err(error) = persist_dispatch_snapshot_updates(
        session,
        stage_result,
        &router_decision,
        stage_result_path,
    ) {
        return Err(partial.fail(error));
    }
    let _stage_completed_event_path = match write_runtime_event(
        &session.paths,
        "stage_completed",
        stage_completed_data(request, stage_result, stage_result_path),
        &stage_result.completed_at,
    ) {
        Ok(path) => path,
        Err(error) => return Err(partial.fail(error)),
    };

    if let Err(error) = evaluate_and_apply_usage_governance(
        session,
        &stage_result.completed_at,
        Some((stage_result, stage_result_path)),
    ) {
        return Err(partial.fail(error));
    }

    let router_event_path = match write_runtime_event(
        &session.paths,
        "router_decision",
        router_decision_data(request, stage_result, &router_decision),
        &stage_result.completed_at,
    ) {
        Ok(path) => path,
        Err(error) => return Err(partial.fail(error)),
    };

    let spawned_paths =
        match apply_router_decision(session, &router_decision, stage_result, stage_result_path) {
            Ok(paths) => paths,
            Err(error) => return Err(partial.fail(error)),
        };
    let spawned_work = learning_request_paths
        .iter()
        .map(|path| spawned_work_ref_from_path(path, stage_result, "learning_trigger"))
        .chain(spawned_paths.iter().map(|path| {
            spawned_work_ref_from_path(path, stage_result, router_decision.reason.clone())
        }))
        .collect();
    record_router_decision_trace(
        &session.paths,
        run_dir,
        stage_result,
        &router_decision,
        spawned_work,
    );
    if let Err(error) = handle_learning_curator_promotion_boundary(session, stage_result) {
        return Err(partial.fail(error));
    }
    if let Err(error) =
        apply_deferred_learning_promotions_if_safe(session, &stage_result.completed_at)
    {
        return Err(partial.fail(error));
    }

    Ok(DispatchApplicationOutput {
        terminal_marker_path,
        router_decision,
        router_decision_path,
        runtime_error_context_path,
        event_log_path: router_event_path,
    })
}

fn activate_next_work_or_completion(
    session: &mut RuntimeStartupSession,
    options: &RuntimeTickOptions,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    let queue = QueueStore::from_paths(session.paths.clone());
    if let Some(target) = active_closure_target(&session.paths)? {
        if let Some(claim) = queue.claim_next_execution_task(Some(&target.root_spec_id))? {
            activate_claim(session, claim, options, now)?;
            return Ok(());
        }
        if let Some(claim) = queue.claim_next_planning_item(Some(&target.root_spec_id))? {
            activate_claim(session, claim, options, now)?;
            return Ok(());
        }
        maybe_activate_completion_stage(session, target, options, now)?;
        return Ok(());
    }

    if let Some(claim) = queue.claim_next_planning_item(None)? {
        activate_claim(session, claim, options, now)?;
        return Ok(());
    }
    if let Some(claim) = queue.claim_next_execution_task(None)? {
        activate_claim(session, claim, options, now)?;
        return Ok(());
    }
    if session.compiled_plan.learning_graph.is_some() {
        if let Some(claim) = queue.claim_next_learning_request()? {
            activate_claim(session, claim, options, now)?;
            return Ok(());
        }
    }
    if let Some(target) = recover_or_backfill_missing_closure_target(session, now)? {
        maybe_activate_completion_stage(session, target, options, now)?;
    }
    Ok(())
}

pub(crate) fn activate_next_claim_for_plane(
    session: &mut RuntimeStartupSession,
    plane: Plane,
    options: &RuntimeTickOptions,
    now: &Timestamp,
) -> RuntimeTickResult<bool> {
    if active_run_for_plane(&session.snapshot, plane).is_some() {
        return Ok(false);
    }

    let queue = QueueStore::from_paths(session.paths.clone());
    let closure_root_spec_id = active_closure_target(&session.paths)?
        .map(|target| target.root_spec_id)
        .filter(|_| matches!(plane, Plane::Execution | Plane::Planning));
    let claim = match plane {
        Plane::Execution => queue.claim_next_execution_task(closure_root_spec_id.as_deref())?,
        Plane::Planning => queue.claim_next_planning_item(closure_root_spec_id.as_deref())?,
        Plane::Learning => {
            if session.compiled_plan.learning_graph.is_none() {
                None
            } else {
                queue.claim_next_learning_request()?
            }
        }
    };

    let Some(claim) = claim else {
        return Ok(false);
    };
    activate_claim(session, claim, options, now)?;
    Ok(true)
}

fn activate_claim(
    session: &mut RuntimeStartupSession,
    claim: QueueClaim,
    options: &RuntimeTickOptions,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    maybe_create_closure_target_for_claim(session, &claim, now)?;
    let activation = activation_for_claim(&session.compiled_plan, &claim)?;
    let active_run = ActiveRunState {
        plane: activation.plane,
        stage: activation.stage,
        node_id: activation.node_id,
        stage_kind_id: activation.stage_kind_id,
        run_id: options.run_id.clone().unwrap_or_else(|| new_run_id("run")),
        request_kind: if claim.work_item_kind == WorkItemKind::LearningRequest {
            ActiveRunRequestKind::LearningRequest
        } else {
            ActiveRunRequestKind::ActiveWorkItem
        },
        work_item_kind: Some(claim.work_item_kind),
        work_item_id: Some(claim.work_item_id),
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        active_since: now.clone(),
        running_status_marker: None,
    };
    snapshot_with_active_run(&mut session.snapshot, active_run, now);
    session.snapshot.current_failure_class = None;
    save_snapshot(&session.paths, &session.snapshot)?;
    Ok(())
}

pub(crate) fn activate_completion_stage_if_ready(
    session: &mut RuntimeStartupSession,
    options: &RuntimeTickOptions,
    now: &Timestamp,
) -> RuntimeTickResult<bool> {
    if active_run_for_plane(&session.snapshot, Plane::Planning).is_some() {
        return Ok(false);
    }
    let target = match active_closure_target(&session.paths)? {
        Some(target) => target,
        None => {
            let Some(target) = recover_or_backfill_missing_closure_target(session, now)? else {
                return Ok(false);
            };
            target
        }
    };
    maybe_activate_completion_stage(session, target, options, now)?;
    Ok(active_run_for_plane(&session.snapshot, Plane::Planning).is_some())
}

fn maybe_activate_completion_stage(
    session: &mut RuntimeStartupSession,
    mut target: ClosureTargetState,
    options: &RuntimeTickOptions,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    if !target.closure_open {
        return Ok(());
    }
    target = refresh_closure_target_readiness(&session.paths, target)?;
    if target.closure_blocked_by_lineage_work {
        return Ok(());
    }
    if block_on_closure_lineage_drift_if_present(session, &target, now)? {
        return Ok(());
    }
    let activation = completion_activation_for_graph(&session.compiled_plan)?;
    let active_run = ActiveRunState {
        plane: activation.plane,
        stage: activation.stage,
        node_id: activation.node_id,
        stage_kind_id: activation.stage_kind_id,
        run_id: options.run_id.clone().unwrap_or_else(|| new_run_id("run")),
        request_kind: ActiveRunRequestKind::ClosureTarget,
        work_item_kind: None,
        work_item_id: None,
        closure_target_root_spec_id: Some(target.root_spec_id),
        closure_target_root_idea_id: Some(target.root_idea_id),
        active_since: now.clone(),
        running_status_marker: None,
    };
    snapshot_with_active_run(&mut session.snapshot, active_run, now);
    session.snapshot.current_failure_class = None;
    save_snapshot(&session.paths, &session.snapshot)?;
    Ok(())
}

fn maybe_create_closure_target_for_claim(
    session: &RuntimeStartupSession,
    claim: &QueueClaim,
    now: &Timestamp,
) -> RuntimeTickResult<Option<ClosureTargetState>> {
    if claim.work_item_kind != WorkItemKind::Spec {
        return Ok(None);
    }
    let spec = read_spec_document(&claim.path)?;
    create_closure_target_for_root_spec(session, &claim.path, &spec, now)
}

fn recover_or_backfill_missing_closure_target(
    session: &mut RuntimeStartupSession,
    now: &Timestamp,
) -> RuntimeTickResult<Option<ClosureTargetState>> {
    if session
        .compiled_plan
        .planning_graph
        .completion_behavior
        .is_none()
    {
        return Ok(None);
    }
    let Some((spec_path, spec)) = latest_root_spec_candidate(&session.paths)? else {
        return Ok(None);
    };
    let (Some(root_spec_id), Some(_root_idea_id)) = (&spec.root_spec_id, &spec.root_idea_id) else {
        mark_completion_behavior_blocked(
            session,
            "missing_root_lineage",
            &spec.spec_id,
            &spec_path,
            now,
        )?;
        return Ok(None);
    };

    match load_closure_target_state(&session.paths, root_spec_id) {
        Ok(target) => {
            if target.closure_open {
                Ok(Some(target))
            } else {
                Ok(None)
            }
        }
        Err(LineageRepairError::MissingClosureTarget { .. }) => {
            let Some(target) =
                create_closure_target_for_root_spec(session, &spec_path, &spec, now)?
            else {
                return Ok(None);
            };
            write_runtime_event(
                &session.paths,
                "completion_behavior_target_backfilled",
                json_object([
                    ("root_spec_id", Value::String(target.root_spec_id.clone())),
                    ("root_idea_id", Value::String(target.root_idea_id.clone())),
                    (
                        "spec_path",
                        Value::String(workspace_relative(&session.paths, &spec_path)),
                    ),
                ]),
                now,
            )?;
            Ok(Some(target))
        }
        Err(error) => Err(error.into()),
    }
}

fn create_closure_target_for_root_spec(
    session: &RuntimeStartupSession,
    spec_path: &Path,
    spec: &SpecDocument,
    now: &Timestamp,
) -> RuntimeTickResult<Option<ClosureTargetState>> {
    let (Some(root_spec_id), Some(root_idea_id)) = (&spec.root_spec_id, &spec.root_idea_id) else {
        return Ok(None);
    };
    if spec.spec_id != root_spec_id.as_str() {
        return Ok(None);
    }
    match load_closure_target_state(&session.paths, root_spec_id) {
        Ok(target) => return Ok(Some(target)),
        Err(LineageRepairError::MissingClosureTarget { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    let open_targets = open_closure_targets(&session.paths)?;
    let actionable_targets = actionable_open_closure_targets(open_targets);
    if actionable_targets.len() > 1 {
        return Err(invalid_state(
            "multiple actionable open closure targets found",
        ));
    }
    if !actionable_targets.is_empty() {
        return Err(invalid_state(
            "cannot open closure target while another open closure target exists",
        ));
    }

    let Some(idea_markdown) = load_root_idea_markdown(&session.paths, spec)? else {
        return Ok(None);
    };
    let spec_markdown =
        fs::read_to_string(spec_path).map_err(|error| io_error(spec_path, error))?;
    let root_idea_path = session
        .paths
        .arbiter_idea_contracts_dir
        .join(format!("{root_idea_id}.md"));
    let root_spec_path = session
        .paths
        .arbiter_root_spec_contracts_dir
        .join(format!("{root_spec_id}.md"));
    atomic_write_text(&root_idea_path, &idea_markdown)?;
    atomic_write_text(&root_spec_path, &spec_markdown)?;

    let target = ClosureTargetState {
        schema_version: "1.0".to_owned(),
        kind: "closure_target_state".to_owned(),
        root_spec_id: root_spec_id.clone(),
        root_idea_id: root_idea_id.clone(),
        root_intake_kind: None,
        root_intake_id: None,
        root_spec_path: workspace_relative(&session.paths, &root_spec_path),
        root_idea_path: workspace_relative(&session.paths, &root_idea_path),
        rubric_path: workspace_relative(
            &session.paths,
            &session
                .paths
                .arbiter_rubrics_dir
                .join(format!("{root_spec_id}.md")),
        ),
        latest_verdict_path: None,
        latest_report_path: None,
        closure_open: true,
        closure_blocked_by_lineage_work: false,
        blocking_work_ids: Vec::new(),
        opened_at: now.clone(),
        closed_at: None,
        last_arbiter_run_id: None,
    };
    save_closure_target_state(&session.paths, &target)?;
    Ok(Some(target))
}

fn latest_root_spec_candidate(
    paths: &WorkspacePaths,
) -> RuntimeTickResult<Option<(PathBuf, SpecDocument)>> {
    let mut candidates = Vec::new();
    for directory in [
        &paths.specs_active_dir,
        &paths.specs_done_dir,
        &paths.specs_queue_dir,
        &paths.specs_blocked_dir,
    ] {
        for path in markdown_files(directory)? {
            let Ok(spec) = read_spec_document(&path) else {
                continue;
            };
            if is_root_spec_candidate(&spec) {
                candidates.push((path, spec));
            }
        }
    }
    candidates.sort_by(|(left_path, left), (right_path, right)| {
        right
            .created_at
            .as_str()
            .cmp(left.created_at.as_str())
            .then_with(|| right.spec_id.cmp(&left.spec_id))
            .then_with(|| right_path.cmp(left_path))
    });
    Ok(candidates.into_iter().next())
}

fn read_spec_document(path: &Path) -> RuntimeTickResult<SpecDocument> {
    let raw = fs::read_to_string(path).map_err(|error| io_error(path, error))?;
    parse_spec_document_with_source(&raw, &path.display().to_string()).map_err(|source| {
        RuntimeTickError::WorkDocument {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn is_root_spec_candidate(spec: &SpecDocument) -> bool {
    if let Some(root_spec_id) = spec.root_spec_id.as_deref() {
        return root_spec_id == spec.spec_id;
    }
    matches!(
        spec.source_type,
        SpecSourceType::Idea | SpecSourceType::Manual
    ) && !has_parent_spec(spec)
}

fn has_parent_spec(spec: &SpecDocument) -> bool {
    spec.parent_spec_id
        .as_deref()
        .is_some_and(|value| !value.trim().eq_ignore_ascii_case("none"))
}

fn load_root_idea_markdown(
    paths: &WorkspacePaths,
    spec: &SpecDocument,
) -> RuntimeTickResult<Option<String>> {
    for candidate in root_idea_source_candidates(paths, spec) {
        if candidate.is_file() {
            return fs::read_to_string(&candidate)
                .map(Some)
                .map_err(|error| io_error(&candidate, error));
        }
    }
    Ok(None)
}

fn root_idea_source_candidates(paths: &WorkspacePaths, spec: &SpecDocument) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for reference in &spec.references {
        push_unique_path(&mut candidates, resolve_reference_path(paths, reference));
    }
    if let Some(source_id) = &spec.source_id {
        push_unique_path(
            &mut candidates,
            paths
                .root
                .join("ideas")
                .join("inbox")
                .join(format!("{source_id}.md")),
        );
    }
    if let Some(root_idea_id) = &spec.root_idea_id {
        push_unique_path(
            &mut candidates,
            paths
                .root
                .join("ideas")
                .join("inbox")
                .join(format!("{root_idea_id}.md")),
        );
    }
    candidates
}

fn resolve_reference_path(paths: &WorkspacePaths, reference: &str) -> PathBuf {
    let candidate = PathBuf::from(reference);
    if candidate.is_absolute() {
        candidate
    } else {
        paths.root.join(candidate)
    }
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|candidate| candidate == &path) {
        paths.push(path);
    }
}

fn block_on_closure_lineage_drift_if_present(
    session: &mut RuntimeStartupSession,
    target: &ClosureTargetState,
    now: &Timestamp,
) -> RuntimeTickResult<bool> {
    let Some(diagnostic) = scan_closure_lineage_drift(&session.paths, target)? else {
        return Ok(false);
    };
    let diagnostic_path = write_lineage_drift_diagnostic(&session.paths, &diagnostic)?;
    let mut blocked_target = target.clone();
    blocked_target.closure_blocked_by_lineage_work = true;
    blocked_target.blocking_work_ids = diagnostic
        .findings
        .iter()
        .map(|finding| finding.work_item_id.clone())
        .collect();
    save_closure_target_state(&session.paths, &blocked_target)?;
    mark_completion_behavior_blocked(
        session,
        "closure_lineage_drift",
        &target.root_spec_id,
        &diagnostic_path,
        now,
    )?;
    write_runtime_event(
        &session.paths,
        "closure_lineage_drift_detected",
        json_object([
            (
                "root_spec_id",
                Value::String(diagnostic.root_spec_id.clone()),
            ),
            (
                "root_idea_id",
                Value::String(diagnostic.root_idea_id.clone()),
            ),
            (
                "finding_count",
                Value::Number((diagnostic.findings.len() as u64).into()),
            ),
            (
                "diagnostic_path",
                Value::String(workspace_relative(&session.paths, &diagnostic_path)),
            ),
        ]),
        now,
    )?;
    Ok(true)
}

fn mark_completion_behavior_blocked(
    session: &mut RuntimeStartupSession,
    failure_class: &str,
    spec_id: &str,
    spec_path: &Path,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    if session.snapshot.planning_status_marker == "### BLOCKED"
        && session.snapshot.current_failure_class.as_deref() == Some(failure_class)
    {
        return Ok(());
    }
    session.snapshot.current_failure_class = Some(failure_class.to_owned());
    session.snapshot.updated_at = now.clone();
    set_status_for_plane(&session.paths, Plane::Planning, "### BLOCKED")?;
    set_snapshot_status_for_plane(&mut session.snapshot, Plane::Planning, "### BLOCKED");
    save_snapshot(&session.paths, &session.snapshot)?;
    write_runtime_event(
        &session.paths,
        "completion_behavior_blocked",
        json_object([
            ("reason", Value::String(failure_class.to_owned())),
            ("spec_id", Value::String(spec_id.to_owned())),
            (
                "spec_path",
                Value::String(workspace_relative(&session.paths, spec_path)),
            ),
        ]),
        now,
    )?;
    Ok(())
}

fn build_stage_run_request(
    session: &RuntimeStartupSession,
    stage_plan: &MaterializedGraphNodePlan,
    options: &RuntimeTickOptions,
    now: &Timestamp,
) -> RuntimeTickResult<StageRunRequest> {
    let active_run = active_run_for_plane(&session.snapshot, stage_plan.plane)
        .ok_or_else(|| invalid_state("active run is missing for stage request"))?;
    let active_work_item_kind = active_run.work_item_kind;
    let active_work_item_id = active_run.work_item_id.clone();
    let run_id = active_run.run_id.clone();
    let run_dir = session.paths.runs_dir.join(&run_id);
    create_dir_all(&run_dir)?;
    let request_id = options
        .request_id
        .clone()
        .unwrap_or_else(|| new_request_id("request"));
    let stage = stage_name_for_plane(stage_plan.plane, &stage_plan.stage_kind_id)?;
    let required_skill_paths =
        runtime_asset_paths(&session.paths, &stage_plan.required_skill_paths);
    let attached_skill_paths =
        runtime_asset_paths(&session.paths, &stage_plan.attached_skill_additions);
    let skill_revision_evidence_path = write_skill_revision_evidence_if_enabled(
        session,
        &run_dir,
        &request_id,
        &run_id,
        &required_skill_paths,
        &attached_skill_paths,
        now,
    )?;
    let runtime_error_fields = runtime_error_request_fields(session, &active_run, stage)?;
    let mut request = StageRunRequest {
        request_id,
        run_id,
        plane: stage_plan.plane,
        stage,
        request_kind: request_kind_for_active_run(&active_run),
        mode_id: session.snapshot.active_mode_id.clone(),
        compiled_plan_id: session.snapshot.compiled_plan_id.clone(),
        node_id: stage_plan.node_id.clone(),
        stage_kind_id: stage_plan.stage_kind_id.clone(),
        running_status_marker: stage_plan.running_status_marker.clone(),
        legal_terminal_markers: Vec::new(),
        allowed_result_classes_by_outcome: policy_for_stage_plan(stage, stage_plan),
        entrypoint_path: session
            .paths
            .runtime_root
            .join(&stage_plan.entrypoint_path)
            .display()
            .to_string(),
        entrypoint_contract_id: stage_plan.entrypoint_contract_id.clone(),
        required_skill_paths,
        attached_skill_paths,
        active_work_item_kind,
        active_work_item_id: active_work_item_id.clone(),
        active_work_item_path: active_work_item_path(
            &session.paths,
            active_work_item_kind,
            active_work_item_id.as_deref(),
        )
        .map(|path| path.display().to_string()),
        closure_target_path: None,
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        canonical_root_spec_path: None,
        canonical_seed_idea_path: None,
        preferred_rubric_path: None,
        preferred_verdict_path: None,
        preferred_report_path: None,
        run_dir: run_dir.display().to_string(),
        summary_status_path: status_path_for_plane(&session.paths, stage_plan.plane)
            .display()
            .to_string(),
        runtime_snapshot_path: session.paths.runtime_snapshot_file.display().to_string(),
        recovery_counters_path: session.paths.recovery_counters_file.display().to_string(),
        preferred_troubleshoot_report_path: Some(
            run_dir.join("troubleshoot_report.md").display().to_string(),
        ),
        runtime_error_code: runtime_error_fields.runtime_error_code,
        runtime_error_report_path: runtime_error_fields.runtime_error_report_path,
        runtime_error_catalog_path: runtime_error_fields.runtime_error_catalog_path,
        skill_revision_evidence_path: skill_revision_evidence_path
            .map(|path| path.display().to_string()),
        runner_name: stage_plan.runner_name.clone(),
        model_name: stage_plan.model_name.clone(),
        thinking_level: stage_plan.thinking_level.clone(),
        model_reasoning_effort: stage_plan.model_reasoning_effort.clone(),
        timeout_seconds: stage_plan.timeout_seconds,
    };
    request.validate()?;
    Ok(request)
}

pub(crate) fn build_stage_run_request_for_plane(
    session: &RuntimeStartupSession,
    plane: Plane,
    options: &RuntimeTickOptions,
    now: &Timestamp,
) -> RuntimeTickResult<StageRunRequest> {
    let active_run = active_run_for_plane(&session.snapshot, plane)
        .ok_or_else(|| invalid_state("active run is missing for stage request"))?;
    let stage_plan = stage_plan_for_active_state(
        &session.compiled_plan,
        active_run.plane,
        active_run.stage,
        Some(&active_run.node_id),
    )?;
    if active_run.request_kind == ActiveRunRequestKind::ClosureTarget {
        let target = active_closure_target_for_active_run(&session.paths, &active_run)?;
        build_closure_target_stage_run_request(session, stage_plan, &target, options, now)
    } else {
        build_stage_run_request(session, stage_plan, options, now)
    }
}

fn build_closure_target_stage_run_request(
    session: &RuntimeStartupSession,
    stage_plan: &MaterializedGraphNodePlan,
    target: &ClosureTargetState,
    options: &RuntimeTickOptions,
    now: &Timestamp,
) -> RuntimeTickResult<StageRunRequest> {
    let active_run = active_run_for_plane(&session.snapshot, stage_plan.plane)
        .ok_or_else(|| invalid_state("active closure run is missing for stage request"))?;
    let run_id = active_run.run_id.clone();
    let run_dir = session.paths.runs_dir.join(&run_id);
    create_dir_all(&run_dir)?;
    let request_id = options
        .request_id
        .clone()
        .unwrap_or_else(|| new_request_id("request"));
    let stage = stage_name_for_plane(stage_plan.plane, &stage_plan.stage_kind_id)?;
    let required_skill_paths =
        runtime_asset_paths(&session.paths, &stage_plan.required_skill_paths);
    let attached_skill_paths =
        runtime_asset_paths(&session.paths, &stage_plan.attached_skill_additions);
    let skill_revision_evidence_path = write_skill_revision_evidence_if_enabled(
        session,
        &run_dir,
        &request_id,
        &run_id,
        &required_skill_paths,
        &attached_skill_paths,
        now,
    )?;
    let mut request = StageRunRequest {
        request_id,
        run_id,
        plane: stage_plan.plane,
        stage,
        request_kind: RequestKind::ClosureTarget,
        mode_id: session.snapshot.active_mode_id.clone(),
        compiled_plan_id: session.snapshot.compiled_plan_id.clone(),
        node_id: stage_plan.node_id.clone(),
        stage_kind_id: stage_plan.stage_kind_id.clone(),
        running_status_marker: stage_plan.running_status_marker.clone(),
        legal_terminal_markers: Vec::new(),
        allowed_result_classes_by_outcome: policy_for_stage_plan(stage, stage_plan),
        entrypoint_path: session
            .paths
            .runtime_root
            .join(&stage_plan.entrypoint_path)
            .display()
            .to_string(),
        entrypoint_contract_id: stage_plan.entrypoint_contract_id.clone(),
        required_skill_paths,
        attached_skill_paths,
        active_work_item_kind: None,
        active_work_item_id: None,
        active_work_item_path: None,
        closure_target_path: Some(
            session
                .paths
                .arbiter_targets_dir
                .join(format!("{}.json", target.root_spec_id))
                .display()
                .to_string(),
        ),
        closure_target_root_spec_id: Some(target.root_spec_id.clone()),
        closure_target_root_idea_id: Some(target.root_idea_id.clone()),
        canonical_root_spec_path: Some(target.root_spec_path.clone()),
        canonical_seed_idea_path: Some(target.root_idea_path.clone()),
        preferred_rubric_path: Some(target.rubric_path.clone()),
        preferred_verdict_path: target.latest_verdict_path.clone().or_else(|| {
            Some(
                session
                    .paths
                    .arbiter_verdicts_dir
                    .join(format!("{}.json", target.root_spec_id))
                    .display()
                    .to_string(),
            )
        }),
        preferred_report_path: Some(run_dir.join("arbiter_report.md").display().to_string()),
        run_dir: run_dir.display().to_string(),
        summary_status_path: session.paths.planning_status_file.display().to_string(),
        runtime_snapshot_path: session.paths.runtime_snapshot_file.display().to_string(),
        recovery_counters_path: session.paths.recovery_counters_file.display().to_string(),
        preferred_troubleshoot_report_path: None,
        runtime_error_code: None,
        runtime_error_report_path: None,
        runtime_error_catalog_path: None,
        skill_revision_evidence_path: skill_revision_evidence_path
            .map(|path| path.display().to_string()),
        runner_name: stage_plan.runner_name.clone(),
        model_name: stage_plan.model_name.clone(),
        thinking_level: stage_plan.thinking_level.clone(),
        model_reasoning_effort: stage_plan.model_reasoning_effort.clone(),
        timeout_seconds: stage_plan.timeout_seconds,
    };
    request.validate()?;
    Ok(request)
}

pub(crate) fn guard_stage_work_item_ownership_for_plane(
    session: &mut RuntimeStartupSession,
    plane: Plane,
    now: &Timestamp,
) -> RuntimeTickResult<Option<PathBuf>> {
    let Some(active_run) = active_run_for_plane(&session.snapshot, plane) else {
        return Ok(None);
    };
    let Some(violation) = stage_work_item_ownership_violation(&active_run) else {
        return Ok(None);
    };
    record_stage_work_item_ownership_invalid(session, &active_run, violation, now).map(Some)
}

fn stage_work_item_ownership_violation(
    active_run: &ActiveRunState,
) -> Option<StageWorkItemOwnershipViolation> {
    if active_run.request_kind == ActiveRunRequestKind::ClosureTarget {
        return None;
    }

    let expected_work_item_kinds = allowed_work_item_kinds(active_run.stage).to_vec();
    let Some(work_item_kind) = active_run.work_item_kind else {
        return Some(StageWorkItemOwnershipViolation {
            reason: "missing_work_item_kind",
            expected_work_item_kinds,
            message: format!(
                "stage {} active run is missing work_item_kind",
                active_run.stage.as_str()
            ),
        });
    };
    if active_run.work_item_id.as_deref().is_none_or(str::is_empty) {
        return Some(StageWorkItemOwnershipViolation {
            reason: "missing_work_item_id",
            expected_work_item_kinds,
            message: format!(
                "stage {} active run is missing work_item_id",
                active_run.stage.as_str()
            ),
        });
    }
    if active_run.request_kind == ActiveRunRequestKind::LearningRequest
        && work_item_kind != WorkItemKind::LearningRequest
    {
        return Some(StageWorkItemOwnershipViolation {
            reason: "learning_request_kind_mismatch",
            expected_work_item_kinds,
            message: format!(
                "learning_request active run for stage {} has work item kind {}",
                active_run.stage.as_str(),
                work_item_kind.as_str()
            ),
        });
    }
    if active_run.request_kind == ActiveRunRequestKind::ActiveWorkItem
        && work_item_kind == WorkItemKind::LearningRequest
    {
        return Some(StageWorkItemOwnershipViolation {
            reason: "active_work_item_learning_request",
            expected_work_item_kinds,
            message: format!(
                "active_work_item run for stage {} has learning_request work item kind",
                active_run.stage.as_str()
            ),
        });
    }
    if !stage_allows_work_item_kind(active_run.stage, work_item_kind) {
        let expected = render_work_item_kind_list(&expected_work_item_kinds);
        return Some(StageWorkItemOwnershipViolation {
            reason: "stage_work_item_kind_mismatch",
            expected_work_item_kinds,
            message: if expected.is_empty() {
                format!(
                    "stage {} does not allow active work items; got {}",
                    active_run.stage.as_str(),
                    work_item_kind.as_str()
                )
            } else {
                format!(
                    "stage {} cannot receive work item kind {}; expected one of: {}",
                    active_run.stage.as_str(),
                    work_item_kind.as_str(),
                    expected
                )
            },
        });
    }
    None
}

fn record_stage_work_item_ownership_invalid(
    session: &mut RuntimeStartupSession,
    active_run: &ActiveRunState,
    violation: StageWorkItemOwnershipViolation,
    now: &Timestamp,
) -> RuntimeTickResult<PathBuf> {
    let context_paths =
        write_stage_work_item_ownership_runtime_error(session, active_run, &violation, now)?;
    let queue = QueueStore::from_paths(session.paths.clone());
    let requeued_count =
        requeue_all_active_items(&session.paths, &queue, STAGE_WORK_ITEM_OWNERSHIP_INVALID)?;

    if let (Some(work_item_kind), Some(work_item_id)) = (
        active_run.work_item_kind,
        active_run.work_item_id.as_deref(),
    ) {
        reset_forward_progress_counters(&session.paths, work_item_kind, work_item_id)?;
        session.counters = load_recovery_counters(&session.paths)?;
    }

    reset_runtime_to_idle(
        &session.paths,
        &mut session.snapshot,
        true,
        false,
        false,
        now,
    )?;
    session.snapshot.current_failure_class = Some(STAGE_WORK_ITEM_OWNERSHIP_INVALID.to_owned());
    session.snapshot.updated_at = now.clone();
    save_snapshot(&session.paths, &session.snapshot)?;

    write_runtime_event(
        &session.paths,
        "runtime_stage_work_item_ownership_invalid",
        json_object([
            ("reason", Value::String(violation.reason.to_owned())),
            ("message", Value::String(violation.message)),
            ("plane", Value::String(active_run.plane.as_str().to_owned())),
            ("stage", Value::String(active_run.stage.as_str().to_owned())),
            ("node_id", Value::String(active_run.node_id.clone())),
            (
                "stage_kind_id",
                Value::String(active_run.stage_kind_id.clone()),
            ),
            ("run_id", Value::String(active_run.run_id.clone())),
            (
                "request_kind",
                Value::String(active_run.request_kind.as_str().to_owned()),
            ),
            (
                "work_item_kind",
                active_run
                    .work_item_kind
                    .map(|kind| Value::String(kind.as_str().to_owned()))
                    .unwrap_or(Value::Null),
            ),
            (
                "work_item_id",
                active_run
                    .work_item_id
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            ),
            (
                "expected_work_item_kinds",
                Value::Array(
                    violation
                        .expected_work_item_kinds
                        .iter()
                        .map(|kind| Value::String(kind.as_str().to_owned()))
                        .collect(),
                ),
            ),
            (
                "requeued_count",
                Value::Number(serde_json::Number::from(requeued_count)),
            ),
            (
                "runtime_error_context_path",
                context_paths
                    .context_path
                    .as_ref()
                    .map(|path| Value::String(workspace_relative(&session.paths, path)))
                    .unwrap_or(Value::Null),
            ),
            (
                "runtime_error_report_path",
                context_paths
                    .report_path
                    .as_ref()
                    .map(|path| Value::String(workspace_relative(&session.paths, path)))
                    .unwrap_or(Value::Null),
            ),
        ]),
        now,
    )
}

fn write_stage_work_item_ownership_runtime_error(
    session: &RuntimeStartupSession,
    active_run: &ActiveRunState,
    violation: &StageWorkItemOwnershipViolation,
    now: &Timestamp,
) -> RuntimeTickResult<StageWorkItemOwnershipContextPaths> {
    let (Some(work_item_kind), Some(work_item_id)) =
        (active_run.work_item_kind, active_run.work_item_id.as_ref())
    else {
        return Ok(StageWorkItemOwnershipContextPaths::default());
    };

    let report_path = session
        .paths
        .runs_dir
        .join(&active_run.run_id)
        .join("runtime_error_report.md");
    if let Some(parent) = report_path.parent() {
        create_dir_all(parent)?;
    }
    let context = RuntimeErrorContext {
        schema_version: "1.0".to_owned(),
        kind: "runtime_error_context".to_owned(),
        error_code: RuntimeErrorCode::StageWorkItemOwnershipInvalid,
        plane: active_run.plane,
        failed_stage: active_run.stage,
        repair_stage: active_run.stage,
        work_item_kind,
        work_item_id: work_item_id.clone(),
        run_id: active_run.run_id.clone(),
        router_action: None,
        terminal_result: None,
        stage_result_path: None,
        report_path: report_path.display().to_string(),
        exception_type: "StageWorkItemOwnershipInvalid".to_owned(),
        exception_message: violation.message.clone(),
        captured_at: now.clone(),
    };
    context.validate().map_err(|error| {
        invalid_state(format!("runtime error context validation failed: {error}"))
    })?;
    write_runtime_error_report(&report_path, &context)?;
    write_pretty_json(&session.paths.runtime_error_context_file, &context)?;
    Ok(StageWorkItemOwnershipContextPaths {
        context_path: Some(session.paths.runtime_error_context_file.clone()),
        report_path: Some(report_path),
    })
}

fn runtime_error_request_fields(
    session: &RuntimeStartupSession,
    active_run: &ActiveRunState,
    stage: StageName,
) -> RuntimeTickResult<RuntimeErrorRequestFields> {
    if !matches!(stage, StageName::Troubleshooter | StageName::Mechanic) {
        return Ok(RuntimeErrorRequestFields::default());
    }
    if !session.paths.runtime_error_context_file.is_file() {
        return Ok(RuntimeErrorRequestFields::default());
    }
    let raw = fs::read_to_string(&session.paths.runtime_error_context_file)
        .map_err(|error| io_error(&session.paths.runtime_error_context_file, error))?;
    let context = RuntimeErrorContext::from_json_str(&raw).map_err(|source| {
        RuntimeTickError::InvalidState {
            message: format!("runtime error context is invalid: {source}"),
        }
    })?;
    if context.plane != active_run.plane
        || context.repair_stage != stage
        || context.run_id != active_run.run_id
        || active_run.work_item_kind != Some(context.work_item_kind)
        || active_run.work_item_id.as_deref() != Some(context.work_item_id.as_str())
    {
        return Ok(RuntimeErrorRequestFields::default());
    }
    let catalog_path = session
        .paths
        .root
        .join("docs")
        .join("runtime")
        .join("millrace-runtime-error-codes.md");
    Ok(RuntimeErrorRequestFields {
        runtime_error_code: Some(context.error_code.as_str().to_owned()),
        runtime_error_report_path: Some(context.report_path),
        runtime_error_catalog_path: catalog_path
            .is_file()
            .then(|| catalog_path.display().to_string()),
    })
}

pub(crate) fn mark_stage_running_and_emit_started(
    session: &mut RuntimeStartupSession,
    request: &StageRunRequest,
    now: &Timestamp,
) -> RuntimeTickResult<PathBuf> {
    mark_active_stage_running(session, request, now)?;
    write_runtime_event(
        &session.paths,
        "stage_started",
        stage_started_data(request),
        now,
    )
}

fn mark_active_stage_running(
    session: &mut RuntimeStartupSession,
    request: &StageRunRequest,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    let status_marker = running_status_marker(&request.running_status_marker);
    set_status_for_plane(&session.paths, request.plane, &status_marker)?;
    set_snapshot_status_for_plane(&mut session.snapshot, request.plane, &status_marker);
    if let Some(active_run) = session
        .snapshot
        .active_runs_by_plane
        .get_mut(&request.plane)
    {
        active_run.running_status_marker = Some(request.running_status_marker.clone());
    }
    session.snapshot.updated_at = now.clone();
    save_snapshot(&session.paths, &session.snapshot)?;
    Ok(())
}

fn write_stage_request_artifact(request: &StageRunRequest) -> RuntimeTickResult<PathBuf> {
    let path = request_artifact_path(request, "stage_requests", "json");
    write_pretty_json(&path, request)?;
    Ok(path)
}

fn write_runner_raw_result_artifact(
    request: &StageRunRequest,
    raw_result: &RunnerRawResult,
) -> RuntimeTickResult<PathBuf> {
    let path = request_artifact_path(request, "runner_results", "json");
    write_pretty_json(&path, raw_result)?;
    Ok(path)
}

fn stage_result_artifact_path(request: &StageRunRequest) -> PathBuf {
    request_artifact_path(request, "stage_results", "json")
}

fn write_stage_result_artifact(
    path: &Path,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<()> {
    let mut validated = stage_result.clone();
    validated
        .validate()
        .map_err(|error| invalid_state(format!("stage result validation failed: {error}")))?;
    write_pretty_json(path, &validated)
}

fn write_terminal_marker_artifact(
    request: &StageRunRequest,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<PathBuf> {
    let path = request_artifact_path(request, "terminal_markers", "txt");
    atomic_write_text(&path, &(stage_result.terminal_result.marker() + "\n"))?;
    Ok(path)
}

fn write_router_decision_artifact(
    request: &StageRunRequest,
    decision: &RouterDecision,
    stage_result_path: &Path,
) -> RuntimeTickResult<PathBuf> {
    let path = request_artifact_path(request, "router_decisions", "json");
    let payload = json!({
        "schema_version": "1.0",
        "kind": "router_decision",
        "request_id": &request.request_id,
        "run_id": &request.run_id,
        "plane": request.plane,
        "stage": request.stage,
        "node_id": &request.node_id,
        "stage_kind_id": &request.stage_kind_id,
        "stage_result_path": stage_result_path.display().to_string(),
        "decision": decision,
    });
    write_pretty_json(&path, &payload)?;
    Ok(path)
}

fn write_recovery_router_decision_artifact(
    request: &StageRunRequest,
    decision: &RouterDecision,
    stage_result_path: Option<&Path>,
) -> RuntimeTickResult<PathBuf> {
    let path = Path::new(&request.run_dir)
        .join("router_decisions")
        .join(format!("{}.runtime_recovery.json", request.request_id));
    let payload = json!({
        "schema_version": "1.0",
        "kind": "router_decision",
        "request_id": &request.request_id,
        "run_id": &request.run_id,
        "plane": request.plane,
        "stage": request.stage,
        "node_id": &request.node_id,
        "stage_kind_id": &request.stage_kind_id,
        "stage_result_path": stage_result_path.map(|path| path.display().to_string()),
        "decision": decision,
    });
    write_pretty_json(&path, &payload)?;
    Ok(path)
}

fn request_artifact_path(request: &StageRunRequest, directory: &str, extension: &str) -> PathBuf {
    Path::new(&request.run_dir)
        .join(directory)
        .join(format!("{}.{}", request.request_id, extension))
}

fn enrich_stage_result_artifact_metadata(
    stage_result: &mut StageResultEnvelope,
    stage_request_path: &Path,
    runner_raw_result_path: &Path,
    stage_result_path: &Path,
) {
    stage_result.metadata.insert(
        "stage_request_path".to_owned(),
        Value::String(stage_request_path.display().to_string()),
    );
    stage_result.metadata.insert(
        "runner_raw_result_path".to_owned(),
        Value::String(runner_raw_result_path.display().to_string()),
    );
    stage_result.metadata.insert(
        "stage_result_path".to_owned(),
        Value::String(stage_result_path.display().to_string()),
    );
}

fn persist_dispatch_snapshot_updates(
    session: &mut RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    router_decision: &RouterDecision,
    stage_result_path: &Path,
) -> RuntimeTickResult<()> {
    set_status_for_plane(
        &session.paths,
        stage_result.plane,
        &stage_result.summary_status_marker,
    )?;
    set_snapshot_status_for_plane(
        &mut session.snapshot,
        stage_result.plane,
        &stage_result.summary_status_marker,
    );
    session.snapshot.last_terminal_result = Some(stage_result.terminal_result);
    session.snapshot.last_stage_result_path =
        Some(path_relative_to_root(&session.paths, stage_result_path));
    session.snapshot.current_failure_class = router_decision.failure_class.clone();
    refresh_queue_depths(&session.paths, &mut session.snapshot)?;
    session.snapshot.updated_at = stage_result.completed_at.clone();
    save_snapshot(&session.paths, &session.snapshot)?;
    Ok(())
}

fn maybe_persist_runtime_error_context(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    router_decision: &RouterDecision,
    stage_result_path: &Path,
) -> RuntimeTickResult<Option<PathBuf>> {
    if stage_result.result_class != ResultClass::RecoverableFailure {
        return Ok(None);
    }
    let Some((error_code, default_repair_stage)) =
        runtime_error_code_and_repair_stage(stage_result.plane)
    else {
        return Ok(None);
    };
    let repair_stage = router_decision
        .next_stage
        .filter(|stage| stage.plane() == stage_result.plane)
        .unwrap_or(default_repair_stage);
    let report_path = session
        .paths
        .runs_dir
        .join(&stage_result.run_id)
        .join("runtime_error_report.md");
    let failure_class = failure_class_from_stage_result(stage_result)
        .unwrap_or_else(|| "stage_result_normalization_failed".to_owned());
    let context = RuntimeErrorContext {
        schema_version: "1.0".to_owned(),
        kind: "runtime_error_context".to_owned(),
        error_code,
        plane: stage_result.plane,
        failed_stage: stage_result.stage,
        repair_stage,
        work_item_kind: stage_result.work_item_kind,
        work_item_id: stage_result.work_item_id.clone(),
        run_id: stage_result.run_id.clone(),
        router_action: Some(router_decision.action.as_str().to_owned()),
        terminal_result: Some(stage_result.terminal_result),
        stage_result_path: Some(path_relative_to_root(&session.paths, stage_result_path)),
        report_path: report_path.display().to_string(),
        exception_type: "StageResultNormalizationFailure".to_owned(),
        exception_message: failure_class,
        captured_at: stage_result.completed_at.clone(),
    };
    context.validate().map_err(|error| {
        invalid_state(format!("runtime error context validation failed: {error}"))
    })?;
    write_runtime_error_report(&report_path, &context)?;
    write_pretty_json(&session.paths.runtime_error_context_file, &context)?;
    Ok(Some(session.paths.runtime_error_context_file.clone()))
}

fn runtime_error_code_and_repair_stage(plane: Plane) -> Option<(RuntimeErrorCode, StageName)> {
    match plane {
        Plane::Execution => Some((
            RuntimeErrorCode::ExecutionPostStageApplyFailed,
            StageName::Troubleshooter,
        )),
        Plane::Planning => Some((
            RuntimeErrorCode::PlanningPostStageApplyFailed,
            StageName::Mechanic,
        )),
        Plane::Learning => None,
    }
}

fn schedule_post_stage_exception_recovery(
    session: &mut RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    error: &RuntimeTickError,
    router_decision: Option<&RouterDecision>,
    stage_result_path: Option<&Path>,
) -> RuntimeTickResult<PostStageRecoveryOutput> {
    let Some((_default_error_code, repair_stage)) =
        runtime_error_code_and_repair_stage(stage_result.plane)
    else {
        return Err(invalid_state("learning post-stage recovery is unsupported"));
    };
    let error_code =
        classify_post_stage_application_error(stage_result.plane, error, router_decision)?;
    let source_router_action = router_decision.map(|decision| decision.action.as_str().to_owned());
    let captured_at = stage_result.completed_at.clone();
    let report_path = session
        .paths
        .runs_dir
        .join(&stage_result.run_id)
        .join("runtime_error_report.md");
    let (repair_node_id, repair_stage_kind_id) =
        compiled_identity_for_stage(&session.compiled_plan, stage_result.plane, repair_stage);
    let context = RuntimeErrorContext {
        schema_version: "1.0".to_owned(),
        kind: "runtime_error_context".to_owned(),
        error_code,
        plane: stage_result.plane,
        failed_stage: stage_result.stage,
        repair_stage,
        work_item_kind: stage_result.work_item_kind,
        work_item_id: stage_result.work_item_id.clone(),
        run_id: stage_result.run_id.clone(),
        router_action: source_router_action.clone(),
        terminal_result: Some(stage_result.terminal_result),
        stage_result_path: stage_result_path
            .map(|path| path_relative_to_root(&session.paths, path)),
        report_path: report_path.display().to_string(),
        exception_type: runtime_tick_error_type(error).to_owned(),
        exception_message: error.to_string(),
        captured_at: captured_at.clone(),
    };
    context.validate().map_err(|error| {
        invalid_state(format!("runtime error context validation failed: {error}"))
    })?;
    write_runtime_error_report(&report_path, &context)?;
    write_pretty_json(&session.paths.runtime_error_context_file, &context)?;

    set_status_for_plane(&session.paths, stage_result.plane, "### BLOCKED")?;
    set_snapshot_status_for_plane(&mut session.snapshot, stage_result.plane, "### BLOCKED");
    set_recovery_stage_for_plane(
        &mut session.snapshot,
        stage_result,
        repair_stage,
        repair_node_id.clone(),
        repair_stage_kind_id.clone(),
        error_code.as_str(),
        &captured_at,
    );
    refresh_queue_depths(&session.paths, &mut session.snapshot)?;
    session.snapshot.last_terminal_result = Some(stage_result.terminal_result);
    session.snapshot.last_stage_result_path =
        stage_result_path.map(|path| path_relative_to_root(&session.paths, path));
    session.snapshot.updated_at = captured_at.clone();
    save_snapshot(&session.paths, &session.snapshot)?;

    let router_decision = RouterDecision {
        action: RouterAction::RunStage,
        next_plane: Some(stage_result.plane),
        next_stage: Some(repair_stage),
        reason: format!("runtime_exception:{}", error_code.as_str()),
        next_node_id: Some(repair_node_id.clone()),
        next_stage_kind_id: Some(repair_stage_kind_id.clone()),
        failure_class: Some(error_code.as_str().to_owned()),
        counter_key: None,
        create_incident: false,
    };
    let event_log_path = write_runtime_event(
        &session.paths,
        "runtime_post_stage_recovery_scheduled",
        json_object([
            ("error_code", Value::String(error_code.as_str().to_owned())),
            (
                "plane",
                Value::String(stage_result.plane.as_str().to_owned()),
            ),
            (
                "failed_stage",
                Value::String(stage_result.stage.as_str().to_owned()),
            ),
            (
                "repair_stage",
                Value::String(repair_stage.as_str().to_owned()),
            ),
            ("repair_node_id", Value::String(repair_node_id)),
            ("repair_stage_kind_id", Value::String(repair_stage_kind_id)),
            (
                "router_action",
                source_router_action
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            ),
            (
                "terminal_result",
                Value::String(stage_result.terminal_result.as_str().to_owned()),
            ),
            (
                "work_item_kind",
                Value::String(stage_result.work_item_kind.as_str().to_owned()),
            ),
            (
                "work_item_id",
                Value::String(stage_result.work_item_id.clone()),
            ),
            (
                "exception_type",
                Value::String(context.exception_type.clone()),
            ),
            (
                "exception_message",
                Value::String(context.exception_message.clone()),
            ),
            ("report_path", Value::String(context.report_path.clone())),
        ]),
        &captured_at,
    )?;

    Ok(PostStageRecoveryOutput {
        router_decision,
        runtime_error_context_path: session.paths.runtime_error_context_file.clone(),
        event_log_path,
    })
}

fn classify_post_stage_application_error(
    plane: Plane,
    error: &RuntimeTickError,
    router_decision: Option<&RouterDecision>,
) -> RuntimeTickResult<RuntimeErrorCode> {
    let completion_conflict = router_decision.is_some_and(|decision| {
        decision.action == RouterAction::Idle
            && matches!(
                error,
                RuntimeTickError::Queue(QueueStoreError::InvalidState { .. })
            )
    });
    Ok(match (plane, completion_conflict) {
        (Plane::Execution, true) => RuntimeErrorCode::ExecutionWorkItemCompletionConflict,
        (Plane::Planning, true) => RuntimeErrorCode::PlanningWorkItemCompletionConflict,
        (Plane::Execution, false) => RuntimeErrorCode::ExecutionPostStageApplyFailed,
        (Plane::Planning, false) => RuntimeErrorCode::PlanningPostStageApplyFailed,
        (Plane::Learning, _) => {
            return Err(invalid_state("learning post-stage recovery is unsupported"));
        }
    })
}

fn runtime_tick_error_type(error: &RuntimeTickError) -> &'static str {
    match error {
        RuntimeTickError::Startup(_) => "RuntimeStartupError",
        RuntimeTickError::Queue(QueueStoreError::InvalidState { .. }) => "QueueStateError",
        RuntimeTickError::Queue(_) => "QueueStoreError",
        RuntimeTickError::StateStore(_) => "StateStoreError",
        RuntimeTickError::Lineage(_) => "LineageRepairError",
        RuntimeTickError::Contract(_) => "ContractError",
        RuntimeTickError::StageRunRequest(_) => "StageRunRequestError",
        RuntimeTickError::Runner(_) => "RunnerError",
        RuntimeTickError::WorkDocument { .. } => "WorkDocumentError",
        RuntimeTickError::Io { .. } => "IoError",
        RuntimeTickError::InvalidState { .. } => "RuntimeStateError",
        RuntimeTickError::Time { .. } => "TimeError",
    }
}

fn compiled_identity_for_stage(
    plan: &CompiledRunPlan,
    plane: Plane,
    stage: StageName,
) -> (String, String) {
    let stage_id = stage.as_str().to_owned();
    let Ok(graph) = graph_for_plane(plan, plane) else {
        return (stage_id.clone(), stage_id);
    };
    graph
        .nodes
        .iter()
        .find(|node| {
            node.node_id == stage.as_str()
                || stage_name_for_plane(plane, &node.stage_kind_id)
                    .ok()
                    .is_some_and(|node_stage| node_stage == stage)
        })
        .map(|node| (node.node_id.clone(), node.stage_kind_id.clone()))
        .unwrap_or_else(|| (stage_id.clone(), stage_id))
}

fn set_recovery_stage_for_plane(
    snapshot: &mut RuntimeSnapshot,
    stage_result: &StageResultEnvelope,
    repair_stage: StageName,
    repair_node_id: String,
    repair_stage_kind_id: String,
    failure_class: &str,
    now: &Timestamp,
) {
    let mut active_run =
        active_run_for_plane(snapshot, stage_result.plane).unwrap_or_else(|| ActiveRunState {
            plane: stage_result.plane,
            stage: stage_result.stage,
            node_id: stage_result.node_id.clone(),
            stage_kind_id: stage_result.stage_kind_id.clone(),
            run_id: stage_result.run_id.clone(),
            request_kind: if stage_result.work_item_kind == WorkItemKind::LearningRequest {
                ActiveRunRequestKind::LearningRequest
            } else {
                ActiveRunRequestKind::ActiveWorkItem
            },
            work_item_kind: Some(stage_result.work_item_kind),
            work_item_id: Some(stage_result.work_item_id.clone()),
            closure_target_root_spec_id: None,
            closure_target_root_idea_id: None,
            active_since: now.clone(),
            running_status_marker: None,
        });
    active_run.stage = repair_stage;
    active_run.node_id = repair_node_id;
    active_run.stage_kind_id = repair_stage_kind_id;
    active_run.active_since = now.clone();
    active_run.running_status_marker = None;
    snapshot_with_active_run(snapshot, active_run, now);
    snapshot.current_failure_class = Some(failure_class.to_owned());
}

fn write_runtime_error_report(path: &Path, context: &RuntimeErrorContext) -> RuntimeTickResult<()> {
    let terminal_result = context
        .terminal_result
        .map(|result| result.as_str())
        .unwrap_or("none");
    let mut lines = vec![
        "# Runtime Error Report".to_owned(),
        String::new(),
        format!("Error-Code: {}", context.error_code.as_str()),
        format!("Plane: {}", context.plane.as_str()),
        format!("Failed-Stage: {}", context.failed_stage.as_str()),
        format!("Repair-Stage: {}", context.repair_stage.as_str()),
        format!("Run-ID: {}", context.run_id),
        format!(
            "Work-Item: {} {}",
            context.work_item_kind.as_str(),
            context.work_item_id
        ),
        format!(
            "Router-Action: {}",
            context.router_action.as_deref().unwrap_or("none")
        ),
        format!("Terminal-Result: {terminal_result}"),
        format!(
            "Stage-Result-Path: {}",
            context.stage_result_path.as_deref().unwrap_or("none")
        ),
        format!("Exception-Type: {}", context.exception_type),
        format!("Exception-Message: {}", context.exception_message),
        format!("Captured-At: {}", context.captured_at.as_str()),
        String::new(),
        "Summary:".to_owned(),
    ];
    if context.exception_type == "StageResultNormalizationFailure" {
        lines.push(
            "- The runtime normalized malformed or missing terminal output into recovery evidence."
                .to_owned(),
        );
        lines
            .push("- Forward progress should route through the compiled recovery path.".to_owned());
    } else {
        lines.push(
            "- The runtime hit an exception after a stage returned a legal terminal result."
                .to_owned(),
        );
        lines.push(
            "- Forward progress was rerouted into the default recovery stage instead of exiting the daemon."
                .to_owned(),
        );
        lines.push(
            "- Consult the runtime error catalog when the error code needs interpretation."
                .to_owned(),
        );
    }
    let payload = lines.join("\n") + "\n";
    atomic_write_text(path, &payload)?;
    Ok(())
}

fn path_relative_to_root(paths: &WorkspacePaths, path: &Path) -> String {
    path.strip_prefix(&paths.root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn apply_router_decision(
    session: &mut RuntimeStartupSession,
    decision: &RouterDecision,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> RuntimeTickResult<Vec<PathBuf>> {
    clear_runtime_error_context_if_consumed(session, stage_result)?;

    if is_closure_target_stage_result(stage_result) {
        return apply_closure_target_result(session, decision, stage_result, stage_result_path);
    }

    if is_recon_stage_result(stage_result) {
        return apply_recon_router_decision(session, decision, stage_result, stage_result_path);
    }

    match decision.action {
        RouterAction::RunStage => {
            apply_run_stage_decision(session, decision, stage_result).map(|()| Vec::new())
        }
        RouterAction::Idle => apply_idle_decision(session, stage_result).map(|()| Vec::new()),
        RouterAction::Handoff => {
            apply_handoff_decision(session, decision, stage_result, stage_result_path)
        }
        RouterAction::Blocked => {
            apply_blocked_decision(session, decision, stage_result, stage_result_path)
                .map(|()| Vec::new())
        }
    }
}

fn enqueue_learning_requests_for_stage_result(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> RuntimeTickResult<Vec<PathBuf>> {
    if session.compiled_plan.learning_graph.is_none() {
        return Ok(Vec::new());
    }

    let queue = QueueStore::from_paths(session.paths.clone());
    let mut queued_paths = Vec::new();
    for rule in &session.compiled_plan.learning_trigger_rules {
        if rule.source_plane != stage_result.plane
            || rule.source_stage != stage_result.stage
            || !rule
                .on_terminal_results
                .iter()
                .any(|outcome| outcome == stage_result.terminal_result.as_str())
        {
            continue;
        }

        let artifact_paths = learning_request_artifact_paths(stage_result, stage_result_path);
        let stage_result_artifact_path = stage_result_path.display().to_string();
        let document = LearningRequestDocument {
            learning_request_id: new_request_id("learn"),
            title: format!(
                "Learn from {} {}",
                stage_result.node_id,
                stage_result.terminal_result.as_str()
            ),
            summary: format!(
                "Runtime-generated learning request from a compiler-frozen trigger rule: {}",
                rule.rule_id
            ),
            requested_action: rule.requested_action,
            target_skill_id: rule.target_skill_id.clone(),
            target_stage: Some(rule.target_stage),
            source_refs: vec![
                format!("run:{}", stage_result.run_id),
                format!(
                    "request:{}",
                    metadata_string(stage_result, "request_id")
                        .unwrap_or_else(|| "unknown".to_owned())
                ),
                format!("node:{}", stage_result.node_id),
                format!("stage_kind:{}", stage_result.stage_kind_id),
                format!("stage:{}", stage_result.stage.as_str()),
                format!("terminal:{}", stage_result.terminal_result.as_str()),
            ],
            preferred_output_paths: rule.preferred_output_paths.clone(),
            trigger_metadata: json!({
                "rule_id": rule.rule_id,
                "source_plane": stage_result.plane.as_str(),
                "source_stage": stage_result.stage.as_str(),
                "source_node_id": stage_result.node_id,
                "source_stage_kind_id": stage_result.stage_kind_id,
                "terminal_result": stage_result.terminal_result.as_str(),
                "target_stage": rule.target_stage.as_str(),
                "target_skill_id": rule.target_skill_id,
                "preferred_output_paths": rule.preferred_output_paths,
                "stage_result_path": stage_result_artifact_path,
                "artifact_paths": artifact_paths.clone(),
                "run_id": stage_result.run_id,
                "work_item_kind": stage_result.work_item_kind.as_str(),
                "work_item_id": stage_result.work_item_id,
                "source_work_item_kind": stage_result.work_item_kind.as_str(),
                "source_work_item_id": stage_result.work_item_id,
                "source_active_work_item_path": metadata_string(stage_result, "active_work_item_path"),
            }),
            originating_run_ids: vec![stage_result.run_id.clone()],
            artifact_paths,
            references: Vec::new(),
            created_at: stage_result.completed_at.clone(),
            created_by: "millrace runtime".to_owned(),
            updated_at: None,
        };
        let queued_path = queue.enqueue_learning_request(&document)?;
        write_runtime_event(
            &session.paths,
            "learning_request_enqueued",
            json_object([
                (
                    "learning_request_id",
                    Value::String(document.learning_request_id.clone()),
                ),
                ("rule_id", Value::String(rule.rule_id.clone())),
                (
                    "source_plane",
                    Value::String(stage_result.plane.as_str().to_owned()),
                ),
                (
                    "source_stage",
                    Value::String(stage_result.stage.as_str().to_owned()),
                ),
                (
                    "source_node_id",
                    Value::String(stage_result.node_id.clone()),
                ),
                (
                    "source_stage_kind_id",
                    Value::String(stage_result.stage_kind_id.clone()),
                ),
                (
                    "terminal_result",
                    Value::String(stage_result.terminal_result.as_str().to_owned()),
                ),
                (
                    "target_stage",
                    Value::String(rule.target_stage.as_str().to_owned()),
                ),
                (
                    "target_skill_id",
                    rule.target_skill_id
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                ("preferred_output_paths", json!(rule.preferred_output_paths)),
                ("path", Value::String(queued_path.display().to_string())),
            ]),
            &stage_result.completed_at,
        )?;
        queued_paths.push(queued_path);
    }
    Ok(queued_paths)
}

fn learning_request_artifact_paths(
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> Vec<String> {
    let mut paths = vec![stage_result_path.display().to_string()];
    for artifact_path in &stage_result.artifact_paths {
        if !paths.contains(artifact_path) {
            paths.push(artifact_path.clone());
        }
    }
    paths
}

fn handle_learning_curator_promotion_boundary(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<Option<PathBuf>> {
    let artifacts = curator_skill_update_artifacts(stage_result);
    if artifacts.is_empty() {
        return Ok(None);
    }
    let state = if foreground_active_planes(&session.snapshot).is_empty() {
        "applied"
    } else {
        "deferred"
    };
    write_learning_curator_promotion_record(session, stage_result, &artifacts, state).map(Some)
}

fn apply_deferred_learning_promotions_if_safe(
    session: &RuntimeStartupSession,
    now: &Timestamp,
) -> RuntimeTickResult<usize> {
    if !foreground_active_planes(&session.snapshot).is_empty() {
        return Ok(0);
    }

    let deferred_dir = promotion_dir(&session.paths, "deferred");
    let applied_dir = promotion_dir(&session.paths, "applied");
    let mut applied_count = 0;
    for path in json_files(&deferred_dir)? {
        create_dir_all(&applied_dir)?;
        let target = applied_dir.join(
            path.file_name()
                .ok_or_else(|| invalid_state("deferred promotion path is missing filename"))?,
        );
        let raw = fs::read_to_string(&path).map_err(|error| io_error(&path, error))?;
        let mut payload: Value =
            serde_json::from_str(&raw).map_err(|error| RuntimeTickError::InvalidState {
                message: format!(
                    "learning promotion record {} is invalid: {error}",
                    path.display()
                ),
            })?;
        if let Value::Object(object) = &mut payload {
            object.insert("state".to_owned(), Value::String("applied".to_owned()));
            object.insert(
                "applied_at".to_owned(),
                Value::String(now.as_str().to_owned()),
            );
            write_pretty_json(&target, &payload)?;
        } else {
            write_runtime_text(&target, &raw)?;
        }
        fs::remove_file(&path).map_err(|error| io_error(&path, error))?;
        applied_count += 1;
        write_runtime_event(
            &session.paths,
            "learning_curator_promotion_applied",
            json_object([
                (
                    "promotion_record_path",
                    Value::String(path_relative_to_root(&session.paths, &target)),
                ),
                ("source", Value::String("deferred_safe_boundary".to_owned())),
            ]),
            now,
        )?;
    }
    Ok(applied_count)
}

fn curator_skill_update_artifacts(stage_result: &StageResultEnvelope) -> Vec<String> {
    if stage_result.plane != Plane::Learning
        || stage_result.stage != StageName::Curator
        || stage_result.terminal_result
            != TerminalResult::Learning(LearningTerminalResult::CuratorComplete)
    {
        return Vec::new();
    }
    stage_result
        .artifact_paths
        .iter()
        .filter(|artifact| is_skill_update_artifact(artifact))
        .cloned()
        .collect()
}

fn is_skill_update_artifact(raw_path: &str) -> bool {
    let name = Path::new(raw_path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(raw_path)
        .to_ascii_lowercase();
    name.contains("skill_update") || name.contains("skill-update")
}

fn write_learning_curator_promotion_record(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    artifacts: &[String],
    state: &str,
) -> RuntimeTickResult<PathBuf> {
    let directory = promotion_dir(&session.paths, state);
    create_dir_all(&directory)?;
    let record_path = directory.join(format!(
        "{}-{}.json",
        stage_result.run_id, stage_result.work_item_id
    ));
    let foreground_active_planes = foreground_active_plane_values(&session.snapshot);
    let payload = json!({
        "schema_version": "1.0",
        "kind": "learning_curator_promotion",
        "state": state,
        "run_id": stage_result.run_id,
        "work_item_id": stage_result.work_item_id,
        "artifact_paths": artifacts,
        "foreground_active_planes": foreground_active_planes,
        "recorded_at": stage_result.completed_at.as_str(),
    });
    write_pretty_json(&record_path, &payload)?;

    let event_type = if state == "deferred" {
        "learning_curator_promotion_deferred"
    } else {
        "learning_curator_promotion_applied"
    };
    write_runtime_event(
        &session.paths,
        event_type,
        json_object([
            (
                "promotion_record_path",
                Value::String(path_relative_to_root(&session.paths, &record_path)),
            ),
            (
                "work_item_id",
                Value::String(stage_result.work_item_id.clone()),
            ),
            ("artifact_paths", json!(artifacts)),
            ("foreground_active_planes", json!(foreground_active_planes)),
        ]),
        &stage_result.completed_at,
    )?;
    Ok(record_path)
}

fn promotion_dir(paths: &WorkspacePaths, state: &str) -> PathBuf {
    paths.learning_update_candidates_dir.join(state)
}

fn foreground_active_planes(snapshot: &RuntimeSnapshot) -> Vec<Plane> {
    [Plane::Planning, Plane::Execution]
        .into_iter()
        .filter(|plane| snapshot.active_runs_by_plane.contains_key(plane))
        .collect()
}

fn foreground_active_plane_values(snapshot: &RuntimeSnapshot) -> Vec<String> {
    foreground_active_planes(snapshot)
        .into_iter()
        .map(|plane| plane.as_str().to_owned())
        .collect()
}

fn apply_run_stage_decision(
    session: &mut RuntimeStartupSession,
    decision: &RouterDecision,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<()> {
    let next_plane = decision.next_plane.unwrap_or(stage_result.plane);
    if next_plane != stage_result.plane {
        return Err(invalid_state(
            "run_stage decisions must stay on the source plane",
        ));
    }
    let next_stage = decision
        .next_stage
        .ok_or_else(|| invalid_state("run_stage decision is missing next_stage"))?;
    let next_node_id = decision
        .next_node_id
        .clone()
        .unwrap_or_else(|| next_stage.as_str().to_owned());
    let next_stage_kind_id = decision
        .next_stage_kind_id
        .clone()
        .unwrap_or_else(|| next_stage.as_str().to_owned());

    set_next_stage_for_plane(
        &mut session.snapshot,
        stage_result.plane,
        next_stage,
        next_node_id,
        next_stage_kind_id,
        &stage_result.completed_at,
        decision.failure_class.clone(),
    )?;
    increment_route_counters(session, decision, stage_result)?;
    refresh_queue_depths(&session.paths, &mut session.snapshot)?;
    session.snapshot.updated_at = stage_result.completed_at.clone();
    save_snapshot(&session.paths, &session.snapshot)?;
    Ok(())
}

fn apply_idle_decision(
    session: &mut RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<()> {
    mark_active_work_item_complete(&session.paths, stage_result)?;
    clear_active_plane(
        &mut session.snapshot,
        stage_result.plane,
        None,
        &stage_result.completed_at,
    );
    reset_snapshot_route_counters(&mut session.snapshot);
    set_idle_status_after_terminal(session, stage_result.plane)?;
    reset_forward_progress_counters(
        &session.paths,
        stage_result.work_item_kind,
        &stage_result.work_item_id,
    )?;
    refresh_queue_depths(&session.paths, &mut session.snapshot)?;
    session.snapshot.updated_at = stage_result.completed_at.clone();
    save_snapshot(&session.paths, &session.snapshot)?;
    Ok(())
}

fn apply_handoff_decision(
    session: &mut RuntimeStartupSession,
    decision: &RouterDecision,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> RuntimeTickResult<Vec<PathBuf>> {
    let mut spawned_paths = Vec::new();
    if decision.create_incident {
        spawned_paths.push(enqueue_handoff_incident(
            session,
            decision,
            stage_result,
            stage_result_path,
        )?);
    }
    mark_active_work_item_blocked(&session.paths, stage_result)?;
    persist_blocked_metadata_and_event(session, decision, stage_result, stage_result_path)?;
    clear_active_plane(
        &mut session.snapshot,
        stage_result.plane,
        decision.failure_class.clone(),
        &stage_result.completed_at,
    );
    reset_snapshot_route_counters(&mut session.snapshot);
    reset_forward_progress_counters(
        &session.paths,
        stage_result.work_item_kind,
        &stage_result.work_item_id,
    )?;
    refresh_queue_depths(&session.paths, &mut session.snapshot)?;
    session.snapshot.updated_at = stage_result.completed_at.clone();
    save_snapshot(&session.paths, &session.snapshot)?;
    Ok(spawned_paths)
}

fn apply_blocked_decision(
    session: &mut RuntimeStartupSession,
    decision: &RouterDecision,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> RuntimeTickResult<()> {
    mark_active_work_item_blocked(&session.paths, stage_result)?;
    persist_blocked_metadata_and_event(session, decision, stage_result, stage_result_path)?;
    clear_active_plane(
        &mut session.snapshot,
        stage_result.plane,
        decision.failure_class.clone(),
        &stage_result.completed_at,
    );
    reset_snapshot_route_counters(&mut session.snapshot);
    reset_forward_progress_counters(
        &session.paths,
        stage_result.work_item_kind,
        &stage_result.work_item_id,
    )?;
    refresh_queue_depths(&session.paths, &mut session.snapshot)?;
    session.snapshot.updated_at = stage_result.completed_at.clone();
    save_snapshot(&session.paths, &session.snapshot)?;
    Ok(())
}

fn persist_blocked_metadata_and_event(
    session: &RuntimeStartupSession,
    decision: &RouterDecision,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> RuntimeTickResult<PathBuf> {
    let record =
        persist_blocked_item_metadata(&session.paths, stage_result, decision, stage_result_path)?;
    write_runtime_event(
        &session.paths,
        "blocked_item_metadata_written",
        json_object([
            (
                "work_item_kind",
                Value::String(record.metadata.work_item_kind.as_str().to_owned()),
            ),
            (
                "work_item_id",
                Value::String(record.metadata.work_item_id.clone()),
            ),
            (
                "failure_class",
                Value::String(record.metadata.failure_class.clone()),
            ),
            (
                "failure_scope",
                Value::String(record.metadata.failure_scope.as_str().to_owned()),
            ),
            (
                "auto_requeue_candidate",
                Value::Bool(record.metadata.auto_requeue_candidate),
            ),
            (
                "metadata_path",
                Value::String(path_relative_to_root(&session.paths, &record.path)),
            ),
        ]),
        &stage_result.completed_at,
    )?;
    Ok(record.path)
}

fn is_recon_stage_result(stage_result: &StageResultEnvelope) -> bool {
    stage_result.stage_kind_id == "recon" && stage_result.work_item_kind == WorkItemKind::Probe
}

fn apply_recon_router_decision(
    session: &mut RuntimeStartupSession,
    decision: &RouterDecision,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> RuntimeTickResult<Vec<PathBuf>> {
    match apply_recon_router_decision_inner(session, decision, stage_result, stage_result_path) {
        Ok(paths) => Ok(paths),
        Err(error) => {
            block_invalid_recon_handoff(session, decision, stage_result, stage_result_path, error)
        }
    }
}

fn apply_recon_router_decision_inner(
    session: &mut RuntimeStartupSession,
    decision: &RouterDecision,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> RuntimeTickResult<Vec<PathBuf>> {
    let terminal_result = match stage_result.terminal_result {
        TerminalResult::Planning(result) => result,
        _ => {
            return Err(invalid_state(
                "recon stage results must use planning terminal results",
            ));
        }
    };
    match terminal_result {
        PlanningTerminalResult::ReconToExecution
        | PlanningTerminalResult::ReconToPlanning
        | PlanningTerminalResult::ReconNoop => {
            if decision.action != RouterAction::Idle {
                return Err(invalid_state(
                    "successful recon terminal results require an idle router decision",
                ));
            }
        }
        PlanningTerminalResult::ReconBlocked | PlanningTerminalResult::Blocked => {
            if decision.action != RouterAction::Blocked {
                return Err(invalid_state(
                    "blocked recon terminal results require a blocked router decision",
                ));
            }
        }
        _ => {
            return Err(invalid_state(format!(
                "unsupported recon terminal result: {}",
                stage_result.terminal_result.as_str()
            )));
        }
    }

    let packet = read_and_validate_recon_packet(session, stage_result, terminal_result)?;
    match terminal_result {
        PlanningTerminalResult::ReconToExecution => {
            persist_recon_packet(session, &packet)?;
            let task = generated_recon_task(session, stage_result, &packet)?;
            let spawned_path = QueueStore::from_paths(session.paths.clone())
                .enqueue_task(&task)
                .map_err(RuntimeTickError::from)?;
            apply_idle_decision(session, stage_result)?;
            Ok(vec![spawned_path])
        }
        PlanningTerminalResult::ReconToPlanning => {
            persist_recon_packet(session, &packet)?;
            let spec = generated_recon_spec(session, stage_result, &packet)?;
            let spawned_path = QueueStore::from_paths(session.paths.clone())
                .enqueue_spec(&spec)
                .map_err(RuntimeTickError::from)?;
            apply_idle_decision(session, stage_result)?;
            Ok(vec![spawned_path])
        }
        PlanningTerminalResult::ReconNoop => {
            persist_recon_packet(session, &packet)?;
            apply_idle_decision(session, stage_result)?;
            Ok(Vec::new())
        }
        PlanningTerminalResult::ReconBlocked | PlanningTerminalResult::Blocked => {
            persist_recon_packet(session, &packet)?;
            apply_blocked_decision(session, decision, stage_result, stage_result_path)?;
            Ok(Vec::new())
        }
        _ => unreachable!("unsupported recon terminal result was rejected"),
    }
}

fn block_invalid_recon_handoff(
    session: &mut RuntimeStartupSession,
    source_decision: &RouterDecision,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
    error: RuntimeTickError,
) -> RuntimeTickResult<Vec<PathBuf>> {
    let report_path = session
        .paths
        .runs_dir
        .join(&stage_result.run_id)
        .join("runtime_error_report.md");
    let context = RuntimeErrorContext {
        schema_version: "1.0".to_owned(),
        kind: "runtime_error_context".to_owned(),
        error_code: RuntimeErrorCode::ReconHandoffInvalid,
        plane: stage_result.plane,
        failed_stage: stage_result.stage,
        repair_stage: StageName::Recon,
        work_item_kind: stage_result.work_item_kind,
        work_item_id: stage_result.work_item_id.clone(),
        run_id: stage_result.run_id.clone(),
        router_action: Some(source_decision.action.as_str().to_owned()),
        terminal_result: Some(stage_result.terminal_result),
        stage_result_path: Some(path_relative_to_root(&session.paths, stage_result_path)),
        report_path: report_path.display().to_string(),
        exception_type: runtime_tick_error_type(&error).to_owned(),
        exception_message: error.to_string(),
        captured_at: stage_result.completed_at.clone(),
    };
    context.validate().map_err(|error| {
        invalid_state(format!("runtime error context validation failed: {error}"))
    })?;
    write_runtime_error_report(&report_path, &context)?;
    write_pretty_json(&session.paths.runtime_error_context_file, &context)?;

    set_status_for_plane(&session.paths, stage_result.plane, "### BLOCKED")?;
    set_snapshot_status_for_plane(&mut session.snapshot, stage_result.plane, "### BLOCKED");
    let blocked_decision = RouterDecision {
        action: RouterAction::Blocked,
        next_plane: None,
        next_stage: None,
        reason: RuntimeErrorCode::ReconHandoffInvalid.as_str().to_owned(),
        next_node_id: None,
        next_stage_kind_id: None,
        failure_class: Some(RuntimeErrorCode::ReconHandoffInvalid.as_str().to_owned()),
        counter_key: None,
        create_incident: false,
    };
    apply_blocked_decision(session, &blocked_decision, stage_result, stage_result_path)?;
    Ok(Vec::new())
}

fn read_and_validate_recon_packet(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    terminal_result: PlanningTerminalResult,
) -> RuntimeTickResult<ReconPacketDocument> {
    let source = session
        .paths
        .runs_dir
        .join(&stage_result.run_id)
        .join("recon_packet.md");
    let packet =
        read_recon_packet(&source).map_err(|source_error| RuntimeTickError::InvalidState {
            message: format!(
                "failed to read recon packet {}: {source_error}",
                source.display()
            ),
        })?;
    validate_recon_packet_for_stage_result(&packet, stage_result, terminal_result)?;
    Ok(packet)
}

fn persist_recon_packet(
    session: &RuntimeStartupSession,
    packet: &ReconPacketDocument,
) -> RuntimeTickResult<PathBuf> {
    let destination = session
        .paths
        .recon_packets_dir
        .join(format!("{}.md", packet.recon_packet_id));
    if destination.exists() {
        return Err(invalid_state(format!(
            "recon packet already exists: {}",
            destination.display()
        )));
    }
    write_runtime_text(&destination, &render_recon_packet(packet))?;
    Ok(destination)
}

fn validate_recon_packet_for_stage_result(
    packet: &ReconPacketDocument,
    stage_result: &StageResultEnvelope,
    terminal_result: PlanningTerminalResult,
) -> RuntimeTickResult<()> {
    if packet.probe_id != stage_result.work_item_id {
        return Err(invalid_state(
            "recon packet probe_id must match active probe",
        ));
    }
    let expected_decision = recon_decision_for_terminal(terminal_result)?;
    if packet.decision != expected_decision {
        return Err(invalid_state(
            "recon packet decision must match terminal result",
        ));
    }
    Ok(())
}

fn recon_decision_for_terminal(
    terminal_result: PlanningTerminalResult,
) -> RuntimeTickResult<ReconDecision> {
    Ok(match terminal_result {
        PlanningTerminalResult::ReconToExecution => ReconDecision::ToExecution,
        PlanningTerminalResult::ReconToPlanning => ReconDecision::ToPlanning,
        PlanningTerminalResult::ReconNoop => ReconDecision::Noop,
        PlanningTerminalResult::ReconBlocked | PlanningTerminalResult::Blocked => {
            ReconDecision::Blocked
        }
        _ => {
            return Err(invalid_state(format!(
                "unsupported recon terminal result: {}",
                terminal_result.as_str()
            )));
        }
    })
}

fn generated_recon_task(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    packet: &ReconPacketDocument,
) -> RuntimeTickResult<TaskDocument> {
    let emitted_task_id = packet
        .emitted_task_id
        .as_deref()
        .ok_or_else(|| invalid_state("recon execution route requires emitted_task_id"))?;
    let source = session
        .paths
        .runs_dir
        .join(&stage_result.run_id)
        .join("generated_task.md");
    let mut task = read_generated_task_artifact(&source)?;
    if task.task_id != emitted_task_id {
        return Err(invalid_state(
            "generated task id must match recon packet emitted_task_id",
        ));
    }
    apply_probe_task_lineage(&mut task, packet);
    Ok(task)
}

fn generated_recon_spec(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    packet: &ReconPacketDocument,
) -> RuntimeTickResult<SpecDocument> {
    let emitted_spec_id = packet
        .emitted_spec_id
        .as_deref()
        .ok_or_else(|| invalid_state("recon planning route requires emitted_spec_id"))?;
    let source = session
        .paths
        .runs_dir
        .join(&stage_result.run_id)
        .join("generated_spec.md");
    let mut spec = read_generated_spec_artifact(&source)?;
    if spec.spec_id != emitted_spec_id {
        return Err(invalid_state(
            "generated spec id must match recon packet emitted_spec_id",
        ));
    }
    apply_probe_spec_lineage(&mut spec, packet);
    Ok(spec)
}

fn read_generated_task_artifact(path: &Path) -> RuntimeTickResult<TaskDocument> {
    let raw = fs::read_to_string(path).map_err(|error| io_error(path, error))?;
    if raw.trim_start().starts_with('{') {
        parse_task_json_import_with_source(&raw, &path.display().to_string()).map_err(|source| {
            RuntimeTickError::WorkDocument {
                path: path.to_path_buf(),
                source,
            }
        })
    } else {
        parse_task_document_with_source(&raw, &path.display().to_string()).map_err(|source| {
            RuntimeTickError::WorkDocument {
                path: path.to_path_buf(),
                source,
            }
        })
    }
}

fn read_generated_spec_artifact(path: &Path) -> RuntimeTickResult<SpecDocument> {
    let raw = fs::read_to_string(path).map_err(|error| io_error(path, error))?;
    if raw.trim_start().starts_with('{') {
        parse_spec_json_import_with_source(&raw, &path.display().to_string()).map_err(|source| {
            RuntimeTickError::WorkDocument {
                path: path.to_path_buf(),
                source,
            }
        })
    } else {
        parse_spec_document_with_source(&raw, &path.display().to_string()).map_err(|source| {
            RuntimeTickError::WorkDocument {
                path: path.to_path_buf(),
                source,
            }
        })
    }
}

fn apply_probe_task_lineage(task: &mut TaskDocument, packet: &ReconPacketDocument) {
    task.root_intake_kind = Some(RootIntakeKind::Probe);
    task.root_intake_id = Some(packet.probe_id.clone());
    append_required_recon_references(&mut task.references, packet);
}

fn apply_probe_spec_lineage(spec: &mut SpecDocument, packet: &ReconPacketDocument) {
    spec.source_type = SpecSourceType::Probe;
    spec.source_id = Some(packet.probe_id.clone());
    spec.root_intake_kind = Some(RootIntakeKind::Probe);
    spec.root_intake_id = Some(packet.probe_id.clone());
    if spec.root_spec_id.is_none() {
        spec.root_spec_id = Some(spec.spec_id.clone());
    }
    append_required_recon_references(&mut spec.references, packet);
}

fn append_required_recon_references(references: &mut Vec<String>, packet: &ReconPacketDocument) {
    for reference in [
        format!("millrace-agents/probes/active/{}.md", packet.probe_id),
        format!(
            "millrace-agents/recon/packets/{}.md",
            packet.recon_packet_id
        ),
    ] {
        if !references.iter().any(|existing| existing == &reference) {
            references.push(reference);
        }
    }
}

fn apply_closure_target_result(
    session: &mut RuntimeStartupSession,
    decision: &RouterDecision,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> RuntimeTickResult<Vec<PathBuf>> {
    let mut target = load_closure_target_for_stage_result(&session.paths, stage_result)?;
    let previous_arbiter_run_id = target.last_arbiter_run_id.clone();
    target.latest_verdict_path = existing_workspace_artifact(
        &session.paths,
        metadata_string(stage_result, "preferred_verdict_path").as_deref(),
    )?;
    target.latest_report_path = canonicalize_arbiter_report(&session.paths, stage_result)?;
    target.last_arbiter_run_id = Some(stage_result.run_id.clone());
    target.closure_blocked_by_lineage_work = false;
    target.blocking_work_ids.clear();

    match decision.action {
        RouterAction::Idle => {
            target.closure_open = false;
            target.closed_at = Some(stage_result.completed_at.clone());
            save_closure_target_state(&session.paths, &target)?;
            clear_active_plane(
                &mut session.snapshot,
                stage_result.plane,
                None,
                &stage_result.completed_at,
            );
            reset_snapshot_route_counters(&mut session.snapshot);
            set_idle_status_after_terminal(session, stage_result.plane)?;
            refresh_queue_depths(&session.paths, &mut session.snapshot)?;
            session.snapshot.updated_at = stage_result.completed_at.clone();
            save_snapshot(&session.paths, &session.snapshot)?;
            write_closure_target_event(
                &session.paths,
                "closure_target_closed",
                &target,
                stage_result,
                decision,
                None,
            )?;
            Ok(Vec::new())
        }
        RouterAction::Handoff => {
            target.closure_open = true;
            target.closed_at = None;
            save_closure_target_state(&session.paths, &target)?;
            if is_repeated_remediation_without_execution(
                &session.paths,
                previous_arbiter_run_id.as_deref(),
            )? {
                block_repeated_closure_remediation(session, &target, stage_result)?;
                return Ok(Vec::new());
            }
            let incident_path = if decision.create_incident {
                Some(enqueue_handoff_incident(
                    session,
                    decision,
                    stage_result,
                    stage_result_path,
                )?)
            } else {
                None
            };
            clear_active_plane(
                &mut session.snapshot,
                stage_result.plane,
                decision.failure_class.clone(),
                &stage_result.completed_at,
            );
            reset_snapshot_route_counters(&mut session.snapshot);
            refresh_queue_depths(&session.paths, &mut session.snapshot)?;
            session.snapshot.updated_at = stage_result.completed_at.clone();
            save_snapshot(&session.paths, &session.snapshot)?;
            write_closure_target_event(
                &session.paths,
                "closure_target_remediation_requested",
                &target,
                stage_result,
                decision,
                incident_path.as_deref(),
            )?;
            Ok(incident_path.into_iter().collect())
        }
        RouterAction::Blocked => {
            target.closure_open = true;
            target.closed_at = None;
            save_closure_target_state(&session.paths, &target)?;
            clear_active_plane(
                &mut session.snapshot,
                stage_result.plane,
                decision.failure_class.clone(),
                &stage_result.completed_at,
            );
            reset_snapshot_route_counters(&mut session.snapshot);
            refresh_queue_depths(&session.paths, &mut session.snapshot)?;
            session.snapshot.updated_at = stage_result.completed_at.clone();
            save_snapshot(&session.paths, &session.snapshot)?;
            write_closure_target_event(
                &session.paths,
                "closure_target_blocked",
                &target,
                stage_result,
                decision,
                None,
            )?;
            Ok(Vec::new())
        }
        RouterAction::RunStage => Err(invalid_state(
            "closure-target results cannot route to run_stage",
        )),
    }
}

fn load_closure_target_for_stage_result(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<ClosureTargetState> {
    let root_spec_id = metadata_string(stage_result, "closure_target_root_spec_id")
        .ok_or_else(|| invalid_state("closure_target_root_spec_id is required"))?;
    load_closure_target_state(paths, &root_spec_id).map_err(Into::into)
}

fn existing_workspace_artifact(
    paths: &WorkspacePaths,
    candidate: Option<&str>,
) -> RuntimeTickResult<Option<String>> {
    let Some(candidate) = candidate.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let path = rooted_candidate_path(paths, candidate);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(path_relative_to_root(paths, &path)))
}

fn canonicalize_arbiter_report(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<Option<String>> {
    let Some(report_artifact) = stage_result
        .report_artifact
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(None);
    };
    let source_path = rooted_candidate_path(paths, report_artifact);
    if !source_path.exists() {
        return Ok(None);
    }
    let destination = paths
        .arbiter_reports_dir
        .join(format!("{}.md", stage_result.run_id));
    if let Some(parent) = destination.parent() {
        create_dir_all(parent)?;
    }
    fs::copy(&source_path, &destination).map_err(|error| io_error(&destination, error))?;
    Ok(Some(path_relative_to_root(paths, &destination)))
}

fn rooted_candidate_path(paths: &WorkspacePaths, candidate: &str) -> PathBuf {
    let path = PathBuf::from(candidate);
    if path.is_absolute() {
        path
    } else {
        paths.root.join(path)
    }
}

fn is_repeated_remediation_without_execution(
    paths: &WorkspacePaths,
    previous_arbiter_run_id: Option<&str>,
) -> RuntimeTickResult<bool> {
    let Some(previous_arbiter_run_id) = previous_arbiter_run_id else {
        return Ok(false);
    };
    let event_log = paths.logs_dir.join("runtime_events.jsonl");
    let raw = match fs::read_to_string(&event_log) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(io_error(&event_log, error)),
    };
    let mut seen_previous_arbiter = false;
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value =
            serde_json::from_str(line).map_err(|error| RuntimeTickError::InvalidState {
                message: format!(
                    "runtime event log {} is invalid: {error}",
                    event_log.display()
                ),
            })?;
        let data = event.get("data").and_then(Value::as_object);
        if data
            .and_then(|payload| payload.get("run_id"))
            .and_then(Value::as_str)
            == Some(previous_arbiter_run_id)
        {
            seen_previous_arbiter = true;
            continue;
        }
        if !seen_previous_arbiter {
            continue;
        }
        if event.get("event_type").and_then(Value::as_str) != Some("stage_completed") {
            continue;
        }
        if data
            .and_then(|payload| payload.get("plane"))
            .and_then(Value::as_str)
            == Some(Plane::Execution.as_str())
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn block_repeated_closure_remediation(
    session: &mut RuntimeStartupSession,
    target: &ClosureTargetState,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<()> {
    let failure_class = "closure_repeated_remediation_without_execution";
    set_status_for_plane(&session.paths, Plane::Planning, "### BLOCKED")?;
    set_snapshot_status_for_plane(&mut session.snapshot, Plane::Planning, "### BLOCKED");
    clear_active_plane(
        &mut session.snapshot,
        Plane::Planning,
        Some(failure_class.to_owned()),
        &stage_result.completed_at,
    );
    reset_snapshot_route_counters(&mut session.snapshot);
    refresh_queue_depths(&session.paths, &mut session.snapshot)?;
    session.snapshot.updated_at = stage_result.completed_at.clone();
    save_snapshot(&session.paths, &session.snapshot)?;
    write_runtime_event(
        &session.paths,
        "closure_repeated_remediation_blocked",
        json_object([
            ("root_spec_id", Value::String(target.root_spec_id.clone())),
            ("failure_class", Value::String(failure_class.to_owned())),
            ("run_id", Value::String(stage_result.run_id.clone())),
        ]),
        &stage_result.completed_at,
    )?;
    Ok(())
}

fn write_closure_target_event(
    paths: &WorkspacePaths,
    event_type: &str,
    target: &ClosureTargetState,
    stage_result: &StageResultEnvelope,
    decision: &RouterDecision,
    incident_path: Option<&Path>,
) -> RuntimeTickResult<PathBuf> {
    write_runtime_event(
        paths,
        event_type,
        json_object([
            ("root_spec_id", Value::String(target.root_spec_id.clone())),
            ("root_idea_id", Value::String(target.root_idea_id.clone())),
            ("run_id", Value::String(stage_result.run_id.clone())),
            (
                "terminal_result",
                Value::String(stage_result.terminal_result.as_str().to_owned()),
            ),
            ("reason", Value::String(decision.reason.clone())),
            (
                "latest_verdict_path",
                target
                    .latest_verdict_path
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            ),
            (
                "latest_report_path",
                target
                    .latest_report_path
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            ),
            (
                "incident_path",
                incident_path
                    .map(|path| Value::String(path_relative_to_root(paths, path)))
                    .unwrap_or(Value::Null),
            ),
        ]),
        &stage_result.completed_at,
    )
}

fn mark_active_work_item_complete(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<PathBuf> {
    let queue = QueueStore::from_paths(paths.clone());
    Ok(match stage_result.work_item_kind {
        WorkItemKind::Task => queue.mark_task_done(&stage_result.work_item_id)?,
        WorkItemKind::Probe => queue.mark_probe_done(&stage_result.work_item_id)?,
        WorkItemKind::Spec => queue.mark_spec_done(&stage_result.work_item_id)?,
        WorkItemKind::Incident => queue.mark_incident_resolved(&stage_result.work_item_id)?,
        WorkItemKind::LearningRequest => {
            queue.mark_learning_request_done(&stage_result.work_item_id)?
        }
    })
}

fn mark_active_work_item_blocked(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<PathBuf> {
    let queue = QueueStore::from_paths(paths.clone());
    Ok(match stage_result.work_item_kind {
        WorkItemKind::Task => queue.mark_task_blocked(&stage_result.work_item_id)?,
        WorkItemKind::Probe => queue.mark_probe_blocked(&stage_result.work_item_id)?,
        WorkItemKind::Spec => queue.mark_spec_blocked(&stage_result.work_item_id)?,
        WorkItemKind::Incident => queue.mark_incident_blocked(&stage_result.work_item_id)?,
        WorkItemKind::LearningRequest => {
            queue.mark_learning_request_blocked(&stage_result.work_item_id)?
        }
    })
}

fn set_next_stage_for_plane(
    snapshot: &mut RuntimeSnapshot,
    plane: Plane,
    stage: StageName,
    node_id: String,
    stage_kind_id: String,
    now: &Timestamp,
    current_failure_class: Option<String>,
) -> RuntimeTickResult<()> {
    let active_run = snapshot
        .active_runs_by_plane
        .get_mut(&plane)
        .ok_or_else(|| invalid_state(format!("no active run for plane {}", plane.as_str())))?;
    active_run.stage = stage;
    active_run.node_id = node_id;
    active_run.stage_kind_id = stage_kind_id;
    active_run.active_since = now.clone();
    active_run.running_status_marker = None;
    project_foreground_active_run(snapshot);
    snapshot.current_failure_class = current_failure_class;
    snapshot.updated_at = now.clone();
    Ok(())
}

fn clear_active_plane(
    snapshot: &mut RuntimeSnapshot,
    plane: Plane,
    current_failure_class: Option<String>,
    now: &Timestamp,
) {
    snapshot.active_runs_by_plane.remove(&plane);
    project_foreground_active_run(snapshot);
    snapshot.current_failure_class = current_failure_class;
    snapshot.updated_at = now.clone();
}

fn reset_snapshot_route_counters(snapshot: &mut RuntimeSnapshot) {
    snapshot.troubleshoot_attempt_count = 0;
    snapshot.mechanic_attempt_count = 0;
    snapshot.fix_cycle_count = 0;
    snapshot.consultant_invocations = 0;
}

fn set_idle_status_after_terminal(
    session: &mut RuntimeStartupSession,
    completed_plane: Plane,
) -> RuntimeTickResult<()> {
    if session.snapshot.active_runs_by_plane.is_empty() {
        for plane in [Plane::Execution, Plane::Planning, Plane::Learning] {
            set_status_for_plane(&session.paths, plane, IDLE_STATUS_MARKER)?;
            set_snapshot_status_for_plane(&mut session.snapshot, plane, IDLE_STATUS_MARKER);
        }
    } else {
        set_status_for_plane(&session.paths, completed_plane, IDLE_STATUS_MARKER)?;
        set_snapshot_status_for_plane(&mut session.snapshot, completed_plane, IDLE_STATUS_MARKER);
    }
    Ok(())
}

fn increment_route_counters(
    session: &mut RuntimeStartupSession,
    decision: &RouterDecision,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<()> {
    let Some(field) = route_counter_field(decision, stage_result) else {
        return Ok(());
    };
    let failure_class = decision.failure_class.as_deref().unwrap_or(match field {
        RecoveryCounterField::FixCycleCount => "fix_cycle",
        _ => "recoverable_failure",
    });
    increment_counter_field(
        &session.paths,
        &mut session.snapshot,
        failure_class,
        stage_result.work_item_kind,
        &stage_result.work_item_id,
        field,
        &stage_result.completed_at,
    )
}

fn route_counter_field(
    decision: &RouterDecision,
    stage_result: &StageResultEnvelope,
) -> Option<RecoveryCounterField> {
    match decision.next_stage {
        Some(StageName::Troubleshooter) => Some(RecoveryCounterField::TroubleshootAttemptCount),
        Some(StageName::Mechanic) => Some(RecoveryCounterField::MechanicAttemptCount),
        Some(StageName::Consultant) => Some(RecoveryCounterField::ConsultantInvocations),
        _ if stage_result.terminal_result.as_str() == "FIX_NEEDED" => {
            Some(RecoveryCounterField::FixCycleCount)
        }
        _ => None,
    }
}

fn increment_counter_field(
    paths: &WorkspacePaths,
    snapshot: &mut RuntimeSnapshot,
    failure_class: &str,
    work_item_kind: WorkItemKind,
    work_item_id: &str,
    field: RecoveryCounterField,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    if failure_class.trim().is_empty() {
        return Err(invalid_state("failure_class is required"));
    }
    if work_item_id.trim().is_empty() {
        return Err(invalid_state("work_item_id is required"));
    }
    let mut counters = load_recovery_counters(paths)?;
    let entry = recovery_counter_entry_mut(
        &mut counters,
        failure_class,
        work_item_kind,
        work_item_id,
        now,
    );
    match field {
        RecoveryCounterField::TroubleshootAttemptCount => {
            entry.troubleshoot_attempt_count += 1;
            snapshot.troubleshoot_attempt_count = entry.troubleshoot_attempt_count;
        }
        RecoveryCounterField::MechanicAttemptCount => {
            entry.mechanic_attempt_count += 1;
            snapshot.mechanic_attempt_count = entry.mechanic_attempt_count;
        }
        RecoveryCounterField::FixCycleCount => {
            entry.fix_cycle_count += 1;
            snapshot.fix_cycle_count = entry.fix_cycle_count;
        }
        RecoveryCounterField::ConsultantInvocations => {
            entry.consultant_invocations += 1;
            snapshot.consultant_invocations = entry.consultant_invocations;
        }
    }
    entry.last_updated_at = now.clone();
    save_recovery_counters(paths, &counters)?;
    Ok(())
}

fn recovery_counter_entry_mut<'a>(
    counters: &'a mut RecoveryCounters,
    failure_class: &str,
    work_item_kind: WorkItemKind,
    work_item_id: &str,
    now: &Timestamp,
) -> &'a mut crate::contracts::RecoveryCounterEntry {
    if let Some(index) = counters.entries.iter().position(|entry| {
        entry.failure_class == failure_class
            && entry.work_item_kind == work_item_kind
            && entry.work_item_id == work_item_id
    }) {
        return &mut counters.entries[index];
    }
    counters
        .entries
        .push(crate::contracts::RecoveryCounterEntry {
            failure_class: failure_class.to_owned(),
            work_item_kind,
            work_item_id: work_item_id.to_owned(),
            troubleshoot_attempt_count: 0,
            mechanic_attempt_count: 0,
            fix_cycle_count: 0,
            consultant_invocations: 0,
            last_updated_at: now.clone(),
        });
    counters
        .entries
        .last_mut()
        .expect("counter entry was just pushed")
}

fn enqueue_handoff_incident(
    session: &RuntimeStartupSession,
    decision: &RouterDecision,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> RuntimeTickResult<PathBuf> {
    let lineage = work_item_lineage_for_stage_result(&session.paths, stage_result)?;
    let root_spec_id =
        metadata_string(stage_result, "closure_target_root_spec_id").or(lineage.root_spec_id);
    let root_idea_id =
        metadata_string(stage_result, "closure_target_root_idea_id").or(lineage.root_idea_id);
    let is_closure_target = is_closure_target_stage_result(stage_result);
    let source_task_id = if is_closure_target {
        None
    } else {
        lineage.source_task_id
    };
    let source_spec_id = if is_closure_target {
        root_spec_id.clone()
    } else {
        lineage.source_spec_id
    };
    let incident_id = if is_closure_target {
        format!(
            "arbiter-gap-{}-{}",
            root_spec_id
                .as_deref()
                .unwrap_or(&stage_result.work_item_id),
            stable_short_hash(stage_result, decision)
        )
    } else {
        format!(
            "incident-{}-{}",
            stage_result.work_item_id,
            stable_short_hash(stage_result, decision)
        )
    };

    let mut evidence_paths = stage_result.artifact_paths.clone();
    push_unique(
        &mut evidence_paths,
        path_relative_to_root(&session.paths, stage_result_path),
    );
    for key in [
        "preferred_rubric_path",
        "preferred_verdict_path",
        "preferred_report_path",
    ] {
        if let Some(value) = metadata_string(stage_result, key) {
            push_unique(&mut evidence_paths, value);
        }
    }

    let title = if is_closure_target {
        format!(
            "Arbiter remediation for {}",
            root_spec_id
                .as_deref()
                .unwrap_or(&stage_result.work_item_id)
        )
    } else {
        format!(
            "Planning handoff for {} {}",
            stage_result.work_item_kind.as_str(),
            stage_result.work_item_id
        )
    };
    let summary = if is_closure_target {
        format!(
            "Arbiter found parity gaps for root spec {}; planning remediation required.",
            root_spec_id
                .as_deref()
                .unwrap_or(&stage_result.work_item_id)
        )
    } else {
        format!(
            "Stage {} returned {}; planning remediation required.",
            stage_result.stage.as_str(),
            stage_result.terminal_result.as_str()
        )
    };
    let failure_class = decision.failure_class.clone().unwrap_or_else(|| {
        if is_closure_target {
            "arbiter_parity_gap".to_owned()
        } else {
            "consultant_needs_planning".to_owned()
        }
    });
    let related_stage_result = session
        .snapshot
        .last_stage_result_path
        .clone()
        .unwrap_or_else(|| path_relative_to_root(&session.paths, stage_result_path));
    let document = IncidentDocument {
        incident_id: incident_id.clone(),
        title,
        summary,
        root_idea_id,
        root_spec_id,
        root_intake_kind: None,
        root_intake_id: None,
        source_task_id,
        source_spec_id,
        source_stage: stage_result.stage,
        source_plane: stage_result.plane,
        failure_class,
        severity: IncidentSeverity::High,
        needs_planning: true,
        trigger_reason: decision.reason.clone(),
        observed_symptoms: stage_result.notes.clone(),
        failed_attempts: Vec::new(),
        consultant_decision: IncidentDecision::NeedsPlanning,
        evidence_paths,
        related_run_ids: vec![stage_result.run_id.clone()],
        related_stage_results: vec![related_stage_result],
        references: Vec::new(),
        opened_at: stage_result.completed_at.clone(),
        opened_by: "runtime".to_owned(),
        updated_at: None,
    };
    let destination = QueueStore::from_paths(session.paths.clone()).enqueue_incident(&document)?;
    write_runtime_event(
        &session.paths,
        "runtime_handoff_incident_enqueued",
        json_object([
            ("incident_id", Value::String(incident_id)),
            (
                "source_work_item_kind",
                Value::String(stage_result.work_item_kind.as_str().to_owned()),
            ),
            (
                "source_work_item_id",
                Value::String(stage_result.work_item_id.clone()),
            ),
            (
                "destination",
                Value::String(path_relative_to_root(&session.paths, &destination)),
            ),
        ]),
        &stage_result.completed_at,
    )?;
    Ok(destination)
}

fn work_item_lineage_for_stage_result(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<WorkItemLineage> {
    if is_closure_target_stage_result(stage_result) {
        return Ok(WorkItemLineage::default());
    }
    let Some(path) = active_work_item_path(
        paths,
        Some(stage_result.work_item_kind),
        Some(&stage_result.work_item_id),
    ) else {
        return Ok(WorkItemLineage::default());
    };
    if !path.exists() {
        return Ok(WorkItemLineage::default());
    }
    let raw = fs::read_to_string(&path).map_err(|error| io_error(&path, error))?;
    match stage_result.work_item_kind {
        WorkItemKind::Task => {
            let document = parse_task_document_with_source(&raw, &path.display().to_string())
                .map_err(|source| RuntimeTickError::WorkDocument {
                    path: path.clone(),
                    source,
                })?;
            Ok(WorkItemLineage {
                root_idea_id: document.root_idea_id,
                root_spec_id: document.root_spec_id.clone().or(document.spec_id.clone()),
                source_task_id: Some(document.task_id),
                source_spec_id: document.spec_id.or(document.root_spec_id),
            })
        }
        WorkItemKind::Spec => {
            let document = parse_spec_document_with_source(&raw, &path.display().to_string())
                .map_err(|source| RuntimeTickError::WorkDocument {
                    path: path.clone(),
                    source,
                })?;
            Ok(WorkItemLineage {
                root_idea_id: document.root_idea_id,
                root_spec_id: document
                    .root_spec_id
                    .clone()
                    .or(Some(document.spec_id.clone())),
                source_task_id: None,
                source_spec_id: Some(document.spec_id),
            })
        }
        WorkItemKind::Incident => {
            let document = parse_incident_document_with_source(&raw, &path.display().to_string())
                .map_err(|source| RuntimeTickError::WorkDocument {
                path: path.clone(),
                source,
            })?;
            Ok(WorkItemLineage {
                root_idea_id: document.root_idea_id,
                root_spec_id: document
                    .root_spec_id
                    .clone()
                    .or(document.source_spec_id.clone()),
                source_task_id: document.source_task_id,
                source_spec_id: document.source_spec_id,
            })
        }
        WorkItemKind::Probe | WorkItemKind::LearningRequest => Ok(WorkItemLineage::default()),
    }
}

fn clear_runtime_error_context_if_consumed(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<()> {
    if !matches!(
        stage_result.stage,
        StageName::Troubleshooter | StageName::Mechanic
    ) {
        return Ok(());
    }
    match fs::remove_file(&session.paths.runtime_error_context_file) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(&session.paths.runtime_error_context_file, error)),
    }
}

fn is_closure_target_stage_result(stage_result: &StageResultEnvelope) -> bool {
    metadata_string(stage_result, "request_kind").as_deref() == Some("closure_target")
}

fn metadata_string(stage_result: &StageResultEnvelope, key: &str) -> Option<String> {
    stage_result
        .metadata
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn stable_short_hash(stage_result: &StageResultEnvelope, decision: &RouterDecision) -> String {
    let mut digest = Sha256::new();
    digest.update(stage_result.run_id.as_bytes());
    digest.update(stage_result.work_item_id.as_bytes());
    digest.update(stage_result.terminal_result.as_str().as_bytes());
    digest.update(decision.reason.as_bytes());
    digest
        .finalize()
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryCounterField {
    TroubleshootAttemptCount,
    MechanicAttemptCount,
    FixCycleCount,
    ConsultantInvocations,
}

#[derive(Debug, Default)]
struct WorkItemLineage {
    root_idea_id: Option<String>,
    root_spec_id: Option<String>,
    source_task_id: Option<String>,
    source_spec_id: Option<String>,
}

#[derive(Debug, Default)]
struct RuntimeErrorRequestFields {
    runtime_error_code: Option<String>,
    runtime_error_report_path: Option<String>,
    runtime_error_catalog_path: Option<String>,
}

#[derive(Debug)]
struct StageWorkItemOwnershipViolation {
    reason: &'static str,
    expected_work_item_kinds: Vec<WorkItemKind>,
    message: String,
}

#[derive(Debug, Default)]
struct StageWorkItemOwnershipContextPaths {
    context_path: Option<PathBuf>,
    report_path: Option<PathBuf>,
}

#[derive(Debug)]
struct DispatchApplicationOutput {
    terminal_marker_path: PathBuf,
    router_decision: RouterDecision,
    router_decision_path: PathBuf,
    runtime_error_context_path: Option<PathBuf>,
    event_log_path: PathBuf,
}

#[derive(Debug)]
struct DispatchApplicationError {
    source: Box<RuntimeTickError>,
    terminal_marker_path: Option<PathBuf>,
    router_decision: Option<Box<RouterDecision>>,
    stage_result_path: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct DispatchApplicationPartial {
    terminal_marker_path: Option<PathBuf>,
    router_decision: Option<RouterDecision>,
    stage_result_path: Option<PathBuf>,
}

impl DispatchApplicationPartial {
    fn fail(self, source: RuntimeTickError) -> DispatchApplicationError {
        DispatchApplicationError {
            source: Box::new(source),
            terminal_marker_path: self.terminal_marker_path,
            router_decision: self.router_decision.map(Box::new),
            stage_result_path: self.stage_result_path,
        }
    }
}

#[derive(Debug)]
struct PostStageRecoveryOutput {
    router_decision: RouterDecision,
    runtime_error_context_path: PathBuf,
    event_log_path: PathBuf,
}

fn route_stage_result_from_graph(
    plan: &CompiledRunPlan,
    snapshot: &RuntimeSnapshot,
    stage_result: &StageResultEnvelope,
    counters: &crate::contracts::RecoveryCounters,
) -> RuntimeTickResult<RouterDecision> {
    validate_stage_result_matches_snapshot(snapshot, stage_result)?;
    let graph = graph_for_plane(plan, stage_result.plane)?;
    match stage_result.plane {
        Plane::Execution => route_execution_stage_result(graph, snapshot, stage_result, counters),
        Plane::Planning => route_planning_stage_result(graph, snapshot, stage_result, counters),
        Plane::Learning => route_learning_stage_result(graph, stage_result),
    }
}

fn validate_stage_result_matches_snapshot(
    snapshot: &RuntimeSnapshot,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<()> {
    let active_run = active_run_for_plane(snapshot, stage_result.plane)
        .ok_or_else(|| invalid_state("stage result has no active run for its plane"))?;
    if active_run.stage != stage_result.stage {
        return Err(invalid_state(
            "stage result stage does not match active run",
        ));
    }
    if active_run.node_id != stage_result.node_id {
        return Err(invalid_state(
            "stage result node_id does not match active run",
        ));
    }
    if active_run.stage_kind_id != stage_result.stage_kind_id {
        return Err(invalid_state(
            "stage result stage_kind_id does not match active run",
        ));
    }
    match active_run.request_kind {
        ActiveRunRequestKind::ClosureTarget => {
            if active_run.closure_target_root_spec_id.as_deref()
                != Some(stage_result.work_item_id.as_str())
            {
                return Err(invalid_state(
                    "closure target stage result does not match active root spec",
                ));
            }
        }
        ActiveRunRequestKind::ActiveWorkItem | ActiveRunRequestKind::LearningRequest => {
            if active_run.work_item_kind != Some(stage_result.work_item_kind)
                || active_run.work_item_id.as_deref() != Some(stage_result.work_item_id.as_str())
            {
                return Err(invalid_state(
                    "stage result work item identity does not match active run",
                ));
            }
        }
    }
    Ok(())
}

fn route_execution_stage_result(
    graph: &FrozenGraphPlanePlan,
    snapshot: &RuntimeSnapshot,
    stage_result: &StageResultEnvelope,
    counters: &crate::contracts::RecoveryCounters,
) -> RuntimeTickResult<RouterDecision> {
    let outcome = stage_result.terminal_result.as_str();
    let source_node_id = stage_result.node_id.as_str();
    let source_stage = stage_result.stage_kind_id.as_str();

    if outcome == "FIX_NEEDED" {
        if let Some(policy) = threshold_policy_for_source(
            graph,
            source_node_id,
            outcome,
            GraphLoopCounterName::FixCycleCount,
        ) {
            if snapshot.fix_cycle_count >= policy.threshold {
                let failure_class =
                    resolve_failure_class(snapshot, stage_result, "fix_cycle_exhausted")?;
                return decision_from_threshold_resolution(
                    graph,
                    snapshot,
                    policy,
                    failure_class,
                    "fix_cycle_exhausted",
                );
            }
        }
    }

    if outcome == "BLOCKED" && source_stage != "consultant" {
        let failure_class =
            resolve_failure_class(snapshot, stage_result, &format!("{source_stage}_blocked"))?;
        if let Some(policy) = threshold_policy_for_source(
            graph,
            source_node_id,
            outcome,
            GraphLoopCounterName::TroubleshootAttemptCount,
        ) {
            if counter_attempts(snapshot, counters, &failure_class, Plane::Execution)?
                >= policy.threshold
            {
                return decision_from_threshold_resolution(
                    graph,
                    snapshot,
                    policy,
                    failure_class,
                    &format!("{source_stage}_blocked"),
                );
            }
        }
    }

    if let Some(policy) = resume_policy_for_source(graph, source_node_id, outcome) {
        return decision_from_resume_policy(graph, source_stage, stage_result, policy);
    }

    let transition = transition_for_source(graph, source_node_id, outcome)?;
    if let Some(target_node_id) = &transition.target_node_id {
        if outcome == "FIX_NEEDED" {
            return run_stage_decision(graph, target_node_id, "fix_needed", None, None);
        }
        if outcome == "BLOCKED" {
            let failure_class =
                resolve_failure_class(snapshot, stage_result, &format!("{source_stage}_blocked"))?;
            let counter_key = counter_key_from_snapshot(snapshot, &failure_class)?;
            return run_stage_decision(
                graph,
                target_node_id,
                &format!("{source_stage}_blocked"),
                Some(failure_class),
                counter_key,
            );
        }
        return run_stage_decision(
            graph,
            target_node_id,
            &format!("{source_stage}:{outcome}"),
            None,
            None,
        );
    }

    match (
        source_stage,
        outcome,
        transition.terminal_state_id.as_deref(),
    ) {
        ("updater", "UPDATE_COMPLETE", _) => Ok(idle_decision("updater_complete")),
        ("consultant", "NEEDS_PLANNING", _) => Ok(RouterDecision {
            action: RouterAction::Handoff,
            next_plane: Some(Plane::Planning),
            next_stage: Some(StageName::Auditor),
            reason: "consultant_needs_planning".to_owned(),
            next_node_id: None,
            next_stage_kind_id: None,
            failure_class: None,
            counter_key: None,
            create_incident: true,
        }),
        ("consultant", "BLOCKED", _) => Ok(blocked_decision(
            "consultant_blocked",
            resolve_failure_class(snapshot, stage_result, "consultant_blocked").ok(),
            None,
        )),
        _ => Err(invalid_state(format!(
            "unsupported execution terminal transition for {source_stage}:{outcome}"
        ))),
    }
}

fn route_planning_stage_result(
    graph: &FrozenGraphPlanePlan,
    snapshot: &RuntimeSnapshot,
    stage_result: &StageResultEnvelope,
    counters: &crate::contracts::RecoveryCounters,
) -> RuntimeTickResult<RouterDecision> {
    let outcome = stage_result.terminal_result.as_str();
    let source_node_id = stage_result.node_id.as_str();
    let source_stage = stage_result.stage_kind_id.as_str();

    if outcome == "BLOCKED" {
        let failure_class =
            resolve_failure_class(snapshot, stage_result, &format!("{source_stage}_blocked"))?;
        if let Some(policy) = threshold_policy_for_source(
            graph,
            source_node_id,
            outcome,
            GraphLoopCounterName::MechanicAttemptCount,
        ) {
            if counter_attempts(snapshot, counters, &failure_class, Plane::Planning)?
                >= policy.threshold
            {
                return decision_from_threshold_resolution(
                    graph,
                    snapshot,
                    policy,
                    failure_class,
                    &format!("{source_stage}_blocked"),
                );
            }
        }
    }

    if let Some(policy) = resume_policy_for_source(graph, source_node_id, outcome) {
        return decision_from_resume_policy(graph, source_stage, stage_result, policy);
    }

    let transition = transition_for_source(graph, source_node_id, outcome)?;
    if let Some(target_node_id) = &transition.target_node_id {
        if outcome == "BLOCKED" {
            let failure_class =
                resolve_failure_class(snapshot, stage_result, &format!("{source_stage}_blocked"))?;
            let counter_key = counter_key_from_snapshot(snapshot, &failure_class)?;
            return run_stage_decision(
                graph,
                target_node_id,
                &format!("{source_stage}_blocked"),
                Some(failure_class),
                counter_key,
            );
        }
        return run_stage_decision(
            graph,
            target_node_id,
            &format!("{source_stage}:{outcome}"),
            None,
            None,
        );
    }

    match (
        source_stage,
        outcome,
        transition.terminal_state_id.as_deref(),
    ) {
        ("recon", "RECON_TO_EXECUTION", _) => Ok(idle_decision("recon_to_execution")),
        ("recon", "RECON_TO_PLANNING", _) => Ok(idle_decision("recon_to_planning")),
        ("recon", "RECON_NOOP", _) => Ok(idle_decision("recon_noop")),
        ("recon", "RECON_BLOCKED" | "BLOCKED", _) => Ok(blocked_decision(
            "recon_blocked",
            resolve_failure_class(snapshot, stage_result, "recon_blocked").ok(),
            None,
        )),
        ("manager", "MANAGER_COMPLETE", _) => Ok(idle_decision("manager_complete")),
        ("arbiter", "ARBITER_COMPLETE", _) => Ok(idle_decision("arbiter_complete")),
        ("arbiter", "REMEDIATION_NEEDED", _) => Ok(RouterDecision {
            action: RouterAction::Handoff,
            next_plane: Some(Plane::Planning),
            next_stage: Some(StageName::Auditor),
            reason: "arbiter_remediation_needed".to_owned(),
            next_node_id: None,
            next_stage_kind_id: None,
            failure_class: Some("arbiter_parity_gap".to_owned()),
            counter_key: None,
            create_incident: true,
        }),
        ("arbiter", "BLOCKED", _) => Ok(blocked_decision(
            "arbiter_blocked",
            resolve_failure_class(snapshot, stage_result, "arbiter_blocked").ok(),
            None,
        )),
        _ => Err(invalid_state(format!(
            "unsupported planning terminal transition for {source_stage}:{outcome}"
        ))),
    }
}

fn route_learning_stage_result(
    graph: &FrozenGraphPlanePlan,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<RouterDecision> {
    let outcome = stage_result.terminal_result.as_str();
    let source_stage = stage_result.stage_kind_id.as_str();
    let transition = transition_for_source(graph, &stage_result.node_id, outcome)?;
    if let Some(target_node_id) = &transition.target_node_id {
        return run_stage_decision(
            graph,
            target_node_id,
            &format!("{source_stage}:{outcome}"),
            None,
            None,
        );
    }
    if outcome == "BLOCKED" {
        return Ok(blocked_decision(
            &format!("{source_stage}_blocked"),
            None,
            None,
        ));
    }
    Ok(idle_decision(&format!("{source_stage}:{outcome}")))
}

fn decision_from_resume_policy(
    graph: &FrozenGraphPlanePlan,
    source_stage: &str,
    stage_result: &StageResultEnvelope,
    policy: &crate::compiler::CompiledGraphResumePolicyPlan,
) -> RuntimeTickResult<RouterDecision> {
    let mut target_node_id = policy.default_target_node_id.clone();
    for metadata_key in &policy.metadata_stage_keys {
        let Some(candidate) = stage_result
            .metadata
            .get(metadata_key)
            .and_then(Value::as_str)
        else {
            continue;
        };
        let normalized = candidate.trim().to_ascii_lowercase();
        if normalized.is_empty()
            || policy
                .disallowed_target_node_ids
                .iter()
                .any(|node_id| node_id == &normalized)
            || graph.nodes.iter().all(|node| node.node_id != normalized)
        {
            continue;
        }
        target_node_id = normalized;
        break;
    }
    let reason = match source_stage {
        "troubleshooter" => "troubleshoot_complete",
        "consultant" => "consultant_local_recovery",
        "mechanic" => "mechanic_complete",
        _ => {
            return Err(invalid_state(format!(
                "unsupported resume-policy source stage: {source_stage}"
            )));
        }
    };
    run_stage_decision(graph, &target_node_id, reason, None, None)
}

fn decision_from_threshold_resolution(
    graph: &FrozenGraphPlanePlan,
    snapshot: &RuntimeSnapshot,
    policy: &CompiledGraphThresholdPolicyPlan,
    failure_class: String,
    reason: &str,
) -> RuntimeTickResult<RouterDecision> {
    let counter_key = counter_key_from_snapshot(snapshot, &failure_class)?;
    if let Some(target_node_id) = &policy.exhausted_target_node_id {
        return run_stage_decision(
            graph,
            target_node_id,
            reason,
            Some(failure_class),
            counter_key,
        );
    }
    let terminal_state_id = policy
        .exhausted_terminal_state_id
        .as_deref()
        .ok_or_else(|| invalid_state("threshold policy is missing exhausted target"))?;
    let terminal_state = terminal_state_by_id(graph, terminal_state_id)?;
    if terminal_state.terminal_class != GraphLoopTerminalClass::Blocked {
        return Err(invalid_state(format!(
            "unsupported threshold terminal class: {}",
            terminal_state.terminal_class.as_str()
        )));
    }
    Ok(blocked_decision(
        &format!("{reason}:mechanic_attempts_exhausted"),
        Some(failure_class),
        counter_key,
    ))
}

fn transition_for_source<'a>(
    graph: &'a FrozenGraphPlanePlan,
    source_node_id: &str,
    outcome: &str,
) -> RuntimeTickResult<&'a crate::compiler::CompiledGraphTransitionPlan> {
    graph
        .compiled_transitions
        .iter()
        .filter(|transition| {
            transition.source_node_id == source_node_id && transition.outcome == outcome
        })
        .min_by(|left, right| left.priority.cmp(&right.priority))
        .ok_or_else(|| {
            invalid_state(format!(
                "compiled graph is missing transition for {source_node_id}:{outcome}"
            ))
        })
}

fn resume_policy_for_source<'a>(
    graph: &'a FrozenGraphPlanePlan,
    source_node_id: &str,
    outcome: &str,
) -> Option<&'a crate::compiler::CompiledGraphResumePolicyPlan> {
    graph
        .compiled_resume_policies
        .iter()
        .find(|policy| policy.source_node_id == source_node_id && policy.on_outcome == outcome)
}

fn threshold_policy_for_source<'a>(
    graph: &'a FrozenGraphPlanePlan,
    source_node_id: &str,
    outcome: &str,
    counter_name: GraphLoopCounterName,
) -> Option<&'a CompiledGraphThresholdPolicyPlan> {
    graph.compiled_threshold_policies.iter().find(|policy| {
        policy
            .source_node_ids
            .iter()
            .any(|node| node == source_node_id)
            && policy.on_outcome == outcome
            && policy.counter_name == counter_name
    })
}

fn terminal_state_by_id<'a>(
    graph: &'a FrozenGraphPlanePlan,
    terminal_state_id: &str,
) -> RuntimeTickResult<&'a crate::compiler::GraphLoopTerminalStateDefinition> {
    graph
        .terminal_states
        .iter()
        .find(|state| state.terminal_state_id == terminal_state_id)
        .ok_or_else(|| {
            invalid_state(format!(
                "compiled graph is missing terminal state `{terminal_state_id}`"
            ))
        })
}

fn run_stage_decision(
    graph: &FrozenGraphPlanePlan,
    target_node_id: &str,
    reason: &str,
    failure_class: Option<String>,
    counter_key: Option<String>,
) -> RuntimeTickResult<RouterDecision> {
    let node = node_plan_by_id(graph, target_node_id)?;
    Ok(RouterDecision {
        action: RouterAction::RunStage,
        next_plane: Some(graph.plane),
        next_stage: Some(stage_name_for_plane(graph.plane, &node.stage_kind_id)?),
        reason: reason.to_owned(),
        next_node_id: Some(node.node_id.clone()),
        next_stage_kind_id: Some(node.stage_kind_id.clone()),
        failure_class,
        counter_key,
        create_incident: false,
    })
}

fn idle_decision(reason: &str) -> RouterDecision {
    RouterDecision {
        action: RouterAction::Idle,
        next_plane: None,
        next_stage: None,
        reason: reason.to_owned(),
        next_node_id: None,
        next_stage_kind_id: None,
        failure_class: None,
        counter_key: None,
        create_incident: false,
    }
}

fn blocked_decision(
    reason: &str,
    failure_class: Option<String>,
    counter_key: Option<String>,
) -> RouterDecision {
    RouterDecision {
        action: RouterAction::Blocked,
        next_plane: None,
        next_stage: None,
        reason: reason.to_owned(),
        next_node_id: None,
        next_stage_kind_id: None,
        failure_class,
        counter_key,
        create_incident: false,
    }
}

fn node_plan_by_id<'a>(
    graph: &'a FrozenGraphPlanePlan,
    node_id: &str,
) -> RuntimeTickResult<&'a MaterializedGraphNodePlan> {
    graph
        .nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .ok_or_else(|| invalid_state(format!("compiled graph is missing `{node_id}` node")))
}

fn resolve_failure_class(
    snapshot: &RuntimeSnapshot,
    stage_result: &StageResultEnvelope,
    default: &str,
) -> RuntimeTickResult<String> {
    if let Some(failure_class) = failure_class_from_stage_result(stage_result) {
        return normalize_failure_class(&failure_class);
    }
    if let Some(failure_class) = &snapshot.current_failure_class {
        if !failure_class.trim().is_empty() {
            return normalize_failure_class(failure_class);
        }
    }
    normalize_failure_class(default)
}

fn failure_class_from_stage_result(stage_result: &StageResultEnvelope) -> Option<String> {
    stage_result
        .metadata
        .get("failure_class")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

fn normalize_failure_class(value: &str) -> RuntimeTickResult<String> {
    let mut normalized = String::with_capacity(value.len());
    let mut previous_was_separator = true;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            normalized.push('_');
            previous_was_separator = true;
        }
    }
    while normalized.ends_with('_') {
        normalized.pop();
    }
    if normalized.is_empty() {
        Err(invalid_state("failure_class cannot be empty"))
    } else {
        Ok(normalized)
    }
}

fn counter_attempts(
    snapshot: &RuntimeSnapshot,
    counters: &crate::contracts::RecoveryCounters,
    failure_class: &str,
    plane: Plane,
) -> RuntimeTickResult<u64> {
    let (Some(work_item_kind), Some(work_item_id)) = (
        snapshot.active_work_item_kind,
        snapshot.active_work_item_id.as_deref(),
    ) else {
        return Ok(0);
    };
    let normalized_failure_class = normalize_failure_class(failure_class)?;
    let Some(entry) = counters.entries.iter().find(|entry| {
        entry.work_item_kind == work_item_kind
            && entry.work_item_id == work_item_id
            && normalize_failure_class(&entry.failure_class)
                .is_ok_and(|entry_failure_class| entry_failure_class == normalized_failure_class)
    }) else {
        return Ok(0);
    };
    Ok(match plane {
        Plane::Execution => entry.troubleshoot_attempt_count,
        Plane::Planning | Plane::Learning => entry.mechanic_attempt_count,
    })
}

fn counter_key_from_snapshot(
    snapshot: &RuntimeSnapshot,
    failure_class: &str,
) -> RuntimeTickResult<Option<String>> {
    let (Some(work_item_kind), Some(work_item_id)) = (
        snapshot.active_work_item_kind,
        snapshot.active_work_item_id.as_deref(),
    ) else {
        return Ok(None);
    };
    Ok(Some(format!(
        "{}:{}:{}",
        work_item_kind.as_str(),
        work_item_id.trim(),
        normalize_failure_class(failure_class)?
    )))
}

fn stopped_outcome(
    session: &mut RuntimeStartupSession,
    now: &Timestamp,
) -> RuntimeTickResult<RuntimeTickOutcome> {
    let event_log_path = record_runtime_stopped_cycle(session, now)?;
    session.close()?;
    Ok(RuntimeTickOutcome {
        kind: RuntimeTickOutcomeKind::Stopped,
        reason: "stop_requested".to_owned(),
        stage_request: None,
        snapshot: session.snapshot.clone(),
        event_log_path: Some(event_log_path),
    })
}

pub(crate) fn record_runtime_stopped_cycle(
    session: &mut RuntimeStartupSession,
    now: &Timestamp,
) -> RuntimeTickResult<PathBuf> {
    reset_runtime_to_idle(
        &session.paths,
        &mut session.snapshot,
        false,
        true,
        true,
        now,
    )?;
    write_runtime_event(&session.paths, "runtime_tick_stopped", Map::new(), now)
}

fn idle_outcome(
    session: &mut RuntimeStartupSession,
    now: &Timestamp,
) -> RuntimeTickResult<RuntimeTickOutcome> {
    session.snapshot.updated_at = now.clone();
    save_snapshot(&session.paths, &session.snapshot)?;
    let event_log_path = write_runtime_event(&session.paths, "runtime_tick_idle", Map::new(), now)?;
    Ok(RuntimeTickOutcome {
        kind: RuntimeTickOutcomeKind::NoWork,
        reason: "no_work".to_owned(),
        stage_request: None,
        snapshot: session.snapshot.clone(),
        event_log_path: Some(event_log_path),
    })
}

fn drain_mailbox(session: &mut RuntimeStartupSession, now: &Timestamp) -> RuntimeTickResult<()> {
    let mut commands = json_files(&session.paths.mailbox_incoming_dir)?;
    commands.sort();
    for path in commands {
        let envelope = match read_mailbox_envelope(&path) {
            Ok(envelope) => envelope,
            Err(error) => {
                archive_unreadable_mailbox_payload(&session.paths, &path, &error.to_string(), now)?;
                write_runtime_event(
                    &session.paths,
                    "mailbox_command_failed",
                    json_object([
                        (
                            "source_path",
                            Value::String(workspace_relative(&session.paths, &path)),
                        ),
                        ("error", Value::String(error.to_string())),
                    ]),
                    now,
                )?;
                continue;
            }
        };

        match apply_mailbox_command(session, &envelope, now) {
            Ok(result) => {
                archive_mailbox_command(&session.paths, &path, &envelope, true, result, None, now)?;
            }
            Err(error) => {
                archive_mailbox_command(
                    &session.paths,
                    &path,
                    &envelope,
                    false,
                    Map::new(),
                    Some(error.to_string()),
                    now,
                )?;
                write_runtime_event(
                    &session.paths,
                    "mailbox_command_failed",
                    json_object([
                        ("command_id", Value::String(envelope.command_id.clone())),
                        (
                            "command",
                            Value::String(envelope.command.as_str().to_owned()),
                        ),
                        (
                            "source_path",
                            Value::String(workspace_relative(&session.paths, &path)),
                        ),
                        ("error", Value::String(error.to_string())),
                    ]),
                    now,
                )?;
            }
        }
    }
    save_snapshot(&session.paths, &session.snapshot)?;
    Ok(())
}

fn consume_watcher_events(
    session: &mut RuntimeStartupSession,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    if !session.watcher_session.poll_fallback_ready {
        return Ok(());
    }

    let targets = session.watcher_session.targets.clone();
    let debounce_ms = session.watcher_session.debounce_ms;
    let observed_at_millis = timestamp_millis(now)?;
    let Some(poller) = session.watcher_session.poller.as_mut() else {
        return Ok(());
    };
    let events = poller.poll_once(&targets, now.clone(), observed_at_millis, debounce_ms);
    if events.is_empty() {
        return Ok(());
    }

    let mut handled_count = 0_u64;
    let mut failure_count = 0_u64;
    let mut event_values = Vec::new();
    for event in &events {
        event_values.push(watcher_event_value(&session.paths, event));
        match handle_watch_event(session, event, now) {
            Ok(true) => handled_count += 1,
            Ok(false) => {}
            Err(error) => {
                failure_count += 1;
                write_runtime_event(
                    &session.paths,
                    "watcher_event_failed",
                    json_object([
                        ("target", Value::String(event.target.clone())),
                        (
                            "path",
                            Value::String(workspace_relative(&session.paths, &event.path)),
                        ),
                        ("error", Value::String(error.to_string())),
                    ]),
                    now,
                )?;
            }
        }
    }

    session.snapshot.updated_at = now.clone();
    write_runtime_event(
        &session.paths,
        "watcher_events_consumed",
        json_object([
            ("count", Value::Number((events.len() as u64).into())),
            ("handled_count", Value::Number(handled_count.into())),
            ("failure_count", Value::Number(failure_count.into())),
            ("events", Value::Array(event_values)),
        ]),
        now,
    )?;
    Ok(())
}

fn handle_watch_event(
    session: &mut RuntimeStartupSession,
    event: &RuntimeWatchEvent,
    now: &Timestamp,
) -> RuntimeTickResult<bool> {
    match event.target.as_str() {
        "ideas_inbox" => normalize_idea_watch_event(session, &event.path, now),
        "config" | "tasks_queue" | "specs_queue" => Ok(true),
        target => {
            write_runtime_event(
                &session.paths,
                "watcher_event_ignored",
                json_object([
                    ("target", Value::String(target.to_owned())),
                    (
                        "path",
                        Value::String(workspace_relative(&session.paths, &event.path)),
                    ),
                ]),
                now,
            )?;
            Ok(false)
        }
    }
}

fn normalize_idea_watch_event(
    session: &mut RuntimeStartupSession,
    idea_path: &Path,
    now: &Timestamp,
) -> RuntimeTickResult<bool> {
    if !idea_path.is_file() {
        return Ok(false);
    }

    let content = fs::read_to_string(idea_path).map_err(|error| io_error(idea_path, error))?;
    let fallback = idea_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("idea");
    let (title, summary) = derive_idea_title_summary(&content, fallback);
    let spec_id = safe_spec_id_from_idea_path(idea_path);
    let idea_reference = workspace_relative(&session.paths, idea_path);

    if idea_already_represented(&session.paths, &spec_id, &idea_reference)? {
        write_idea_normalization_skipped_event(
            &session.paths,
            idea_path,
            &spec_id,
            "already_represented",
            now,
        )?;
        return Ok(true);
    }

    let document = SpecDocument {
        spec_id: spec_id.clone(),
        title,
        summary: summary.clone(),
        source_type: SpecSourceType::Idea,
        source_id: Some(spec_id.clone()),
        parent_spec_id: None,
        root_idea_id: Some(spec_id.clone()),
        root_spec_id: Some(spec_id.clone()),
        root_intake_kind: None,
        root_intake_id: None,
        goals: vec![summary],
        non_goals: Vec::new(),
        scope: Vec::new(),
        constraints: vec!["generated from ideas/inbox watcher event".to_owned()],
        assumptions: Vec::new(),
        risks: Vec::new(),
        target_paths: Vec::new(),
        entrypoints: Vec::new(),
        required_skills: Vec::new(),
        decomposition_hints: Vec::new(),
        acceptance: vec!["planner processes this idea-derived spec".to_owned()],
        references: vec![idea_reference],
        created_at: now.clone(),
        created_by: "watcher".to_owned(),
        updated_at: None,
    };

    let destination = match QueueStore::from_paths(session.paths.clone()).enqueue_spec(&document) {
        Ok(destination) => destination,
        Err(QueueStoreError::InvalidState { message }) if message.contains("already exists") => {
            write_idea_normalization_skipped_event(
                &session.paths,
                idea_path,
                &spec_id,
                "already_represented",
                now,
            )?;
            return Ok(true);
        }
        Err(error) => return Err(error.into()),
    };

    write_runtime_event(
        &session.paths,
        "idea_normalized_to_spec",
        json_object([
            (
                "idea_path",
                Value::String(workspace_relative(&session.paths, idea_path)),
            ),
            ("spec_id", Value::String(spec_id)),
            (
                "spec_path",
                Value::String(workspace_relative(&session.paths, &destination)),
            ),
        ]),
        now,
    )?;
    Ok(true)
}

fn write_idea_normalization_skipped_event(
    paths: &WorkspacePaths,
    idea_path: &Path,
    spec_id: &str,
    reason: &str,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    write_runtime_event(
        paths,
        "idea_normalization_skipped",
        json_object([
            (
                "idea_path",
                Value::String(workspace_relative(paths, idea_path)),
            ),
            ("spec_id", Value::String(spec_id.to_owned())),
            ("reason", Value::String(reason.to_owned())),
        ]),
        now,
    )?;
    Ok(())
}

fn idea_already_represented(
    paths: &WorkspacePaths,
    spec_id: &str,
    idea_reference: &str,
) -> RuntimeTickResult<bool> {
    for directory in [
        &paths.specs_queue_dir,
        &paths.specs_active_dir,
        &paths.specs_done_dir,
        &paths.specs_blocked_dir,
    ] {
        for path in markdown_files(directory)? {
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(document) = parse_spec_document_with_source(&raw, &path.display().to_string())
            else {
                continue;
            };
            if document.spec_id == spec_id
                || document.root_idea_id.as_deref() == Some(spec_id)
                || document
                    .references
                    .iter()
                    .any(|reference| reference == idea_reference)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn derive_idea_title_summary(content: &str, fallback: &str) -> (String, String) {
    let lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let mut title = fallback.to_owned();
    for line in &lines {
        if line.starts_with('#') {
            let candidate = line.trim_start_matches('#').trim();
            if !candidate.is_empty() {
                title = candidate.to_owned();
                break;
            }
        }
    }
    if title == fallback && !lines.is_empty() {
        title = lines[0].to_owned();
    }

    let mut summary = String::new();
    for line in &lines {
        let candidate = line.trim_start_matches('#').trim();
        if !candidate.is_empty() && candidate != title {
            summary = candidate.to_owned();
            break;
        }
    }
    if summary.is_empty() {
        summary = format!("Idea captured from {fallback}");
    }
    (title, summary)
}

fn safe_spec_id_from_idea_path(path: &Path) -> String {
    let raw = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("idea");
    let mut normalized = String::new();
    let mut previous_was_replacement = false;
    for character in raw.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            normalized.push(character);
            previous_was_replacement = false;
        } else if !previous_was_replacement {
            normalized.push('-');
            previous_was_replacement = true;
        }
    }
    let normalized = normalized.trim_matches(['-', '.']).to_owned();
    let normalized = if normalized.is_empty() {
        "idea".to_owned()
    } else {
        normalized
    };
    if normalized.starts_with("idea-") {
        normalized
    } else {
        format!("idea-{normalized}")
    }
}

fn watcher_event_value(paths: &WorkspacePaths, event: &RuntimeWatchEvent) -> Value {
    json!({
        "target": event.target,
        "path": workspace_relative(paths, &event.path),
        "event_kind": event.event_kind,
        "observed_at": event.observed_at.as_str(),
    })
}

fn timestamp_millis(timestamp: &Timestamp) -> RuntimeTickResult<i128> {
    let parsed = OffsetDateTime::parse(timestamp.as_str(), &Rfc3339).map_err(|error| {
        RuntimeTickError::Time {
            field_name: "observed_at",
            message: error.to_string(),
        }
    })?;
    Ok(i128::from(parsed.unix_timestamp()) * 1_000 + i128::from(parsed.nanosecond() / 1_000_000))
}

fn read_mailbox_envelope(path: &Path) -> RuntimeTickResult<MailboxCommandEnvelope> {
    let raw = fs::read_to_string(path).map_err(|error| io_error(path, error))?;
    MailboxCommandEnvelope::from_json_str(&raw).map_err(|source| RuntimeTickError::InvalidState {
        message: format!("mailbox command {} is invalid: {source}", path.display()),
    })
}

fn apply_mailbox_command(
    session: &mut RuntimeStartupSession,
    envelope: &MailboxCommandEnvelope,
    now: &Timestamp,
) -> RuntimeTickResult<Map<String, Value>> {
    match envelope.command {
        MailboxCommand::Pause => {
            if !session
                .snapshot
                .pause_sources
                .contains(&PauseSource::Operator)
            {
                session.snapshot.pause_sources.push(PauseSource::Operator);
            }
            session.snapshot.paused = true;
            session.snapshot.updated_at = now.clone();
            write_runtime_event(
                &session.paths,
                "mailbox_pause_applied",
                mailbox_event_data(envelope, Map::new()),
                now,
            )?;
            Ok(mailbox_result(true, "runtime paused"))
        }
        MailboxCommand::Resume => {
            session
                .snapshot
                .pause_sources
                .retain(|source| *source != PauseSource::Operator);
            session.snapshot.paused = !session.snapshot.pause_sources.is_empty();
            session.snapshot.updated_at = now.clone();
            write_runtime_event(
                &session.paths,
                "mailbox_resume_applied",
                mailbox_event_data(envelope, Map::new()),
                now,
            )?;
            Ok(mailbox_result(true, "runtime resumed"))
        }
        MailboxCommand::Stop => {
            session.snapshot.stop_requested = true;
            session.snapshot.updated_at = now.clone();
            write_runtime_event(
                &session.paths,
                "mailbox_stop_applied",
                mailbox_event_data(envelope, Map::new()),
                now,
            )?;
            Ok(mailbox_result(true, "runtime stop requested"))
        }
        MailboxCommand::AddTask => {
            let payload =
                MailboxAddTaskPayload::from_json_value(Value::Object(envelope.payload.clone()))
                    .map_err(|source| RuntimeTickError::InvalidState {
                        message: format!("mailbox add_task payload is invalid: {source}"),
                    })?;
            let destination =
                QueueStore::from_paths(session.paths.clone()).enqueue_task(&payload.document)?;
            refresh_queue_depths(&session.paths, &mut session.snapshot)?;
            session.snapshot.updated_at = now.clone();
            let relative_path = workspace_relative(&session.paths, &destination);
            write_runtime_event(
                &session.paths,
                "mailbox_add_task_applied",
                mailbox_event_data(
                    envelope,
                    json_object([
                        ("task_id", Value::String(payload.document.task_id.clone())),
                        ("path", Value::String(relative_path.clone())),
                    ]),
                ),
                now,
            )?;
            Ok(mailbox_result_with_path(true, "task queued", relative_path))
        }
        MailboxCommand::AddProbe => {
            let payload =
                MailboxAddProbePayload::from_json_value(Value::Object(envelope.payload.clone()))
                    .map_err(|source| RuntimeTickError::InvalidState {
                        message: format!("mailbox add_probe payload is invalid: {source}"),
                    })?;
            let destination =
                QueueStore::from_paths(session.paths.clone()).enqueue_probe(&payload.document)?;
            refresh_queue_depths(&session.paths, &mut session.snapshot)?;
            session.snapshot.updated_at = now.clone();
            let relative_path = workspace_relative(&session.paths, &destination);
            write_runtime_event(
                &session.paths,
                "mailbox_add_probe_applied",
                mailbox_event_data(
                    envelope,
                    json_object([
                        ("probe_id", Value::String(payload.document.probe_id.clone())),
                        ("path", Value::String(relative_path.clone())),
                    ]),
                ),
                now,
            )?;
            Ok(mailbox_result_with_path(
                true,
                "probe queued",
                relative_path,
            ))
        }
        MailboxCommand::AddSpec => {
            let payload =
                MailboxAddSpecPayload::from_json_value(Value::Object(envelope.payload.clone()))
                    .map_err(|source| RuntimeTickError::InvalidState {
                        message: format!("mailbox add_spec payload is invalid: {source}"),
                    })?;
            let destination =
                QueueStore::from_paths(session.paths.clone()).enqueue_spec(&payload.document)?;
            refresh_queue_depths(&session.paths, &mut session.snapshot)?;
            session.snapshot.updated_at = now.clone();
            let relative_path = workspace_relative(&session.paths, &destination);
            write_runtime_event(
                &session.paths,
                "mailbox_add_spec_applied",
                mailbox_event_data(
                    envelope,
                    json_object([
                        ("spec_id", Value::String(payload.document.spec_id.clone())),
                        ("path", Value::String(relative_path.clone())),
                    ]),
                ),
                now,
            )?;
            Ok(mailbox_result_with_path(true, "spec queued", relative_path))
        }
        MailboxCommand::AddIdea => {
            let payload =
                MailboxAddIdeaPayload::from_json_value(Value::Object(envelope.payload.clone()))
                    .map_err(|source| RuntimeTickError::InvalidState {
                        message: format!("mailbox add_idea payload is invalid: {source}"),
                    })?;
            let destination = session
                .paths
                .root
                .join("ideas")
                .join("inbox")
                .join(&payload.source_name);
            if destination.exists() {
                return Err(invalid_state(format!(
                    "idea document already exists: {}",
                    destination.display()
                )));
            }
            atomic_write_text(&destination, &payload.markdown)?;
            refresh_queue_depths(&session.paths, &mut session.snapshot)?;
            session.snapshot.updated_at = now.clone();
            let relative_path = workspace_relative(&session.paths, &destination);
            write_runtime_event(
                &session.paths,
                "mailbox_add_idea_applied",
                mailbox_event_data(
                    envelope,
                    json_object([
                        ("source_name", Value::String(payload.source_name.clone())),
                        ("path", Value::String(relative_path.clone())),
                    ]),
                ),
                now,
            )?;
            Ok(mailbox_result_with_path(true, "idea staged", relative_path))
        }
        MailboxCommand::RetryActive => retry_active_from_mailbox(session, envelope, now),
        MailboxCommand::ClearStaleState => clear_stale_from_mailbox(session, envelope, now),
        MailboxCommand::ReloadConfig => reload_config_from_mailbox(session, envelope, now),
    }
}

fn retry_active_from_mailbox(
    session: &mut RuntimeStartupSession,
    envelope: &MailboxCommandEnvelope,
    now: &Timestamp,
) -> RuntimeTickResult<Map<String, Value>> {
    let scope = mailbox_retry_scope(envelope)?;
    let reason = mailbox_reason(envelope, "operator requested retry");
    let active_run = active_run_for_retry(&session.snapshot, scope);
    if active_run.is_none() {
        let detail = retry_active_missing_detail(&session.snapshot, scope);
        emit_retry_active_skipped(session, envelope, scope, "missing_active_run", now)?;
        return Ok(mailbox_result(false, detail));
    }
    if scope.is_none() && session.snapshot.active_runs_by_plane.len() > 1 {
        emit_retry_active_skipped(session, envelope, scope, "multiple_active_planes", now)?;
        return Ok(mailbox_result(
            false,
            "multiple active planes; retry-active requires a plane scope",
        ));
    }

    let active_run = active_run.expect("active run checked above");
    let Some(work_item_kind) = active_run.work_item_kind else {
        emit_retry_active_skipped(session, envelope, scope, "non_retryable_active_run", now)?;
        return Ok(mailbox_result(
            false,
            format!(
                "active {} run is not a retryable work item",
                active_run.plane.as_str()
            ),
        ));
    };
    let Some(work_item_id) = active_run.work_item_id.clone() else {
        emit_retry_active_skipped(session, envelope, scope, "non_retryable_active_run", now)?;
        return Ok(mailbox_result(
            false,
            format!(
                "active {} run is not a retryable work item",
                active_run.plane.as_str()
            ),
        ));
    };

    let queue = QueueStore::from_paths(session.paths.clone());
    if let Err(error) = requeue_active_item(&queue, work_item_kind, &work_item_id, &reason) {
        if let QueueStoreError::InvalidState { message } = error {
            emit_retry_active_skipped(session, envelope, scope, "queue_state", now)?;
            return Ok(mailbox_result(false, message));
        }
        return Err(error.into());
    }

    clear_retry_active_plane(session, active_run.plane, now)?;
    reset_forward_progress_counters(&session.paths, work_item_kind, &work_item_id)?;
    session.counters = load_recovery_counters(&session.paths)?;
    refresh_queue_depths(&session.paths, &mut session.snapshot)?;
    session.snapshot.updated_at = now.clone();
    save_snapshot(&session.paths, &session.snapshot)?;

    let detail = format!(
        "active {} {} requeued",
        work_item_kind.as_str(),
        work_item_id
    );
    write_runtime_event(
        &session.paths,
        "retry_active_applied",
        mailbox_event_data(
            envelope,
            json_object([
                ("plane", Value::String(active_run.plane.as_str().to_owned())),
                (
                    "work_item_kind",
                    Value::String(work_item_kind.as_str().to_owned()),
                ),
                ("work_item_id", Value::String(work_item_id)),
                ("reason", Value::String(reason)),
            ]),
        ),
        now,
    )?;
    Ok(mailbox_result(true, detail))
}

fn clear_stale_from_mailbox(
    session: &mut RuntimeStartupSession,
    envelope: &MailboxCommandEnvelope,
    now: &Timestamp,
) -> RuntimeTickResult<Map<String, Value>> {
    let reason = mailbox_reason(envelope, "operator requested stale-state clear");
    let had_counters = !load_recovery_counters(&session.paths)?.entries.is_empty();
    let queue = QueueStore::from_paths(session.paths.clone());
    let requeued_count = requeue_all_active_items(&session.paths, &queue, &reason)?;
    let snapshot_had_state = session.snapshot.active_stage.is_some()
        || session.snapshot.process_running
        || session.snapshot.paused
        || session.snapshot.stop_requested
        || !session.snapshot.active_runs_by_plane.is_empty();

    reset_runtime_to_idle(&session.paths, &mut session.snapshot, true, true, true, now)?;
    let counters = RecoveryCounters {
        schema_version: "1.0".to_owned(),
        kind: "recovery_counters".to_owned(),
        entries: Vec::new(),
    };
    save_recovery_counters(&session.paths, &counters)?;
    session.counters = counters;

    let applied = requeued_count > 0 || had_counters || snapshot_had_state;
    let detail = format!("cleared stale runtime state; requeued={requeued_count}");
    write_runtime_event(
        &session.paths,
        "clear_stale_state_applied",
        mailbox_event_data(
            envelope,
            json_object([
                ("applied", Value::Bool(applied)),
                ("requeued_count", Value::Number(requeued_count.into())),
                ("reason", Value::String(reason)),
            ]),
        ),
        now,
    )?;
    Ok(mailbox_result(applied, detail))
}

fn reload_config_from_mailbox(
    session: &mut RuntimeStartupSession,
    envelope: &MailboxCommandEnvelope,
    now: &Timestamp,
) -> RuntimeTickResult<Map<String, Value>> {
    let active_planes = active_plane_values(&session.snapshot);
    if !active_planes.is_empty() {
        let deferred_command_id = deferred_reload_command_id(envelope);
        let deferred = MailboxCommandEnvelope {
            command_id: deferred_command_id.clone(),
            issued_at: now.clone(),
            ..envelope.clone()
        };
        write_mailbox_envelope(&session.paths, &deferred)?;
        session.snapshot.last_reload_error = Some("deferred until active planes drain".to_owned());
        session.snapshot.updated_at = now.clone();
        save_snapshot(&session.paths, &session.snapshot)?;

        let active_planes_json = active_planes
            .iter()
            .map(|plane| Value::String((*plane).to_owned()))
            .collect::<Vec<_>>();
        write_runtime_event(
            &session.paths,
            "runtime_config_reload_deferred",
            mailbox_event_data(
                envelope,
                json_object([
                    ("reason", Value::String("active_planes".to_owned())),
                    ("active_planes", Value::Array(active_planes_json.clone())),
                    ("deferred_command_id", Value::String(deferred_command_id)),
                ]),
            ),
            now,
        )?;

        let mut result = mailbox_result(false, "reload deferred until active planes drain");
        result.insert("deferred".to_owned(), Value::Bool(true));
        result.insert("active_planes".to_owned(), Value::Array(active_planes_json));
        return Ok(result);
    }

    let reloaded_config = match load_runtime_startup_config(&session.config.config_path) {
        Ok(config) => config,
        Err(error) => {
            let errors = error.to_string();
            session.snapshot.last_reload_outcome = Some(ReloadOutcome::FailedRetainedPreviousPlan);
            session.snapshot.last_reload_error = Some(errors.clone());
            session.snapshot.process_running = false;
            session.snapshot.stop_requested = true;
            session.snapshot.updated_at = now.clone();
            save_snapshot(&session.paths, &session.snapshot)?;
            write_runtime_event(
                &session.paths,
                "runtime_config_reload_failed",
                mailbox_event_data(
                    envelope,
                    json_object([
                        ("error", Value::String(errors.clone())),
                        ("retained_previous_plan", Value::Bool(false)),
                        (
                            "compiled_plan_id",
                            Value::String(session.snapshot.compiled_plan_id.clone()),
                        ),
                    ]),
                ),
                now,
            )?;
            return Err(invalid_state(errors));
        }
    };
    let compile_outcome = compile_and_persist_workspace_plan_for_paths(
        &session.paths,
        CompileWorkspaceOptions {
            requested_mode_id: Some(session.snapshot.active_mode_id.clone()),
            compiled_at: Some(now.clone()),
            compile_if_needed: true,
            refuse_stale_last_known_good: true,
            config_path: Some(reloaded_config.config_path.clone()),
            persist_failure_diagnostics: true,
        },
    )
    .map_err(RuntimeStartupError::from)?;

    let errors = compile_outcome.diagnostics.errors.join(", ");
    let errors = if errors.is_empty() {
        "compile failed".to_owned()
    } else {
        errors
    };

    let Some(active_plan) = compile_outcome.active_plan else {
        session.snapshot.last_reload_outcome = Some(ReloadOutcome::FailedRetainedPreviousPlan);
        session.snapshot.last_reload_error = Some(errors.clone());
        session.snapshot.process_running = false;
        session.snapshot.stop_requested = true;
        session.snapshot.updated_at = now.clone();
        save_snapshot(&session.paths, &session.snapshot)?;
        write_runtime_event(
            &session.paths,
            "runtime_config_reload_failed",
            mailbox_event_data(
                envelope,
                json_object([
                    ("error", Value::String(errors.clone())),
                    ("retained_previous_plan", Value::Bool(false)),
                ]),
            ),
            now,
        )?;
        return Err(invalid_state(errors));
    };

    if !compile_outcome.diagnostics.ok {
        session.snapshot.last_reload_outcome = Some(ReloadOutcome::FailedRetainedPreviousPlan);
        session.snapshot.last_reload_error = Some(errors.clone());
        session.snapshot.updated_at = now.clone();
        save_snapshot(&session.paths, &session.snapshot)?;
        write_runtime_event(
            &session.paths,
            "runtime_config_reload_failed",
            mailbox_event_data(
                envelope,
                json_object([
                    ("error", Value::String(errors.clone())),
                    ("retained_previous_plan", Value::Bool(true)),
                    (
                        "compiled_plan_id",
                        Value::String(session.snapshot.compiled_plan_id.clone()),
                    ),
                ]),
            ),
            now,
        )?;
        return Ok(mailbox_result(false, errors));
    }

    session.config = reloaded_config;
    session.compiled_plan = active_plan;
    session.watcher_session =
        build_runtime_watcher_session(&session.paths, &session.config, session.config.run_style);
    project_reload_snapshot(session, now)?;
    save_snapshot(&session.paths, &session.snapshot)?;
    write_runtime_event(
        &session.paths,
        "runtime_config_reloaded",
        mailbox_event_data(
            envelope,
            json_object([
                (
                    "mode_id",
                    Value::String(session.compiled_plan.mode_id.clone()),
                ),
                (
                    "compiled_plan_id",
                    Value::String(session.compiled_plan.compiled_plan_id.clone()),
                ),
            ]),
        ),
        now,
    )?;
    Ok(mailbox_result(true, "runtime config reloaded"))
}

fn project_reload_snapshot(
    session: &mut RuntimeStartupSession,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    session.snapshot.runtime_mode = session.config.run_style;
    session.snapshot.active_mode_id = session.compiled_plan.mode_id.clone();
    session.snapshot.execution_loop_id = session.compiled_plan.execution_loop_id.clone();
    session.snapshot.planning_loop_id = session.compiled_plan.planning_loop_id.clone();
    session.snapshot.learning_loop_id = session.compiled_plan.learning_loop_id.clone();
    session.snapshot.loop_ids_by_plane = session.compiled_plan.loop_ids_by_plane.clone();
    session.snapshot.compiled_plan_id = session.compiled_plan.compiled_plan_id.clone();
    session.snapshot.compiled_plan_path =
        workspace_relative(&session.paths, &session.paths.compiled_plan_file);
    refresh_queue_depths(&session.paths, &mut session.snapshot)?;
    session.snapshot.config_version = session.config.config_version.clone();
    session.snapshot.watcher_mode = session.watcher_session.mode;
    session.snapshot.last_reload_outcome = Some(ReloadOutcome::Applied);
    session.snapshot.last_reload_error = None;
    session.snapshot.updated_at = now.clone();
    Ok(())
}

fn archive_mailbox_command(
    paths: &WorkspacePaths,
    source_path: &Path,
    envelope: &MailboxCommandEnvelope,
    success: bool,
    result: Map<String, Value>,
    error: Option<String>,
    now: &Timestamp,
) -> RuntimeTickResult<PathBuf> {
    let archive_dir = if success {
        &paths.mailbox_processed_dir
    } else {
        &paths.mailbox_failed_dir
    };
    let archive_path = mailbox_archive_path(archive_dir, source_path)?;
    let disposition = if success { "processed" } else { "failed" };
    let mut archive_payload = json_object([
        ("schema_version", Value::String("1.0".to_owned())),
        ("kind", Value::String("mailbox_archive".to_owned())),
        ("disposition", Value::String(disposition.to_owned())),
        ("archived_at", Value::String(now.as_str().to_owned())),
        (
            "source_path",
            Value::String(workspace_relative(paths, source_path)),
        ),
        ("envelope", json_value(envelope)?),
        ("result", Value::Object(result)),
    ]);
    if let Some(error) = error {
        archive_payload.insert("error".to_owned(), Value::String(error));
    }
    write_pretty_json(&archive_path, &Value::Object(archive_payload))?;
    remove_file_if_exists(source_path)?;
    Ok(archive_path)
}

fn archive_unreadable_mailbox_payload(
    paths: &WorkspacePaths,
    source_path: &Path,
    error: &str,
    now: &Timestamp,
) -> RuntimeTickResult<PathBuf> {
    let archive_path = mailbox_archive_path(&paths.mailbox_failed_dir, source_path)?;
    let raw_payload = fs::read_to_string(source_path).ok();
    let mut archive_payload = json_object([
        ("schema_version", Value::String("1.0".to_owned())),
        ("kind", Value::String("mailbox_archive".to_owned())),
        ("disposition", Value::String("failed".to_owned())),
        ("archived_at", Value::String(now.as_str().to_owned())),
        (
            "source_path",
            Value::String(workspace_relative(paths, source_path)),
        ),
        ("error", Value::String(error.to_owned())),
    ]);
    if let Some(raw_payload) = raw_payload {
        archive_payload.insert("raw_payload".to_owned(), Value::String(raw_payload));
    }
    write_pretty_json(&archive_path, &Value::Object(archive_payload))?;
    remove_file_if_exists(source_path)?;
    Ok(archive_path)
}

fn mailbox_archive_path(destination_dir: &Path, source_path: &Path) -> RuntimeTickResult<PathBuf> {
    create_dir_all(destination_dir)?;
    let filename = source_path
        .file_name()
        .ok_or_else(|| invalid_state("mailbox command path is missing a filename"))?;
    let mut destination = destination_dir.join(filename);
    if destination.exists() {
        let stem = source_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("mailbox-command");
        let extension = source_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("json");
        let suffix = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        destination = destination_dir.join(format!("{stem}.{suffix}.{extension}"));
    }
    Ok(destination)
}

fn write_mailbox_envelope(
    paths: &WorkspacePaths,
    envelope: &MailboxCommandEnvelope,
) -> RuntimeTickResult<PathBuf> {
    let mut validated = envelope.clone();
    validated
        .validate_contract()
        .map_err(|source| RuntimeTickError::InvalidState {
            message: format!("mailbox command is invalid: {source}"),
        })?;
    let destination = paths
        .mailbox_incoming_dir
        .join(mailbox_command_filename(&validated.command_id)?);
    if destination.exists() {
        return Err(invalid_state(format!(
            "mailbox command already exists: {}",
            destination.display()
        )));
    }
    write_pretty_json(&destination, &validated)?;
    Ok(destination)
}

fn mailbox_command_filename(command_id: &str) -> RuntimeTickResult<String> {
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
        return Err(invalid_state(
            "command_id must include at least one filename-safe character",
        ));
    }
    Ok(format!("{safe}.json"))
}

fn remove_file_if_exists(path: &Path) -> RuntimeTickResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path, error)),
    }
}

fn mailbox_result(applied: bool, detail: impl Into<String>) -> Map<String, Value> {
    json_object([
        ("applied", Value::Bool(applied)),
        ("detail", Value::String(detail.into())),
    ])
}

fn mailbox_result_with_path(
    applied: bool,
    detail: impl Into<String>,
    path: String,
) -> Map<String, Value> {
    let mut result = mailbox_result(applied, detail);
    result.insert("path".to_owned(), Value::String(path));
    result
}

fn mailbox_event_data(
    envelope: &MailboxCommandEnvelope,
    mut extra: Map<String, Value>,
) -> Map<String, Value> {
    extra.insert(
        "command_id".to_owned(),
        Value::String(envelope.command_id.clone()),
    );
    extra.insert(
        "command".to_owned(),
        Value::String(envelope.command.as_str().to_owned()),
    );
    extra.insert("issuer".to_owned(), Value::String(envelope.issuer.clone()));
    extra
}

fn mailbox_reason(envelope: &MailboxCommandEnvelope, default: &str) -> String {
    envelope
        .payload
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default.to_owned())
}

fn mailbox_retry_scope(envelope: &MailboxCommandEnvelope) -> RuntimeTickResult<Option<Plane>> {
    let Some(value) = envelope.payload.get("scope") else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(invalid_state("retry_active scope must be a string"));
    };
    Plane::from_value(value)
        .map(Some)
        .map_err(|_| invalid_state(format!("Unsupported retry_active scope: {value}")))
}

fn active_run_for_retry(
    snapshot: &RuntimeSnapshot,
    scope: Option<Plane>,
) -> Option<ActiveRunState> {
    if let Some(scope) = scope {
        return active_run_for_plane(snapshot, scope);
    }
    if snapshot.active_runs_by_plane.len() == 1 {
        return snapshot.active_runs_by_plane.values().next().cloned();
    }
    if snapshot.active_runs_by_plane.len() > 1 {
        return [Plane::Planning, Plane::Execution, Plane::Learning]
            .into_iter()
            .find_map(|plane| snapshot.active_runs_by_plane.get(&plane).cloned());
    }
    snapshot
        .active_plane
        .and_then(|plane| active_run_for_plane(snapshot, plane))
}

fn retry_active_missing_detail(snapshot: &RuntimeSnapshot, scope: Option<Plane>) -> String {
    if let Some(scope) = scope {
        let active_planes = active_plane_values(snapshot);
        let active_planes = if active_planes.is_empty() {
            "none".to_owned()
        } else {
            active_planes.join(", ")
        };
        format!(
            "{} retry requires matching active plane; current active planes are {active_planes}",
            scope.as_str()
        )
    } else {
        "no active work item to retry".to_owned()
    }
}

fn emit_retry_active_skipped(
    session: &RuntimeStartupSession,
    envelope: &MailboxCommandEnvelope,
    scope: Option<Plane>,
    reason: &str,
    now: &Timestamp,
) -> RuntimeTickResult<PathBuf> {
    let active_planes = active_plane_values(&session.snapshot)
        .into_iter()
        .map(Value::String)
        .collect::<Vec<_>>();
    write_runtime_event(
        &session.paths,
        "retry_active_skipped",
        mailbox_event_data(
            envelope,
            json_object([
                ("reason", Value::String(reason.to_owned())),
                (
                    "requested_scope",
                    scope
                        .map(|plane| Value::String(plane.as_str().to_owned()))
                        .unwrap_or(Value::Null),
                ),
                ("active_planes", Value::Array(active_planes)),
            ]),
        ),
        now,
    )
}

fn clear_retry_active_plane(
    session: &mut RuntimeStartupSession,
    plane: Plane,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    session.snapshot.active_runs_by_plane.remove(&plane);
    if session.snapshot.active_runs_by_plane.is_empty() {
        reset_runtime_to_idle(
            &session.paths,
            &mut session.snapshot,
            true,
            false,
            false,
            now,
        )?;
        return Ok(());
    }

    project_foreground_active_run(&mut session.snapshot);
    session.snapshot.current_failure_class = None;
    session.snapshot.updated_at = now.clone();
    save_snapshot(&session.paths, &session.snapshot)?;
    set_status_for_plane(&session.paths, plane, IDLE_STATUS_MARKER)?;
    Ok(())
}

fn requeue_all_active_items(
    paths: &WorkspacePaths,
    queue: &QueueStore,
    reason: &str,
) -> RuntimeTickResult<u64> {
    let mut requeued_count = 0;
    for task_id in markdown_stems(&paths.tasks_active_dir)? {
        if ignore_invalid_queue_state(queue.requeue_task(&task_id, reason))? {
            requeued_count += 1;
        }
    }
    for spec_id in markdown_stems(&paths.specs_active_dir)? {
        if ignore_invalid_queue_state(queue.requeue_spec(&spec_id, reason))? {
            requeued_count += 1;
        }
    }
    for probe_id in markdown_stems(&paths.probes_active_dir)? {
        if ignore_invalid_queue_state(queue.requeue_probe(&probe_id, reason))? {
            requeued_count += 1;
        }
    }
    for incident_id in markdown_stems(&paths.incidents_active_dir)? {
        if ignore_invalid_queue_state(queue.requeue_incident(&incident_id, reason))? {
            requeued_count += 1;
        }
    }
    for learning_request_id in markdown_stems(&paths.learning_requests_active_dir)? {
        if ignore_invalid_queue_state(queue.requeue_learning_request(&learning_request_id, reason))?
        {
            requeued_count += 1;
        }
    }
    Ok(requeued_count)
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

fn ignore_invalid_queue_state(result: Result<PathBuf, QueueStoreError>) -> RuntimeTickResult<bool> {
    match result {
        Ok(_) => Ok(true),
        Err(QueueStoreError::InvalidState { .. }) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn markdown_stems(directory: &Path) -> RuntimeTickResult<Vec<String>> {
    let mut stems = Vec::new();
    for path in markdown_files(directory)? {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                invalid_state(format!(
                    "markdown path has no valid stem: {}",
                    path.display()
                ))
            })?;
        stems.push(stem.to_owned());
    }
    stems.sort();
    Ok(stems)
}

fn active_plane_values(snapshot: &RuntimeSnapshot) -> Vec<String> {
    [Plane::Planning, Plane::Execution, Plane::Learning]
        .into_iter()
        .filter(|plane| snapshot.active_runs_by_plane.contains_key(plane))
        .map(|plane| plane.as_str().to_owned())
        .collect()
}

fn deferred_reload_command_id(envelope: &MailboxCommandEnvelope) -> String {
    let suffix = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-deferred-{suffix:08x}", envelope.command.as_str())
}

fn json_value<T: Serialize>(value: &T) -> RuntimeTickResult<Value> {
    serde_json::to_value(value).map_err(|error| RuntimeTickError::InvalidState {
        message: error.to_string(),
    })
}

fn workspace_relative(paths: &WorkspacePaths, path: &Path) -> String {
    path.strip_prefix(&paths.root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn render_work_item_kind_list(kinds: &[WorkItemKind]) -> String {
    kinds
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn activation_for_claim(
    plan: &CompiledRunPlan,
    claim: &QueueClaim,
) -> RuntimeTickResult<GraphActivationDecision> {
    if claim.work_item_kind == WorkItemKind::LearningRequest {
        let document = read_learning_request(&claim.path)?;
        if let Some(target_stage) = document.target_stage {
            return learning_stage_activation_for_graph(plan, StageName::from(target_stage));
        }
    }
    work_item_activation_for_graph(plan, claim.work_item_kind)
}

fn work_item_activation_for_graph(
    plan: &CompiledRunPlan,
    work_item_kind: WorkItemKind,
) -> RuntimeTickResult<GraphActivationDecision> {
    let (graph, entry_key) = match work_item_kind {
        WorkItemKind::Task => (&plan.execution_graph, GraphLoopEntryKey::Task),
        WorkItemKind::Probe => (&plan.planning_graph, GraphLoopEntryKey::Probe),
        WorkItemKind::Spec => (&plan.planning_graph, GraphLoopEntryKey::Spec),
        WorkItemKind::Incident => (&plan.planning_graph, GraphLoopEntryKey::Incident),
        WorkItemKind::LearningRequest => (
            plan.learning_graph
                .as_ref()
                .ok_or_else(|| invalid_state("compiled graph is missing learning plane"))?,
            GraphLoopEntryKey::LearningRequest,
        ),
    };
    activation_from_entry(graph, entry_key)
}

fn learning_stage_activation_for_graph(
    plan: &CompiledRunPlan,
    target_stage: StageName,
) -> RuntimeTickResult<GraphActivationDecision> {
    let graph = plan
        .learning_graph
        .as_ref()
        .ok_or_else(|| invalid_state("compiled graph is missing learning plane"))?;
    let node = graph
        .nodes
        .iter()
        .find(|node| {
            stage_name_for_plane(graph.plane, &node.stage_kind_id)
                .ok()
                .is_some_and(|stage| stage == target_stage)
        })
        .ok_or_else(|| {
            invalid_state(format!(
                "compiled graph is missing learning stage kind `{}`",
                target_stage.as_str()
            ))
        })?;
    activation_from_node(graph, &node.node_id, GraphLoopEntryKey::LearningRequest)
}

fn completion_activation_for_graph(
    plan: &CompiledRunPlan,
) -> RuntimeTickResult<GraphActivationDecision> {
    let entry = plan
        .planning_graph
        .compiled_completion_entry
        .as_ref()
        .ok_or_else(|| {
            invalid_state("compiled graph is missing closure_target completion entry")
        })?;
    activation_from_completion_entry(&plan.planning_graph, entry)
}

fn activation_from_entry(
    graph: &FrozenGraphPlanePlan,
    entry_key: GraphLoopEntryKey,
) -> RuntimeTickResult<GraphActivationDecision> {
    let entry = graph
        .compiled_entries
        .iter()
        .find(|entry| entry.entry_key == entry_key)
        .ok_or_else(|| {
            invalid_state(format!(
                "compiled graph is missing `{}` activation entry",
                entry_key.as_str()
            ))
        })?;
    Ok(GraphActivationDecision {
        plane: graph.plane,
        stage: stage_name_for_plane(graph.plane, &entry.stage_kind_id)?,
        node_id: entry.node_id.clone(),
        stage_kind_id: entry.stage_kind_id.clone(),
    })
}

fn activation_from_node(
    graph: &FrozenGraphPlanePlan,
    node_id: &str,
    _entry_key: GraphLoopEntryKey,
) -> RuntimeTickResult<GraphActivationDecision> {
    let node = graph
        .nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .ok_or_else(|| invalid_state(format!("compiled graph is missing `{node_id}` node")))?;
    Ok(GraphActivationDecision {
        plane: graph.plane,
        stage: stage_name_for_plane(graph.plane, &node.stage_kind_id)?,
        node_id: node.node_id.clone(),
        stage_kind_id: node.stage_kind_id.clone(),
    })
}

fn activation_from_completion_entry(
    graph: &FrozenGraphPlanePlan,
    entry: &CompiledGraphCompletionEntryPlan,
) -> RuntimeTickResult<GraphActivationDecision> {
    Ok(GraphActivationDecision {
        plane: graph.plane,
        stage: stage_name_for_plane(graph.plane, &entry.stage_kind_id)?,
        node_id: entry.node_id.clone(),
        stage_kind_id: entry.stage_kind_id.clone(),
    })
}

fn stage_plan_for_active_state<'a>(
    plan: &'a CompiledRunPlan,
    plane: Plane,
    stage: StageName,
    node_id: Option<&str>,
) -> RuntimeTickResult<&'a MaterializedGraphNodePlan> {
    let graph = graph_for_plane(plan, plane)?;
    if let Some(node_id) = node_id {
        if let Some(node) = graph.nodes.iter().find(|node| node.node_id == node_id) {
            return Ok(node);
        }
    }
    graph
        .nodes
        .iter()
        .find(|node| {
            stage_name_for_plane(plane, &node.stage_kind_id)
                .ok()
                .is_some_and(|node_stage| node_stage == stage)
        })
        .ok_or_else(|| {
            invalid_state(format!(
                "no compiled graph node plan for {}:{}",
                plane.as_str(),
                stage.as_str()
            ))
        })
}

fn graph_for_plane(
    plan: &CompiledRunPlan,
    plane: Plane,
) -> RuntimeTickResult<&FrozenGraphPlanePlan> {
    match plane {
        Plane::Execution => Ok(&plan.execution_graph),
        Plane::Planning => Ok(&plan.planning_graph),
        Plane::Learning => plan
            .learning_graph
            .as_ref()
            .ok_or_else(|| invalid_state("compiled graph is missing learning plane")),
    }
}

fn policy_for_stage_plan(
    stage: StageName,
    stage_plan: &MaterializedGraphNodePlan,
) -> AllowedResultClassesByOutcome {
    let default_policy = AllowedResultClassesByOutcome::for_stage(stage);
    if stage_plan.allowed_result_classes_by_outcome.is_empty() {
        return default_policy;
    }
    let mut remaining: HashMap<_, _> = stage_plan
        .allowed_result_classes_by_outcome
        .iter()
        .map(|(outcome, classes)| (outcome.clone(), classes.clone()))
        .collect();
    let mut entries = Vec::new();
    for marker in crate::contracts::legal_terminal_markers(stage) {
        if let Some(outcome) = marker.strip_prefix("### ") {
            if let Some(result_classes) = remaining.remove(outcome).or_else(|| {
                default_policy
                    .result_classes_for(outcome)
                    .map(<[_]>::to_vec)
            }) {
                entries.push(AllowedResultClassPolicy {
                    outcome: outcome.to_owned(),
                    result_classes,
                });
            }
        }
    }
    let mut leftover = remaining.into_iter().collect::<Vec<_>>();
    leftover.sort_by(|left, right| left.0.cmp(&right.0));
    entries.extend(leftover.into_iter().map(|(outcome, result_classes)| {
        AllowedResultClassPolicy {
            outcome,
            result_classes,
        }
    }));
    AllowedResultClassesByOutcome::new(entries)
}

fn active_closure_target(paths: &WorkspacePaths) -> RuntimeTickResult<Option<ClosureTargetState>> {
    let open_targets = open_closure_targets(paths)?;
    let actionable_targets = actionable_open_closure_targets(open_targets);
    match actionable_targets.len() {
        0 => Ok(None),
        1 => Ok(actionable_targets.into_iter().next()),
        _ => Err(invalid_state(
            "multiple actionable open closure targets found",
        )),
    }
}

fn active_closure_target_for_snapshot(
    paths: &WorkspacePaths,
    snapshot: &RuntimeSnapshot,
) -> RuntimeTickResult<ClosureTargetState> {
    let active_run = snapshot
        .active_plane
        .and_then(|plane| active_run_for_plane(snapshot, plane))
        .ok_or_else(|| invalid_state("closure stage is active without an active run"))?;
    let root_spec_id = active_run
        .closure_target_root_spec_id
        .as_deref()
        .ok_or_else(|| invalid_state("closure stage is active without closure root spec id"))?;
    crate::workspace::load_closure_target_state(paths, root_spec_id).map_err(Into::into)
}

fn active_closure_target_for_active_run(
    paths: &WorkspacePaths,
    active_run: &ActiveRunState,
) -> RuntimeTickResult<ClosureTargetState> {
    let root_spec_id = active_run
        .closure_target_root_spec_id
        .as_deref()
        .ok_or_else(|| invalid_state("closure stage is active without closure root spec id"))?;
    crate::workspace::load_closure_target_state(paths, root_spec_id).map_err(Into::into)
}

fn open_closure_targets(paths: &WorkspacePaths) -> RuntimeTickResult<Vec<ClosureTargetState>> {
    let mut targets = Vec::new();
    for path in json_files(&paths.arbiter_targets_dir)? {
        let raw = fs::read_to_string(&path).map_err(|error| io_error(&path, error))?;
        let target: ClosureTargetState =
            serde_json::from_str(&raw).map_err(|error| RuntimeTickError::InvalidState {
                message: format!("closure target {} is invalid: {error}", path.display()),
            })?;
        target
            .validate()
            .map_err(|source| RuntimeTickError::WorkDocument {
                path: path.clone(),
                source,
            })?;
        if target.closure_open {
            targets.push(target);
        }
    }
    targets.sort_by(|left, right| left.root_spec_id.cmp(&right.root_spec_id));
    Ok(targets)
}

fn actionable_open_closure_targets(
    open_targets: Vec<ClosureTargetState>,
) -> Vec<ClosureTargetState> {
    open_targets
        .into_iter()
        .filter(|target| !target.closure_blocked_by_lineage_work)
        .collect()
}

fn refresh_closure_target_readiness(
    paths: &WorkspacePaths,
    mut target: ClosureTargetState,
) -> RuntimeTickResult<ClosureTargetState> {
    let blocking_work_ids = open_lineage_work_ids(paths, &target.root_spec_id)?;
    target.closure_blocked_by_lineage_work = !blocking_work_ids.is_empty();
    target.blocking_work_ids = blocking_work_ids;
    save_closure_target_state(paths, &target)?;
    Ok(target)
}

fn open_lineage_work_ids(
    paths: &WorkspacePaths,
    root_spec_id: &str,
) -> RuntimeTickResult<Vec<String>> {
    let mut ids = BTreeSet::new();
    collect_task_lineage_ids(&paths.tasks_queue_dir, root_spec_id, &mut ids)?;
    collect_task_lineage_ids(&paths.tasks_active_dir, root_spec_id, &mut ids)?;
    collect_task_lineage_ids(&paths.tasks_blocked_dir, root_spec_id, &mut ids)?;
    collect_spec_lineage_ids(&paths.specs_queue_dir, root_spec_id, &mut ids)?;
    collect_spec_lineage_ids(&paths.specs_active_dir, root_spec_id, &mut ids)?;
    collect_spec_lineage_ids(&paths.specs_blocked_dir, root_spec_id, &mut ids)?;
    collect_incident_lineage_ids(&paths.incidents_incoming_dir, root_spec_id, &mut ids)?;
    collect_incident_lineage_ids(&paths.incidents_active_dir, root_spec_id, &mut ids)?;
    collect_incident_lineage_ids(&paths.incidents_blocked_dir, root_spec_id, &mut ids)?;
    Ok(ids.into_iter().collect())
}

fn collect_task_lineage_ids(
    directory: &Path,
    root_spec_id: &str,
    ids: &mut BTreeSet<String>,
) -> RuntimeTickResult<()> {
    for path in markdown_files(directory)? {
        let raw = fs::read_to_string(&path).map_err(|error| io_error(&path, error))?;
        let document = parse_task_document_with_source(&raw, &path.display().to_string()).map_err(
            |source| RuntimeTickError::WorkDocument {
                path: path.clone(),
                source,
            },
        )?;
        if document
            .root_spec_id
            .as_deref()
            .or(document.spec_id.as_deref())
            == Some(root_spec_id)
        {
            ids.insert(document.task_id);
        }
    }
    Ok(())
}

fn collect_spec_lineage_ids(
    directory: &Path,
    root_spec_id: &str,
    ids: &mut BTreeSet<String>,
) -> RuntimeTickResult<()> {
    for path in markdown_files(directory)? {
        let raw = fs::read_to_string(&path).map_err(|error| io_error(&path, error))?;
        let document = parse_spec_document_with_source(&raw, &path.display().to_string()).map_err(
            |source| RuntimeTickError::WorkDocument {
                path: path.clone(),
                source,
            },
        )?;
        let effective_root = document.root_spec_id.as_deref().or({
            if matches!(
                document.source_type,
                SpecSourceType::Idea | SpecSourceType::Manual
            ) {
                Some(document.spec_id.as_str())
            } else {
                None
            }
        });
        if effective_root == Some(root_spec_id) {
            ids.insert(document.spec_id);
        }
    }
    Ok(())
}

fn collect_incident_lineage_ids(
    directory: &Path,
    root_spec_id: &str,
    ids: &mut BTreeSet<String>,
) -> RuntimeTickResult<()> {
    for path in markdown_files(directory)? {
        let raw = fs::read_to_string(&path).map_err(|error| io_error(&path, error))?;
        let document = parse_incident_document_with_source(&raw, &path.display().to_string())
            .map_err(|source| RuntimeTickError::WorkDocument {
                path: path.clone(),
                source,
            })?;
        if document
            .root_spec_id
            .as_deref()
            .or(document.source_spec_id.as_deref())
            == Some(root_spec_id)
        {
            ids.insert(document.incident_id);
        }
    }
    Ok(())
}

fn read_learning_request(path: &Path) -> RuntimeTickResult<LearningRequestDocument> {
    let raw = fs::read_to_string(path).map_err(|error| io_error(path, error))?;
    parse_learning_request_document_with_source(&raw, &path.display().to_string()).map_err(
        |source| RuntimeTickError::WorkDocument {
            path: path.to_path_buf(),
            source,
        },
    )
}

fn refresh_queue_depths(
    paths: &WorkspacePaths,
    snapshot: &mut RuntimeSnapshot,
) -> RuntimeTickResult<()> {
    set_queue_depth(
        snapshot,
        Plane::Execution,
        count_markdown_files(&paths.tasks_queue_dir)?,
    );
    set_queue_depth(
        snapshot,
        Plane::Planning,
        count_markdown_files(&paths.probes_queue_dir)?
            + count_markdown_files(&paths.specs_queue_dir)?
            + count_markdown_files(&paths.incidents_incoming_dir)?,
    );
    set_queue_depth(
        snapshot,
        Plane::Learning,
        count_markdown_files(&paths.learning_requests_queue_dir)?,
    );
    Ok(())
}

fn reset_runtime_to_idle(
    paths: &WorkspacePaths,
    snapshot: &mut RuntimeSnapshot,
    process_running: bool,
    clear_stop_requested: bool,
    clear_paused: bool,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    snapshot.process_running = process_running;
    snapshot.active_plane = None;
    snapshot.active_stage = None;
    snapshot.active_node_id = None;
    snapshot.active_stage_kind_id = None;
    snapshot.active_run_id = None;
    snapshot.active_work_item_kind = None;
    snapshot.active_work_item_id = None;
    snapshot.active_runs_by_plane.clear();
    snapshot.active_since = None;
    snapshot.current_failure_class = None;
    snapshot.troubleshoot_attempt_count = 0;
    snapshot.mechanic_attempt_count = 0;
    snapshot.fix_cycle_count = 0;
    snapshot.consultant_invocations = 0;
    if clear_paused {
        snapshot.paused = false;
        snapshot.pause_sources.clear();
    }
    if clear_stop_requested {
        snapshot.stop_requested = false;
    }
    set_snapshot_status_for_plane(snapshot, Plane::Execution, IDLE_STATUS_MARKER);
    set_snapshot_status_for_plane(snapshot, Plane::Planning, IDLE_STATUS_MARKER);
    set_snapshot_status_for_plane(snapshot, Plane::Learning, IDLE_STATUS_MARKER);
    refresh_queue_depths(paths, snapshot)?;
    snapshot.updated_at = now.clone();
    set_execution_status(paths, IDLE_STATUS_MARKER)?;
    set_planning_status(paths, IDLE_STATUS_MARKER)?;
    set_learning_status(paths, IDLE_STATUS_MARKER)?;
    save_snapshot(paths, snapshot)?;
    Ok(())
}

fn snapshot_with_active_run(
    snapshot: &mut RuntimeSnapshot,
    active_run: ActiveRunState,
    now: &Timestamp,
) {
    snapshot
        .active_runs_by_plane
        .insert(active_run.plane, active_run);
    project_foreground_active_run(snapshot);
    snapshot.updated_at = now.clone();
}

fn project_foreground_active_run(snapshot: &mut RuntimeSnapshot) {
    let active_run = [Plane::Planning, Plane::Execution, Plane::Learning]
        .into_iter()
        .find_map(|plane| snapshot.active_runs_by_plane.get(&plane));
    if let Some(active_run) = active_run {
        snapshot.active_plane = Some(active_run.plane);
        snapshot.active_stage = Some(active_run.stage);
        snapshot.active_node_id = Some(active_run.node_id.clone());
        snapshot.active_stage_kind_id = Some(active_run.stage_kind_id.clone());
        snapshot.active_run_id = Some(active_run.run_id.clone());
        snapshot.active_work_item_kind = active_run.work_item_kind;
        snapshot.active_work_item_id = active_run.work_item_id.clone();
        snapshot.active_since = Some(active_run.active_since.clone());
    } else {
        snapshot.active_plane = None;
        snapshot.active_stage = None;
        snapshot.active_node_id = None;
        snapshot.active_stage_kind_id = None;
        snapshot.active_run_id = None;
        snapshot.active_work_item_kind = None;
        snapshot.active_work_item_id = None;
        snapshot.active_since = None;
    }
}

pub(crate) fn active_run_for_plane(
    snapshot: &RuntimeSnapshot,
    plane: Plane,
) -> Option<ActiveRunState> {
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
            ActiveRunRequestKind::LearningRequest
        } else {
            ActiveRunRequestKind::ActiveWorkItem
        },
        work_item_kind: Some(active_work_item_kind),
        work_item_id: Some(active_work_item_id),
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        active_since,
        running_status_marker: None,
    })
}

fn set_queue_depth(snapshot: &mut RuntimeSnapshot, plane: Plane, depth: u64) {
    match plane {
        Plane::Execution => snapshot.queue_depth_execution = depth,
        Plane::Planning => snapshot.queue_depth_planning = depth,
        Plane::Learning => snapshot.queue_depth_learning = depth,
    }
    snapshot.queue_depths_by_plane.insert(plane, depth);
}

fn set_snapshot_status_for_plane(snapshot: &mut RuntimeSnapshot, plane: Plane, marker: &str) {
    match plane {
        Plane::Execution => snapshot.execution_status_marker = marker.to_owned(),
        Plane::Planning => snapshot.planning_status_marker = marker.to_owned(),
        Plane::Learning => snapshot.learning_status_marker = marker.to_owned(),
    }
    snapshot
        .status_markers_by_plane
        .insert(plane, marker.to_owned());
}

fn set_status_for_plane(
    paths: &WorkspacePaths,
    plane: Plane,
    marker: &str,
) -> RuntimeTickResult<String> {
    Ok(match plane {
        Plane::Execution => set_execution_status(paths, marker)?,
        Plane::Planning => set_planning_status(paths, marker)?,
        Plane::Learning => set_learning_status(paths, marker)?,
    })
}

fn status_path_for_plane(paths: &WorkspacePaths, plane: Plane) -> &Path {
    match plane {
        Plane::Execution => &paths.execution_status_file,
        Plane::Planning => &paths.planning_status_file,
        Plane::Learning => &paths.learning_status_file,
    }
}

fn active_work_item_path(
    paths: &WorkspacePaths,
    work_item_kind: Option<WorkItemKind>,
    work_item_id: Option<&str>,
) -> Option<PathBuf> {
    let work_item_id = work_item_id?;
    Some(match work_item_kind? {
        WorkItemKind::Task => paths.tasks_active_dir.join(format!("{work_item_id}.md")),
        WorkItemKind::Probe => paths.probes_active_dir.join(format!("{work_item_id}.md")),
        WorkItemKind::Spec => paths.specs_active_dir.join(format!("{work_item_id}.md")),
        WorkItemKind::Incident => paths
            .incidents_active_dir
            .join(format!("{work_item_id}.md")),
        WorkItemKind::LearningRequest => paths
            .learning_requests_active_dir
            .join(format!("{work_item_id}.md")),
    })
}

fn request_kind_for_active_run(active_run: &ActiveRunState) -> RequestKind {
    match active_run.request_kind {
        ActiveRunRequestKind::ActiveWorkItem => RequestKind::ActiveWorkItem,
        ActiveRunRequestKind::ClosureTarget => RequestKind::ClosureTarget,
        ActiveRunRequestKind::LearningRequest => RequestKind::LearningRequest,
    }
}

fn runtime_asset_paths(paths: &WorkspacePaths, values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| paths.runtime_root.join(value).display().to_string())
        .collect()
}

fn write_skill_revision_evidence_if_enabled(
    session: &RuntimeStartupSession,
    run_dir: &Path,
    request_id: &str,
    run_id: &str,
    required_skill_paths: &[String],
    attached_skill_paths: &[String],
    now: &Timestamp,
) -> RuntimeTickResult<Option<PathBuf>> {
    if session.compiled_plan.learning_graph.is_none() {
        return Ok(None);
    }
    let evidence_path = run_dir.join(format!("skill_revision_evidence.{request_id}.json"));
    let skills = required_skill_paths
        .iter()
        .chain(attached_skill_paths.iter())
        .map(|path| skill_evidence(path))
        .collect::<RuntimeTickResult<Vec<_>>>()?;
    let payload = json!({
        "schema_version": "1.0",
        "kind": "skill_revision_evidence",
        "request_id": request_id,
        "run_id": run_id,
        "mode_id": session.snapshot.active_mode_id,
        "compiled_plan_id": session.snapshot.compiled_plan_id,
        "emitted_at": now.as_str(),
        "skills": skills,
    });
    write_pretty_json(&evidence_path, &payload)?;
    Ok(Some(evidence_path))
}

fn skill_evidence(path: &str) -> RuntimeTickResult<Value> {
    let path_buf = PathBuf::from(path);
    let exists = path_buf.is_file();
    if exists {
        let content = fs::read(&path_buf).map_err(|error| io_error(&path_buf, error))?;
        let mut digest = Sha256::new();
        digest.update(&content);
        Ok(json!({
            "path": path,
            "exists": true,
            "sha256": format!("{:x}", digest.finalize()),
            "size_bytes": content.len(),
        }))
    } else {
        Ok(json!({
            "path": path,
            "exists": false,
            "sha256": Value::Null,
            "size_bytes": Value::Null,
        }))
    }
}

fn write_runtime_event(
    paths: &WorkspacePaths,
    event_type: &str,
    data: Map<String, Value>,
    occurred_at: &Timestamp,
) -> RuntimeTickResult<PathBuf> {
    let event_type = event_type.trim();
    if event_type.is_empty() {
        return Err(invalid_state("runtime event payload is missing event_type"));
    }
    let log_path = paths.logs_dir.join("runtime_events.jsonl");
    if let Some(parent) = log_path.parent() {
        create_dir_all(parent)?;
    }
    let payload = json!({
        "schema_version": "1.0",
        "kind": "runtime_event",
        "event_type": event_type,
        "occurred_at": occurred_at.as_str(),
        "data": data,
    });
    let line = serde_json::to_string(&payload).map_err(|error| RuntimeTickError::InvalidState {
        message: error.to_string(),
    })? + "\n";
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| io_error(&log_path, error))?;
    file.write_all(line.as_bytes())
        .map_err(|error| io_error(&log_path, error))?;
    Ok(log_path)
}

fn stage_started_data(request: &StageRunRequest) -> Map<String, Value> {
    json_object([
        ("request_id", Value::String(request.request_id.clone())),
        ("stage", Value::String(request.stage.as_str().to_owned())),
        ("node_id", Value::String(request.node_id.clone())),
        (
            "stage_kind_id",
            Value::String(request.stage_kind_id.clone()),
        ),
        ("plane", Value::String(request.plane.as_str().to_owned())),
        ("run_id", Value::String(request.run_id.clone())),
        (
            "status_marker",
            Value::String(running_status_marker(&request.running_status_marker)),
        ),
        (
            "work_item_kind",
            request
                .active_work_item_kind
                .map(|kind| Value::String(kind.as_str().to_owned()))
                .unwrap_or(Value::Null),
        ),
        (
            "work_item_id",
            request
                .active_work_item_id
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "troubleshoot_report_path",
            request
                .preferred_troubleshoot_report_path
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
    ])
}

fn stage_completed_data(
    request: &StageRunRequest,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> Map<String, Value> {
    json_object([
        ("request_id", Value::String(request.request_id.clone())),
        (
            "stage",
            Value::String(stage_result.stage.as_str().to_owned()),
        ),
        ("node_id", Value::String(stage_result.node_id.clone())),
        (
            "stage_kind_id",
            Value::String(stage_result.stage_kind_id.clone()),
        ),
        (
            "plane",
            Value::String(stage_result.plane.as_str().to_owned()),
        ),
        ("run_id", Value::String(stage_result.run_id.clone())),
        (
            "work_item_kind",
            Value::String(stage_result.work_item_kind.as_str().to_owned()),
        ),
        (
            "work_item_id",
            Value::String(stage_result.work_item_id.clone()),
        ),
        (
            "terminal_result",
            Value::String(stage_result.terminal_result.as_str().to_owned()),
        ),
        (
            "result_class",
            Value::String(stage_result.result_class.as_str().to_owned()),
        ),
        (
            "summary_status_marker",
            Value::String(stage_result.summary_status_marker.clone()),
        ),
        ("duration_seconds", json!(stage_result.duration_seconds)),
        (
            "started_at",
            Value::String(stage_result.started_at.as_str().to_owned()),
        ),
        (
            "completed_at",
            Value::String(stage_result.completed_at.as_str().to_owned()),
        ),
        ("token_usage", json!(stage_result.token_usage)),
        (
            "failure_class",
            failure_class_from_stage_result(stage_result)
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "stage_result_path",
            Value::String(stage_result_path.display().to_string()),
        ),
        (
            "troubleshoot_report_path",
            stage_result
                .report_artifact
                .clone()
                .or_else(|| request.preferred_troubleshoot_report_path.clone())
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
    ])
}

fn router_decision_data(
    request: &StageRunRequest,
    stage_result: &StageResultEnvelope,
    decision: &RouterDecision,
) -> Map<String, Value> {
    json_object([
        ("request_id", Value::String(request.request_id.clone())),
        ("action", Value::String(decision.action.as_str().to_owned())),
        (
            "plane",
            Value::String(stage_result.plane.as_str().to_owned()),
        ),
        ("run_id", Value::String(stage_result.run_id.clone())),
        (
            "work_item_kind",
            Value::String(stage_result.work_item_kind.as_str().to_owned()),
        ),
        (
            "work_item_id",
            Value::String(stage_result.work_item_id.clone()),
        ),
        (
            "stage",
            Value::String(stage_result.stage.as_str().to_owned()),
        ),
        ("node_id", Value::String(stage_result.node_id.clone())),
        (
            "stage_kind_id",
            Value::String(stage_result.stage_kind_id.clone()),
        ),
        (
            "terminal_result",
            Value::String(stage_result.terminal_result.as_str().to_owned()),
        ),
        (
            "failure_class",
            decision
                .failure_class
                .clone()
                .or_else(|| failure_class_from_stage_result(stage_result))
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "next_stage",
            decision
                .next_stage
                .map(|stage| Value::String(stage.as_str().to_owned()))
                .unwrap_or(Value::Null),
        ),
        (
            "next_node_id",
            decision
                .next_node_id
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "next_stage_kind_id",
            decision
                .next_stage_kind_id
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        ("reason", Value::String(decision.reason.clone())),
    ])
}

fn write_pretty_json<T: Serialize>(path: &Path, value: &T) -> RuntimeTickResult<()> {
    let mut payload =
        serde_json::to_string_pretty(value).map_err(|error| RuntimeTickError::InvalidState {
            message: error.to_string(),
        })?;
    payload.push('\n');
    atomic_write_text(path, &payload)?;
    Ok(())
}

fn stale_reconciliation_blocks_tick(session: &RuntimeStartupSession) -> bool {
    if session.snapshot.active_stage.is_some() {
        return false;
    }
    session.reconciliation.execution.is_stale
        || session.reconciliation.planning.is_stale
        || session.reconciliation.learning.is_stale
}

fn is_completion_stage_active(snapshot: &RuntimeSnapshot) -> bool {
    snapshot
        .active_plane
        .and_then(|plane| active_run_for_plane(snapshot, plane))
        .is_some_and(|active_run| active_run.request_kind == ActiveRunRequestKind::ClosureTarget)
}

fn count_markdown_files(directory: &Path) -> RuntimeTickResult<u64> {
    Ok(markdown_files(directory)?.len() as u64)
}

fn markdown_files(directory: &Path) -> RuntimeTickResult<Vec<PathBuf>> {
    files_with_extension(directory, "md")
}

fn json_files(directory: &Path) -> RuntimeTickResult<Vec<PathBuf>> {
    files_with_extension(directory, "json")
}

fn files_with_extension(directory: &Path, extension: &str) -> RuntimeTickResult<Vec<PathBuf>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| io_error(directory, error))? {
        let entry = entry.map_err(|error| io_error(directory, error))?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|value| value.to_str()) == Some(extension) {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn create_dir_all(path: &Path) -> RuntimeTickResult<()> {
    fs::create_dir_all(path).map_err(|error| io_error(path, error))
}

fn running_status_marker(marker: &str) -> String {
    if marker.starts_with("### ") {
        marker.to_owned()
    } else {
        format!("### {marker}")
    }
}

fn new_run_id(prefix: &str) -> String {
    let counter = RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{counter}", process::id())
}

fn new_request_id(prefix: &str) -> String {
    let counter = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{counter}", process::id())
}

fn utc_now_timestamp(field_name: &'static str) -> RuntimeTickResult<Timestamp> {
    let rendered = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| RuntimeTickError::Time {
            field_name,
            message: error.to_string(),
        })?;
    Timestamp::parse(field_name, &rendered).map_err(|error| RuntimeTickError::Time {
        field_name,
        message: error.to_string(),
    })
}

fn json_object<I>(entries: I) -> Map<String, Value>
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn invalid_state(message: impl Into<String>) -> RuntimeTickError {
    RuntimeTickError::InvalidState {
        message: message.into(),
    }
}

fn io_error(path: &Path, error: io::Error) -> RuntimeTickError {
    RuntimeTickError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphActivationDecision {
    plane: Plane,
    stage: StageName,
    node_id: String,
    stage_kind_id: String,
}
