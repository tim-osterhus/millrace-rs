mod support;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process,
};

use serde_json::{Value, json};
use tempfile::TempDir;

use millrace_ai::contracts::{RuntimeMode, SpecDocument, SpecSourceType, TaskDocument, Timestamp};
use millrace_ai::workspace::{QueueStore, RuntimeOwnershipLockOptions, initialize_workspace};
use millrace_ai::{
    FakeRunner, FakeRunnerResult, RuntimeStartupOptions, RuntimeTickOptions,
    RuntimeTickOutcomeKind, run_serial_runtime_tick_with_runner, startup_runtime_once_for_paths,
};
use support::parity::run_rust_millrace;

const NOW: &str = "2026-05-21T07:10:00Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("timestamp", value).unwrap()
}

fn lock_options(session_id: &str) -> RuntimeOwnershipLockOptions {
    RuntimeOwnershipLockOptions::new(process::id(), "test-host", session_id, NOW).unwrap()
}

fn startup_options(session_id: &str) -> RuntimeStartupOptions {
    RuntimeStartupOptions {
        requested_mode_id: Some("learning_codex".to_owned()),
        runtime_mode: RuntimeMode::Once,
        lock_options: Some(lock_options(session_id)),
        now: Some(timestamp(NOW)),
        ..RuntimeStartupOptions::default()
    }
}

fn tick_options(run_id: &str, request_id: &str) -> RuntimeTickOptions {
    RuntimeTickOptions {
        now: Some(timestamp("2026-05-21T07:11:00Z")),
        run_id: Some(run_id.to_owned()),
        request_id: Some(request_id.to_owned()),
    }
}

fn task_document(task_id: &str) -> TaskDocument {
    TaskDocument {
        task_id: task_id.to_owned(),
        title: format!("Task {task_id}"),
        summary: "run inspection test".to_owned(),
        root_idea_id: Some("idea-inspection".to_owned()),
        root_spec_id: Some("spec-inspection".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        spec_id: Some("spec-inspection".to_owned()),
        parent_task_id: None,
        incident_id: None,
        target_paths: vec!["src/cli/read_only.rs".to_owned()],
        acceptance: vec!["run inspection exposes context evidence".to_owned()],
        required_checks: vec!["cargo test --test runtime_run_inspection".to_owned()],
        references: vec!["src/cli/read_only.rs".to_owned()],
        risk: vec!["inspection must remain read-only".to_owned()],
        depends_on: Vec::new(),
        blocks: Vec::new(),
        tags: vec!["run-inspection".to_owned()],
        status_hint: None,
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn spec_document(spec_id: &str) -> SpecDocument {
    SpecDocument {
        spec_id: spec_id.to_owned(),
        title: format!("Spec {spec_id}"),
        summary: "runtime-effect run inspection test".to_owned(),
        source_type: SpecSourceType::Idea,
        source_id: Some("idea-inspection".to_owned()),
        parent_spec_id: None,
        root_idea_id: Some("idea-inspection".to_owned()),
        root_spec_id: Some(spec_id.to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        goals: vec!["plan runtime-effect inspection work".to_owned()],
        non_goals: Vec::new(),
        scope: vec!["runtime effects".to_owned()],
        constraints: vec!["run inspection must remain read-only".to_owned()],
        assumptions: Vec::new(),
        risks: vec!["inspection evidence can drift".to_owned()],
        target_paths: vec!["src/runtime/".to_owned()],
        entrypoints: Vec::new(),
        required_skills: Vec::new(),
        decomposition_hints: Vec::new(),
        acceptance: vec!["run inspection exposes runtime-effect evidence".to_owned()],
        references: vec!["src/runtime/effects.rs".to_owned()],
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn planner_disposition(source_spec_id: &str, emitted_spec_id: &str) -> Value {
    json!({
        "schema_version": "1.0",
        "kind": "planner_disposition",
        "source_work_item_family_id": "spec",
        "source_work_item_id": source_spec_id,
        "disposition": "emitted_child_specs",
        "emitted_spec_ids": [emitted_spec_id],
        "refined_active_source": true,
        "recommended_next_action": "continue_to_manager",
        "created_at": "2026-05-21T07:11:00Z",
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

fn runtime_tree_snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let runtime_root = root.join("millrace-agents");
    let mut files = BTreeMap::new();
    if runtime_root.exists() {
        collect_file_snapshot(&runtime_root, &runtime_root, &mut files);
    }
    files
}

fn collect_file_snapshot(root: &Path, directory: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_file_snapshot(root, &path, files);
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(relative, fs::read(&path).unwrap());
        }
    }
}

#[test]
fn status_and_run_views_expose_lane_context_and_runtime_outcome_without_mutation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("workspace");
    let paths = initialize_workspace(&root).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-inspection"))
        .unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("inspection")).unwrap();
    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"))
            .unwrap();
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-inspection", "request-inspection"),
        &runner,
    )
    .unwrap();
    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::StageDispatched);
    let request = outcome.stage_request.as_ref().unwrap();
    assert_eq!(request.lane_id.as_deref(), Some("execution.main"));
    assert_eq!(
        request
            .context_bundle_path
            .as_deref()
            .map(Path::new)
            .unwrap(),
        paths
            .runs_dir
            .join("run-inspection/context/request_context.json")
    );
    session.finish().unwrap();

    let before_views = runtime_tree_snapshot(&root);
    let status = run_rust_millrace(["status", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace status");
    status.assert_success();
    assert!(status.stdout.contains("pending_plan: none\n"));
    assert!(
        status
            .stdout
            .contains("lane_state: lane=execution.main plane=execution status=active")
    );
    assert!(status.stdout.contains("context_bundle_path: "));
    assert!(status.stdout.contains("request_context.json\n"));
    assert!(status.stdout.contains("latest_launch_plan_id: "));
    assert!(status.stdout.contains("latest_visible_context_refs: "));
    assert!(status.stdout.contains("task:task-inspection"));
    assert!(
        status
            .stdout
            .contains("latest_artifact_parse_status: valid\n")
    );
    assert!(status.stdout.contains("latest_runtime_outcome: complete\n"));

    let status_json = run_rust_millrace([
        "status",
        "--workspace",
        root.to_str().unwrap(),
        "--format",
        "json",
    ])
    .expect("run Rust millrace status JSON");
    status_json.assert_success();
    let payload: Value = serde_json::from_str(&status_json.stdout).unwrap();
    assert_eq!(payload["pending_plan"], Value::Null);
    assert_eq!(payload["lane_state"]["execution.main"]["status"], "active");
    assert!(
        payload["context_bundle_path"]
            .as_str()
            .unwrap()
            .ends_with("context/request_context.json")
    );
    assert!(
        payload["latest_launch_plan_id"]
            .as_str()
            .unwrap()
            .starts_with("plan-")
    );
    assert!(
        payload["latest_visible_context_refs"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("task:task-inspection"))
    );
    assert_eq!(payload["latest_artifact_parse_status"], "valid");
    assert_eq!(payload["latest_runtime_outcome"], "complete");

    let list = run_rust_millrace(["runs", "ls", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace runs ls");
    list.assert_success();
    assert!(list.stdout.contains("run_id: run-inspection\n"));
    assert!(list.stdout.contains("runtime_outcome: complete\n"));
    assert!(list.stdout.contains("launch_plan_id: "));
    assert!(list.stdout.contains("lane_id: execution.main\n"));
    assert!(
        list.stdout
            .contains("context_bundle_path: context/request_context.json\n")
    );
    assert!(list.stdout.contains("visible_context_refs: "));
    assert!(list.stdout.contains("task:task-inspection"));
    assert!(list.stdout.contains("failure_origin: none\n"));
    assert!(list.stdout.contains("artifact_parse_status: valid\n"));

    let show = run_rust_millrace([
        "runs",
        "show",
        "run-inspection",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs show");
    show.assert_success();
    for expected in [
        "runtime_outcome: complete\n",
        "launch_plan_id: ",
        "lane_id: execution.main\n",
        "artifact_parse_status: valid\n",
        "context_bundle_path: context/request_context.json\n",
        "stage_request_context_bundle_path: context/request_context.json\n",
        "stage_request_visible_context_ref: task:task-inspection\n",
        "visible_context_ref: task:task-inspection\n",
    ] {
        assert!(
            show.stdout.contains(expected),
            "missing expected runs show fragment: {expected}\nstdout:\n{}",
            show.stdout
        );
    }

    let tail = run_rust_millrace([
        "runs",
        "tail",
        "run-inspection",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs tail");
    tail.assert_success();
    assert!(
        tail.stdout
            .contains("fake runner output\n### BUILDER_COMPLETE\n")
    );
    assert_eq!(runtime_tree_snapshot(&root), before_views);
}

#[test]
fn status_and_run_views_expose_runtime_effect_evidence_without_mutation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("workspace");
    let paths = initialize_workspace(&root).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    let source = spec_document("spec-inspection-effect-root");
    let mut child = spec_document("spec-inspection-effect-child");
    child.parent_spec_id = Some(source.spec_id.clone());
    child.root_spec_id = Some(source.spec_id.clone());
    child.created_at = timestamp("2026-05-21T07:10:30Z");
    queue.enqueue_spec(&source).unwrap();
    queue.enqueue_spec(&child).unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("inspection-effect")).unwrap();
    let run_id = "run-inspection-effect";
    let request_id = "request-inspection-effect";
    write_planner_disposition(
        &paths.runs_dir.join(run_id),
        &planner_disposition(&source.spec_id, &child.spec_id),
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
    session.finish().unwrap();

    let before_views = runtime_tree_snapshot(&root);
    let status = run_rust_millrace(["status", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace status");
    status.assert_success();
    for expected in [
        "latest_artifact_parse_status: valid\n",
        "latest_runtime_outcome: complete\n",
        "latest_runtime_effect_handler_id: planner_disposition\n",
        "latest_runtime_effect_decision: request_complete_source\n",
        "latest_runtime_effect_mutation_phase: pre_mutation\n",
        "latest_runtime_effect_failure_class: none\n",
        "latest_runtime_effect_failure_policy_id: none\n",
        "latest_runtime_effect_source_lifecycle_plan_id: complete_source_after_effect\n",
        "latest_runtime_effect_source_lifecycle_action: complete\n",
        "latest_runtime_effect_created_paths: millrace-agents/specs/queue/spec-inspection-effect-child.md\n",
    ] {
        assert!(
            status.stdout.contains(expected),
            "missing expected status fragment: {expected}\nstdout:\n{}",
            status.stdout
        );
    }
    assert!(!status.stdout.contains("latest_failure_origin: "));

    let status_json = run_rust_millrace([
        "status",
        "--workspace",
        root.to_str().unwrap(),
        "--format",
        "json",
    ])
    .expect("run Rust millrace status JSON");
    status_json.assert_success();
    let payload: Value = serde_json::from_str(&status_json.stdout).unwrap();
    assert_eq!(payload["latest_failure_origin"], Value::Null);
    assert_eq!(payload["latest_artifact_parse_status"], "valid");
    assert_eq!(payload["latest_runtime_outcome"], "complete");
    assert_eq!(
        payload["latest_runtime_effect_handler_id"],
        "planner_disposition"
    );
    assert_eq!(
        payload["latest_runtime_effect_decision"],
        "request_complete_source"
    );
    assert_eq!(
        payload["latest_runtime_effect_mutation_phase"],
        "pre_mutation"
    );
    assert_eq!(payload["latest_runtime_effect_failure_class"], Value::Null);
    assert_eq!(
        payload["latest_runtime_effect_failure_policy_id"],
        Value::Null
    );
    assert_eq!(
        payload["latest_runtime_effect_source_lifecycle_plan_id"],
        "complete_source_after_effect"
    );
    assert_eq!(
        payload["latest_runtime_effect_source_lifecycle_action"],
        "complete"
    );
    assert_eq!(
        payload["latest_runtime_effect_created_paths"],
        json!(["millrace-agents/specs/queue/spec-inspection-effect-child.md"])
    );

    let list = run_rust_millrace(["runs", "ls", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace runs ls");
    list.assert_success();
    for expected in [
        "run_id: run-inspection-effect\n",
        "runtime_outcome: complete\n",
        "failure_origin: none\n",
        "artifact_parse_status: valid\n",
        "runtime_effect_handler_id: planner_disposition\n",
        "runtime_effect_decision: request_complete_source\n",
        "runtime_effect_failure_class: none\n",
    ] {
        assert!(
            list.stdout.contains(expected),
            "missing expected runs ls fragment: {expected}\nstdout:\n{}",
            list.stdout
        );
    }

    let show = run_rust_millrace([
        "runs",
        "show",
        "run-inspection-effect",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs show");
    show.assert_success();
    for expected in [
        "runtime_outcome: complete\n",
        "failure_origin: none\n",
        "artifact_parse_status: valid\n",
        "runtime_effect_handler_id: planner_disposition\n",
        "runtime_effect_decision: request_complete_source\n",
        "runtime_effect_mutation_phase: pre_mutation\n",
        "runtime_effect_failure_class: none\n",
        "runtime_effect_failure_policy_id: none\n",
        "runtime_effect_recovery_action: none\n",
        "runtime_effect_source_lifecycle_plan_id: complete_source_after_effect\n",
        "runtime_effect_source_lifecycle_action: complete\n",
        "runtime_effect_created_paths: millrace-agents/specs/queue/spec-inspection-effect-child.md\n",
        "runtime_effect_created_path: millrace-agents/specs/queue/spec-inspection-effect-child.md\n",
    ] {
        assert!(
            show.stdout.contains(expected),
            "missing expected runs show fragment: {expected}\nstdout:\n{}",
            show.stdout
        );
    }
    for expected in [
        format!("artifact_path: runtime_effect_decisions/{request_id}.json\n"),
        format!("artifact_path: runtime_effect_results/{request_id}.json\n"),
    ] {
        assert!(
            show.stdout.contains(&expected),
            "missing expected runs show fragment: {expected}\nstdout:\n{}",
            show.stdout
        );
    }

    assert_eq!(runtime_tree_snapshot(&root), before_views);
}
