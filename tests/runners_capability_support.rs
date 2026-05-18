use std::{fs, path::Path};

use serde_json::{Value, json};
use tempfile::TempDir;

use millrace_ai::contracts::{
    CapabilityDecisionState, CapabilityEnforcementMode, CapabilityEvidenceStatus, CapabilityScope,
    CapabilitySupportState, ExecutionCapabilityGrant, Plane, ResultClass, StageName, Timestamp,
    WorkItemKind,
};
use millrace_ai::{
    CodexCliConfig, CodexCliRunnerAdapter, CodexPermissionLevel, FakeRunner, FakeRunnerResult,
    PiRpcConfig, PiRpcRunnerAdapter, RequestKind, RunnerExitKind, RunnerRawResult, RunnerRegistry,
    StageRunRequest, StageRunnerAdapter, StageRunnerDispatcher, build_stage_prompt,
    normalize_stage_result,
};

const STARTED_AT: &str = "2026-05-12T00:00:00Z";
const ENDED_AT: &str = "2026-05-12T00:00:01Z";

fn grant(
    capability_id: &str,
    enforcement_mode: CapabilityEnforcementMode,
    evidence_requirements: Vec<&str>,
) -> ExecutionCapabilityGrant {
    ExecutionCapabilityGrant {
        grant_id: format!("grant-{}", capability_id.replace('.', "-")),
        request_id: format!("request-{}", capability_id.replace('.', "-")),
        capability_id: capability_id.to_owned(),
        access: "execute".to_owned(),
        scope: CapabilityScope {
            kind: if capability_id == "runner.invoke" {
                "runner".to_owned()
            } else {
                "workspace".to_owned()
            },
            value: "workspace".to_owned(),
            metadata: Default::default(),
        },
        required: true,
        decision_state: CapabilityDecisionState::Granted,
        enforcement_mode,
        approval_policy_ref: None,
        evidence_requirements: evidence_requirements
            .into_iter()
            .map(str::to_owned)
            .collect(),
        evidence_status: CapabilityEvidenceStatus::Pending,
        decision_reason: "test grant".to_owned(),
        resolved_by: "test".to_owned(),
        fingerprint: String::new(),
    }
}

fn request(root: &Path, runner_name: &str) -> StageRunRequest {
    let run_dir = root.join("runs").join("run-capability");
    fs::create_dir_all(&run_dir).unwrap();
    let entrypoint_path = root.join("entrypoint.md");
    fs::write(&entrypoint_path, "# Entrypoint\n").unwrap();
    let mut request = StageRunRequest {
        request_id: "req-capability".to_owned(),
        run_id: "run-capability".to_owned(),
        plane: Plane::Execution,
        stage: StageName::Builder,
        request_kind: RequestKind::ActiveWorkItem,
        mode_id: "learning_codex_auto_port".to_owned(),
        compiled_plan_id: "plan-capability".to_owned(),
        node_id: String::new(),
        stage_kind_id: String::new(),
        running_status_marker: String::new(),
        legal_terminal_markers: Vec::new(),
        allowed_result_classes_by_outcome: Default::default(),
        entrypoint_path: entrypoint_path.display().to_string(),
        entrypoint_contract_id: Some("builder.contract.v1".to_owned()),
        required_skill_paths: Vec::new(),
        attached_skill_paths: Vec::new(),
        active_work_item_kind: Some(WorkItemKind::Task),
        active_work_item_id: Some("task-capability".to_owned()),
        active_work_item_path: Some(root.join("task-capability.md").display().to_string()),
        closure_target_path: None,
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        canonical_root_spec_path: None,
        canonical_seed_idea_path: None,
        preferred_rubric_path: None,
        preferred_verdict_path: None,
        preferred_report_path: None,
        run_dir: run_dir.display().to_string(),
        summary_status_path: root.join("execution_status.md").display().to_string(),
        runtime_snapshot_path: root.join("runtime_snapshot.json").display().to_string(),
        recovery_counters_path: root.join("recovery_counters.json").display().to_string(),
        preferred_troubleshoot_report_path: Some(
            run_dir.join("troubleshoot_report.md").display().to_string(),
        ),
        runtime_error_code: None,
        runtime_error_report_path: None,
        runtime_error_catalog_path: None,
        skill_revision_evidence_path: None,
        runner_name: Some(runner_name.to_owned()),
        model_name: Some("gpt-5".to_owned()),
        thinking_level: None,
        model_reasoning_effort: None,
        timeout_seconds: 120,
        execution_capability_grants: vec![
            grant(
                "runner.invoke",
                CapabilityEnforcementMode::RuntimeEnforced,
                vec!["runner_invocation", "runner_completion"],
            ),
            grant(
                "workspace.write",
                CapabilityEnforcementMode::AdvisoryOnly,
                vec!["runner_invocation", "runner_completion"],
            ),
        ],
        capability_support_decisions: Vec::new(),
    };
    request.validate().unwrap();
    request
}

fn timestamp(raw: &str) -> Timestamp {
    Timestamp::parse("timestamp", raw).unwrap()
}

#[test]
fn request_prompt_renders_grants_and_runner_support_decisions() {
    let temp = TempDir::new().unwrap();
    let adapter = CodexCliRunnerAdapter::new(CodexCliConfig::default(), temp.path());
    let request = adapter.request_with_capability_support(&request(temp.path(), "codex_cli"));

    let prompt = build_stage_prompt(&request);

    assert!(prompt.contains("Execution Capability Grants:"));
    assert!(prompt.contains(
        "- grant-runner-invoke runner.invoke decision=granted enforcement=runtime_enforced"
    ));
    assert!(prompt.contains("Capability Support Decisions:"));
    assert!(prompt.contains(
        "- grant-workspace-write support=supported enforcement=advisory_only runner=codex_cli"
    ));
}

#[test]
fn fake_runner_artifacts_raw_result_and_normalization_carry_capability_evidence() {
    let temp = TempDir::new().unwrap();
    let request = request(temp.path(), "fake_runner");
    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"))
            .unwrap();

    let raw_result = runner.run(&request).unwrap();
    let run_dir = Path::new(&request.run_dir);
    let invocation: Value = serde_json::from_str(
        &fs::read_to_string(run_dir.join("runner_invocation.req-capability.json")).unwrap(),
    )
    .unwrap();
    let completion: Value = serde_json::from_str(
        &fs::read_to_string(run_dir.join("runner_completion.req-capability.json")).unwrap(),
    )
    .unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_eq!(
        invocation["execution_capability_grants"][0]["grant_id"],
        "grant-runner-invoke"
    );
    assert_eq!(
        invocation["capability_support_decisions"][0]["support_state"],
        "supported"
    );
    assert_eq!(
        completion["capability_evidence_refs"],
        json!([
            "runner_invocation:req-capability",
            "runner_completion:req-capability"
        ])
    );
    assert_eq!(raw_result.capability_evidence_refs.len(), 2);
    assert_eq!(
        envelope.metadata["execution_capability_grants"][1]["capability_id"],
        "workspace.write"
    );
    assert_eq!(
        envelope.metadata["capability_support_decisions"][1]["runner_id"],
        "fake_runner"
    );
}

#[test]
fn codex_and_pi_support_reporting_stays_honest_about_advisory_boundaries() {
    let temp = TempDir::new().unwrap();
    let request = request(temp.path(), "codex_cli");
    let codex = CodexCliRunnerAdapter::new(CodexCliConfig::default(), temp.path());
    let workspace_grant = &request.execution_capability_grants[1];

    let maximum = codex.evaluate_capability_grant(workspace_grant, &request);
    assert_eq!(maximum.support_state, CapabilitySupportState::Supported);
    assert_eq!(
        maximum.enforcement_mode,
        CapabilityEnforcementMode::AdvisoryOnly
    );
    assert!(maximum.reason.contains("maximum"));

    let basic_config = CodexCliConfig {
        permission_default: CodexPermissionLevel::Basic,
        ..Default::default()
    };
    let basic = CodexCliRunnerAdapter::new(basic_config, temp.path())
        .evaluate_capability_grant(&request.execution_capability_grants[0], &request);
    assert_eq!(basic.support_state, CapabilitySupportState::Supported);
    assert_eq!(
        basic.enforcement_mode,
        CapabilityEnforcementMode::RuntimeEnforced
    );

    let pi = PiRpcRunnerAdapter::new(PiRpcConfig::default(), temp.path())
        .evaluate_capability_grant(workspace_grant, &request);
    assert_eq!(pi.support_state, CapabilitySupportState::PartiallySupported);
    assert_eq!(pi.enforcement_mode, CapabilityEnforcementMode::AdvisoryOnly);
    assert!(pi.reason.contains("advisory"));
}

#[test]
fn dispatcher_evaluates_capability_support_through_resolved_adapter() {
    let temp = TempDir::new().unwrap();
    let request = request(temp.path(), "codex_cli");
    let mut registry = RunnerRegistry::new();
    registry
        .register(
            "codex_cli",
            CodexCliRunnerAdapter::new(
                CodexCliConfig {
                    permission_default: CodexPermissionLevel::Basic,
                    ..Default::default()
                },
                temp.path(),
            ),
        )
        .unwrap();
    let dispatcher = StageRunnerDispatcher::new(registry);

    let decision =
        dispatcher.evaluate_capability_grant(&request.execution_capability_grants[0], &request);

    assert_eq!(decision.runner_id, "codex_cli");
    assert_eq!(decision.support_state, CapabilitySupportState::Supported);
    assert_eq!(
        decision.enforcement_mode,
        CapabilityEnforcementMode::RuntimeEnforced
    );
    assert!(decision.reason.contains("Codex CLI"));
}

#[test]
fn dispatcher_unknown_runner_support_decision_stays_conservative() {
    let temp = TempDir::new().unwrap();
    let request = request(temp.path(), "missing_runner");
    let dispatcher = StageRunnerDispatcher::new(RunnerRegistry::new());

    let decision =
        dispatcher.evaluate_capability_grant(&request.execution_capability_grants[1], &request);

    assert_eq!(decision.runner_id, "missing_runner");
    assert_eq!(
        decision.support_state,
        CapabilitySupportState::PartiallySupported
    );
    assert_eq!(
        decision.enforcement_mode,
        CapabilityEnforcementMode::AdvisoryOnly
    );
    assert!(decision.evidence_available.is_empty());
    assert!(decision.reason.contains("unknown runner"));
}

#[test]
fn missing_required_capability_evidence_normalizes_to_runtime_policy_failure() {
    let temp = TempDir::new().unwrap();
    let request = request(temp.path(), "raw");
    let stdout_path = Path::new(&request.run_dir).join("runner_stdout.txt");
    fs::write(&stdout_path, "### BUILDER_COMPLETE\n").unwrap();
    let raw_result = RunnerRawResult {
        request_id: request.request_id.clone(),
        run_id: request.run_id.clone(),
        stage: request.stage,
        runner_name: "raw".to_owned(),
        model_name: None,
        thinking_level: None,
        model_reasoning_effort: None,
        exit_kind: RunnerExitKind::Completed,
        exit_code: Some(0),
        observed_exit_kind: None,
        observed_exit_code: None,
        stdout_path: Some(stdout_path.display().to_string()),
        stderr_path: None,
        terminal_result_path: None,
        event_log_path: None,
        token_usage: None,
        failure_capability_class: Some("capability_evidence_missing".to_owned()),
        capability_support_decisions: request.capability_support_decisions.clone(),
        capability_evidence_refs: vec!["runner_invocation:req-capability".to_owned()],
        missing_capability_evidence_refs: vec!["grant-workspace-write".to_owned()],
        started_at: timestamp(STARTED_AT),
        ended_at: timestamp(ENDED_AT),
    };

    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_eq!(envelope.result_class, ResultClass::RecoverableFailure);
    assert_eq!(
        envelope.metadata["failure_class"],
        "capability_evidence_missing"
    );
    assert_eq!(envelope.metadata["failure_scope"], "runtime_policy");
    assert_eq!(
        envelope.metadata["missing_capability_evidence_refs"],
        json!(["grant-workspace-write"])
    );
}
