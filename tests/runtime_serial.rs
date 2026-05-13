use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tempfile::TempDir;

use millrace_ai::contracts::{
    ActiveRunRequestKind, ActiveRunState, ClosureTargetState, ExecutionTerminalResult,
    IncidentDecision, IncidentDocument, IncidentSeverity, LearningRequestAction,
    LearningRequestDocument, LearningStageName, LearningTerminalResult, Plane,
    PlanningTerminalResult, ProbeDocument, ReconConfidence, ReconDecision, ReconPacketDocument,
    ReconPathFinding, ReconRiskLevel, ReconVerificationPlan, RecoveryCounterEntry,
    RecoveryCounters, ResultClass, RootIntakeKind, RunTraceGraph, RuntimeErrorContext,
    RuntimeJsonContract, RuntimeMode, SpecDocument, SpecSourceType, StageName, StageResultEnvelope,
    SubscriptionQuotaWindowReading, TaskDocument, TerminalResult, Timestamp, TokenUsage,
    UsageGovernanceDegradedPolicy, UsageGovernanceRuntimeTokenMetric,
    UsageGovernanceRuntimeTokenWindow, UsageGovernanceSubscriptionWindow, WorkItemKind,
};
use millrace_ai::recon_packets::render_recon_packet;
use millrace_ai::work_documents::{
    parse_incident_document, parse_learning_request_document, parse_spec_document,
    parse_task_document,
};
use millrace_ai::workspace::{
    QueueStore, RuntimeOwnershipLockOptions, RuntimeOwnershipLockState,
    acquire_runtime_ownership_lock_with_options, initialize_workspace,
    inspect_runtime_ownership_lock, load_execution_status, load_learning_status,
    load_planning_status, load_recovery_counters, load_snapshot, load_usage_governance_ledger,
    load_usage_governance_state, save_closure_target_state, save_recovery_counters, save_snapshot,
};
use millrace_ai::{
    FakeRunner, FakeRunnerConfig, FakeRunnerOutput, FakeRunnerResult, ProcessExecutionResult,
    ProcessExitKind, RequestKind, RouterAction, RouterDecision, RunnerCompletionArtifact,
    RunnerCompletionArtifactContext, RunnerEnvironmentDelta, RunnerError, RunnerExitKind,
    RunnerInvocationArtifact, RunnerRawResult, RunnerRegistry, RunnerResult, RuntimeStartupError,
    RuntimeStartupOptions, RuntimeTickOptions, RuntimeTickOutcomeKind, RuntimeTokenRuleConfig,
    RuntimeTokenRulesConfig, StageRunRequest, StageRunnerAdapter, StageRunnerDispatcher,
    SubscriptionQuotaRulesConfig, UsageGovernanceConfig, blocked_metadata_allows_auto_requeue,
    blocked_task_metadata_path, build_runtime_runner_dispatcher, build_stage_prompt,
    completion_artifact_from_raw_result, evaluate_runtime_token_rules,
    evaluate_subscription_quota_rules, evaluate_usage_governance, find_stranded_blocked_dependency,
    healthy_subscription_quota_status, inspect_run_trace, invocation_artifact_from_request,
    load_blocked_task_metadata, normalize_stage_result, reconcile_usage_ledger_from_stage_results,
    record_router_decision_trace, record_stage_result_usage, render_stage_request_context_lines,
    run_serial_runtime_tick, run_serial_runtime_tick_with_runner, runner_prompt_path,
    spawned_work_ref_from_path, startup_runtime_once, startup_runtime_once_for_paths,
    subscription_quota_status_unavailable, trace_path_for_run_dir, upsert_stage_result_trace_node,
    write_runner_completion, write_runner_invocation, write_stage_prompt_artifact,
};

const RUN_ID: &str = "run-001";
const REQUEST_ID: &str = "request-001";
const STARTUP_NOW: &str = "2026-04-28T20:00:00Z";

fn sample_request(run_dir: &Path) -> StageRunRequest {
    let run_dir = run_dir.display().to_string();
    let mut request = StageRunRequest {
        request_id: REQUEST_ID.to_owned(),
        run_id: RUN_ID.to_owned(),
        plane: Plane::Execution,
        stage: StageName::Builder,
        request_kind: RequestKind::ActiveWorkItem,
        mode_id: "learning_codex".to_owned(),
        compiled_plan_id: "plan-001".to_owned(),
        node_id: "builder-node".to_owned(),
        stage_kind_id: "builder".to_owned(),
        running_status_marker: "BUILDER_RUNNING".to_owned(),
        legal_terminal_markers: Vec::new(),
        allowed_result_classes_by_outcome: Default::default(),
        entrypoint_path: "millrace-agents/entrypoints/execution/builder.md".to_owned(),
        entrypoint_contract_id: Some("builder.contract.v1".to_owned()),
        required_skill_paths: vec![
            "millrace-agents/skills/stage/execution/builder-core/SKILL.md".to_owned(),
        ],
        attached_skill_paths: Vec::new(),
        active_work_item_kind: Some(WorkItemKind::Task),
        active_work_item_id: Some("task-001".to_owned()),
        active_work_item_path: Some("millrace-agents/tasks/active/task-001.md".to_owned()),
        closure_target_path: None,
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        canonical_root_spec_path: None,
        canonical_seed_idea_path: None,
        preferred_rubric_path: None,
        preferred_verdict_path: None,
        preferred_report_path: None,
        run_dir: run_dir.clone(),
        summary_status_path: "millrace-agents/state/execution_status.md".to_owned(),
        runtime_snapshot_path: "millrace-agents/state/runtime_snapshot.json".to_owned(),
        recovery_counters_path: "millrace-agents/state/recovery_counters.json".to_owned(),
        preferred_troubleshoot_report_path: Some(format!("{run_dir}/troubleshoot_report.md")),
        runtime_error_code: None,
        runtime_error_report_path: None,
        runtime_error_catalog_path: None,
        skill_revision_evidence_path: Some(format!("{run_dir}/skill_revision.json")),
        runner_name: Some("fake_runner".to_owned()),
        model_name: Some("fake-model".to_owned()),
        thinking_level: Some("medium".to_owned()),
        model_reasoning_effort: Some("medium".to_owned()),
        timeout_seconds: 3600,
    };
    request.validate().unwrap();
    request
}

fn sample_learning_request(run_dir: &Path, stage: StageName) -> StageRunRequest {
    let mut request = sample_request(run_dir);
    request.plane = Plane::Learning;
    request.stage = stage;
    request.request_kind = RequestKind::LearningRequest;
    request.node_id = stage.as_str().to_owned();
    request.stage_kind_id = stage.as_str().to_owned();
    request.running_status_marker = format!("{}_RUNNING", stage.as_str().to_ascii_uppercase());
    request.legal_terminal_markers = Vec::new();
    request.allowed_result_classes_by_outcome = Default::default();
    request.entrypoint_path = format!("millrace-agents/entrypoints/learning/{}.md", stage.as_str());
    request.entrypoint_contract_id = Some(format!("{}.contract.v1", stage.as_str()));
    request.required_skill_paths = vec![format!(
        "millrace-agents/skills/stage/learning/{}-core/SKILL.md",
        stage.as_str()
    )];
    request.active_work_item_kind = Some(WorkItemKind::LearningRequest);
    request.active_work_item_id = Some("learn-001".to_owned());
    request.active_work_item_path =
        Some("millrace-agents/learning/requests/active/learn-001.md".to_owned());
    request.summary_status_path = "millrace-agents/state/learning_status.md".to_owned();
    request.validate().unwrap();
    request
}

fn sample_stage_result_with_tokens(
    token_usage: TokenUsage,
    completed_at: &str,
) -> StageResultEnvelope {
    StageResultEnvelope {
        schema_version: "1.0".to_owned(),
        kind: "stage_result".to_owned(),
        run_id: RUN_ID.to_owned(),
        plane: Plane::Execution,
        stage: StageName::Builder,
        node_id: "builder".to_owned(),
        stage_kind_id: "builder".to_owned(),
        work_item_kind: WorkItemKind::Task,
        work_item_id: "task-001".to_owned(),
        terminal_result: TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete),
        result_class: ResultClass::Success,
        summary_status_marker: "### BUILDER_COMPLETE".to_owned(),
        success: true,
        retryable: false,
        exit_code: 0,
        duration_seconds: 1.0,
        prompt_artifact: None,
        report_artifact: None,
        artifact_paths: Vec::new(),
        detected_marker: Some("### BUILDER_COMPLETE".to_owned()),
        stdout_path: None,
        stderr_path: None,
        runner_name: Some("fake".to_owned()),
        model_name: None,
        thinking_level: None,
        model_reasoning_effort: None,
        token_usage: Some(token_usage),
        notes: Vec::new(),
        metadata: serde_json::Map::new(),
        started_at: timestamp(completed_at),
        completed_at: timestamp(completed_at),
    }
}

fn failure_class(value: &Value) -> Option<&str> {
    value
        .get("failure_class")
        .and_then(serde_json::Value::as_str)
}

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("created_at", value).unwrap()
}

#[test]
fn usage_governance_is_inert_by_default_for_contract_evaluation() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let stage_result = sample_stage_result_with_tokens(
        TokenUsage {
            input_tokens: 100,
            cached_input_tokens: 20,
            output_tokens: 30,
            thinking_tokens: 5,
            total_tokens: 135,
        },
        STARTUP_NOW,
    );

    let state = evaluate_usage_governance(
        &paths,
        &UsageGovernanceConfig::default(),
        timestamp(STARTUP_NOW),
        Some("daemon-session".to_owned()),
        false,
        Some((
            &stage_result,
            &paths
                .runs_dir
                .join("run-001/stage_results/request-001.json"),
        )),
        None,
    )
    .unwrap();

    assert!(!state.enabled);
    assert!(!paths.usage_governance_state_file.exists());
    assert!(!paths.usage_governance_ledger_file.exists());
}

#[test]
fn usage_governance_ledger_records_once_and_reconciles_stage_results() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let config = UsageGovernanceConfig {
        enabled: true,
        runtime_token_rules: RuntimeTokenRulesConfig {
            enabled: true,
            rules: vec![RuntimeTokenRuleConfig {
                rule_id: "test-rolling".to_owned(),
                window: UsageGovernanceRuntimeTokenWindow::Rolling5h,
                metric: UsageGovernanceRuntimeTokenMetric::TotalTokens,
                threshold: 100,
            }],
        },
        ..UsageGovernanceConfig::default()
    };
    let stage_result = sample_stage_result_with_tokens(
        TokenUsage {
            input_tokens: 50,
            cached_input_tokens: 0,
            output_tokens: 75,
            thinking_tokens: 0,
            total_tokens: 125,
        },
        STARTUP_NOW,
    );
    let stage_result_path = paths
        .runs_dir
        .join("run-001")
        .join("stage_results")
        .join("request-001.json");
    fs::create_dir_all(stage_result_path.parent().unwrap()).unwrap();
    fs::write(
        &stage_result_path,
        serde_json::to_string_pretty(&stage_result).unwrap() + "\n",
    )
    .unwrap();

    assert!(
        record_stage_result_usage(
            &paths,
            &config,
            &stage_result,
            &stage_result_path,
            timestamp(STARTUP_NOW),
            Some("daemon-session"),
        )
        .unwrap()
    );
    assert!(
        !record_stage_result_usage(
            &paths,
            &config,
            &stage_result,
            &stage_result_path,
            timestamp(STARTUP_NOW),
            Some("daemon-session"),
        )
        .unwrap()
    );
    let ledger = load_usage_governance_ledger(&paths).unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].token_usage.total_tokens, 125);

    fs::remove_file(&paths.usage_governance_ledger_file).unwrap();
    let repaired = reconcile_usage_ledger_from_stage_results(
        &paths,
        &config,
        timestamp("2026-04-28T20:01:00Z"),
        Some("daemon-session"),
    )
    .unwrap();
    assert_eq!(repaired, 1);
    let reconciled = load_usage_governance_ledger(&paths).unwrap();
    assert_eq!(reconciled.len(), 1);
    assert_eq!(
        reconciled[0].dedupe_key,
        "millrace-agents/runs/run-001/stage_results/request-001.json"
    );
}

#[test]
fn usage_governance_evaluates_token_windows_and_subscription_quota_contracts() {
    let token_config = RuntimeTokenRulesConfig {
        enabled: true,
        rules: vec![RuntimeTokenRuleConfig {
            rule_id: "test-rolling".to_owned(),
            window: UsageGovernanceRuntimeTokenWindow::Rolling5h,
            metric: UsageGovernanceRuntimeTokenMetric::TotalTokens,
            threshold: 100,
        }],
    };
    let stage_result = sample_stage_result_with_tokens(
        TokenUsage {
            input_tokens: 0,
            cached_input_tokens: 0,
            output_tokens: 0,
            thinking_tokens: 0,
            total_tokens: 125,
        },
        STARTUP_NOW,
    );
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let entry = millrace_ai::ledger_entry_from_stage_result(
        &paths,
        &stage_result,
        &paths
            .runs_dir
            .join("run-001/stage_results/request-001.json"),
        timestamp(STARTUP_NOW),
        Some("daemon-session"),
    )
    .unwrap();

    let blockers = evaluate_runtime_token_rules(
        &[entry],
        &token_config,
        &timestamp(STARTUP_NOW),
        Some("daemon-session"),
        "UTC",
    )
    .unwrap();
    assert_eq!(blockers.len(), 1);
    assert_eq!(blockers[0].rule_id, "test-rolling");
    assert_eq!(
        blockers[0].next_auto_resume_at.as_ref().unwrap().as_str(),
        "2026-04-29T01:00:00Z"
    );

    let mut quota_config = SubscriptionQuotaRulesConfig {
        enabled: true,
        degraded_policy: UsageGovernanceDegradedPolicy::FailOpen,
        ..SubscriptionQuotaRulesConfig::default()
    };
    let degraded = subscription_quota_status_unavailable(&quota_config, timestamp(STARTUP_NOW));
    assert!(evaluate_subscription_quota_rules(&degraded, &quota_config).is_empty());

    quota_config.degraded_policy = UsageGovernanceDegradedPolicy::FailClosed;
    let degraded = subscription_quota_status_unavailable(&quota_config, timestamp(STARTUP_NOW));
    let degraded_blockers = evaluate_subscription_quota_rules(&degraded, &quota_config);
    assert_eq!(
        degraded_blockers[0].rule_id,
        "subscription-quota-degraded-fail-closed"
    );
    assert!(!degraded_blockers[0].auto_resume_possible);

    let mut windows = BTreeMap::new();
    windows.insert(
        UsageGovernanceSubscriptionWindow::FiveHour,
        SubscriptionQuotaWindowReading {
            window: UsageGovernanceSubscriptionWindow::FiveHour,
            percent_used: 96.0,
            resets_at: Some(timestamp("2026-04-28T21:00:00Z")),
            read_at: timestamp(STARTUP_NOW),
        },
    );
    let healthy = healthy_subscription_quota_status(&quota_config, timestamp(STARTUP_NOW), windows);
    let quota_blockers = evaluate_subscription_quota_rules(&healthy, &quota_config);
    assert_eq!(quota_blockers[0].rule_id, "codex-five-hour-default");
    assert_eq!(
        quota_blockers[0]
            .next_auto_resume_at
            .as_ref()
            .unwrap()
            .as_str(),
        "2026-04-28T21:00:00Z"
    );
}

#[test]
fn malformed_usage_governance_state_and_ledger_fail_with_paths() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();

    fs::write(&paths.usage_governance_state_file, "{\n").unwrap();
    let state_error = load_usage_governance_state(&paths).unwrap_err();
    assert!(
        state_error
            .to_string()
            .contains("usage_governance_state.json")
    );

    fs::write(
        &paths.usage_governance_ledger_file,
        serde_json::json!({
            "dedupe_key": "a.json",
            "counted_at": STARTUP_NOW,
            "stage_completed_at": STARTUP_NOW,
            "plane": "execution",
            "run_id": "run-001",
            "stage_id": "builder",
            "work_item_kind": "task",
            "work_item_id": "task-001",
            "token_usage": {"total_tokens": 1},
            "stage_result_path": "different.json",
            "daemon_session_id": null
        })
        .to_string()
            + "\n",
    )
    .unwrap();
    let ledger_error = load_usage_governance_ledger(&paths).unwrap_err();
    assert!(
        ledger_error
            .to_string()
            .contains("usage_governance_ledger.jsonl")
    );
    assert!(ledger_error.to_string().contains("line 1"));
}

#[test]
fn serial_runtime_governance_pauses_after_token_stage_and_auto_resumes() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    write_runtime_token_governance_config(&paths, 100, true);
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-governed"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("governance-serial")).unwrap();
    let builder = TokenTerminalRunner {
        runner_name: "token-builder",
        terminal_marker: "### BUILDER_COMPLETE",
        token_usage: TokenUsage {
            input_tokens: 50,
            cached_input_tokens: 0,
            output_tokens: 75,
            thinking_tokens: 0,
            total_tokens: 125,
        },
        completed_at: STARTUP_NOW,
    };
    let first = run_serial_runtime_tick_with_runner(
        &mut session,
        RuntimeTickOptions {
            now: Some(timestamp(STARTUP_NOW)),
            run_id: Some("run-governed".to_owned()),
            request_id: Some("request-builder".to_owned()),
        },
        &builder,
    )
    .unwrap();

    assert_eq!(first.kind, RuntimeTickOutcomeKind::StageDispatched);
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.paused);
    assert_eq!(
        snapshot.pause_sources,
        vec![millrace_ai::contracts::PauseSource::UsageGovernance]
    );
    let state = load_usage_governance_state(&paths).unwrap();
    assert_eq!(state.active_blockers[0].rule_id, "test-rolling");
    assert_eq!(state.active_blockers[0].observed, 125.0);
    assert_eq!(load_usage_governance_ledger(&paths).unwrap().len(), 1);

    let paused = run_serial_runtime_tick(
        &mut session,
        RuntimeTickOptions {
            now: Some(timestamp("2026-04-28T20:30:00Z")),
            run_id: Some("run-paused-governance".to_owned()),
            request_id: Some("request-paused-governance".to_owned()),
        },
    )
    .unwrap();
    assert_eq!(paused.kind, RuntimeTickOutcomeKind::Paused);
    assert!(paused.stage_request.is_none());

    let checker = TokenTerminalRunner {
        runner_name: "token-checker",
        terminal_marker: "### CHECKER_PASS",
        token_usage: TokenUsage {
            input_tokens: 1,
            cached_input_tokens: 0,
            output_tokens: 0,
            thinking_tokens: 0,
            total_tokens: 1,
        },
        completed_at: "2026-04-29T02:00:00Z",
    };
    let resumed = run_serial_runtime_tick_with_runner(
        &mut session,
        RuntimeTickOptions {
            now: Some(timestamp("2026-04-29T02:00:00Z")),
            run_id: Some("run-unused-after-resume".to_owned()),
            request_id: Some("request-checker".to_owned()),
        },
        &checker,
    )
    .unwrap();

    assert_eq!(resumed.kind, RuntimeTickOutcomeKind::StageDispatched);
    assert_eq!(resumed.stage_result.unwrap().stage, StageName::Checker);
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(!snapshot.paused);
    assert!(snapshot.pause_sources.is_empty());
    let events = runtime_events(&paths);
    assert!(
        events
            .iter()
            .any(|event| event["event_type"] == "usage_governance_blocked")
    );
    assert!(
        events
            .iter()
            .any(|event| event["event_type"] == "usage_governance_paused")
    );
    assert!(
        events
            .iter()
            .any(|event| event["event_type"] == "usage_governance_resumed")
    );
}

#[test]
fn serial_runtime_subscription_quota_degraded_fail_open_and_fail_closed() {
    let fail_open_temp = TempDir::new().unwrap();
    let fail_open_paths = initialize_workspace(fail_open_temp.path().join("workspace")).unwrap();
    write_subscription_governance_config(&fail_open_paths, false);
    QueueStore::from_paths(fail_open_paths.clone())
        .enqueue_task(&task_document("task-quota-open"))
        .unwrap();
    let mut fail_open_session =
        startup_runtime_once_for_paths(&fail_open_paths, startup_options("quota-open")).unwrap();
    let fail_open = run_serial_runtime_tick(
        &mut fail_open_session,
        tick_options("run-quota-open", "request-quota-open"),
    )
    .unwrap();
    assert_eq!(fail_open.kind, RuntimeTickOutcomeKind::StageRequestReady);
    let fail_open_state = load_usage_governance_state(&fail_open_paths).unwrap();
    assert!(fail_open_state.active_blockers.is_empty());

    let fail_closed_temp = TempDir::new().unwrap();
    let fail_closed_paths =
        initialize_workspace(fail_closed_temp.path().join("workspace")).unwrap();
    write_subscription_governance_config(&fail_closed_paths, true);
    QueueStore::from_paths(fail_closed_paths.clone())
        .enqueue_task(&task_document("task-quota-closed"))
        .unwrap();
    let mut fail_closed_session =
        startup_runtime_once_for_paths(&fail_closed_paths, startup_options("quota-closed"))
            .unwrap();
    let fail_closed = run_serial_runtime_tick(
        &mut fail_closed_session,
        tick_options("run-quota-closed", "request-quota-closed"),
    )
    .unwrap();
    assert_eq!(fail_closed.kind, RuntimeTickOutcomeKind::Paused);
    assert!(
        fail_closed_paths
            .tasks_queue_dir
            .join("task-quota-closed.md")
            .is_file()
    );
    let fail_closed_state = load_usage_governance_state(&fail_closed_paths).unwrap();
    assert_eq!(
        fail_closed_state.active_blockers[0].rule_id,
        "subscription-quota-degraded-fail-closed"
    );
    assert!(
        runtime_events(&fail_closed_paths)
            .iter()
            .any(|event| event["event_type"] == "usage_governance_degraded")
    );
}

fn startup_lock_options(session_id: &str) -> RuntimeOwnershipLockOptions {
    RuntimeOwnershipLockOptions::new(std::process::id(), "test-host", session_id, STARTUP_NOW)
        .unwrap()
}

fn startup_options(session_id: &str) -> RuntimeStartupOptions {
    RuntimeStartupOptions {
        lock_options: Some(startup_lock_options(session_id)),
        now: Some(timestamp(STARTUP_NOW)),
        ..RuntimeStartupOptions::default()
    }
}

fn task_document(task_id: &str) -> TaskDocument {
    TaskDocument {
        task_id: task_id.to_owned(),
        title: format!("Task {task_id}"),
        summary: "runtime startup test".to_owned(),
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        spec_id: Some("spec-root-001".to_owned()),
        parent_task_id: None,
        incident_id: None,
        target_paths: vec!["src/runtime/".to_owned()],
        acceptance: vec!["startup handles active state".to_owned()],
        required_checks: vec!["cargo test --test runtime_serial".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/runtime/lifecycle.py".to_owned()],
        risk: vec!["stale startup state".to_owned()],
        depends_on: Vec::new(),
        blocks: Vec::new(),
        tags: vec!["runtime".to_owned()],
        status_hint: None,
        created_at: timestamp(STARTUP_NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn spec_document(spec_id: &str) -> SpecDocument {
    SpecDocument {
        spec_id: spec_id.to_owned(),
        title: format!("Spec {spec_id}"),
        summary: "runtime activation spec".to_owned(),
        source_type: SpecSourceType::Idea,
        source_id: Some("idea-001".to_owned()),
        parent_spec_id: None,
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some(spec_id.to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        goals: vec!["plan runtime work".to_owned()],
        non_goals: Vec::new(),
        scope: vec!["runtime activation".to_owned()],
        constraints: vec!["serial once mode".to_owned()],
        assumptions: Vec::new(),
        risks: vec!["planning priority drift".to_owned()],
        target_paths: vec!["src/runtime/".to_owned()],
        entrypoints: Vec::new(),
        required_skills: Vec::new(),
        decomposition_hints: Vec::new(),
        acceptance: vec!["planner activates first".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/runtime/activation.py".to_owned()],
        created_at: timestamp(STARTUP_NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn probe_document_for_recon(probe_id: &str) -> ProbeDocument {
    ProbeDocument {
        probe_id: probe_id.to_owned(),
        title: format!("Probe {probe_id}"),
        summary: "runtime recon routing test".to_owned(),
        request: "Research the repo surface and route this work safely.".to_owned(),
        target_paths: vec!["src/runtime/".to_owned()],
        constraints: vec!["Do not implement during recon.".to_owned()],
        acceptance: vec!["Recon packet is applied by the runtime.".to_owned()],
        risk_notes: vec!["result application can move probes too early".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/runtime/recon_transitions.py".to_owned()],
        tags: vec!["probe".to_owned(), "recon".to_owned()],
        status_hint: None,
        created_at: timestamp(STARTUP_NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn recon_packet_for(probe_id: &str, decision: ReconDecision) -> ReconPacketDocument {
    ReconPacketDocument {
        schema_version: "1.0".to_owned(),
        kind: "recon_packet".to_owned(),
        recon_packet_id: format!("recon-{probe_id}"),
        probe_id: probe_id.to_owned(),
        decision,
        confidence: ReconConfidence::High,
        risk_level: ReconRiskLevel::Medium,
        request_summary: "Research before routing.".to_owned(),
        interpreted_goal: "Route this work through the smallest safe lane.".to_owned(),
        relevant_paths: vec![ReconPathFinding {
            path: "src/runtime/".to_owned(),
            reason: "Runtime result application owns Recon handoff.".to_owned(),
        }],
        relevant_symbols: Vec::new(),
        relevant_tests: vec![ReconPathFinding {
            path: "tests/runtime_serial.rs".to_owned(),
            reason: "Serial runtime coverage proves routing and lifecycle.".to_owned(),
        }],
        semantic_invariants: vec![
            "Validate packet and generated artifact before moving probe.".to_owned(),
        ],
        edge_cases_to_preserve: Vec::new(),
        verification_plan: ReconVerificationPlan {
            required_commands: vec!["cargo test --test runtime_serial".to_owned()],
            focused_checks: vec!["Recon result application tests".to_owned()],
            fallback_checks: Vec::new(),
        },
        open_questions: Vec::new(),
        handoff_target: decision.handoff_target(),
        emitted_task_id: (decision == ReconDecision::ToExecution)
            .then(|| "task-from-probe".to_owned()),
        emitted_spec_id: (decision == ReconDecision::ToPlanning)
            .then(|| "spec-from-probe".to_owned()),
        created_at: timestamp(STARTUP_NOW),
        created_by: "recon".to_owned(),
    }
}

fn generated_probe_task(task_id: &str) -> TaskDocument {
    let mut task = task_document(task_id);
    task.title = "Task from probe".to_owned();
    task.summary = "direct execution route".to_owned();
    task.root_idea_id = Some("idea-from-probe".to_owned());
    task.root_spec_id = Some("spec-from-probe-root".to_owned());
    task.root_intake_kind = None;
    task.root_intake_id = None;
    task.references = vec!["millrace-agents/probes/active/probe-001.md".to_owned()];
    task.created_by = "recon".to_owned();
    task
}

fn generated_probe_spec(spec_id: &str) -> SpecDocument {
    let mut spec = spec_document(spec_id);
    spec.title = "Spec from probe".to_owned();
    spec.summary = "planning route".to_owned();
    spec.source_type = SpecSourceType::Manual;
    spec.source_id = None;
    spec.root_idea_id = Some("idea-from-probe".to_owned());
    spec.root_spec_id = None;
    spec.root_intake_kind = None;
    spec.root_intake_id = None;
    spec.references = vec!["millrace-agents/probes/active/probe-001.md".to_owned()];
    spec.created_by = "recon".to_owned();
    spec
}

fn incident_document(incident_id: &str) -> IncidentDocument {
    IncidentDocument {
        incident_id: incident_id.to_owned(),
        title: format!("Incident {incident_id}"),
        summary: "runtime activation incident".to_owned(),
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        source_task_id: None,
        source_spec_id: Some("spec-root-001".to_owned()),
        source_stage: StageName::Checker,
        source_plane: Plane::Execution,
        failure_class: "runtime_activation_gap".to_owned(),
        severity: IncidentSeverity::Medium,
        needs_planning: true,
        trigger_reason: "runtime_serial_test".to_owned(),
        observed_symptoms: vec!["closure activation should remain blocked".to_owned()],
        failed_attempts: Vec::new(),
        consultant_decision: IncidentDecision::NeedsPlanning,
        evidence_paths: vec!["tests/runtime_serial.rs".to_owned()],
        related_run_ids: Vec::new(),
        related_stage_results: Vec::new(),
        references: vec!["../millrace-py/tests/runtime/test_completion_behavior.py".to_owned()],
        opened_at: timestamp(STARTUP_NOW),
        opened_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn write_idea(paths: &millrace_ai::workspace::WorkspacePaths, idea_id: &str, body: &str) {
    let idea_path = paths
        .root
        .join("ideas")
        .join("inbox")
        .join(format!("{idea_id}.md"));
    fs::create_dir_all(idea_path.parent().unwrap()).unwrap();
    fs::write(idea_path, body).unwrap();
}

fn learning_request_document(learning_request_id: &str) -> LearningRequestDocument {
    LearningRequestDocument {
        learning_request_id: learning_request_id.to_owned(),
        title: format!("Learning request {learning_request_id}"),
        summary: "runtime startup learning surface".to_owned(),
        requested_action: LearningRequestAction::Improve,
        target_skill_id: Some("builder-core".to_owned()),
        target_stage: None,
        source_refs: vec!["run:run-001".to_owned()],
        preferred_output_paths: Vec::new(),
        trigger_metadata: json!({"source": "runtime_startup_test"}),
        originating_run_ids: vec!["run-001".to_owned()],
        artifact_paths: Vec::new(),
        references: Vec::new(),
        created_at: timestamp(STARTUP_NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn closure_target_state(root_spec_id: &str, root_idea_id: &str) -> ClosureTargetState {
    ClosureTargetState {
        schema_version: "1.0".to_owned(),
        kind: "closure_target_state".to_owned(),
        root_spec_id: root_spec_id.to_owned(),
        root_idea_id: root_idea_id.to_owned(),
        root_intake_kind: None,
        root_intake_id: None,
        root_spec_path: format!("millrace-agents/arbiter/contracts/root-specs/{root_spec_id}.md"),
        root_idea_path: format!("millrace-agents/arbiter/contracts/ideas/{root_idea_id}.md"),
        rubric_path: format!("millrace-agents/arbiter/rubrics/{root_spec_id}.md"),
        latest_verdict_path: None,
        latest_report_path: None,
        closure_open: true,
        closure_blocked_by_lineage_work: false,
        blocking_work_ids: Vec::new(),
        opened_at: timestamp(STARTUP_NOW),
        closed_at: None,
        last_arbiter_run_id: None,
    }
}

fn active_run_state(
    plane: Plane,
    stage: StageName,
    node_id: &str,
    run_id: &str,
    request_kind: ActiveRunRequestKind,
    work_item_kind: Option<WorkItemKind>,
    work_item_id: Option<&str>,
) -> ActiveRunState {
    ActiveRunState {
        plane,
        stage,
        node_id: node_id.to_owned(),
        stage_kind_id: node_id.to_owned(),
        run_id: run_id.to_owned(),
        request_kind,
        work_item_kind,
        work_item_id: work_item_id.map(str::to_owned),
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        active_since: timestamp("2026-04-28T20:11:00Z"),
        running_status_marker: None,
    }
}

fn install_single_active_run(
    paths: &millrace_ai::WorkspacePaths,
    session: &mut millrace_ai::RuntimeStartupSession,
    active_run: ActiveRunState,
) {
    session.snapshot.active_runs_by_plane.clear();
    session
        .snapshot
        .active_runs_by_plane
        .insert(active_run.plane, active_run.clone());
    session.snapshot.active_plane = Some(active_run.plane);
    session.snapshot.active_stage = Some(active_run.stage);
    session.snapshot.active_node_id = Some(active_run.node_id.clone());
    session.snapshot.active_stage_kind_id = Some(active_run.stage_kind_id.clone());
    session.snapshot.active_run_id = Some(active_run.run_id.clone());
    session.snapshot.active_work_item_kind = active_run.work_item_kind;
    session.snapshot.active_work_item_id = active_run.work_item_id.clone();
    session.snapshot.active_since = Some(active_run.active_since.clone());
    save_snapshot(paths, &session.snapshot).unwrap();
}

fn active_item_path(
    paths: &millrace_ai::WorkspacePaths,
    work_item_kind: WorkItemKind,
    work_item_id: &str,
) -> PathBuf {
    match work_item_kind {
        WorkItemKind::Task => paths.tasks_active_dir.join(format!("{work_item_id}.md")),
        WorkItemKind::Probe => paths.probes_active_dir.join(format!("{work_item_id}.md")),
        WorkItemKind::Spec => paths.specs_active_dir.join(format!("{work_item_id}.md")),
        WorkItemKind::Incident => paths
            .incidents_active_dir
            .join(format!("{work_item_id}.md")),
        WorkItemKind::LearningRequest => paths
            .learning_requests_active_dir
            .join(format!("{work_item_id}.md")),
    }
}

fn queued_item_path(
    paths: &millrace_ai::WorkspacePaths,
    work_item_kind: WorkItemKind,
    work_item_id: &str,
) -> PathBuf {
    match work_item_kind {
        WorkItemKind::Task => paths.tasks_queue_dir.join(format!("{work_item_id}.md")),
        WorkItemKind::Probe => paths.probes_queue_dir.join(format!("{work_item_id}.md")),
        WorkItemKind::Spec => paths.specs_queue_dir.join(format!("{work_item_id}.md")),
        WorkItemKind::Incident => paths
            .incidents_incoming_dir
            .join(format!("{work_item_id}.md")),
        WorkItemKind::LearningRequest => paths
            .learning_requests_queue_dir
            .join(format!("{work_item_id}.md")),
    }
}

fn assert_stage_work_item_ownership_guard_requeues(
    case_id: &str,
    plane: Plane,
    stage: StageName,
    request_kind: ActiveRunRequestKind,
    work_item_kind: WorkItemKind,
    work_item_id: &str,
    claim_active: impl FnOnce(&QueueStore),
) {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    claim_active(&queue);
    assert!(active_item_path(&paths, work_item_kind, work_item_id).is_file());

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options(&format!("tick-{case_id}")))
            .unwrap();
    install_single_active_run(
        &paths,
        &mut session,
        active_run_state(
            plane,
            stage,
            stage.as_str(),
            &format!("run-{case_id}"),
            request_kind,
            Some(work_item_kind),
            Some(work_item_id),
        ),
    );

    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options(
            &format!("ignored-run-{case_id}"),
            &format!("request-{case_id}"),
        ),
    )
    .unwrap();

    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::Blocked);
    assert_eq!(outcome.reason, "stage_work_item_ownership_invalid");
    assert!(outcome.stage_request.is_none());
    assert!(!active_item_path(&paths, work_item_kind, work_item_id).exists());
    assert!(queued_item_path(&paths, work_item_kind, work_item_id).is_file());

    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.active_runs_by_plane.is_empty());
    assert!(snapshot.active_stage.is_none());
    assert_eq!(
        snapshot.current_failure_class.as_deref(),
        Some("stage_work_item_ownership_invalid")
    );
    assert_eq!(load_execution_status(&paths).unwrap(), "### IDLE");
    assert_eq!(load_planning_status(&paths).unwrap(), "### IDLE");
    assert_eq!(load_learning_status(&paths).unwrap(), "### IDLE");

    let context = RuntimeErrorContext::from_json_str(
        &fs::read_to_string(&paths.runtime_error_context_file).unwrap(),
    )
    .unwrap();
    assert_eq!(
        context.error_code.as_str(),
        "stage_work_item_ownership_invalid"
    );
    assert_eq!(context.failed_stage, stage);
    assert_eq!(context.work_item_kind, work_item_kind);
    assert_eq!(context.work_item_id, work_item_id);
    assert!(Path::new(&context.report_path).is_file());

    let events = runtime_events(&paths);
    let event = events
        .iter()
        .find(|event| event["event_type"] == "runtime_stage_work_item_ownership_invalid")
        .unwrap();
    assert_eq!(event["data"]["stage"], stage.as_str());
    assert_eq!(event["data"]["work_item_kind"], work_item_kind.as_str());
    assert_eq!(event["data"]["work_item_id"], work_item_id);
    assert_eq!(event["data"]["requeued_count"], 1);

    session.finish().unwrap();
}

fn tick_options(run_id: &str, request_id: &str) -> RuntimeTickOptions {
    RuntimeTickOptions {
        now: Some(timestamp("2026-04-28T20:10:00Z")),
        run_id: Some(run_id.to_owned()),
        request_id: Some(request_id.to_owned()),
    }
}

fn runtime_events(paths: &millrace_ai::WorkspacePaths) -> Vec<Value> {
    let event_log = paths.logs_dir.join("runtime_events.jsonl");
    fs::read_to_string(event_log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn assert_curator_decision_artifact_is_inspectable(
    outcome: &millrace_ai::RuntimeTickDispatchOutcome,
    run_dir: &Path,
) {
    assert!(run_dir.join("curator_decision.md").is_file());

    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert!(
        stage_result
            .artifact_paths
            .iter()
            .any(|path| path == "curator_decision.md")
    );

    let stage_result_path = outcome.stage_result_path.as_ref().unwrap();
    let persisted_stage_result: Value =
        serde_json::from_str(&fs::read_to_string(stage_result_path).unwrap()).unwrap();
    assert!(
        persisted_stage_result["artifact_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path.as_str() == Some("curator_decision.md"))
    );

    let evidence_path = outcome
        .stage_request
        .as_ref()
        .and_then(|request| request.skill_revision_evidence_path.as_ref())
        .map(PathBuf::from)
        .unwrap();
    assert!(evidence_path.starts_with(run_dir));
    assert!(evidence_path.is_file());
    assert_eq!(
        stage_result.metadata["skill_revision_evidence_path"],
        evidence_path.display().to_string()
    );
}

fn assert_no_learning_update_candidate_records(paths: &millrace_ai::WorkspacePaths) {
    for state in ["deferred", "applied"] {
        let directory = paths.learning_update_candidates_dir.join(state);
        if !directory.exists() {
            continue;
        }
        let records = fs::read_dir(&directory)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        assert!(
            records.is_empty(),
            "unexpected learning update candidate records in {}: {:?}",
            directory.display(),
            records
        );
    }
}

#[test]
fn inspect_run_trace_derives_incomplete_fallback_from_stage_results() {
    let temp = TempDir::new().unwrap();
    let run_dir = temp.path().join("run-fallback");
    let stage_results_dir = run_dir.join("stage_results");
    fs::create_dir_all(&stage_results_dir).unwrap();
    let mut stage_result = sample_stage_result_with_tokens(
        TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 2,
            output_tokens: 4,
            thinking_tokens: 1,
            total_tokens: 14,
        },
        STARTUP_NOW,
    );
    stage_result.run_id = "run-fallback".to_owned();
    stage_result.metadata.insert(
        "request_id".to_owned(),
        Value::String("request-fallback".to_owned()),
    );
    stage_result.metadata.insert(
        "compiled_plan_id".to_owned(),
        Value::String("plan-fallback".to_owned()),
    );
    fs::write(
        stage_results_dir.join("request-fallback.json"),
        serde_json::to_string_pretty(&stage_result).unwrap() + "\n",
    )
    .unwrap();

    let trace = inspect_run_trace(&run_dir).unwrap();

    assert_eq!(trace.kind, "run_trace_graph");
    assert_eq!(trace.status.as_str(), "incomplete");
    assert_eq!(trace.run_id, "run-fallback");
    assert_eq!(trace.compiled_plan_id.as_deref(), Some("plan-fallback"));
    assert_eq!(trace.nodes[0].trace_node_id, "request-fallback");
    assert_eq!(trace.nodes[0].terminal_result, "BUILDER_COMPLETE");
    assert_eq!(
        trace.nodes[0].token_usage.as_ref().unwrap().total_tokens,
        14
    );
    assert!(trace.edges.is_empty());
    assert!(
        trace
            .notes
            .iter()
            .any(|note| note == "derived from stage result artifacts")
    );
    assert!(!trace_path_for_run_dir(&run_dir).exists());
}

#[test]
fn inspect_run_trace_derives_malformed_fallback_when_trace_json_cannot_decode() {
    let temp = TempDir::new().unwrap();
    let run_dir = temp.path().join("run-malformed");
    let stage_results_dir = run_dir.join("stage_results");
    fs::create_dir_all(&stage_results_dir).unwrap();
    fs::write(run_dir.join("run_trace.json"), "{bad\n").unwrap();
    let mut stage_result = sample_stage_result_with_tokens(
        TokenUsage {
            input_tokens: 1,
            cached_input_tokens: 0,
            output_tokens: 1,
            thinking_tokens: 0,
            total_tokens: 2,
        },
        STARTUP_NOW,
    );
    stage_result.run_id = "run-malformed".to_owned();
    stage_result.metadata.insert(
        "request_id".to_owned(),
        Value::String("request-malformed".to_owned()),
    );
    fs::write(
        stage_results_dir.join("request-malformed.json"),
        serde_json::to_string_pretty(&stage_result).unwrap() + "\n",
    )
    .unwrap();

    let trace = inspect_run_trace(&run_dir).unwrap();

    assert_eq!(trace.status.as_str(), "malformed");
    assert!(
        trace
            .notes
            .iter()
            .any(|note| note.contains("run_trace.json malformed"))
    );
    assert_eq!(trace.nodes[0].trace_node_id, "request-malformed");
    assert_eq!(
        fs::read_to_string(run_dir.join("run_trace.json")).unwrap(),
        "{bad\n"
    );
}

#[test]
fn record_router_decision_trace_includes_spawned_learning_request_edge_evidence() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let run_dir = paths.runs_dir.join("run-spawned");
    let stage_results_dir = run_dir.join("stage_results");
    fs::create_dir_all(&stage_results_dir).unwrap();
    let stage_result_path = stage_results_dir.join("request-spawned.json");
    let mut stage_result = sample_stage_result_with_tokens(
        TokenUsage {
            input_tokens: 1,
            cached_input_tokens: 0,
            output_tokens: 1,
            thinking_tokens: 0,
            total_tokens: 2,
        },
        STARTUP_NOW,
    );
    stage_result.run_id = "run-spawned".to_owned();
    stage_result.metadata.insert(
        "request_id".to_owned(),
        Value::String("request-spawned".to_owned()),
    );
    fs::write(
        &stage_result_path,
        serde_json::to_string_pretty(&stage_result).unwrap() + "\n",
    )
    .unwrap();
    upsert_stage_result_trace_node(&paths, &run_dir, &stage_result, &stage_result_path);
    let learning_request_path = paths.learning_requests_queue_dir.join("learn-spawned.md");
    fs::create_dir_all(learning_request_path.parent().unwrap()).unwrap();
    fs::write(&learning_request_path, "# Learn\n").unwrap();

    let decision = RouterDecision {
        action: RouterAction::RunStage,
        next_plane: None,
        next_stage: Some(StageName::Checker),
        next_node_id: Some("checker".to_owned()),
        next_stage_kind_id: Some("checker".to_owned()),
        failure_class: None,
        counter_key: None,
        create_incident: false,
        reason: "builder:BUILDER_COMPLETE".to_owned(),
    };
    record_router_decision_trace(
        &paths,
        &run_dir,
        &stage_result,
        &decision,
        vec![spawned_work_ref_from_path(
            &learning_request_path,
            &stage_result,
            "learning_trigger",
        )],
    );

    let trace =
        RunTraceGraph::from_json_str(&fs::read_to_string(run_dir.join("run_trace.json")).unwrap())
            .unwrap();
    assert_eq!(trace.edges[0].target_node_id.as_deref(), Some("checker"));
    assert_eq!(
        trace.edges[0].spawned_work[0].kind.as_str(),
        "learning_request"
    );
    assert_eq!(trace.edges[0].spawned_work[0].item_id, "learn-spawned");
    assert_eq!(
        trace.edges[0].spawned_work[0].reason.as_deref(),
        Some("learning_trigger")
    );
}

#[test]
fn run_trace_write_failure_emits_runtime_event_without_blocking_stage_result() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let run_dir = paths.runs_dir.join("run-trace-write-fail");
    let stage_results_dir = run_dir.join("stage_results");
    fs::create_dir_all(&stage_results_dir).unwrap();
    fs::create_dir_all(run_dir.join("run_trace.json")).unwrap();
    let stage_result_path = stage_results_dir.join("request-trace-write-fail.json");
    let mut stage_result = sample_stage_result_with_tokens(
        TokenUsage {
            input_tokens: 1,
            cached_input_tokens: 0,
            output_tokens: 1,
            thinking_tokens: 0,
            total_tokens: 2,
        },
        STARTUP_NOW,
    );
    stage_result.run_id = "run-trace-write-fail".to_owned();
    stage_result.metadata.insert(
        "request_id".to_owned(),
        Value::String("request-trace-write-fail".to_owned()),
    );
    fs::write(
        &stage_result_path,
        serde_json::to_string_pretty(&stage_result).unwrap() + "\n",
    )
    .unwrap();

    upsert_stage_result_trace_node(&paths, &run_dir, &stage_result, &stage_result_path);

    assert!(stage_result_path.is_file());
    let events = runtime_events(&paths);
    assert_eq!(events[0]["event_type"], "run_trace_write_failed");
    assert_eq!(events[0]["data"]["run_id"], "run-trace-write-fail");
    assert_eq!(events[0]["data"]["phase"], "node");
}

fn default_source_skill_tree() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/assets/baseline/skills")
}

fn file_tree_snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    collect_file_tree_snapshot(root, root, &mut files);
    files
}

fn collect_file_tree_snapshot(root: &Path, current: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
    let mut entries = fs::read_dir(current)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            collect_file_tree_snapshot(root, &path, files);
        } else if file_type.is_file() {
            let relative_path = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(relative_path, fs::read(&path).unwrap());
        }
    }
}

fn write_runtime_token_governance_config(
    paths: &millrace_ai::WorkspacePaths,
    threshold: u64,
    auto_resume: bool,
) {
    fs::write(
        &paths.runtime_config_file,
        vec![
            "[runtime]".to_owned(),
            "default_mode = \"default_codex\"".to_owned(),
            "run_style = \"daemon\"".to_owned(),
            String::new(),
            "[usage_governance]".to_owned(),
            "enabled = true".to_owned(),
            format!("auto_resume = {auto_resume}"),
            "calendar_timezone = \"UTC\"".to_owned(),
            String::new(),
            "[usage_governance.runtime_token_rules]".to_owned(),
            "enabled = true".to_owned(),
            String::new(),
            "[[usage_governance.runtime_token_rules.rules]]".to_owned(),
            "rule_id = \"test-rolling\"".to_owned(),
            "window = \"rolling_5h\"".to_owned(),
            "metric = \"total_tokens\"".to_owned(),
            format!("threshold = {threshold}"),
            String::new(),
            "[usage_governance.subscription_quota_rules]".to_owned(),
            "enabled = false".to_owned(),
            String::new(),
        ]
        .join("\n"),
    )
    .unwrap();
}

fn write_subscription_governance_config(paths: &millrace_ai::WorkspacePaths, fail_closed: bool) {
    fs::write(
        &paths.runtime_config_file,
        vec![
            "[runtime]".to_owned(),
            "default_mode = \"default_codex\"".to_owned(),
            "run_style = \"daemon\"".to_owned(),
            String::new(),
            "[usage_governance]".to_owned(),
            "enabled = true".to_owned(),
            "auto_resume = true".to_owned(),
            String::new(),
            "[usage_governance.runtime_token_rules]".to_owned(),
            "enabled = false".to_owned(),
            String::new(),
            "[usage_governance.subscription_quota_rules]".to_owned(),
            "enabled = true".to_owned(),
            format!(
                "degraded_policy = \"{}\"",
                if fail_closed {
                    "fail_closed"
                } else {
                    "fail_open"
                }
            ),
            "refresh_interval_seconds = 60".to_owned(),
            String::new(),
            "[[usage_governance.subscription_quota_rules.rules]]".to_owned(),
            "rule_id = \"quota-five-hour-test\"".to_owned(),
            "window = \"five_hour\"".to_owned(),
            "pause_at_percent_used = 95".to_owned(),
            String::new(),
        ]
        .join("\n"),
    )
    .unwrap();
}

fn write_runtime_error_catalog(root: &Path) -> PathBuf {
    let path = root
        .join("docs")
        .join("runtime")
        .join("millrace-runtime-error-codes.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "# Runtime Error Codes\n").unwrap();
    path
}

fn rename_compiled_node(
    session: &mut millrace_ai::RuntimeStartupSession,
    plane: Plane,
    old_node_id: &str,
    new_node_id: &str,
) {
    let graph = match plane {
        Plane::Execution => &mut session.compiled_plan.execution_graph,
        Plane::Planning => &mut session.compiled_plan.planning_graph,
        Plane::Learning => session.compiled_plan.learning_graph.as_mut().unwrap(),
    };
    let node = graph
        .nodes
        .iter_mut()
        .find(|node| node.node_id == old_node_id)
        .unwrap();
    node.node_id = new_node_id.to_owned();
}

struct PreemptiveCompletionRunner {
    paths: millrace_ai::WorkspacePaths,
    inner: FakeRunner,
    stage: StageName,
    work_item_kind: WorkItemKind,
    work_item_id: &'static str,
}

impl StageRunnerAdapter for PreemptiveCompletionRunner {
    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        if request.stage == self.stage {
            let queue = QueueStore::from_paths(self.paths.clone());
            match self.work_item_kind {
                WorkItemKind::Task => {
                    queue.mark_task_done(self.work_item_id).unwrap();
                }
                WorkItemKind::Spec => {
                    queue.mark_spec_done(self.work_item_id).unwrap();
                }
                WorkItemKind::Probe | WorkItemKind::Incident | WorkItemKind::LearningRequest => {
                    panic!("unsupported preemptive completion kind");
                }
            }
        }
        self.inner.run(request)
    }
}

#[derive(Clone)]
struct StaticTokenRunner {
    runner_name: &'static str,
    terminal_marker: &'static str,
}

impl StageRunnerAdapter for StaticTokenRunner {
    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        let run_dir = Path::new(&request.run_dir);
        fs::create_dir_all(run_dir).map_err(|error| RunnerError::Io {
            path: run_dir.display().to_string(),
            message: error.to_string(),
        })?;
        let stdout_path = run_dir.join(format!(
            "static-{}-{}.stdout.txt",
            self.runner_name, request.request_id
        ));
        fs::write(&stdout_path, format!("{}\n", self.terminal_marker)).map_err(|error| {
            RunnerError::Io {
                path: stdout_path.display().to_string(),
                message: error.to_string(),
            }
        })?;
        let raw_result = RunnerRawResult {
            request_id: request.request_id.clone(),
            run_id: request.run_id.clone(),
            stage: request.stage,
            runner_name: self.runner_name.to_owned(),
            model_name: request.model_name.clone(),
            thinking_level: request.thinking_level.clone(),
            model_reasoning_effort: request.model_reasoning_effort.clone(),
            exit_kind: millrace_ai::RunnerExitKind::Completed,
            exit_code: Some(0),
            observed_exit_kind: None,
            observed_exit_code: None,
            stdout_path: Some(stdout_path.display().to_string()),
            stderr_path: None,
            terminal_result_path: None,
            event_log_path: None,
            token_usage: None,
            started_at: timestamp("2026-04-28T20:10:00Z"),
            ended_at: timestamp("2026-04-28T20:10:02Z"),
        };
        raw_result.validate()?;
        Ok(raw_result)
    }
}

#[derive(Clone)]
struct TokenTerminalRunner {
    runner_name: &'static str,
    terminal_marker: &'static str,
    token_usage: TokenUsage,
    completed_at: &'static str,
}

impl StageRunnerAdapter for TokenTerminalRunner {
    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        let run_dir = Path::new(&request.run_dir);
        fs::create_dir_all(run_dir).map_err(|error| RunnerError::Io {
            path: run_dir.display().to_string(),
            message: error.to_string(),
        })?;
        let stdout_path = run_dir.join(format!(
            "token-{}-{}.stdout.txt",
            self.runner_name, request.request_id
        ));
        fs::write(&stdout_path, format!("{}\n", self.terminal_marker)).map_err(|error| {
            RunnerError::Io {
                path: stdout_path.display().to_string(),
                message: error.to_string(),
            }
        })?;
        let raw_result = RunnerRawResult {
            request_id: request.request_id.clone(),
            run_id: request.run_id.clone(),
            stage: request.stage,
            runner_name: self.runner_name.to_owned(),
            model_name: request.model_name.clone(),
            thinking_level: request.thinking_level.clone(),
            model_reasoning_effort: request.model_reasoning_effort.clone(),
            exit_kind: RunnerExitKind::Completed,
            exit_code: Some(0),
            observed_exit_kind: None,
            observed_exit_code: None,
            stdout_path: Some(stdout_path.display().to_string()),
            stderr_path: None,
            terminal_result_path: None,
            event_log_path: None,
            token_usage: Some(self.token_usage.clone()),
            started_at: timestamp(self.completed_at),
            ended_at: timestamp(self.completed_at),
        };
        raw_result.validate()?;
        Ok(raw_result)
    }
}

struct ArbiterArtifactRunner {
    terminal_marker: &'static str,
    verdict_json: &'static str,
    report_text: &'static str,
}

impl StageRunnerAdapter for ArbiterArtifactRunner {
    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        let verdict_path = request
            .preferred_verdict_path
            .as_ref()
            .map(PathBuf::from)
            .expect("arbiter request should include preferred verdict path");
        if let Some(parent) = verdict_path.parent() {
            fs::create_dir_all(parent).map_err(|error| millrace_ai::RunnerError::Io {
                path: parent.display().to_string(),
                message: error.to_string(),
            })?;
        }
        fs::write(&verdict_path, self.verdict_json).map_err(|error| {
            millrace_ai::RunnerError::Io {
                path: verdict_path.display().to_string(),
                message: error.to_string(),
            }
        })?;

        let report_path = request
            .preferred_report_path
            .as_ref()
            .map(PathBuf::from)
            .expect("arbiter request should include preferred report path");
        if let Some(parent) = report_path.parent() {
            fs::create_dir_all(parent).map_err(|error| millrace_ai::RunnerError::Io {
                path: parent.display().to_string(),
                message: error.to_string(),
            })?;
        }
        fs::write(&report_path, self.report_text).map_err(|error| {
            millrace_ai::RunnerError::Io {
                path: report_path.display().to_string(),
                message: error.to_string(),
            }
        })?;

        let mut config =
            FakeRunnerConfig::new(FakeRunnerResult::terminal_marker(self.terminal_marker)).unwrap();
        config.fixed_started_at = timestamp("2026-04-28T20:10:00Z");
        config.fixed_ended_at = timestamp("2026-04-28T20:10:01Z");
        FakeRunner::new(config).run(request)
    }
}

enum ReconGeneratedArtifact {
    Task(TaskDocument),
    Spec(SpecDocument),
}

struct ReconArtifactRunner {
    marker: &'static str,
    packet: ReconPacketDocument,
    generated: Option<ReconGeneratedArtifact>,
}

impl ReconArtifactRunner {
    fn new(
        marker: &'static str,
        packet: ReconPacketDocument,
        generated: Option<ReconGeneratedArtifact>,
    ) -> Self {
        Self {
            marker,
            packet,
            generated,
        }
    }
}

impl StageRunnerAdapter for ReconArtifactRunner {
    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        let run_dir = Path::new(&request.run_dir);
        fs::create_dir_all(run_dir).map_err(|error| RunnerError::Io {
            path: run_dir.display().to_string(),
            message: error.to_string(),
        })?;

        let packet_path = run_dir.join("recon_packet.md");
        fs::write(&packet_path, render_recon_packet(&self.packet)).map_err(|error| {
            RunnerError::Io {
                path: packet_path.display().to_string(),
                message: error.to_string(),
            }
        })?;

        if let Some(generated) = &self.generated {
            let (path, payload) = match generated {
                ReconGeneratedArtifact::Task(task) => (
                    run_dir.join("generated_task.md"),
                    serde_json::to_string_pretty(task).map_err(|error| {
                        RunnerError::InvalidRawResult {
                            message: error.to_string(),
                        }
                    })?,
                ),
                ReconGeneratedArtifact::Spec(spec) => (
                    run_dir.join("generated_spec.md"),
                    serde_json::to_string_pretty(spec).map_err(|error| {
                        RunnerError::InvalidRawResult {
                            message: error.to_string(),
                        }
                    })?,
                ),
            };
            fs::write(&path, format!("{payload}\n")).map_err(|error| RunnerError::Io {
                path: path.display().to_string(),
                message: error.to_string(),
            })?;
        }

        let stdout_path = run_dir.join(format!("recon-{}.stdout.txt", request.request_id));
        fs::write(&stdout_path, format!("{}\n", self.marker)).map_err(|error| RunnerError::Io {
            path: stdout_path.display().to_string(),
            message: error.to_string(),
        })?;

        let raw_result = RunnerRawResult {
            request_id: request.request_id.clone(),
            run_id: request.run_id.clone(),
            stage: request.stage,
            runner_name: "recon-artifact-runner".to_owned(),
            model_name: request.model_name.clone(),
            thinking_level: request.thinking_level.clone(),
            model_reasoning_effort: request.model_reasoning_effort.clone(),
            exit_kind: RunnerExitKind::Completed,
            exit_code: Some(0),
            observed_exit_kind: None,
            observed_exit_code: None,
            stdout_path: Some(stdout_path.display().to_string()),
            stderr_path: None,
            terminal_result_path: None,
            event_log_path: None,
            token_usage: None,
            started_at: timestamp("2026-04-28T20:10:00Z"),
            ended_at: timestamp("2026-04-28T20:10:01Z"),
        };
        raw_result.validate()?;
        Ok(raw_result)
    }
}

struct RawReconPacketRunner {
    marker: &'static str,
    packet_text: &'static str,
}

impl StageRunnerAdapter for RawReconPacketRunner {
    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        let run_dir = Path::new(&request.run_dir);
        fs::create_dir_all(run_dir).map_err(|error| RunnerError::Io {
            path: run_dir.display().to_string(),
            message: error.to_string(),
        })?;

        let packet_path = run_dir.join("recon_packet.md");
        fs::write(&packet_path, self.packet_text).map_err(|error| RunnerError::Io {
            path: packet_path.display().to_string(),
            message: error.to_string(),
        })?;

        let stdout_path = run_dir.join(format!("recon-{}.stdout.txt", request.request_id));
        fs::write(&stdout_path, format!("{}\n", self.marker)).map_err(|error| RunnerError::Io {
            path: stdout_path.display().to_string(),
            message: error.to_string(),
        })?;

        let raw_result = RunnerRawResult {
            request_id: request.request_id.clone(),
            run_id: request.run_id.clone(),
            stage: request.stage,
            runner_name: "raw-recon-packet-runner".to_owned(),
            model_name: request.model_name.clone(),
            thinking_level: request.thinking_level.clone(),
            model_reasoning_effort: request.model_reasoning_effort.clone(),
            exit_kind: RunnerExitKind::Completed,
            exit_code: Some(0),
            observed_exit_kind: None,
            observed_exit_code: None,
            stdout_path: Some(stdout_path.display().to_string()),
            stderr_path: None,
            terminal_result_path: None,
            event_log_path: None,
            token_usage: None,
            started_at: timestamp("2026-04-28T20:10:00Z"),
            ended_at: timestamp("2026-04-28T20:10:01Z"),
        };
        raw_result.validate()?;
        Ok(raw_result)
    }
}

#[derive(Clone, Copy)]
enum ScriptedStageOutput {
    Terminal(&'static str),
    TerminalWithFixContract {
        marker: &'static str,
        contract_text: &'static str,
    },
    Stdout(&'static str),
}

#[derive(Clone, Copy)]
struct ScriptedArbiterArtifacts {
    verdict_json: &'static str,
    report_text: &'static str,
}

struct ScriptedE2eRunner {
    outputs: Vec<(StageName, ScriptedStageOutput)>,
    paths: Option<millrace_ai::WorkspacePaths>,
    manager_task: RefCell<Option<TaskDocument>>,
    arbiter_artifacts: Option<ScriptedArbiterArtifacts>,
    stage_order: RefCell<Vec<StageName>>,
    requests: RefCell<Vec<StageRunRequest>>,
}

impl ScriptedE2eRunner {
    fn new(outputs: Vec<(StageName, ScriptedStageOutput)>) -> Self {
        Self {
            outputs,
            paths: None,
            manager_task: RefCell::new(None),
            arbiter_artifacts: None,
            stage_order: RefCell::new(Vec::new()),
            requests: RefCell::new(Vec::new()),
        }
    }

    fn with_manager_task(mut self, paths: millrace_ai::WorkspacePaths, task: TaskDocument) -> Self {
        self.paths = Some(paths);
        *self.manager_task.borrow_mut() = Some(task);
        self
    }

    fn with_arbiter_artifacts(mut self, artifacts: ScriptedArbiterArtifacts) -> Self {
        self.arbiter_artifacts = Some(artifacts);
        self
    }

    fn stage_order(&self) -> Vec<StageName> {
        self.stage_order.borrow().clone()
    }

    fn requests(&self) -> Vec<StageRunRequest> {
        self.requests.borrow().clone()
    }

    fn output_for_stage(&self, stage: StageName) -> ScriptedStageOutput {
        self.outputs
            .iter()
            .find_map(|(candidate, output)| (*candidate == stage).then_some(*output))
            .unwrap_or_else(|| panic!("missing scripted output for stage {stage}"))
    }
}

impl StageRunnerAdapter for ScriptedE2eRunner {
    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        self.stage_order.borrow_mut().push(request.stage);
        self.requests.borrow_mut().push(request.clone());

        if request.stage == StageName::Manager {
            if let Some(task) = self.manager_task.borrow_mut().take() {
                let paths = self
                    .paths
                    .as_ref()
                    .expect("manager task enqueue needs workspace paths");
                QueueStore::from_paths(paths.clone())
                    .enqueue_task(&task)
                    .map_err(|error| RunnerError::InvalidRawResult {
                        message: error.to_string(),
                    })?;
            }
        }

        if request.stage == StageName::Arbiter {
            if let Some(artifacts) = self.arbiter_artifacts {
                let verdict_path = request
                    .preferred_verdict_path
                    .as_ref()
                    .map(PathBuf::from)
                    .expect("arbiter request should include preferred verdict path");
                if let Some(parent) = verdict_path.parent() {
                    fs::create_dir_all(parent).map_err(|error| RunnerError::Io {
                        path: parent.display().to_string(),
                        message: error.to_string(),
                    })?;
                }
                fs::write(&verdict_path, artifacts.verdict_json).map_err(|error| {
                    RunnerError::Io {
                        path: verdict_path.display().to_string(),
                        message: error.to_string(),
                    }
                })?;

                let report_path = request
                    .preferred_report_path
                    .as_ref()
                    .map(PathBuf::from)
                    .expect("arbiter request should include preferred report path");
                if let Some(parent) = report_path.parent() {
                    fs::create_dir_all(parent).map_err(|error| RunnerError::Io {
                        path: parent.display().to_string(),
                        message: error.to_string(),
                    })?;
                }
                fs::write(&report_path, artifacts.report_text).map_err(|error| {
                    RunnerError::Io {
                        path: report_path.display().to_string(),
                        message: error.to_string(),
                    }
                })?;
            }
        }

        let run_dir = Path::new(&request.run_dir);
        fs::create_dir_all(run_dir).map_err(|error| RunnerError::Io {
            path: run_dir.display().to_string(),
            message: error.to_string(),
        })?;
        let stdout_path = run_dir.join(format!("e2e-{}.stdout.txt", request.request_id));
        let output = self.output_for_stage(request.stage);
        let mut terminal_result_path = None;
        if let ScriptedStageOutput::TerminalWithFixContract { contract_text, .. } = output {
            let contract_path = run_dir.join("fix_contract.md");
            fs::write(&contract_path, contract_text).map_err(|error| RunnerError::Io {
                path: contract_path.display().to_string(),
                message: error.to_string(),
            })?;
        }
        if let ScriptedStageOutput::TerminalWithFixContract { marker, .. } = output {
            let terminal_path =
                run_dir.join(format!("e2e-{}.terminal_result.json", request.request_id));
            let payload = serde_json::to_string_pretty(&json!({
                "terminal_result": marker.trim_start_matches("###").trim(),
                "summary_artifact_paths": ["fix_contract.md"],
            }))
            .map_err(|error| RunnerError::InvalidRawResult {
                message: error.to_string(),
            })?;
            fs::write(&terminal_path, format!("{payload}\n")).map_err(|error| RunnerError::Io {
                path: terminal_path.display().to_string(),
                message: error.to_string(),
            })?;
            terminal_result_path = Some(terminal_path.display().to_string());
        }
        let stdout_payload = match output {
            ScriptedStageOutput::Terminal(marker) => format!("{marker}\n"),
            ScriptedStageOutput::TerminalWithFixContract { marker, .. } => format!("{marker}\n"),
            ScriptedStageOutput::Stdout(payload) => payload.to_owned(),
        };
        fs::write(&stdout_path, stdout_payload).map_err(|error| RunnerError::Io {
            path: stdout_path.display().to_string(),
            message: error.to_string(),
        })?;

        let raw_result = RunnerRawResult {
            request_id: request.request_id.clone(),
            run_id: request.run_id.clone(),
            stage: request.stage,
            runner_name: "scripted-e2e-runner".to_owned(),
            model_name: request.model_name.clone(),
            thinking_level: request.thinking_level.clone(),
            model_reasoning_effort: request.model_reasoning_effort.clone(),
            exit_kind: RunnerExitKind::Completed,
            exit_code: Some(0),
            observed_exit_kind: None,
            observed_exit_code: None,
            stdout_path: Some(stdout_path.display().to_string()),
            stderr_path: None,
            terminal_result_path,
            event_log_path: None,
            token_usage: None,
            started_at: timestamp("2026-04-28T20:10:00Z"),
            ended_at: timestamp("2026-04-28T20:10:01Z"),
        };
        raw_result.validate()?;
        Ok(raw_result)
    }
}

fn standard_e2e_outputs() -> Vec<(StageName, ScriptedStageOutput)> {
    vec![
        (
            StageName::Auditor,
            ScriptedStageOutput::Terminal("### AUDITOR_COMPLETE"),
        ),
        (
            StageName::Planner,
            ScriptedStageOutput::Terminal("### PLANNER_COMPLETE"),
        ),
        (
            StageName::Manager,
            ScriptedStageOutput::Terminal("### MANAGER_COMPLETE"),
        ),
        (
            StageName::Builder,
            ScriptedStageOutput::Terminal("### BUILDER_COMPLETE"),
        ),
        (
            StageName::Checker,
            ScriptedStageOutput::Terminal("### CHECKER_PASS"),
        ),
        (
            StageName::Updater,
            ScriptedStageOutput::Terminal("### UPDATE_COMPLETE"),
        ),
    ]
}

fn task_document_for_lineage(
    task_id: &str,
    root_spec_id: &str,
    root_idea_id: &str,
) -> TaskDocument {
    let mut task = task_document(task_id);
    task.root_spec_id = Some(root_spec_id.to_owned());
    task.spec_id = Some(root_spec_id.to_owned());
    task.root_idea_id = Some(root_idea_id.to_owned());
    task
}

fn spec_document_for_lineage(spec_id: &str, root_idea_id: &str) -> SpecDocument {
    let mut spec = spec_document(spec_id);
    spec.root_spec_id = Some(spec_id.to_owned());
    spec.root_idea_id = Some(root_idea_id.to_owned());
    spec.source_id = Some(root_idea_id.to_owned());
    spec.references = vec![format!("ideas/inbox/{root_idea_id}.md")];
    spec
}

fn first_incident_document(paths: &millrace_ai::WorkspacePaths) -> IncidentDocument {
    let incident_path = fs::read_dir(&paths.incidents_incoming_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("md"))
        .expect("incoming incident should exist");
    parse_incident_document(&fs::read_to_string(incident_path).unwrap()).unwrap()
}

#[test]
fn stage_run_request_serializes_python_compatible_context_fields() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path());

    let serialized = serde_json::to_value(&request).unwrap();
    assert_eq!(serialized["request_id"], REQUEST_ID);
    assert_eq!(serialized["run_id"], RUN_ID);
    assert_eq!(serialized["request_kind"], "active_work_item");
    assert_eq!(serialized["entrypoint_contract_id"], "builder.contract.v1");
    assert_eq!(
        serialized["legal_terminal_markers"][0],
        "### BUILDER_COMPLETE"
    );
    assert_eq!(
        serialized["allowed_result_classes_by_outcome"]["BUILDER_COMPLETE"][0],
        "success"
    );
    assert_eq!(
        serialized["required_skill_paths"][0],
        "millrace-agents/skills/stage/execution/builder-core/SKILL.md"
    );
    assert_eq!(serialized["thinking_level"], "medium");
    assert_eq!(serialized["timeout_seconds"], 3600);

    let decoded = StageRunRequest::from_json_value(serialized.clone()).unwrap();
    assert_eq!(decoded, request);

    let context_lines = render_stage_request_context_lines(&decoded);
    assert!(context_lines.contains(&"Entrypoint Contract ID: builder.contract.v1".to_owned()));
    assert!(context_lines.contains(&"Active Work Item: task task-001".to_owned()));
    assert!(context_lines.contains(&"Thinking Level: medium".to_owned()));
    assert!(context_lines.contains(&"Timeout Seconds: 3600".to_owned()));

    let mut unknown_field = serialized;
    unknown_field["unsupported"] = json!(true);
    let error = StageRunRequest::from_json_value(unknown_field).unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn stage_run_request_rejects_stage_work_item_ownership_mismatch() {
    let temp = TempDir::new().unwrap();
    let mut request = sample_request(temp.path());
    request.stage = StageName::Builder;
    request.active_work_item_kind = Some(WorkItemKind::Spec);
    request.active_work_item_id = Some("spec-wrong".to_owned());
    request.active_work_item_path = Some("millrace-agents/specs/active/spec-wrong.md".to_owned());

    let error = request.validate().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("stage builder does not allow active_work_item_kind spec")
    );

    let mut recon_request = sample_request(temp.path());
    recon_request.plane = Plane::Planning;
    recon_request.stage = StageName::Recon;
    recon_request.node_id = "recon".to_owned();
    recon_request.stage_kind_id = "recon".to_owned();
    recon_request.running_status_marker = "RECON_RUNNING".to_owned();
    recon_request.legal_terminal_markers.clear();
    recon_request.allowed_result_classes_by_outcome = Default::default();
    recon_request.summary_status_path = "millrace-agents/state/planning_status.md".to_owned();
    let error = recon_request.validate().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("stage recon does not allow active_work_item_kind task")
    );
}

#[test]
fn python_v0_17_4_stage_run_request_preserves_learning_no_op_allowed_policy() {
    let temp = TempDir::new().unwrap();
    let request = sample_learning_request(temp.path(), StageName::Analyst);

    let serialized = serde_json::to_value(&request).unwrap();
    assert_eq!(serialized["plane"], "learning");
    assert_eq!(serialized["stage"], "analyst");
    assert_eq!(serialized["legal_terminal_markers"][1], "### ANALYST_NOOP");
    assert_eq!(
        serialized["allowed_result_classes_by_outcome"]["ANALYST_NOOP"],
        json!(["no_op"])
    );
    assert_eq!(
        serialized["allowed_result_classes_by_outcome"]["ANALYST_COMPLETE"],
        json!(["success"])
    );
    assert_eq!(
        serialized["allowed_result_classes_by_outcome"]["BLOCKED"],
        json!(["blocked", "recoverable_failure"])
    );

    let decoded = StageRunRequest::from_json_value(serialized).unwrap();
    assert_eq!(
        decoded
            .allowed_result_classes_by_outcome
            .result_classes_for("ANALYST_NOOP"),
        Some(&[ResultClass::NoOp][..])
    );
    let context_lines = render_stage_request_context_lines(&decoded);
    assert!(context_lines.contains(&"- ANALYST_NOOP: no_op".to_owned()));
}

#[test]
fn shared_prompt_renderer_persists_stage_request_context() {
    let temp = TempDir::new().unwrap();
    let mut request = sample_request(temp.path());
    request.legal_terminal_markers = vec!["### BUILDER_COMPLETE".to_owned()];
    request.allowed_result_classes_by_outcome =
        millrace_ai::AllowedResultClassesByOutcome::new(vec![
            millrace_ai::AllowedResultClassPolicy {
                outcome: "BUILDER_COMPLETE".to_owned(),
                result_classes: vec![ResultClass::Success],
            },
        ]);
    request.validate().unwrap();

    let prompt = build_stage_prompt(&request);
    assert!(prompt.contains("Stage Request Context:"));
    assert!(prompt.contains("Entrypoint Contract ID: builder.contract.v1"));
    assert!(prompt.contains("Active Work Item: task task-001"));
    assert!(prompt.contains("Thinking Level: medium"));
    assert!(prompt.contains("Required Skill Paths:"));
    assert!(prompt.contains("- millrace-agents/skills/stage/execution/builder-core/SKILL.md"));
    assert!(prompt.contains("Legal markers for this stage: `### BUILDER_COMPLETE`."));
    assert!(!prompt.contains("### TOKEN"));

    let prompt_path = write_stage_prompt_artifact(&request).unwrap();
    assert_eq!(prompt_path, runner_prompt_path(&request));
    assert_eq!(fs::read_to_string(prompt_path).unwrap(), prompt);
}

#[test]
fn runner_artifact_contracts_capture_invocation_completion_and_process_evidence() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path());
    let prompt_path = write_stage_prompt_artifact(&request).unwrap();
    let mut env_delta = RunnerEnvironmentDelta::default();
    env_delta
        .set
        .insert("MILLRACE_TEST_ENV".to_owned(), "1".to_owned());
    env_delta.unset.push("NO_PROXY".to_owned());
    let command = vec!["codex".to_owned(), "exec".to_owned()];
    let emitted_at = timestamp("2026-04-28T20:11:00Z");

    let invocation = invocation_artifact_from_request(
        &request,
        "codex_cli",
        command.clone(),
        "/tmp/workspace",
        env_delta.clone(),
        prompt_path.display().to_string(),
        emitted_at.clone(),
    )
    .unwrap();
    assert_eq!(invocation.kind, "runner_invocation");
    assert_eq!(invocation.request_kind, RequestKind::ActiveWorkItem);
    assert_eq!(invocation.runner_name, "codex_cli");
    assert_eq!(invocation.thinking_level.as_deref(), Some("medium"));
    assert_eq!(invocation.environment_delta.set["MILLRACE_TEST_ENV"], "1");

    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"))
            .unwrap();
    let raw_result = runner.run(&request).unwrap();
    let completion_context = RunnerCompletionArtifactContext::new(
        "codex_cli",
        command.clone(),
        "/tmp/workspace",
        env_delta.clone(),
        Some(prompt_path.display().to_string()),
        emitted_at.clone(),
    )
    .with_notes(vec!["runner completed".to_owned()]);
    let completion =
        completion_artifact_from_raw_result(&request, &raw_result, completion_context).unwrap();
    assert_eq!(completion.kind, "runner_completion");
    assert_eq!(completion.thinking_level.as_deref(), Some("medium"));
    assert_eq!(completion.duration_seconds, 1.0);
    assert_eq!(completion.exit_code, Some(0));
    assert!(!completion.timed_out);
    assert_eq!(completion.stdout_path, raw_result.stdout_path);
    assert_eq!(completion.notes, vec!["runner completed"]);

    let invocation_path = temp.path().join("runner_invocation.request-001.json");
    let completion_path = temp.path().join("runner_completion.request-001.json");
    write_runner_invocation(&invocation_path, &invocation).unwrap();
    write_runner_completion(&completion_path, &completion).unwrap();
    let invocation_json: Value =
        serde_json::from_str(&fs::read_to_string(invocation_path).unwrap()).unwrap();
    let completion_json: Value =
        serde_json::from_str(&fs::read_to_string(completion_path).unwrap()).unwrap();
    assert_eq!(invocation_json["command"][0], "codex");
    assert_eq!(invocation_json["thinking_level"], "medium");
    assert_eq!(completion_json["thinking_level"], "medium");
    assert_eq!(invocation_json["cwd"], "/tmp/workspace");
    assert_eq!(
        completion_json["prompt_path"],
        prompt_path.display().to_string()
    );
    assert_eq!(completion_json["environment_delta"]["unset"][0], "NO_PROXY");

    let mut process_result = ProcessExecutionResult::new(
        command,
        "/tmp/workspace",
        env_delta,
        ProcessExitKind::Timeout,
        Some(124),
        timestamp("2026-04-28T20:11:00Z"),
        timestamp("2026-04-28T20:11:03Z"),
    )
    .unwrap();
    process_result.stderr_path = Some("runner_stderr.request-001.txt".to_owned());
    process_result.event_log_path = Some("runner_events.request-001.jsonl".to_owned());
    process_result.notes.push("timeout captured".to_owned());
    process_result.validate().unwrap();
    let process_json = serde_json::to_value(process_result).unwrap();
    assert_eq!(process_json["kind"], "process_execution_result");
    assert_eq!(process_json["exit_kind"], "timeout");
    assert_eq!(process_json["timed_out"], true);
    assert_eq!(process_json["exit_code"], 124);
    assert_eq!(
        process_json["event_log_path"],
        "runner_events.request-001.jsonl"
    );

    let _: RunnerInvocationArtifact = serde_json::from_value(invocation_json).unwrap();
    let _: RunnerCompletionArtifact = serde_json::from_value(completion_json).unwrap();
}

#[test]
fn runner_registry_and_dispatcher_resolve_in_python_compatible_order() {
    let temp = TempDir::new().unwrap();
    let mut request = sample_request(temp.path());
    request.runner_name = Some("request_runner".to_owned());
    request.validate().unwrap();

    let mut registry = RunnerRegistry::new();
    registry
        .register(
            "codex_cli",
            StaticTokenRunner {
                runner_name: "codex_cli",
                terminal_marker: "### BLOCKED",
            },
        )
        .unwrap();
    registry
        .register(
            "default_runner",
            StaticTokenRunner {
                runner_name: "default_runner",
                terminal_marker: "### BUILDER_COMPLETE",
            },
        )
        .unwrap();
    registry
        .register(
            "request_runner",
            StaticTokenRunner {
                runner_name: "request_runner",
                terminal_marker: "### BUILDER_COMPLETE",
            },
        )
        .unwrap();
    let dispatcher = StageRunnerDispatcher::with_default_runner(registry, "default_runner");

    assert_eq!(dispatcher.resolve_runner_name(&request), "request_runner");
    let raw = dispatcher.run(&request).unwrap();
    assert_eq!(raw.runner_name, "request_runner");

    request.runner_name = None;
    request.validate().unwrap();
    assert_eq!(dispatcher.resolve_runner_name(&request), "default_runner");
    let raw = dispatcher.run(&request).unwrap();
    assert_eq!(raw.runner_name, "default_runner");

    let mut registry = RunnerRegistry::new();
    registry
        .register(
            "codex_cli",
            StaticTokenRunner {
                runner_name: "codex_cli",
                terminal_marker: "### BUILDER_COMPLETE",
            },
        )
        .unwrap();
    let fallback_dispatcher = StageRunnerDispatcher::new(registry);
    assert_eq!(
        fallback_dispatcher.resolve_runner_name(&request),
        StageRunnerDispatcher::fallback_runner_name()
    );
    let raw = fallback_dispatcher.run(&request).unwrap();
    assert_eq!(raw.runner_name, "codex_cli");
}

#[test]
fn runner_registry_reports_duplicate_and_unknown_adapter_names() {
    let temp = TempDir::new().unwrap();
    let mut request = sample_request(temp.path());
    request.runner_name = Some("missing_runner".to_owned());
    request.validate().unwrap();

    let mut registry = RunnerRegistry::new();
    registry
        .register(
            "codex_cli",
            StaticTokenRunner {
                runner_name: "codex_cli",
                terminal_marker: "### BUILDER_COMPLETE",
            },
        )
        .unwrap();
    let duplicate = registry
        .register(
            "codex_cli",
            StaticTokenRunner {
                runner_name: "duplicate",
                terminal_marker: "### BLOCKED",
            },
        )
        .unwrap_err();
    assert!(duplicate.to_string().contains("duplicate"));

    let dispatcher = StageRunnerDispatcher::new(registry);
    let unknown = dispatcher.run(&request).unwrap_err();
    match unknown {
        RunnerError::UnknownRunner {
            requested,
            available,
        } => {
            assert_eq!(requested, "missing_runner");
            assert_eq!(available, vec!["codex_cli"]);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn fake_runner_success_normalizes_to_stage_result_envelope() {
    let temp = TempDir::new().unwrap();
    let troubleshoot_report_path = temp.path().join("troubleshoot_report.md");
    fs::write(&troubleshoot_report_path, "troubleshoot summary\n").unwrap();
    let request = sample_request(temp.path());
    let token_usage = TokenUsage {
        input_tokens: 10,
        cached_input_tokens: 2,
        output_tokens: 5,
        thinking_tokens: 1,
        total_tokens: 16,
    };
    let runner = FakeRunner::with_default(
        FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE")
            .with_token_usage(token_usage.clone()),
    )
    .unwrap();

    let raw_result = runner.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_eq!(
        envelope.terminal_result,
        TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete)
    );
    assert_eq!(envelope.result_class, ResultClass::Success);
    assert!(envelope.success);
    assert_eq!(
        envelope.report_artifact.as_deref(),
        Some(troubleshoot_report_path.to_str().unwrap())
    );
    assert_eq!(envelope.token_usage, Some(token_usage));
    assert_eq!(envelope.stdout_path, raw_result.stdout_path);
    assert_eq!(envelope.duration_seconds, 1.0);
    assert_eq!(
        envelope.metadata["normalization_source"],
        "stdout_terminal_token"
    );
    assert_eq!(failure_class(&Value::Object(envelope.metadata)), None);
}

#[test]
fn python_v0_17_4_learning_noop_terminal_normalizes_to_non_success_noop_result() {
    let temp = TempDir::new().unwrap();
    let request = sample_learning_request(temp.path(), StageName::Analyst);
    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### ANALYST_NOOP")).unwrap();

    let raw_result = runner.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_eq!(
        envelope.terminal_result,
        TerminalResult::Learning(LearningTerminalResult::AnalystNoop)
    );
    assert_eq!(envelope.result_class, ResultClass::NoOp);
    assert!(!envelope.success);
    assert!(!envelope.retryable);
    assert_eq!(envelope.work_item_kind, WorkItemKind::LearningRequest);
    assert_eq!(envelope.work_item_id, "learn-001");
}

#[test]
fn python_v0_17_4_learning_noop_rejects_mismatched_terminal_result_class_pairs() {
    let temp = TempDir::new().unwrap();
    let request = sample_learning_request(temp.path(), StageName::Analyst);

    let noop_as_success = FakeRunner::with_default(FakeRunnerResult::structured_terminal_result(
        "ANALYST_NOOP",
        Some(ResultClass::Success),
    ))
    .unwrap();
    let raw_result = noop_as_success.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();
    assert_eq!(envelope.result_class, ResultClass::RecoverableFailure);
    assert_eq!(
        envelope.terminal_result,
        TerminalResult::Learning(LearningTerminalResult::Blocked)
    );
    assert_eq!(
        envelope.metadata["failure_class"],
        "illegal_terminal_result"
    );

    let complete_as_noop = FakeRunner::with_default(FakeRunnerResult::structured_terminal_result(
        "ANALYST_COMPLETE",
        Some(ResultClass::NoOp),
    ))
    .unwrap();
    let raw_result = complete_as_noop.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();
    assert_eq!(envelope.result_class, ResultClass::RecoverableFailure);
    assert_eq!(
        envelope.terminal_result,
        TerminalResult::Learning(LearningTerminalResult::Blocked)
    );
    assert_eq!(
        envelope.metadata["failure_class"],
        "illegal_terminal_result"
    );
}

#[test]
fn malformed_stdout_terminal_output_becomes_typed_recoverable_failure() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path());
    let runner = FakeRunner::with_default(FakeRunnerResult::malformed_stdout(
        "### BUILDER_COMPLETE\n### BLOCKED\n",
    ))
    .unwrap();

    let raw_result = runner.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_eq!(envelope.result_class, ResultClass::RecoverableFailure);
    assert!(envelope.retryable);
    assert_eq!(
        envelope.metadata["failure_class"],
        "conflicting_terminal_results"
    );
    assert_eq!(envelope.detected_marker.as_deref(), Some("### BLOCKED"));
}

#[test]
fn illegal_stdout_terminal_marker_becomes_valid_recoverable_failure() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path());
    let runner =
        FakeRunner::with_default(FakeRunnerResult::malformed_stdout("### NOT_A_TERMINAL\n"))
            .unwrap();

    let raw_result = runner.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_eq!(envelope.result_class, ResultClass::RecoverableFailure);
    assert_eq!(envelope.terminal_result.as_str(), "BLOCKED");
    assert_eq!(
        envelope.metadata["failure_class"],
        "illegal_terminal_result"
    );
    assert_eq!(
        envelope.metadata["raw_detected_marker"],
        "### NOT_A_TERMINAL"
    );
    assert_eq!(envelope.detected_marker, None);
}

#[test]
fn missing_terminal_output_becomes_typed_recoverable_failure() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path());
    let runner = FakeRunner::with_default(FakeRunnerResult::missing_terminal_output()).unwrap();

    let raw_result = runner.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_eq!(raw_result.thinking_level.as_deref(), Some("medium"));
    assert_eq!(envelope.thinking_level.as_deref(), Some("medium"));
    assert_eq!(envelope.metadata["thinking_level"], json!("medium"));
    assert_eq!(envelope.result_class, ResultClass::RecoverableFailure);
    assert_eq!(
        envelope.metadata["failure_class"],
        "missing_terminal_result"
    );
    assert!(envelope.stdout_path.is_none());
}

#[test]
fn structured_terminal_rejects_illegal_result_class_combination() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path());
    let runner = FakeRunner::with_default(FakeRunnerResult::structured_terminal_result(
        "BUILDER_COMPLETE",
        Some(ResultClass::Blocked),
    ))
    .unwrap();

    let raw_result = runner.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_eq!(envelope.result_class, ResultClass::RecoverableFailure);
    assert_eq!(
        envelope.metadata["failure_class"],
        "illegal_terminal_result"
    );
    assert_eq!(envelope.metadata["normalization_source"], "failure");
    assert!(envelope.terminal_result.as_str() == "BLOCKED");
}

#[test]
fn fake_runner_selection_is_deterministic() {
    let temp = TempDir::new().unwrap();
    let mut request = sample_request(temp.path());
    request.request_id = "request-overrides-node".to_owned();
    request.validate().unwrap();
    let config = FakeRunnerConfig::new(FakeRunnerResult::missing_terminal_output())
        .unwrap()
        .with_stage_result(
            StageName::Builder,
            FakeRunnerResult::terminal_marker("### BLOCKED"),
        )
        .with_node_result(
            "builder-node",
            FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"),
        )
        .with_request_result(
            "request-overrides-node",
            FakeRunnerResult::terminal_marker("### BLOCKED"),
        );
    let runner = FakeRunner::new(config);

    let raw_result = runner.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_eq!(envelope.terminal_result.as_str(), "BLOCKED");
    assert_eq!(envelope.result_class, ResultClass::Blocked);
    assert!(!envelope.success);
}

#[test]
fn once_startup_requires_initialized_workspace_without_creating_it() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("workspace");

    let error = startup_runtime_once(&root, startup_options("uninitialized")).unwrap_err();

    assert!(error.to_string().contains("workspace is not initialized"));
    assert!(!root.join("millrace-agents").exists());
}

#[test]
fn once_startup_compiles_loads_state_projects_snapshot_and_close_releases_lock() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-001")).unwrap();

    let mut paused = load_snapshot(&paths).unwrap();
    paused.paused = true;
    save_snapshot(&paths, &paused).unwrap();

    let mut options = startup_options("startup-success");
    options.requested_mode_id = Some("standard_plain".to_owned());
    let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();

    assert_eq!(session.snapshot.runtime_mode, RuntimeMode::Once);
    assert!(session.snapshot.process_running);
    assert!(session.snapshot.paused);
    assert_eq!(session.snapshot.active_mode_id, "default_codex");
    assert_eq!(
        session.snapshot.compiled_plan_id,
        session.compiled_plan.compiled_plan_id
    );
    assert_eq!(
        session.snapshot.compiled_plan_path,
        "millrace-agents/state/compiled_plan.json"
    );
    assert_eq!(session.snapshot.queue_depth_execution, 1);
    assert_eq!(session.reconciliation.signals, Vec::new());
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Active
    );

    assert!(session.close().unwrap());
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Absent
    );
    assert!(!load_snapshot(&paths).unwrap().process_running);
}

#[test]
fn once_startup_lock_contention_preserves_compiler_and_state_artifacts() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();

    let session = startup_runtime_once_for_paths(&paths, startup_options("initial-owner")).unwrap();
    session.finish().unwrap();

    let compiled_before = fs::read(&paths.compiled_plan_file).unwrap();
    let diagnostics_before = fs::read(&paths.compile_diagnostics_file).unwrap();
    let snapshot_before = fs::read(&paths.runtime_snapshot_file).unwrap();
    let counters_before = fs::read(&paths.recovery_counters_file).unwrap();

    acquire_runtime_ownership_lock_with_options(&paths, startup_lock_options("external-owner"))
        .unwrap();

    let error = startup_runtime_once_for_paths(&paths, startup_options("contender")).unwrap_err();
    assert!(matches!(error, RuntimeStartupError::RuntimeLock(_)));
    assert_eq!(
        fs::read(&paths.compiled_plan_file).unwrap(),
        compiled_before
    );
    assert_eq!(
        fs::read(&paths.compile_diagnostics_file).unwrap(),
        diagnostics_before
    );
    assert_eq!(
        fs::read(&paths.runtime_snapshot_file).unwrap(),
        snapshot_before
    );
    assert_eq!(
        fs::read(&paths.recovery_counters_file).unwrap(),
        counters_before
    );
}

#[test]
fn once_startup_compile_failure_preserves_artifacts_and_releases_lock() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();

    let session =
        startup_runtime_once_for_paths(&paths, startup_options("initial-compile")).unwrap();
    session.finish().unwrap();

    let compiled_before = fs::read(&paths.compiled_plan_file).unwrap();
    let diagnostics_before = fs::read(&paths.compile_diagnostics_file).unwrap();
    let snapshot_before = fs::read(&paths.runtime_snapshot_file).unwrap();
    let counters_before = fs::read(&paths.recovery_counters_file).unwrap();
    fs::write(
        &paths.runtime_config_file,
        "[runtime]\ndefault_mode = 'missing_mode'\n",
    )
    .unwrap();

    let error =
        startup_runtime_once_for_paths(&paths, startup_options("compile-fails")).unwrap_err();

    assert!(matches!(
        error,
        RuntimeStartupError::MissingActiveCompiledPlan { .. }
    ));
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Absent
    );
    assert_eq!(
        fs::read(&paths.compiled_plan_file).unwrap(),
        compiled_before
    );
    assert_eq!(
        fs::read(&paths.compile_diagnostics_file).unwrap(),
        diagnostics_before
    );
    assert_eq!(
        fs::read(&paths.runtime_snapshot_file).unwrap(),
        snapshot_before
    );
    assert_eq!(
        fs::read(&paths.recovery_counters_file).unwrap(),
        counters_before
    );
}

#[test]
fn once_startup_reconciles_stale_execution_active_state_to_recovery_stage() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-stale")).unwrap();
    assert!(queue.claim_next_execution_task(None).unwrap().is_some());

    let mut stale = load_snapshot(&paths).unwrap();
    stale.process_running = false;
    stale.active_plane = Some(Plane::Execution);
    stale.active_stage = Some(StageName::Checker);
    stale.active_node_id = Some("checker".to_owned());
    stale.active_stage_kind_id = Some("checker".to_owned());
    stale.active_run_id = Some("run-stale".to_owned());
    stale.active_work_item_kind = Some(WorkItemKind::Task);
    stale.active_work_item_id = Some("task-stale".to_owned());
    stale.active_since = Some(timestamp(STARTUP_NOW));
    stale.active_runs_by_plane.clear();
    save_snapshot(&paths, &stale).unwrap();

    let mut options = startup_options("reconcile-owner");
    options.recovery_run_id = Some("run-recovery-fixed".to_owned());
    let session = startup_runtime_once_for_paths(&paths, options).unwrap();

    assert_eq!(
        session.reconciliation.signals[0].code,
        "stale_active_ownership"
    );
    assert_eq!(
        session.snapshot.active_stage,
        Some(StageName::Troubleshooter)
    );
    assert_eq!(
        session.snapshot.active_run_id.as_deref(),
        Some("run-recovery-fixed")
    );
    assert_eq!(
        session.snapshot.current_failure_class.as_deref(),
        Some("stale_active_ownership")
    );
    assert_eq!(session.snapshot.troubleshoot_attempt_count, 1);

    let counters = load_recovery_counters(&paths).unwrap();
    assert_eq!(counters.entries.len(), 1);
    assert_eq!(counters.entries[0].work_item_id, "task-stale");
    assert_eq!(counters.entries[0].troubleshoot_attempt_count, 1);

    session.finish().unwrap();
}

#[test]
fn once_startup_detects_learning_active_state_without_repairing_queue() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_learning_request(&learning_request_document("learn-stale"))
        .unwrap();
    assert!(queue.claim_next_learning_request().unwrap().is_some());

    let session =
        startup_runtime_once_for_paths(&paths, startup_options("learning-detect")).unwrap();

    assert!(session.reconciliation.learning.is_stale);
    assert!(
        session
            .reconciliation
            .learning
            .reasons
            .contains(&"active_without_snapshot".to_owned())
    );
    assert!(
        paths
            .learning_requests_active_dir
            .join("learn-stale.md")
            .is_file()
    );
    assert!(
        !paths
            .learning_requests_queue_dir
            .join("learn-stale.md")
            .exists()
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_claims_planning_before_execution_and_marks_stage_started() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-ready")).unwrap();
    queue.enqueue_spec(&spec_document("spec-ready")).unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-planning")).unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-planning", "request-planning"),
    )
    .unwrap();

    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::StageRequestReady);
    let request = outcome.stage_request.unwrap();
    assert_eq!(request.request_id, "request-planning");
    assert_eq!(request.run_id, "run-planning");
    assert_eq!(request.plane, Plane::Planning);
    assert_eq!(request.stage, StageName::Planner);
    assert_eq!(request.request_kind, RequestKind::ActiveWorkItem);
    assert_eq!(request.active_work_item_kind, Some(WorkItemKind::Spec));
    assert_eq!(request.active_work_item_id.as_deref(), Some("spec-ready"));
    assert!(
        request
            .active_work_item_path
            .unwrap()
            .ends_with("spec-ready.md")
    );
    assert_eq!(load_planning_status(&paths).unwrap(), "### PLANNER_RUNNING");
    assert!(paths.specs_active_dir.join("spec-ready.md").is_file());
    assert!(paths.tasks_queue_dir.join("task-ready.md").is_file());
    assert_eq!(
        session
            .snapshot
            .active_runs_by_plane
            .get(&Plane::Planning)
            .unwrap()
            .running_status_marker
            .as_deref(),
        Some("PLANNER_RUNNING")
    );
    let events = runtime_events(&paths);
    assert_eq!(events[0]["event_type"], "stage_started");
    assert_eq!(events[0]["data"]["request_id"], "request-planning");
    assert_eq!(events[0]["data"]["stage"], "planner");

    session.finish().unwrap();
}

#[test]
fn serial_tick_claims_probe_for_recon_stage_request_metadata() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_probe(&probe_document_for_recon("probe-ready"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-recon-activate")).unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-recon-activate", "request-recon-activate"),
    )
    .unwrap();

    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::StageRequestReady);
    let request = outcome.stage_request.unwrap();
    assert_eq!(request.request_id, "request-recon-activate");
    assert_eq!(request.run_id, "run-recon-activate");
    assert_eq!(request.plane, Plane::Planning);
    assert_eq!(request.stage, StageName::Recon);
    assert_eq!(request.node_id, "recon");
    assert_eq!(request.stage_kind_id, "recon");
    assert_eq!(request.request_kind, RequestKind::ActiveWorkItem);
    assert_eq!(request.active_work_item_kind, Some(WorkItemKind::Probe));
    assert_eq!(request.active_work_item_id.as_deref(), Some("probe-ready"));
    assert!(
        request
            .active_work_item_path
            .as_deref()
            .unwrap()
            .ends_with("millrace-agents/probes/active/probe-ready.md")
    );
    assert_eq!(request.running_status_marker, "RECON_RUNNING");
    assert_eq!(
        request.legal_terminal_markers,
        vec![
            "### RECON_TO_EXECUTION".to_owned(),
            "### RECON_TO_PLANNING".to_owned(),
            "### RECON_NOOP".to_owned(),
            "### RECON_BLOCKED".to_owned(),
            "### BLOCKED".to_owned(),
        ]
    );
    assert!(
        request
            .required_skill_paths
            .iter()
            .any(|path| path.ends_with("skills/stage/planning/recon-core/SKILL.md"))
    );
    assert!(Path::new(&request.run_dir).is_dir());
    assert!(
        request
            .preferred_troubleshoot_report_path
            .as_deref()
            .unwrap()
            .starts_with(&request.run_dir)
    );
    assert_eq!(load_planning_status(&paths).unwrap(), "### RECON_RUNNING");
    assert!(paths.probes_active_dir.join("probe-ready.md").is_file());
    assert!(!paths.probes_queue_dir.join("probe-ready.md").exists());

    session.finish().unwrap();
}

#[test]
fn serial_tick_claims_execution_when_planning_has_no_work() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-build"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-execution")).unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-execution", "request-execution"),
    )
    .unwrap();

    let request = outcome.stage_request.unwrap();
    assert_eq!(request.plane, Plane::Execution);
    assert_eq!(request.stage, StageName::Builder);
    assert_eq!(request.node_id, "builder");
    assert_eq!(request.stage_kind_id, "builder");
    assert_eq!(request.active_work_item_kind, Some(WorkItemKind::Task));
    assert_eq!(request.active_work_item_id.as_deref(), Some("task-build"));
    assert_eq!(request.running_status_marker, "BUILDER_RUNNING");
    assert_eq!(
        load_execution_status(&paths).unwrap(),
        "### BUILDER_RUNNING"
    );
    assert!(paths.runs_dir.join("run-execution").is_dir());

    session.finish().unwrap();
}

#[test]
fn recon_to_execution_persists_packet_marks_probe_done_enqueues_task_and_traces() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_probe(&probe_document_for_recon("probe-001"))
        .unwrap();
    let runner = ReconArtifactRunner::new(
        "### RECON_TO_EXECUTION",
        recon_packet_for("probe-001", ReconDecision::ToExecution),
        Some(ReconGeneratedArtifact::Task(generated_probe_task(
            "task-from-probe",
        ))),
    );
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-recon-exec")).unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-recon-exec", "request-recon-exec"),
        &runner,
    )
    .unwrap();

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Idle);
    assert_eq!(decision.reason, "recon_to_execution");
    assert!(paths.probes_done_dir.join("probe-001.md").is_file());
    assert!(!paths.probes_active_dir.join("probe-001.md").exists());
    assert!(paths.recon_packets_dir.join("recon-probe-001.md").is_file());

    let task_path = paths.tasks_queue_dir.join("task-from-probe.md");
    assert!(task_path.is_file());
    let task = parse_task_document(&fs::read_to_string(&task_path).unwrap()).unwrap();
    assert_eq!(task.root_intake_kind, Some(RootIntakeKind::Probe));
    assert_eq!(task.root_intake_id.as_deref(), Some("probe-001"));
    assert_eq!(task.root_idea_id.as_deref(), Some("idea-from-probe"));
    assert_eq!(task.root_spec_id.as_deref(), Some("spec-from-probe-root"));
    assert!(
        task.references
            .contains(&"millrace-agents/probes/active/probe-001.md".to_owned())
    );
    assert!(
        task.references
            .contains(&"millrace-agents/recon/packets/recon-probe-001.md".to_owned())
    );

    let trace = inspect_run_trace(paths.runs_dir.join("run-recon-exec")).unwrap();
    assert_eq!(trace.status.as_str(), "complete");
    assert_eq!(trace.edges[0].outcome, "RECON_TO_EXECUTION");
    assert_eq!(trace.edges[0].edge_kind, "idle");
    assert_eq!(
        trace.edges[0].decision_reason.as_deref(),
        Some("recon_to_execution")
    );
    assert_eq!(trace.edges[0].spawned_work.len(), 1);
    assert_eq!(trace.edges[0].spawned_work[0].kind.as_str(), "task");
    assert_eq!(trace.edges[0].spawned_work[0].item_id, "task-from-probe");
    assert_eq!(
        trace.edges[0].spawned_work[0].reason.as_deref(),
        Some("recon_to_execution")
    );

    session.finish().unwrap();
}

#[test]
fn recon_to_planning_persists_packet_marks_probe_done_enqueues_spec() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_probe(&probe_document_for_recon("probe-001"))
        .unwrap();
    let runner = ReconArtifactRunner::new(
        "### RECON_TO_PLANNING",
        recon_packet_for("probe-001", ReconDecision::ToPlanning),
        Some(ReconGeneratedArtifact::Spec(generated_probe_spec(
            "spec-from-probe",
        ))),
    );
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-recon-plan")).unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-recon-plan", "request-recon-plan"),
        &runner,
    )
    .unwrap();

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Idle);
    assert_eq!(decision.reason, "recon_to_planning");
    assert!(paths.probes_done_dir.join("probe-001.md").is_file());
    assert!(paths.recon_packets_dir.join("recon-probe-001.md").is_file());

    let spec_path = paths.specs_queue_dir.join("spec-from-probe.md");
    assert!(spec_path.is_file());
    let spec = parse_spec_document(&fs::read_to_string(&spec_path).unwrap()).unwrap();
    assert_eq!(spec.source_type, SpecSourceType::Probe);
    assert_eq!(spec.source_id.as_deref(), Some("probe-001"));
    assert_eq!(spec.root_intake_kind, Some(RootIntakeKind::Probe));
    assert_eq!(spec.root_intake_id.as_deref(), Some("probe-001"));
    assert_eq!(spec.root_idea_id.as_deref(), Some("idea-from-probe"));
    assert_eq!(spec.root_spec_id.as_deref(), Some("spec-from-probe"));
    assert!(
        spec.references
            .contains(&"millrace-agents/probes/active/probe-001.md".to_owned())
    );
    assert!(
        spec.references
            .contains(&"millrace-agents/recon/packets/recon-probe-001.md".to_owned())
    );

    let trace = inspect_run_trace(paths.runs_dir.join("run-recon-plan")).unwrap();
    assert_eq!(trace.status.as_str(), "complete");
    assert_eq!(trace.edges[0].outcome, "RECON_TO_PLANNING");
    assert_eq!(trace.edges[0].spawned_work.len(), 1);
    assert_eq!(trace.edges[0].spawned_work[0].kind.as_str(), "spec");
    assert_eq!(trace.edges[0].spawned_work[0].item_id, "spec-from-probe");

    session.finish().unwrap();
}

#[test]
fn recon_noop_persists_packet_marks_probe_done_without_spawned_work() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_probe(&probe_document_for_recon("probe-001"))
        .unwrap();
    let runner = ReconArtifactRunner::new(
        "### RECON_NOOP",
        recon_packet_for("probe-001", ReconDecision::Noop),
        None,
    );
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-recon-noop")).unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-recon-noop", "request-recon-noop"),
        &runner,
    )
    .unwrap();

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Idle);
    assert_eq!(decision.reason, "recon_noop");
    assert!(paths.probes_done_dir.join("probe-001.md").is_file());
    assert!(paths.recon_packets_dir.join("recon-probe-001.md").is_file());
    assert_eq!(fs::read_dir(&paths.tasks_queue_dir).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&paths.specs_queue_dir).unwrap().count(), 0);

    let trace = inspect_run_trace(paths.runs_dir.join("run-recon-noop")).unwrap();
    assert_eq!(trace.status.as_str(), "complete");
    assert_eq!(trace.edges[0].outcome, "RECON_NOOP");
    assert_eq!(trace.edges[0].spawned_work.len(), 0);
    assert_eq!(
        trace.edges[0].decision_reason.as_deref(),
        Some("recon_noop")
    );

    session.finish().unwrap();
}

#[test]
fn recon_blocked_persists_packet_marks_probe_blocked() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_probe(&probe_document_for_recon("probe-001"))
        .unwrap();
    let runner = ReconArtifactRunner::new(
        "### RECON_BLOCKED",
        recon_packet_for("probe-001", ReconDecision::Blocked),
        None,
    );
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-recon-blocked")).unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-recon-blocked", "request-recon-blocked"),
        &runner,
    )
    .unwrap();

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Blocked);
    assert_eq!(decision.reason, "recon_blocked");
    assert_eq!(decision.failure_class.as_deref(), Some("recon_blocked"));
    assert!(paths.probes_blocked_dir.join("probe-001.md").is_file());
    assert!(paths.recon_packets_dir.join("recon-probe-001.md").is_file());
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.active_runs_by_plane.is_empty());
    assert_eq!(
        snapshot.current_failure_class.as_deref(),
        Some("recon_blocked")
    );
    assert_eq!(load_planning_status(&paths).unwrap(), "### RECON_BLOCKED");

    let trace = inspect_run_trace(paths.runs_dir.join("run-recon-blocked")).unwrap();
    assert_eq!(trace.status.as_str(), "blocked");
    assert_eq!(trace.edges[0].outcome, "RECON_BLOCKED");
    assert_eq!(trace.edges[0].edge_kind, "blocked");

    session.finish().unwrap();
}

#[test]
fn recon_generic_blocked_persists_packet_marks_probe_blocked() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_probe(&probe_document_for_recon("probe-001"))
        .unwrap();
    let runner = ReconArtifactRunner::new(
        "### BLOCKED",
        recon_packet_for("probe-001", ReconDecision::Blocked),
        None,
    );
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-recon-generic-blocked"))
            .unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-recon-generic-blocked", "request-recon-generic-blocked"),
        &runner,
    )
    .unwrap();

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Blocked);
    assert_eq!(decision.reason, "recon_blocked");
    assert_eq!(decision.failure_class.as_deref(), Some("recon_blocked"));
    assert!(paths.probes_blocked_dir.join("probe-001.md").is_file());
    assert!(paths.recon_packets_dir.join("recon-probe-001.md").is_file());
    let trace = inspect_run_trace(paths.runs_dir.join("run-recon-generic-blocked")).unwrap();
    assert_eq!(trace.status.as_str(), "blocked");
    assert_eq!(trace.edges[0].outcome, "BLOCKED");
    assert_eq!(
        trace.edges[0].decision_reason.as_deref(),
        Some("recon_blocked")
    );

    session.finish().unwrap();
}

#[test]
fn recon_packet_decision_mismatch_blocks_probe_with_invalid_handoff_evidence() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_probe(&probe_document_for_recon("probe-001"))
        .unwrap();
    let runner = ReconArtifactRunner::new(
        "### RECON_TO_EXECUTION",
        recon_packet_for("probe-001", ReconDecision::Noop),
        Some(ReconGeneratedArtifact::Task(generated_probe_task(
            "task-from-probe",
        ))),
    );
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-recon-mismatch")).unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-recon-mismatch", "request-recon-mismatch"),
        &runner,
    )
    .unwrap();

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Idle);
    assert_eq!(decision.next_stage, None);
    assert_eq!(outcome.runtime_error_context_path, None);
    assert!(!paths.probes_active_dir.join("probe-001.md").exists());
    assert!(!paths.probes_done_dir.join("probe-001.md").exists());
    assert!(paths.probes_blocked_dir.join("probe-001.md").is_file());
    assert!(!paths.recon_packets_dir.join("recon-probe-001.md").exists());
    assert!(!paths.tasks_queue_dir.join("task-from-probe.md").exists());
    assert_eq!(load_planning_status(&paths).unwrap(), "### BLOCKED");
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.active_runs_by_plane.is_empty());
    assert_eq!(
        snapshot.current_failure_class.as_deref(),
        Some("recon_handoff_invalid")
    );
    let context = RuntimeErrorContext::from_json_str(
        &fs::read_to_string(&paths.runtime_error_context_file).unwrap(),
    )
    .unwrap();
    assert_eq!(context.error_code.as_str(), "recon_handoff_invalid");
    assert_eq!(context.failed_stage, StageName::Recon);
    assert_eq!(context.repair_stage, StageName::Recon);
    assert_eq!(context.router_action.as_deref(), Some("idle"));
    assert!(
        context
            .exception_message
            .contains("recon packet decision must match terminal result")
    );
    let report = fs::read_to_string(
        paths
            .runs_dir
            .join("run-recon-mismatch/runtime_error_report.md"),
    )
    .unwrap();
    assert!(report.contains("Error-Code: recon_handoff_invalid"));
    assert!(
        !runtime_events(&paths)
            .iter()
            .any(|event| { event["event_type"] == "runtime_post_stage_recovery_scheduled" })
    );

    session.finish().unwrap();
}

#[test]
fn recon_generated_task_id_mismatch_blocks_probe_without_enqueuing_work() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_probe(&probe_document_for_recon("probe-001"))
        .unwrap();
    let runner = ReconArtifactRunner::new(
        "### RECON_TO_EXECUTION",
        recon_packet_for("probe-001", ReconDecision::ToExecution),
        Some(ReconGeneratedArtifact::Task(generated_probe_task(
            "task-other",
        ))),
    );
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-recon-id-mismatch")).unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-recon-id-mismatch", "request-recon-id-mismatch"),
        &runner,
    )
    .unwrap();

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Idle);
    assert_eq!(decision.next_stage, None);
    assert!(!paths.probes_active_dir.join("probe-001.md").exists());
    assert!(!paths.probes_done_dir.join("probe-001.md").exists());
    assert!(paths.probes_blocked_dir.join("probe-001.md").is_file());
    assert!(paths.recon_packets_dir.join("recon-probe-001.md").is_file());
    assert_eq!(fs::read_dir(&paths.tasks_queue_dir).unwrap().count(), 0);
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.active_runs_by_plane.is_empty());
    assert_eq!(
        snapshot.current_failure_class.as_deref(),
        Some("recon_handoff_invalid")
    );
    let context = RuntimeErrorContext::from_json_str(
        &fs::read_to_string(&paths.runtime_error_context_file).unwrap(),
    )
    .unwrap();
    assert_eq!(context.error_code.as_str(), "recon_handoff_invalid");
    assert_eq!(context.failed_stage, StageName::Recon);
    assert_eq!(context.repair_stage, StageName::Recon);
    assert!(
        context
            .exception_message
            .contains("generated task id must match recon packet emitted_task_id")
    );
    assert!(
        !runtime_events(&paths)
            .iter()
            .any(|event| { event["event_type"] == "runtime_post_stage_recovery_scheduled" })
    );

    session.finish().unwrap();
}

#[test]
fn recon_missing_generated_task_artifact_blocks_probe_without_enqueuing_work() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_probe(&probe_document_for_recon("probe-001"))
        .unwrap();
    let runner = ReconArtifactRunner::new(
        "### RECON_TO_EXECUTION",
        recon_packet_for("probe-001", ReconDecision::ToExecution),
        None,
    );
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-recon-missing-generated"))
            .unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(
            "run-recon-missing-generated",
            "request-recon-missing-generated",
        ),
        &runner,
    )
    .unwrap();

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Idle);
    assert!(!paths.probes_active_dir.join("probe-001.md").exists());
    assert!(paths.probes_blocked_dir.join("probe-001.md").is_file());
    assert!(paths.recon_packets_dir.join("recon-probe-001.md").is_file());
    assert_eq!(fs::read_dir(&paths.tasks_queue_dir).unwrap().count(), 0);
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.active_runs_by_plane.is_empty());
    assert_eq!(
        snapshot.current_failure_class.as_deref(),
        Some("recon_handoff_invalid")
    );
    let context = RuntimeErrorContext::from_json_str(
        &fs::read_to_string(&paths.runtime_error_context_file).unwrap(),
    )
    .unwrap();
    assert_eq!(context.error_code.as_str(), "recon_handoff_invalid");
    assert!(context.exception_message.contains("generated_task.md"));

    session.finish().unwrap();
}

#[test]
fn recon_malformed_packet_artifact_blocks_probe_without_planning_recovery() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_probe(&probe_document_for_recon("probe-001"))
        .unwrap();
    let runner = RawReconPacketRunner {
        marker: "### RECON_TO_EXECUTION",
        packet_text: "Decision: to_execution\n",
    };
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-recon-malformed")).unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-recon-malformed", "request-recon-malformed"),
        &runner,
    )
    .unwrap();

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Idle);
    assert!(!paths.probes_active_dir.join("probe-001.md").exists());
    assert!(paths.probes_blocked_dir.join("probe-001.md").is_file());
    assert!(!paths.recon_packets_dir.join("recon-probe-001.md").exists());
    assert_eq!(fs::read_dir(&paths.tasks_queue_dir).unwrap().count(), 0);
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.active_runs_by_plane.is_empty());
    assert_eq!(
        snapshot.current_failure_class.as_deref(),
        Some("recon_handoff_invalid")
    );
    let context = RuntimeErrorContext::from_json_str(
        &fs::read_to_string(&paths.runtime_error_context_file).unwrap(),
    )
    .unwrap();
    assert_eq!(context.error_code.as_str(), "recon_handoff_invalid");
    assert!(
        context
            .exception_message
            .contains("must start with a markdown H1 title")
    );
    assert!(
        !runtime_events(&paths)
            .iter()
            .any(|event| { event["event_type"] == "runtime_post_stage_recovery_scheduled" })
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_dispatches_fake_runner_persists_artifacts_and_routes_from_graph() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        "[runtime]\ndefault_mode = \"default_codex\"\n\n[stages.builder]\nthinking_level = \"high\"\n",
    )
    .unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-dispatch"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-dispatch")).unwrap();
    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"))
            .unwrap();
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-dispatch", "request-dispatch"),
        &runner,
    )
    .unwrap();

    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::StageDispatched);
    assert_eq!(outcome.reason, "stage_dispatched");
    assert!(outcome.stage_request_path.as_ref().unwrap().is_file());
    assert!(outcome.runner_raw_result_path.as_ref().unwrap().is_file());
    assert!(outcome.stage_result_path.as_ref().unwrap().is_file());
    assert!(outcome.terminal_marker_path.as_ref().unwrap().is_file());
    assert!(outcome.router_decision_path.as_ref().unwrap().is_file());
    assert_eq!(
        fs::read_to_string(outcome.terminal_marker_path.as_ref().unwrap()).unwrap(),
        "### BUILDER_COMPLETE\n"
    );
    let request = outcome.stage_request.as_ref().unwrap();
    assert_eq!(request.thinking_level.as_deref(), Some("high"));
    assert_eq!(request.model_reasoning_effort.as_deref(), Some("high"));
    let persisted_request = StageRunRequest::from_json_str(
        &fs::read_to_string(outcome.stage_request_path.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted_request.thinking_level.as_deref(), Some("high"));
    assert_eq!(
        outcome
            .runner_raw_result
            .as_ref()
            .unwrap()
            .thinking_level
            .as_deref(),
        Some("high")
    );

    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(stage_result.thinking_level.as_deref(), Some("high"));
    assert_eq!(stage_result.metadata["thinking_level"], json!("high"));
    assert_eq!(
        stage_result.terminal_result,
        TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete)
    );
    assert_eq!(stage_result.result_class, ResultClass::Success);
    assert_eq!(
        stage_result.metadata["stage_request_path"],
        outcome
            .stage_request_path
            .as_ref()
            .unwrap()
            .display()
            .to_string()
    );
    let persisted_stage_result = StageResultEnvelope::from_json_str(
        &fs::read_to_string(outcome.stage_result_path.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted_stage_result, *stage_result);

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::RunStage);
    assert_eq!(decision.next_stage, Some(StageName::Checker));
    assert_eq!(decision.next_node_id.as_deref(), Some("checker"));
    assert_eq!(decision.reason, "builder:BUILDER_COMPLETE");
    let router_json: Value =
        serde_json::from_str(&fs::read_to_string(outcome.router_decision_path.unwrap()).unwrap())
            .unwrap();
    assert_eq!(router_json["decision"]["action"], "run_stage");
    assert_eq!(router_json["decision"]["next_stage"], "checker");

    let snapshot = load_snapshot(&paths).unwrap();
    assert_eq!(
        snapshot.last_terminal_result,
        Some(TerminalResult::Execution(
            ExecutionTerminalResult::BuilderComplete
        ))
    );
    assert_eq!(
        snapshot.last_stage_result_path.as_deref(),
        Some("millrace-agents/runs/run-dispatch/stage_results/request-dispatch.json")
    );
    assert_eq!(
        load_execution_status(&paths).unwrap(),
        "### BUILDER_COMPLETE"
    );
    assert!(paths.tasks_active_dir.join("task-dispatch.md").is_file());
    assert!(!paths.tasks_done_dir.join("task-dispatch.md").exists());

    let events = runtime_events(&paths);
    assert_eq!(events[0]["event_type"], "stage_started");
    assert_eq!(events[1]["event_type"], "stage_completed");
    assert_eq!(
        events[1]["data"]["stage_result_path"],
        outcome.stage_result_path.unwrap().display().to_string()
    );
    assert_eq!(events[2]["event_type"], "router_decision");
    assert_eq!(events[2]["data"]["next_stage"], "checker");

    let trace_path = paths.runs_dir.join("run-dispatch/run_trace.json");
    assert!(trace_path.is_file());
    let trace = RunTraceGraph::from_json_str(&fs::read_to_string(&trace_path).unwrap()).unwrap();
    assert_eq!(trace.status.as_str(), "active");
    assert_eq!(trace.run_id, "run-dispatch");
    assert_eq!(trace.nodes.len(), 1);
    assert_eq!(trace.nodes[0].trace_node_id, "request-dispatch");
    assert_eq!(trace.nodes[0].stage, "builder");
    assert_eq!(trace.nodes[0].thinking_level.as_deref(), Some("high"));
    assert!(
        trace.nodes[0]
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "stage_result")
    );
    assert_eq!(trace.edges.len(), 1);
    assert_eq!(trace.edges[0].source_trace_node_id, "request-dispatch");
    assert_eq!(trace.edges[0].outcome, "BUILDER_COMPLETE");
    assert_eq!(trace.edges[0].edge_kind, "run_stage");
    assert_eq!(trace.edges[0].target_node_id.as_deref(), Some("checker"));
    assert_eq!(
        inspect_run_trace(paths.runs_dir.join("run-dispatch")).unwrap(),
        trace
    );

    session.finish().unwrap();
}

#[test]
fn integrated_mode_routes_builder_to_integrator_then_checker_and_traces_sequence() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        "[runtime]\ndefault_mode = \"default_codex_integrated\"\n",
    )
    .unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-integrated"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-integrated")).unwrap();
    assert_eq!(
        session.compiled_plan.execution_loop_id,
        "execution.with_integrator"
    );
    assert_eq!(session.compiled_plan.planning_loop_id, "planning.standard");

    let builder_runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"))
            .unwrap();
    let builder = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-integrated", "request-integrated-builder"),
        &builder_runner,
    )
    .unwrap();
    let builder_decision = builder.router_decision.as_ref().unwrap();
    assert_eq!(builder_decision.action, RouterAction::RunStage);
    assert_eq!(builder_decision.next_stage, Some(StageName::Integrator));
    assert_eq!(builder_decision.next_node_id.as_deref(), Some("integrator"));
    assert_eq!(builder_decision.reason, "builder:BUILDER_COMPLETE");
    assert_eq!(
        load_snapshot(&paths).unwrap().active_stage,
        Some(StageName::Integrator)
    );

    let integrator_runner = FakeRunner::with_default(FakeRunnerResult::terminal_marker(
        "### INTEGRATION_COMPLETE",
    ))
    .unwrap();
    let integrator = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-integrated-integrator"),
        &integrator_runner,
    )
    .unwrap();
    let request = integrator.stage_request.as_ref().unwrap();
    assert_eq!(request.request_id, "request-integrated-integrator");
    assert_eq!(request.run_id, "run-integrated");
    assert_eq!(request.mode_id, "default_codex_integrated");
    assert_eq!(request.plane, Plane::Execution);
    assert_eq!(request.stage, StageName::Integrator);
    assert_eq!(request.node_id, "integrator");
    assert_eq!(request.stage_kind_id, "integrator");
    assert_eq!(request.running_status_marker, "INTEGRATOR_RUNNING");
    assert!(
        request
            .entrypoint_path
            .ends_with("millrace-agents/entrypoints/execution/integrator.md")
    );
    assert_eq!(
        request.entrypoint_contract_id.as_deref(),
        Some("integrator.contract.v1")
    );
    assert_eq!(
        request.legal_terminal_markers,
        vec![
            "### INTEGRATION_COMPLETE".to_owned(),
            "### BLOCKED".to_owned(),
        ]
    );
    assert_eq!(
        request
            .allowed_result_classes_by_outcome
            .result_classes_for("INTEGRATION_COMPLETE")
            .unwrap(),
        [ResultClass::Success]
    );
    assert_eq!(
        request
            .allowed_result_classes_by_outcome
            .result_classes_for("BLOCKED")
            .unwrap(),
        [ResultClass::Blocked, ResultClass::RecoverableFailure]
    );
    assert_eq!(
        request.required_skill_paths,
        vec![
            paths
                .runtime_root
                .join("skills/stage/execution/integrator-core/SKILL.md")
                .display()
                .to_string()
        ]
    );
    assert!(request.attached_skill_paths.is_empty());
    assert_eq!(request.active_work_item_kind, Some(WorkItemKind::Task));
    assert_eq!(
        request.active_work_item_id.as_deref(),
        Some("task-integrated")
    );
    assert!(
        request
            .active_work_item_path
            .as_deref()
            .unwrap()
            .ends_with("millrace-agents/tasks/active/task-integrated.md")
    );
    assert_eq!(
        request.run_dir,
        paths.runs_dir.join("run-integrated").display().to_string()
    );
    assert_eq!(
        request.summary_status_path,
        paths.execution_status_file.display().to_string()
    );
    assert_eq!(
        request.runtime_snapshot_path,
        paths.runtime_snapshot_file.display().to_string()
    );
    assert_eq!(
        request.recovery_counters_path,
        paths.recovery_counters_file.display().to_string()
    );
    assert!(
        request
            .preferred_troubleshoot_report_path
            .as_deref()
            .unwrap()
            .ends_with("millrace-agents/runs/run-integrated/troubleshoot_report.md")
    );
    assert_eq!(request.runner_name.as_deref(), Some("codex_cli"));
    assert_eq!(request.model_name, None);
    assert_eq!(request.thinking_level, None);
    assert_eq!(request.model_reasoning_effort, None);
    assert_eq!(request.timeout_seconds, 3600);

    let stage_result = integrator.stage_result.as_ref().unwrap();
    assert_eq!(stage_result.stage, StageName::Integrator);
    assert_eq!(
        stage_result.terminal_result,
        TerminalResult::Execution(ExecutionTerminalResult::IntegrationComplete)
    );
    let integrator_decision = integrator.router_decision.as_ref().unwrap();
    assert_eq!(integrator_decision.action, RouterAction::RunStage);
    assert_eq!(integrator_decision.next_stage, Some(StageName::Checker));
    assert_eq!(integrator_decision.next_node_id.as_deref(), Some("checker"));
    assert_eq!(
        integrator_decision.reason,
        "integrator:INTEGRATION_COMPLETE"
    );

    let trace = inspect_run_trace(paths.runs_dir.join("run-integrated")).unwrap();
    assert_eq!(trace.status.as_str(), "active");
    assert_eq!(
        trace
            .nodes
            .iter()
            .map(|node| node.stage.as_str())
            .collect::<Vec<_>>(),
        vec!["builder", "integrator"]
    );
    assert_eq!(trace.edges.len(), 2);
    assert_eq!(trace.edges[0].outcome, "BUILDER_COMPLETE");
    assert_eq!(trace.edges[0].target_node_id.as_deref(), Some("integrator"));
    assert_eq!(trace.edges[1].outcome, "INTEGRATION_COMPLETE");
    assert_eq!(trace.edges[1].target_node_id.as_deref(), Some("checker"));

    assert_eq!(
        load_snapshot(&paths).unwrap().active_stage,
        Some(StageName::Checker)
    );
    assert!(paths.tasks_active_dir.join("task-integrated.md").is_file());
    assert!(!paths.tasks_done_dir.join("task-integrated.md").exists());

    session.finish().unwrap();
}

#[test]
fn integrated_mode_routes_integrator_blocked_to_recovery_and_threshold_consultant() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        "[runtime]\ndefault_mode = \"default_codex_integrated\"\n",
    )
    .unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-integrator-blocked"))
        .unwrap();
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-integrator-blocked")).unwrap();
    let builder_runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"))
            .unwrap();
    run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(
            "run-integrator-blocked",
            "request-integrator-blocked-builder",
        ),
        &builder_runner,
    )
    .unwrap();

    let blocked_runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### BLOCKED")).unwrap();
    let blocked = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-integrator-blocked-integrator"),
        &blocked_runner,
    )
    .unwrap();
    let decision = blocked.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::RunStage);
    assert_eq!(decision.next_stage, Some(StageName::Troubleshooter));
    assert_eq!(decision.next_node_id.as_deref(), Some("troubleshooter"));
    assert_eq!(decision.reason, "integrator_blocked");
    assert_eq!(
        decision.failure_class.as_deref(),
        Some("integrator_blocked")
    );
    assert_eq!(
        decision.counter_key.as_deref(),
        Some("task:task-integrator-blocked:integrator_blocked")
    );
    let counters = load_recovery_counters(&paths).unwrap();
    assert_eq!(counters.entries[0].failure_class, "integrator_blocked");
    assert_eq!(counters.entries[0].troubleshoot_attempt_count, 1);
    session.finish().unwrap();

    let threshold_temp = TempDir::new().unwrap();
    let threshold_paths = initialize_workspace(threshold_temp.path().join("workspace")).unwrap();
    fs::write(
        &threshold_paths.runtime_config_file,
        "[runtime]\ndefault_mode = \"default_codex_integrated\"\n",
    )
    .unwrap();
    QueueStore::from_paths(threshold_paths.clone())
        .enqueue_task(&task_document("task-integrator-threshold"))
        .unwrap();
    let mut threshold_session = startup_runtime_once_for_paths(
        &threshold_paths,
        startup_options("tick-integrator-threshold"),
    )
    .unwrap();
    run_serial_runtime_tick_with_runner(
        &mut threshold_session,
        tick_options(
            "run-integrator-threshold",
            "request-integrator-threshold-builder",
        ),
        &builder_runner,
    )
    .unwrap();
    save_recovery_counters(
        &threshold_paths,
        &RecoveryCounters {
            schema_version: "1.0".to_owned(),
            kind: "recovery_counters".to_owned(),
            entries: vec![RecoveryCounterEntry {
                failure_class: "integrator_blocked".to_owned(),
                work_item_id: "task-integrator-threshold".to_owned(),
                work_item_kind: WorkItemKind::Task,
                troubleshoot_attempt_count: 2,
                mechanic_attempt_count: 0,
                fix_cycle_count: 0,
                consultant_invocations: 0,
                last_updated_at: timestamp("2026-04-28T20:09:00Z"),
            }],
        },
    )
    .unwrap();

    let threshold = run_serial_runtime_tick_with_runner(
        &mut threshold_session,
        tick_options("ignored-run", "request-integrator-threshold-integrator"),
        &blocked_runner,
    )
    .unwrap();
    let threshold_decision = threshold.router_decision.as_ref().unwrap();
    assert_eq!(threshold_decision.action, RouterAction::RunStage);
    assert_eq!(threshold_decision.next_stage, Some(StageName::Consultant));
    assert_eq!(
        threshold_decision.next_node_id.as_deref(),
        Some("consultant")
    );
    assert_eq!(threshold_decision.reason, "integrator_blocked");
    assert_eq!(
        threshold_decision.failure_class.as_deref(),
        Some("integrator_blocked")
    );
    let threshold_counters = load_recovery_counters(&threshold_paths).unwrap();
    assert_eq!(threshold_counters.entries[0].troubleshoot_attempt_count, 2);
    assert_eq!(threshold_counters.entries[0].consultant_invocations, 1);

    threshold_session.finish().unwrap();
}

#[test]
fn serial_tick_can_dispatch_through_registry_dispatcher_without_real_runner() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-dispatcher"))
        .unwrap();

    let mut fake_config =
        FakeRunnerConfig::new(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE")).unwrap();
    fake_config.runner_name = "codex_cli".to_owned();
    let mut registry = RunnerRegistry::new();
    registry
        .register("codex_cli", FakeRunner::new(fake_config))
        .unwrap();
    let dispatcher = StageRunnerDispatcher::new(registry);

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-dispatcher")).unwrap();
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-dispatcher", "request-dispatcher"),
        &dispatcher,
    )
    .unwrap();

    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::StageDispatched);
    assert_eq!(
        outcome.runner_raw_result.as_ref().unwrap().runner_name,
        "codex_cli"
    );
    assert!(
        paths
            .runs_dir
            .join("run-dispatcher")
            .join("runner_prompt.request-dispatcher.md")
            .is_file()
    );
    assert!(
        paths
            .runs_dir
            .join("run-dispatcher")
            .join("runner_invocation.request-dispatcher.json")
            .is_file()
    );
    assert!(
        paths
            .runs_dir
            .join("run-dispatcher")
            .join("runner_completion.request-dispatcher.json")
            .is_file()
    );

    session.finish().unwrap();
}

#[test]
fn runtime_configured_dispatcher_registers_real_adapters_and_fake_test_adapter() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        "[runners]\ndefault_runner = \"pi_rpc\"\n",
    )
    .unwrap();
    let session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-configured-dispatcher"))
            .unwrap();
    let dispatcher = build_runtime_runner_dispatcher(&session).unwrap();
    assert_eq!(dispatcher.registry().names(), vec!["codex_cli", "pi_rpc"]);

    let mut request = sample_request(temp.path());
    request.runner_name = None;
    assert_eq!(dispatcher.resolve_runner_name(&request), "pi_rpc");

    session.finish().unwrap();
}

#[test]
fn serial_tick_normalizes_dispatcher_unknown_runner_through_recovery_path() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-missing-runner"))
        .unwrap();

    let mut registry = RunnerRegistry::new();
    registry
        .register(
            "codex_cli",
            StaticTokenRunner {
                runner_name: "codex_cli",
                terminal_marker: "### BUILDER_COMPLETE",
            },
        )
        .unwrap();
    let dispatcher = StageRunnerDispatcher::new(registry);
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-missing-runner")).unwrap();
    session
        .compiled_plan
        .execution_graph
        .nodes
        .iter_mut()
        .find(|node| node.node_id == "builder")
        .unwrap()
        .runner_name = Some("missing_runner".to_owned());

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-missing-runner", "request-missing-runner"),
        &dispatcher,
    )
    .unwrap();

    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::StageDispatched);
    assert_eq!(
        outcome.runner_raw_result.as_ref().unwrap().runner_name,
        "missing_runner"
    );
    assert_eq!(
        outcome.runner_raw_result.as_ref().unwrap().exit_kind,
        RunnerExitKind::RunnerError
    );
    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(
        stage_result.terminal_result,
        TerminalResult::Execution(ExecutionTerminalResult::Blocked)
    );
    assert_eq!(stage_result.result_class, ResultClass::RecoverableFailure);
    assert_eq!(
        stage_result.metadata["failure_class"],
        "runner_transport_failure"
    );
    let run_dir = paths.runs_dir.join("run-missing-runner");
    assert!(
        run_dir
            .join("runner_invocation.request-missing-runner.json")
            .is_file()
    );
    assert!(
        run_dir
            .join("runner_completion.request-missing-runner.json")
            .is_file()
    );
    assert!(
        fs::read_to_string(run_dir.join("runner_stderr.request-missing-runner.txt"))
            .unwrap()
            .contains("Unknown stage runner: missing_runner")
    );
    assert!(
        outcome
            .stage_result_path
            .as_ref()
            .is_some_and(|path| path.is_file())
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_applies_successful_stage_chain_and_clears_completed_task() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-complete"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-complete")).unwrap();
    let runner = FakeRunner::new(
        FakeRunnerConfig::new(FakeRunnerResult::missing_terminal_output())
            .unwrap()
            .with_stage_result(
                StageName::Builder,
                FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"),
            )
            .with_stage_result(
                StageName::Checker,
                FakeRunnerResult::terminal_marker("### CHECKER_PASS"),
            )
            .with_stage_result(
                StageName::Updater,
                FakeRunnerResult::terminal_marker("### UPDATE_COMPLETE"),
            ),
    );

    let builder = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-complete", "request-complete-builder"),
        &runner,
    )
    .unwrap();
    assert_eq!(
        builder.router_decision.as_ref().unwrap().next_stage,
        Some(StageName::Checker)
    );
    assert_eq!(session.snapshot.active_stage, Some(StageName::Checker));
    assert!(paths.tasks_active_dir.join("task-complete.md").is_file());

    let checker = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-complete-checker"),
        &runner,
    )
    .unwrap();
    assert_eq!(
        checker.router_decision.as_ref().unwrap().next_stage,
        Some(StageName::Updater)
    );
    assert_eq!(session.snapshot.active_stage, Some(StageName::Updater));

    let updater = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-complete-updater"),
        &runner,
    )
    .unwrap();
    assert_eq!(
        updater.router_decision.as_ref().unwrap().action,
        RouterAction::Idle
    );
    assert!(paths.tasks_done_dir.join("task-complete.md").is_file());
    assert!(!paths.tasks_active_dir.join("task-complete.md").exists());
    assert_eq!(session.snapshot.active_stage, None);
    assert_eq!(session.snapshot.active_runs_by_plane.len(), 0);
    assert_eq!(load_execution_status(&paths).unwrap(), "### IDLE");
    assert_eq!(session.snapshot.queue_depth_execution, 0);
    assert_eq!(load_recovery_counters(&paths).unwrap().entries.len(), 0);

    session.finish().unwrap();
}

#[test]
fn serial_tick_execution_run_stage_preserves_active_learning_lane() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-lane-execution"))
        .unwrap();
    queue
        .enqueue_learning_request(&learning_request_document("learn-lane"))
        .unwrap();

    let mut options = startup_options("tick-lane-execution");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    queue.claim_next_learning_request().unwrap().unwrap();

    session.snapshot.active_runs_by_plane.clear();
    session.snapshot.active_runs_by_plane.insert(
        Plane::Learning,
        active_run_state(
            Plane::Learning,
            StageName::Curator,
            "curator",
            "run-learning-lane",
            ActiveRunRequestKind::LearningRequest,
            Some(WorkItemKind::LearningRequest),
            Some("learn-lane"),
        ),
    );
    session.snapshot.active_runs_by_plane.insert(
        Plane::Execution,
        active_run_state(
            Plane::Execution,
            StageName::Builder,
            "builder",
            "run-execution-lane",
            ActiveRunRequestKind::ActiveWorkItem,
            Some(WorkItemKind::Task),
            Some("task-lane-execution"),
        ),
    );
    session.snapshot.active_plane = Some(Plane::Execution);
    session.snapshot.active_stage = Some(StageName::Builder);
    session.snapshot.active_node_id = Some("builder".to_owned());
    session.snapshot.active_stage_kind_id = Some("builder".to_owned());
    session.snapshot.active_run_id = Some("run-execution-lane".to_owned());
    session.snapshot.active_work_item_kind = Some(WorkItemKind::Task);
    session.snapshot.active_work_item_id = Some("task-lane-execution".to_owned());
    session.snapshot.active_since = Some(timestamp("2026-04-28T20:11:00Z"));
    save_snapshot(&paths, &session.snapshot).unwrap();

    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"))
            .unwrap();
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-lane-builder"),
        &runner,
    )
    .unwrap();

    assert_eq!(
        outcome.router_decision.as_ref().unwrap().next_stage,
        Some(StageName::Checker)
    );
    let snapshot = load_snapshot(&paths).unwrap();
    assert_eq!(snapshot.active_stage, Some(StageName::Checker));
    assert_eq!(snapshot.active_runs_by_plane.len(), 2);
    let learning_run = snapshot.active_runs_by_plane.get(&Plane::Learning).unwrap();
    assert_eq!(learning_run.stage, StageName::Curator);
    assert_eq!(learning_run.work_item_id.as_deref(), Some("learn-lane"));
    assert!(
        paths
            .learning_requests_active_dir
            .join("learn-lane.md")
            .is_file()
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_learning_idle_preserves_active_execution_lane() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-lane-execution"))
        .unwrap();
    queue
        .enqueue_learning_request(&learning_request_document("learn-lane"))
        .unwrap();

    let mut options = startup_options("tick-lane-learning");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    queue.claim_next_learning_request().unwrap().unwrap();

    session.snapshot.active_runs_by_plane.clear();
    session.snapshot.active_runs_by_plane.insert(
        Plane::Execution,
        active_run_state(
            Plane::Execution,
            StageName::Builder,
            "builder",
            "run-execution-lane",
            ActiveRunRequestKind::ActiveWorkItem,
            Some(WorkItemKind::Task),
            Some("task-lane-execution"),
        ),
    );
    session.snapshot.active_runs_by_plane.insert(
        Plane::Learning,
        active_run_state(
            Plane::Learning,
            StageName::Curator,
            "curator",
            "run-learning-lane",
            ActiveRunRequestKind::LearningRequest,
            Some(WorkItemKind::LearningRequest),
            Some("learn-lane"),
        ),
    );
    session.snapshot.active_plane = Some(Plane::Learning);
    session.snapshot.active_stage = Some(StageName::Curator);
    session.snapshot.active_node_id = Some("curator".to_owned());
    session.snapshot.active_stage_kind_id = Some("curator".to_owned());
    session.snapshot.active_run_id = Some("run-learning-lane".to_owned());
    session.snapshot.active_work_item_kind = Some(WorkItemKind::LearningRequest);
    session.snapshot.active_work_item_id = Some("learn-lane".to_owned());
    session.snapshot.active_since = Some(timestamp("2026-04-28T20:11:00Z"));
    save_snapshot(&paths, &session.snapshot).unwrap();

    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### CURATOR_COMPLETE"))
            .unwrap();
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-lane-curator"),
        &runner,
    )
    .unwrap();

    assert_eq!(
        outcome.router_decision.as_ref().unwrap().action,
        RouterAction::Idle
    );
    assert!(
        paths
            .learning_requests_done_dir
            .join("learn-lane.md")
            .is_file()
    );
    assert!(
        paths
            .tasks_active_dir
            .join("task-lane-execution.md")
            .is_file()
    );
    assert_eq!(load_learning_status(&paths).unwrap(), "### IDLE");

    let snapshot = load_snapshot(&paths).unwrap();
    assert_eq!(snapshot.active_runs_by_plane.len(), 1);
    let execution_run = snapshot
        .active_runs_by_plane
        .get(&Plane::Execution)
        .unwrap();
    assert_eq!(execution_run.stage, StageName::Builder);
    assert_eq!(
        execution_run.work_item_id.as_deref(),
        Some("task-lane-execution")
    );
    assert_eq!(snapshot.active_plane, Some(Plane::Execution));
    assert_eq!(snapshot.active_stage, Some(StageName::Builder));

    session.finish().unwrap();
}

#[test]
fn serial_tick_learning_trigger_enqueues_analyst_first_request() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-learning-trigger"))
        .unwrap();

    let mut options = startup_options("tick-learning-trigger");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();
    let runner = FakeRunner::new(
        FakeRunnerConfig::new(FakeRunnerResult::missing_terminal_output())
            .unwrap()
            .with_stage_result(
                StageName::Builder,
                FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"),
            )
            .with_stage_result(
                StageName::Checker,
                FakeRunnerResult::terminal_marker("### FIX_NEEDED"),
            )
            .with_stage_result(
                StageName::Fixer,
                FakeRunnerResult::terminal_marker("### FIXER_COMPLETE"),
            )
            .with_stage_result(
                StageName::Doublechecker,
                FakeRunnerResult::terminal_marker("### DOUBLECHECK_PASS"),
            ),
    );

    for (run_id, request_id) in [
        ("run-trigger-builder", "request-trigger-builder"),
        ("ignored-run", "request-trigger-checker"),
        ("ignored-run", "request-trigger-fixer"),
        ("ignored-run", "request-trigger-doublechecker"),
    ] {
        run_serial_runtime_tick_with_runner(
            &mut session,
            tick_options(run_id, request_id),
            &runner,
        )
        .unwrap();
    }

    let queued_learning = fs::read_dir(&paths.learning_requests_queue_dir)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(queued_learning.len(), 1);
    assert!(
        paths
            .tasks_active_dir
            .join("task-learning-trigger.md")
            .is_file()
    );
    assert!(
        !paths
            .tasks_done_dir
            .join("task-learning-trigger.md")
            .exists()
    );

    let raw = fs::read_to_string(queued_learning[0].path()).unwrap();
    let document = parse_learning_request_document(&raw).unwrap();
    assert_eq!(document.requested_action, LearningRequestAction::Improve);
    assert_eq!(document.target_skill_id, None);
    assert!(document.preferred_output_paths.is_empty());
    assert_eq!(
        document.target_stage.map(StageName::from),
        Some(StageName::Analyst)
    );
    assert_eq!(
        document.originating_run_ids,
        vec!["run-trigger-builder".to_owned()]
    );
    assert_eq!(
        document.trigger_metadata["rule_id"],
        "execution.doublechecker.success-to-analyst"
    );
    assert_eq!(document.trigger_metadata["source_stage"], "doublechecker");
    assert_eq!(
        document.trigger_metadata["terminal_result"],
        "DOUBLECHECK_PASS"
    );
    assert_eq!(document.trigger_metadata["target_stage"], "analyst");
    assert_eq!(document.trigger_metadata["target_skill_id"], Value::Null);
    assert_eq!(
        document.trigger_metadata["preferred_output_paths"],
        json!([])
    );
    assert!(
        document
            .artifact_paths
            .iter()
            .any(|path| path.contains("request-trigger-doublechecker.json"))
    );
    assert!(
        runtime_events(&paths)
            .iter()
            .any(|event| event["event_type"] == "learning_request_enqueued")
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_planner_complete_triggers_librarian_request_with_planner_artifacts() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document("spec-planner-trigger"))
        .unwrap();

    let run_id = "run-planner-trigger";
    let request_id = "request-planner-trigger";
    let run_dir = paths.runs_dir.join(run_id);
    fs::create_dir_all(&run_dir).unwrap();
    fs::write(
        run_dir.join("planner_summary.md"),
        "# Planner Summary\n\nGenerated or refined spec paths:\n- millrace-agents/specs/active/spec-planner-trigger.md\n",
    )
    .unwrap();

    let mut options = startup_options("tick-planner-librarian-trigger");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();
    let runner = FakeRunner::new(
        FakeRunnerConfig::new(FakeRunnerResult {
            output: FakeRunnerOutput::StructuredTerminalResult {
                terminal_result: "PLANNER_COMPLETE".to_owned(),
                result_class: Some(ResultClass::Success),
                summary_artifact_paths: vec!["planner_summary.md".to_owned()],
            },
            exit_kind: RunnerExitKind::Completed,
            exit_code: Some(0),
            observed_exit_kind: None,
            observed_exit_code: None,
            stderr: None,
            event_log: None,
            token_usage: None,
        })
        .unwrap(),
    );

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(run_id, request_id),
        &runner,
    )
    .unwrap();

    assert_eq!(
        outcome.stage_result.as_ref().unwrap().stage,
        StageName::Planner
    );
    assert!(
        outcome
            .stage_result
            .as_ref()
            .unwrap()
            .artifact_paths
            .iter()
            .any(|path| path == "planner_summary.md")
    );

    let queued_learning = fs::read_dir(&paths.learning_requests_queue_dir)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(queued_learning.len(), 1);
    let raw = fs::read_to_string(queued_learning[0].path()).unwrap();
    let document = parse_learning_request_document(&raw).unwrap();

    assert_eq!(document.requested_action, LearningRequestAction::Install);
    assert_eq!(
        document.target_stage.map(StageName::from),
        Some(StageName::Librarian)
    );
    assert_eq!(
        document.trigger_metadata["rule_id"],
        "planning.planner.complete-to-librarian"
    );
    assert_eq!(document.trigger_metadata["source_stage"], "planner");
    assert_eq!(document.trigger_metadata["source_work_item_kind"], "spec");
    assert_eq!(
        document.trigger_metadata["source_work_item_id"],
        "spec-planner-trigger"
    );
    assert!(
        document.trigger_metadata["source_active_work_item_path"]
            .as_str()
            .unwrap()
            .ends_with("millrace-agents/specs/active/spec-planner-trigger.md")
    );
    assert!(document.artifact_paths.iter().any(
        |path| path.contains("stage_results") && path.ends_with("request-planner-trigger.json")
    ));
    assert!(
        document
            .artifact_paths
            .iter()
            .any(|path| path.ends_with("planner_summary.md"))
    );
    assert!(
        document.trigger_metadata["stage_result_path"]
            .as_str()
            .unwrap()
            .ends_with("stage_results/request-planner-trigger.json")
    );
    assert!(
        document.trigger_metadata["artifact_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path
                .as_str()
                .is_some_and(|path| path.ends_with("planner_summary.md")))
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_default_mode_planner_complete_does_not_trigger_librarian_request() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document("spec-default-planner"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-default-planner")).unwrap();
    assert!(session.compiled_plan.learning_graph.is_none());
    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"))
            .unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-default-planner", "request-default-planner"),
        &runner,
    )
    .unwrap();

    assert_eq!(
        outcome.stage_result.as_ref().unwrap().stage,
        StageName::Planner
    );
    assert_eq!(
        outcome.stage_result.as_ref().unwrap().terminal_result,
        TerminalResult::Planning(PlanningTerminalResult::PlannerComplete)
    );
    assert_eq!(
        fs::read_dir(&paths.learning_requests_queue_dir)
            .unwrap()
            .count(),
        0
    );
    assert!(
        !runtime_events(&paths)
            .iter()
            .any(|event| event["event_type"] == "learning_request_enqueued")
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_targeted_librarian_learning_request_uses_librarian_metadata_and_marks_done() {
    for (learning_request_id, terminal, expected_terminal, expected_class, expected_success) in [
        (
            "learn-librarian-complete",
            "LIBRARIAN_COMPLETE",
            TerminalResult::Learning(LearningTerminalResult::LibrarianComplete),
            ResultClass::Success,
            true,
        ),
        (
            "learn-librarian-noop",
            "LIBRARIAN_NOOP",
            TerminalResult::Learning(LearningTerminalResult::LibrarianNoop),
            ResultClass::NoOp,
            false,
        ),
    ] {
        let temp = TempDir::new().unwrap();
        let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
        let mut document = learning_request_document(learning_request_id);
        document.requested_action = LearningRequestAction::Install;
        document.target_stage = Some(LearningStageName::Librarian);
        QueueStore::from_paths(paths.clone())
            .enqueue_learning_request(&document)
            .unwrap();

        let mut options = startup_options(&format!("tick-{learning_request_id}"));
        options.requested_mode_id = Some("learning_codex".to_owned());
        let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();
        let runner = FakeRunner::with_default(FakeRunnerResult::structured_terminal_result(
            terminal,
            Some(expected_class),
        ))
        .unwrap();

        let outcome = run_serial_runtime_tick_with_runner(
            &mut session,
            tick_options(
                &format!("run-{learning_request_id}"),
                &format!("request-{learning_request_id}"),
            ),
            &runner,
        )
        .unwrap();

        let request = outcome.stage_request.as_ref().unwrap();
        assert_eq!(request.stage, StageName::Librarian);
        assert_eq!(request.request_kind, RequestKind::LearningRequest);
        assert_eq!(request.node_id, "librarian");
        assert_eq!(request.stage_kind_id, "librarian");
        assert_eq!(request.running_status_marker, "LIBRARIAN_RUNNING");
        assert_eq!(
            request.legal_terminal_markers,
            vec![
                "### LIBRARIAN_COMPLETE".to_owned(),
                "### LIBRARIAN_NOOP".to_owned(),
                "### BLOCKED".to_owned()
            ]
        );
        assert!(
            request
                .entrypoint_path
                .ends_with("millrace-agents/entrypoints/learning/librarian.md")
        );
        assert_eq!(
            request.entrypoint_contract_id.as_deref(),
            Some("librarian.contract.v1")
        );
        assert_eq!(request.required_skill_paths.len(), 1);
        assert!(
            request.required_skill_paths[0]
                .ends_with("millrace-agents/skills/stage/learning/librarian-core/SKILL.md")
        );
        assert_eq!(request.runner_name.as_deref(), Some("codex_cli"));
        assert_eq!(
            request.active_work_item_kind,
            Some(WorkItemKind::LearningRequest)
        );
        assert_eq!(
            request.active_work_item_id.as_deref(),
            Some(learning_request_id)
        );
        assert!(
            request
                .active_work_item_path
                .as_deref()
                .unwrap()
                .ends_with(&format!(
                    "millrace-agents/learning/requests/active/{learning_request_id}.md"
                ))
        );

        let stage_result = outcome.stage_result.as_ref().unwrap();
        assert_eq!(stage_result.stage, StageName::Librarian);
        assert_eq!(stage_result.terminal_result, expected_terminal);
        assert_eq!(stage_result.result_class, expected_class);
        assert_eq!(stage_result.success, expected_success);
        assert_eq!(
            outcome.router_decision.as_ref().unwrap().action,
            RouterAction::Idle
        );
        assert!(
            paths
                .learning_requests_done_dir
                .join(format!("{learning_request_id}.md"))
                .is_file()
        );
        assert!(
            !paths
                .learning_requests_active_dir
                .join(format!("{learning_request_id}.md"))
                .exists()
        );
        assert_eq!(load_learning_status(&paths).unwrap(), "### IDLE");

        session.finish().unwrap();
    }
}

#[test]
fn serial_tick_librarian_blocked_preserves_recoverable_failure_evidence() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let mut document = learning_request_document("learn-librarian-blocked");
    document.requested_action = LearningRequestAction::Install;
    document.target_stage = Some(LearningStageName::Librarian);
    QueueStore::from_paths(paths.clone())
        .enqueue_learning_request(&document)
        .unwrap();

    let mut options = startup_options("tick-librarian-blocked");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();
    let runner = FakeRunner::with_default(FakeRunnerResult::structured_terminal_result(
        "BLOCKED",
        Some(ResultClass::RecoverableFailure),
    ))
    .unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-librarian-blocked", "request-librarian-blocked"),
        &runner,
    )
    .unwrap();

    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(stage_result.stage, StageName::Librarian);
    assert_eq!(
        stage_result.terminal_result,
        TerminalResult::Learning(LearningTerminalResult::Blocked)
    );
    assert_eq!(stage_result.result_class, ResultClass::RecoverableFailure);
    assert_eq!(
        outcome.router_decision.as_ref().unwrap().action,
        RouterAction::Blocked
    );
    assert_eq!(
        outcome.router_decision.as_ref().unwrap().reason,
        "librarian_blocked"
    );
    assert!(outcome.stage_result_path.as_ref().unwrap().is_file());
    assert!(outcome.terminal_marker_path.as_ref().unwrap().is_file());
    assert!(outcome.router_decision_path.as_ref().unwrap().is_file());
    assert!(
        paths
            .learning_requests_blocked_dir
            .join("learn-librarian-blocked.md")
            .is_file()
    );
    assert!(
        !paths
            .learning_requests_active_dir
            .join("learn-librarian-blocked.md")
            .exists()
    );
    assert_eq!(load_learning_status(&paths).unwrap(), "### BLOCKED");

    session.finish().unwrap();
}

#[test]
fn python_v0_17_4_runtime_generated_learning_request_copies_trigger_destination_metadata() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-learning-targeted-trigger"))
        .unwrap();

    let mode_path = paths.modes_dir.join("learning_codex.json");
    let mut mode: Value = serde_json::from_str(&fs::read_to_string(&mode_path).unwrap()).unwrap();
    mode["learning_trigger_rules"] = json!([
        {
            "rule_id": "execution.doublechecker.precise-to-curator",
            "source_plane": "execution",
            "source_stage": "doublechecker",
            "on_terminal_results": ["DOUBLECHECK_PASS"],
            "target_stage": "curator",
            "requested_action": "improve",
            "target_skill_id": "doublechecker-core",
            "preferred_output_paths": [
                "millrace-agents/skills/stage/execution/doublechecker-core/SKILL.md"
            ]
        }
    ]);
    fs::write(
        &mode_path,
        serde_json::to_string_pretty(&mode).unwrap() + "\n",
    )
    .unwrap();

    let mut options = startup_options("tick-learning-targeted-trigger");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();
    let runner = FakeRunner::new(
        FakeRunnerConfig::new(FakeRunnerResult::missing_terminal_output())
            .unwrap()
            .with_stage_result(
                StageName::Builder,
                FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"),
            )
            .with_stage_result(
                StageName::Checker,
                FakeRunnerResult::terminal_marker("### FIX_NEEDED"),
            )
            .with_stage_result(
                StageName::Fixer,
                FakeRunnerResult::terminal_marker("### FIXER_COMPLETE"),
            )
            .with_stage_result(
                StageName::Doublechecker,
                FakeRunnerResult::terminal_marker("### DOUBLECHECK_PASS"),
            ),
    );

    for (run_id, request_id) in [
        ("run-targeted-trigger", "request-targeted-builder"),
        ("ignored-run", "request-targeted-checker"),
        ("ignored-run", "request-targeted-fixer"),
        ("ignored-run", "request-targeted-doublechecker"),
    ] {
        run_serial_runtime_tick_with_runner(
            &mut session,
            tick_options(run_id, request_id),
            &runner,
        )
        .unwrap();
    }

    let queued_learning = fs::read_dir(&paths.learning_requests_queue_dir)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(queued_learning.len(), 1);
    let raw = fs::read_to_string(queued_learning[0].path()).unwrap();
    let document = parse_learning_request_document(&raw).unwrap();
    let expected_path = "millrace-agents/skills/stage/execution/doublechecker-core/SKILL.md";

    assert_eq!(
        document.target_stage.map(StageName::from),
        Some(StageName::Curator)
    );
    assert_eq!(
        document.target_skill_id.as_deref(),
        Some("doublechecker-core")
    );
    assert_eq!(
        document.preferred_output_paths,
        vec![expected_path.to_owned()]
    );
    assert_eq!(
        document.trigger_metadata["rule_id"],
        "execution.doublechecker.precise-to-curator"
    );
    assert_eq!(document.trigger_metadata["target_stage"], "curator");
    assert_eq!(
        document.trigger_metadata["target_skill_id"],
        "doublechecker-core"
    );
    assert_eq!(
        document.trigger_metadata["preferred_output_paths"],
        json!([expected_path])
    );

    let event = runtime_events(&paths)
        .into_iter()
        .find(|event| event["event_type"] == "learning_request_enqueued")
        .unwrap();
    assert_eq!(event["data"]["target_skill_id"], "doublechecker-core");
    assert_eq!(
        event["data"]["preferred_output_paths"],
        json!([expected_path])
    );

    session.finish().unwrap();
}

#[test]
fn python_v0_17_4_learning_noop_terminal_marks_request_done_with_noop_evidence() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_learning_request(&learning_request_document("learn-noop"))
        .unwrap();

    let mut options = startup_options("tick-learning-noop");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();
    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### ANALYST_NOOP")).unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-learning-noop", "request-learning-noop"),
        &runner,
    )
    .unwrap();

    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(stage_result.stage, StageName::Analyst);
    assert_eq!(
        stage_result.terminal_result,
        TerminalResult::Learning(LearningTerminalResult::AnalystNoop)
    );
    assert_eq!(stage_result.result_class, ResultClass::NoOp);
    assert!(!stage_result.success);
    assert_eq!(
        outcome.router_decision.as_ref().unwrap().action,
        RouterAction::Idle
    );
    assert_eq!(
        outcome.router_decision.as_ref().unwrap().reason,
        "analyst:ANALYST_NOOP"
    );
    assert!(outcome.stage_result_path.as_ref().unwrap().is_file());
    assert!(outcome.terminal_marker_path.as_ref().unwrap().is_file());
    assert!(outcome.router_decision_path.as_ref().unwrap().is_file());
    assert!(
        paths
            .learning_requests_done_dir
            .join("learn-noop.md")
            .is_file()
    );
    assert!(
        !paths
            .learning_requests_blocked_dir
            .join("learn-noop.md")
            .exists()
    );
    assert_eq!(load_learning_status(&paths).unwrap(), "### IDLE");

    session.finish().unwrap();
}

#[test]
fn serial_tick_curator_promotion_defers_until_foreground_drain() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-promotion"))
        .unwrap();
    queue
        .enqueue_learning_request(&learning_request_document("learn-promotion"))
        .unwrap();

    let mut options = startup_options("tick-learning-promotion");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    queue.claim_next_learning_request().unwrap().unwrap();

    session.snapshot.active_runs_by_plane.clear();
    session.snapshot.active_runs_by_plane.insert(
        Plane::Execution,
        active_run_state(
            Plane::Execution,
            StageName::Updater,
            "updater",
            "run-execution-promotion",
            ActiveRunRequestKind::ActiveWorkItem,
            Some(WorkItemKind::Task),
            Some("task-promotion"),
        ),
    );
    session.snapshot.active_runs_by_plane.insert(
        Plane::Learning,
        active_run_state(
            Plane::Learning,
            StageName::Curator,
            "curator",
            "run-learning-promotion",
            ActiveRunRequestKind::LearningRequest,
            Some(WorkItemKind::LearningRequest),
            Some("learn-promotion"),
        ),
    );
    session.snapshot.active_plane = Some(Plane::Learning);
    session.snapshot.active_stage = Some(StageName::Curator);
    session.snapshot.active_node_id = Some("curator".to_owned());
    session.snapshot.active_stage_kind_id = Some("curator".to_owned());
    session.snapshot.active_run_id = Some("run-learning-promotion".to_owned());
    session.snapshot.active_work_item_kind = Some(WorkItemKind::LearningRequest);
    session.snapshot.active_work_item_id = Some("learn-promotion".to_owned());
    session.snapshot.active_since = Some(timestamp("2026-04-28T20:11:00Z"));
    save_snapshot(&paths, &session.snapshot).unwrap();

    let learning_run_dir = paths.runs_dir.join("run-learning-promotion");
    fs::create_dir_all(&learning_run_dir).unwrap();
    fs::write(
        learning_run_dir.join("skill_update.checker-core.json"),
        "{}\n",
    )
    .unwrap();
    let mut curator_result = FakeRunnerResult::structured_terminal_result(
        "CURATOR_COMPLETE",
        Some(ResultClass::Success),
    );
    if let FakeRunnerOutput::StructuredTerminalResult {
        summary_artifact_paths,
        ..
    } = &mut curator_result.output
    {
        summary_artifact_paths.push("skill_update.checker-core.json".to_owned());
    }
    let curator_runner = FakeRunner::with_default(curator_result).unwrap();
    let curator = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-promotion-curator"),
        &curator_runner,
    )
    .unwrap();

    let evidence_path = curator
        .stage_request
        .as_ref()
        .and_then(|request| request.skill_revision_evidence_path.as_ref())
        .map(PathBuf::from)
        .unwrap();
    assert!(evidence_path.is_file());
    assert_eq!(
        curator.stage_result.as_ref().unwrap().metadata["skill_revision_evidence_path"],
        evidence_path.display().to_string()
    );

    let deferred_path = paths
        .learning_update_candidates_dir
        .join("deferred/run-learning-promotion-learn-promotion.json");
    let applied_path = paths
        .learning_update_candidates_dir
        .join("applied/run-learning-promotion-learn-promotion.json");
    assert!(deferred_path.is_file());
    assert!(!applied_path.exists());
    let deferred: Value =
        serde_json::from_str(&fs::read_to_string(&deferred_path).unwrap()).unwrap();
    assert_eq!(deferred["kind"], "learning_curator_promotion");
    assert_eq!(deferred["state"], "deferred");
    assert_eq!(deferred["foreground_active_planes"], json!(["execution"]));
    assert_eq!(
        deferred["artifact_paths"],
        json!(["skill_update.checker-core.json"])
    );
    assert!(
        runtime_events(&paths)
            .iter()
            .any(|event| event["event_type"] == "learning_curator_promotion_deferred")
    );

    let updater_runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### UPDATE_COMPLETE")).unwrap();
    run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-promotion-updater"),
        &updater_runner,
    )
    .unwrap();

    assert!(!deferred_path.exists());
    assert!(applied_path.is_file());
    let applied: Value = serde_json::from_str(&fs::read_to_string(&applied_path).unwrap()).unwrap();
    assert_eq!(applied["state"], "applied");
    assert!(applied["applied_at"].as_str().is_some());
    assert!(runtime_events(&paths).iter().any(|event| {
        event["event_type"] == "learning_curator_promotion_applied"
            && event["data"]["source"] == "deferred_safe_boundary"
    }));

    session.finish().unwrap();
}

#[test]
fn serial_tick_curator_rejected_decision_keeps_evidence_without_promotion_or_source_mutation() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_learning_request(&learning_request_document("learn-rejected"))
        .unwrap();

    let mut options = startup_options("tick-learning-rejected");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();
    queue.claim_next_learning_request().unwrap().unwrap();

    session.snapshot.active_runs_by_plane.clear();
    session.snapshot.active_runs_by_plane.insert(
        Plane::Learning,
        active_run_state(
            Plane::Learning,
            StageName::Curator,
            "curator",
            "run-learning-rejected",
            ActiveRunRequestKind::LearningRequest,
            Some(WorkItemKind::LearningRequest),
            Some("learn-rejected"),
        ),
    );
    session.snapshot.active_plane = Some(Plane::Learning);
    session.snapshot.active_stage = Some(StageName::Curator);
    session.snapshot.active_node_id = Some("curator".to_owned());
    session.snapshot.active_stage_kind_id = Some("curator".to_owned());
    session.snapshot.active_run_id = Some("run-learning-rejected".to_owned());
    session.snapshot.active_work_item_kind = Some(WorkItemKind::LearningRequest);
    session.snapshot.active_work_item_id = Some("learn-rejected".to_owned());
    session.snapshot.active_since = Some(timestamp("2026-04-28T20:11:00Z"));
    save_snapshot(&paths, &session.snapshot).unwrap();

    let learning_run_dir = paths.runs_dir.join("run-learning-rejected");
    fs::create_dir_all(&learning_run_dir).unwrap();
    fs::write(
        learning_run_dir.join("curator_decision.md"),
        "# Curator Decision\n\nDecision: rejected\nSummary: no skill update accepted.\n",
    )
    .unwrap();
    let source_skill_tree = default_source_skill_tree();
    let source_before = file_tree_snapshot(&source_skill_tree);
    assert!(!source_before.is_empty());

    let mut curator_result = FakeRunnerResult::structured_terminal_result(
        "CURATOR_COMPLETE",
        Some(ResultClass::Success),
    );
    if let FakeRunnerOutput::StructuredTerminalResult {
        summary_artifact_paths,
        ..
    } = &mut curator_result.output
    {
        summary_artifact_paths.push("curator_decision.md".to_owned());
    }
    let curator_runner = FakeRunner::with_default(curator_result).unwrap();
    let curator = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-rejected-curator"),
        &curator_runner,
    )
    .unwrap();

    assert_eq!(
        curator.router_decision.as_ref().unwrap().action,
        RouterAction::Idle
    );
    assert!(
        paths
            .learning_requests_done_dir
            .join("learn-rejected.md")
            .is_file()
    );
    assert!(
        !paths
            .learning_requests_active_dir
            .join("learn-rejected.md")
            .exists()
    );
    assert_curator_decision_artifact_is_inspectable(&curator, &learning_run_dir);
    assert_no_learning_update_candidate_records(&paths);
    assert!(!runtime_events(&paths).iter().any(|event| {
        event["event_type"]
            .as_str()
            .is_some_and(|value| value.starts_with("learning_curator_promotion_"))
    }));
    assert_eq!(source_before, file_tree_snapshot(&source_skill_tree));

    session.finish().unwrap();
}

#[test]
fn serial_tick_curator_blocked_decision_keeps_evidence_without_promotion_or_source_mutation() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_learning_request(&learning_request_document("learn-blocked"))
        .unwrap();

    let mut options = startup_options("tick-learning-blocked");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();
    queue.claim_next_learning_request().unwrap().unwrap();

    session.snapshot.active_runs_by_plane.clear();
    session.snapshot.active_runs_by_plane.insert(
        Plane::Learning,
        active_run_state(
            Plane::Learning,
            StageName::Curator,
            "curator",
            "run-learning-blocked",
            ActiveRunRequestKind::LearningRequest,
            Some(WorkItemKind::LearningRequest),
            Some("learn-blocked"),
        ),
    );
    session.snapshot.active_plane = Some(Plane::Learning);
    session.snapshot.active_stage = Some(StageName::Curator);
    session.snapshot.active_node_id = Some("curator".to_owned());
    session.snapshot.active_stage_kind_id = Some("curator".to_owned());
    session.snapshot.active_run_id = Some("run-learning-blocked".to_owned());
    session.snapshot.active_work_item_kind = Some(WorkItemKind::LearningRequest);
    session.snapshot.active_work_item_id = Some("learn-blocked".to_owned());
    session.snapshot.active_since = Some(timestamp("2026-04-28T20:11:00Z"));
    save_snapshot(&paths, &session.snapshot).unwrap();

    let learning_run_dir = paths.runs_dir.join("run-learning-blocked");
    fs::create_dir_all(&learning_run_dir).unwrap();
    fs::write(
        learning_run_dir.join("curator_decision.md"),
        "# Curator Decision\n\nDecision: blocked\nSummary: curation needs operator input.\n",
    )
    .unwrap();
    let source_skill_tree = default_source_skill_tree();
    let source_before = file_tree_snapshot(&source_skill_tree);
    assert!(!source_before.is_empty());

    let mut curator_result =
        FakeRunnerResult::structured_terminal_result("BLOCKED", Some(ResultClass::Blocked));
    if let FakeRunnerOutput::StructuredTerminalResult {
        summary_artifact_paths,
        ..
    } = &mut curator_result.output
    {
        summary_artifact_paths.push("curator_decision.md".to_owned());
    }
    let curator_runner = FakeRunner::with_default(curator_result).unwrap();
    let curator = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-blocked-curator"),
        &curator_runner,
    )
    .unwrap();

    assert_eq!(
        curator.router_decision.as_ref().unwrap().action,
        RouterAction::Blocked
    );
    assert!(
        paths
            .learning_requests_blocked_dir
            .join("learn-blocked.md")
            .is_file()
    );
    assert!(
        !paths
            .learning_requests_active_dir
            .join("learn-blocked.md")
            .exists()
    );
    assert_curator_decision_artifact_is_inspectable(&curator, &learning_run_dir);
    assert_no_learning_update_candidate_records(&paths);
    assert!(!runtime_events(&paths).iter().any(|event| {
        event["event_type"]
            .as_str()
            .is_some_and(|value| value.starts_with("learning_curator_promotion_"))
    }));
    assert_eq!(source_before, file_tree_snapshot(&source_skill_tree));

    session.finish().unwrap();
}

#[test]
fn serial_tick_routes_execution_completion_conflict_into_troubleshooter_recovery() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let catalog_path = write_runtime_error_catalog(&paths.root);
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-conflict"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-execution-conflict")).unwrap();
    let custom_troubleshooter_node_id = "recovery.execution.troubleshooter";
    rename_compiled_node(
        &mut session,
        Plane::Execution,
        "troubleshooter",
        custom_troubleshooter_node_id,
    );
    let inner = FakeRunner::new(
        FakeRunnerConfig::new(FakeRunnerResult::missing_terminal_output())
            .unwrap()
            .with_stage_result(
                StageName::Builder,
                FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"),
            )
            .with_stage_result(
                StageName::Checker,
                FakeRunnerResult::terminal_marker("### CHECKER_PASS"),
            )
            .with_stage_result(
                StageName::Updater,
                FakeRunnerResult::terminal_marker("### UPDATE_COMPLETE"),
            ),
    );
    let runner = PreemptiveCompletionRunner {
        paths: paths.clone(),
        inner,
        stage: StageName::Updater,
        work_item_kind: WorkItemKind::Task,
        work_item_id: "task-conflict",
    };

    run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(
            "run-execution-conflict",
            "request-execution-conflict-builder",
        ),
        &runner,
    )
    .unwrap();
    run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-execution-conflict-checker"),
        &runner,
    )
    .unwrap();
    let updater = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-execution-conflict-updater"),
        &runner,
    )
    .unwrap();

    let decision = updater.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::RunStage);
    assert_eq!(decision.next_stage, Some(StageName::Troubleshooter));
    assert_eq!(
        decision.next_node_id.as_deref(),
        Some(custom_troubleshooter_node_id)
    );
    assert_eq!(
        decision.failure_class.as_deref(),
        Some("execution_work_item_completion_conflict")
    );
    assert_eq!(
        decision.reason,
        "runtime_exception:execution_work_item_completion_conflict"
    );
    assert!(paths.tasks_done_dir.join("task-conflict.md").is_file());
    assert!(!paths.tasks_active_dir.join("task-conflict.md").exists());
    assert_eq!(load_execution_status(&paths).unwrap(), "### BLOCKED");

    let snapshot = load_snapshot(&paths).unwrap();
    assert_eq!(snapshot.active_stage, Some(StageName::Troubleshooter));
    assert_eq!(
        snapshot.active_node_id.as_deref(),
        Some(custom_troubleshooter_node_id)
    );
    assert_eq!(
        snapshot.active_stage_kind_id.as_deref(),
        Some("troubleshooter")
    );
    assert_eq!(
        snapshot.current_failure_class.as_deref(),
        Some("execution_work_item_completion_conflict")
    );
    assert_eq!(
        snapshot.last_stage_result_path.as_deref(),
        Some(
            "millrace-agents/runs/run-execution-conflict/stage_results/request-execution-conflict-updater.json"
        )
    );

    assert_eq!(
        updater.runtime_error_context_path.as_ref().unwrap(),
        &paths.runtime_error_context_file
    );
    let context = RuntimeErrorContext::from_json_str(
        &fs::read_to_string(updater.runtime_error_context_path.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        context.error_code.as_str(),
        "execution_work_item_completion_conflict"
    );
    assert_eq!(context.failed_stage, StageName::Updater);
    assert_eq!(context.repair_stage, StageName::Troubleshooter);
    assert_eq!(context.router_action.as_deref(), Some("idle"));
    assert_eq!(context.terminal_result.unwrap().as_str(), "UPDATE_COMPLETE");
    assert_eq!(
        context.stage_result_path.as_deref(),
        Some(
            "millrace-agents/runs/run-execution-conflict/stage_results/request-execution-conflict-updater.json"
        )
    );
    let report_text = fs::read_to_string(&context.report_path).unwrap();
    assert!(report_text.contains("execution_work_item_completion_conflict"));
    assert!(report_text.contains("QueueStateError"));
    assert!(report_text.contains("task task-conflict is not active"));

    let events = runtime_events(&paths);
    assert_eq!(
        events.last().unwrap()["event_type"],
        "runtime_post_stage_recovery_scheduled"
    );
    assert_eq!(
        events.last().unwrap()["data"]["error_code"],
        "execution_work_item_completion_conflict"
    );

    let recovery_request = run_serial_runtime_tick(
        &mut session,
        tick_options(
            "ignored-recovery-run",
            "request-execution-conflict-recovery",
        ),
    )
    .unwrap()
    .stage_request
    .unwrap();
    assert_eq!(recovery_request.stage, StageName::Troubleshooter);
    assert_eq!(recovery_request.node_id, custom_troubleshooter_node_id);
    assert_eq!(recovery_request.stage_kind_id, "troubleshooter");
    assert_eq!(
        recovery_request.runtime_error_code.as_deref(),
        Some("execution_work_item_completion_conflict")
    );
    assert_eq!(
        recovery_request.runtime_error_report_path.as_deref(),
        Some(context.report_path.as_str())
    );
    assert_eq!(
        recovery_request.runtime_error_catalog_path.as_deref(),
        Some(catalog_path.to_str().unwrap())
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_routes_planning_completion_conflict_into_mechanic_recovery() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let catalog_path = write_runtime_error_catalog(&paths.root);
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document("spec-conflict"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-planning-conflict")).unwrap();
    let inner = FakeRunner::new(
        FakeRunnerConfig::new(FakeRunnerResult::missing_terminal_output())
            .unwrap()
            .with_stage_result(
                StageName::Planner,
                FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"),
            )
            .with_stage_result(
                StageName::Manager,
                FakeRunnerResult::terminal_marker("### MANAGER_COMPLETE"),
            ),
    );
    let runner = PreemptiveCompletionRunner {
        paths: paths.clone(),
        inner,
        stage: StageName::Manager,
        work_item_kind: WorkItemKind::Spec,
        work_item_id: "spec-conflict",
    };

    run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-planning-conflict", "request-planning-conflict-planner"),
        &runner,
    )
    .unwrap();
    let manager = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-planning-conflict-manager"),
        &runner,
    )
    .unwrap();

    let decision = manager.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::RunStage);
    assert_eq!(decision.next_stage, Some(StageName::Mechanic));
    assert_eq!(
        decision.failure_class.as_deref(),
        Some("planning_work_item_completion_conflict")
    );
    assert_eq!(
        decision.reason,
        "runtime_exception:planning_work_item_completion_conflict"
    );
    assert!(paths.specs_done_dir.join("spec-conflict.md").is_file());
    assert!(!paths.specs_active_dir.join("spec-conflict.md").exists());
    assert_eq!(load_planning_status(&paths).unwrap(), "### BLOCKED");

    let snapshot = load_snapshot(&paths).unwrap();
    assert_eq!(snapshot.active_stage, Some(StageName::Mechanic));
    assert_eq!(snapshot.active_node_id.as_deref(), Some("mechanic"));
    assert_eq!(snapshot.active_stage_kind_id.as_deref(), Some("mechanic"));
    assert_eq!(
        snapshot.current_failure_class.as_deref(),
        Some("planning_work_item_completion_conflict")
    );

    assert_eq!(
        manager.runtime_error_context_path.as_ref().unwrap(),
        &paths.runtime_error_context_file
    );
    let context = RuntimeErrorContext::from_json_str(
        &fs::read_to_string(manager.runtime_error_context_path.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        context.error_code.as_str(),
        "planning_work_item_completion_conflict"
    );
    assert_eq!(context.failed_stage, StageName::Manager);
    assert_eq!(context.repair_stage, StageName::Mechanic);
    assert_eq!(context.router_action.as_deref(), Some("idle"));
    assert_eq!(
        context.terminal_result.unwrap().as_str(),
        "MANAGER_COMPLETE"
    );
    assert_eq!(
        context.stage_result_path.as_deref(),
        Some(
            "millrace-agents/runs/run-planning-conflict/stage_results/request-planning-conflict-manager.json"
        )
    );
    let report_text = fs::read_to_string(&context.report_path).unwrap();
    assert!(report_text.contains("planning_work_item_completion_conflict"));
    assert!(report_text.contains("QueueStateError"));
    assert!(report_text.contains("spec spec-conflict is not active"));

    let recovery_request = run_serial_runtime_tick(
        &mut session,
        tick_options("ignored-recovery-run", "request-planning-conflict-recovery"),
    )
    .unwrap()
    .stage_request
    .unwrap();
    assert_eq!(recovery_request.stage, StageName::Mechanic);
    assert_eq!(
        recovery_request.runtime_error_code.as_deref(),
        Some("planning_work_item_completion_conflict")
    );
    assert_eq!(
        recovery_request.runtime_error_report_path.as_deref(),
        Some(context.report_path.as_str())
    );
    assert_eq!(
        recovery_request.runtime_error_catalog_path.as_deref(),
        Some(catalog_path.to_str().unwrap())
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_dispatch_missing_terminal_routes_recovery_and_persists_error_context() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-missing"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-missing-terminal")).unwrap();
    let runner = FakeRunner::with_default(FakeRunnerResult::missing_terminal_output()).unwrap();
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-missing", "request-missing"),
        &runner,
    )
    .unwrap();

    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(stage_result.result_class, ResultClass::RecoverableFailure);
    assert_eq!(stage_result.terminal_result.as_str(), "BLOCKED");
    assert_eq!(
        stage_result.metadata["failure_class"],
        "missing_terminal_result"
    );
    assert_eq!(
        fs::read_to_string(outcome.terminal_marker_path.as_ref().unwrap()).unwrap(),
        "### BLOCKED\n"
    );

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::RunStage);
    assert_eq!(decision.next_stage, Some(StageName::Troubleshooter));
    assert_eq!(
        decision.failure_class.as_deref(),
        Some("missing_terminal_result")
    );
    assert_eq!(
        decision.counter_key.as_deref(),
        Some("task:task-missing:missing_terminal_result")
    );
    assert_eq!(
        outcome.runtime_error_context_path.as_ref().unwrap(),
        &paths.runtime_error_context_file
    );
    let context = RuntimeErrorContext::from_json_str(
        &fs::read_to_string(outcome.runtime_error_context_path.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        context.error_code.as_str(),
        "execution_post_stage_apply_failed"
    );
    assert_eq!(context.failed_stage, StageName::Builder);
    assert_eq!(context.repair_stage, StageName::Troubleshooter);
    assert_eq!(
        context.stage_result_path.as_deref(),
        Some("millrace-agents/runs/run-missing/stage_results/request-missing.json")
    );
    assert!(Path::new(&context.report_path).is_file());
    assert_eq!(load_execution_status(&paths).unwrap(), "### BLOCKED");
    let snapshot = load_snapshot(&paths).unwrap();
    assert_eq!(snapshot.active_stage, Some(StageName::Troubleshooter));
    assert_eq!(snapshot.troubleshoot_attempt_count, 1);
    let counters = load_recovery_counters(&paths).unwrap();
    assert_eq!(counters.entries.len(), 1);
    assert_eq!(counters.entries[0].work_item_id, "task-missing");
    assert_eq!(counters.entries[0].failure_class, "missing_terminal_result");
    assert_eq!(counters.entries[0].troubleshoot_attempt_count, 1);

    let events = runtime_events(&paths);
    assert_eq!(events[1]["event_type"], "stage_completed");
    assert_eq!(
        events[1]["data"]["failure_class"],
        "missing_terminal_result"
    );
    assert_eq!(events[2]["event_type"], "router_decision");
    assert_eq!(events[2]["data"]["next_stage"], "troubleshooter");

    let recovery_request = run_serial_runtime_tick(
        &mut session,
        tick_options("ignored-recovery-run", "request-missing-recovery"),
    )
    .unwrap()
    .stage_request
    .unwrap();
    assert_eq!(recovery_request.stage, StageName::Troubleshooter);
    assert_eq!(
        recovery_request.runtime_error_code.as_deref(),
        Some("execution_post_stage_apply_failed")
    );
    assert_eq!(
        recovery_request.runtime_error_report_path.as_deref(),
        Some(context.report_path.as_str())
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_applies_consultant_handoff_through_typed_incident_and_blocked_task() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-handoff")).unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-handoff")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    let active_since = timestamp("2026-04-28T20:11:00Z");
    let active_run = ActiveRunState {
        plane: Plane::Execution,
        stage: StageName::Consultant,
        node_id: "consultant".to_owned(),
        stage_kind_id: "consultant".to_owned(),
        run_id: "run-handoff".to_owned(),
        request_kind: ActiveRunRequestKind::ActiveWorkItem,
        work_item_kind: Some(WorkItemKind::Task),
        work_item_id: Some("task-handoff".to_owned()),
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        active_since: active_since.clone(),
        running_status_marker: None,
    };
    session
        .snapshot
        .active_runs_by_plane
        .insert(Plane::Execution, active_run);
    session.snapshot.active_plane = Some(Plane::Execution);
    session.snapshot.active_stage = Some(StageName::Consultant);
    session.snapshot.active_node_id = Some("consultant".to_owned());
    session.snapshot.active_stage_kind_id = Some("consultant".to_owned());
    session.snapshot.active_run_id = Some("run-handoff".to_owned());
    session.snapshot.active_work_item_kind = Some(WorkItemKind::Task);
    session.snapshot.active_work_item_id = Some("task-handoff".to_owned());
    session.snapshot.active_since = Some(active_since);
    save_snapshot(&paths, &session.snapshot).unwrap();

    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### NEEDS_PLANNING")).unwrap();
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-handoff-run", "request-handoff"),
        &runner,
    )
    .unwrap();

    assert_eq!(
        outcome.router_decision.as_ref().unwrap().action,
        RouterAction::Handoff
    );
    assert!(paths.tasks_blocked_dir.join("task-handoff.md").is_file());
    assert!(!paths.tasks_active_dir.join("task-handoff.md").exists());
    assert_eq!(session.snapshot.active_stage, None);
    assert_eq!(session.snapshot.queue_depth_planning, 1);

    let incident_path = fs::read_dir(&paths.incidents_incoming_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("md"))
        .unwrap();
    let incident = parse_incident_document(&fs::read_to_string(&incident_path).unwrap()).unwrap();
    assert!(incident.incident_id.starts_with("incident-task-handoff-"));
    assert_eq!(incident.root_spec_id.as_deref(), Some("spec-root-001"));
    assert_eq!(incident.source_task_id.as_deref(), Some("task-handoff"));
    assert_eq!(incident.source_stage, StageName::Consultant);
    assert_eq!(incident.failure_class, "consultant_needs_planning");
    assert_eq!(incident.related_run_ids, vec!["run-handoff".to_owned()]);
    assert_eq!(
        incident.related_stage_results,
        vec!["millrace-agents/runs/run-handoff/stage_results/request-handoff.json".to_owned()]
    );
    let blocked_metadata = load_blocked_task_metadata(&paths, "task-handoff")
        .unwrap()
        .expect("blocked handoff metadata");
    assert_eq!(blocked_metadata.work_item_id, "task-handoff");
    assert_eq!(
        blocked_metadata.root_spec_id.as_deref(),
        Some("spec-root-001")
    );
    assert_eq!(blocked_metadata.root_idea_id.as_deref(), Some("idea-001"));
    assert_eq!(blocked_metadata.blocked_origin.as_str(), "stage_terminal");
    assert_eq!(blocked_metadata.failure_scope.as_str(), "semantic");
    assert_eq!(blocked_metadata.failure_class, "stage_declared_blocked");
    assert!(!blocked_metadata.auto_requeue_candidate);
    assert_eq!(blocked_metadata.source_stage.as_deref(), Some("consultant"));
    assert_eq!(
        blocked_metadata.terminal_result.as_deref(),
        Some("NEEDS_PLANNING")
    );
    assert_eq!(
        blocked_metadata.stage_result_path.as_deref(),
        Some("millrace-agents/runs/run-handoff/stage_results/request-handoff.json")
    );
    let events = runtime_events(&paths);
    assert_eq!(events[3]["event_type"], "runtime_handoff_incident_enqueued");
    assert!(events.iter().any(
        |event| event["event_type"] == "blocked_item_metadata_written"
            && event["data"]["metadata_path"]
                == "millrace-agents/diagnostics/blocked/task-task-handoff.json"
    ));

    session.finish().unwrap();
}

#[test]
fn serial_tick_consultant_runner_failure_blocked_persists_retryable_metadata() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-network-blocked"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-network-blocked")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    let active_since = timestamp("2026-04-28T20:12:00Z");
    let active_run = ActiveRunState {
        plane: Plane::Execution,
        stage: StageName::Consultant,
        node_id: "consultant".to_owned(),
        stage_kind_id: "consultant".to_owned(),
        run_id: "run-network-blocked".to_owned(),
        request_kind: ActiveRunRequestKind::ActiveWorkItem,
        work_item_kind: Some(WorkItemKind::Task),
        work_item_id: Some("task-network-blocked".to_owned()),
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        active_since: active_since.clone(),
        running_status_marker: None,
    };
    session
        .snapshot
        .active_runs_by_plane
        .insert(Plane::Execution, active_run);
    session.snapshot.active_plane = Some(Plane::Execution);
    session.snapshot.active_stage = Some(StageName::Consultant);
    session.snapshot.active_node_id = Some("consultant".to_owned());
    session.snapshot.active_stage_kind_id = Some("consultant".to_owned());
    session.snapshot.active_run_id = Some("run-network-blocked".to_owned());
    session.snapshot.active_work_item_kind = Some(WorkItemKind::Task);
    session.snapshot.active_work_item_id = Some("task-network-blocked".to_owned());
    session.snapshot.active_since = Some(active_since);
    save_snapshot(&paths, &session.snapshot).unwrap();

    let mut runner_result = FakeRunnerResult::malformed_stdout("provider failed before terminal\n")
        .with_exit(RunnerExitKind::ProviderError, Some(1));
    runner_result.stderr = Some("could not resolve host api.openai.com\n".to_owned());
    let runner = FakeRunner::with_default(runner_result).unwrap();
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-network-run", "request-network-blocked"),
        &runner,
    )
    .unwrap();

    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(stage_result.result_class, ResultClass::RecoverableFailure);
    assert_eq!(
        stage_result.metadata["failure_class"],
        "network_unavailable"
    );
    assert_eq!(stage_result.metadata["blocked_origin"], "runner_failure");
    assert_eq!(stage_result.metadata["failure_scope"], "environment");
    assert_eq!(stage_result.metadata["auto_requeue_candidate"], true);
    assert_eq!(
        stage_result.metadata["failure_classifier_code"],
        "network_unavailable"
    );
    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Blocked);
    assert!(
        paths
            .tasks_blocked_dir
            .join("task-network-blocked.md")
            .is_file()
    );

    let metadata_path = blocked_task_metadata_path(&paths, "task-network-blocked");
    assert!(metadata_path.is_file());
    let blocked_metadata = load_blocked_task_metadata(&paths, "task-network-blocked")
        .unwrap()
        .expect("blocked runner failure metadata");
    assert!(blocked_metadata_allows_auto_requeue(Some(
        &blocked_metadata
    )));
    assert_eq!(blocked_metadata.failure_class, "network_unavailable");
    assert_eq!(blocked_metadata.blocked_origin.as_str(), "runner_failure");
    assert_eq!(blocked_metadata.failure_scope.as_str(), "environment");
    assert_eq!(
        blocked_metadata
            .failure_classifier_code
            .map(|code| code.as_str()),
        Some("network_unavailable")
    );
    assert_eq!(
        blocked_metadata.stage_result_path.as_deref(),
        Some("millrace-agents/runs/run-network-blocked/stage_results/request-network-blocked.json")
    );
    assert_eq!(
        blocked_metadata.stdout_path.as_deref(),
        Some("millrace-agents/runs/run-network-blocked/runner_stdout.request-network-blocked.txt")
    );
    assert_eq!(
        blocked_metadata.stderr_path.as_deref(),
        Some("millrace-agents/runs/run-network-blocked/runner_stderr.request-network-blocked.txt")
    );

    let events = runtime_events(&paths);
    assert!(events.iter().any(|event| {
        event["event_type"] == "blocked_item_metadata_written"
            && event["data"]["failure_class"] == "network_unavailable"
            && event["data"]["auto_requeue_candidate"] == true
    }));

    session.finish().unwrap();
}

#[test]
fn blocked_metadata_load_and_stranded_dependency_boundary_tolerates_missing_or_malformed_metadata()
{
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-blocked")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    queue.mark_task_blocked("task-blocked").unwrap();
    let mut dependent = task_document("task-dependent");
    dependent.depends_on = vec!["task-blocked".to_owned()];
    queue.enqueue_task(&dependent).unwrap();

    assert!(
        load_blocked_task_metadata(&paths, "task-blocked")
            .unwrap()
            .is_none()
    );
    let missing = find_stranded_blocked_dependency(&paths)
        .unwrap()
        .expect("stranded dependency without metadata");
    assert_eq!(missing.blocked_task_id, "task-blocked");
    assert_eq!(
        missing.queued_dependent_ids,
        vec!["task-dependent".to_owned()]
    );
    assert_eq!(missing.root_spec_id.as_deref(), Some("spec-root-001"));
    assert!(missing.metadata.is_none());

    fs::create_dir_all(
        blocked_task_metadata_path(&paths, "task-blocked")
            .parent()
            .unwrap(),
    )
    .unwrap();
    fs::write(
        blocked_task_metadata_path(&paths, "task-blocked"),
        "{ not valid json\n",
    )
    .unwrap();
    assert!(
        load_blocked_task_metadata(&paths, "task-blocked")
            .unwrap()
            .is_none()
    );
    let malformed = find_stranded_blocked_dependency(&paths)
        .unwrap()
        .expect("stranded dependency with malformed metadata");
    assert!(malformed.metadata.is_none());
}

#[test]
fn serial_tick_dispatch_illegal_terminal_routes_recovery_and_persists_error_context() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-illegal"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-illegal-terminal")).unwrap();
    let runner =
        FakeRunner::with_default(FakeRunnerResult::malformed_stdout("### NOT_A_TERMINAL\n"))
            .unwrap();
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-illegal", "request-illegal"),
        &runner,
    )
    .unwrap();

    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::StageDispatched);
    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(stage_result.result_class, ResultClass::RecoverableFailure);
    assert_eq!(stage_result.terminal_result.as_str(), "BLOCKED");
    assert_eq!(
        stage_result.metadata["failure_class"],
        "illegal_terminal_result"
    );
    assert_eq!(
        stage_result.metadata["raw_detected_marker"],
        "### NOT_A_TERMINAL"
    );
    assert_eq!(stage_result.detected_marker, None);
    assert_eq!(
        fs::read_to_string(outcome.terminal_marker_path.as_ref().unwrap()).unwrap(),
        "### BLOCKED\n"
    );
    let persisted_stage_result = StageResultEnvelope::from_json_str(
        &fs::read_to_string(outcome.stage_result_path.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted_stage_result, *stage_result);

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::RunStage);
    assert_eq!(decision.next_stage, Some(StageName::Troubleshooter));
    assert_eq!(
        decision.failure_class.as_deref(),
        Some("illegal_terminal_result")
    );
    assert_eq!(
        outcome.runtime_error_context_path.as_ref().unwrap(),
        &paths.runtime_error_context_file
    );
    let context = RuntimeErrorContext::from_json_str(
        &fs::read_to_string(outcome.runtime_error_context_path.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(context.failed_stage, StageName::Builder);
    assert_eq!(context.repair_stage, StageName::Troubleshooter);
    assert_eq!(
        context.stage_result_path.as_deref(),
        Some("millrace-agents/runs/run-illegal/stage_results/request-illegal.json")
    );
    assert!(Path::new(&context.report_path).is_file());
    assert_eq!(load_execution_status(&paths).unwrap(), "### BLOCKED");

    let events = runtime_events(&paths);
    assert_eq!(events[1]["event_type"], "stage_completed");
    assert_eq!(events[1]["data"]["terminal_result"], "BLOCKED");
    assert_eq!(events[1]["data"]["result_class"], "recoverable_failure");
    assert_eq!(
        events[1]["data"]["failure_class"],
        "illegal_terminal_result"
    );
    assert_eq!(events[2]["event_type"], "router_decision");
    assert_eq!(events[2]["data"]["next_stage"], "troubleshooter");
    assert_eq!(
        events[2]["data"]["failure_class"],
        "illegal_terminal_result"
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_activates_learning_request_only_when_learning_graph_exists() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_learning_request(&learning_request_document("learn-ready"))
        .unwrap();

    let mut options = startup_options("tick-learning");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_once_for_paths(&paths, options).unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-learning", "request-learning"),
    )
    .unwrap();

    let request = outcome.stage_request.unwrap();
    assert_eq!(request.request_kind, RequestKind::LearningRequest);
    assert_eq!(request.plane, Plane::Learning);
    assert_eq!(request.stage, StageName::Analyst);
    assert_eq!(
        request.active_work_item_kind,
        Some(WorkItemKind::LearningRequest)
    );
    assert_eq!(request.active_work_item_id.as_deref(), Some("learn-ready"));
    assert_eq!(load_learning_status(&paths).unwrap(), "### ANALYST_RUNNING");
    let evidence_path = request.skill_revision_evidence_path.unwrap();
    let evidence: Value =
        serde_json::from_str(&fs::read_to_string(evidence_path).unwrap()).unwrap();
    assert_eq!(evidence["kind"], "skill_revision_evidence");
    assert_eq!(evidence["request_id"], "request-learning");

    session.finish().unwrap();
}

#[test]
fn serial_tick_requeues_and_blocks_stage_work_item_ownership_mismatches() {
    assert_stage_work_item_ownership_guard_requeues(
        "ownership-execution-spec",
        Plane::Execution,
        StageName::Builder,
        ActiveRunRequestKind::ActiveWorkItem,
        WorkItemKind::Spec,
        "spec-ownership-execution",
        |queue| {
            queue
                .enqueue_spec(&spec_document("spec-ownership-execution"))
                .unwrap();
            queue.claim_next_planning_item(None).unwrap().unwrap();
        },
    );
    assert_stage_work_item_ownership_guard_requeues(
        "ownership-planning-task",
        Plane::Planning,
        StageName::Planner,
        ActiveRunRequestKind::ActiveWorkItem,
        WorkItemKind::Task,
        "task-ownership-planning",
        |queue| {
            queue
                .enqueue_task(&task_document("task-ownership-planning"))
                .unwrap();
            queue.claim_next_execution_task(None).unwrap().unwrap();
        },
    );
    assert_stage_work_item_ownership_guard_requeues(
        "ownership-recon-spec",
        Plane::Planning,
        StageName::Recon,
        ActiveRunRequestKind::ActiveWorkItem,
        WorkItemKind::Spec,
        "spec-ownership-recon",
        |queue| {
            queue
                .enqueue_spec(&spec_document("spec-ownership-recon"))
                .unwrap();
            queue.claim_next_planning_item(None).unwrap().unwrap();
        },
    );
    assert_stage_work_item_ownership_guard_requeues(
        "ownership-learning-task",
        Plane::Learning,
        StageName::Analyst,
        ActiveRunRequestKind::ActiveWorkItem,
        WorkItemKind::Task,
        "task-ownership-learning",
        |queue| {
            queue
                .enqueue_task(&task_document("task-ownership-learning"))
                .unwrap();
            queue.claim_next_execution_task(None).unwrap().unwrap();
        },
    );
}

#[test]
fn serial_tick_returns_no_work_paused_and_stopped_without_stage_request() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();

    let mut idle_session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-idle")).unwrap();
    let idle = run_serial_runtime_tick(&mut idle_session, tick_options("run-idle", "request-idle"))
        .unwrap();
    assert_eq!(idle.kind, RuntimeTickOutcomeKind::NoWork);
    assert!(idle.stage_request.is_none());
    assert_eq!(runtime_events(&paths)[0]["event_type"], "runtime_tick_idle");
    idle_session.finish().unwrap();

    let mut paused_session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-paused")).unwrap();
    paused_session.snapshot.paused = true;
    let paused = run_serial_runtime_tick(
        &mut paused_session,
        tick_options("run-paused", "request-paused"),
    )
    .unwrap();
    assert_eq!(paused.kind, RuntimeTickOutcomeKind::Paused);
    assert!(paused.stage_request.is_none());
    paused_session.finish().unwrap();

    let mut stopped_session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-stopped")).unwrap();
    stopped_session.snapshot.stop_requested = true;
    stopped_session.snapshot.paused = true;
    let stopped = run_serial_runtime_tick(
        &mut stopped_session,
        tick_options("run-stopped", "request-stopped"),
    )
    .unwrap();
    assert_eq!(stopped.kind, RuntimeTickOutcomeKind::Stopped);
    assert!(stopped.stage_request.is_none());
    assert!(!stopped.snapshot.process_running);
    assert!(!stopped.snapshot.stop_requested);
    assert!(!stopped.snapshot.paused);
    assert_eq!(load_execution_status(&paths).unwrap(), "### IDLE");
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Absent
    );
}

#[test]
fn serial_tick_opens_closure_target_when_root_spec_claim_activates() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    write_idea(
        &paths,
        "idea-001",
        "# Idea 001\n\nClaim-created closure target.\n",
    );
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document("spec-root-claim"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-root-claim")).unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-root-claim", "request-root-claim"),
    )
    .unwrap();

    let request = outcome.stage_request.unwrap();
    assert_eq!(request.request_kind, RequestKind::ActiveWorkItem);
    assert_eq!(request.active_work_item_kind, Some(WorkItemKind::Spec));
    assert_eq!(
        request.active_work_item_id.as_deref(),
        Some("spec-root-claim")
    );
    let target =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-root-claim").unwrap();
    assert!(target.closure_open);
    assert_eq!(target.root_idea_id, "idea-001");
    assert_eq!(
        target.root_spec_path,
        "millrace-agents/arbiter/contracts/root-specs/spec-root-claim.md"
    );
    assert_eq!(
        target.root_idea_path,
        "millrace-agents/arbiter/contracts/ideas/idea-001.md"
    );
    assert!(
        paths
            .arbiter_root_spec_contracts_dir
            .join("spec-root-claim.md")
            .is_file()
    );
    assert_eq!(
        fs::read_to_string(paths.arbiter_idea_contracts_dir.join("idea-001.md")).unwrap(),
        "# Idea 001\n\nClaim-created closure target.\n"
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_closure_target_prefers_durable_idea_source_over_legacy_references() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    write_idea(&paths, "idea-001", "# Idea 001\n\nTransient inbox copy.\n");
    millrace_ai::workspace::write_idea_source_artifact(
        &paths,
        "idea-001",
        "# Idea 001\n\nDurable runtime-owned copy.\n",
    )
    .unwrap();
    let mut spec = spec_document("spec-root-durable");
    spec.references = vec!["ideas/inbox/idea-001.md".to_owned()];
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec)
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-root-durable")).unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-root-durable", "request-root-durable"),
    )
    .unwrap();

    assert_eq!(
        outcome.stage_request.unwrap().request_kind,
        RequestKind::ActiveWorkItem
    );
    let target =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-root-durable").unwrap();
    assert!(target.closure_open);
    assert_eq!(
        fs::read_to_string(paths.arbiter_idea_contracts_dir.join("idea-001.md")).unwrap(),
        "# Idea 001\n\nDurable runtime-owned copy.\n"
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_closure_target_falls_back_to_legacy_idea_reference() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    write_idea(&paths, "idea-001", "# Idea 001\n\nLegacy reference copy.\n");
    let mut spec = spec_document("spec-root-legacy");
    spec.references = vec!["ideas/inbox/idea-001.md".to_owned()];
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec)
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-root-legacy")).unwrap();
    run_serial_runtime_tick(
        &mut session,
        tick_options("run-root-legacy", "request-root-legacy"),
    )
    .unwrap();

    let target =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-root-legacy").unwrap();
    assert!(target.closure_open);
    assert_eq!(
        fs::read_to_string(paths.arbiter_idea_contracts_dir.join("idea-001.md")).unwrap(),
        "# Idea 001\n\nLegacy reference copy.\n"
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_backfills_closure_target_from_done_root_spec() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    write_idea(
        &paths,
        "idea-001",
        "# Idea 001\n\nBackfilled closure target.\n",
    );
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_spec(&spec_document("spec-root-backfill"))
        .unwrap();
    queue.claim_next_planning_item(None).unwrap().unwrap();
    queue.mark_spec_done("spec-root-backfill").unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-backfill")).unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-backfill", "request-backfill"),
    )
    .unwrap();

    let request = outcome.stage_request.unwrap();
    assert_eq!(request.request_kind, RequestKind::ClosureTarget);
    assert_eq!(
        request.closure_target_root_spec_id.as_deref(),
        Some("spec-root-backfill")
    );
    assert_eq!(
        request.closure_target_root_idea_id.as_deref(),
        Some("idea-001")
    );
    let target =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-root-backfill").unwrap();
    assert!(target.closure_open);
    assert_eq!(
        target.root_spec_path,
        "millrace-agents/arbiter/contracts/root-specs/spec-root-backfill.md"
    );
    assert!(runtime_events(&paths).iter().any(|event| {
        event["event_type"] == "completion_behavior_target_backfilled"
            && event["data"]["root_spec_id"] == "spec-root-backfill"
    }));

    session.finish().unwrap();
}

#[test]
fn serial_tick_blocks_planning_when_backfill_root_idea_source_is_missing() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_spec(&spec_document("spec-root-missing-source"))
        .unwrap();
    queue.claim_next_planning_item(None).unwrap().unwrap();
    queue.mark_spec_done("spec-root-missing-source").unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-missing-source")).unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-missing-source", "request-missing-source"),
    )
    .unwrap();

    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::NoWork);
    assert!(outcome.stage_request.is_none());
    assert_eq!(load_planning_status(&paths).unwrap(), "### BLOCKED");
    assert_eq!(
        load_snapshot(&paths)
            .unwrap()
            .current_failure_class
            .as_deref(),
        Some("missing_root_idea_source")
    );
    let events = runtime_events(&paths);
    assert!(events.iter().any(|event| {
        event["event_type"] == "completion_behavior_blocked"
            && event["data"]["reason"] == "missing_root_idea_source"
            && event["data"]["spec_id"] == "spec-root-missing-source"
    }));
    assert!(events.iter().any(|event| {
        event["event_type"] == "root_idea_source_missing"
            && event["data"]["root_idea_id"] == "idea-001"
            && event["data"]["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .any(|candidate| {
                    candidate.as_str() == Some("millrace-agents/intake/ideas/idea-001.md")
                })
    }));

    session.finish().unwrap();
}

#[test]
fn serial_tick_activates_closure_target_request_without_active_work_item() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    save_closure_target_state(&paths, &closure_target_state("spec-root", "idea-root")).unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-closure")).unwrap();
    let outcome =
        run_serial_runtime_tick(&mut session, tick_options("run-closure", "request-closure"))
            .unwrap();

    let request = outcome.stage_request.unwrap();
    assert_eq!(request.request_kind, RequestKind::ClosureTarget);
    assert_eq!(request.plane, Plane::Planning);
    assert_eq!(request.stage, StageName::Arbiter);
    assert_eq!(request.active_work_item_kind, None);
    assert_eq!(request.active_work_item_id, None);
    assert!(request.active_work_item_path.is_none());
    assert_eq!(
        request.closure_target_root_spec_id.as_deref(),
        Some("spec-root")
    );
    assert_eq!(
        request.closure_target_root_idea_id.as_deref(),
        Some("idea-root")
    );
    assert!(
        request
            .closure_target_path
            .as_deref()
            .unwrap()
            .ends_with("millrace-agents/arbiter/targets/spec-root.json")
    );
    assert_eq!(
        request.canonical_root_spec_path.as_deref(),
        Some("millrace-agents/arbiter/contracts/root-specs/spec-root.md")
    );
    assert_eq!(
        request.canonical_seed_idea_path.as_deref(),
        Some("millrace-agents/arbiter/contracts/ideas/idea-root.md")
    );
    assert_eq!(
        request.preferred_rubric_path.as_deref(),
        Some("millrace-agents/arbiter/rubrics/spec-root.md")
    );
    assert!(
        request
            .preferred_verdict_path
            .as_deref()
            .unwrap()
            .ends_with("millrace-agents/arbiter/verdicts/spec-root.json")
    );
    assert_eq!(
        request.preferred_report_path.as_deref(),
        Some(
            paths
                .runs_dir
                .join("run-closure")
                .join("arbiter_report.md")
                .to_str()
                .unwrap()
        )
    );
    let active_run = session
        .snapshot
        .active_runs_by_plane
        .get(&Plane::Planning)
        .unwrap();
    assert_eq!(active_run.request_kind, ActiveRunRequestKind::ClosureTarget);
    assert_eq!(active_run.work_item_kind, None);
    assert_eq!(load_planning_status(&paths).unwrap(), "### ARBITER_RUNNING");

    session.finish().unwrap();
}

#[test]
fn serial_tick_suppresses_closure_target_when_queued_lineage_work_remains() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    save_closure_target_state(&paths, &closure_target_state("spec-root-001", "idea-001")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    let mut queued_task = task_document("task-queued-lineage");
    queued_task.depends_on = vec!["task-unfinished-prereq".to_owned()];
    queue.enqueue_task(&queued_task).unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-closure-queued")).unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-closure-queued", "request-closure-queued"),
    )
    .unwrap();

    let target =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-root-001").unwrap();
    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::NoWork);
    assert!(outcome.stage_request.is_none());
    assert!(target.closure_open);
    assert!(target.closure_blocked_by_lineage_work);
    assert_eq!(
        target.blocking_work_ids,
        vec!["task-queued-lineage".to_owned()]
    );
    assert!(
        paths
            .tasks_queue_dir
            .join("task-queued-lineage.md")
            .is_file()
    );
    assert!(
        !paths
            .tasks_active_dir
            .join("task-queued-lineage.md")
            .exists()
    );
    assert_eq!(runtime_events(&paths)[0]["event_type"], "runtime_tick_idle");

    session.finish().unwrap();
}

#[test]
fn serial_tick_suppresses_closure_target_when_blocked_lineage_work_remains() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    save_closure_target_state(&paths, &closure_target_state("spec-root-001", "idea-001")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-blocked")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    queue.mark_task_blocked("task-blocked").unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-closure-blocked")).unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-closure-blocked", "request-closure-blocked"),
    )
    .unwrap();

    let target =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-root-001").unwrap();
    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::NoWork);
    assert!(outcome.stage_request.is_none());
    assert!(target.closure_open);
    assert!(target.closure_blocked_by_lineage_work);
    assert_eq!(target.blocking_work_ids, vec!["task-blocked".to_owned()]);
    assert_eq!(runtime_events(&paths)[0]["event_type"], "runtime_tick_idle");

    session.finish().unwrap();
}

#[test]
fn serial_tick_blocked_closure_target_allows_unrelated_root_spec_to_activate() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-blocked")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    queue.mark_task_blocked("task-blocked").unwrap();

    let mut blocked_target = closure_target_state("spec-root-001", "idea-001");
    blocked_target.closure_blocked_by_lineage_work = true;
    blocked_target.blocking_work_ids = vec!["task-blocked".to_owned()];
    save_closure_target_state(&paths, &blocked_target).unwrap();
    write_idea(
        &paths,
        "idea-002",
        "# Idea 002\n\nUnrelated root spec can start.\n",
    );
    queue
        .enqueue_spec(&spec_document_for_lineage("spec-root-002", "idea-002"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-closure-blocked-unrelated"))
            .unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-unrelated-root", "request-unrelated-root"),
    )
    .unwrap();

    let request = outcome.stage_request.unwrap();
    assert_eq!(request.request_kind, RequestKind::ActiveWorkItem);
    assert_eq!(request.plane, Plane::Planning);
    assert_eq!(request.stage, StageName::Planner);
    assert_eq!(request.active_work_item_kind, Some(WorkItemKind::Spec));
    assert_eq!(
        request.active_work_item_id.as_deref(),
        Some("spec-root-002")
    );
    assert!(paths.specs_active_dir.join("spec-root-002.md").is_file());
    let blocked =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-root-001").unwrap();
    let unrelated =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-root-002").unwrap();
    assert!(blocked.closure_open);
    assert!(blocked.closure_blocked_by_lineage_work);
    assert!(unrelated.closure_open);
    assert!(!unrelated.closure_blocked_by_lineage_work);

    session.finish().unwrap();
}

#[test]
fn serial_tick_refuses_multiple_actionable_open_closure_targets() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    save_closure_target_state(&paths, &closure_target_state("spec-root-001", "idea-001")).unwrap();
    save_closure_target_state(&paths, &closure_target_state("spec-root-002", "idea-002")).unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-closure-ambiguous")).unwrap();
    let error = run_serial_runtime_tick(
        &mut session,
        tick_options("run-ambiguous", "request-ambiguous"),
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("multiple actionable open closure targets found")
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_reports_active_spec_and_blocked_incident_lineage_ids_before_arbiter() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    save_closure_target_state(&paths, &closure_target_state("spec-root-001", "idea-001")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());

    queue
        .enqueue_incident(&incident_document("incident-blocked-lineage"))
        .unwrap();
    queue
        .claim_next_planning_item(Some("spec-root-001"))
        .unwrap()
        .unwrap();
    queue
        .mark_incident_blocked("incident-blocked-lineage")
        .unwrap();

    let mut active_spec = spec_document("spec-active-lineage");
    active_spec.root_spec_id = Some("spec-root-001".to_owned());
    queue.enqueue_spec(&active_spec).unwrap();
    queue
        .claim_next_planning_item(Some("spec-root-001"))
        .unwrap()
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-lineage-ids")).unwrap();
    session.reconciliation.planning.is_stale = false;
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-lineage-ids", "request-lineage-ids"),
    )
    .unwrap();

    let target =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-root-001").unwrap();
    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::NoWork);
    assert!(outcome.stage_request.is_none());
    assert!(target.closure_blocked_by_lineage_work);
    assert_eq!(
        target.blocking_work_ids,
        vec![
            "incident-blocked-lineage".to_owned(),
            "spec-active-lineage".to_owned()
        ]
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_blocks_closure_target_on_lineage_drift_diagnostic() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let canonical_root = "idea-idea-browser-local-qa";
    let stale_root = "idea-browser-local-qa";
    save_closure_target_state(
        &paths,
        &closure_target_state(canonical_root, canonical_root),
    )
    .unwrap();
    let mut drifted_task = task_document("task-drifted-lineage");
    drifted_task.root_spec_id = Some(stale_root.to_owned());
    drifted_task.spec_id = Some(stale_root.to_owned());
    drifted_task.root_idea_id = Some(canonical_root.to_owned());
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&drifted_task)
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-lineage-drift")).unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options("run-lineage-drift", "request-lineage-drift"),
    )
    .unwrap();

    let snapshot = load_snapshot(&paths).unwrap();
    let target = millrace_ai::workspace::load_closure_target_state(&paths, canonical_root).unwrap();
    let diagnostic_path = paths
        .arbiter_dir
        .join("diagnostics")
        .join("lineage-drift")
        .join(format!("{canonical_root}.json"));
    let diagnostic: Value =
        serde_json::from_str(&fs::read_to_string(&diagnostic_path).unwrap()).unwrap();
    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::NoWork);
    assert!(outcome.stage_request.is_none());
    assert_eq!(snapshot.planning_status_marker, "### BLOCKED");
    assert_eq!(
        snapshot.current_failure_class.as_deref(),
        Some("closure_lineage_drift")
    );
    assert!(target.closure_blocked_by_lineage_work);
    assert_eq!(
        target.blocking_work_ids,
        vec!["task-drifted-lineage".to_owned()]
    );
    assert_eq!(diagnostic["kind"], "closure_lineage_drift_diagnostic");
    assert_eq!(diagnostic["findings"][0]["actual_root_spec_id"], stale_root);
    assert!(runtime_events(&paths).iter().any(|event| {
        event["event_type"] == "closure_lineage_drift_detected"
            && event["data"]["root_spec_id"] == canonical_root
    }));

    session.finish().unwrap();
}

#[test]
fn serial_tick_closes_closure_target_on_arbiter_complete() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    save_closure_target_state(&paths, &closure_target_state("spec-root-001", "idea-001")).unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-closure-complete")).unwrap();
    let runner = ArbiterArtifactRunner {
        terminal_marker: "### ARBITER_COMPLETE",
        verdict_json: "{\"status\":\"pass\"}\n",
        report_text: "# Arbiter Report\n\nParity holds.\n",
    };
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-closure-complete", "request-closure-complete"),
        &runner,
    )
    .unwrap();

    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(
        stage_result.terminal_result,
        TerminalResult::Planning(PlanningTerminalResult::ArbiterComplete)
    );
    assert_eq!(
        outcome.router_decision.as_ref().unwrap().action,
        RouterAction::Idle
    );
    assert_eq!(
        outcome.router_decision.as_ref().unwrap().reason,
        "arbiter_complete"
    );
    let target =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-root-001").unwrap();
    assert!(!target.closure_open);
    assert_eq!(target.closed_at, Some(stage_result.completed_at.clone()));
    assert_eq!(
        target.last_arbiter_run_id.as_deref(),
        Some("run-closure-complete")
    );
    assert_eq!(
        target.latest_verdict_path.as_deref(),
        Some("millrace-agents/arbiter/verdicts/spec-root-001.json")
    );
    assert_eq!(
        target.latest_report_path.as_deref(),
        Some("millrace-agents/arbiter/reports/run-closure-complete.md")
    );
    assert!(
        paths
            .arbiter_reports_dir
            .join("run-closure-complete.md")
            .is_file()
    );
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.active_stage.is_none());
    assert_eq!(load_planning_status(&paths).unwrap(), "### IDLE");
    assert!(
        runtime_events(&paths)
            .iter()
            .any(|event| event["event_type"] == "closure_target_closed")
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_enqueues_remediation_incident_for_arbiter_gap() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    save_closure_target_state(&paths, &closure_target_state("spec-root-gap", "idea-gap")).unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-closure-gap")).unwrap();
    let runner = ArbiterArtifactRunner {
        terminal_marker: "### REMEDIATION_NEEDED",
        verdict_json: "{\"status\":\"gap\"}\n",
        report_text: "# Arbiter Report\n\nParity gaps remain.\n",
    };
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-closure-gap", "request-closure-gap"),
        &runner,
    )
    .unwrap();

    let target =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-root-gap").unwrap();
    assert!(target.closure_open);
    assert_eq!(target.closed_at, None);
    assert_eq!(
        target.last_arbiter_run_id.as_deref(),
        Some("run-closure-gap")
    );
    assert_eq!(
        target.latest_verdict_path.as_deref(),
        Some("millrace-agents/arbiter/verdicts/spec-root-gap.json")
    );
    assert_eq!(
        target.latest_report_path.as_deref(),
        Some("millrace-agents/arbiter/reports/run-closure-gap.md")
    );
    assert_eq!(
        outcome.router_decision.as_ref().unwrap().reason,
        "arbiter_remediation_needed"
    );

    let incident_paths: Vec<PathBuf> = fs::read_dir(&paths.incidents_incoming_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(incident_paths.len(), 1);
    let incident_text = fs::read_to_string(&incident_paths[0]).unwrap();
    let incident = parse_incident_document(&incident_text).unwrap();
    assert_eq!(incident.failure_class, "arbiter_parity_gap");
    assert_eq!(incident.root_spec_id.as_deref(), Some("spec-root-gap"));
    assert_eq!(incident.root_idea_id.as_deref(), Some("idea-gap"));
    assert_eq!(incident.source_stage, StageName::Arbiter);
    assert_eq!(incident.trigger_reason, "arbiter_remediation_needed");
    assert!(
        incident
            .evidence_paths
            .iter()
            .any(|path| path.ends_with("arbiter_report.md"))
    );
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.active_stage.is_none());
    assert_eq!(
        snapshot.current_failure_class.as_deref(),
        Some("arbiter_parity_gap")
    );
    assert_eq!(
        load_planning_status(&paths).unwrap(),
        "### REMEDIATION_NEEDED"
    );
    assert!(
        runtime_events(&paths)
            .iter()
            .any(|event| event["event_type"] == "closure_target_remediation_requested")
    );

    session.finish().unwrap();
}

#[test]
fn serial_tick_blocks_repeated_arbiter_remediation_without_execution() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let mut target = closure_target_state("spec-root-repeat", "idea-repeat");
    target.last_arbiter_run_id = Some("run-previous-arbiter".to_owned());
    save_closure_target_state(&paths, &target).unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-closure-repeat")).unwrap();
    let runner = ArbiterArtifactRunner {
        terminal_marker: "### REMEDIATION_NEEDED",
        verdict_json: "{\"status\":\"gap\"}\n",
        report_text: "# Arbiter Report\n\nParity gaps still remain.\n",
    };
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-closure-repeat", "request-closure-repeat"),
        &runner,
    )
    .unwrap();

    assert_eq!(
        outcome.router_decision.as_ref().unwrap().reason,
        "arbiter_remediation_needed"
    );
    assert_eq!(
        fs::read_dir(&paths.incidents_incoming_dir).unwrap().count(),
        0
    );
    let target =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-root-repeat").unwrap();
    assert!(target.closure_open);
    assert_eq!(
        target.last_arbiter_run_id.as_deref(),
        Some("run-closure-repeat")
    );
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.active_stage.is_none());
    assert_eq!(snapshot.planning_status_marker, "### BLOCKED");
    assert_eq!(
        snapshot.current_failure_class.as_deref(),
        Some("closure_repeated_remediation_without_execution")
    );
    assert_eq!(load_planning_status(&paths).unwrap(), "### BLOCKED");
    assert!(runtime_events(&paths).iter().any(|event| {
        event["event_type"] == "closure_repeated_remediation_blocked"
            && event["data"]["root_spec_id"] == "spec-root-repeat"
    }));

    session.finish().unwrap();
}

#[test]
fn e2e_direct_task_handoff_happy_path_uses_runtime_queue_and_status_transitions() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-e2e-direct"))
        .unwrap();

    let runner = ScriptedE2eRunner::new(vec![
        (
            StageName::Builder,
            ScriptedStageOutput::Terminal("### BUILDER_COMPLETE"),
        ),
        (
            StageName::Checker,
            ScriptedStageOutput::Terminal("### CHECKER_PASS"),
        ),
        (
            StageName::Updater,
            ScriptedStageOutput::Terminal("### UPDATE_COMPLETE"),
        ),
    ]);
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-e2e-direct")).unwrap();

    let first = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-e2e-direct", "request-e2e-direct-builder"),
        &runner,
    )
    .unwrap();
    let second = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-e2e-direct-checker"),
        &runner,
    )
    .unwrap();
    let third = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-e2e-direct-updater"),
        &runner,
    )
    .unwrap();

    assert_eq!(
        [
            first.stage_result.as_ref().unwrap().stage,
            second.stage_result.as_ref().unwrap().stage,
            third.stage_result.as_ref().unwrap().stage,
        ],
        [StageName::Builder, StageName::Checker, StageName::Updater]
    );
    assert_eq!(
        runner.stage_order(),
        vec![StageName::Builder, StageName::Checker, StageName::Updater]
    );
    assert!(paths.tasks_done_dir.join("task-e2e-direct.md").is_file());
    assert!(!paths.tasks_active_dir.join("task-e2e-direct.md").exists());
    assert!(!paths.tasks_queue_dir.join("task-e2e-direct.md").exists());
    assert_eq!(load_execution_status(&paths).unwrap(), "### IDLE");

    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.active_stage.is_none());
    assert!(snapshot.active_work_item_id.is_none());
    assert!(snapshot.active_runs_by_plane.is_empty());
    assert_eq!(snapshot.queue_depth_execution, 0);

    session.finish().unwrap();
}

#[test]
fn e2e_repair_loop_fix_needed_cycle_preserves_fix_evidence_and_finishes() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-e2e-fix"))
        .unwrap();
    let run_dir = paths.runs_dir.join("run-e2e-fix");
    let fix_contract_path = run_dir.join("fix_contract.md");
    let fix_contract_text = "# Fix Contract: task-e2e-fix\n\n\
## Issue Found\n\
Checker found deterministic missing repair evidence in the scripted handoff.\n\n\
## Required Fix\n\
Fixer must preserve the fix contract before Doublechecker confirms the repair.\n";

    let runner = ScriptedE2eRunner::new(vec![
        (
            StageName::Builder,
            ScriptedStageOutput::Terminal("### BUILDER_COMPLETE"),
        ),
        (
            StageName::Checker,
            ScriptedStageOutput::TerminalWithFixContract {
                marker: "### FIX_NEEDED",
                contract_text: fix_contract_text,
            },
        ),
        (
            StageName::Fixer,
            ScriptedStageOutput::Terminal("### FIXER_COMPLETE"),
        ),
        (
            StageName::Doublechecker,
            ScriptedStageOutput::Terminal("### DOUBLECHECK_PASS"),
        ),
        (
            StageName::Updater,
            ScriptedStageOutput::Terminal("### UPDATE_COMPLETE"),
        ),
    ]);
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-e2e-fix")).unwrap();

    run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-e2e-fix", "request-e2e-fix-builder"),
        &runner,
    )
    .unwrap();
    let checker = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-e2e-fix-checker"),
        &runner,
    )
    .unwrap();
    let checker_stage_result_path = checker.stage_result_path.clone().unwrap();
    let checker_stage_result = checker.stage_result.as_ref().unwrap();
    assert_eq!(checker_stage_result.terminal_result.as_str(), "FIX_NEEDED");
    assert_eq!(checker_stage_result.run_id, "run-e2e-fix");
    assert!(
        checker_stage_result
            .artifact_paths
            .iter()
            .any(|path| path == "fix_contract.md")
    );
    assert_eq!(
        checker.router_decision.as_ref().unwrap().next_stage,
        Some(StageName::Fixer)
    );
    assert!(fix_contract_path.is_file());
    let persisted_fix_contract = fs::read_to_string(&fix_contract_path).unwrap();
    assert_eq!(persisted_fix_contract, fix_contract_text);
    assert!(persisted_fix_contract.contains("## Issue Found"));
    assert!(persisted_fix_contract.contains("deterministic missing repair evidence"));
    assert!(persisted_fix_contract.contains("## Required Fix"));
    assert!(persisted_fix_contract.contains("preserve the fix contract"));
    let counters_after_checker = load_recovery_counters(&paths).unwrap();
    assert_eq!(counters_after_checker.entries.len(), 1);
    assert_eq!(
        counters_after_checker.entries[0].work_item_id,
        "task-e2e-fix"
    );
    assert_eq!(counters_after_checker.entries[0].fix_cycle_count, 1);
    let snapshot_after_checker = load_snapshot(&paths).unwrap();
    let execution_run = snapshot_after_checker
        .active_runs_by_plane
        .get(&Plane::Execution)
        .unwrap();
    assert_eq!(execution_run.run_id, "run-e2e-fix");
    assert_eq!(execution_run.stage, StageName::Fixer);

    run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-e2e-fix-fixer"),
        &runner,
    )
    .unwrap();
    run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-e2e-fix-doublechecker"),
        &runner,
    )
    .unwrap();
    run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("ignored-run", "request-e2e-fix-updater"),
        &runner,
    )
    .unwrap();

    assert_eq!(
        runner.stage_order(),
        vec![
            StageName::Builder,
            StageName::Checker,
            StageName::Fixer,
            StageName::Doublechecker,
            StageName::Updater,
        ]
    );
    let fixer_request = runner
        .requests()
        .into_iter()
        .find(|request| request.stage == StageName::Fixer)
        .unwrap();
    assert_eq!(
        fixer_request.active_work_item_id.as_deref(),
        Some("task-e2e-fix")
    );
    assert!(
        fixer_request
            .legal_terminal_markers
            .iter()
            .any(|marker| marker == "### FIXER_COMPLETE")
    );
    assert_eq!(Path::new(&fixer_request.run_dir), run_dir);
    assert!(
        Path::new(&fixer_request.run_dir)
            .join("fix_contract.md")
            .is_file()
    );
    assert!(Path::new(&fixer_request.runtime_snapshot_path).is_file());
    assert!(Path::new(&fixer_request.recovery_counters_path).is_file());
    assert!(checker_stage_result_path.is_file());
    assert!(paths.tasks_done_dir.join("task-e2e-fix.md").is_file());
    assert!(load_recovery_counters(&paths).unwrap().entries.is_empty());
    assert_eq!(load_execution_status(&paths).unwrap(), "### IDLE");

    session.finish().unwrap();
}

fn assert_e2e_recovery_routes_to_consultant_with_incident_evidence(
    task_id: &str,
    builder_output: ScriptedStageOutput,
    expected_failure_class: &str,
) {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document(task_id))
        .unwrap();

    let runner = ScriptedE2eRunner::new(vec![
        (StageName::Builder, builder_output),
        (
            StageName::Troubleshooter,
            ScriptedStageOutput::Terminal("### BLOCKED"),
        ),
        (
            StageName::Consultant,
            ScriptedStageOutput::Terminal("### NEEDS_PLANNING"),
        ),
    ]);
    let mut session = startup_runtime_once_for_paths(
        &paths,
        startup_options(&format!("tick-e2e-recovery-{task_id}")),
    )
    .unwrap();

    let builder = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(
            &format!("run-e2e-recovery-{task_id}"),
            &format!("request-e2e-recovery-{task_id}-builder"),
        ),
        &runner,
    )
    .unwrap();
    let builder_stage_result = builder.stage_result.as_ref().unwrap();
    assert_eq!(
        builder_stage_result.result_class,
        ResultClass::RecoverableFailure
    );
    assert_eq!(
        builder_stage_result.metadata["failure_class"],
        expected_failure_class
    );
    let builder_stage_result_path = builder.stage_result_path.as_ref().unwrap();
    let persisted_builder =
        StageResultEnvelope::from_json_str(&fs::read_to_string(builder_stage_result_path).unwrap())
            .unwrap();
    assert_eq!(persisted_builder, *builder_stage_result);

    let context = RuntimeErrorContext::from_json_str(
        &fs::read_to_string(&paths.runtime_error_context_file).unwrap(),
    )
    .unwrap();
    assert_eq!(
        context.error_code.as_str(),
        "execution_post_stage_apply_failed"
    );
    assert_eq!(context.failed_stage, StageName::Builder);
    assert_eq!(context.repair_stage, StageName::Troubleshooter);
    assert_eq!(context.exception_message, expected_failure_class);
    let expected_stage_result_path = format!(
        "millrace-agents/runs/run-e2e-recovery-{task_id}/stage_results/request-e2e-recovery-{task_id}-builder.json"
    );
    assert_eq!(
        context.stage_result_path.as_deref(),
        Some(expected_stage_result_path.as_str())
    );
    assert!(Path::new(&context.report_path).is_file());

    run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(
            "ignored-run",
            &format!("request-e2e-recovery-{task_id}-troubleshooter-1"),
        ),
        &runner,
    )
    .unwrap();
    let second_recovery = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(
            "ignored-run",
            &format!("request-e2e-recovery-{task_id}-troubleshooter-2"),
        ),
        &runner,
    )
    .unwrap();
    assert_eq!(
        second_recovery.router_decision.as_ref().unwrap().next_stage,
        Some(StageName::Consultant)
    );
    let snapshot_before_handoff = load_snapshot(&paths).unwrap();
    assert_eq!(
        snapshot_before_handoff.active_stage,
        Some(StageName::Consultant)
    );
    assert_eq!(
        snapshot_before_handoff.current_failure_class.as_deref(),
        Some(expected_failure_class)
    );
    let counters = load_recovery_counters(&paths).unwrap();
    assert_eq!(counters.entries.len(), 1);
    assert_eq!(counters.entries[0].work_item_id, task_id);
    assert_eq!(counters.entries[0].failure_class, expected_failure_class);
    assert_eq!(counters.entries[0].troubleshoot_attempt_count, 2);
    assert_eq!(counters.entries[0].consultant_invocations, 1);

    let consultant = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(
            "ignored-run",
            &format!("request-e2e-recovery-{task_id}-consultant"),
        ),
        &runner,
    )
    .unwrap();
    assert_eq!(
        consultant.router_decision.as_ref().unwrap().action,
        RouterAction::Handoff
    );
    assert!(
        paths
            .tasks_blocked_dir
            .join(format!("{task_id}.md"))
            .is_file()
    );

    let incident = first_incident_document(&paths);
    assert_eq!(incident.failure_class, "consultant_needs_planning");
    assert_eq!(incident.source_task_id.as_deref(), Some(task_id));
    assert_eq!(incident.source_stage, StageName::Consultant);
    assert_eq!(incident.related_run_ids.len(), 1);
    assert_eq!(incident.related_stage_results.len(), 1);
    assert!(
        paths
            .root
            .join(&incident.related_stage_results[0])
            .is_file()
    );
    assert!(
        incident
            .evidence_paths
            .iter()
            .any(|path| paths.root.join(path).is_file())
    );
    assert!(
        runtime_events(&paths)
            .iter()
            .any(|event| event["event_type"] == "runtime_handoff_incident_enqueued")
    );

    session.finish().unwrap();
}

#[test]
fn e2e_recovery_malformed_result_routes_to_consultant_with_incident_evidence() {
    assert_e2e_recovery_routes_to_consultant_with_incident_evidence(
        "task-e2e-malformed",
        ScriptedStageOutput::Stdout("no terminal token\n"),
        "missing_terminal_result",
    );
}

#[test]
fn e2e_recovery_illegal_terminal_result_routes_to_consultant_with_incident_evidence() {
    assert_e2e_recovery_routes_to_consultant_with_incident_evidence(
        "task-e2e-illegal",
        ScriptedStageOutput::Stdout("### NOT_A_TERMINAL\n"),
        "illegal_terminal_result",
    );
}

#[test]
fn e2e_needs_planning_incident_intake_reenters_execution_preserving_lineage() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    let mut incident = incident_document("incident-e2e-planning");
    incident.source_task_id = Some("task-e2e-recover-source".to_owned());
    incident.source_spec_id = None;
    queue.enqueue_incident(&incident).unwrap();

    let mut remediation_task = task_document("task-e2e-remediate");
    remediation_task.incident_id = Some("incident-e2e-planning".to_owned());
    let runner = ScriptedE2eRunner::new(standard_e2e_outputs())
        .with_manager_task(paths.clone(), remediation_task);
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-e2e-planning")).unwrap();

    for (run_id, request_id) in [
        ("run-e2e-planning", "request-e2e-planning-auditor"),
        ("ignored-run", "request-e2e-planning-planner"),
        ("ignored-run", "request-e2e-planning-manager"),
        ("ignored-run", "request-e2e-planning-builder"),
        ("ignored-run", "request-e2e-planning-checker"),
        ("ignored-run", "request-e2e-planning-updater"),
    ] {
        run_serial_runtime_tick_with_runner(
            &mut session,
            tick_options(run_id, request_id),
            &runner,
        )
        .unwrap();
    }

    assert_eq!(
        runner.stage_order(),
        vec![
            StageName::Auditor,
            StageName::Planner,
            StageName::Manager,
            StageName::Builder,
            StageName::Checker,
            StageName::Updater,
        ]
    );
    assert!(
        paths
            .incidents_resolved_dir
            .join("incident-e2e-planning.md")
            .is_file()
    );
    assert!(paths.tasks_done_dir.join("task-e2e-remediate.md").is_file());
    let task_text = fs::read_to_string(paths.tasks_done_dir.join("task-e2e-remediate.md")).unwrap();
    assert!(task_text.contains("Root-Idea-ID: idea-001\n"));
    assert!(task_text.contains("Root-Spec-ID: spec-root-001\n"));
    assert!(task_text.contains("Incident-ID: incident-e2e-planning\n"));
    assert_eq!(load_planning_status(&paths).unwrap(), "### IDLE");
    assert_eq!(load_execution_status(&paths).unwrap(), "### IDLE");
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.active_runs_by_plane.is_empty());
    assert_eq!(snapshot.queue_depth_planning, 0);
    assert_eq!(snapshot.queue_depth_execution, 0);

    session.finish().unwrap();
}

#[test]
fn e2e_lineage_drain_triggers_arbiter_complete_and_closes_target() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    write_idea(
        &paths,
        "idea-e2e-complete",
        "# idea-e2e-complete\n\nClosure completion seed.\n",
    );
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document_for_lineage(
            "spec-e2e-complete",
            "idea-e2e-complete",
        ))
        .unwrap();

    let mut outputs = standard_e2e_outputs();
    outputs.push((
        StageName::Arbiter,
        ScriptedStageOutput::Terminal("### ARBITER_COMPLETE"),
    ));
    let runner = ScriptedE2eRunner::new(outputs)
        .with_manager_task(
            paths.clone(),
            task_document_for_lineage(
                "task-e2e-complete",
                "spec-e2e-complete",
                "idea-e2e-complete",
            ),
        )
        .with_arbiter_artifacts(ScriptedArbiterArtifacts {
            verdict_json: "{\"status\":\"pass\"}\n",
            report_text: "# Arbiter Report\n\nParity holds.\n",
        });
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-e2e-complete")).unwrap();

    for (run_id, request_id) in [
        ("run-e2e-complete", "request-e2e-complete-planner"),
        ("ignored-run", "request-e2e-complete-manager"),
        ("ignored-run", "request-e2e-complete-builder"),
        ("ignored-run", "request-e2e-complete-checker"),
        ("ignored-run", "request-e2e-complete-updater"),
        ("run-e2e-complete-arbiter", "request-e2e-complete-arbiter"),
    ] {
        run_serial_runtime_tick_with_runner(
            &mut session,
            tick_options(run_id, request_id),
            &runner,
        )
        .unwrap();
    }

    assert_eq!(
        runner.stage_order(),
        vec![
            StageName::Planner,
            StageName::Manager,
            StageName::Builder,
            StageName::Checker,
            StageName::Updater,
            StageName::Arbiter,
        ]
    );
    let arbiter_request = runner
        .requests()
        .into_iter()
        .find(|request| request.stage == StageName::Arbiter)
        .unwrap();
    assert_eq!(arbiter_request.request_kind, RequestKind::ClosureTarget);
    assert_eq!(arbiter_request.active_work_item_kind, None);
    assert_eq!(
        arbiter_request.closure_target_root_spec_id.as_deref(),
        Some("spec-e2e-complete")
    );
    let target =
        millrace_ai::workspace::load_closure_target_state(&paths, "spec-e2e-complete").unwrap();
    assert!(!target.closure_open);
    assert!(target.closed_at.is_some());
    assert!(paths.specs_done_dir.join("spec-e2e-complete.md").is_file());
    assert!(paths.tasks_done_dir.join("task-e2e-complete.md").is_file());
    assert_eq!(load_planning_status(&paths).unwrap(), "### IDLE");

    session.finish().unwrap();
}

#[test]
fn e2e_lineage_drain_triggers_arbiter_remediation_gap_and_blocks_repeat() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    write_idea(
        &paths,
        "idea-e2e-gap",
        "# idea-e2e-gap\n\nClosure remediation seed.\n",
    );
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document_for_lineage("spec-e2e-gap", "idea-e2e-gap"))
        .unwrap();

    let mut outputs = standard_e2e_outputs();
    outputs.push((
        StageName::Arbiter,
        ScriptedStageOutput::Terminal("### REMEDIATION_NEEDED"),
    ));
    let runner = ScriptedE2eRunner::new(outputs)
        .with_manager_task(
            paths.clone(),
            task_document_for_lineage("task-e2e-gap", "spec-e2e-gap", "idea-e2e-gap"),
        )
        .with_arbiter_artifacts(ScriptedArbiterArtifacts {
            verdict_json: "{\"status\":\"gap\"}\n",
            report_text: "# Arbiter Report\n\nParity gaps remain.\n",
        });
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("tick-e2e-gap")).unwrap();

    for (run_id, request_id) in [
        ("run-e2e-gap", "request-e2e-gap-planner"),
        ("ignored-run", "request-e2e-gap-manager"),
        ("ignored-run", "request-e2e-gap-builder"),
        ("ignored-run", "request-e2e-gap-checker"),
        ("ignored-run", "request-e2e-gap-updater"),
        ("run-e2e-gap-arbiter", "request-e2e-gap-arbiter"),
    ] {
        run_serial_runtime_tick_with_runner(
            &mut session,
            tick_options(run_id, request_id),
            &runner,
        )
        .unwrap();
    }

    let target = millrace_ai::workspace::load_closure_target_state(&paths, "spec-e2e-gap").unwrap();
    assert!(target.closure_open);
    assert!(target.closed_at.is_none());
    assert_eq!(
        target.last_arbiter_run_id.as_deref(),
        Some("run-e2e-gap-arbiter")
    );
    assert_eq!(
        load_planning_status(&paths).unwrap(),
        "### REMEDIATION_NEEDED"
    );
    let incident = first_incident_document(&paths);
    assert_eq!(incident.failure_class, "arbiter_parity_gap");
    assert_eq!(incident.root_spec_id.as_deref(), Some("spec-e2e-gap"));
    assert_eq!(incident.root_idea_id.as_deref(), Some("idea-e2e-gap"));
    assert_eq!(incident.source_stage, StageName::Arbiter);
    assert!(
        incident
            .evidence_paths
            .iter()
            .any(|path| path.ends_with("arbiter_report.md"))
    );

    session.finish().unwrap();

    let repeat_paths = initialize_workspace(temp.path().join("repeat-workspace")).unwrap();
    let mut repeat_target = closure_target_state("spec-e2e-repeat", "idea-e2e-repeat");
    repeat_target.last_arbiter_run_id = Some("run-previous-arbiter".to_owned());
    save_closure_target_state(&repeat_paths, &repeat_target).unwrap();
    let repeat_runner = ScriptedE2eRunner::new(vec![(
        StageName::Arbiter,
        ScriptedStageOutput::Terminal("### REMEDIATION_NEEDED"),
    )])
    .with_arbiter_artifacts(ScriptedArbiterArtifacts {
        verdict_json: "{\"status\":\"gap\"}\n",
        report_text: "# Arbiter Report\n\nParity gaps still remain.\n",
    });
    let mut repeat_session = startup_runtime_once_for_paths(
        &repeat_paths,
        startup_options("tick-e2e-repeat-remediation"),
    )
    .unwrap();
    run_serial_runtime_tick_with_runner(
        &mut repeat_session,
        tick_options(
            "run-e2e-repeat-remediation",
            "request-e2e-repeat-remediation",
        ),
        &repeat_runner,
    )
    .unwrap();

    assert_eq!(repeat_runner.stage_order(), vec![StageName::Arbiter]);
    assert_eq!(
        fs::read_dir(&repeat_paths.incidents_incoming_dir)
            .unwrap()
            .count(),
        0
    );
    let repeat_snapshot = load_snapshot(&repeat_paths).unwrap();
    assert_eq!(repeat_snapshot.planning_status_marker, "### BLOCKED");
    assert_eq!(
        repeat_snapshot.current_failure_class.as_deref(),
        Some("closure_repeated_remediation_without_execution")
    );
    assert!(runtime_events(&repeat_paths).iter().any(|event| {
        event["event_type"] == "closure_repeated_remediation_blocked"
            && event["data"]["root_spec_id"] == "spec-e2e-repeat"
    }));

    repeat_session.finish().unwrap();
}
