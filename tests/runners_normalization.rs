use std::{
    collections::VecDeque,
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use serde_json::json;
use tempfile::TempDir;

use millrace_ai::contracts::{Plane, StageName, Timestamp, WorkItemKind};
use millrace_ai::{
    CodexCliConfig, CodexCliRunnerAdapter, CodexProcessError, CodexProcessExecutor,
    CodexProcessRequest, FakeRunner, FakeRunnerResult, PiRpcClientCreateRequest, PiRpcClientError,
    PiRpcClientFactory, PiRpcConfig, PiRpcPromptClient, PiRpcRunnerAdapter, PiRpcSessionResult,
    ProcessExecutionResult, ProcessExitKind, RequestKind, RunnerExitKind, RunnerRawResult,
    StageRunRequest, StageRunnerAdapter, normalize_stage_result,
};

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
