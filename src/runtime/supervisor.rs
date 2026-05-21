//! Daemon supervisor boundary for plane-aware worker dispatch.

use std::{collections::HashMap, fmt, fs, io, path::PathBuf, thread, time::Duration};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    compiler::PlaneConcurrencyPolicyDefinition,
    contracts::{
        ActiveRunRequestKind, ActiveRunState, Plane, RuntimeSnapshot, StageResultEnvelope,
        Timestamp,
    },
    runners::{RunnerError, RunnerRawResult, StageRunnerAdapter},
};

use super::{
    RequestKind, RouterDecision, RuntimeMonitorSink, RuntimeStartupSession,
    RuntimeTickDispatchOutcome, RuntimeTickError, RuntimeTickOptions, RuntimeTickOutcomeKind,
    RuntimeTickResult, StageRunRequest,
    blocked_recovery::attempt_stranded_dependency_auto_recovery,
    can_dispatch_lane, capability_gate_failure_result,
    evaluate_stage_request_capabilities_with_runner, lane_id_for_plane,
    record_capability_gate_result, runtime_monitor_events_from_jsonl,
    tick::{
        self, activate_completion_stage_if_ready, activate_next_claim_for_plane,
        apply_stage_worker_raw_result, build_stage_run_request_for_plane,
        evaluate_and_apply_usage_governance, ingest_runtime_cycle_inputs,
        mark_stage_running_and_emit_started, record_runtime_blocked_cycle,
        record_runtime_idle_cycle, record_runtime_paused_cycle, record_runtime_stopped_cycle,
        runtime_dispatch_paused, runtime_reconciliation_blocks_dispatch, runtime_stop_requested,
    },
};

const FOREGROUND_PLANES: [Plane; 2] = [Plane::Planning, Plane::Execution];
const DISPATCH_ORDER: [Plane; 3] = [Plane::Planning, Plane::Execution, Plane::Learning];

/// Typed result returned by an isolated stage worker.
#[derive(Debug, Clone, PartialEq)]
pub enum StageWorkerResult {
    /// Runner returned its normal raw result payload.
    RawResult(Box<RunnerRawResult>),
    /// Runner failed before returning a raw result.
    Exception {
        /// Stable exception type label.
        exception_type: String,
        /// Human-readable failure detail.
        exception_message: String,
        /// Runner name involved in the exception when the dispatcher could resolve it.
        runner_name: Option<String>,
    },
}

/// Typed completion payload returned by a stage worker.
#[derive(Debug, Clone, PartialEq)]
pub struct StageWorkerOutcome {
    /// Plane that owns the worker.
    pub plane: Plane,
    /// Run id the worker was launched for.
    pub run_id: String,
    /// Active-run metadata captured when the worker was launched.
    pub active_run: ActiveRunState,
    /// Stage request sent to the runner.
    pub request: StageRunRequest,
    /// Worker start timestamp.
    pub started_at: Timestamp,
    /// Worker completion timestamp.
    pub completed_at: Timestamp,
    /// Raw runner result or captured runner exception.
    pub result: StageWorkerResult,
}

impl StageWorkerOutcome {
    /// Validates that worker metadata is self-consistent before owner-side application.
    pub fn validate(&self) -> RuntimeTickResult<()> {
        if self.plane != self.active_run.plane {
            return Err(worker_mismatch(
                "worker outcome plane must match active run plane",
            ));
        }
        if self.run_id != self.active_run.run_id {
            return Err(worker_mismatch(
                "worker outcome run_id must match active run run_id",
            ));
        }
        if self.request.plane != self.plane {
            return Err(worker_mismatch(
                "worker outcome request plane must match outcome plane",
            ));
        }
        if self.request.run_id != self.run_id {
            return Err(worker_mismatch(
                "worker outcome request run_id must match outcome run_id",
            ));
        }
        if self.request.stage != self.active_run.stage {
            return Err(worker_mismatch(
                "worker outcome request stage must match active run stage",
            ));
        }
        if self.request.node_id != self.active_run.node_id {
            return Err(worker_mismatch(
                "worker outcome request node_id must match active run node_id",
            ));
        }
        if self.request.stage_kind_id != self.active_run.stage_kind_id {
            return Err(worker_mismatch(
                "worker outcome request stage_kind_id must match active run stage_kind_id",
            ));
        }
        if !request_kind_matches(self.request.request_kind, self.active_run.request_kind) {
            return Err(worker_mismatch(
                "worker outcome request kind must match active run request kind",
            ));
        }

        match self.active_run.request_kind {
            ActiveRunRequestKind::ClosureTarget => {
                if self.request.closure_target_root_spec_id.as_deref()
                    != self.active_run.closure_target_root_spec_id.as_deref()
                    || self.request.closure_target_root_idea_id.as_deref()
                        != self.active_run.closure_target_root_idea_id.as_deref()
                    || self.request.active_work_item_kind.is_some()
                    || self.request.active_work_item_id.is_some()
                {
                    return Err(worker_mismatch(
                        "worker outcome closure target identity must match active run",
                    ));
                }
            }
            ActiveRunRequestKind::ActiveWorkItem | ActiveRunRequestKind::LearningRequest => {
                if self.request.active_work_item_kind != self.active_run.work_item_kind
                    || self.request.active_work_item_id.as_deref()
                        != self.active_run.work_item_id.as_deref()
                {
                    return Err(worker_mismatch(
                        "worker outcome work item identity must match active run",
                    ));
                }
            }
        }

        match &self.result {
            StageWorkerResult::RawResult(raw_result) => raw_result.validate().map_err(Into::into),
            StageWorkerResult::Exception {
                exception_type,
                exception_message,
                runner_name,
            } => {
                if exception_type.trim().is_empty() || exception_message.trim().is_empty() {
                    Err(worker_mismatch(
                        "exception worker outcomes require exception type and message",
                    ))
                } else if runner_name
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
                {
                    Err(worker_mismatch(
                        "exception worker outcomes cannot use a blank runner name",
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }
}

/// Supervisor-owned result-application outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct StageCompletionOutcome {
    /// Plane whose worker completion was drained.
    pub plane: Plane,
    /// Run id that completed.
    pub run_id: String,
    /// Request applied by the owner.
    pub request: StageRunRequest,
    /// Normalized stage result when application reached that point.
    pub stage_result: Option<StageResultEnvelope>,
    /// Persisted stage-result path when available.
    pub stage_result_path: Option<PathBuf>,
    /// Router decision selected by graph-authoritative application.
    pub router_decision: Option<RouterDecision>,
    /// Full serial runtime dispatch outcome reused by the daemon owner.
    pub dispatch_outcome: RuntimeTickDispatchOutcome,
}

/// One daemon supervisor cycle outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeDaemonCycleOutcome {
    /// High-level cycle class.
    pub kind: RuntimeTickOutcomeKind,
    /// Stable reason token.
    pub reason: String,
    /// Completed workers applied at the start of the cycle.
    pub completions: Vec<StageCompletionOutcome>,
    /// Number of workers started during the cycle.
    pub dispatched_count: usize,
    /// Snapshot after completion draining and any dispatches.
    pub snapshot: RuntimeSnapshot,
    /// Runtime event log path when this cycle emitted a cycle-level event.
    pub event_log_path: Option<PathBuf>,
}

/// Reason a bounded daemon loop stopped running supervisor cycles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeDaemonLoopExitReason {
    /// The configured maximum completed tick count was reached.
    MaxTicks,
    /// The daemon observed a stop request and drained worker completions.
    StopRequested,
    /// The runtime snapshot no longer marked the daemon process as running.
    ProcessStopped,
    /// One no-work cycle completed and the caller requested an idle exit.
    NoWorkIdle,
    /// Runtime reconciliation blocked new dispatch safely.
    Blocked,
}

impl RuntimeDaemonLoopExitReason {
    /// Returns the stable outcome token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MaxTicks => "max_ticks",
            Self::StopRequested => "stop_requested",
            Self::ProcessStopped => "process_stopped",
            Self::NoWorkIdle => "no_work_idle",
            Self::Blocked => "blocked",
        }
    }
}

impl fmt::Display for RuntimeDaemonLoopExitReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Caller controls for a bounded daemon loop.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RuntimeDaemonLoopOptions {
    /// Stop after this many completed supervisor cycles.
    pub max_ticks: Option<u64>,
    /// Stop after a clean idle cycle with no active workers or active runs.
    pub exit_on_idle: bool,
    /// Optional idle sleep override; defaults to startup config.
    pub idle_sleep_seconds: Option<f64>,
    /// Deterministic tick options passed into each supervisor cycle.
    pub tick_options: RuntimeTickOptions,
}

impl RuntimeDaemonLoopOptions {
    /// Build options that stop after a positive number of completed ticks.
    #[must_use]
    pub fn max_ticks(max_ticks: u64) -> Self {
        Self {
            max_ticks: Some(max_ticks),
            ..Self::default()
        }
    }

    /// Build options that stop after one clean no-work idle cycle.
    #[must_use]
    pub fn exit_on_idle() -> Self {
        Self {
            exit_on_idle: true,
            ..Self::default()
        }
    }
}

/// Summary produced by a bounded daemon loop.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeDaemonLoopOutcome {
    /// Number of completed supervisor cycles.
    pub completed_tick_count: u64,
    /// Reason the loop stopped.
    pub exit_reason: RuntimeDaemonLoopExitReason,
    /// Per-cycle outcomes in execution order.
    pub cycle_outcomes: Vec<RuntimeDaemonCycleOutcome>,
    /// Worker completions drained immediately after each cycle.
    pub post_cycle_completions: Vec<StageCompletionOutcome>,
    /// Worker completions drained during bounded shutdown.
    pub shutdown_completions: Vec<StageCompletionOutcome>,
    /// Number of idle waits performed between completed cycles.
    pub idle_sleep_count: u64,
    /// Whether the matching runtime ownership lock was released by shutdown.
    pub runtime_ownership_released: bool,
    /// Snapshot after loop shutdown and session close.
    pub final_snapshot: RuntimeSnapshot,
}

/// Test-controllable idle wait hook for daemon loops.
pub trait RuntimeDaemonSleeper {
    /// Sleep or wait for the configured idle interval.
    fn sleep(&mut self, seconds: f64) -> RuntimeTickResult<()>;
}

/// Sleeper that delegates to `std::thread::sleep`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ThreadRuntimeDaemonSleeper;

impl RuntimeDaemonSleeper for ThreadRuntimeDaemonSleeper {
    fn sleep(&mut self, seconds: f64) -> RuntimeTickResult<()> {
        thread::sleep(Duration::from_secs_f64(seconds));
        Ok(())
    }
}

/// Daemon supervisor that owns worker dispatch and serialized completion application.
#[derive(Debug, Clone)]
pub struct RuntimeDaemonSupervisor<R> {
    runner: R,
    completed_workers: HashMap<Plane, StageWorkerOutcome>,
}

impl<R> RuntimeDaemonSupervisor<R>
where
    R: StageRunnerAdapter,
{
    /// Build a supervisor using the provided stage runner.
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            completed_workers: HashMap::new(),
        }
    }

    /// Planes with completed worker payloads waiting for owner-side application.
    #[must_use]
    pub fn active_worker_planes(&self) -> Vec<Plane> {
        DISPATCH_ORDER
            .into_iter()
            .filter(|plane| self.completed_workers.contains_key(plane))
            .collect()
    }

    /// Returns true when worker outcomes still need owner-side application.
    #[must_use]
    pub fn has_pending_workers(&self) -> bool {
        !self.completed_workers.is_empty()
    }

    /// Run one supervisor cycle: drain completions, prepare inputs, then dispatch eligible work.
    pub fn run_cycle(
        &mut self,
        session: &mut RuntimeStartupSession,
        options: RuntimeTickOptions,
    ) -> RuntimeTickResult<RuntimeDaemonCycleOutcome> {
        let completions = self.drain_completed(session)?;
        let now = tick_timestamp(&options, "updated_at")?;

        ingest_runtime_cycle_inputs(session, &now)?;
        if runtime_stop_requested(session) {
            return Ok(RuntimeDaemonCycleOutcome {
                kind: RuntimeTickOutcomeKind::Stopped,
                reason: "stop_requested".to_owned(),
                completions,
                dispatched_count: 0,
                snapshot: session.snapshot.clone(),
                event_log_path: None,
            });
        }
        evaluate_and_apply_usage_governance(session, &now, None)?;
        if runtime_dispatch_paused(session) {
            let event_log_path = record_runtime_paused_cycle(session, &now)?;
            return Ok(RuntimeDaemonCycleOutcome {
                kind: RuntimeTickOutcomeKind::Paused,
                reason: "paused".to_owned(),
                completions,
                dispatched_count: 0,
                snapshot: session.snapshot.clone(),
                event_log_path: Some(event_log_path),
            });
        }
        if runtime_reconciliation_blocks_dispatch(session) {
            let event_log_path = record_runtime_blocked_cycle(session, "stale_active_state", &now)?;
            return Ok(RuntimeDaemonCycleOutcome {
                kind: RuntimeTickOutcomeKind::Blocked,
                reason: "stale_active_state".to_owned(),
                completions,
                dispatched_count: 0,
                snapshot: session.snapshot.clone(),
                event_log_path: Some(event_log_path),
            });
        }

        let dispatched_count = self.dispatch_ready_work_at(session, &options, &now, false)?;
        if completions.is_empty()
            && dispatched_count == 0
            && self.completed_workers.is_empty()
            && session.snapshot.active_runs_by_plane.is_empty()
        {
            if attempt_stranded_dependency_auto_recovery(
                &session.paths,
                &session.config.auto_recovery,
                &mut session.snapshot,
                &now,
            )?
            .is_some()
            {
                return Ok(RuntimeDaemonCycleOutcome {
                    kind: RuntimeTickOutcomeKind::Recovered,
                    reason: "blocked_dependency_auto_requeued".to_owned(),
                    completions,
                    dispatched_count,
                    snapshot: session.snapshot.clone(),
                    event_log_path: Some(session.paths.logs_dir.join("runtime_events.jsonl")),
                });
            }
            let event_log_path = record_runtime_idle_cycle(session, &now)?;
            return Ok(RuntimeDaemonCycleOutcome {
                kind: RuntimeTickOutcomeKind::NoWork,
                reason: "no_work".to_owned(),
                completions,
                dispatched_count,
                snapshot: session.snapshot.clone(),
                event_log_path: Some(event_log_path),
            });
        }

        Ok(RuntimeDaemonCycleOutcome {
            kind: if dispatched_count > 0 {
                RuntimeTickOutcomeKind::StageRequestReady
            } else {
                RuntimeTickOutcomeKind::StageDispatched
            },
            reason: if dispatched_count > 0 {
                "stage_started".to_owned()
            } else {
                "completions_drained".to_owned()
            },
            completions,
            dispatched_count,
            snapshot: session.snapshot.clone(),
            event_log_path: None,
        })
    }

    /// Dispatch all currently eligible active and newly claimed lanes.
    pub fn dispatch_ready_work(
        &mut self,
        session: &mut RuntimeStartupSession,
        options: RuntimeTickOptions,
    ) -> RuntimeTickResult<usize> {
        let now = tick_timestamp(&options, "updated_at")?;
        self.dispatch_ready_work_at(session, &options, &now, true)
    }

    fn dispatch_ready_work_at(
        &mut self,
        session: &mut RuntimeStartupSession,
        options: &RuntimeTickOptions,
        now: &Timestamp,
        process_completed: bool,
    ) -> RuntimeTickResult<usize> {
        if process_completed {
            self.drain_completed(session)?;
            ingest_runtime_cycle_inputs(session, now)?;
            evaluate_and_apply_usage_governance(session, now, None)?;
        }

        if runtime_stop_requested(session)
            || runtime_dispatch_paused(session)
            || runtime_reconciliation_blocks_dispatch(session)
        {
            return Ok(0);
        }

        let mut dispatched = 0;
        for active_run in active_runs_in_dispatch_order(&session.snapshot) {
            if !self.completed_workers.contains_key(&active_run.plane)
                && self.start_worker(session, active_run.plane, options, now)?
            {
                dispatched += 1;
            }
        }

        if runtime_stop_requested(session)
            || runtime_dispatch_paused(session)
            || runtime_reconciliation_blocks_dispatch(session)
        {
            return Ok(dispatched);
        }

        let foreground_dispatched = self.dispatch_foreground_lane(session, options, now)?;
        dispatched += foreground_dispatched;
        dispatched += self.dispatch_claim_for_plane(session, Plane::Learning, options, now)?;
        Ok(dispatched)
    }

    /// Apply all completed worker payloads through the owner-side serial result path.
    pub fn drain_completed(
        &mut self,
        session: &mut RuntimeStartupSession,
    ) -> RuntimeTickResult<Vec<StageCompletionOutcome>> {
        let mut completions = Vec::new();
        for plane in DISPATCH_ORDER {
            let Some(outcome) = self.completed_workers.remove(&plane) else {
                continue;
            };
            completions.push(apply_stage_worker_outcome(session, outcome)?);
        }
        Ok(completions)
    }

    /// Drain all worker outcomes during shutdown.
    pub fn drain_for_shutdown(
        &mut self,
        session: &mut RuntimeStartupSession,
    ) -> RuntimeTickResult<Vec<StageCompletionOutcome>> {
        self.drain_completed(session)
    }

    fn dispatch_foreground_lane(
        &mut self,
        session: &mut RuntimeStartupSession,
        options: &RuntimeTickOptions,
        now: &Timestamp,
    ) -> RuntimeTickResult<usize> {
        for plane in FOREGROUND_PLANES {
            let dispatched = self.dispatch_claim_for_plane(session, plane, options, now)?;
            if dispatched > 0 {
                return Ok(dispatched);
            }
        }
        if self.completed_workers.is_empty()
            && can_dispatch_plane(
                session.compiled_plan.concurrency_policy.as_ref(),
                active_planes(&session.snapshot),
                Plane::Planning,
            )
            && can_dispatch_lane(
                session.compiled_plan.lane_policy.as_ref(),
                session.compiled_plan.concurrency_policy.as_ref(),
                active_lane_ids(&session.snapshot),
                &lane_id_for_plane(Some(&session.compiled_plan), Plane::Planning),
            )
            && activate_completion_stage_if_ready(session, options, now)?
        {
            return self
                .start_worker(session, Plane::Planning, options, now)
                .map(|started| if started { 1 } else { 0 });
        }
        Ok(0)
    }

    fn dispatch_claim_for_plane(
        &mut self,
        session: &mut RuntimeStartupSession,
        plane: Plane,
        options: &RuntimeTickOptions,
        now: &Timestamp,
    ) -> RuntimeTickResult<usize> {
        if !can_dispatch_plane(
            session.compiled_plan.concurrency_policy.as_ref(),
            active_planes(&session.snapshot),
            plane,
        ) {
            return Ok(0);
        }
        let candidate_lane_id = lane_id_for_plane(Some(&session.compiled_plan), plane);
        if !can_dispatch_lane(
            session.compiled_plan.lane_policy.as_ref(),
            session.compiled_plan.concurrency_policy.as_ref(),
            active_lane_ids(&session.snapshot),
            &candidate_lane_id,
        ) {
            return Ok(0);
        }
        if !activate_next_claim_for_plane(session, plane, options, now)? {
            return Ok(0);
        }
        self.start_worker(session, plane, options, now)
            .map(|started| if started { 1 } else { 0 })
    }

    fn start_worker(
        &mut self,
        session: &mut RuntimeStartupSession,
        plane: Plane,
        options: &RuntimeTickOptions,
        now: &Timestamp,
    ) -> RuntimeTickResult<bool> {
        if self.completed_workers.contains_key(&plane) {
            return Ok(false);
        }
        if tick::guard_stage_work_item_ownership_for_plane(session, plane, now)?.is_some() {
            return Ok(false);
        }
        let request = build_stage_run_request_for_plane(session, plane, options, now)?;
        let gate_result = evaluate_stage_request_capabilities_with_runner(
            &session.paths,
            &request,
            &self.runner,
            now,
        )?;
        let request = gate_result.request.clone();
        record_capability_gate_result(&session.paths, &request, &gate_result, now)?;
        let active_run = tick::active_run_for_plane(&session.snapshot, plane)
            .ok_or_else(|| worker_mismatch("worker start requires active run for plane"))?;
        if !gate_result.allowed {
            let raw_result = capability_gate_failure_result(&request, &gate_result, now)?;
            let outcome = gate_blocked_worker_outcome(active_run, request, raw_result, now)?;
            outcome.validate()?;
            self.completed_workers.insert(plane, outcome);
            return Ok(true);
        }

        mark_stage_running_and_emit_started(session, &request, now)?;
        let active_run = tick::active_run_for_plane(&session.snapshot, plane)
            .ok_or_else(|| worker_mismatch("worker start requires active run for plane"))?;
        let outcome = run_stage_worker(active_run, request, &self.runner)?;
        outcome.validate()?;
        self.completed_workers.insert(plane, outcome);
        Ok(true)
    }
}

/// Run a bounded daemon loop using a real thread sleeper.
pub fn run_runtime_daemon_loop<R>(
    session: RuntimeStartupSession,
    runner: R,
    options: RuntimeDaemonLoopOptions,
) -> RuntimeTickResult<RuntimeDaemonLoopOutcome>
where
    R: StageRunnerAdapter,
{
    let mut session = session;
    let mut supervisor = RuntimeDaemonSupervisor::new(runner);
    let mut sleeper = ThreadRuntimeDaemonSleeper;
    run_runtime_daemon_supervisor_loop_with_sleeper(
        &mut session,
        &mut supervisor,
        options,
        &mut sleeper,
    )
}

/// Run a bounded daemon loop and stream persisted owner-side events to a monitor sink.
pub fn run_runtime_daemon_loop_with_monitor<R, M>(
    session: RuntimeStartupSession,
    runner: R,
    options: RuntimeDaemonLoopOptions,
    monitor: &mut M,
) -> RuntimeTickResult<RuntimeDaemonLoopOutcome>
where
    R: StageRunnerAdapter,
    M: RuntimeMonitorSink,
{
    let mut session = session;
    let mut supervisor = RuntimeDaemonSupervisor::new(runner);
    let mut sleeper = ThreadRuntimeDaemonSleeper;
    run_runtime_daemon_supervisor_loop_with_sleeper_and_monitor(
        &mut session,
        &mut supervisor,
        options,
        &mut sleeper,
        monitor,
    )
}

/// Run a bounded daemon loop around an existing supervisor with a test-controllable sleeper.
pub fn run_runtime_daemon_supervisor_loop_with_sleeper<R, S>(
    session: &mut RuntimeStartupSession,
    supervisor: &mut RuntimeDaemonSupervisor<R>,
    options: RuntimeDaemonLoopOptions,
    sleeper: &mut S,
) -> RuntimeTickResult<RuntimeDaemonLoopOutcome>
where
    R: StageRunnerAdapter,
    S: RuntimeDaemonSleeper,
{
    let body_result = run_daemon_loop_body(session, supervisor, options, sleeper, None);
    finish_daemon_loop(session, body_result)
}

/// Run a bounded daemon loop with a monitor sink and a test-controllable sleeper.
pub fn run_runtime_daemon_supervisor_loop_with_sleeper_and_monitor<R, S, M>(
    session: &mut RuntimeStartupSession,
    supervisor: &mut RuntimeDaemonSupervisor<R>,
    options: RuntimeDaemonLoopOptions,
    sleeper: &mut S,
    monitor: &mut M,
) -> RuntimeTickResult<RuntimeDaemonLoopOutcome>
where
    R: StageRunnerAdapter,
    S: RuntimeDaemonSleeper,
    M: RuntimeMonitorSink,
{
    let body_result = run_daemon_loop_body(
        session,
        supervisor,
        options,
        sleeper,
        Some(monitor as &mut dyn RuntimeMonitorSink),
    );
    finish_daemon_loop(session, body_result)
}

fn finish_daemon_loop(
    session: &mut RuntimeStartupSession,
    body_result: RuntimeTickResult<RuntimeDaemonLoopOutcome>,
) -> RuntimeTickResult<RuntimeDaemonLoopOutcome> {
    let close_result = session.close().map_err(RuntimeTickError::from);
    match (body_result, close_result) {
        (Ok(mut outcome), Ok(released)) => {
            outcome.runtime_ownership_released = released;
            outcome.final_snapshot = session.snapshot.clone();
            Ok(outcome)
        }
        (Err(error), Ok(_)) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(_close_error)) => Err(error),
    }
}

fn run_daemon_loop_body<R, S>(
    session: &mut RuntimeStartupSession,
    supervisor: &mut RuntimeDaemonSupervisor<R>,
    options: RuntimeDaemonLoopOptions,
    sleeper: &mut S,
    mut monitor: Option<&mut dyn RuntimeMonitorSink>,
) -> RuntimeTickResult<RuntimeDaemonLoopOutcome>
where
    R: StageRunnerAdapter,
    S: RuntimeDaemonSleeper,
{
    if options.max_ticks == Some(0) {
        return Err(loop_invalid_state(
            "daemon loop max_ticks must be greater than or equal to 1",
        ));
    }
    let idle_sleep_seconds = idle_sleep_seconds(session, &options)?;
    let mut outcome = RuntimeDaemonLoopOutcome {
        completed_tick_count: 0,
        exit_reason: RuntimeDaemonLoopExitReason::MaxTicks,
        cycle_outcomes: Vec::new(),
        post_cycle_completions: Vec::new(),
        shutdown_completions: Vec::new(),
        idle_sleep_count: 0,
        runtime_ownership_released: false,
        final_snapshot: session.snapshot.clone(),
    };
    let mut monitor_cursor = if monitor.is_some() {
        Some(RuntimeMonitorLogCursor::new(session)?)
    } else {
        None
    };

    let exit_reason = loop {
        if options
            .max_ticks
            .is_some_and(|max_ticks| outcome.completed_tick_count >= max_ticks)
        {
            break RuntimeDaemonLoopExitReason::MaxTicks;
        }

        let cycle = supervisor.run_cycle(session, options.tick_options.clone())?;
        let cycle_kind = cycle.kind;
        let cycle_dispatched_count = cycle.dispatched_count;
        let cycle_completion_count = cycle.completions.len();
        emit_pending_runtime_monitor_events(&mut monitor_cursor, &mut monitor, session)?;
        outcome.cycle_outcomes.push(cycle);

        let post_cycle_completions = supervisor.drain_completed(session)?;
        emit_pending_runtime_monitor_events(&mut monitor_cursor, &mut monitor, session)?;
        let post_cycle_completion_count = post_cycle_completions.len();
        outcome
            .post_cycle_completions
            .extend(post_cycle_completions);
        outcome.completed_tick_count += 1;

        if session.snapshot.stop_requested && !supervisor.has_pending_workers() {
            if session.snapshot.active_runs_by_plane.is_empty() {
                let now = tick_timestamp(&options.tick_options, "updated_at")?;
                record_runtime_stopped_cycle(session, &now)?;
                emit_pending_runtime_monitor_events(&mut monitor_cursor, &mut monitor, session)?;
            }
            break RuntimeDaemonLoopExitReason::StopRequested;
        }
        if !session.snapshot.process_running && !supervisor.has_pending_workers() {
            break RuntimeDaemonLoopExitReason::ProcessStopped;
        }
        if cycle_kind == RuntimeTickOutcomeKind::Blocked {
            break RuntimeDaemonLoopExitReason::Blocked;
        }
        if options.exit_on_idle
            && cycle_kind == RuntimeTickOutcomeKind::NoWork
            && cycle_completion_count == 0
            && post_cycle_completion_count == 0
            && cycle_dispatched_count == 0
            && !supervisor.has_pending_workers()
            && session.snapshot.active_runs_by_plane.is_empty()
        {
            break RuntimeDaemonLoopExitReason::NoWorkIdle;
        }
        if options
            .max_ticks
            .is_some_and(|max_ticks| outcome.completed_tick_count >= max_ticks)
        {
            break RuntimeDaemonLoopExitReason::MaxTicks;
        }

        if should_idle_wait_after_cycle(
            cycle_kind,
            cycle_completion_count + post_cycle_completion_count,
            cycle_dispatched_count,
            supervisor.has_pending_workers(),
        ) {
            sleeper.sleep(idle_sleep_seconds)?;
            outcome.idle_sleep_count += 1;
        }
    };

    if options.max_ticks.is_some() {
        let shutdown_completions = supervisor.drain_for_shutdown(session)?;
        emit_pending_runtime_monitor_events(&mut monitor_cursor, &mut monitor, session)?;
        outcome.shutdown_completions.extend(shutdown_completions);
    }

    outcome.exit_reason = exit_reason;
    outcome.final_snapshot = session.snapshot.clone();
    Ok(outcome)
}

struct RuntimeMonitorLogCursor {
    event_log_path: PathBuf,
    emitted_line_count: usize,
}

impl RuntimeMonitorLogCursor {
    fn new(session: &RuntimeStartupSession) -> RuntimeTickResult<Self> {
        let event_log_path = session.paths.logs_dir.join("runtime_events.jsonl");
        let emitted_line_count = read_runtime_event_lines(&event_log_path)?.len();
        Ok(Self {
            event_log_path,
            emitted_line_count,
        })
    }

    fn emit_new(&mut self, monitor: &mut dyn RuntimeMonitorSink) -> RuntimeTickResult<()> {
        let lines = read_runtime_event_lines(&self.event_log_path)?;
        if lines.len() < self.emitted_line_count {
            self.emitted_line_count = 0;
        }
        let new_lines = &lines[self.emitted_line_count..];
        if new_lines.is_empty() {
            return Ok(());
        }

        let raw = new_lines.join("\n");
        let events = runtime_monitor_events_from_jsonl(&raw).map_err(|message| {
            loop_invalid_state(format!("runtime monitor event parse failed: {message}"))
        })?;
        for event in &events {
            monitor.emit(event).map_err(|error| RuntimeTickError::Io {
                path: self.event_log_path.clone(),
                message: format!("runtime monitor emit failed: {error}"),
            })?;
        }
        self.emitted_line_count = lines.len();
        Ok(())
    }
}

fn emit_pending_runtime_monitor_events(
    cursor: &mut Option<RuntimeMonitorLogCursor>,
    monitor: &mut Option<&mut dyn RuntimeMonitorSink>,
    _session: &RuntimeStartupSession,
) -> RuntimeTickResult<()> {
    let (Some(cursor), Some(monitor)) = (cursor.as_mut(), monitor.as_deref_mut()) else {
        return Ok(());
    };
    cursor.emit_new(monitor)
}

fn read_runtime_event_lines(path: &PathBuf) -> RuntimeTickResult<Vec<String>> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(RuntimeTickError::Io {
                path: path.clone(),
                message: error.to_string(),
            });
        }
    };
    Ok(raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect())
}

fn idle_sleep_seconds(
    session: &RuntimeStartupSession,
    options: &RuntimeDaemonLoopOptions,
) -> RuntimeTickResult<f64> {
    let seconds = options
        .idle_sleep_seconds
        .unwrap_or(session.config.idle_sleep_seconds);
    if seconds.is_finite() && seconds > 0.0 {
        Ok(seconds)
    } else {
        Err(loop_invalid_state(
            "daemon loop idle_sleep_seconds must be a positive finite number",
        ))
    }
}

fn should_idle_wait_after_cycle(
    kind: RuntimeTickOutcomeKind,
    completion_count: usize,
    dispatched_count: usize,
    has_pending_workers: bool,
) -> bool {
    completion_count == 0
        && dispatched_count == 0
        && !has_pending_workers
        && matches!(
            kind,
            RuntimeTickOutcomeKind::NoWork | RuntimeTickOutcomeKind::Paused
        )
}

/// Return whether `candidate` may start beside the currently active planes.
#[must_use]
pub fn can_dispatch_plane<I>(
    policy: Option<&PlaneConcurrencyPolicyDefinition>,
    active_planes: I,
    candidate: Plane,
) -> bool
where
    I: IntoIterator<Item = Plane>,
{
    let mut active_plane_set = Vec::new();
    for plane in active_planes {
        if !active_plane_set.contains(&plane) {
            active_plane_set.push(plane);
        }
    }
    if active_plane_set.contains(&candidate) {
        return false;
    }
    if active_plane_set.is_empty() {
        return true;
    }
    let Some(policy) = policy else {
        return false;
    };

    for active_plane in active_plane_set {
        if pair_in_groups(candidate, active_plane, &policy.mutually_exclusive_planes) {
            return false;
        }
        if !pair_in_groups(candidate, active_plane, &policy.may_run_concurrently) {
            return false;
        }
    }
    true
}

/// Run one isolated stage worker and return a typed payload for owner-side application.
pub fn run_stage_worker(
    active_run: ActiveRunState,
    request: StageRunRequest,
    runner: &impl StageRunnerAdapter,
) -> RuntimeTickResult<StageWorkerOutcome> {
    let started_at = worker_timestamp("started_at")?;
    let result = match runner.run(&request) {
        Ok(raw_result) => StageWorkerResult::RawResult(Box::new(raw_result)),
        Err(error) => StageWorkerResult::Exception {
            exception_type: "RunnerError".to_owned(),
            exception_message: error.to_string(),
            runner_name: runner_name_from_error(&error),
        },
    };
    let completed_at = worker_timestamp("completed_at")?;
    Ok(StageWorkerOutcome {
        plane: active_run.plane,
        run_id: active_run.run_id.clone(),
        active_run,
        request,
        started_at,
        completed_at,
        result,
    })
}

fn gate_blocked_worker_outcome(
    active_run: ActiveRunState,
    request: StageRunRequest,
    raw_result: RunnerRawResult,
    now: &Timestamp,
) -> RuntimeTickResult<StageWorkerOutcome> {
    Ok(StageWorkerOutcome {
        plane: active_run.plane,
        run_id: active_run.run_id.clone(),
        active_run,
        request,
        started_at: now.clone(),
        completed_at: now.clone(),
        result: StageWorkerResult::RawResult(Box::new(raw_result)),
    })
}

/// Apply a completed worker through existing serial result-application helpers.
pub fn apply_stage_worker_outcome(
    session: &mut RuntimeStartupSession,
    outcome: StageWorkerOutcome,
) -> RuntimeTickResult<StageCompletionOutcome> {
    outcome.validate()?;
    let current_active_run = tick::active_run_for_plane(&session.snapshot, outcome.plane)
        .ok_or_else(|| worker_mismatch("stage worker completion has no active run for plane"))?;
    if current_active_run != outcome.active_run {
        return Err(worker_mismatch(
            "stage worker completion no longer matches active run state",
        ));
    }

    let raw_result = match outcome.result {
        StageWorkerResult::RawResult(raw_result) => *raw_result,
        StageWorkerResult::Exception {
            exception_type,
            exception_message,
            runner_name,
        } => tick::runner_exception_raw_result(
            &outcome.request,
            &outcome.started_at,
            &outcome.completed_at,
            &exception_type,
            &exception_message,
            runner_name.as_deref(),
        )?,
    };
    let dispatch_outcome =
        apply_stage_worker_raw_result(session, outcome.request.clone(), raw_result)?;

    Ok(StageCompletionOutcome {
        plane: outcome.plane,
        run_id: outcome.run_id,
        request: outcome.request,
        stage_result: dispatch_outcome.stage_result.clone(),
        stage_result_path: dispatch_outcome.stage_result_path.clone(),
        router_decision: dispatch_outcome.router_decision.clone(),
        dispatch_outcome,
    })
}

fn request_kind_matches(request_kind: RequestKind, active_kind: ActiveRunRequestKind) -> bool {
    matches!(
        (request_kind, active_kind),
        (
            RequestKind::ActiveWorkItem,
            ActiveRunRequestKind::ActiveWorkItem
        ) | (
            RequestKind::ClosureTarget,
            ActiveRunRequestKind::ClosureTarget
        ) | (
            RequestKind::LearningRequest,
            ActiveRunRequestKind::LearningRequest
        )
    )
}

fn active_runs_in_dispatch_order(snapshot: &RuntimeSnapshot) -> Vec<ActiveRunState> {
    DISPATCH_ORDER
        .into_iter()
        .filter_map(|plane| tick::active_run_for_plane(snapshot, plane))
        .collect()
}

fn active_planes(snapshot: &RuntimeSnapshot) -> Vec<Plane> {
    DISPATCH_ORDER
        .into_iter()
        .filter(|plane| snapshot.active_runs_by_plane.contains_key(plane))
        .collect()
}

fn active_lane_ids(snapshot: &RuntimeSnapshot) -> Vec<String> {
    DISPATCH_ORDER
        .into_iter()
        .filter_map(|plane| tick::active_run_for_plane(snapshot, plane))
        .map(|active_run| active_run.lane_id)
        .collect()
}

fn pair_in_groups(candidate: Plane, active_plane: Plane, groups: &[Vec<Plane>]) -> bool {
    groups.iter().any(|group| {
        group.contains(&candidate)
            && group.contains(&active_plane)
            && (candidate == active_plane || group.len() >= 2)
    })
}

fn tick_timestamp(
    options: &RuntimeTickOptions,
    field_name: &'static str,
) -> RuntimeTickResult<Timestamp> {
    options
        .now
        .clone()
        .map(Ok)
        .unwrap_or_else(|| worker_timestamp(field_name))
}

fn worker_timestamp(field_name: &'static str) -> RuntimeTickResult<Timestamp> {
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

fn worker_mismatch(message: impl fmt::Display) -> RuntimeTickError {
    RuntimeTickError::InvalidState {
        message: format!("stage_worker_metadata_mismatch: {message}"),
    }
}

fn loop_invalid_state(message: impl Into<String>) -> RuntimeTickError {
    RuntimeTickError::InvalidState {
        message: message.into(),
    }
}

fn runner_name_from_error(error: &RunnerError) -> Option<String> {
    match error {
        RunnerError::UnknownRunner { requested, .. } => Some(requested.clone()),
        RunnerError::RunnerBinaryNotFound { binary } => Some(binary.clone()),
        _ => None,
    }
}
