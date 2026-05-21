use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use tempfile::TempDir;

use millrace_ai::contracts::{Plane, StageName, Timestamp, WorkItemKind};
use millrace_ai::runtime::attach_default_request_context;
use millrace_ai::{
    FakeRunner, FakeRunnerResult, RequestKind, RunnerEnvironmentDelta, RunnerExitKind,
    RunnerRawResult, StageRunRequest, StageRunnerAdapter, build_stage_prompt,
    invocation_artifact_from_request, normalize_stage_result,
};

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("timestamp", value).unwrap()
}

fn sample_request(root: &Path, request_id: &str) -> StageRunRequest {
    let run_dir = root
        .join("millrace-agents")
        .join("runs")
        .join("run-context");
    fs::create_dir_all(&run_dir).unwrap();
    let active_work_item_path = root
        .join("millrace-agents")
        .join("tasks")
        .join("active")
        .join("task-context.md");
    let mut request = StageRunRequest {
        request_id: request_id.to_owned(),
        run_id: "run-context".to_owned(),
        plane: Plane::Execution,
        stage: StageName::Builder,
        request_kind: RequestKind::ActiveWorkItem,
        mode_id: "learning_codex_auto_port".to_owned(),
        compiled_plan_id: "plan-compiled".to_owned(),
        launch_plan_id: Some("plan-launch".to_owned()),
        lane_id: Some("execution.main".to_owned()),
        node_id: "builder".to_owned(),
        stage_kind_id: "builder".to_owned(),
        running_status_marker: "BUILDER_RUNNING".to_owned(),
        legal_terminal_markers: Vec::new(),
        allowed_result_classes_by_outcome: Default::default(),
        entrypoint_path: root
            .join("millrace-agents/entrypoints/execution/builder.md")
            .display()
            .to_string(),
        entrypoint_contract_id: Some("builder.contract.v1".to_owned()),
        required_skill_paths: vec![
            root.join("millrace-agents/skills/stage/execution/builder-core/SKILL.md")
                .display()
                .to_string(),
        ],
        attached_skill_paths: Vec::new(),
        active_work_item_family_id: Some("task".to_owned()),
        active_work_item_kind: Some(WorkItemKind::Task),
        active_work_item_id: Some("task-context".to_owned()),
        active_work_item_path: Some(active_work_item_path.display().to_string()),
        closure_target_path: None,
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        canonical_root_spec_path: None,
        canonical_seed_idea_path: None,
        preferred_rubric_path: None,
        preferred_verdict_path: None,
        preferred_report_path: None,
        run_dir: run_dir.display().to_string(),
        summary_status_path: root
            .join("millrace-agents/state/execution_status.md")
            .display()
            .to_string(),
        runtime_snapshot_path: root
            .join("millrace-agents/state/runtime_snapshot.json")
            .display()
            .to_string(),
        recovery_counters_path: root
            .join("millrace-agents/state/recovery_counters.json")
            .display()
            .to_string(),
        preferred_troubleshoot_report_path: Some(
            run_dir.join("troubleshoot_report.md").display().to_string(),
        ),
        runtime_error_code: None,
        runtime_error_report_path: None,
        runtime_error_catalog_path: None,
        skill_revision_evidence_path: Some(
            run_dir
                .join("skill_revision_evidence.json")
                .display()
                .to_string(),
        ),
        runner_name: Some("fake_runner".to_owned()),
        model_name: Some("fake-model".to_owned()),
        thinking_level: Some("medium".to_owned()),
        model_reasoning_effort: Some("medium".to_owned()),
        timeout_seconds: 300,
        execution_capability_grants: Vec::new(),
        capability_support_decisions: Vec::new(),
        request_context_profile_id: None,
        context_bundle_path: None,
        context_artifact_refs: Vec::new(),
        context_render_plan_id: None,
        rendered_prompt_context_path: None,
    };
    request.validate().unwrap();
    request
}

fn raw_result(request: &StageRunRequest, stdout_path: &Path) -> RunnerRawResult {
    fs::write(stdout_path, "fake output\n### BUILDER_COMPLETE\n").unwrap();
    RunnerRawResult {
        request_id: request.request_id.clone(),
        run_id: request.run_id.clone(),
        stage: request.stage,
        runner_name: "fake_runner".to_owned(),
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
        failure_capability_class: None,
        capability_support_decisions: Vec::new(),
        capability_evidence_refs: Vec::new(),
        missing_capability_evidence_refs: Vec::new(),
        started_at: timestamp("2026-05-21T07:05:00Z"),
        ended_at: timestamp("2026-05-21T07:05:01Z"),
    }
}

#[test]
fn request_context_render_writes_bundle_prompt_and_runner_evidence() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("workspace");
    let request = sample_request(&root, "request-context");
    let request = attach_default_request_context(&root, request, None).unwrap();

    assert_eq!(
        request.request_context_profile_id.as_deref(),
        Some("builder.default")
    );
    assert_eq!(
        request.context_render_plan_id.as_deref(),
        Some("builder.context.v1")
    );
    assert!(
        request
            .context_artifact_refs
            .contains(&"task:task-context".to_owned())
    );

    let bundle_path = PathBuf::from(request.context_bundle_path.as_ref().unwrap());
    let rendered_path = PathBuf::from(request.rendered_prompt_context_path.as_ref().unwrap());
    let manifest_path = rendered_path.with_file_name("render_manifest.json");
    assert!(bundle_path.is_file());
    assert!(rendered_path.is_file());
    assert!(manifest_path.is_file());

    let bundle: Value = serde_json::from_str(&fs::read_to_string(&bundle_path).unwrap()).unwrap();
    assert_eq!(bundle["kind"], "request_context_bundle");
    assert_eq!(bundle["profile_id"], "builder.default");
    assert_eq!(
        bundle["visible_context_refs"],
        json!([
            "task:task-context",
            format!(
                "active_work_item_path:{}",
                request.active_work_item_path.as_deref().unwrap()
            ),
            format!(
                "skill_revision_evidence:{}",
                request.skill_revision_evidence_path.as_deref().unwrap()
            )
        ])
    );

    let prompt = build_stage_prompt(&request);
    assert!(prompt.contains("Rendered Request Context:"));
    assert!(prompt.contains("Render Plan ID: builder.context.v1"));
    assert!(prompt.contains("Visible Context References:"));

    let invocation = invocation_artifact_from_request(
        &request,
        "fake_runner",
        vec!["fake_runner".to_owned(), "run-stage".to_owned()],
        request.run_dir.clone(),
        RunnerEnvironmentDelta::default(),
        rendered_path.display().to_string(),
        timestamp("2026-05-21T07:05:00Z"),
    )
    .unwrap();
    assert_eq!(
        invocation.context_bundle_path.as_deref(),
        request.context_bundle_path.as_deref()
    );
    assert_eq!(invocation.lane_id.as_deref(), Some("execution.main"));
    assert_eq!(invocation.launch_plan_id.as_deref(), Some("plan-launch"));

    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"))
            .unwrap();
    let raw = runner.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw).unwrap();
    assert_eq!(envelope.metadata["artifact_parse_status"], "valid");
    assert_eq!(envelope.metadata["lane_id"], json!("execution.main"));
    assert_eq!(envelope.metadata["launch_plan_id"], json!("plan-launch"));
    assert_eq!(
        envelope.metadata["visible_context_refs"],
        json!(request.context_artifact_refs)
    );
    assert!(
        envelope
            .artifact_paths
            .iter()
            .any(|path| path == request.context_bundle_path.as_ref().unwrap())
    );
    assert!(
        envelope
            .artifact_paths
            .iter()
            .any(|path| path == request.rendered_prompt_context_path.as_ref().unwrap())
    );

    let completion_path =
        Path::new(&request.run_dir).join(format!("runner_completion.{}.json", request.request_id));
    let completion: Value =
        serde_json::from_str(&fs::read_to_string(completion_path).unwrap()).unwrap();
    assert_eq!(
        completion["context_bundle_path"],
        json!(request.context_bundle_path)
    );
    assert_eq!(completion["context_render_plan_id"], "builder.context.v1");
}

#[test]
fn runner_normalization_keeps_context_parse_status_separate_from_runtime_outcome() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("workspace");
    let stdout_path = root.join("stdout.txt");
    let request = sample_request(&root, "request-missing-context");
    let raw = raw_result(&request, &stdout_path);
    let missing = normalize_stage_result(&request, &raw).unwrap();
    assert_eq!(
        missing.metadata["artifact_parse_status"],
        "missing_context_bundle"
    );
    assert!(missing.success);

    let mut malformed_request = request.clone();
    let malformed_bundle = root.join("malformed-request-context.json");
    fs::write(&malformed_bundle, "{not-json").unwrap();
    malformed_request.context_bundle_path = Some(malformed_bundle.display().to_string());
    malformed_request.rendered_prompt_context_path =
        Some(root.join("prompt_context.md").display().to_string());
    let malformed = normalize_stage_result(&malformed_request, &raw).unwrap();
    assert_eq!(
        malformed.metadata["artifact_parse_status"],
        "malformed_context_bundle"
    );
    assert!(malformed.success);

    let mut missing_prompt_request = request.clone();
    let valid_bundle = root.join("valid-request-context.json");
    fs::write(&valid_bundle, "{}\n").unwrap();
    missing_prompt_request.context_bundle_path = Some(valid_bundle.display().to_string());
    missing_prompt_request.rendered_prompt_context_path =
        Some(root.join("missing-prompt-context.md").display().to_string());
    let missing_prompt = normalize_stage_result(&missing_prompt_request, &raw).unwrap();
    assert_eq!(
        missing_prompt.metadata["artifact_parse_status"],
        "missing_prompt_context"
    );
    assert!(missing_prompt.success);
}
