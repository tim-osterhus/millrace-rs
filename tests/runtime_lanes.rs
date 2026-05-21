use std::process;

use tempfile::TempDir;

use millrace_ai::contracts::{
    ActiveRunRequestKind, ActiveRunState, LaneRuntimeStatus, LearningRequestAction,
    LearningRequestDocument, Plane, RuntimeJsonContract, RuntimeMode, StageName, TaskDocument,
    Timestamp, WorkItemKind,
};
use millrace_ai::runtime::{can_dispatch_lane, ensure_snapshot_lanes, lane_dispatch_order};
use millrace_ai::workspace::{
    QueueStore, RuntimeOwnershipLockOptions, initialize_workspace, load_snapshot,
};
use millrace_ai::{
    FakeRunner, FakeRunnerConfig, FakeRunnerResult, RuntimeDaemonSupervisor, RuntimeStartupOptions,
    RuntimeTickOptions, startup_runtime_daemon_for_paths,
};

const NOW: &str = "2026-05-21T07:00:00Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("timestamp", value).unwrap()
}

fn lock_options(session_id: &str) -> RuntimeOwnershipLockOptions {
    RuntimeOwnershipLockOptions::new(process::id(), "test-host", session_id, NOW).unwrap()
}

fn daemon_options(session_id: &str) -> RuntimeStartupOptions {
    RuntimeStartupOptions {
        requested_mode_id: Some("learning_codex".to_owned()),
        runtime_mode: RuntimeMode::Daemon,
        lock_options: Some(lock_options(session_id)),
        now: Some(timestamp(NOW)),
        ..RuntimeStartupOptions::default()
    }
}

fn tick_options() -> RuntimeTickOptions {
    RuntimeTickOptions {
        now: Some(timestamp("2026-05-21T07:01:00Z")),
        run_id: None,
        request_id: None,
    }
}

fn task_document(task_id: &str) -> TaskDocument {
    TaskDocument {
        task_id: task_id.to_owned(),
        title: format!("Task {task_id}"),
        summary: "lane dispatch test".to_owned(),
        root_idea_id: Some("idea-lanes".to_owned()),
        root_spec_id: Some("spec-lanes".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        spec_id: Some("spec-lanes".to_owned()),
        parent_task_id: None,
        incident_id: None,
        target_paths: vec!["src/runtime/lanes.rs".to_owned()],
        acceptance: vec!["runtime records durable lane state".to_owned()],
        required_checks: vec!["cargo test --test runtime_lanes".to_owned()],
        references: vec!["src/runtime/lanes.rs".to_owned()],
        risk: vec!["lane dispatch must remain single-writer".to_owned()],
        depends_on: Vec::new(),
        blocks: Vec::new(),
        tags: vec!["scheduler-lanes".to_owned()],
        status_hint: None,
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn learning_request_document(learning_request_id: &str) -> LearningRequestDocument {
    LearningRequestDocument {
        learning_request_id: learning_request_id.to_owned(),
        title: format!("Learning request {learning_request_id}"),
        summary: "lane sidecar test".to_owned(),
        requested_action: LearningRequestAction::Improve,
        target_skill_id: Some("builder-core".to_owned()),
        target_stage: None,
        source_refs: vec!["run:run-lanes".to_owned()],
        preferred_output_paths: Vec::new(),
        trigger_metadata: serde_json::json!({"source": "runtime_lanes"}),
        originating_run_ids: Vec::new(),
        artifact_paths: Vec::new(),
        references: Vec::new(),
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn runner() -> FakeRunner {
    let config = FakeRunnerConfig::new(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"))
        .unwrap()
        .with_stage_result(
            StageName::Analyst,
            FakeRunnerResult::terminal_marker("### ANALYST_COMPLETE"),
        );
    FakeRunner::new(config)
}

#[test]
fn compiled_lane_policy_enforces_plane_concurrency_and_lane_limits() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("lane-policy")).unwrap();
    let lane_policy = session.compiled_plan.lane_policy.as_ref().unwrap();
    let concurrency_policy = session.compiled_plan.concurrency_policy.as_ref();

    assert_eq!(
        lane_dispatch_order(Some(&session.compiled_plan)),
        vec![
            "planning.main".to_owned(),
            "execution.main".to_owned(),
            "learning.main".to_owned()
        ]
    );
    assert!(can_dispatch_lane(
        Some(lane_policy),
        concurrency_policy,
        vec!["execution.main"],
        "learning.main",
    ));
    assert!(!can_dispatch_lane(
        Some(lane_policy),
        concurrency_policy,
        vec!["execution.main"],
        "planning.main",
    ));
    assert!(!can_dispatch_lane(
        Some(lane_policy),
        concurrency_policy,
        vec!["execution.main"],
        "execution.main",
    ));
    assert!(!can_dispatch_lane(
        Some(lane_policy),
        concurrency_policy,
        vec!["execution.main"],
        "unknown.main",
    ));

    session.close().unwrap();
}

#[test]
fn daemon_supervisor_persists_lane_state_for_parallel_execution_and_learning() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-lane")).unwrap();
    queue
        .enqueue_learning_request(&learning_request_document("learn-lane"))
        .unwrap();

    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("lane-supervisor")).unwrap();
    let mut supervisor = RuntimeDaemonSupervisor::new(runner());
    let outcome = supervisor.run_cycle(&mut session, tick_options()).unwrap();

    assert_eq!(outcome.dispatched_count, 2);
    assert_eq!(
        supervisor.active_worker_planes(),
        vec![Plane::Execution, Plane::Learning]
    );

    let snapshot = load_snapshot(&paths).unwrap();
    let execution = snapshot.lanes_by_id.get("execution.main").unwrap();
    assert_eq!(execution.status, LaneRuntimeStatus::Active);
    assert_eq!(execution.compiled_plan_id, snapshot.compiled_plan_id);
    assert_eq!(execution.active_run_ids.len(), 1);
    assert_eq!(execution.active_work_refs, vec!["task:task-lane"]);

    let learning = snapshot.lanes_by_id.get("learning.main").unwrap();
    assert_eq!(learning.status, LaneRuntimeStatus::Active);
    assert_eq!(learning.active_run_ids.len(), 1);
    assert_eq!(
        learning.active_work_refs,
        vec!["learning_request:learn-lane"]
    );

    session.close().unwrap();
}

#[test]
fn ensure_snapshot_lanes_repairs_declared_idle_state_without_losing_active_lanes() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let mut session =
        startup_runtime_daemon_for_paths(&paths, daemon_options("lane-repair")).unwrap();
    let mut snapshot = session.snapshot.clone();
    snapshot.lanes_by_id.clear();
    snapshot.active_runs_by_plane.insert(
        Plane::Execution,
        ActiveRunState {
            plane: Plane::Execution,
            lane_id: "execution.main".to_owned(),
            stage: StageName::Builder,
            node_id: "builder".to_owned(),
            stage_kind_id: "builder".to_owned(),
            run_id: "run-active-lane".to_owned(),
            compiled_plan_id: snapshot.compiled_plan_id.clone(),
            compiled_plan_fingerprint: snapshot.compiled_plan_fingerprint.clone(),
            request_kind: ActiveRunRequestKind::ActiveWorkItem,
            work_item_family_id: Some("task".to_owned()),
            work_item_kind: Some(WorkItemKind::Task),
            work_item_id: Some("task-active-lane".to_owned()),
            closure_target_root_spec_id: None,
            closure_target_root_idea_id: None,
            active_since: timestamp("2026-05-21T07:02:00Z"),
            running_status_marker: Some("### BUILDER_RUNNING".to_owned()),
        },
    );

    ensure_snapshot_lanes(&mut snapshot, &session.compiled_plan);
    snapshot.validate_contract().unwrap();

    assert_eq!(
        snapshot.lanes_by_id.get("execution.main").unwrap().status,
        LaneRuntimeStatus::Active
    );
    assert_eq!(
        snapshot
            .lanes_by_id
            .get("execution.main")
            .unwrap()
            .active_run_ids,
        vec!["run-active-lane"]
    );
    assert_eq!(
        snapshot.lanes_by_id.get("planning.main").unwrap().status,
        LaneRuntimeStatus::Idle
    );
    assert_eq!(
        snapshot.lanes_by_id.get("learning.main").unwrap().status,
        LaneRuntimeStatus::Idle
    );

    session.close().unwrap();
}
