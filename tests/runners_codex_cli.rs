use std::{cell::RefCell, collections::BTreeMap, fs, path::Path, rc::Rc};

use serde_json::Value;
use tempfile::TempDir;

use millrace_ai::contracts::{
    Plane, ResultClass, StageName, TerminalResult, Timestamp, TokenUsage, WorkItemKind,
};
use millrace_ai::{
    CodexCliConfig, CodexCliRunnerAdapter, CodexPermissionLevel, CodexProcessError,
    CodexProcessExecutor, CodexProcessRequest, ProcessExecutionResult, ProcessExitKind,
    RequestKind, RunnerExitKind, StageRunRequest, StageRunnerAdapter, build_stage_prompt,
    normalize_stage_result, permission_flags,
};

const RUN_ID: &str = "run-001";
const REQUEST_ID: &str = "req-001";
const STARTED_AT: &str = "2026-04-29T00:00:00Z";
const ENDED_AT: &str = "2026-04-29T00:00:01Z";

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

fn sample_request(
    root: &Path,
    stage: StageName,
    request_id: &str,
    run_id: &str,
) -> StageRunRequest {
    let run_dir = root.join("runs").join(run_id);
    fs::create_dir_all(&run_dir).unwrap();
    let entrypoint_path = root.join("entrypoint.md");
    fs::write(&entrypoint_path, "# Entrypoint\n").unwrap();
    let mut request = StageRunRequest {
        request_id: request_id.to_owned(),
        run_id: run_id.to_owned(),
        plane: Plane::Execution,
        stage,
        request_kind: RequestKind::ActiveWorkItem,
        mode_id: "standard_plain".to_owned(),
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
        runner_name: Some("codex_cli".to_owned()),
        model_name: Some("gpt-5".to_owned()),
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

fn process_result(
    request: &CodexProcessRequest,
    exit_kind: ProcessExitKind,
    exit_code: Option<i32>,
) -> ProcessExecutionResult {
    let mut result = ProcessExecutionResult::new(
        request.command.clone(),
        request.cwd.display().to_string(),
        request.environment_delta.clone(),
        exit_kind,
        exit_code,
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

#[test]
fn codex_adapter_writes_prompt_invocation_completion_events_stdout_and_tokens() {
    let temp = TempDir::new().unwrap();
    let mut request = sample_request(temp.path(), StageName::Builder, REQUEST_ID, RUN_ID);
    request.thinking_level = Some("high".to_owned());
    request.model_reasoning_effort = Some("medium".to_owned());
    request.validate().unwrap();
    let seen = Rc::new(RefCell::new(None));
    let seen_for_executor = Rc::clone(&seen);
    let executor = FnExecutor(move |process_request: &CodexProcessRequest| {
        *seen_for_executor.borrow_mut() = Some(process_request.clone());
        fs::write(
            &process_request.stdout_path,
            concat!(
                "{\"type\":\"thread.started\",\"thread_id\":\"thread-001\"}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":",
                "{\"total_token_usage\":{\"input_tokens\":120,\"cached_input_tokens\":40,",
                "\"output_tokens\":12,\"reasoning_output_tokens\":5,\"total_tokens\":132}}}}\n"
            ),
        )
        .unwrap();
        fs::write(
            command_option_value(&process_request.command, "--output-last-message"),
            "### BUILDER_COMPLETE\n",
        )
        .unwrap();
        fs::write(&process_request.stderr_path, "").unwrap();
        Ok(process_result(
            process_request,
            ProcessExitKind::Completed,
            Some(0),
        ))
    });
    let adapter = CodexCliRunnerAdapter::with_process_executor(
        CodexCliConfig::default(),
        temp.path(),
        executor,
    );

    let result = adapter.run(&request).unwrap();

    assert_eq!(result.exit_kind, RunnerExitKind::Completed);
    assert_eq!(result.thinking_level.as_deref(), Some("high"));
    assert_eq!(result.model_reasoning_effort.as_deref(), Some("high"));
    assert_eq!(
        result.token_usage,
        Some(TokenUsage {
            input_tokens: 120,
            cached_input_tokens: 40,
            output_tokens: 12,
            thinking_tokens: 5,
            total_tokens: 132,
        })
    );
    let stdout_path = Path::new(result.stdout_path.as_ref().unwrap());
    let event_log_path = Path::new(result.event_log_path.as_ref().unwrap());
    assert_eq!(
        fs::read_to_string(stdout_path).unwrap(),
        "### BUILDER_COMPLETE\n"
    );
    assert!(
        fs::read_to_string(event_log_path)
            .unwrap()
            .contains("token_count")
    );

    let run_dir = Path::new(&request.run_dir);
    let prompt = fs::read_to_string(run_dir.join("runner_prompt.req-001.md")).unwrap();
    assert!(prompt.contains("Stage Request Context:"));
    assert!(prompt.contains("Entrypoint Contract ID: builder.contract.v1"));
    assert!(prompt.contains("Thinking Level: high"));
    assert!(prompt.contains("Required Skill Paths:"));
    assert!(prompt.contains("Do not invent or rename terminal markers."));

    let invocation: Value = serde_json::from_str(
        &fs::read_to_string(run_dir.join("runner_invocation.req-001.json")).unwrap(),
    )
    .unwrap();
    let completion: Value = serde_json::from_str(
        &fs::read_to_string(run_dir.join("runner_completion.req-001.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(invocation["runner_name"], "codex_cli");
    assert_eq!(invocation["request_id"], request.request_id);
    assert_eq!(invocation["thinking_level"], "high");
    assert_eq!(invocation["model_reasoning_effort"], "medium");
    assert_eq!(completion["runner_name"], "codex_cli");
    assert_eq!(completion["run_id"], request.run_id);
    assert_eq!(completion["thinking_level"], "high");
    assert_eq!(completion["model_reasoning_effort"], "high");
    let event_log_text = event_log_path.display().to_string();
    assert_eq!(
        completion["event_log_path"].as_str(),
        Some(event_log_text.as_str())
    );
    assert_eq!(completion["token_usage"]["total_tokens"], 132);
    assert_eq!(seen.borrow().as_ref().unwrap().timeout_seconds, 120);
}

#[test]
fn codex_command_preserves_python_flag_order_and_environment_delta() {
    let temp = TempDir::new().unwrap();
    let mut request = sample_request(temp.path(), StageName::Builder, REQUEST_ID, RUN_ID);
    request.thinking_level = Some("high".to_owned());
    request.model_reasoning_effort = Some("medium".to_owned());
    request.validate().unwrap();
    let mut env = BTreeMap::new();
    env.insert("MILLRACE_TEST_ENV".to_owned(), "1".to_owned());
    let config = CodexCliConfig {
        command: "codex-beta".to_owned(),
        args: vec!["exec".to_owned(), "--experimental".to_owned()],
        profile: Some("work".to_owned()),
        permission_default: CodexPermissionLevel::Basic,
        model_reasoning_effort: Some("medium".to_owned()),
        extra_config: vec![
            "model_reasoning_effort=\"xhigh\"".to_owned(),
            "sandbox_workspace_write=true".to_owned(),
        ],
        env,
        ..CodexCliConfig::default()
    };
    let seen = Rc::new(RefCell::new(None));
    let seen_for_executor = Rc::clone(&seen);
    let executor = FnExecutor(move |process_request: &CodexProcessRequest| {
        *seen_for_executor.borrow_mut() = Some(process_request.clone());
        fs::write(&process_request.stdout_path, "### BUILDER_COMPLETE\n").unwrap();
        fs::write(&process_request.stderr_path, "").unwrap();
        Ok(process_result(
            process_request,
            ProcessExitKind::Completed,
            Some(0),
        ))
    });
    let adapter = CodexCliRunnerAdapter::with_process_executor(config, temp.path(), executor);

    adapter.run(&request).unwrap();

    let observed = seen.borrow();
    let process_request = observed.as_ref().unwrap();
    let command = &process_request.command;
    assert_eq!(command[0], "codex-beta");
    assert_eq!(command[1], "exec");
    assert_eq!(command[2], "--experimental");
    assert_eq!(command_option_value(command, "--profile"), "work");
    assert_eq!(command_option_value(command, "--model"), "gpt-5");
    assert!(command.contains(&"--skip-git-repo-check".to_owned()));
    assert!(command.contains(&"--full-auto".to_owned()));
    assert_eq!(
        command_option_value(command, "--cd"),
        temp.path().display().to_string()
    );
    assert!(command.contains(&"--json".to_owned()));
    assert!(command.contains(&"--output-last-message".to_owned()));
    assert_eq!(command.last().unwrap(), &build_stage_prompt(&request));
    assert_eq!(
        process_request
            .environment_delta
            .set
            .get("MILLRACE_TEST_ENV"),
        Some(&"1".to_owned())
    );

    let c_values = command
        .iter()
        .enumerate()
        .filter(|&(_, value)| value == "-c")
        .map(|(index, _)| command[index + 1].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        &c_values[c_values.len() - 3..],
        &[
            "model_reasoning_effort=\"xhigh\"".to_owned(),
            "sandbox_workspace_write=true".to_owned(),
            "model_reasoning_effort=\"high\"".to_owned()
        ],
    );
}

#[test]
fn codex_adapter_resolves_permission_precedence_and_mapping() {
    let temp = TempDir::new().unwrap();
    let mut config = CodexCliConfig {
        permission_default: CodexPermissionLevel::Basic,
        ..CodexCliConfig::default()
    };
    config
        .permission_by_model
        .insert("gpt-5".to_owned(), CodexPermissionLevel::Elevated);
    config
        .permission_by_stage
        .insert("builder".to_owned(), CodexPermissionLevel::Maximum);

    assert_eq!(
        permission_flags(CodexPermissionLevel::Basic),
        &["--full-auto"]
    );
    assert_eq!(
        permission_flags(CodexPermissionLevel::Elevated),
        &[
            "-c",
            "approval_policy=\"never\"",
            "--sandbox",
            "danger-full-access"
        ]
    );
    assert_eq!(
        permission_flags(CodexPermissionLevel::Maximum),
        &["--dangerously-bypass-approvals-and-sandbox"]
    );

    let seen = Rc::new(RefCell::new(BTreeMap::<String, Vec<String>>::new()));
    let seen_for_executor = Rc::clone(&seen);
    let executor = FnExecutor(move |process_request: &CodexProcessRequest| {
        let request_id = process_request
            .stdout_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .trim_start_matches("runner_stdout.")
            .to_owned();
        seen_for_executor
            .borrow_mut()
            .insert(request_id.clone(), process_request.command.clone());
        let marker = match request_id.as_str() {
            "builder-req" => "### BUILDER_COMPLETE\n",
            "checker-req" => "### CHECKER_PASS\n",
            "updater-req" => "### UPDATE_COMPLETE\n",
            _ => unreachable!(),
        };
        fs::write(&process_request.stdout_path, marker).unwrap();
        fs::write(&process_request.stderr_path, "").unwrap();
        Ok(process_result(
            process_request,
            ProcessExitKind::Completed,
            Some(0),
        ))
    });
    let adapter = CodexCliRunnerAdapter::with_process_executor(config, temp.path(), executor);

    adapter
        .run(&sample_request(
            temp.path(),
            StageName::Builder,
            "builder-req",
            "builder-run",
        ))
        .unwrap();
    adapter
        .run(&sample_request(
            temp.path(),
            StageName::Checker,
            "checker-req",
            "checker-run",
        ))
        .unwrap();
    let mut updater = sample_request(
        temp.path(),
        StageName::Updater,
        "updater-req",
        "updater-run",
    );
    updater.model_name = Some("gpt-4".to_owned());
    adapter.run(&updater).unwrap();

    let commands = seen.borrow();
    let builder = commands.get("builder-req").unwrap();
    let checker = commands.get("checker-req").unwrap();
    let updater = commands.get("updater-req").unwrap();
    assert!(builder.contains(&"--dangerously-bypass-approvals-and-sandbox".to_owned()));
    assert!(!builder.contains(&"--full-auto".to_owned()));
    assert!(checker.contains(&"approval_policy=\"never\"".to_owned()));
    assert!(checker.contains(&"--sandbox".to_owned()));
    assert!(checker.contains(&"danger-full-access".to_owned()));
    assert!(!checker.contains(&"--dangerously-bypass-approvals-and-sandbox".to_owned()));
    assert!(updater.contains(&"--full-auto".to_owned()));
    assert!(!updater.contains(&"approval_policy=\"never\"".to_owned()));
}

#[test]
fn codex_adapter_uses_one_hour_fallback_timeout_when_request_timeout_is_zero() {
    let temp = TempDir::new().unwrap();
    let mut request = sample_request(temp.path(), StageName::Builder, REQUEST_ID, RUN_ID);
    request.timeout_seconds = 0;
    let observed_timeout = Rc::new(RefCell::new(None));
    let observed_timeout_for_executor = Rc::clone(&observed_timeout);
    let executor = FnExecutor(move |process_request: &CodexProcessRequest| {
        *observed_timeout_for_executor.borrow_mut() = Some(process_request.timeout_seconds);
        fs::write(&process_request.stdout_path, "### BUILDER_COMPLETE\n").unwrap();
        fs::write(&process_request.stderr_path, "").unwrap();
        Ok(process_result(
            process_request,
            ProcessExitKind::Completed,
            Some(0),
        ))
    });
    let adapter = CodexCliRunnerAdapter::with_process_executor(
        CodexCliConfig::default(),
        temp.path(),
        executor,
    );

    adapter.run(&request).unwrap();

    assert_eq!(*observed_timeout.borrow(), Some(3600));
}

#[test]
fn codex_adapter_reconciles_timeout_after_final_terminal_marker() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path(), StageName::Builder, REQUEST_ID, RUN_ID);
    let executor = FnExecutor(|process_request: &CodexProcessRequest| {
        fs::write(
            &process_request.stdout_path,
            "{\"type\":\"thread.started\",\"thread_id\":\"thread-001\"}\n",
        )
        .unwrap();
        fs::write(
            command_option_value(&process_request.command, "--output-last-message"),
            "\n### BUILDER_COMPLETE\n\n",
        )
        .unwrap();
        fs::write(
            &process_request.stderr_path,
            "runner timed out after 120 seconds\n",
        )
        .unwrap();
        Ok(process_result(
            process_request,
            ProcessExitKind::Timeout,
            Some(124),
        ))
    });
    let adapter = CodexCliRunnerAdapter::with_process_executor(
        CodexCliConfig::default(),
        temp.path(),
        executor,
    );

    let result = adapter.run(&request).unwrap();

    assert_eq!(result.exit_kind, RunnerExitKind::Completed);
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.observed_exit_kind, Some(RunnerExitKind::Timeout));
    assert_eq!(result.observed_exit_code, Some(124));
    assert_eq!(
        fs::read_to_string(result.stdout_path.as_ref().unwrap()).unwrap(),
        "\n### BUILDER_COMPLETE\n\n"
    );
    let completion: Value = serde_json::from_str(
        &fs::read_to_string(Path::new(&request.run_dir).join("runner_completion.req-001.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(completion["exit_kind"], "completed");
    assert_eq!(completion["observed_exit_kind"], "timeout");
    assert_eq!(completion["failure_class"], Value::Null);
    assert!(
        completion["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note.as_str().unwrap().contains("reconciled"))
    );
    let envelope = normalize_stage_result(&request, &result).unwrap();
    assert_eq!(
        envelope.terminal_result,
        TerminalResult::Execution(millrace_ai::contracts::ExecutionTerminalResult::BuilderComplete)
    );
    assert_eq!(envelope.result_class, ResultClass::Success);
}

#[test]
fn codex_adapter_maps_failure_modes_to_inspectable_raw_results_and_completion_notes() {
    let temp = TempDir::new().unwrap();

    let missing = sample_request(
        temp.path(),
        StageName::Builder,
        "missing-req",
        "missing-run",
    );
    let missing_adapter = CodexCliRunnerAdapter::with_process_executor(
        CodexCliConfig::default(),
        temp.path(),
        FnExecutor(|_process_request: &CodexProcessRequest| {
            Err(CodexProcessError::BinaryNotFound {
                binary: "codex".to_owned(),
            })
        }),
    );
    let missing_result = missing_adapter.run(&missing).unwrap();
    assert_eq!(missing_result.exit_kind, RunnerExitKind::RunnerError);
    assert_eq!(missing_result.exit_code, Some(127));
    assert!(
        fs::read_to_string(missing_result.stderr_path.as_ref().unwrap())
            .unwrap()
            .contains("runner binary not found")
    );
    let missing_completion: Value = serde_json::from_str(
        &fs::read_to_string(Path::new(&missing.run_dir).join("runner_completion.missing-req.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        missing_completion["failure_class"],
        "runner_binary_not_found"
    );

    let nonzero = sample_request(
        temp.path(),
        StageName::Builder,
        "nonzero-req",
        "nonzero-run",
    );
    let nonzero_adapter = CodexCliRunnerAdapter::with_process_executor(
        CodexCliConfig::default(),
        temp.path(),
        FnExecutor(|process_request: &CodexProcessRequest| {
            fs::write(&process_request.stdout_path, "### BUILDER_COMPLETE\n").unwrap();
            fs::write(&process_request.stderr_path, "nonzero\n").unwrap();
            Ok(process_result(
                process_request,
                ProcessExitKind::Completed,
                Some(2),
            ))
        }),
    );
    let nonzero_result = nonzero_adapter.run(&nonzero).unwrap();
    assert_eq!(nonzero_result.exit_kind, RunnerExitKind::RunnerError);
    let nonzero_completion: Value = serde_json::from_str(
        &fs::read_to_string(Path::new(&nonzero.run_dir).join("runner_completion.nonzero-req.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(nonzero_completion["failure_class"], "runner_non_zero_exit");

    let transport = sample_request(
        temp.path(),
        StageName::Builder,
        "transport-req",
        "transport-run",
    );
    let transport_adapter = CodexCliRunnerAdapter::with_process_executor(
        CodexCliConfig::default(),
        temp.path(),
        FnExecutor(|process_request: &CodexProcessRequest| {
            fs::write(&process_request.stdout_path, "### BUILDER_COMPLETE\n").unwrap();
            fs::write(&process_request.stderr_path, "transport warning\n").unwrap();
            let mut result = process_result(process_request, ProcessExitKind::Completed, Some(0));
            result.transport_error = Some("transport_error".to_owned());
            Ok(result)
        }),
    );
    let transport_result = transport_adapter.run(&transport).unwrap();
    assert_eq!(transport_result.exit_kind, RunnerExitKind::RunnerError);
    let transport_completion: Value = serde_json::from_str(
        &fs::read_to_string(
            Path::new(&transport.run_dir).join("runner_completion.transport-req.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        transport_completion["failure_class"],
        "runner_transport_failure"
    );
}

#[test]
fn codex_adapter_preserves_empty_final_text_for_normalization_failure() {
    let temp = TempDir::new().unwrap();
    let request = sample_request(temp.path(), StageName::Builder, REQUEST_ID, RUN_ID);
    let executor = FnExecutor(|process_request: &CodexProcessRequest| {
        fs::write(&process_request.stdout_path, "").unwrap();
        fs::write(
            command_option_value(&process_request.command, "--output-last-message"),
            "",
        )
        .unwrap();
        fs::write(&process_request.stderr_path, "").unwrap();
        Ok(process_result(
            process_request,
            ProcessExitKind::Completed,
            Some(0),
        ))
    });
    let adapter = CodexCliRunnerAdapter::with_process_executor(
        CodexCliConfig::default(),
        temp.path(),
        executor,
    );

    let result = adapter.run(&request).unwrap();

    assert_eq!(result.exit_kind, RunnerExitKind::Completed);
    assert_eq!(
        fs::read_to_string(result.stdout_path.as_ref().unwrap()).unwrap(),
        ""
    );
    let envelope = normalize_stage_result(&request, &result).unwrap();
    assert_eq!(envelope.terminal_result.as_str(), "BLOCKED");
    assert_eq!(
        envelope
            .metadata
            .get("failure_class")
            .and_then(Value::as_str),
        Some("missing_terminal_result")
    );
}
