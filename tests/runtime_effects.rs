use std::{fs, path::Path};

use serde_json::{Value, json};
use tempfile::TempDir;

use millrace_ai::contracts::{
    CapabilitySupportDecision, ExecutionCapabilityGrant, SpecDocument, SpecSourceType, Timestamp,
};
use millrace_ai::workspace::{
    QueueStore, RuntimeOwnershipLockOptions, initialize_workspace, load_planning_status,
};
use millrace_ai::{
    FakeRunner, FakeRunnerResult, RouterAction, RunnerRawResult, RunnerResult,
    RuntimeStartupOptions, RuntimeStartupSession, RuntimeTickOptions, RuntimeTickOutcomeKind,
    StageRunRequest, StageRunnerAdapter, inspect_run_trace, run_serial_runtime_tick_with_runner,
    startup_runtime_once_for_paths,
};

const STARTUP_NOW: &str = "2026-04-28T20:00:00Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("created_at", value).unwrap()
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

fn tick_options(run_id: &str, request_id: &str) -> RuntimeTickOptions {
    RuntimeTickOptions {
        now: Some(timestamp("2026-04-28T20:10:00Z")),
        run_id: Some(run_id.to_owned()),
        request_id: Some(request_id.to_owned()),
    }
}

fn spec_document(spec_id: &str) -> SpecDocument {
    SpecDocument {
        spec_id: spec_id.to_owned(),
        title: format!("Spec {spec_id}"),
        summary: "runtime effect spec".to_owned(),
        source_type: SpecSourceType::Idea,
        source_id: Some("idea-001".to_owned()),
        parent_spec_id: None,
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some(spec_id.to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        goals: vec!["plan runtime work".to_owned()],
        non_goals: Vec::new(),
        scope: vec!["runtime effects".to_owned()],
        constraints: vec!["serial once mode".to_owned()],
        assumptions: Vec::new(),
        risks: vec!["runtime effect lifecycle drift".to_owned()],
        target_paths: vec!["src/runtime/".to_owned()],
        entrypoints: Vec::new(),
        required_skills: Vec::new(),
        decomposition_hints: Vec::new(),
        acceptance: vec!["runtime effect dispatch is applied".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/runtime/effects.py".to_owned()],
        created_at: timestamp(STARTUP_NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn planner_disposition(
    source_spec_id: &str,
    disposition: &str,
    emitted_spec_ids: &[&str],
) -> Value {
    json!({
        "schema_version": "1.0",
        "kind": "planner_disposition",
        "source_work_item_family_id": "spec",
        "source_work_item_id": source_spec_id,
        "disposition": disposition,
        "emitted_spec_ids": emitted_spec_ids,
        "refined_active_source": true,
        "recommended_next_action": "continue_to_manager",
        "created_at": "2026-04-28T20:10:00Z",
        "created_by": "planner",
    })
}

fn write_planner_disposition(run_dir: &Path, disposition: &Value) {
    fs::create_dir_all(run_dir).unwrap();
    fs::write(
        run_dir.join("planner_disposition.json"),
        serde_json::to_string_pretty(disposition).unwrap() + "\n",
    )
    .unwrap();
}

fn runtime_events(event_log: &Path) -> Vec<Value> {
    fs::read_to_string(event_log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn planner_effect_rule_selections(session: &mut RuntimeStartupSession) -> &mut Vec<String> {
    &mut session
        .compiled_plan
        .planning_graph
        .nodes
        .iter_mut()
        .find(|node| node.node_id == "planner")
        .unwrap()
        .runtime_effect_rule_selections
}

fn add_runtime_effect_rule_clone(
    session: &mut RuntimeStartupSession,
    rule_id: &str,
    handler_id: &str,
) {
    let mut rule = session
        .compiled_plan
        .workflow_primitives
        .runtime_effect_rules
        .iter()
        .find(|rule| rule.rule_id == "planner_disposition_on_complete")
        .unwrap()
        .clone();
    rule.rule_id = rule_id.to_owned();
    rule.handler_id = handler_id.to_owned();
    session
        .compiled_plan
        .workflow_primitives
        .runtime_effect_rules
        .push(rule);
}

struct DeletePlannerDispositionRunner {
    inner: FakeRunner,
}

impl StageRunnerAdapter for DeletePlannerDispositionRunner {
    fn evaluate_capability_grant(
        &self,
        grant: &ExecutionCapabilityGrant,
        request: &StageRunRequest,
    ) -> CapabilitySupportDecision {
        self.inner.evaluate_capability_grant(grant, request)
    }

    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        let result = self.inner.run(request)?;
        let _ = fs::remove_file(Path::new(&request.run_dir).join("planner_disposition.json"));
        Ok(result)
    }
}

#[test]
fn planner_emitted_child_specs_completes_source_and_writes_effect_evidence() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    let root = spec_document("spec-effect-root");
    queue.enqueue_spec(&root).unwrap();
    let mut child = spec_document("spec-effect-child");
    child.parent_spec_id = Some(root.spec_id.clone());
    child.root_spec_id = Some(root.spec_id.clone());
    child.created_at = timestamp("2026-04-28T20:05:00Z");
    queue.enqueue_spec(&child).unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("runtime-effects-emitted")).unwrap();
    let run_id = "run-runtime-effects-emitted";
    let request_id = "request-runtime-effects-emitted";
    let run_dir = paths.runs_dir.join(run_id);
    write_planner_disposition(
        &run_dir,
        &planner_disposition(&root.spec_id, "emitted_child_specs", &[&child.spec_id]),
    );
    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"))
            .unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(run_id, request_id),
        &runner,
    )
    .unwrap();

    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::StageDispatched);
    assert_eq!(
        outcome.router_decision.as_ref().unwrap().action,
        RouterAction::Idle
    );
    assert!(paths.specs_done_dir.join("spec-effect-root.md").is_file());
    assert!(paths.specs_queue_dir.join("spec-effect-child.md").is_file());

    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(
        stage_result.metadata["runtime_effect_handler_id"],
        "planner_disposition"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_decision"],
        "request_complete_source"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_source_lifecycle_action"],
        "complete"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_mutation_phase"],
        "pre_mutation"
    );
    let created_paths = stage_result.metadata["runtime_effect_created_paths"]
        .as_array()
        .unwrap();
    assert!(created_paths.iter().any(|path| {
        path.as_str()
            .is_some_and(|path| path.ends_with("millrace-agents/specs/queue/spec-effect-child.md"))
    }));
    assert!(
        stage_result
            .artifact_paths
            .iter()
            .any(|path| { path == &format!("runtime_effect_decisions/{request_id}.json") })
    );
    assert!(
        stage_result
            .artifact_paths
            .iter()
            .any(|path| { path == &format!("runtime_effect_results/{request_id}.json") })
    );
    assert!(
        run_dir
            .join("runtime_effect_decisions")
            .join(format!("{request_id}.json"))
            .is_file()
    );
    assert!(
        run_dir
            .join("runtime_effect_results")
            .join(format!("{request_id}.json"))
            .is_file()
    );

    let events = runtime_events(&paths.logs_dir.join("runtime_events.jsonl"));
    assert!(events.iter().any(|event| {
        event["event_type"] == "runtime_effect_applied"
            && event["data"]["decision"] == "request_complete_source"
    }));
    let trace = inspect_run_trace(&run_dir).unwrap();
    let artifacts = &trace.nodes[0].artifacts;
    assert!(artifacts.iter().any(|artifact| {
        artifact.path == format!("runtime_effect_decisions/{request_id}.json")
    }));
    assert!(
        artifacts.iter().any(|artifact| {
            artifact.path == format!("runtime_effect_results/{request_id}.json")
        })
    );

    let decision_payload: Value = serde_json::from_str(
        &fs::read_to_string(
            run_dir
                .join("runtime_effect_decisions")
                .join(format!("{request_id}.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(decision_payload["handler_id"], "planner_disposition");
    assert_eq!(
        decision_payload["rule_id"],
        "planner_disposition_on_complete"
    );
    assert_eq!(decision_payload["terminal_result"], "PLANNER_COMPLETE");

    let result_payload: Value = serde_json::from_str(
        &fs::read_to_string(
            run_dir
                .join("runtime_effect_results")
                .join(format!("{request_id}.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(result_payload["handler_id"], "planner_disposition");
    assert_eq!(result_payload["decision"], "request_complete_source");
    assert_eq!(result_payload["mutation_phase"], "pre_mutation");
    assert_eq!(
        result_payload["source_lifecycle_intent"]["action"],
        "complete"
    );
}

#[test]
fn planner_missing_disposition_blocks_source_with_runtime_effect_origin() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    let root = spec_document("spec-effect-missing");
    queue.enqueue_spec(&root).unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("runtime-effects-missing")).unwrap();
    let runner = DeletePlannerDispositionRunner {
        inner: FakeRunner::with_default(FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"))
            .unwrap(),
    };
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(
            "run-runtime-effects-missing",
            "request-runtime-effects-missing",
        ),
        &runner,
    )
    .unwrap();

    assert_eq!(
        outcome.router_decision.as_ref().unwrap().action,
        RouterAction::Blocked
    );
    assert_eq!(
        outcome
            .router_decision
            .as_ref()
            .unwrap()
            .failure_class
            .as_deref(),
        Some("planner_disposition_missing")
    );
    assert!(
        paths
            .specs_blocked_dir
            .join("spec-effect-missing.md")
            .is_file()
    );
    assert_eq!(load_planning_status(&paths).unwrap(), "### BLOCKED");

    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(stage_result.metadata["failure_origin"], "runtime_effect");
    assert_eq!(
        stage_result.metadata["runtime_effect_failure_class"],
        "planner_disposition_missing"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_decision"],
        "request_block_source"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_source_lifecycle_action"],
        "block"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_mutation_phase"],
        "pre_mutation"
    );
}

#[test]
fn planner_disposition_source_family_or_id_mismatch_blocks_conservatively() {
    for (case, family_id, source_id) in [
        ("family", "incident", "spec-effect-mismatch-family"),
        ("id", "spec", "spec-effect-unexpected"),
    ] {
        let temp = TempDir::new().unwrap();
        let paths = initialize_workspace(temp.path().join(format!("workspace-{case}"))).unwrap();
        let queue = QueueStore::from_paths(paths.clone());
        let root = spec_document(&format!("spec-effect-mismatch-{case}"));
        queue.enqueue_spec(&root).unwrap();

        let mut session = startup_runtime_once_for_paths(
            &paths,
            startup_options(&format!("runtime-effects-mismatch-{case}")),
        )
        .unwrap();
        let run_id = format!("run-runtime-effects-mismatch-{case}");
        let request_id = format!("request-runtime-effects-mismatch-{case}");
        let run_dir = paths.runs_dir.join(&run_id);
        let mut disposition =
            planner_disposition(&root.spec_id, "active_source_ready_for_manager", &[]);
        disposition["source_work_item_family_id"] = json!(family_id);
        disposition["source_work_item_id"] = json!(source_id);
        write_planner_disposition(&run_dir, &disposition);
        let runner =
            FakeRunner::with_default(FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"))
                .unwrap();

        let outcome = run_serial_runtime_tick_with_runner(
            &mut session,
            tick_options(&run_id, &request_id),
            &runner,
        )
        .unwrap();

        let decision = outcome.router_decision.as_ref().unwrap();
        assert_eq!(decision.action, RouterAction::Blocked);
        assert_eq!(
            decision.failure_class.as_deref(),
            Some("planner_disposition_source_mismatch")
        );
        assert_eq!(decision.next_node_id, None);
        assert!(
            paths
                .specs_blocked_dir
                .join(format!("{}.md", root.spec_id))
                .is_file()
        );
        assert!(
            !paths
                .specs_done_dir
                .join(format!("{}.md", root.spec_id))
                .exists()
        );

        let stage_result = outcome.stage_result.as_ref().unwrap();
        assert_eq!(
            stage_result.metadata["runtime_effect_failure_class"],
            "planner_disposition_source_mismatch"
        );
        assert_eq!(
            stage_result.metadata["runtime_effect_decision"],
            "request_block_source"
        );
        assert_eq!(
            stage_result.metadata["runtime_effect_source_lifecycle_action"],
            "block"
        );
        assert!(
            stage_result.metadata["runtime_effect_failure_message"]
                .as_str()
                .unwrap()
                .contains("mismatch")
        );
    }
}

#[test]
fn planner_disposition_terminal_mismatch_blocks_without_manager_dispatch() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    let root = spec_document("spec-effect-terminal-mismatch");
    queue.enqueue_spec(&root).unwrap();

    let mut session = startup_runtime_once_for_paths(
        &paths,
        startup_options("runtime-effects-terminal-mismatch"),
    )
    .unwrap();
    let run_id = "run-runtime-effects-terminal-mismatch";
    let request_id = "request-runtime-effects-terminal-mismatch";
    let run_dir = paths.runs_dir.join(run_id);
    write_planner_disposition(
        &run_dir,
        &planner_disposition(&root.spec_id, "active_source_ready_for_manager", &[]),
    );
    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### BLOCKED")).unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(run_id, request_id),
        &runner,
    )
    .unwrap();

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Blocked);
    assert_eq!(
        decision.failure_class.as_deref(),
        Some("planner_disposition_terminal_mismatch")
    );
    assert_eq!(decision.next_node_id, None);
    assert!(
        paths
            .specs_blocked_dir
            .join("spec-effect-terminal-mismatch.md")
            .is_file()
    );
    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(
        stage_result.metadata["runtime_effect_failure_class"],
        "planner_disposition_terminal_mismatch"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_decision"],
        "request_block_source"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_source_lifecycle_action"],
        "block"
    );
    assert!(
        stage_result.metadata["runtime_effect_failure_message"]
            .as_str()
            .unwrap()
            .contains("requires PLANNER_COMPLETE")
    );
}

#[test]
fn runtime_effect_dispatch_rejects_unknown_handler_bindings() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_spec(&spec_document("spec-effect-unknown-handler"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("runtime-effects-unknown")).unwrap();
    add_runtime_effect_rule_clone(
        &mut session,
        "planner_disposition_unknown_handler",
        "unknown_runtime_effect_handler",
    );
    *planner_effect_rule_selections(&mut session) =
        vec!["planner_disposition_unknown_handler".to_owned()];
    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"))
            .unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(
            "run-runtime-effects-unknown",
            "request-runtime-effects-unknown",
        ),
        &runner,
    )
    .unwrap();
    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::RunStage);
    assert_eq!(
        decision.failure_class.as_deref(),
        Some("planning_post_stage_apply_failed")
    );
    let context_payload: Value = serde_json::from_str(
        &fs::read_to_string(outcome.runtime_error_context_path.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        context_payload["error_code"],
        "planning_post_stage_apply_failed"
    );
    let message = context_payload["exception_message"].as_str().unwrap();
    assert!(
        message.contains(
            "runtime effect rule references unknown handler unknown_runtime_effect_handler"
        ),
        "unexpected runtime-effect error: {message}"
    );
    let _ = session.finish();
}

#[test]
fn runtime_effect_dispatch_rejects_duplicate_matching_rule_bindings() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_spec(&spec_document("spec-effect-duplicate-rule"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("runtime-effects-duplicate"))
            .unwrap();
    add_runtime_effect_rule_clone(
        &mut session,
        "planner_disposition_duplicate",
        "planner_disposition",
    );
    planner_effect_rule_selections(&mut session).push("planner_disposition_duplicate".to_owned());
    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"))
            .unwrap();

    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(
            "run-runtime-effects-duplicate",
            "request-runtime-effects-duplicate",
        ),
        &runner,
    )
    .unwrap();
    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::RunStage);
    assert_eq!(
        decision.failure_class.as_deref(),
        Some("planning_post_stage_apply_failed")
    );
    let context_payload: Value = serde_json::from_str(
        &fs::read_to_string(outcome.runtime_error_context_path.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        context_payload["error_code"],
        "planning_post_stage_apply_failed"
    );
    let message = context_payload["exception_message"].as_str().unwrap();
    assert!(
        message.contains("multiple runtime effect rules matched planner/PLANNER_COMPLETE"),
        "unexpected runtime-effect error: {message}"
    );
    assert!(message.contains("planner_disposition_on_complete"));
    assert!(message.contains("planner_disposition_duplicate"));
    let _ = session.finish();
}
