mod support;

use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use serde_json::{Value, json};
use tempfile::TempDir;

use millrace_ai::contracts::{Plane, ResultClass, StageName, Timestamp, WorkItemKind};
use millrace_ai::{
    CodexCliConfig, CodexCliRunnerAdapter, CodexProcessError, CodexProcessExecutor,
    CodexProcessRequest, FakeRunner, FakeRunnerResult, PiRpcClientCreateRequest, PiRpcClientError,
    PiRpcClientFactory, PiRpcConfig, PiRpcPromptClient, PiRpcRunnerAdapter, PiRpcSessionResult,
    ProcessExecutionResult, ProcessExitKind, RequestKind, RunnerExitKind, RunnerRawResult,
    StageRunRequest, StageRunnerAdapter, normalize_stage_result,
};
use support::parity::read_json_fixture;

const RUN_ID: &str = "run-normalization";
const STARTED_AT: &str = "2026-05-12T00:00:00Z";
const ENDED_AT: &str = "2026-05-12T00:00:01Z";

struct FnExecutor<F>(F);

impl<F> CodexProcessExecutor for FnExecutor<F>
where
    F: Fn(&CodexProcessRequest) -> Result<ProcessExecutionResult, CodexProcessError>,
{
    fn execute(
        &self,
        request: &CodexProcessRequest,
    ) -> Result<ProcessExecutionResult, CodexProcessError> {
        (self.0)(request)
    }
}

#[derive(Clone)]
struct MockPiFactory {
    results: Arc<Mutex<VecDeque<Result<PiRpcSessionResult, PiRpcClientError>>>>,
}

impl MockPiFactory {
    fn new(results: Vec<Result<PiRpcSessionResult, PiRpcClientError>>) -> Self {
        Self {
            results: Arc::new(Mutex::new(results.into())),
        }
    }
}

impl PiRpcClientFactory for MockPiFactory {
    fn create(
        &self,
        _request: PiRpcClientCreateRequest,
    ) -> Result<Box<dyn PiRpcPromptClient>, PiRpcClientError> {
        Ok(Box::new(MockPiClient {
            result: self.results.lock().unwrap().pop_front().unwrap(),
        }))
    }
}

struct MockPiClient {
    result: Result<PiRpcSessionResult, PiRpcClientError>,
}

impl PiRpcPromptClient for MockPiClient {
    fn run_prompt(
        &mut self,
        _prompt: &str,
        _timeout_seconds: u64,
    ) -> Result<PiRpcSessionResult, PiRpcClientError> {
        self.result.clone()
    }
}

fn sample_request(root: &Path, request_id: &str, runner_name: &str) -> StageRunRequest {
    let run_dir = root.join("runs").join(RUN_ID);
    fs::create_dir_all(&run_dir).unwrap();
    let entrypoint_path = root.join("builder.md");
    fs::write(&entrypoint_path, "# Builder\n").unwrap();
    let active_work_item_path = root
        .join("millrace-agents")
        .join("tasks")
        .join("active")
        .join("task-source.md");

    let mut request = StageRunRequest {
        request_id: request_id.to_owned(),
        run_id: RUN_ID.to_owned(),
        plane: Plane::Execution,
        stage: StageName::Builder,
        request_kind: RequestKind::ActiveWorkItem,
        mode_id: "learning_codex_auto_port".to_owned(),
        compiled_plan_id: "plan-normalization".to_owned(),
        launch_plan_id: None,
        lane_id: None,
        node_id: String::new(),
        stage_kind_id: String::new(),
        running_status_marker: String::new(),
        legal_terminal_markers: Vec::new(),
        allowed_result_classes_by_outcome: Default::default(),
        entrypoint_path: entrypoint_path.display().to_string(),
        entrypoint_contract_id: Some("builder.contract.v1".to_owned()),
        required_skill_paths: Vec::new(),
        attached_skill_paths: Vec::new(),
        active_work_item_family_id: None,
        active_work_item_kind: Some(WorkItemKind::Task),
        active_work_item_id: Some("task-source".to_owned()),
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
        model_name: None,
        thinking_level: None,
        model_reasoning_effort: None,
        timeout_seconds: 120,
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

fn timestamp(raw: &str) -> Timestamp {
    Timestamp::parse("timestamp", raw).unwrap()
}

fn completed_process_result(request: &CodexProcessRequest) -> ProcessExecutionResult {
    let mut result = ProcessExecutionResult::new(
        request.command.clone(),
        request.cwd.display().to_string(),
        request.environment_delta.clone(),
        ProcessExitKind::Completed,
        Some(0),
        timestamp(STARTED_AT),
        timestamp(ENDED_AT),
    )
    .unwrap();
    result.stdout_path = Some(request.stdout_path.display().to_string());
    result.stderr_path = Some(request.stderr_path.display().to_string());
    result
}

fn command_option_value<'a>(command: &'a [String], flag: &str) -> &'a str {
    let index = command.iter().position(|value| value == flag).unwrap();
    &command[index + 1]
}

fn completed_pi_result() -> PiRpcSessionResult {
    PiRpcSessionResult {
        exit_kind: RunnerExitKind::Completed,
        exit_code: Some(0),
        started_at: timestamp(STARTED_AT),
        ended_at: timestamp(ENDED_AT),
        event_lines: Vec::new(),
        assistant_text: Some("### BUILDER_COMPLETE\n".to_owned()),
        token_usage: None,
        failure_class: None,
        notes: Vec::new(),
        stderr_text: String::new(),
        observed_exit_kind: None,
        observed_exit_code: None,
    }
}

fn assert_active_work_item_metadata(
    envelope: &millrace_ai::contracts::StageResultEnvelope,
    request: &StageRunRequest,
) {
    assert_eq!(envelope.work_item_kind, WorkItemKind::Task);
    assert_eq!(envelope.work_item_id, "task-source");
    assert_eq!(envelope.metadata["active_work_item_kind"], json!("task"));
    assert_eq!(
        envelope.metadata["active_work_item_id"],
        json!("task-source")
    );
    assert_eq!(
        envelope.metadata["active_work_item_path"],
        json!(request.active_work_item_path.as_deref())
    );
}

fn raw_result_for_failure(
    request: &StageRunRequest,
    exit_kind: RunnerExitKind,
    exit_code: Option<i32>,
    stdout_path: Option<&Path>,
    stderr_path: Option<&Path>,
) -> RunnerRawResult {
    RunnerRawResult {
        request_id: request.request_id.clone(),
        run_id: request.run_id.clone(),
        stage: request.stage,
        runner_name: request
            .runner_name
            .clone()
            .unwrap_or_else(|| "raw".to_owned()),
        model_name: None,
        thinking_level: None,
        model_reasoning_effort: None,
        exit_kind,
        exit_code,
        observed_exit_kind: None,
        observed_exit_code: None,
        stdout_path: stdout_path.map(|path| path.display().to_string()),
        stderr_path: stderr_path.map(|path| path.display().to_string()),
        terminal_result_path: None,
        event_log_path: None,
        token_usage: None,
        failure_capability_class: None,
        capability_support_decisions: Vec::new(),
        capability_evidence_refs: Vec::new(),
        missing_capability_evidence_refs: Vec::new(),
        started_at: timestamp(STARTED_AT),
        ended_at: timestamp(ENDED_AT),
    }
}

fn assert_failure_metadata(
    envelope: &millrace_ai::contracts::StageResultEnvelope,
    failure_class: &str,
    blocked_origin: &str,
    failure_scope: &str,
    auto_requeue_candidate: bool,
    failure_classifier_code: &str,
) {
    assert_eq!(envelope.terminal_result.marker(), "### BLOCKED");
    assert_eq!(envelope.result_class, ResultClass::RecoverableFailure);
    assert!(!envelope.success);
    assert_eq!(envelope.metadata["failure_class"], json!(failure_class));
    assert_eq!(envelope.metadata["blocked_origin"], json!(blocked_origin));
    assert_eq!(envelope.metadata["failure_scope"], json!(failure_scope));
    assert_eq!(
        envelope.metadata["auto_requeue_candidate"],
        json!(auto_requeue_candidate)
    );
    assert_eq!(
        envelope.metadata["failure_classifier_code"],
        json!(failure_classifier_code)
    );
    assert_eq!(envelope.metadata["valid_terminal_result"], json!(false));
}

#[test]
fn runner_normalization_v0_18_4_guardrail_fixture_requires_failure_classifier_metadata() {
    let fixture = read_json_fixture("runtime_json/auto_port_v0_18_4_runtime_contract_scout.json");
    assert_eq!(fixture["kind"], "auto_port_v0_18_4_runtime_contract_scout");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.4");
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.4");

    let classifier = &fixture["failure_classifier_metadata"];
    let retryable: BTreeSet<_> = classifier["retryable_failure_classes"]
        .as_array()
        .expect("retryable failure classes are present")
        .iter()
        .map(|value| value.as_str().expect("retryable class"))
        .collect();
    assert_eq!(
        retryable,
        BTreeSet::from([
            "network_unavailable",
            "provider_unavailable",
            "provider_rate_limited",
            "runner_timeout",
        ])
    );

    let non_auto: BTreeSet<_> = classifier["non_auto_requeue_failure_classes"]
        .as_array()
        .expect("non-auto failure classes are present")
        .iter()
        .map(|value| value.as_str().expect("non-auto class"))
        .collect();
    for failure_class in [
        "runner_binary_missing",
        "auth_missing_or_invalid",
        "missing_terminal_result",
        "illegal_terminal_result",
        "conflicting_terminal_results",
        "missing_required_artifact",
        "runner_transport_failure",
    ] {
        assert!(
            non_auto.contains(failure_class),
            "missing v0.18.4 non-auto runner failure class {failure_class}"
        );
    }

    let metadata_keys: BTreeSet<_> = classifier["metadata_keys"]
        .as_array()
        .expect("metadata keys are present")
        .iter()
        .map(|value| value.as_str().expect("metadata key"))
        .collect();
    for key in [
        "failure_class",
        "blocked_origin",
        "failure_scope",
        "auto_requeue_candidate",
        "failure_classifier_code",
        "valid_terminal_result",
        "raw_exit_kind",
        "raw_exit_code",
        "timeout_reconciled",
        "active_work_item_kind",
        "active_work_item_id",
        "active_work_item_path",
    ] {
        assert!(
            metadata_keys.contains(key),
            "missing v0.18.4 runner metadata key {key}"
        );
    }

    let classifier_codes: BTreeSet<_> = classifier["classifier_codes"]
        .as_array()
        .expect("classifier codes are present")
        .iter()
        .map(|value| value.as_str().expect("classifier code"))
        .collect();
    for code in [
        "exit_timeout",
        "runner_binary_missing",
        "auth_missing_or_invalid",
        "provider_rate_limited",
        "network_unavailable",
        "provider_unavailable",
        "provider_default_unavailable",
        "runner_timeout",
        "unclassified_failure",
    ] {
        assert!(
            classifier_codes.contains(code),
            "missing v0.18.4 classifier code {code}"
        );
    }
}

#[test]
fn runner_normalization_v0_19_0_guardrail_fixture_requires_capability_support_and_evidence_metadata()
 {
    let fixture = read_json_fixture("runtime_json/auto_port_v0_19_0_runtime_contract_scout.json");
    assert_eq!(fixture["kind"], "auto_port_v0_19_0_runtime_contract_scout");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.19.0");

    let sources: BTreeSet<_> = fixture["contract_sources"]
        .as_array()
        .expect("contract source references are present")
        .iter()
        .map(|value| value.as_str().expect("contract source"))
        .collect();
    for source in [
        "../millrace-py/src/millrace_ai/runners/base.py",
        "../millrace-py/src/millrace_ai/runners/contracts.py",
        "../millrace-py/src/millrace_ai/runners/normalization.py",
        "../millrace-py/src/millrace_ai/runners/requests.py",
        "../millrace-py/src/millrace_ai/runners/adapters/codex_cli.py",
        "../millrace-py/src/millrace_ai/runners/adapters/pi_rpc.py",
        "../millrace-py/tests/runners/test_capability_support.py",
    ] {
        assert!(
            sources.contains(source),
            "missing v0.19.0 runner capability source {source}"
        );
    }

    let runner = &fixture["runner_support_contract"];
    assert_eq!(
        runner["support_states"],
        json!(["supported", "unsupported", "partially_supported"])
    );
    assert_eq!(
        runner["normalization_failure_class"],
        "capability_evidence_missing"
    );
    for field in [
        "execution_capability_grants",
        "capability_support_decisions",
        "capability_evidence_refs",
        "missing_capability_evidence_refs",
        "failure_capability_class",
    ] {
        assert!(
            runner["artifact_fields"]
                .as_array()
                .expect("runner artifact fields are present")
                .iter()
                .any(|value| value.as_str() == Some(field)),
            "missing v0.19.0 runner capability artifact field {field}"
        );
    }
    assert_eq!(runner["codex_advisory_permission"], "maximum");
    assert!(
        runner["pi_rpc_support_note"]
            .as_str()
            .expect("Pi RPC support note")
            .contains("advisory support"),
        "missing v0.19.0 Pi RPC advisory support note"
    );
}

#[test]
fn runner_failure_metadata_classifies_retryable_transient_failures() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path(), "request-transient", "raw");
    let run_dir = Path::new(&request.run_dir);

    let timeout_stdout = run_dir.join("timeout_stdout.txt");
    fs::write(&timeout_stdout, "### BUILDER_COMPLETE\n").unwrap();
    let timeout = normalize_stage_result(
        &request,
        &raw_result_for_failure(
            &request,
            RunnerExitKind::Timeout,
            Some(124),
            Some(&timeout_stdout),
            None,
        ),
    )
    .unwrap();
    assert_failure_metadata(
        &timeout,
        "runner_timeout",
        "runner_failure",
        "environment",
        true,
        "exit_timeout",
    );
    assert_eq!(timeout.metadata["raw_exit_kind"], json!("timeout"));
    assert_eq!(timeout.metadata["raw_exit_code"], json!(124));

    let provider_default = normalize_stage_result(
        &request,
        &raw_result_for_failure(&request, RunnerExitKind::ProviderError, Some(1), None, None),
    )
    .unwrap();
    assert_failure_metadata(
        &provider_default,
        "provider_unavailable",
        "runner_failure",
        "provider",
        true,
        "provider_default_unavailable",
    );

    let rate_limit_stderr = run_dir.join("rate_limit_stderr.txt");
    fs::write(
        &rate_limit_stderr,
        "provider returned 429 too many requests\n",
    )
    .unwrap();
    let rate_limited = normalize_stage_result(
        &request,
        &raw_result_for_failure(
            &request,
            RunnerExitKind::ProviderError,
            Some(1),
            None,
            Some(&rate_limit_stderr),
        ),
    )
    .unwrap();
    assert_failure_metadata(
        &rate_limited,
        "provider_rate_limited",
        "runner_failure",
        "provider",
        true,
        "provider_rate_limited",
    );

    let network_stderr = run_dir.join("network_stderr.txt");
    fs::write(
        &network_stderr,
        "failed to reach provider: could not resolve host api.openai.com\n",
    )
    .unwrap();
    let network = normalize_stage_result(
        &request,
        &raw_result_for_failure(
            &request,
            RunnerExitKind::RunnerError,
            Some(1),
            None,
            Some(&network_stderr),
        ),
    )
    .unwrap();
    assert_failure_metadata(
        &network,
        "network_unavailable",
        "runner_failure",
        "environment",
        true,
        "network_unavailable",
    );
}

#[test]
fn runner_failure_metadata_classifies_non_auto_retryable_failures() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path(), "request-non-auto", "raw");
    let run_dir = Path::new(&request.run_dir);

    let binary_stderr = run_dir.join("binary_stderr.txt");
    fs::write(&binary_stderr, "runner binary not found: codex\n").unwrap();
    let binary = normalize_stage_result(
        &request,
        &raw_result_for_failure(
            &request,
            RunnerExitKind::RunnerError,
            Some(127),
            None,
            Some(&binary_stderr),
        ),
    )
    .unwrap();
    assert_failure_metadata(
        &binary,
        "runner_binary_missing",
        "runner_failure",
        "local_configuration",
        false,
        "runner_binary_missing",
    );

    let auth_stderr = run_dir.join("auth_stderr.txt");
    fs::write(&auth_stderr, "401 unauthorized: invalid api key\n").unwrap();
    let auth = normalize_stage_result(
        &request,
        &raw_result_for_failure(
            &request,
            RunnerExitKind::RunnerError,
            Some(1),
            None,
            Some(&auth_stderr),
        ),
    )
    .unwrap();
    assert_failure_metadata(
        &auth,
        "auth_missing_or_invalid",
        "runner_failure",
        "local_configuration",
        false,
        "auth_missing_or_invalid",
    );

    let illegal_stdout = run_dir.join("illegal_stdout.txt");
    fs::write(&illegal_stdout, "### CHECKER_PASS\n").unwrap();
    let illegal_terminal = normalize_stage_result(
        &request,
        &raw_result_for_failure(
            &request,
            RunnerExitKind::Completed,
            Some(0),
            Some(&illegal_stdout),
            None,
        ),
    )
    .unwrap();
    assert_failure_metadata(
        &illegal_terminal,
        "illegal_terminal_result",
        "stage_terminal",
        "contract",
        false,
        "illegal_terminal_result",
    );
    assert_eq!(
        illegal_terminal.metadata["raw_detected_marker"],
        json!("### CHECKER_PASS")
    );

    let unknown_stderr = run_dir.join("unknown_stderr.txt");
    fs::write(&unknown_stderr, "unexpected runner transport failure\n").unwrap();
    let unknown = normalize_stage_result(
        &request,
        &raw_result_for_failure(
            &request,
            RunnerExitKind::RunnerError,
            Some(1),
            None,
            Some(&unknown_stderr),
        ),
    )
    .unwrap();
    assert_failure_metadata(
        &unknown,
        "runner_transport_failure",
        "runner_failure",
        "unknown",
        false,
        "unclassified_failure",
    );
}

#[test]
fn stage_authored_blocked_terminal_remains_valid_and_not_auto_requeued() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path(), "request-semantic-blocked", "raw");
    let stdout_path = Path::new(&request.run_dir).join("blocked_stdout.txt");
    fs::write(&stdout_path, "### BLOCKED\n").unwrap();

    let envelope = normalize_stage_result(
        &request,
        &raw_result_for_failure(
            &request,
            RunnerExitKind::Completed,
            Some(0),
            Some(&stdout_path),
            None,
        ),
    )
    .unwrap();

    assert_eq!(envelope.terminal_result.marker(), "### BLOCKED");
    assert_eq!(envelope.result_class, ResultClass::Blocked);
    assert!(!envelope.retryable);
    assert_eq!(envelope.metadata["failure_class"], Value::Null);
    assert_eq!(envelope.metadata["valid_terminal_result"], json!(true));
    assert!(envelope.metadata.get("auto_requeue_candidate").is_none());
}

#[test]
fn raw_result_normalization_preserves_active_work_item_source_metadata() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path(), "request-raw", "raw");
    let stdout_path = Path::new(&request.run_dir).join("runner_stdout.raw.txt");
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
        failure_capability_class: None,
        capability_support_decisions: Vec::new(),
        capability_evidence_refs: Vec::new(),
        missing_capability_evidence_refs: Vec::new(),
        started_at: timestamp(STARTED_AT),
        ended_at: timestamp(ENDED_AT),
    };

    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_active_work_item_metadata(&envelope, &request);
}

#[test]
fn fake_runner_normalization_preserves_active_work_item_source_metadata() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path(), "request-fake", "fake_runner");
    let runner =
        FakeRunner::with_default(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"))
            .unwrap();

    let raw_result = runner.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_active_work_item_metadata(&envelope, &request);
}

#[test]
fn codex_runner_normalization_preserves_active_work_item_source_metadata() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path(), "request-codex", "codex_cli");
    let executor = FnExecutor(|process_request: &CodexProcessRequest| {
        fs::write(&process_request.stdout_path, "").unwrap();
        fs::write(&process_request.stderr_path, "").unwrap();
        fs::write(
            command_option_value(&process_request.command, "--output-last-message"),
            "### BUILDER_COMPLETE\n",
        )
        .unwrap();
        Ok(completed_process_result(process_request))
    });
    let adapter = CodexCliRunnerAdapter::with_process_executor(
        CodexCliConfig::default(),
        temp.path(),
        executor,
    );

    let raw_result = adapter.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_active_work_item_metadata(&envelope, &request);
}

#[test]
fn pi_runner_normalization_preserves_active_work_item_source_metadata() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path(), "request-pi", "pi_rpc");
    let adapter = PiRpcRunnerAdapter::with_client_factory(
        PiRpcConfig::default(),
        temp.path(),
        MockPiFactory::new(vec![Ok(completed_pi_result())]),
    );

    let raw_result = adapter.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &raw_result).unwrap();

    assert_active_work_item_metadata(&envelope, &request);
}
