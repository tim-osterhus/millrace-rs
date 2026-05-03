use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::Value;
use tempfile::TempDir;

use millrace_ai::contracts::{
    Plane, ResultClass, StageName, TerminalResult, Timestamp, TokenUsage, WorkItemKind,
};
use millrace_ai::{
    PiEventLogPolicy, PiRpcClientCreateRequest, PiRpcClientError, PiRpcClientFactory, PiRpcConfig,
    PiRpcJsonlClient, PiRpcPromptClient, PiRpcRunnerAdapter, PiRpcSessionResult, PiRpcStreamEvent,
    PiRpcTransport, RequestKind, RunnerExitKind, StageRunRequest, StageRunnerAdapter,
    build_pi_rpc_command, normalize_stage_result, persistable_event_lines,
};

const RUN_ID: &str = "run-001";
const REQUEST_ID: &str = "req-001";
const STARTED_AT: &str = "2026-04-29T00:00:00Z";
const ENDED_AT: &str = "2026-04-29T00:00:01Z";

#[derive(Clone)]
struct MockClientFactory {
    results: Arc<Mutex<VecDeque<Result<PiRpcSessionResult, PiRpcClientError>>>>,
    create_requests: Arc<Mutex<Vec<PiRpcClientCreateRequest>>>,
    prompts: Arc<Mutex<Vec<(String, u64)>>>,
}

impl MockClientFactory {
    fn new(results: Vec<Result<PiRpcSessionResult, PiRpcClientError>>) -> Self {
        Self {
            results: Arc::new(Mutex::new(results.into())),
            create_requests: Arc::new(Mutex::new(Vec::new())),
            prompts: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl PiRpcClientFactory for MockClientFactory {
    fn create(
        &self,
        request: PiRpcClientCreateRequest,
    ) -> Result<Box<dyn PiRpcPromptClient>, PiRpcClientError> {
        self.create_requests.lock().unwrap().push(request);
        Ok(Box::new(MockPromptClient {
            result: self.results.lock().unwrap().pop_front().unwrap(),
            prompts: Arc::clone(&self.prompts),
        }))
    }
}

struct MockPromptClient {
    result: Result<PiRpcSessionResult, PiRpcClientError>,
    prompts: Arc<Mutex<Vec<(String, u64)>>>,
}

impl PiRpcPromptClient for MockPromptClient {
    fn run_prompt(
        &mut self,
        prompt: &str,
        timeout_seconds: u64,
    ) -> Result<PiRpcSessionResult, PiRpcClientError> {
        self.prompts
            .lock()
            .unwrap()
            .push((prompt.to_owned(), timeout_seconds));
        self.result.clone()
    }
}

#[derive(Debug, Default)]
struct ScriptedTransport {
    sent: Vec<Value>,
    events: VecDeque<PiRpcStreamEvent>,
    stderr: String,
    exit_code: Option<i32>,
    closed_stdin: bool,
    terminate_count: usize,
    kill_count: usize,
}

impl ScriptedTransport {
    fn with_events(events: Vec<PiRpcStreamEvent>) -> Self {
        Self {
            events: events.into(),
            exit_code: Some(0),
            ..Self::default()
        }
    }
}

impl PiRpcTransport for ScriptedTransport {
    fn send_json(&mut self, payload: &Value) -> Result<(), PiRpcClientError> {
        self.sent.push(payload.clone());
        Ok(())
    }

    fn read_stdout_event(
        &mut self,
        _timeout: Duration,
    ) -> Result<PiRpcStreamEvent, PiRpcClientError> {
        Ok(self.events.pop_front().unwrap_or(PiRpcStreamEvent::Timeout))
    }

    fn stderr_text(&self) -> String {
        self.stderr.clone()
    }

    fn poll_exit_code(&mut self) -> Result<Option<i32>, PiRpcClientError> {
        Ok(self.exit_code)
    }

    fn close_stdin(&mut self) {
        self.closed_stdin = true;
    }

    fn terminate(&mut self) -> Result<(), PiRpcClientError> {
        self.terminate_count += 1;
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PiRpcClientError> {
        self.kill_count += 1;
        self.exit_code = Some(-9);
        Ok(())
    }

    fn wait(&mut self, _timeout: Duration) -> Result<Option<i32>, PiRpcClientError> {
        Ok(self.exit_code)
    }
}

fn sample_request(root: &Path) -> StageRunRequest {
    let run_dir = root.join("runs").join(RUN_ID);
    fs::create_dir_all(&run_dir).unwrap();
    let entrypoint_path = root.join("entrypoint.md");
    fs::write(&entrypoint_path, "# Builder\n").unwrap();
    let mut request = StageRunRequest {
        request_id: REQUEST_ID.to_owned(),
        run_id: RUN_ID.to_owned(),
        plane: Plane::Execution,
        stage: StageName::Builder,
        request_kind: RequestKind::ActiveWorkItem,
        mode_id: "default_pi".to_owned(),
        compiled_plan_id: "plan-001".to_owned(),
        node_id: String::new(),
        stage_kind_id: String::new(),
        running_status_marker: String::new(),
        legal_terminal_markers: Vec::new(),
        allowed_result_classes_by_outcome: Default::default(),
        entrypoint_path: entrypoint_path.display().to_string(),
        entrypoint_contract_id: Some("builder.contract.v1".to_owned()),
        required_skill_paths: vec![
            "millrace-agents/skills/stage/execution/builder-core/SKILL.md".to_owned(),
        ],
        attached_skill_paths: Vec::new(),
        active_work_item_kind: Some(WorkItemKind::Task),
        active_work_item_id: Some("task-001".to_owned()),
        active_work_item_path: Some(root.join("task-001.md").display().to_string()),
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
        skill_revision_evidence_path: Some(
            run_dir.join("skill_revision.json").display().to_string(),
        ),
        runner_name: Some("pi_rpc".to_owned()),
        model_name: Some("openai/gpt-5.4".to_owned()),
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

fn completed_session_result(event_lines: Vec<String>) -> PiRpcSessionResult {
    PiRpcSessionResult {
        exit_kind: RunnerExitKind::Completed,
        exit_code: Some(0),
        started_at: timestamp(STARTED_AT),
        ended_at: timestamp(ENDED_AT),
        event_lines,
        assistant_text: Some("\n### BUILDER_COMPLETE\n\n".to_owned()),
        token_usage: Some(TokenUsage {
            input_tokens: 120,
            cached_input_tokens: 20,
            output_tokens: 14,
            thinking_tokens: 0,
            total_tokens: 134,
        }),
        failure_class: None,
        notes: Vec::new(),
        stderr_text: String::new(),
        observed_exit_kind: None,
        observed_exit_code: None,
    }
}

#[test]
fn pi_adapter_writes_prompt_artifacts_stdout_tokens_and_default_command() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path());
    let factory = MockClientFactory::new(vec![Ok(completed_session_result(vec![
        r#"{"type":"agent_start"}"#.to_owned(),
        r#"{"type":"agent_end"}"#.to_owned(),
    ]))]);
    let adapter = PiRpcRunnerAdapter::with_client_factory(
        PiRpcConfig::default(),
        temp.path(),
        factory.clone(),
    );

    let result = adapter.run(&request).unwrap();

    assert_eq!(result.exit_kind, RunnerExitKind::Completed);
    assert_eq!(result.runner_name, "pi_rpc");
    assert_eq!(
        fs::read_to_string(result.stdout_path.as_ref().unwrap()).unwrap(),
        "\n### BUILDER_COMPLETE\n\n"
    );
    assert_eq!(result.event_log_path, None);
    assert!(
        !Path::new(&request.run_dir)
            .join("runner_events.req-001.jsonl")
            .exists()
    );
    assert_eq!(
        result.token_usage,
        Some(TokenUsage {
            input_tokens: 120,
            cached_input_tokens: 20,
            output_tokens: 14,
            thinking_tokens: 0,
            total_tokens: 134,
        })
    );

    let create_requests = factory.create_requests.lock().unwrap();
    assert_eq!(
        create_requests[0].command,
        vec![
            "pi",
            "--mode",
            "rpc",
            "--no-session",
            "--model",
            "openai/gpt-5.4",
            "--no-context-files",
            "--no-skills",
        ]
    );
    assert_eq!(factory.prompts.lock().unwrap()[0].1, 120);
    assert!(
        factory.prompts.lock().unwrap()[0]
            .0
            .contains("Stage Request Context:")
    );

    let run_dir = Path::new(&request.run_dir);
    let invocation: Value = serde_json::from_str(
        &fs::read_to_string(run_dir.join("runner_invocation.req-001.json")).unwrap(),
    )
    .unwrap();
    let completion: Value = serde_json::from_str(
        &fs::read_to_string(run_dir.join("runner_completion.req-001.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(invocation["runner_name"], "pi_rpc");
    assert_eq!(completion["runner_name"], "pi_rpc");
    assert_eq!(completion["token_usage"]["total_tokens"], 134);
}

#[test]
fn pi_adapter_prefers_request_thinking_level_over_global_default() {
    let temp = TempDir::new().unwrap();
    let mut request = sample_request(temp.path());
    request.thinking_level = Some("high".to_owned());
    request.validate().unwrap();
    let factory = MockClientFactory::new(vec![Ok(completed_session_result(Vec::new()))]);
    let adapter = PiRpcRunnerAdapter::with_client_factory(
        PiRpcConfig {
            thinking: Some("medium".to_owned()),
            ..PiRpcConfig::default()
        },
        temp.path(),
        factory.clone(),
    );

    let result = adapter.run(&request).unwrap();

    assert_eq!(result.thinking_level.as_deref(), Some("high"));
    let create_requests = factory.create_requests.lock().unwrap();
    let thinking_index = create_requests[0]
        .command
        .iter()
        .position(|value| value == "--thinking")
        .unwrap();
    assert_eq!(create_requests[0].command[thinking_index + 1], "high");

    let run_dir = Path::new(&request.run_dir);
    let invocation: Value = serde_json::from_str(
        &fs::read_to_string(run_dir.join("runner_invocation.req-001.json")).unwrap(),
    )
    .unwrap();
    let completion: Value = serde_json::from_str(
        &fs::read_to_string(run_dir.join("runner_completion.req-001.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(invocation["thinking_level"], "high");
    assert_eq!(completion["thinking_level"], "high");
}

#[test]
fn pi_command_preserves_provider_model_thinking_defaults_env_and_rejects_reserved_args() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path());
    let mut env = BTreeMap::new();
    env.insert("PI_TEST_ENV".to_owned(), "1".to_owned());
    let config = PiRpcConfig {
        command: "pi-dev".to_owned(),
        args: vec!["--profile".to_owned(), "work".to_owned()],
        provider: Some("openai".to_owned()),
        thinking: Some("high".to_owned()),
        env,
        ..PiRpcConfig::default()
    };
    let factory = MockClientFactory::new(vec![Ok(completed_session_result(Vec::new()))]);
    let adapter =
        PiRpcRunnerAdapter::with_client_factory(config.clone(), temp.path(), factory.clone());

    adapter.run(&request).unwrap();

    let create_requests = factory.create_requests.lock().unwrap();
    assert_eq!(
        create_requests[0].command,
        vec![
            "pi-dev",
            "--profile",
            "work",
            "--mode",
            "rpc",
            "--no-session",
            "--provider",
            "openai",
            "--model",
            "openai/gpt-5.4",
            "--thinking",
            "high",
            "--no-context-files",
            "--no-skills",
        ]
    );
    assert_eq!(
        create_requests[0].environment_delta.set.get("PI_TEST_ENV"),
        Some(&"1".to_owned())
    );

    let rejected = PiRpcConfig {
        args: vec!["--model=other".to_owned(), "--no-skills".to_owned()],
        ..PiRpcConfig::default()
    };
    let error = build_pi_rpc_command(&rejected, &request).unwrap_err();
    assert!(error.to_string().contains("reserved pi runner flags"));
}

#[test]
fn pi_event_log_policy_filters_message_updates_for_success_and_failure() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path());
    let factory = MockClientFactory::new(vec![Ok(completed_session_result(vec![
        r#"{"type":"agent_start"}"#.to_owned(),
        r#"{"type":"message_update"}"#.to_owned(),
        r#"{"type":"agent_end"}"#.to_owned(),
    ]))]);
    let adapter = PiRpcRunnerAdapter::with_client_factory(
        PiRpcConfig {
            event_log_policy: PiEventLogPolicy::Full,
            ..PiRpcConfig::default()
        },
        temp.path(),
        factory,
    );

    let result = adapter.run(&request).unwrap();

    assert_eq!(
        fs::read_to_string(result.event_log_path.as_ref().unwrap())
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![r#"{"type":"agent_start"}"#, r#"{"type":"agent_end"}"#]
    );

    let request = sample_request(temp.path());
    let failed = PiRpcSessionResult {
        exit_kind: RunnerExitKind::Timeout,
        exit_code: Some(124),
        started_at: timestamp(STARTED_AT),
        ended_at: timestamp(ENDED_AT),
        event_lines: vec![
            r#"{"type":"agent_start"}"#.to_owned(),
            r#"{"type":"message_update"}"#.to_owned(),
            r#"{"type":"agent_end","stopReason":"error"}"#.to_owned(),
        ],
        assistant_text: None,
        token_usage: None,
        failure_class: Some("runner_timeout".to_owned()),
        notes: vec!["runner process exceeded timeout".to_owned()],
        stderr_text: "timed out".to_owned(),
        observed_exit_kind: None,
        observed_exit_code: None,
    };
    let factory = MockClientFactory::new(vec![Ok(failed)]);
    let adapter =
        PiRpcRunnerAdapter::with_client_factory(PiRpcConfig::default(), temp.path(), factory);

    let result = adapter.run(&request).unwrap();

    assert_eq!(result.exit_kind, RunnerExitKind::Timeout);
    assert_eq!(
        fs::read_to_string(result.event_log_path.as_ref().unwrap())
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![
            r#"{"type":"agent_start"}"#,
            r#"{"type":"agent_end","stopReason":"error"}"#
        ]
    );
    assert!(persistable_event_lines(&[r#"{"type":"message_update"}"#.to_owned()]).is_empty());
}

#[test]
fn pi_adapter_maps_provider_empty_text_invalid_json_binary_and_timeout_failures() {
    let temp = TempDir::new().unwrap();

    let provider_request = sample_request(temp.path());
    let provider_failure = PiRpcSessionResult {
        exit_kind: RunnerExitKind::ProviderError,
        exit_code: Some(1),
        started_at: timestamp(STARTED_AT),
        ended_at: timestamp(ENDED_AT),
        event_lines: vec![r#"{"type":"agent_end","message":{"stopReason":"error"}}"#.to_owned()],
        assistant_text: None,
        token_usage: None,
        failure_class: Some("runner_provider_failure".to_owned()),
        notes: Vec::new(),
        stderr_text: "provider failed".to_owned(),
        observed_exit_kind: None,
        observed_exit_code: None,
    };
    let adapter = PiRpcRunnerAdapter::with_client_factory(
        PiRpcConfig::default(),
        temp.path(),
        MockClientFactory::new(vec![Ok(provider_failure)]),
    );
    let provider_result = adapter.run(&provider_request).unwrap();
    assert_eq!(provider_result.exit_kind, RunnerExitKind::ProviderError);
    assert_eq!(
        normalize_stage_result(&provider_request, &provider_result)
            .unwrap()
            .metadata
            .get("failure_class")
            .and_then(Value::as_str),
        Some("provider_failure")
    );

    let empty_request = sample_request(temp.path());
    let empty_text = PiRpcSessionResult {
        exit_kind: RunnerExitKind::RunnerError,
        exit_code: Some(1),
        started_at: timestamp(STARTED_AT),
        ended_at: timestamp(ENDED_AT),
        event_lines: Vec::new(),
        assistant_text: Some(String::new()),
        token_usage: None,
        failure_class: Some("runner_empty_assistant_text".to_owned()),
        notes: Vec::new(),
        stderr_text: String::new(),
        observed_exit_kind: None,
        observed_exit_code: None,
    };
    let adapter = PiRpcRunnerAdapter::with_client_factory(
        PiRpcConfig::default(),
        temp.path(),
        MockClientFactory::new(vec![Ok(empty_text)]),
    );
    let empty_result = adapter.run(&empty_request).unwrap();
    assert_eq!(empty_result.exit_kind, RunnerExitKind::RunnerError);
    assert_eq!(
        fs::read_to_string(empty_result.stdout_path.as_ref().unwrap()).unwrap(),
        ""
    );
    let completion: Value = serde_json::from_str(
        &fs::read_to_string(
            Path::new(&empty_request.run_dir).join("runner_completion.req-001.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(completion["failure_class"], "runner_empty_assistant_text");

    let invalid_request = sample_request(temp.path());
    let adapter = PiRpcRunnerAdapter::with_client_factory(
        PiRpcConfig::default(),
        temp.path(),
        MockClientFactory::new(vec![Err(PiRpcClientError::InvalidJson {
            message: "invalid JSON in pi rpc stream".to_owned(),
        })]),
    );
    let invalid_result = adapter.run(&invalid_request).unwrap();
    assert_eq!(invalid_result.exit_kind, RunnerExitKind::RunnerError);
    assert!(
        fs::read_to_string(invalid_result.stderr_path.as_ref().unwrap())
            .unwrap()
            .contains("invalid JSON")
    );

    struct MissingBinaryFactory;
    impl PiRpcClientFactory for MissingBinaryFactory {
        fn create(
            &self,
            _request: PiRpcClientCreateRequest,
        ) -> Result<Box<dyn PiRpcPromptClient>, PiRpcClientError> {
            Err(PiRpcClientError::BinaryNotFound {
                binary: "pi".to_owned(),
            })
        }
    }
    let missing_request = sample_request(temp.path());
    let adapter = PiRpcRunnerAdapter::with_client_factory(
        PiRpcConfig::default(),
        temp.path(),
        MissingBinaryFactory,
    );
    let missing_result = adapter.run(&missing_request).unwrap();
    assert_eq!(missing_result.exit_code, Some(127));
    assert_eq!(missing_result.stdout_path, None);
}

#[test]
fn pi_jsonl_client_runs_prompt_queries_final_text_and_session_stats() {
    let events = vec![
        PiRpcStreamEvent::Line(r#"{"type":"response","id":"prompt-1","success":true}"#.to_owned()),
        PiRpcStreamEvent::Line(r#"{"type":"agent_start"}"#.to_owned()),
        PiRpcStreamEvent::Line(r#"{"type":"agent_end"}"#.to_owned()),
        PiRpcStreamEvent::Line(
            "{\"type\":\"response\",\"id\":\"last-assistant-1\",\"success\":true,\"data\":{\"text\":\"### BUILDER_COMPLETE\\n\"}}"
                .to_owned(),
        ),
        PiRpcStreamEvent::Line(
            r#"{"type":"response","id":"session-stats-1","success":true,"data":{"tokens":{"input":10,"cacheRead":2,"output":3,"total":13}}}"#
                .to_owned(),
        ),
    ];
    let transport = ScriptedTransport::with_events(events);
    let mut client = PiRpcJsonlClient::new(transport);

    let result = client.run_prompt("prompt text", 120).unwrap();
    let transport = client.into_transport();

    assert_eq!(result.exit_kind, RunnerExitKind::Completed);
    assert_eq!(
        result.assistant_text,
        Some("### BUILDER_COMPLETE\n".to_owned())
    );
    assert_eq!(
        result.token_usage,
        Some(TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 2,
            output_tokens: 3,
            thinking_tokens: 0,
            total_tokens: 13,
        })
    );
    assert_eq!(
        result.event_lines,
        vec![r#"{"type":"agent_start"}"#, r#"{"type":"agent_end"}"#]
    );
    assert_eq!(transport.sent[0]["type"], "prompt");
    assert_eq!(transport.sent[0]["message"], "prompt text");
    assert_eq!(transport.sent[1]["type"], "get_last_assistant_text");
    assert_eq!(transport.sent[2]["type"], "get_session_stats");
    assert!(transport.closed_stdin);
}

#[test]
fn pi_jsonl_client_detects_provider_failure_invalid_json_timeout_abort_and_hard_kill() {
    let provider_events = vec![
        PiRpcStreamEvent::Line(r#"{"type":"response","id":"prompt-1","success":true}"#.to_owned()),
        PiRpcStreamEvent::Line(
            r#"{"type":"agent_end","message":{"stopReason":"error"}}"#.to_owned(),
        ),
        PiRpcStreamEvent::Line(
            r#"{"type":"response","id":"last-assistant-1","success":true,"data":{"text":""}}"#
                .to_owned(),
        ),
        PiRpcStreamEvent::Line(
            r#"{"type":"response","id":"session-stats-1","success":true,"data":{"tokens":{"input":1,"output":2}}}"#
                .to_owned(),
        ),
    ];
    let mut client = PiRpcJsonlClient::new(ScriptedTransport::with_events(provider_events));
    let provider = client.run_prompt("prompt text", 120).unwrap();
    assert_eq!(provider.exit_kind, RunnerExitKind::ProviderError);
    assert_eq!(
        provider.failure_class.as_deref(),
        Some("runner_provider_failure")
    );

    let invalid_events = vec![PiRpcStreamEvent::Line("not json".to_owned())];
    let mut invalid_transport = ScriptedTransport::with_events(invalid_events);
    invalid_transport.exit_code = Some(1);
    let mut client = PiRpcJsonlClient::new(invalid_transport);
    let error = client.run_prompt("prompt text", 120).unwrap_err();
    assert!(matches!(error, PiRpcClientError::InvalidJson { .. }));

    let timeout_transport = ScriptedTransport {
        exit_code: None,
        ..Default::default()
    };
    let mut client = PiRpcJsonlClient::with_abort_grace_seconds(timeout_transport, 0.0);
    let timeout = client.run_prompt("prompt text", 1).unwrap();
    let transport = client.into_transport();

    assert_eq!(timeout.exit_kind, RunnerExitKind::Timeout);
    assert_eq!(timeout.exit_code, Some(124));
    assert!(
        timeout
            .notes
            .iter()
            .any(|note| note.contains("sent pi rpc abort command"))
    );
    assert!(timeout.notes.iter().any(|note| note.contains("terminate")));
    assert!(timeout.notes.iter().any(|note| note.contains("hard kill")));
    assert_eq!(transport.sent[0]["type"], "prompt");
    assert_eq!(transport.sent[1]["type"], "abort");
    assert_eq!(transport.terminate_count, 1);
    assert_eq!(transport.kill_count, 1);
}

#[test]
fn pi_normalization_preserves_success_and_token_usage() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path());
    let adapter = PiRpcRunnerAdapter::with_client_factory(
        PiRpcConfig::default(),
        temp.path(),
        MockClientFactory::new(vec![Ok(completed_session_result(Vec::new()))]),
    );

    let result = adapter.run(&request).unwrap();
    let envelope = normalize_stage_result(&request, &result).unwrap();

    assert_eq!(envelope.result_class, ResultClass::Success);
    assert_eq!(
        envelope.terminal_result,
        TerminalResult::Execution(millrace_ai::contracts::ExecutionTerminalResult::BuilderComplete)
    );
    assert_eq!(envelope.token_usage.unwrap().total_tokens, 134);
}
