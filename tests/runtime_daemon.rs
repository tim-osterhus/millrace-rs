mod support;

use std::{cell::RefCell, collections::BTreeSet, fs, io, path::Path, process, rc::Rc};

use serde_json::{Map, Value, json};
use tempfile::TempDir;

use millrace_ai::contracts::{
    ActiveRunRequestKind, ActiveRunState, BlockedDependencyAutoRecoveryDiagnostic,
    LearningRequestAction, LearningRequestDocument, LearningStageName, LearningTerminalResult,
    MailboxCommand, MailboxCommandEnvelope, Plane, ProbeDocument, ReloadOutcome, ResultClass,
    RunTraceGraph, RuntimeJsonContract, RuntimeMode, SpecDocument, SpecSourceType, StageName,
    TaskDocument, TerminalResult, Timestamp, TokenUsage, WatcherMode, WorkItemKind,
};
use millrace_ai::work_documents::{
    parse_learning_request_document, parse_spec_document, parse_task_document, render_task_document,
};
use millrace_ai::workspace::{
    QueueStore, RuntimeOwnershipLockOptions, RuntimeOwnershipLockState,
    acquire_runtime_ownership_lock_with_options, initialize_workspace,
    inspect_runtime_ownership_lock, load_execution_status, load_learning_status,
    load_planning_status, load_snapshot, save_snapshot, write_mailbox_command,
};
use millrace_ai::{
    BasicTerminalMonitor, CodexCliRunnerAdapter, FakeRunner, FakeRunnerConfig, FakeRunnerOutput,
    FakeRunnerResult, PiRpcRunnerAdapter, RuntimeConfigApplyBoundary, RuntimeDaemonLoopExitReason,
    RuntimeDaemonLoopOptions, RuntimeDaemonSleeper, RuntimeDaemonSupervisor, RuntimeMonitorEvent,
    RuntimeMonitorFanout, RuntimeStartupError, RuntimeStartupOptions, RuntimeTickOptions,
    RuntimeTickResult, apply_stage_worker_outcome, blocked_task_metadata_path,
    load_runtime_startup_config, run_runtime_daemon_loop,
    run_runtime_daemon_supervisor_loop_with_sleeper,
    run_runtime_daemon_supervisor_loop_with_sleeper_and_monitor, run_serial_runtime_tick,
    run_stage_worker, runtime_config_apply_boundary_for_field, runtime_monitor_events_from_jsonl,
    startup_runtime_daemon, startup_runtime_daemon_for_paths,
};
use support::parity::read_json_fixture;

const STARTUP_NOW: &str = "2026-04-29T02:10:00Z";

fn lock_options(session_id: &str) -> RuntimeOwnershipLockOptions {
    RuntimeOwnershipLockOptions::new(process::id(), "test-host", session_id, STARTUP_NOW).unwrap()
}

fn daemon_options(session_id: &str) -> RuntimeStartupOptions {
    RuntimeStartupOptions {
        runtime_mode: RuntimeMode::Daemon,
        lock_options: Some(lock_options(session_id)),
        now: Some(millrace_ai::contracts::Timestamp::parse("updated_at", STARTUP_NOW).unwrap()),
        ..RuntimeStartupOptions::default()
    }
}

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("created_at", value).unwrap()
}

fn task_document(task_id: &str) -> TaskDocument {
    TaskDocument {
        task_id: task_id.to_owned(),
        title: format!("Task {task_id}"),
        summary: "daemon startup test".to_owned(),
        root_idea_id: Some("idea-daemon".to_owned()),
        root_spec_id: Some("spec-daemon".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        spec_id: Some("spec-daemon".to_owned()),
        parent_task_id: None,
        incident_id: None,
        target_paths: vec!["src/runtime/".to_owned()],
        acceptance: vec!["daemon startup preserves queued work".to_owned()],
        required_checks: vec!["cargo test --test runtime_daemon".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/runtime/lifecycle.py".to_owned()],
        risk: vec!["daemon startup must not claim work".to_owned()],
        depends_on: Vec::new(),
        blocks: Vec::new(),
        tags: vec!["runtime".to_owned(), "daemon".to_owned()],
        status_hint: None,
        created_at: timestamp(STARTUP_NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn task_document_with_dependency(task_id: &str, dependency_id: &str) -> TaskDocument {
    let mut document = task_document(task_id);
    document.depends_on = vec![dependency_id.to_owned()];
    document
}

fn write_blocked_recovery_metadata(
    paths: &millrace_ai::WorkspacePaths,
    task_id: &str,
    blocked_at: &str,
    failure_class: &str,
    auto_requeue_candidate: bool,
) {
    let path = blocked_task_metadata_path(paths, task_id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        serde_json::to_string_pretty(&json!({
            "work_item_kind": "task",
            "work_item_id": task_id,
            "root_spec_id": "spec-daemon",
            "root_idea_id": "idea-daemon",
            "blocked_at": blocked_at,
            "blocked_origin": "runner_failure",
            "failure_class": failure_class,
            "failure_scope": "environment",
            "auto_requeue_candidate": auto_requeue_candidate,
            "failure_classifier_code": null,
            "source_run_id": "run-blocked",
            "source_plane": "execution",
            "source_stage": "builder",
            "terminal_result": "BLOCKED",
            "stage_result_path": "millrace-agents/runs/run-blocked/stage_results/request-blocked.json",
            "stdout_path": null,
            "stderr_path": null
        }))
        .unwrap()
            + "\n",
    )
    .unwrap();
}

fn probe_document(probe_id: &str) -> ProbeDocument {
    ProbeDocument {
        probe_id: probe_id.to_owned(),
        title: format!("Probe {probe_id}"),
        summary: "daemon supervisor probe test".to_owned(),
        request: "Research the current codebase and route the smallest safe change.".to_owned(),
        target_paths: vec!["src/runtime/".to_owned()],
        constraints: vec!["Do not implement during recon.".to_owned()],
        acceptance: vec!["recon routes the probe".to_owned()],
        risk_notes: vec!["mailbox probe intake can drift".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/runtime/mailbox_intake.py".to_owned()],
        tags: vec!["probe".to_owned(), "daemon".to_owned()],
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
        summary: "daemon supervisor planning test".to_owned(),
        source_type: SpecSourceType::Idea,
        source_id: Some("idea-daemon".to_owned()),
        parent_spec_id: None,
        root_idea_id: Some("idea-daemon".to_owned()),
        root_spec_id: Some(spec_id.to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        goals: vec!["plan daemon runtime work".to_owned()],
        non_goals: Vec::new(),
        scope: vec!["runtime supervisor".to_owned()],
        constraints: vec!["use fake runner".to_owned()],
        assumptions: Vec::new(),
        risks: vec!["foreground concurrency drift".to_owned()],
        target_paths: vec!["src/runtime/".to_owned()],
        entrypoints: Vec::new(),
        required_skills: Vec::new(),
        decomposition_hints: Vec::new(),
        acceptance: vec!["planner activates first".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/runtime/supervisor.py".to_owned()],
        created_at: timestamp(STARTUP_NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn learning_request_document(learning_request_id: &str) -> LearningRequestDocument {
    LearningRequestDocument {
        learning_request_id: learning_request_id.to_owned(),
        title: format!("Learning request {learning_request_id}"),
        summary: "daemon supervisor learning lane".to_owned(),
        requested_action: LearningRequestAction::Improve,
        target_skill_id: Some("builder-core".to_owned()),
        target_stage: None,
        source_refs: vec!["run:run-daemon".to_owned()],
        preferred_output_paths: Vec::new(),
        trigger_metadata: json!({"source": "daemon_supervisor_test"}),
        originating_run_ids: vec!["run-daemon".to_owned()],
        artifact_paths: Vec::new(),
        references: Vec::new(),
        created_at: timestamp(STARTUP_NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn runtime_tick_options() -> RuntimeTickOptions {
    RuntimeTickOptions {
        now: Some(timestamp("2026-04-29T02:15:00Z")),
        run_id: None,
        request_id: None,
    }
}

fn runtime_tick_at(value: &str) -> RuntimeTickOptions {
    RuntimeTickOptions {
        now: Some(timestamp(value)),
        run_id: None,
        request_id: None,
    }
}

fn fixed_tick_options(run_id: &str, request_id: &str) -> RuntimeTickOptions {
    RuntimeTickOptions {
        now: Some(timestamp("2026-04-29T02:15:00Z")),
        run_id: Some(run_id.to_owned()),
        request_id: Some(request_id.to_owned()),
    }
}

#[test]
fn runtime_daemon_v0_18_4_guardrail_fixture_requires_auto_recovery_idle_cycle_surface() {
    let fixture = read_json_fixture("runtime_json/auto_port_v0_18_4_runtime_contract_scout.json");
    assert_eq!(fixture["kind"], "auto_port_v0_18_4_runtime_contract_scout");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.4");
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.4");

    let auto_recovery = &fixture["auto_recovery_contract"];
    assert_eq!(
        auto_recovery["diagnostics_path"],
        "millrace-agents/diagnostics/auto-recovery/<TIMESTAMP>-<TASK_ID>.json"
    );
    assert_eq!(
        auto_recovery["diagnostics_kind"],
        "blocked_dependency_auto_recovery"
    );
    assert_eq!(
        auto_recovery["default_policy"]["cooldown_seconds"],
        json!([300, 900, 3600])
    );
    assert_eq!(
        auto_recovery["default_policy"]["max_auto_requeues_per_work_item"],
        json!(3)
    );

    let eligible: BTreeSet<_> = auto_recovery["eligible_failure_classes"]
        .as_array()
        .expect("eligible failure classes are present")
        .iter()
        .map(|value| value.as_str().expect("eligible failure class"))
        .collect();
    assert_eq!(
        eligible,
        BTreeSet::from([
            "network_unavailable",
            "provider_unavailable",
            "provider_rate_limited",
            "runner_timeout",
        ])
    );

    let skip_reasons: BTreeSet<_> = auto_recovery["skip_reasons"]
        .as_array()
        .expect("skip reasons are present")
        .iter()
        .map(|value| value.as_str().expect("skip reason"))
        .collect();
    for reason in [
        "disabled",
        "blocked_dependency_retry_disabled",
        "paused",
        "stop_requested",
        "active_runs_present",
        "no_queued_execution_dependents",
        "blocked_dependency_not_retryable",
        "retry_budget_exhausted",
        "cooldown_active",
        "missing_or_invalid_metadata",
        "root_spec_mismatch",
    ] {
        assert!(
            skip_reasons.contains(reason),
            "missing v0.18.4 daemon auto-recovery skip reason {reason}"
        );
    }

    let event_types: BTreeSet<_> = auto_recovery["event_types"]
        .as_array()
        .expect("auto recovery event types are present")
        .iter()
        .map(|value| value.as_str().expect("event type"))
        .collect();
    assert!(event_types.contains("blocked_dependency_auto_requeued"));
    assert!(event_types.contains("blocked_dependency_auto_requeue_skipped"));

    let monitor_events: BTreeSet<_> = auto_recovery["monitor_events"]
        .as_array()
        .expect("auto recovery monitor events are present")
        .iter()
        .map(|value| value.as_str().expect("monitor event"))
        .collect();
    assert!(monitor_events.contains("blocked_dependency_auto_requeued"));
    assert!(monitor_events.contains("blocked_lineage_requires_operator_review"));
}

#[test]
fn runtime_daemon_v0_18_6_guardrail_fixture_requires_intervention_mailbox_and_durable_idea_source_surfaces()
 {
    let fixture = read_json_fixture("runtime_json/auto_port_v0_18_6_runtime_contract_scout.json");
    assert_eq!(fixture["kind"], "auto_port_v0_18_6_runtime_contract_scout");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.6");
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.5");

    let daemon = &fixture["daemon_mailbox_contract"];
    let applied_commands: BTreeSet<_> = daemon["applied_commands"]
        .as_array()
        .expect("daemon applied commands are present")
        .iter()
        .map(|value| value.as_str().expect("applied command"))
        .collect();
    assert_eq!(
        applied_commands,
        BTreeSet::from([
            "archive_blocked_task",
            "archive_invalid_incident",
            "cancel_incident",
            "cancel_work_item",
            "resolve_incident",
            "retarget_task_dependency",
            "supersede_task",
        ])
    );
    assert_eq!(
        daemon["processed_archive"],
        "millrace-agents/state/mailbox/processed/<COMMAND_ID>.json"
    );
    assert_eq!(
        daemon["failed_archive"],
        "millrace-agents/state/mailbox/failed/<COMMAND_ID>.json"
    );
    assert_eq!(daemon["defer_reason"], "active_runtime_stage");
    assert_eq!(
        daemon["applied_event"],
        "mailbox_operator_intervention_applied"
    );
    assert_eq!(daemon["deferred_event"], "operator_intervention_deferred");

    let monitor_events: BTreeSet<_> = daemon["monitor_events"]
        .as_array()
        .expect("daemon monitor events are present")
        .iter()
        .map(|value| value.as_str().expect("monitor event"))
        .collect();
    for event_type in [
        "operator_intervention_deferred",
        "mailbox_operator_intervention_applied",
        "work_item_cancelled",
        "task_superseded",
        "incident_cancelled",
        "invalid_incident_artifact_archived",
    ] {
        assert!(
            monitor_events.contains(event_type),
            "missing v0.18.6 daemon intervention monitor event {event_type}"
        );
    }

    let read_only = &fixture["read_only_intervention_contract"];
    assert_eq!(
        read_only["status_keys"],
        json!(["latest_operator_intervention"])
    );
    for field in [
        "event_type",
        "occurred_at",
        "work_item_kind",
        "work_item_id",
        "destination_path",
    ] {
        assert!(
            read_only["latest_operator_intervention_fields"]
                .as_array()
                .expect("latest operator intervention fields are present")
                .iter()
                .any(|value| value.as_str() == Some(field)),
            "missing latest operator intervention status field {field}"
        );
    }

    let durable = &fixture["durable_idea_source_contract"];
    assert_eq!(
        durable["durable_source_path_template"],
        "millrace-agents/intake/ideas/<root_idea_id>.md"
    );
    assert_eq!(durable["watcher_event"], "idea_normalized_to_spec");
    assert_eq!(
        durable["watcher_failure_event"],
        "idea_source_artifact_write_failed"
    );
    assert_eq!(
        durable["spec_reference_order"],
        json!([
            "millrace-agents/intake/ideas/<root_idea_id>.md",
            "ideas/inbox/<idea_file>.md"
        ])
    );
    assert_eq!(
        durable["closure_source_preference"],
        json!([
            "durable idea source artifact",
            "legacy spec references",
            "transient ideas/inbox source"
        ])
    );
    assert_eq!(
        durable["missing_source_failure_class"],
        "missing_root_idea_source"
    );
    assert_eq!(durable["missing_source_event"], "root_idea_source_missing");
    assert_eq!(durable["blocked_status_marker"], "### BLOCKED");
}

#[derive(Debug, Default)]
struct RecordingSleeper {
    calls: Vec<f64>,
}

impl RuntimeDaemonSleeper for RecordingSleeper {
    fn sleep(&mut self, seconds: f64) -> RuntimeTickResult<()> {
        self.calls.push(seconds);
        Ok(())
    }
}

fn supervisor_runner() -> FakeRunner {
    let config = FakeRunnerConfig::new(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"))
        .unwrap()
        .with_stage_result(
            StageName::Integrator,
            FakeRunnerResult::terminal_marker("### INTEGRATION_COMPLETE"),
        )
        .with_stage_result(
            StageName::Checker,
            FakeRunnerResult::terminal_marker("### CHECKER_PASS"),
        )
        .with_stage_result(
            StageName::Planner,
            FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"),
        )
        .with_stage_result(
            StageName::Updater,
            FakeRunnerResult::terminal_marker("### UPDATE_COMPLETE"),
        )
        .with_stage_result(
            StageName::Analyst,
            FakeRunnerResult::terminal_marker("### ANALYST_COMPLETE"),
        );
    FakeRunner::new(config)
}

fn governance_supervisor_runner() -> FakeRunner {
    let token_usage = TokenUsage {
        input_tokens: 100,
        cached_input_tokens: 0,
        output_tokens: 25,
        thinking_tokens: 0,
        total_tokens: 125,
    };
    let config = FakeRunnerConfig::new(
        FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE")
            .with_token_usage(token_usage.clone()),
    )
    .unwrap()
    .with_stage_result(
        StageName::Updater,
        FakeRunnerResult::terminal_marker("### UPDATE_COMPLETE").with_token_usage(token_usage),
    );
    FakeRunner::new(config)
}

#[test]
fn daemon_idle_cycle_auto_requeues_one_retryable_stranded_blocked_dependency() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-06")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    queue.mark_task_blocked("task-06").unwrap();
    write_blocked_recovery_metadata(
        &paths,
        "task-06",
        "2026-04-29T02:00:00Z",
        "network_unavailable",
        true,
    );
    queue
        .enqueue_task(&task_document_with_dependency("task-07", "task-06"))
        .unwrap();

    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("supervisor-auto-recovery"))
            .unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());

    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(outcome.kind, millrace_ai::RuntimeTickOutcomeKind::Recovered);
    assert_eq!(outcome.reason, "blocked_dependency_auto_requeued");
    assert_eq!(outcome.dispatched_count, 0);
    assert!(supervisor.active_worker_planes().is_empty());
    assert!(session.snapshot.active_runs_by_plane.is_empty());
    assert!(paths.tasks_queue_dir.join("task-06.md").is_file());
    assert!(paths.tasks_queue_dir.join("task-07.md").is_file());
    assert!(!paths.tasks_active_dir.join("task-07.md").exists());
    assert!(!paths.tasks_blocked_dir.join("task-06.md").exists());
    assert_eq!(session.snapshot.queue_depth_execution, 2);

    let audit = read_json_lines(&paths.tasks_queue_dir.join("task-06.requeue.jsonl"));
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["actor"], "runtime-daemon");
    assert_eq!(audit[0]["auto"], true);
    assert_eq!(audit[0]["attempt_number"], 1);
    assert_eq!(audit[0]["failure_class"], "network_unavailable");

    let diagnostics_dir = paths.runtime_root.join("diagnostics").join("auto-recovery");
    let diagnostics_path = diagnostics_dir.join("20260429T021500Z-task-06.json");
    let diagnostic = BlockedDependencyAutoRecoveryDiagnostic::from_json_str(
        &fs::read_to_string(&diagnostics_path).unwrap(),
    )
    .unwrap();
    assert_eq!(diagnostic.decision, "requeue");
    assert_eq!(diagnostic.reason, "transient blocked dependency");
    assert_eq!(diagnostic.blocked_task_id, "task-06");
    assert_eq!(diagnostic.queued_dependent_ids, vec!["task-07".to_owned()]);
    assert_eq!(diagnostic.root_spec_id.as_deref(), Some("spec-daemon"));
    assert_eq!(diagnostic.auto_attempt_number, 1);
    assert_eq!(diagnostic.pre_recovery_snapshot.queue_depth_execution, 1);

    let events = runtime_events(&paths);
    assert!(events.iter().any(|event| {
        event["event_type"] == "blocked_task_requeued"
            && event["data"]["task_id"] == "task-06"
            && event["data"]["auto"] == true
    }));
    assert!(events.iter().any(|event| {
        event["event_type"] == "blocked_dependency_auto_requeued"
            && event["data"]["task_id"] == "task-06"
            && event["data"]["queued_dependents"] == json!(["task-07"])
            && event["data"]["diagnostics_path"]
                == "millrace-agents/diagnostics/auto-recovery/20260429T021500Z-task-06.json"
    }));
    assert!(
        events
            .iter()
            .all(|event| event["event_type"] != "runtime_tick_idle")
    );

    let raw_events = fs::read_to_string(paths.logs_dir.join("runtime_events.jsonl")).unwrap();
    let monitor_events = runtime_monitor_events_from_jsonl(&raw_events).unwrap();
    let mut output = Vec::new();
    {
        let mut monitor = BasicTerminalMonitor::new(&mut output);
        for event in &monitor_events {
            monitor.emit(event).unwrap();
        }
    }
    let rendered = String::from_utf8(output).unwrap();
    assert!(rendered.contains("blocked dependency auto-requeued task=task-06"));

    let next = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();
    assert_eq!(
        next.kind,
        millrace_ai::RuntimeTickOutcomeKind::StageRequestReady
    );
    assert!(paths.tasks_active_dir.join("task-06.md").is_file());
    assert!(paths.tasks_queue_dir.join("task-07.md").is_file());

    session.close().unwrap();
}

#[test]
fn daemon_idle_cycle_skips_blocked_dependency_auto_recovery_when_review_is_required() {
    struct Case {
        name: &'static str,
        metadata: Option<(&'static str, &'static str, bool)>,
        requeue_attempts: u64,
        blocked_root_spec_id: &'static str,
        expected_reason: &'static str,
    }

    let cases = [
        Case {
            name: "missing-metadata",
            metadata: None,
            requeue_attempts: 0,
            blocked_root_spec_id: "spec-daemon",
            expected_reason: "missing_or_invalid_metadata",
        },
        Case {
            name: "non-retryable",
            metadata: Some(("2026-04-29T02:00:00Z", "stage_declared_blocked", false)),
            requeue_attempts: 0,
            blocked_root_spec_id: "spec-daemon",
            expected_reason: "blocked_dependency_not_retryable",
        },
        Case {
            name: "budget-exhausted",
            metadata: Some(("2026-04-29T02:00:00Z", "network_unavailable", true)),
            requeue_attempts: 3,
            blocked_root_spec_id: "spec-daemon",
            expected_reason: "retry_budget_exhausted",
        },
        Case {
            name: "cooldown-active",
            metadata: Some(("2026-04-29T02:14:00Z", "network_unavailable", true)),
            requeue_attempts: 0,
            blocked_root_spec_id: "spec-daemon",
            expected_reason: "cooldown_active",
        },
        Case {
            name: "root-mismatch",
            metadata: Some(("2026-04-29T02:00:00Z", "network_unavailable", true)),
            requeue_attempts: 0,
            blocked_root_spec_id: "spec-other",
            expected_reason: "root_spec_mismatch",
        },
    ];

    for case in cases {
        let temp = TempDir::new().unwrap();
        let paths = initialize_workspace(temp.path().join(case.name)).unwrap();
        let queue = QueueStore::from_paths(paths.clone());
        let mut blocked = task_document("task-06");
        blocked.root_spec_id = Some(case.blocked_root_spec_id.to_owned());
        blocked.spec_id = Some(case.blocked_root_spec_id.to_owned());
        queue.enqueue_task(&blocked).unwrap();
        queue.claim_next_execution_task(None).unwrap().unwrap();
        queue.mark_task_blocked("task-06").unwrap();
        if let Some((blocked_at, failure_class, auto_requeue_candidate)) = case.metadata {
            write_blocked_recovery_metadata(
                &paths,
                "task-06",
                blocked_at,
                failure_class,
                auto_requeue_candidate,
            );
        }
        let mut dependent = task_document_with_dependency("task-07", "task-06");
        dependent.root_spec_id = Some("spec-daemon".to_owned());
        dependent.spec_id = Some("spec-daemon".to_owned());
        queue.enqueue_task(&dependent).unwrap();
        if case.requeue_attempts > 0 {
            let lines = (0..case.requeue_attempts)
                .map(|attempt| {
                    json!({
                        "at": STARTUP_NOW,
                        "kind": "task",
                        "source_state": "blocked",
                        "destination_state": "queue",
                        "actor": "runtime-daemon",
                        "auto": true,
                        "reason": "prior auto retry",
                        "failure_class": "network_unavailable",
                        "attempt_number": attempt + 1
                    })
                    .to_string()
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            fs::write(paths.tasks_queue_dir.join("task-06.requeue.jsonl"), lines).unwrap();
        }

        let mut session =
            startup_runtime_daemon_for_paths(&paths, daemon_options(case.name)).unwrap();
        let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
        let outcome = supervisor
            .run_cycle(&mut session, runtime_tick_options())
            .unwrap();

        assert_eq!(outcome.kind, millrace_ai::RuntimeTickOutcomeKind::NoWork);
        assert!(paths.tasks_blocked_dir.join("task-06.md").is_file());
        assert!(!paths.tasks_queue_dir.join("task-06.md").exists());

        let events = runtime_events(&paths);
        assert!(events.iter().any(|event| {
            event["event_type"] == "blocked_dependency_auto_requeue_skipped"
                && event["data"]["task_id"] == "task-06"
                && event["data"]["reason"] == case.expected_reason
        }));
        assert!(events.iter().any(|event| {
            event["event_type"] == "blocked_lineage_requires_operator_review"
                && event["data"]["task_id"] == "task-06"
                && event["data"]["reason"] == case.expected_reason
        }));
        assert!(
            events
                .iter()
                .all(|event| event["event_type"] != "blocked_dependency_auto_requeued")
        );
        let diagnostics =
            fs::read_dir(paths.runtime_root.join("diagnostics").join("auto-recovery"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = BlockedDependencyAutoRecoveryDiagnostic::from_json_str(
            &fs::read_to_string(&diagnostics[0]).unwrap(),
        )
        .unwrap();
        assert_eq!(diagnostic.decision, "skip");
        assert_eq!(diagnostic.reason, case.expected_reason);

        let raw_events = fs::read_to_string(paths.logs_dir.join("runtime_events.jsonl")).unwrap();
        let monitor_events = runtime_monitor_events_from_jsonl(&raw_events).unwrap();
        assert!(monitor_events.iter().any(|event| {
            event.event_type == "blocked_lineage_requires_operator_review"
                && event.payload["reason"] == case.expected_reason
        }));

        session.close().unwrap();
    }
}

#[test]
fn daemon_idle_cycle_does_not_auto_recover_when_config_disables_the_gate() {
    for (name, config) in [
        (
            "auto-recovery-disabled",
            "[auto_recovery]\nenabled = false\nblocked_dependency_retry_enabled = true\n",
        ),
        (
            "blocked-dependency-retry-disabled",
            "[auto_recovery]\nenabled = true\nblocked_dependency_retry_enabled = false\n",
        ),
    ] {
        let temp = TempDir::new().unwrap();
        let paths = initialize_workspace(temp.path().join(name)).unwrap();
        fs::write(&paths.runtime_config_file, config).unwrap();
        let queue = QueueStore::from_paths(paths.clone());
        queue.enqueue_task(&task_document("task-06")).unwrap();
        queue.claim_next_execution_task(None).unwrap().unwrap();
        queue.mark_task_blocked("task-06").unwrap();
        write_blocked_recovery_metadata(
            &paths,
            "task-06",
            "2026-04-29T02:00:00Z",
            "network_unavailable",
            true,
        );
        queue
            .enqueue_task(&task_document_with_dependency("task-07", "task-06"))
            .unwrap();

        let mut session = startup_runtime_daemon_for_paths(&paths, daemon_options(name)).unwrap();
        let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
        let outcome = supervisor
            .run_cycle(&mut session, runtime_tick_options())
            .unwrap();

        assert_eq!(outcome.kind, millrace_ai::RuntimeTickOutcomeKind::NoWork);
        assert!(paths.tasks_blocked_dir.join("task-06.md").is_file());
        assert!(!paths.tasks_queue_dir.join("task-06.md").exists());
        assert!(runtime_events(&paths).iter().all(|event| !matches!(
            event["event_type"].as_str(),
            Some("blocked_dependency_auto_requeued" | "blocked_dependency_auto_requeue_skipped")
        )));
        assert!(
            !paths
                .runtime_root
                .join("diagnostics")
                .join("auto-recovery")
                .exists()
        );

        session.close().unwrap();
    }
}

fn write_daemon_session_governance_config(paths: &millrace_ai::WorkspacePaths) {
    fs::write(
        &paths.runtime_config_file,
        [
            "[runtime]",
            "default_mode = \"default_codex\"",
            "run_style = \"daemon\"",
            "",
            "[usage_governance]",
            "enabled = true",
            "auto_resume = true",
            "",
            "[usage_governance.runtime_token_rules]",
            "enabled = true",
            "",
            "[[usage_governance.runtime_token_rules.rules]]",
            "rule_id = \"test-daemon-session\"",
            "window = \"daemon_session\"",
            "metric = \"total_tokens\"",
            "threshold = 100",
            "",
            "[usage_governance.subscription_quota_rules]",
            "enabled = false",
            "",
        ]
        .join("\n"),
    )
    .unwrap();
}

fn force_execution_updater_active(
    session: &mut millrace_ai::RuntimeStartupSession,
    task_id: &str,
    run_id: &str,
) {
    let active_since = timestamp("2026-04-29T02:14:00Z");
    let active_run = ActiveRunState {
        plane: Plane::Execution,
        stage: StageName::Updater,
        node_id: "updater".to_owned(),
        stage_kind_id: "updater".to_owned(),
        run_id: run_id.to_owned(),
        request_kind: ActiveRunRequestKind::ActiveWorkItem,
        work_item_kind: Some(WorkItemKind::Task),
        work_item_id: Some(task_id.to_owned()),
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        active_since: active_since.clone(),
        running_status_marker: None,
    };
    session.snapshot.active_runs_by_plane.clear();
    session
        .snapshot
        .active_runs_by_plane
        .insert(Plane::Execution, active_run);
    session.snapshot.active_plane = Some(Plane::Execution);
    session.snapshot.active_stage = Some(StageName::Updater);
    session.snapshot.active_node_id = Some("updater".to_owned());
    session.snapshot.active_stage_kind_id = Some("updater".to_owned());
    session.snapshot.active_run_id = Some(run_id.to_owned());
    session.snapshot.active_work_item_kind = Some(WorkItemKind::Task);
    session.snapshot.active_work_item_id = Some(task_id.to_owned());
    session.snapshot.active_since = Some(active_since);
    save_snapshot(&session.paths, &session.snapshot).unwrap();
}

fn force_planning_manager_active(
    session: &mut millrace_ai::RuntimeStartupSession,
    spec_id: &str,
    run_id: &str,
) {
    let active_since = timestamp("2026-04-29T02:14:30Z");
    let active_run = ActiveRunState {
        plane: Plane::Planning,
        stage: StageName::Manager,
        node_id: "manager".to_owned(),
        stage_kind_id: "manager".to_owned(),
        run_id: run_id.to_owned(),
        request_kind: ActiveRunRequestKind::ActiveWorkItem,
        work_item_kind: Some(WorkItemKind::Spec),
        work_item_id: Some(spec_id.to_owned()),
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        active_since: active_since.clone(),
        running_status_marker: None,
    };
    session
        .snapshot
        .active_runs_by_plane
        .insert(Plane::Planning, active_run);
    session.snapshot.active_plane = Some(Plane::Planning);
    session.snapshot.active_stage = Some(StageName::Manager);
    session.snapshot.active_node_id = Some("manager".to_owned());
    session.snapshot.active_stage_kind_id = Some("manager".to_owned());
    session.snapshot.active_run_id = Some(run_id.to_owned());
    session.snapshot.active_work_item_kind = Some(WorkItemKind::Spec);
    session.snapshot.active_work_item_id = Some(spec_id.to_owned());
    session.snapshot.active_since = Some(active_since);
    save_snapshot(&session.paths, &session.snapshot).unwrap();
}

fn clear_active_projection(session: &mut millrace_ai::RuntimeStartupSession) {
    session.snapshot.active_runs_by_plane.clear();
    session.snapshot.active_plane = None;
    session.snapshot.active_stage = None;
    session.snapshot.active_node_id = None;
    session.snapshot.active_stage_kind_id = None;
    session.snapshot.active_run_id = None;
    session.snapshot.active_work_item_kind = None;
    session.snapshot.active_work_item_id = None;
    session.snapshot.active_since = None;
    save_snapshot(&session.paths, &session.snapshot).unwrap();
}

fn mailbox_envelope(
    command_id: &str,
    command: MailboxCommand,
    payload: Value,
) -> MailboxCommandEnvelope {
    let payload = match payload {
        Value::Object(map) => map,
        Value::Null => Map::new(),
        other => panic!("mailbox payload must be an object or null, got {other:?}"),
    };
    MailboxCommandEnvelope {
        schema_version: "1.0".to_owned(),
        kind: "mailbox_command".to_owned(),
        command_id: command_id.to_owned(),
        command,
        issued_at: timestamp("2026-04-29T02:12:00Z"),
        issuer: "test-operator".to_owned(),
        payload,
    }
}

fn enqueue_mailbox(
    paths: &millrace_ai::WorkspacePaths,
    command_id: &str,
    command: MailboxCommand,
    payload: Value,
) {
    write_mailbox_command(paths, &mailbox_envelope(command_id, command, payload)).unwrap();
}

fn archive_values(directory: &Path) -> Vec<Value> {
    let mut paths: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect()
}

fn archive_command_ids(directory: &Path) -> Vec<String> {
    archive_values(directory)
        .into_iter()
        .filter_map(|archive| {
            archive["envelope"]["command_id"]
                .as_str()
                .map(str::to_owned)
        })
        .collect()
}

fn mailbox_file_stems(directory: &Path) -> Vec<String> {
    let mut stems: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    stems.sort();
    stems
}

fn runtime_events(paths: &millrace_ai::WorkspacePaths) -> Vec<Value> {
    let event_log = paths.logs_dir.join("runtime_events.jsonl");
    if !event_log.exists() {
        return Vec::new();
    }
    fs::read_to_string(event_log)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn read_json_lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn monitor_event(event_type: &str, occurred_at: &str, payload: Value) -> RuntimeMonitorEvent {
    RuntimeMonitorEvent::new(
        event_type,
        timestamp(occurred_at),
        payload.as_object().unwrap().clone(),
    )
}

fn render_monitor_events(events: &[RuntimeMonitorEvent]) -> String {
    let mut monitor = BasicTerminalMonitor::new(Vec::new());
    for event in events {
        monitor.emit(event).unwrap();
    }
    String::from_utf8(monitor.into_inner()).unwrap()
}

#[derive(Clone)]
struct SharedBuffer(Rc<RefCell<Vec<u8>>>);

impl io::Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn directory_file_count(path: &std::path::Path) -> usize {
    if !path.exists() {
        return 0;
    }
    fs::read_dir(path).unwrap().count()
}

fn watcher_labels(session: &millrace_ai::RuntimeWatcherSession) -> Vec<&str> {
    session
        .targets
        .iter()
        .map(|target| target.target.as_str())
        .collect()
}

#[test]
fn basic_monitor_renders_startup_stage_router_and_status_lines() {
    let output = render_monitor_events(&[
        monitor_event(
            "runtime_started",
            "2026-04-29T02:14:03Z",
            json!({
                "mode_id": "learning_codex",
                "compiled_plan_id": "plan-123",
                "compiled_plan_currentness": "current",
                "baseline_manifest_id": "baseline-abc",
                "baseline_seed_package_version": "0.15.5",
                "loop_ids_by_plane": {
                    "execution": "execution.standard",
                    "planning": "planning.standard",
                    "learning": "learning.standard"
                },
                "concurrency_policy": {
                    "mutually_exclusive_planes": [["execution", "planning"]],
                    "may_run_concurrently": [["learning", "execution"]]
                },
                "scheduler_mode": "plane-concurrent",
                "status_markers_by_plane": {
                    "execution": "### IDLE",
                    "planning": "### IDLE",
                    "learning": "### IDLE"
                },
                "queue_depths_by_plane": {
                    "execution": 2,
                    "planning": 0,
                    "learning": 1
                }
            }),
        ),
        monitor_event(
            "stage_started",
            "2026-04-29T02:14:04Z",
            json!({
                "plane": "planning",
                "stage": "planner",
                "node_id": "planner",
                "stage_kind_id": "planner",
                "run_id": "run-b27cb14119bf410ab390a0ad124d309d",
                "work_item_kind": "spec",
                "work_item_id": "idea-corebound-north-star-spec",
                "status_marker": "### PLANNER_RUNNING"
            }),
        ),
        monitor_event(
            "router_decision",
            "2026-04-29T02:14:05Z",
            json!({
                "action": "run_stage",
                "plane": "planning",
                "run_id": "run-b27cb14119bf410ab390a0ad124d309d",
                "next_stage": "manager",
                "next_node_id": "manager",
                "next_stage_kind_id": "manager",
                "reason": "planner:PLANNER_COMPLETE"
            }),
        ),
        monitor_event(
            "status_marker_changed",
            "2026-04-29T02:14:06Z",
            json!({
                "plane": "planning",
                "run_id": "run-456",
                "previous_marker": "### MANAGER_RUNNING",
                "current_marker": "### NEEDS_EXECUTION",
                "source": "result_application"
            }),
        ),
    ]);

    assert!(
        output.contains("runtime started mode=learning_codex plan=plan-123 currentness=current")
    );
    assert!(output.contains("baseline manifest=baseline-abc seed_package=0.15.5"));
    assert!(
        output.contains("concurrency exclusive=execution+planning concurrent=learning+execution")
    );
    assert!(output.contains("snapshot status execution=IDLE planning=IDLE learning=IDLE"));
    assert!(output.contains(
        "[02:14:04] stage start planning/planner run=b27cb141 work=spec:idea-corebound-north-star-spec"
    ));
    assert!(
        output.contains("[02:14:05] route planning -> manager reason=planner:PLANNER_COMPLETE")
    );
    assert!(output.contains(
        "[02:14:06] status planning run=run-456 from=MANAGER_RUNNING to=NEEDS_EXECUTION"
    ));
}

#[test]
fn basic_monitor_renders_stage_completion_tokens_and_run_aggregate() {
    let output = render_monitor_events(&[monitor_event(
        "stage_completed",
        "2026-04-29T02:14:03Z",
        json!({
            "plane": "execution",
            "stage": "builder",
            "node_id": "builder",
            "stage_kind_id": "builder",
            "run_id": "run-123",
            "terminal_result": "BUILDER_COMPLETE",
            "summary_status_marker": "### BUILDER_COMPLETE",
            "duration_seconds": 39.2,
            "started_at": "2026-04-29T02:14:03Z",
            "completed_at": "2026-04-29T02:14:42.200Z",
            "token_usage": {
                "input_tokens": 1200,
                "cached_input_tokens": 300,
                "output_tokens": 410,
                "thinking_tokens": 900,
                "total_tokens": 2810
            }
        }),
    )]);

    assert!(output.contains(
        "stage done execution/builder run=run-123 result=BUILDER_COMPLETE dur=39.2s tokens=in=1200 cached=300 out=410 think=900 total=2810"
    ));
    assert!(output.contains(
        "run execution run=run-123 elapsed=39.2s tokens=in=1200 cached=300 out=410 think=900 total=2810"
    ));
}

#[test]
fn basic_monitor_suppresses_repeated_idle_and_resets_after_activity() {
    let output = render_monitor_events(&[
        monitor_event("runtime_tick_idle", "2026-04-29T02:14:03Z", json!({})),
        monitor_event("runtime_tick_idle", "2026-04-29T02:14:04Z", json!({})),
        monitor_event("runtime_tick_idle", "2026-04-29T02:16:02Z", json!({})),
        monitor_event("runtime_tick_idle", "2026-04-29T08:14:02Z", json!({})),
        monitor_event("runtime_tick_idle", "2026-04-29T08:14:03Z", json!({})),
        monitor_event(
            "stage_started",
            "2026-04-29T08:14:04Z",
            json!({
                "plane": "execution",
                "stage": "builder",
                "node_id": "builder",
                "stage_kind_id": "builder",
                "run_id": "run-activity",
                "work_item_kind": "task",
                "work_item_id": "task-activity",
                "status_marker": "### BUILDER_RUNNING"
            }),
        ),
        monitor_event("runtime_tick_idle", "2026-04-29T08:14:05Z", json!({})),
        monitor_event(
            "runtime_tick_idle",
            "2026-04-29T08:14:06Z",
            json!({"reason": "mailbox_empty"}),
        ),
        monitor_event("runtime_tick_idle", "2026-04-29T08:14:07Z", json!({})),
    ]);

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "[02:14:03] idle reason=no_work",
            "[08:14:03] idle reason=no_work",
            "[08:14:04] stage start execution/builder run=run-activity work=task:task-activity",
            "[08:14:05] idle reason=no_work",
            "[08:14:06] idle reason=mailbox_empty",
            "[08:14:07] idle reason=no_work",
        ]
    );
}

#[test]
fn basic_monitor_renders_reload_watcher_governance_and_fanout_lines() {
    let events = [
        monitor_event(
            "runtime_config_reload_deferred",
            "2026-04-29T02:14:03Z",
            json!({"reason": "active_planes", "active_planes": ["execution", "learning"]}),
        ),
        monitor_event(
            "runtime_config_reloaded",
            "2026-04-29T02:14:04Z",
            json!({"mode_id": "learning_codex", "compiled_plan_id": "plan-reloaded"}),
        ),
        monitor_event(
            "watcher_events_consumed",
            "2026-04-29T02:14:05Z",
            json!({"count": 3, "handled_count": 2, "failure_count": 1}),
        ),
        monitor_event(
            "usage_governance_paused",
            "2026-04-29T02:14:06Z",
            json!({
                "source": "runtime_token",
                "rule_id": "rolling-5h-default",
                "window": "rolling_5h",
                "observed": 752340,
                "threshold": 750000,
                "next_auto_resume_at": "2026-04-26T17:55:12Z"
            }),
        ),
        monitor_event(
            "usage_governance_blocked",
            "2026-04-29T02:14:07Z",
            json!({
                "source": "subscription_quota",
                "rule_id": "quota-five-hour-test",
                "window": "five_hour",
                "observed": 96,
                "threshold": 95,
                "detail": ""
            }),
        ),
        monitor_event(
            "usage_governance_degraded",
            "2026-04-29T02:14:08Z",
            json!({
                "source": "codex_chatgpt_oauth",
                "policy": "fail_open",
                "detail": "quota_telemetry_unavailable"
            }),
        ),
        monitor_event(
            "usage_governance_reconciled",
            "2026-04-29T02:14:09Z",
            json!({"repaired_count": 1, "ledger_entry_count": 2}),
        ),
        monitor_event(
            "usage_governance_resumed",
            "2026-04-29T02:14:10Z",
            json!({"cleared_rules": "rolling-5h-default"}),
        ),
    ];
    let first = Rc::new(RefCell::new(Vec::new()));
    let second = Rc::new(RefCell::new(Vec::new()));
    let mut fanout = RuntimeMonitorFanout::new(vec![
        Box::new(BasicTerminalMonitor::new(SharedBuffer(first.clone()))),
        Box::new(BasicTerminalMonitor::new(SharedBuffer(second.clone()))),
    ]);

    for event in &events {
        fanout.emit(event).unwrap();
    }

    let first_output = String::from_utf8(first.borrow().clone()).unwrap();
    let second_output = String::from_utf8(second.borrow().clone()).unwrap();
    assert_eq!(first_output, second_output);
    assert!(
        first_output
            .contains("[02:14:03] reload deferred reason=active_planes active=execution,learning")
    );
    assert!(
        first_output.contains("[02:14:04] reload applied mode=learning_codex plan=plan-reloaded")
    );
    assert!(first_output.contains("[02:14:05] watcher events count=3 handled=2 failures=1"));
    assert!(first_output.contains(
        "governance pause source=runtime_token rule=rolling-5h-default window=rolling_5h observed=752340 threshold=750000"
    ));
    assert!(first_output.contains(
        "governance blocked source=subscription_quota rule=quota-five-hour-test window=five_hour observed=96 threshold=95"
    ));
    assert!(first_output.contains(
        "governance degraded source=codex_chatgpt_oauth policy=fail_open detail=quota_telemetry_unavailable"
    ));
    assert!(first_output.contains("governance reconciled repaired=1 ledger_entries=2"));
    assert!(first_output.contains("governance resume cleared_rules=rolling-5h-default"));
}

#[test]
fn basic_monitor_renders_operator_intervention_events() {
    let output = render_monitor_events(&[
        monitor_event(
            "work_item_cancelled",
            "2026-05-12T12:00:00Z",
            json!({
                "work_item_kind": "task",
                "work_item_id": "task-cancel-monitor",
                "destination_path": "millrace-agents/tasks/queue/cancelled/task-cancel-monitor.20260512T120000Z.queue.md"
            }),
        ),
        monitor_event(
            "task_superseded",
            "2026-05-12T12:00:01Z",
            json!({
                "work_item_kind": "task",
                "work_item_id": "task-old-monitor",
                "destination_path": "millrace-agents/tasks/queue/superseded/task-old-monitor.20260512T120001Z.queue.md",
                "replacement_work_item_id": "task-new-monitor",
                "affected_dependents": ["task-dependent-monitor"]
            }),
        ),
        monitor_event(
            "mailbox_operator_intervention_applied",
            "2026-05-12T12:00:02Z",
            json!({
                "command": "cancel_work_item",
                "command_id": "cmd-cancel-monitor",
                "event_type": "work_item_cancelled",
                "work_item_kind": "probe",
                "work_item_id": "probe-cancel-monitor",
                "destination_path": "millrace-agents/probes/queue/cancelled/probe-cancel-monitor.20260512T120002Z.queue.md"
            }),
        ),
        monitor_event(
            "operator_intervention_deferred",
            "2026-05-12T12:00:03Z",
            json!({
                "command": "supersede_task",
                "work_item_kind": "task",
                "work_item_id": "task-deferred-monitor",
                "reason": "active_runtime_stage",
                "active_planes": ["execution"],
                "deferred_command_id": "cmd-supersede-monitor.deferred"
            }),
        ),
    ]);

    assert!(output.contains(
        "[12:00:00] operator intervention event=work_item_cancelled work=task:task-cancel-monitor destination=millrace-agents/tasks/queue/cancelled/task-cancel-monitor.20260512T120000Z.queue.md"
    ));
    assert!(output.contains(
        "[12:00:01] operator intervention event=task_superseded work=task:task-old-monitor destination=millrace-agents/tasks/queue/superseded/task-old-monitor.20260512T120001Z.queue.md replacement=task-new-monitor affected=task-dependent-monitor"
    ));
    assert!(output.contains(
        "[12:00:02] mailbox operator intervention applied command=cancel_work_item event=work_item_cancelled work=probe:probe-cancel-monitor destination=millrace-agents/probes/queue/cancelled/probe-cancel-monitor.20260512T120002Z.queue.md command_id=cmd-cancel-monitor"
    ));
    assert!(output.contains(
        "[12:00:03] operator intervention deferred command=supersede_task reason=active_runtime_stage active=execution work=task:task-deferred-monitor deferred_command_id=cmd-supersede-monitor.deferred"
    ));
}

#[test]
fn monitor_events_parse_persisted_runtime_event_jsonl_data_payloads() {
    let events = runtime_monitor_events_from_jsonl(
        r#"{"schema_version":"1.0","kind":"runtime_event","event_type":"runtime_tick_idle","occurred_at":"2026-04-29T02:14:03Z","data":{"reason":"no_work"}}"#,
    )
    .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "runtime_tick_idle");
    assert_eq!(events[0].payload["reason"], "no_work");
    assert_eq!(
        render_monitor_events(&events).trim(),
        "[02:14:03] idle reason=no_work"
    );
}

#[test]
fn daemon_startup_requires_initialized_workspace_without_creating_it() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("workspace");

    let error = startup_runtime_daemon(&root, daemon_options("uninitialized")).unwrap_err();

    assert!(error.to_string().contains("workspace is not initialized"));
    assert!(!root.join("millrace-agents").exists());
}

#[test]
fn daemon_startup_projects_mode_config_and_deterministic_watcher_state() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let config_path = temp.path().join("daemon-config.toml");
    fs::write(
        &config_path,
        [
            "[runtime]",
            "default_mode = \"learning_codex\"",
            "run_style = \"daemon\"",
            "idle_sleep_seconds = 2.5",
            "",
            "[watchers]",
            "enabled = true",
            "debounce_ms = 750",
            "watch_ideas_inbox = true",
            "watch_specs_queue = false",
            "",
            "[auto_recovery]",
            "enabled = true",
            "blocked_dependency_retry_enabled = true",
            "max_auto_requeues_per_work_item = 2",
            "cooldown_seconds = [300, 900, 3600]",
            "",
        ]
        .join("\n"),
    )
    .unwrap();

    let mut options = daemon_options("daemon-config-default");
    options.config_path = Some(config_path.clone());
    let mut session = startup_runtime_daemon_for_paths(&paths, options).unwrap();

    assert_eq!(session.snapshot.runtime_mode, RuntimeMode::Daemon);
    assert_eq!(session.snapshot.active_mode_id, "learning_codex");
    assert_eq!(session.snapshot.watcher_mode, WatcherMode::Poll);
    assert_eq!(session.config.run_style, RuntimeMode::Daemon);
    assert_eq!(session.config.idle_sleep_seconds, 2.5);
    assert_eq!(session.config.watchers_debounce_ms, 750);
    assert!(session.config.watchers_watch_ideas_inbox);
    assert!(!session.config.watchers_watch_specs_queue);
    assert!(session.config.auto_recovery.enabled);
    assert!(
        session
            .config
            .auto_recovery
            .blocked_dependency_retry_enabled
    );
    assert_eq!(
        session.config.auto_recovery.max_auto_requeues_per_work_item,
        2
    );
    assert_eq!(
        session.config.auto_recovery.cooldown_seconds,
        vec![300, 900, 3600]
    );
    assert_eq!(
        watcher_labels(&session.watcher_session),
        vec!["config", "tasks_queue", "ideas_inbox"]
    );
    assert!(session.watcher_session.poll_fallback_ready);
    assert_eq!(session.watcher_session.debounce_ms, 750);
    assert_eq!(
        session.watcher_session.targets[0].root,
        config_path.parent().unwrap()
    );
    assert!(
        session
            .watcher_session
            .targets
            .iter()
            .find(|target| target.target == "ideas_inbox")
            .unwrap()
            .emit_existing_on_startup
    );
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Active
    );

    assert!(session.close().unwrap());
    assert_eq!(session.watcher_session.mode, WatcherMode::Off);
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Absent
    );

    let mut override_options = daemon_options("daemon-mode-override");
    override_options.config_path = Some(config_path);
    override_options.requested_mode_id = Some("default_pi".to_owned());
    let session = startup_runtime_daemon_for_paths(&paths, override_options).unwrap();
    assert_eq!(session.snapshot.active_mode_id, "default_pi");
    session.finish().unwrap();
}

#[test]
fn daemon_config_loading_uses_python_compatible_defaults_and_validation() {
    let temp = TempDir::new().unwrap();
    let missing_config = temp.path().join("missing.toml");
    let defaults = load_runtime_startup_config(&missing_config).unwrap();

    assert_eq!(defaults.default_mode, "default_codex");
    assert_eq!(defaults.run_style, RuntimeMode::Daemon);
    assert_eq!(defaults.idle_sleep_seconds, 1.0);
    assert!(defaults.watchers_enabled);
    assert_eq!(defaults.watchers_debounce_ms, 250);
    assert!(defaults.watchers_watch_ideas_inbox);
    assert!(defaults.watchers_watch_specs_queue);
    assert!(defaults.auto_recovery.enabled);
    assert!(defaults.auto_recovery.blocked_dependency_retry_enabled);
    assert_eq!(defaults.auto_recovery.max_auto_requeues_per_work_item, 3);
    assert_eq!(
        defaults.auto_recovery.cooldown_seconds,
        vec![300, 900, 3600]
    );

    let explicit_auto_recovery = temp.path().join("auto-recovery.toml");
    fs::write(
        &explicit_auto_recovery,
        [
            "[auto_recovery]",
            "enabled = false",
            "blocked_dependency_retry_enabled = false",
            "max_auto_requeues_per_work_item = 0",
            "cooldown_seconds = [0, 300, 900]",
            "",
        ]
        .join("\n"),
    )
    .unwrap();
    let config = load_runtime_startup_config(&explicit_auto_recovery).unwrap();
    assert!(!config.auto_recovery.enabled);
    assert!(!config.auto_recovery.blocked_dependency_retry_enabled);
    assert_eq!(config.auto_recovery.max_auto_requeues_per_work_item, 0);
    assert_eq!(config.auto_recovery.cooldown_seconds, vec![0, 300, 900]);

    let invalid_idle = temp.path().join("invalid-idle.toml");
    fs::write(&invalid_idle, "[runtime]\nidle_sleep_seconds = 0\n").unwrap();
    let error = load_runtime_startup_config(&invalid_idle).unwrap_err();
    assert!(error.to_string().contains("runtime.idle_sleep_seconds"));

    let invalid_debounce = temp.path().join("invalid-debounce.toml");
    fs::write(&invalid_debounce, "[watchers]\ndebounce_ms = 0\n").unwrap();
    let error = load_runtime_startup_config(&invalid_debounce).unwrap_err();
    assert!(error.to_string().contains("watchers.debounce_ms"));

    for (name, raw, expected) in [
        (
            "invalid-auto-recovery-max.toml",
            "[auto_recovery]\nmax_auto_requeues_per_work_item = -1\n",
            "auto_recovery.max_auto_requeues_per_work_item",
        ),
        (
            "invalid-auto-recovery-cooldown-empty.toml",
            "[auto_recovery]\ncooldown_seconds = []\n",
            "auto_recovery.cooldown_seconds",
        ),
        (
            "invalid-auto-recovery-cooldown-negative.toml",
            "[auto_recovery]\ncooldown_seconds = [300, -1]\n",
            "auto_recovery.cooldown_seconds[1]",
        ),
        (
            "invalid-auto-recovery-key.toml",
            "[auto_recovery]\nblocked_dependency_retries = true\n",
            "auto_recovery.blocked_dependency_retries",
        ),
    ] {
        let path = temp.path().join(name);
        fs::write(&path, raw).unwrap();
        let error = load_runtime_startup_config(&path).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "missing `{expected}` in error: {error}"
        );
    }
}

#[test]
fn daemon_config_boundaries_classify_auto_recovery_as_next_tick() {
    for field in [
        "auto_recovery.enabled",
        "auto_recovery.blocked_dependency_retry_enabled",
        "auto_recovery.max_auto_requeues_per_work_item",
        "auto_recovery.cooldown_seconds",
    ] {
        assert_eq!(
            runtime_config_apply_boundary_for_field(field).unwrap(),
            RuntimeConfigApplyBoundary::NextTick,
            "auto-recovery config field {field} must apply on the next tick"
        );
    }
}

#[test]
fn runtime_config_loading_exposes_real_runner_adapter_settings() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("runner-config.toml");
    fs::write(
        &config_path,
        [
            "[runtime]",
            "default_mode = \"learning_codex\"",
            "run_style = \"daemon\"",
            "",
            "[runners]",
            "default_runner = \"pi_rpc\"",
            "",
            "[runners.codex]",
            "command = \"codex-dev\"",
            "args = [\"exec\", \"--trace\"]",
            "profile = \"ops\"",
            "permission_default = \"elevated\"",
            "permission_by_stage = { builder = \"basic\", professor = \"elevated\" }",
            "permission_by_model = { \"gpt-5\" = \"maximum\" }",
            "model_reasoning_effort = \"high\"",
            "skip_git_repo_check = false",
            "extra_config = [\"sandbox_workspace_write.network_access=true\"]",
            "",
            "[runners.codex.env]",
            "CODEX_HOME = \"/tmp/codex\"",
            "",
            "[runners.pi]",
            "command = \"pi-dev\"",
            "args = [\"--debug\"]",
            "provider = \"openai\"",
            "thinking = \"medium\"",
            "disable_context_files = false",
            "disable_skills = false",
            "event_log_policy = \"full\"",
            "",
            "[runners.pi.env]",
            "PI_HOME = \"/tmp/pi\"",
            "",
            "[usage_governance]",
            "enabled = true",
            "auto_resume = false",
            "calendar_timezone = \"UTC\"",
            "",
            "[usage_governance.runtime_token_rules]",
            "enabled = true",
            "",
            "[[usage_governance.runtime_token_rules.rules]]",
            "rule_id = \"test-rolling\"",
            "window = \"rolling_5h\"",
            "metric = \"total_tokens\"",
            "threshold = 1000",
            "",
            "[usage_governance.subscription_quota_rules]",
            "enabled = true",
            "degraded_policy = \"fail_closed\"",
            "refresh_interval_seconds = 5",
            "",
            "[[usage_governance.subscription_quota_rules.rules]]",
            "rule_id = \"quota-five-hour-test\"",
            "window = \"five_hour\"",
            "pause_at_percent_used = 90",
            "",
            "[stages.builder]",
            "runner = \"pi_rpc\"",
            "model = \"openai/gpt-5.4\"",
            "model_reasoning_effort = \"xhigh\"",
            "timeout_seconds = 45",
            "",
        ]
        .join("\n"),
    )
    .unwrap();

    let config = load_runtime_startup_config(&config_path).unwrap();

    assert_eq!(config.default_mode, "learning_codex");
    assert_eq!(config.runners.default_runner, "pi_rpc");
    assert_eq!(config.runners.codex.command, "codex-dev");
    assert_eq!(config.runners.codex.args, vec!["exec", "--trace"]);
    assert_eq!(config.runners.codex.profile.as_deref(), Some("ops"));
    assert_eq!(config.runners.codex.permission_default.as_str(), "elevated");
    assert_eq!(
        config.runners.codex.permission_by_stage["builder"].as_str(),
        "basic"
    );
    assert_eq!(
        config.runners.codex.permission_by_stage["professor"].as_str(),
        "elevated"
    );
    assert_eq!(
        config.runners.codex.permission_by_model["gpt-5"].as_str(),
        "maximum"
    );
    assert_eq!(
        config.runners.codex.model_reasoning_effort.as_deref(),
        Some("high")
    );
    assert!(!config.runners.codex.skip_git_repo_check);
    assert_eq!(
        config.runners.codex.extra_config,
        vec!["sandbox_workspace_write.network_access=true"]
    );
    assert_eq!(config.runners.codex.env["CODEX_HOME"], "/tmp/codex");
    assert_eq!(config.runners.pi.command, "pi-dev");
    assert_eq!(config.runners.pi.args, vec!["--debug"]);
    assert_eq!(config.runners.pi.provider.as_deref(), Some("openai"));
    assert_eq!(config.runners.pi.thinking.as_deref(), Some("medium"));
    assert!(!config.runners.pi.disable_context_files);
    assert!(!config.runners.pi.disable_skills);
    assert_eq!(config.runners.pi.event_log_policy.as_str(), "full");
    assert_eq!(config.runners.pi.env["PI_HOME"], "/tmp/pi");
    assert!(config.usage_governance_enabled);
    assert!(config.usage_governance.enabled);
    assert!(!config.usage_governance.auto_resume);
    assert_eq!(config.usage_governance.calendar_timezone, "UTC");
    assert_eq!(
        config.usage_governance.runtime_token_rules.rules[0].rule_id,
        "test-rolling"
    );
    assert_eq!(
        config.usage_governance.runtime_token_rules.rules[0].threshold,
        1000
    );
    assert!(config.usage_governance.subscription_quota_rules.enabled);
    assert_eq!(
        config
            .usage_governance
            .subscription_quota_rules
            .degraded_policy
            .as_str(),
        "fail_closed"
    );
    assert_eq!(
        config.usage_governance.subscription_quota_rules.rules[0].pause_at_percent_used,
        90.0
    );

    let builder = &config.stages["builder"];
    assert_eq!(builder.runner.as_deref(), Some("pi_rpc"));
    assert_eq!(builder.model.as_deref(), Some("openai/gpt-5.4"));
    assert_eq!(builder.thinking_level.as_deref(), Some("xhigh"));
    assert_eq!(builder.model_reasoning_effort.as_deref(), Some("xhigh"));
    assert_eq!(builder.timeout_seconds, 45);

    let codex_adapter = CodexCliRunnerAdapter::new(config.runners.codex.clone(), temp.path());
    let pi_adapter = PiRpcRunnerAdapter::new(config.runners.pi.clone(), temp.path());
    assert_eq!(codex_adapter.name(), "codex_cli");
    assert_eq!(pi_adapter.name(), "pi_rpc");
}

#[test]
fn runtime_config_loading_rejects_real_runner_config_failures_with_paths() {
    let temp = TempDir::new().unwrap();
    for (name, raw, expected) in [
        (
            "bad-runner-name.toml",
            "[runners]\ndefault_runner = \"bad runner\"\n",
            "runners.default_runner",
        ),
        (
            "bad-permission.toml",
            "[runners.codex]\npermission_default = \"root\"\n",
            "runners.codex.permission_default",
        ),
        (
            "bad-env.toml",
            "[runners.codex]\nenv = [\"CODEX_HOME=/tmp\"]\n",
            "runners.codex.env",
        ),
        (
            "bad-pi-policy.toml",
            "[runners.pi]\nevent_log_policy = \"never\"\n",
            "runners.pi.event_log_policy",
        ),
        (
            "reserved-pi-flag.toml",
            "[runners.pi]\nargs = [\"--mode=rpc\"]\n",
            "runners.pi.args",
        ),
        (
            "bad-timeout.toml",
            "[stages.builder]\ntimeout_seconds = 0\n",
            "stages.builder.timeout_seconds",
        ),
        (
            "bad-empty-thinking.toml",
            "[stages.builder]\nthinking_level = \" \"\n",
            "stages.builder.thinking_level",
        ),
        (
            "bad-thinking-alias-conflict.toml",
            "[stages.builder]\nthinking_level = \"medium\"\nmodel_reasoning_effort = \"high\"\n",
            "stages.builder.thinking_level",
        ),
        (
            "bad-stage-key.toml",
            "[stages.builder]\npermission_default = \"basic\"\n",
            "stages.builder.permission_default",
        ),
        (
            "bad-token-window.toml",
            "[[usage_governance.runtime_token_rules.rules]]\nrule_id = \"bad-window\"\nwindow = \"hourly\"\nthreshold = 1\n",
            "usage_governance.runtime_token_rules.rules[0].window",
        ),
        (
            "bad-quota-percent.toml",
            "[[usage_governance.subscription_quota_rules.rules]]\nrule_id = \"bad-percent\"\nwindow = \"five_hour\"\npause_at_percent_used = 101\n",
            "usage_governance.subscription_quota_rules.rules[0].pause_at_percent_used",
        ),
    ] {
        let path = temp.path().join(name);
        fs::write(&path, raw).unwrap();
        let error = load_runtime_startup_config(&path).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "missing `{expected}` in error: {error}"
        );
    }
}

#[test]
fn daemon_startup_lock_contention_preserves_compiler_and_state_artifacts() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();

    let session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("initial-owner")).unwrap();
    session.finish().unwrap();

    let compiled_before = fs::read(&paths.compiled_plan_file).unwrap();
    let diagnostics_before = fs::read(&paths.compile_diagnostics_file).unwrap();
    let snapshot_before = fs::read(&paths.runtime_snapshot_file).unwrap();
    let counters_before = fs::read(&paths.recovery_counters_file).unwrap();

    acquire_runtime_ownership_lock_with_options(&paths, lock_options("external-owner")).unwrap();

    let error = startup_runtime_daemon_for_paths(&paths, daemon_options("contender")).unwrap_err();
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
fn daemon_startup_failure_releases_matching_lock_and_preserves_state() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();

    let session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("initial-compile")).unwrap();
    session.finish().unwrap();

    let compiled_before = fs::read(&paths.compiled_plan_file).unwrap();
    let diagnostics_before = fs::read(&paths.compile_diagnostics_file).unwrap();
    let snapshot_before = fs::read(&paths.runtime_snapshot_file).unwrap();
    let counters_before = fs::read(&paths.recovery_counters_file).unwrap();
    fs::write(
        &paths.runtime_config_file,
        "[runtime]\ndefault_mode = \"missing_mode\"\n",
    )
    .unwrap();

    let error =
        startup_runtime_daemon_for_paths(&paths, daemon_options("compile-fails")).unwrap_err();

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
fn daemon_watcher_rebuild_hook_updates_snapshot_without_claiming_work() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-watch")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        [
            "[runtime]",
            "default_mode = \"default_codex\"",
            "run_style = \"daemon\"",
            "",
            "[watchers]",
            "enabled = false",
            "",
        ]
        .join("\n"),
    )
    .unwrap();

    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("watcher-disabled")).unwrap();

    assert_eq!(session.watcher_session.mode, WatcherMode::Off);
    assert_eq!(session.snapshot.watcher_mode, WatcherMode::Off);
    assert!(paths.tasks_queue_dir.join("task-watch.md").is_file());
    assert!(!paths.tasks_active_dir.join("task-watch.md").exists());

    fs::write(
        &paths.runtime_config_file,
        [
            "[runtime]",
            "default_mode = \"default_codex\"",
            "run_style = \"daemon\"",
            "",
            "[watchers]",
            "enabled = true",
            "debounce_ms = 500",
            "watch_ideas_inbox = false",
            "watch_specs_queue = true",
            "",
        ]
        .join("\n"),
    )
    .unwrap();
    session.config = load_runtime_startup_config(&paths.runtime_config_file).unwrap();
    session.rebuild_watcher_session().unwrap();

    assert_eq!(session.watcher_session.mode, WatcherMode::Poll);
    assert_eq!(session.watcher_session.debounce_ms, 500);
    assert_eq!(
        watcher_labels(&session.watcher_session),
        vec!["config", "tasks_queue", "specs_queue"]
    );
    assert_eq!(
        load_snapshot(&paths).unwrap().watcher_mode,
        WatcherMode::Poll
    );
    assert!(paths.tasks_queue_dir.join("task-watch.md").is_file());
    assert!(!paths.tasks_active_dir.join("task-watch.md").exists());

    session.finish().unwrap();
}

#[test]
fn daemon_mailbox_drains_control_and_intake_commands_into_processed_archives() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("mailbox-control-intake")).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());

    enqueue_mailbox(&paths, "01-pause", MailboxCommand::Pause, Value::Null);
    enqueue_mailbox(&paths, "02-resume", MailboxCommand::Resume, Value::Null);
    enqueue_mailbox(
        &paths,
        "03-add-task",
        MailboxCommand::AddTask,
        json!({"document": task_document("task-mailbox-add")}),
    );
    enqueue_mailbox(
        &paths,
        "04-add-spec",
        MailboxCommand::AddSpec,
        json!({"document": spec_document("spec-mailbox-add")}),
    );
    enqueue_mailbox(
        &paths,
        "05-add-probe",
        MailboxCommand::AddProbe,
        json!({"document": probe_document("probe-mailbox-add")}),
    );
    enqueue_mailbox(
        &paths,
        "06-add-idea",
        MailboxCommand::AddIdea,
        json!({"source_name": "idea-mailbox-add.md", "markdown": "# Idea from mailbox\n"}),
    );
    enqueue_mailbox(&paths, "07-stop", MailboxCommand::Stop, Value::Null);

    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(outcome.kind, millrace_ai::RuntimeTickOutcomeKind::Stopped);
    assert!(session.snapshot.stop_requested);
    assert!(!session.snapshot.paused);
    assert_eq!(session.snapshot.queue_depth_execution, 1);
    assert_eq!(session.snapshot.queue_depth_planning, 3);
    assert!(paths.tasks_queue_dir.join("task-mailbox-add.md").is_file());
    assert!(
        paths
            .probes_queue_dir
            .join("probe-mailbox-add.md")
            .is_file()
    );
    assert!(paths.specs_queue_dir.join("spec-mailbox-add.md").is_file());
    assert!(paths.specs_queue_dir.join("idea-mailbox-add.md").is_file());
    assert!(paths.root.join("ideas/inbox/idea-mailbox-add.md").is_file());
    assert_eq!(
        archive_command_ids(&paths.mailbox_processed_dir),
        vec![
            "01-pause",
            "02-resume",
            "03-add-task",
            "04-add-spec",
            "05-add-probe",
            "06-add-idea",
            "07-stop"
        ]
    );
    assert!(archive_values(&paths.mailbox_failed_dir).is_empty());
    assert_eq!(directory_file_count(&paths.mailbox_incoming_dir), 0);

    let events = runtime_events(&paths);
    for event_type in [
        "mailbox_pause_applied",
        "mailbox_resume_applied",
        "mailbox_add_task_applied",
        "mailbox_add_probe_applied",
        "mailbox_add_spec_applied",
        "mailbox_add_idea_applied",
        "mailbox_stop_applied",
    ] {
        assert!(
            events.iter().any(|event| event["event_type"] == event_type),
            "missing event {event_type}"
        );
    }
}

#[test]
fn daemon_mailbox_applies_operator_intervention_when_idle_and_records_audit() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-old")).unwrap();
    queue.enqueue_task(&task_document("task-new")).unwrap();
    let mut dependent = task_document("task-dependent");
    dependent.depends_on = vec!["task-old".to_owned()];
    queue.enqueue_task(&dependent).unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("mailbox-intervention")).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());

    enqueue_mailbox(
        &paths,
        "01-supersede",
        MailboxCommand::SupersedeTask,
        json!({
            "old_task_id": "task-old",
            "replacement_task_id": "task-new",
            "reason": "replacement task has corrected scope",
            "cascade": "retarget"
        }),
    );
    enqueue_mailbox(&paths, "02-stop", MailboxCommand::Stop, Value::Null);

    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(outcome.kind, millrace_ai::RuntimeTickOutcomeKind::Stopped);
    assert!(paths.tasks_queue_dir.join("task-new.md").is_file());
    assert!(!paths.tasks_queue_dir.join("task-old.md").exists());
    let dependent = parse_task_document(
        &fs::read_to_string(paths.tasks_queue_dir.join("task-dependent.md")).unwrap(),
    )
    .unwrap();
    assert_eq!(dependent.depends_on, vec!["task-new".to_owned()]);
    let superseded_dir = paths.tasks_queue_dir.join("superseded");
    assert_eq!(
        read_json_lines(&superseded_dir.join("interventions.jsonl"))[0]["action"],
        "supersede"
    );
    assert_eq!(
        read_json_lines(&paths.tasks_queue_dir.join("interventions.jsonl"))[0]["action"],
        "retarget_dependency"
    );
    assert_eq!(session.snapshot.queue_depth_execution, 2);
    assert_eq!(
        archive_command_ids(&paths.mailbox_processed_dir),
        vec!["01-supersede", "02-stop"]
    );
    let processed = archive_values(&paths.mailbox_processed_dir);
    assert_eq!(processed[0]["result"]["applied"], true);
    assert!(
        processed[0]["result"]["detail"]
            .as_str()
            .unwrap()
            .contains("task_superseded: task task-old")
    );
    let events = runtime_events(&paths);
    for event_type in [
        "task_superseded",
        "task_dependency_retargeted",
        "mailbox_operator_intervention_applied",
    ] {
        assert!(
            events.iter().any(|event| event["event_type"] == event_type),
            "missing event {event_type}"
        );
    }
}

#[test]
fn daemon_mailbox_preserves_invalid_and_failed_payloads_and_continues() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("mailbox-failed-payloads"))
            .unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    let ideas_dir = paths.root.join("ideas/inbox");
    fs::create_dir_all(&ideas_dir).unwrap();
    fs::write(ideas_dir.join("duplicate.md"), "# Existing\n").unwrap();
    fs::write(
        paths.mailbox_incoming_dir.join("00-invalid.json"),
        "{not-json",
    )
    .unwrap();
    enqueue_mailbox(
        &paths,
        "01-duplicate-idea",
        MailboxCommand::AddIdea,
        json!({"source_name": "duplicate.md", "markdown": "# Replacement\n"}),
    );
    enqueue_mailbox(
        &paths,
        "02-good-task",
        MailboxCommand::AddTask,
        json!({"document": task_document("task-after-failure")}),
    );
    enqueue_mailbox(&paths, "03-pause", MailboxCommand::Pause, Value::Null);

    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(outcome.kind, millrace_ai::RuntimeTickOutcomeKind::Paused);
    assert!(
        paths
            .tasks_queue_dir
            .join("task-after-failure.md")
            .is_file()
    );
    assert_eq!(
        fs::read_to_string(ideas_dir.join("duplicate.md")).unwrap(),
        "# Existing\n"
    );

    let failed = archive_values(&paths.mailbox_failed_dir);
    assert_eq!(failed.len(), 2);
    assert_eq!(failed[0]["kind"], "mailbox_archive");
    assert_eq!(failed[0]["disposition"], "failed");
    assert!(
        failed[0]["raw_payload"]
            .as_str()
            .unwrap()
            .contains("{not-json")
    );
    assert_eq!(failed[1]["envelope"]["command_id"], "01-duplicate-idea");
    assert!(
        failed[1]["error"]
            .as_str()
            .unwrap()
            .contains("idea document already exists")
    );
    assert_eq!(
        archive_command_ids(&paths.mailbox_processed_dir),
        vec!["02-good-task", "03-pause"]
    );
    assert_eq!(directory_file_count(&paths.mailbox_incoming_dir), 0);
}

#[test]
fn daemon_mailbox_retry_active_supports_unscoped_and_planning_scoped_retries() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-retry-mailbox"))
        .unwrap();
    queue
        .enqueue_spec(&spec_document("spec-retry-mailbox"))
        .unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("mailbox-retry-active")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    queue.claim_next_planning_item(None).unwrap().unwrap();
    force_execution_updater_active(&mut session, "task-retry-mailbox", "run-retry-exec");
    force_planning_manager_active(&mut session, "spec-retry-mailbox", "run-retry-plan");
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());

    enqueue_mailbox(
        &paths,
        "01-unscoped-skipped",
        MailboxCommand::RetryActive,
        json!({"reason": "first retry"}),
    );
    enqueue_mailbox(
        &paths,
        "02-planning-retry",
        MailboxCommand::RetryActive,
        json!({"reason": "planning retry", "scope": "planning"}),
    );
    enqueue_mailbox(
        &paths,
        "03-execution-retry",
        MailboxCommand::RetryActive,
        json!({"reason": "execution retry"}),
    );
    enqueue_mailbox(&paths, "04-pause", MailboxCommand::Pause, Value::Null);

    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(outcome.kind, millrace_ai::RuntimeTickOutcomeKind::Paused);
    assert!(
        paths
            .specs_queue_dir
            .join("spec-retry-mailbox.md")
            .is_file()
    );
    assert!(
        paths
            .tasks_queue_dir
            .join("task-retry-mailbox.md")
            .is_file()
    );
    assert!(session.snapshot.active_runs_by_plane.is_empty());
    assert_eq!(load_execution_status(&paths).unwrap(), "### IDLE");
    assert_eq!(load_planning_status(&paths).unwrap(), "### IDLE");
    assert_eq!(
        archive_command_ids(&paths.mailbox_processed_dir),
        vec![
            "01-unscoped-skipped",
            "02-planning-retry",
            "03-execution-retry",
            "04-pause"
        ]
    );
    assert!(archive_values(&paths.mailbox_failed_dir).is_empty());
    assert!(runtime_events(&paths).iter().any(|event| {
        event["event_type"] == "retry_active_skipped"
            && event["data"]["reason"] == "multiple_active_planes"
    }));
}

#[test]
fn daemon_mailbox_clear_stale_requeues_active_items_and_resets_daemon_state() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-clear-stale"))
        .unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("mailbox-clear-stale")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    force_execution_updater_active(&mut session, "task-clear-stale", "run-clear-stale");
    session.snapshot.paused = true;
    session.snapshot.stop_requested = true;
    save_snapshot(&paths, &session.snapshot).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());

    enqueue_mailbox(
        &paths,
        "01-clear-stale",
        MailboxCommand::ClearStaleState,
        json!({"reason": "operator cleanup"}),
    );

    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(
        outcome.kind,
        millrace_ai::RuntimeTickOutcomeKind::StageRequestReady
    );
    assert!(paths.tasks_active_dir.join("task-clear-stale.md").is_file());
    assert!(
        session
            .snapshot
            .active_runs_by_plane
            .contains_key(&Plane::Execution)
    );
    let processed = archive_values(&paths.mailbox_processed_dir);
    assert_eq!(processed.len(), 1);
    assert_eq!(processed[0]["result"]["applied"], true);
    assert!(runtime_events(&paths).iter().any(|event| {
        event["event_type"] == "clear_stale_state_applied" && event["data"]["requeued_count"] == 1
    }));
}

#[test]
fn daemon_mailbox_reload_defers_while_active_and_applies_after_planes_drain() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-reload-active"))
        .unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("mailbox-reload-defer")).unwrap();
    let compiled_before = session.snapshot.compiled_plan_id.clone();
    let config_before = session.snapshot.config_version.clone();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    force_execution_updater_active(&mut session, "task-reload-active", "run-reload-active");
    fs::write(
        &paths.runtime_config_file,
        [
            "[runtime]",
            "default_mode = \"default_codex\"",
            "run_style = \"daemon\"",
            "",
            "[watchers]",
            "enabled = false",
            "",
            "[runners.codex]",
            "model_reasoning_effort = \"high\"",
            "",
            "[stages.builder]",
            "timeout_seconds = 45",
            "",
        ]
        .join("\n"),
    )
    .unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    enqueue_mailbox(
        &paths,
        "01-reload",
        MailboxCommand::ReloadConfig,
        Value::Null,
    );

    let first = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(first.dispatched_count, 1);
    assert_eq!(
        load_snapshot(&paths).unwrap().last_reload_error.as_deref(),
        Some("deferred until active planes drain")
    );
    let incoming_after_defer = mailbox_file_stems(&paths.mailbox_incoming_dir);
    assert_eq!(incoming_after_defer.len(), 1);
    assert!(incoming_after_defer[0].starts_with("reload_config-deferred-"));
    assert_eq!(
        archive_command_ids(&paths.mailbox_processed_dir),
        vec!["01-reload"]
    );
    assert!(runtime_events(&paths).iter().any(|event| {
        event["event_type"] == "runtime_config_reload_deferred"
            && event["data"]["active_planes"].as_array().unwrap()
                == &vec![Value::String("execution".to_owned())]
    }));

    supervisor.drain_completed(&mut session).unwrap();
    assert!(paths.tasks_done_dir.join("task-reload-active.md").is_file());
    clear_active_projection(&mut session);

    let second = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(second.kind, millrace_ai::RuntimeTickOutcomeKind::NoWork);
    assert_eq!(
        session.snapshot.last_reload_outcome,
        Some(ReloadOutcome::Applied)
    );
    assert_eq!(session.snapshot.last_reload_error, None);
    assert_eq!(session.snapshot.watcher_mode, WatcherMode::Off);
    assert_ne!(session.snapshot.config_version, config_before);
    assert_ne!(session.snapshot.compiled_plan_id, compiled_before);
    assert_eq!(directory_file_count(&paths.mailbox_incoming_dir), 0);
    assert_eq!(archive_values(&paths.mailbox_processed_dir).len(), 2);
    assert!(runtime_events(&paths).iter().any(|event| {
        event["event_type"] == "runtime_config_reloaded"
            && event["data"]["compiled_plan_id"]
                == Value::String(session.snapshot.compiled_plan_id.clone())
    }));
}

#[test]
fn daemon_mailbox_reload_failure_preserves_previous_plan_and_failed_archive() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("mailbox-reload-fails")).unwrap();
    let compiled_before = fs::read(&paths.compiled_plan_file).unwrap();
    fs::write(
        &paths.runtime_config_file,
        [
            "[runtime]",
            "default_mode = \"default_codex\"",
            "run_style = \"daemon\"",
            "",
            "[runners]",
            "default_runner = 1",
            "",
        ]
        .join("\n"),
    )
    .unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    enqueue_mailbox(
        &paths,
        "01-reload-bad",
        MailboxCommand::ReloadConfig,
        Value::Null,
    );

    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(outcome.kind, millrace_ai::RuntimeTickOutcomeKind::Stopped);
    assert_eq!(
        fs::read(&paths.compiled_plan_file).unwrap(),
        compiled_before
    );
    assert!(!session.snapshot.process_running);
    assert!(session.snapshot.stop_requested);
    assert_eq!(
        session.snapshot.last_reload_outcome,
        Some(ReloadOutcome::FailedRetainedPreviousPlan)
    );
    let failed = archive_values(&paths.mailbox_failed_dir);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["envelope"]["command_id"], "01-reload-bad");
    assert!(
        failed[0]["error"]
            .as_str()
            .unwrap()
            .contains("default_runner")
    );
    assert!(archive_values(&paths.mailbox_processed_dir).is_empty());
    assert!(runtime_events(&paths).iter().any(|event| {
        event["event_type"] == "runtime_config_reload_failed"
            && event["data"]["retained_previous_plan"] == false
    }));
}

#[test]
fn daemon_watcher_startup_scan_normalizes_idea_before_claiming_work() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let ideas_dir = paths.root.join("ideas/inbox");
    fs::create_dir_all(&ideas_dir).unwrap();
    fs::write(
        ideas_dir.join("Root Idea.md"),
        "# Root Idea Title\n\nBuild watcher intake.\n",
    )
    .unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("watcher-startup-scan")).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());

    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(
        outcome.kind,
        millrace_ai::RuntimeTickOutcomeKind::StageRequestReady
    );
    assert!(paths.specs_active_dir.join("idea-Root-Idea.md").is_file());
    let generated = parse_spec_document(
        &fs::read_to_string(paths.specs_active_dir.join("idea-Root-Idea.md")).unwrap(),
    )
    .unwrap();
    assert_eq!(generated.spec_id, "idea-Root-Idea");
    assert_eq!(generated.title, "Root Idea Title");
    assert_eq!(generated.summary, "Build watcher intake.");
    assert_eq!(generated.root_idea_id.as_deref(), Some("idea-Root-Idea"));
    assert_eq!(generated.root_spec_id.as_deref(), Some("idea-Root-Idea"));
    assert_eq!(
        generated.references,
        vec![
            "millrace-agents/intake/ideas/idea-Root-Idea.md",
            "ideas/inbox/Root Idea.md"
        ]
    );
    assert_eq!(
        fs::read_to_string(paths.intake_ideas_dir.join("idea-Root-Idea.md")).unwrap(),
        "# Root Idea Title\n\nBuild watcher intake.\n"
    );
    assert_eq!(session.snapshot.queue_depth_planning, 1);

    let events = runtime_events(&paths);
    assert!(events.iter().any(|event| {
        event["event_type"] == "idea_normalized_to_spec"
            && event["data"]["spec_id"] == "idea-Root-Idea"
            && event["data"]["source_artifact"] == "millrace-agents/intake/ideas/idea-Root-Idea.md"
    }));
    assert!(events.iter().any(|event| {
        event["event_type"] == "watcher_events_consumed"
            && event["data"]["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|watch_event| {
                    watch_event["target"] == "ideas_inbox"
                        && watch_event["path"] == "ideas/inbox/Root Idea.md"
                })
    }));
}

#[test]
fn daemon_watcher_observes_queue_and_config_changes_with_debounce() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        "[runtime]\nrun_style = \"daemon\"\n\n[watchers]\ndebounce_ms = 500\n",
    )
    .unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("watcher-debounce")).unwrap();
    session.snapshot.paused = true;
    save_snapshot(&paths, &session.snapshot).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());

    queue
        .enqueue_task(&task_document("task-watch-change"))
        .unwrap();
    queue
        .enqueue_spec(&spec_document("spec-watch-change"))
        .unwrap();
    fs::write(
        &paths.runtime_config_file,
        "[runtime]\nrun_style = \"daemon\"\nidle_sleep_seconds = 1.5\n\n[watchers]\ndebounce_ms = 500\n",
    )
    .unwrap();

    let first = supervisor
        .run_cycle(&mut session, runtime_tick_at("2026-04-29T02:15:00Z"))
        .unwrap();
    assert_eq!(first.kind, millrace_ai::RuntimeTickOutcomeKind::Paused);

    let consumed = runtime_events(&paths)
        .into_iter()
        .filter(|event| event["event_type"] == "watcher_events_consumed")
        .collect::<Vec<_>>();
    assert_eq!(consumed.len(), 1);
    let targets = consumed[0]["data"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["target"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(targets, vec!["config", "specs_queue", "tasks_queue"]);

    let mut changed_task = task_document("task-watch-change");
    changed_task.summary = "daemon watcher debounce changed task".to_owned();
    fs::write(
        paths.tasks_queue_dir.join("task-watch-change.md"),
        render_task_document(&changed_task),
    )
    .unwrap();

    let suppressed = supervisor
        .run_cycle(&mut session, runtime_tick_at("2026-04-29T02:15:00Z"))
        .unwrap();
    assert_eq!(suppressed.kind, millrace_ai::RuntimeTickOutcomeKind::Paused);
    assert_eq!(
        runtime_events(&paths)
            .iter()
            .filter(|event| event["event_type"] == "watcher_events_consumed")
            .count(),
        1
    );

    let emitted_after_quiet = supervisor
        .run_cycle(&mut session, runtime_tick_at("2026-04-29T02:15:01Z"))
        .unwrap();
    assert_eq!(
        emitted_after_quiet.kind,
        millrace_ai::RuntimeTickOutcomeKind::Paused
    );
    let consumed = runtime_events(&paths)
        .into_iter()
        .filter(|event| event["event_type"] == "watcher_events_consumed")
        .collect::<Vec<_>>();
    assert_eq!(consumed.len(), 2);
    assert_eq!(consumed[1]["data"]["count"], 1);
    assert_eq!(consumed[1]["data"]["events"][0]["target"], "tasks_queue");
}

#[test]
fn daemon_watcher_handles_missing_roots_deleted_files_and_bad_idea_payloads() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("watcher-safe-failures")).unwrap();
    session.snapshot.paused = true;
    save_snapshot(&paths, &session.snapshot).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());

    let missing_root = supervisor
        .run_cycle(&mut session, runtime_tick_at("2026-04-29T02:15:00Z"))
        .unwrap();
    assert_eq!(
        missing_root.kind,
        millrace_ai::RuntimeTickOutcomeKind::Paused
    );
    assert!(
        runtime_events(&paths)
            .iter()
            .all(|event| event["event_type"] != "watcher_events_consumed")
    );

    queue
        .enqueue_task(&task_document("task-delete-watch"))
        .unwrap();
    supervisor
        .run_cycle(&mut session, runtime_tick_at("2026-04-29T02:15:01Z"))
        .unwrap();
    fs::remove_file(paths.tasks_queue_dir.join("task-delete-watch.md")).unwrap();
    supervisor
        .run_cycle(&mut session, runtime_tick_at("2026-04-29T02:15:02Z"))
        .unwrap();
    let consumed_count = runtime_events(&paths)
        .iter()
        .filter(|event| event["event_type"] == "watcher_events_consumed")
        .count();
    assert_eq!(consumed_count, 1);

    let ideas_dir = paths.root.join("ideas/inbox");
    fs::create_dir_all(&ideas_dir).unwrap();
    fs::write(ideas_dir.join("bad-utf8.md"), [0xff, 0xfe, 0xfd]).unwrap();
    supervisor
        .run_cycle(&mut session, runtime_tick_at("2026-04-29T02:15:03Z"))
        .unwrap();
    let events = runtime_events(&paths);
    assert!(events.iter().any(|event| {
        event["event_type"] == "watcher_event_failed"
            && event["data"]["target"] == "ideas_inbox"
            && event["data"]["path"] == "ideas/inbox/bad-utf8.md"
    }));
    assert!(events.iter().any(|event| {
        event["event_type"] == "watcher_events_consumed" && event["data"]["failure_count"] == 1
    }));
    assert!(!paths.specs_queue_dir.join("idea-bad-utf8.md").exists());
}

#[test]
fn daemon_watcher_skips_duplicate_idea_normalization_by_existing_spec_lineage_or_reference() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("watcher-duplicate-ideas"))
            .unwrap();
    session.snapshot.paused = true;
    save_snapshot(&paths, &session.snapshot).unwrap();

    let mut same_spec_id = spec_document("idea-represented-by-spec-id");
    same_spec_id.root_idea_id = Some("existing-root".to_owned());
    same_spec_id.root_spec_id = Some("existing-root".to_owned());
    queue.enqueue_spec(&same_spec_id).unwrap();

    let mut same_root = spec_document("spec-existing-root");
    same_root.root_idea_id = Some("idea-represented-by-root".to_owned());
    same_root.root_spec_id = Some("spec-existing-root".to_owned());
    queue.enqueue_spec(&same_root).unwrap();

    let mut same_reference = spec_document("spec-existing-reference");
    same_reference.references = vec!["ideas/inbox/represented-by-reference.md".to_owned()];
    queue.enqueue_spec(&same_reference).unwrap();

    let ideas_dir = paths.root.join("ideas/inbox");
    fs::create_dir_all(&ideas_dir).unwrap();
    fs::write(ideas_dir.join("represented-by-spec-id.md"), "# Duplicate\n").unwrap();
    fs::write(ideas_dir.join("represented-by-root.md"), "# Duplicate\n").unwrap();
    fs::write(
        ideas_dir.join("represented-by-reference.md"),
        "# Duplicate\n",
    )
    .unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());

    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(outcome.kind, millrace_ai::RuntimeTickOutcomeKind::Paused);
    assert!(
        !paths
            .specs_queue_dir
            .join("idea-represented-by-root.md")
            .exists()
    );
    assert!(
        !paths
            .specs_queue_dir
            .join("idea-represented-by-reference.md")
            .exists()
    );
    assert_eq!(
        runtime_events(&paths)
            .iter()
            .filter(|event| event["event_type"] == "idea_normalization_skipped")
            .count(),
        3
    );
    assert!(
        runtime_events(&paths)
            .iter()
            .all(|event| { event["event_type"] != "idea_normalized_to_spec" })
    );
}

#[test]
fn daemon_supervisor_default_mode_dispatches_only_one_foreground_lane() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_spec(&spec_document("spec-serial")).unwrap();
    queue.enqueue_task(&task_document("task-serial")).unwrap();

    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("supervisor-serial")).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(outcome.dispatched_count, 1);
    assert_eq!(supervisor.active_worker_planes(), vec![Plane::Planning]);
    assert!(paths.specs_active_dir.join("spec-serial.md").is_file());
    assert!(paths.tasks_queue_dir.join("task-serial.md").is_file());
    assert!(!paths.tasks_active_dir.join("task-serial.md").exists());
    assert_eq!(session.snapshot.active_runs_by_plane.len(), 1);
    assert!(
        session
            .snapshot
            .active_runs_by_plane
            .contains_key(&Plane::Planning)
    );

    assert!(paths.tasks_queue_dir.join("task-serial.md").is_file());
}

#[test]
fn daemon_supervisor_learning_mode_dispatches_learning_beside_execution() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-foreground"))
        .unwrap();
    queue
        .enqueue_learning_request(&learning_request_document("learn-sidecar"))
        .unwrap();

    let mut options = daemon_options("supervisor-learning");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_daemon_for_paths(&paths, options).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());

    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(outcome.dispatched_count, 2);
    assert_eq!(
        supervisor.active_worker_planes(),
        vec![Plane::Execution, Plane::Learning]
    );
    assert!(paths.tasks_active_dir.join("task-foreground.md").is_file());
    assert!(
        paths
            .learning_requests_active_dir
            .join("learn-sidecar.md")
            .is_file()
    );
    assert!(
        session
            .snapshot
            .active_runs_by_plane
            .contains_key(&Plane::Execution)
    );
    assert!(
        session
            .snapshot
            .active_runs_by_plane
            .contains_key(&Plane::Learning)
    );
}

#[test]
fn daemon_supervisor_planner_trigger_dispatches_librarian_and_traces_spawned_request() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document("spec-daemon-librarian"))
        .unwrap();

    let planner_run_dir = paths.runs_dir.join("run-daemon-planner-librarian");
    fs::create_dir_all(&planner_run_dir).unwrap();
    fs::write(
        planner_run_dir.join("planner_summary.md"),
        "# Planner Summary\n\nGenerated or refined spec paths:\n- millrace-agents/specs/active/spec-daemon-librarian.md\n",
    )
    .unwrap();

    let mut options = daemon_options("supervisor-planner-librarian");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_daemon_for_paths(&paths, options).unwrap();
    let mut planner_result = FakeRunnerResult::structured_terminal_result(
        "PLANNER_COMPLETE",
        Some(ResultClass::Success),
    );
    if let FakeRunnerOutput::StructuredTerminalResult {
        summary_artifact_paths,
        ..
    } = &mut planner_result.output
    {
        summary_artifact_paths.push("planner_summary.md".to_owned());
    }
    let runner = FakeRunner::new(
        FakeRunnerConfig::new(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"))
            .unwrap()
            .with_stage_result(StageName::Planner, planner_result)
            .with_stage_result(
                StageName::Manager,
                FakeRunnerResult::terminal_marker("### MANAGER_COMPLETE"),
            )
            .with_stage_result(
                StageName::Librarian,
                FakeRunnerResult::structured_terminal_result(
                    "LIBRARIAN_COMPLETE",
                    Some(ResultClass::Success),
                ),
            ),
    );
    let mut supervisor = RuntimeDaemonSupervisor::new(runner);

    let first = supervisor
        .run_cycle(
            &mut session,
            fixed_tick_options("run-daemon-planner-librarian", "request-daemon-planner"),
        )
        .unwrap();
    assert_eq!(first.dispatched_count, 1);
    assert!(first.completions.is_empty());
    assert_eq!(supervisor.active_worker_planes(), vec![Plane::Planning]);

    let mut saw_planner_completion = false;
    let mut saw_librarian_dispatch = false;
    let mut saw_librarian_completion = false;
    for _ in 0..5 {
        let outcome = supervisor
            .run_cycle(&mut session, runtime_tick_options())
            .unwrap();

        if outcome.completions.iter().any(|completion| {
            completion
                .stage_result
                .as_ref()
                .is_some_and(|stage_result| stage_result.stage == StageName::Planner)
        }) {
            saw_planner_completion = true;
            let mut learning_documents = Vec::new();
            for directory in [
                &paths.learning_requests_queue_dir,
                &paths.learning_requests_active_dir,
            ] {
                for entry in fs::read_dir(directory).unwrap() {
                    let path = entry.unwrap().path();
                    if path.extension().and_then(|value| value.to_str()) == Some("md") {
                        learning_documents.push(
                            parse_learning_request_document(&fs::read_to_string(path).unwrap())
                                .unwrap(),
                        );
                    }
                }
            }
            assert_eq!(learning_documents.len(), 1);
            let document = &learning_documents[0];
            assert_eq!(document.requested_action, LearningRequestAction::Install);
            assert_eq!(document.target_stage, Some(LearningStageName::Librarian));
            assert_eq!(
                document.trigger_metadata["rule_id"],
                "planning.planner.complete-to-librarian"
            );
            assert!(
                document.trigger_metadata["source_active_work_item_path"]
                    .as_str()
                    .unwrap()
                    .ends_with("millrace-agents/specs/active/spec-daemon-librarian.md")
            );
            assert!(
                document.trigger_metadata["stage_result_path"]
                    .as_str()
                    .unwrap()
                    .ends_with("stage_results/request-daemon-planner.json")
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

            let trace = RunTraceGraph::from_json_str(
                &fs::read_to_string(planner_run_dir.join("run_trace.json")).unwrap(),
            )
            .unwrap();
            let spawned = trace
                .edges
                .iter()
                .flat_map(|edge| edge.spawned_work.iter())
                .find(|spawned| spawned.kind.as_str() == "learning_request")
                .unwrap();
            assert_eq!(spawned.reason.as_deref(), Some("learning_trigger"));
            assert_eq!(
                spawned.source_terminal_result.as_deref(),
                Some("PLANNER_COMPLETE")
            );
        }

        if session
            .snapshot
            .active_runs_by_plane
            .get(&Plane::Learning)
            .is_some_and(|active| active.stage == StageName::Librarian)
        {
            saw_librarian_dispatch = true;
        }

        if let Some(completion) = outcome.completions.iter().find(|completion| {
            completion
                .stage_result
                .as_ref()
                .is_some_and(|stage_result| stage_result.stage == StageName::Librarian)
        }) {
            saw_librarian_completion = true;
            let stage_result = completion.stage_result.as_ref().unwrap();
            assert_eq!(
                stage_result.terminal_result,
                TerminalResult::Learning(LearningTerminalResult::LibrarianComplete)
            );
            assert_eq!(stage_result.result_class, ResultClass::Success);
            assert_eq!(completion.request.runner_name.as_deref(), Some("codex_cli"));
            assert_eq!(
                completion.request.running_status_marker,
                "LIBRARIAN_RUNNING"
            );
            assert!(
                completion
                    .request
                    .entrypoint_path
                    .ends_with("millrace-agents/entrypoints/learning/librarian.md")
            );
            break;
        }
    }

    assert!(saw_planner_completion);
    assert!(saw_librarian_dispatch);
    assert!(saw_librarian_completion);
    assert_eq!(
        fs::read_dir(&paths.learning_requests_done_dir)
            .unwrap()
            .count(),
        1
    );
    assert_eq!(load_learning_status(&paths).unwrap(), "### IDLE");
}

#[test]
fn daemon_supervisor_learning_mode_keeps_planning_and_execution_mutually_exclusive() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_spec(&spec_document("spec-foreground"))
        .unwrap();
    queue
        .enqueue_task(&task_document("task-blocked-by-planning"))
        .unwrap();

    let mut options = daemon_options("supervisor-foreground-exclusive");
    options.requested_mode_id = Some("learning_codex".to_owned());
    let mut session = startup_runtime_daemon_for_paths(&paths, options).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());

    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(outcome.dispatched_count, 1);
    assert_eq!(supervisor.active_worker_planes(), vec![Plane::Planning]);
    assert!(paths.specs_active_dir.join("spec-foreground.md").is_file());
    assert!(
        paths
            .tasks_queue_dir
            .join("task-blocked-by-planning.md")
            .is_file()
    );
    assert!(
        !session
            .snapshot
            .active_runs_by_plane
            .contains_key(&Plane::Execution)
    );
}

#[test]
fn daemon_supervisor_drains_completed_workers_before_new_claims() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-finishing"))
        .unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("supervisor-drain-first")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    force_execution_updater_active(&mut session, "task-finishing", "run-finishing");
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    let dispatched = supervisor
        .dispatch_ready_work(&mut session, runtime_tick_options())
        .unwrap();
    assert_eq!(dispatched, 1);
    assert_eq!(supervisor.active_worker_planes(), vec![Plane::Execution]);

    queue
        .enqueue_spec(&spec_document("spec-after-completion"))
        .unwrap();
    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(outcome.completions.len(), 1);
    assert_eq!(outcome.completions[0].plane, Plane::Execution);
    assert_eq!(outcome.dispatched_count, 1);
    assert!(paths.tasks_done_dir.join("task-finishing.md").is_file());
    assert!(
        paths
            .specs_active_dir
            .join("spec-after-completion.md")
            .is_file()
    );

    let events = runtime_events(&paths);
    let completed_index = events
        .iter()
        .position(|event| {
            event["event_type"] == "stage_completed"
                && event["data"]["work_item_id"] == "task-finishing"
        })
        .unwrap();
    let started_index = events
        .iter()
        .position(|event| {
            event["event_type"] == "stage_started"
                && event["data"]["work_item_id"] == "spec-after-completion"
        })
        .unwrap();
    assert!(completed_index < started_index);
}

#[test]
fn daemon_supervisor_governance_pause_after_completion_blocks_new_claims() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    write_daemon_session_governance_config(&paths);
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-governed-finishing"))
        .unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("supervisor-governance")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    force_execution_updater_active(
        &mut session,
        "task-governed-finishing",
        "run-governed-finishing",
    );
    let mut supervisor = RuntimeDaemonSupervisor::new(governance_supervisor_runner());
    let dispatched = supervisor
        .dispatch_ready_work(&mut session, runtime_tick_options())
        .unwrap();
    assert_eq!(dispatched, 1);

    queue
        .enqueue_spec(&spec_document("spec-blocked-by-governance"))
        .unwrap();
    let outcome = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();

    assert_eq!(outcome.kind, millrace_ai::RuntimeTickOutcomeKind::Paused);
    assert_eq!(outcome.completions.len(), 1);
    assert_eq!(outcome.dispatched_count, 0);
    assert!(
        paths
            .tasks_done_dir
            .join("task-governed-finishing.md")
            .is_file()
    );
    assert!(
        paths
            .specs_queue_dir
            .join("spec-blocked-by-governance.md")
            .is_file()
    );
    assert!(
        !paths
            .specs_active_dir
            .join("spec-blocked-by-governance.md")
            .exists()
    );
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.paused);
    assert_eq!(
        snapshot.pause_sources,
        vec![millrace_ai::contracts::PauseSource::UsageGovernance]
    );
    let events = runtime_events(&paths);
    let completed_index = events
        .iter()
        .position(|event| {
            event["event_type"] == "stage_completed"
                && event["data"]["work_item_id"] == "task-governed-finishing"
        })
        .unwrap();
    let blocked_index = events
        .iter()
        .position(|event| event["event_type"] == "usage_governance_blocked")
        .unwrap();
    assert!(completed_index < blocked_index);
}

#[test]
fn daemon_supervisor_refuses_worker_metadata_mismatch_before_artifact_application() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-mismatch")).unwrap();

    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("supervisor-mismatch")).unwrap();
    let activation = run_serial_runtime_tick(
        &mut session,
        fixed_tick_options("run-mismatch", "request-mismatch"),
    )
    .unwrap();
    let request = activation.stage_request.unwrap();
    let active_run = session
        .snapshot
        .active_runs_by_plane
        .get(&Plane::Execution)
        .cloned()
        .unwrap();
    let runner = supervisor_runner();
    let mut outcome = run_stage_worker(active_run, request.clone(), &runner).unwrap();
    outcome.active_run.node_id = "wrong-node".to_owned();

    let error = apply_stage_worker_outcome(&mut session, outcome).unwrap_err();

    assert!(error.to_string().contains("stage_worker_metadata_mismatch"));
    assert_eq!(
        directory_file_count(&std::path::Path::new(&request.run_dir).join("stage_results")),
        0
    );
    assert!(paths.tasks_active_dir.join("task-mismatch.md").is_file());
    assert!(!paths.tasks_done_dir.join("task-mismatch.md").exists());
    assert_eq!(
        load_execution_status(&paths).unwrap(),
        "### BUILDER_RUNNING"
    );
}

#[test]
fn daemon_loop_runs_one_max_tick_drains_worker_and_releases_lock() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-loop-max")).unwrap();

    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("loop-max-tick")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    force_execution_updater_active(&mut session, "task-loop-max", "run-loop-max");
    let mut options = RuntimeDaemonLoopOptions::max_ticks(1);
    options.tick_options = runtime_tick_options();
    let outcome = run_runtime_daemon_loop(session, supervisor_runner(), options).unwrap();

    assert_eq!(outcome.completed_tick_count, 1);
    assert_eq!(outcome.exit_reason, RuntimeDaemonLoopExitReason::MaxTicks);
    assert_eq!(outcome.post_cycle_completions.len(), 1);
    assert_eq!(outcome.post_cycle_completions[0].plane, Plane::Execution);
    assert_eq!(outcome.idle_sleep_count, 0);
    assert!(outcome.runtime_ownership_released);
    assert!(!outcome.final_snapshot.process_running);
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Absent
    );
    assert!(paths.tasks_done_dir.join("task-loop-max.md").is_file());
    assert!(!paths.tasks_active_dir.join("task-loop-max.md").exists());
    let trace_path = paths.runs_dir.join("run-loop-max/run_trace.json");
    assert!(trace_path.is_file());
    let trace = RunTraceGraph::from_json_str(&fs::read_to_string(trace_path).unwrap()).unwrap();
    assert_eq!(trace.status.as_str(), "complete");
    assert_eq!(trace.nodes[0].stage, "updater");
    assert_eq!(trace.edges[0].edge_kind, "idle");
}

#[test]
fn daemon_supervisor_integrated_mode_drains_builder_integrator_checker_sequence() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        "[runtime]\ndefault_mode = \"default_codex_integrated\"\nrun_style = \"daemon\"\n",
    )
    .unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-integrated-daemon"))
        .unwrap();

    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("integrated-daemon")).unwrap();
    assert_eq!(
        session.compiled_plan.execution_loop_id,
        "execution.with_integrator"
    );
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());

    let first = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();
    assert_eq!(first.dispatched_count, 1);
    assert!(first.completions.is_empty());

    let second = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();
    assert_eq!(second.completions.len(), 1);
    assert_eq!(
        second.completions[0].stage_result.as_ref().unwrap().stage,
        StageName::Builder
    );
    assert_eq!(
        second.completions[0]
            .router_decision
            .as_ref()
            .unwrap()
            .next_stage,
        Some(StageName::Integrator)
    );
    assert_eq!(second.dispatched_count, 1);
    assert_eq!(
        load_snapshot(&paths).unwrap().active_stage,
        Some(StageName::Integrator)
    );

    let third = supervisor
        .run_cycle(&mut session, runtime_tick_options())
        .unwrap();
    assert_eq!(third.completions.len(), 1);
    assert_eq!(
        third.completions[0].stage_result.as_ref().unwrap().stage,
        StageName::Integrator
    );
    assert_eq!(
        third.completions[0]
            .router_decision
            .as_ref()
            .unwrap()
            .next_stage,
        Some(StageName::Checker)
    );
    assert_eq!(third.dispatched_count, 1);
    assert_eq!(
        load_snapshot(&paths).unwrap().active_stage,
        Some(StageName::Checker)
    );

    let run_id = &second.completions[0].run_id;
    let trace = RunTraceGraph::from_json_str(
        &fs::read_to_string(paths.runs_dir.join(run_id).join("run_trace.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        trace
            .nodes
            .iter()
            .map(|node| node.stage.as_str())
            .collect::<Vec<_>>(),
        vec!["builder", "integrator"]
    );
    assert_eq!(trace.edges[0].target_node_id.as_deref(), Some("integrator"));
    assert_eq!(trace.edges[1].target_node_id.as_deref(), Some("checker"));

    session.finish().unwrap();
}

#[test]
fn daemon_loop_monitor_streams_owner_side_stage_and_router_events() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-monitor-loop"))
        .unwrap();

    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("loop-monitor-stage")).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    let mut sleeper = RecordingSleeper::default();
    let mut monitor = BasicTerminalMonitor::new(Vec::new());
    let mut options = RuntimeDaemonLoopOptions::max_ticks(1);
    options.tick_options = runtime_tick_options();

    let outcome = run_runtime_daemon_supervisor_loop_with_sleeper_and_monitor(
        &mut session,
        &mut supervisor,
        options,
        &mut sleeper,
        &mut monitor,
    )
    .unwrap();
    let output = String::from_utf8(monitor.into_inner()).unwrap();

    assert_eq!(outcome.completed_tick_count, 1);
    assert!(output.contains("stage start execution/builder"));
    assert!(output.contains("stage done execution/builder"));
    assert!(output.contains("route execution"));
    assert!(
        output.find("stage start execution/builder").unwrap()
            < output.find("stage done execution/builder").unwrap()
    );
    assert!(
        output.find("stage done execution/builder").unwrap()
            < output.find("route execution").unwrap()
    );
}

#[test]
fn daemon_loop_monitor_streams_pause_and_stop_lines() {
    let paused_temp = TempDir::new().unwrap();
    let paused_paths = initialize_workspace(paused_temp.path().join("workspace")).unwrap();
    let mut paused_session =
        startup_runtime_daemon_for_paths(&paused_paths, daemon_options("loop-monitor-pause"))
            .unwrap();
    paused_session.snapshot.paused = true;
    save_snapshot(&paused_paths, &paused_session.snapshot).unwrap();
    let mut paused_supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    let mut paused_sleeper = RecordingSleeper::default();
    let mut paused_monitor = BasicTerminalMonitor::new(Vec::new());
    let mut paused_options = RuntimeDaemonLoopOptions::max_ticks(1);
    paused_options.tick_options = runtime_tick_options();

    run_runtime_daemon_supervisor_loop_with_sleeper_and_monitor(
        &mut paused_session,
        &mut paused_supervisor,
        paused_options,
        &mut paused_sleeper,
        &mut paused_monitor,
    )
    .unwrap();
    let paused_output = String::from_utf8(paused_monitor.into_inner()).unwrap();
    assert!(paused_output.contains("paused reason=paused"));

    let stopped_temp = TempDir::new().unwrap();
    let stopped_paths = initialize_workspace(stopped_temp.path().join("workspace")).unwrap();
    let mut stopped_session =
        startup_runtime_daemon_for_paths(&stopped_paths, daemon_options("loop-monitor-stop"))
            .unwrap();
    stopped_session.snapshot.stop_requested = true;
    save_snapshot(&stopped_paths, &stopped_session.snapshot).unwrap();
    let mut stopped_supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    let mut stopped_sleeper = RecordingSleeper::default();
    let mut stopped_monitor = BasicTerminalMonitor::new(Vec::new());
    let stopped_options = RuntimeDaemonLoopOptions {
        tick_options: runtime_tick_options(),
        ..Default::default()
    };

    let stopped_outcome = run_runtime_daemon_supervisor_loop_with_sleeper_and_monitor(
        &mut stopped_session,
        &mut stopped_supervisor,
        stopped_options,
        &mut stopped_sleeper,
        &mut stopped_monitor,
    )
    .unwrap();
    let stopped_output = String::from_utf8(stopped_monitor.into_inner()).unwrap();
    assert_eq!(
        stopped_outcome.exit_reason,
        RuntimeDaemonLoopExitReason::StopRequested
    );
    assert!(stopped_output.contains("stopped reason=stop_requested"));
}

#[test]
fn daemon_loop_uses_configured_idle_sleep_without_spinning() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        [
            "[runtime]",
            "default_mode = \"default_codex\"",
            "run_style = \"daemon\"",
            "idle_sleep_seconds = 2.5",
            "",
        ]
        .join("\n"),
    )
    .unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("loop-idle-sleep")).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    let mut sleeper = RecordingSleeper::default();
    let mut options = RuntimeDaemonLoopOptions::max_ticks(3);
    options.tick_options = runtime_tick_options();

    let outcome = run_runtime_daemon_supervisor_loop_with_sleeper(
        &mut session,
        &mut supervisor,
        options,
        &mut sleeper,
    )
    .unwrap();

    assert_eq!(outcome.completed_tick_count, 3);
    assert_eq!(outcome.exit_reason, RuntimeDaemonLoopExitReason::MaxTicks);
    assert_eq!(outcome.idle_sleep_count, 2);
    assert_eq!(sleeper.calls, vec![2.5, 2.5]);
    assert!(outcome.cycle_outcomes.iter().all(|cycle| {
        cycle.kind == millrace_ai::RuntimeTickOutcomeKind::NoWork
            && cycle.dispatched_count == 0
            && cycle.completions.is_empty()
    }));
    assert!(
        runtime_events(&paths)
            .iter()
            .filter(|event| event["event_type"] == "runtime_tick_idle")
            .count()
            >= 3
    );
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Absent
    );
}

#[test]
fn daemon_loop_continues_after_missing_root_idea_source_blocks_planning() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_spec(&spec_document("spec-daemon-missing-source"))
        .unwrap();
    queue.claim_next_planning_item(None).unwrap().unwrap();
    queue.mark_spec_done("spec-daemon-missing-source").unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("loop-missing-source")).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    let mut sleeper = RecordingSleeper::default();
    let mut options = RuntimeDaemonLoopOptions::max_ticks(2);
    options.tick_options = runtime_tick_options();

    let outcome = run_runtime_daemon_supervisor_loop_with_sleeper(
        &mut session,
        &mut supervisor,
        options,
        &mut sleeper,
    )
    .unwrap();

    assert_eq!(outcome.completed_tick_count, 2);
    assert_eq!(outcome.exit_reason, RuntimeDaemonLoopExitReason::MaxTicks);
    assert!(
        outcome
            .cycle_outcomes
            .iter()
            .all(|cycle| cycle.kind == millrace_ai::RuntimeTickOutcomeKind::NoWork)
    );
    assert_eq!(load_planning_status(&paths).unwrap(), "### BLOCKED");
    assert_eq!(
        load_snapshot(&paths)
            .unwrap()
            .current_failure_class
            .as_deref(),
        Some("missing_root_idea_source")
    );
    assert!(runtime_events(&paths).iter().any(|event| {
        event["event_type"] == "root_idea_source_missing"
            && event["data"]["root_idea_id"] == "idea-daemon"
            && event["data"]["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .any(|candidate| {
                    candidate.as_str() == Some("millrace-agents/intake/ideas/idea-daemon.md")
                })
    }));
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Absent
    );
}

#[test]
fn daemon_loop_can_exit_after_clean_no_work_idle_cycle() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("loop-idle-exit")).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    let mut sleeper = RecordingSleeper::default();
    let mut options = RuntimeDaemonLoopOptions::exit_on_idle();
    options.tick_options = runtime_tick_options();

    let outcome = run_runtime_daemon_supervisor_loop_with_sleeper(
        &mut session,
        &mut supervisor,
        options,
        &mut sleeper,
    )
    .unwrap();

    assert_eq!(outcome.completed_tick_count, 1);
    assert_eq!(outcome.exit_reason, RuntimeDaemonLoopExitReason::NoWorkIdle);
    assert_eq!(outcome.idle_sleep_count, 0);
    assert!(sleeper.calls.is_empty());
    assert!(!outcome.final_snapshot.process_running);
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Absent
    );
}

#[test]
fn daemon_loop_pause_drains_workers_without_claiming_new_work() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-paused-drain"))
        .unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("loop-pause-drain")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    force_execution_updater_active(&mut session, "task-paused-drain", "run-paused-drain");
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    let dispatched = supervisor
        .dispatch_ready_work(&mut session, runtime_tick_options())
        .unwrap();
    assert_eq!(dispatched, 1);
    assert_eq!(supervisor.active_worker_planes(), vec![Plane::Execution]);

    queue
        .enqueue_task(&task_document("task-paused-queued"))
        .unwrap();
    session.snapshot.paused = true;
    save_snapshot(&paths, &session.snapshot).unwrap();

    let mut sleeper = RecordingSleeper::default();
    let mut options = RuntimeDaemonLoopOptions::max_ticks(1);
    options.tick_options = runtime_tick_options();
    let outcome = run_runtime_daemon_supervisor_loop_with_sleeper(
        &mut session,
        &mut supervisor,
        options,
        &mut sleeper,
    )
    .unwrap();

    assert_eq!(outcome.completed_tick_count, 1);
    assert_eq!(outcome.exit_reason, RuntimeDaemonLoopExitReason::MaxTicks);
    assert_eq!(
        outcome.cycle_outcomes[0].kind,
        millrace_ai::RuntimeTickOutcomeKind::Paused
    );
    assert_eq!(outcome.cycle_outcomes[0].completions.len(), 1);
    assert!(paths.tasks_done_dir.join("task-paused-drain.md").is_file());
    assert!(
        paths
            .tasks_queue_dir
            .join("task-paused-queued.md")
            .is_file()
    );
    assert!(
        !paths
            .tasks_active_dir
            .join("task-paused-queued.md")
            .exists()
    );
    assert!(outcome.final_snapshot.paused);
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Absent
    );
}

#[test]
fn daemon_loop_stop_drains_workers_resets_state_and_releases_matching_lock() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&task_document("task-stop-drain"))
        .unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("loop-stop-drain")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();
    force_execution_updater_active(&mut session, "task-stop-drain", "run-stop-drain");
    let mut supervisor = RuntimeDaemonSupervisor::new(supervisor_runner());
    let dispatched = supervisor
        .dispatch_ready_work(&mut session, runtime_tick_options())
        .unwrap();
    assert_eq!(dispatched, 1);
    session.snapshot.stop_requested = true;
    session.snapshot.paused = true;
    save_snapshot(&paths, &session.snapshot).unwrap();

    let mut sleeper = RecordingSleeper::default();
    let options = RuntimeDaemonLoopOptions {
        tick_options: runtime_tick_options(),
        ..Default::default()
    };
    let outcome = run_runtime_daemon_supervisor_loop_with_sleeper(
        &mut session,
        &mut supervisor,
        options,
        &mut sleeper,
    )
    .unwrap();

    assert_eq!(outcome.completed_tick_count, 1);
    assert_eq!(
        outcome.exit_reason,
        RuntimeDaemonLoopExitReason::StopRequested
    );
    assert_eq!(outcome.cycle_outcomes[0].completions.len(), 1);
    assert!(paths.tasks_done_dir.join("task-stop-drain.md").is_file());
    assert!(!outcome.final_snapshot.process_running);
    assert!(!outcome.final_snapshot.stop_requested);
    assert!(!outcome.final_snapshot.paused);
    assert!(outcome.final_snapshot.active_runs_by_plane.is_empty());
    assert_eq!(load_execution_status(&paths).unwrap(), "### IDLE");
    assert!(
        runtime_events(&paths)
            .iter()
            .any(|event| event["event_type"] == "runtime_tick_stopped")
    );
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Absent
    );
}
