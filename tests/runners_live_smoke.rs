use std::{env, fs, path::Path};

use serde::Serialize;
use serde_json::Value;
use tempfile::TempDir;

use millrace_ai::contracts::{
    ExecutionTerminalResult, Plane, ResultClass, StageName, TerminalResult, WorkItemKind,
};
use millrace_ai::{
    CodexCliConfig, CodexCliRunnerAdapter, CodexPermissionLevel, PiEventLogPolicy, PiRpcConfig,
    PiRpcRunnerAdapter, RequestKind, RunnerRawResult, StageRunRequest, StageRunnerAdapter,
    normalize_stage_result,
};

const CODEX_GATE: &str = "MILLRACE_REAL_CODEX_SMOKE";
const PI_GATE: &str = "MILLRACE_REAL_PI_SMOKE";
const CODEX_REQUEST_ID: &str = "live-smoke-codex";
const CODEX_RUN_ID: &str = "run-live-smoke-codex";
const PI_REQUEST_ID: &str = "live-smoke-pi";
const PI_RUN_ID: &str = "run-live-smoke-pi";

#[derive(Debug, Clone, PartialEq, Eq)]
enum SmokeGate {
    Run,
    Skip { variable: &'static str },
}

impl SmokeGate {
    fn from_value(variable: &'static str, value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("1" | "true" | "TRUE" | "yes" | "YES") => Self::Run,
            _ => Self::Skip { variable },
        }
    }

    fn from_env(variable: &'static str) -> Self {
        Self::from_value(variable, env::var(variable).ok().as_deref())
    }
}

#[test]
fn codex_live_smoke_gate_skips_without_env() {
    assert_eq!(
        SmokeGate::from_value(CODEX_GATE, None),
        SmokeGate::Skip {
            variable: CODEX_GATE
        }
    );
    assert_eq!(
        SmokeGate::from_value(CODEX_GATE, Some("0")),
        SmokeGate::Skip {
            variable: CODEX_GATE
        }
    );
    assert_eq!(SmokeGate::from_value(CODEX_GATE, Some("1")), SmokeGate::Run);
}

#[test]
fn pi_live_smoke_gate_skips_without_env() {
    assert_eq!(
        SmokeGate::from_value(PI_GATE, None),
        SmokeGate::Skip { variable: PI_GATE }
    );
    assert_eq!(
        SmokeGate::from_value(PI_GATE, Some("false")),
        SmokeGate::Skip { variable: PI_GATE }
    );
    assert_eq!(SmokeGate::from_value(PI_GATE, Some("yes")), SmokeGate::Run);
}

#[test]
#[ignore = "requires MILLRACE_REAL_CODEX_SMOKE=1, a real Codex CLI, credentials/subscription, and network access"]
fn codex_real_adapter_live_smoke() {
    let SmokeGate::Run = SmokeGate::from_env(CODEX_GATE) else {
        eprintln!("skipping live Codex smoke: set {CODEX_GATE}=1 to opt in");
        return;
    };

    let temp = TempDir::new().unwrap();
    let request = live_smoke_request(
        temp.path(),
        CODEX_REQUEST_ID,
        CODEX_RUN_ID,
        "codex_cli",
        env_var("MILLRACE_REAL_CODEX_MODEL"),
        env_var("MILLRACE_REAL_CODEX_REASONING_EFFORT"),
        env_u64("MILLRACE_REAL_CODEX_TIMEOUT_SECONDS").unwrap_or(120),
    );
    let config = CodexCliConfig {
        command: env_var("MILLRACE_REAL_CODEX_COMMAND").unwrap_or_else(|| "codex".to_owned()),
        args: env_list("MILLRACE_REAL_CODEX_ARGS").unwrap_or_else(|| vec!["exec".to_owned()]),
        profile: env_var("MILLRACE_REAL_CODEX_PROFILE"),
        permission_default: codex_permission_from_env(),
        model_reasoning_effort: env_var("MILLRACE_REAL_CODEX_REASONING_EFFORT"),
        ..CodexCliConfig::default()
    };
    let adapter = CodexCliRunnerAdapter::new(config, temp.path());

    let raw_result = adapter.run(&request).unwrap();
    let stage_result = normalize_stage_result(&request, &raw_result).unwrap();
    write_live_smoke_evidence(&request, &raw_result, &stage_result);

    assert_live_smoke_success(&request, &raw_result, "codex_cli");
    assert_runner_artifacts(&request, "codex_cli", true);
}

#[test]
#[ignore = "requires MILLRACE_REAL_PI_SMOKE=1, a real Pi RPC CLI, credentials/subscription, and network access"]
fn pi_real_adapter_live_smoke() {
    let SmokeGate::Run = SmokeGate::from_env(PI_GATE) else {
        eprintln!("skipping live Pi smoke: set {PI_GATE}=1 to opt in");
        return;
    };

    let temp = TempDir::new().unwrap();
    let request = live_smoke_request(
        temp.path(),
        PI_REQUEST_ID,
        PI_RUN_ID,
        "pi_rpc",
        env_var("MILLRACE_REAL_PI_MODEL"),
        None,
        env_u64("MILLRACE_REAL_PI_TIMEOUT_SECONDS").unwrap_or(120),
    );
    let config = PiRpcConfig {
        command: env_var("MILLRACE_REAL_PI_COMMAND").unwrap_or_else(|| "pi".to_owned()),
        args: env_list("MILLRACE_REAL_PI_ARGS").unwrap_or_default(),
        provider: env_var("MILLRACE_REAL_PI_PROVIDER"),
        thinking: env_var("MILLRACE_REAL_PI_THINKING"),
        event_log_policy: PiEventLogPolicy::Full,
        ..PiRpcConfig::default()
    };
    let adapter = PiRpcRunnerAdapter::new(config, temp.path());

    let raw_result = adapter.run(&request).unwrap();
    let stage_result = normalize_stage_result(&request, &raw_result).unwrap();
    write_live_smoke_evidence(&request, &raw_result, &stage_result);

    assert_live_smoke_success(&request, &raw_result, "pi_rpc");
    assert_runner_artifacts(&request, "pi_rpc", true);
}

fn live_smoke_request(
    root: &Path,
    request_id: &str,
    run_id: &str,
    runner_name: &str,
    model_name: Option<String>,
    model_reasoning_effort: Option<String>,
    timeout_seconds: u64,
) -> StageRunRequest {
    let run_dir = root.join("millrace-agents").join("runs").join(run_id);
    fs::create_dir_all(&run_dir).unwrap();
    let entrypoint_path = root.join("live_smoke_entrypoint.md");
    fs::write(
        &entrypoint_path,
        "# Live Smoke Entrypoint\n\nPrint exactly `### BUILDER_COMPLETE` and no other text.\n",
    )
    .unwrap();
    let task_path = root.join("live_smoke_task.md");
    fs::write(
        &task_path,
        "# Live Smoke Task\n\nEmit exactly the legal Builder success marker.\n",
    )
    .unwrap();

    let mut request = StageRunRequest {
        request_id: request_id.to_owned(),
        run_id: run_id.to_owned(),
        plane: Plane::Execution,
        stage: StageName::Builder,
        request_kind: RequestKind::ActiveWorkItem,
        mode_id: "live_smoke".to_owned(),
        compiled_plan_id: "plan-live-smoke".to_owned(),
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
        active_work_item_id: Some("live-smoke-task".to_owned()),
        active_work_item_path: Some(task_path.display().to_string()),
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
        model_name,
        model_reasoning_effort,
        timeout_seconds,
    };
    request.validate().unwrap();
    request
}

fn assert_live_smoke_success(
    request: &StageRunRequest,
    raw_result: &RunnerRawResult,
    expected_runner: &str,
) {
    assert_eq!(raw_result.request_id, request.request_id);
    assert_eq!(raw_result.run_id, request.run_id);
    assert_eq!(raw_result.runner_name, expected_runner);

    let stage_result = normalize_stage_result(request, raw_result).unwrap();
    assert_eq!(stage_result.run_id, request.run_id);
    assert_eq!(
        stage_result
            .metadata
            .get("request_id")
            .and_then(Value::as_str),
        Some(request.request_id.as_str())
    );
    assert_eq!(stage_result.runner_name.as_deref(), Some(expected_runner));
    assert_eq!(stage_result.result_class, ResultClass::Success);
    assert_eq!(
        stage_result.terminal_result,
        TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete)
    );
    assert_eq!(
        stage_result.detected_marker.as_deref(),
        Some("### BUILDER_COMPLETE")
    );
}

fn assert_runner_artifacts(request: &StageRunRequest, expected_runner: &str, event_log: bool) {
    let run_dir = Path::new(&request.run_dir);
    let prompt_path = run_dir.join(format!("runner_prompt.{}.md", request.request_id));
    let invocation_path = run_dir.join(format!("runner_invocation.{}.json", request.request_id));
    let stdout_path = run_dir.join(format!("runner_stdout.{}.txt", request.request_id));
    let stderr_path = run_dir.join(format!("runner_stderr.{}.txt", request.request_id));
    let event_path = run_dir.join(format!("runner_events.{}.jsonl", request.request_id));
    let completion_path = run_dir.join(format!("runner_completion.{}.json", request.request_id));
    let raw_result_path = run_dir
        .join("runner_results")
        .join(format!("{}.json", request.request_id));
    let stage_result_path = run_dir
        .join("stage_results")
        .join(format!("{}.json", request.request_id));

    assert!(prompt_path.is_file(), "missing {}", prompt_path.display());
    assert!(
        fs::read_to_string(&prompt_path)
            .unwrap()
            .contains("Stage Request Context:")
    );
    assert!(
        invocation_path.is_file(),
        "missing {}",
        invocation_path.display()
    );
    assert!(stdout_path.is_file(), "missing {}", stdout_path.display());
    assert!(stderr_path.is_file(), "missing {}", stderr_path.display());
    if event_log {
        assert!(event_path.is_file(), "missing {}", event_path.display());
    }
    assert!(
        completion_path.is_file(),
        "missing {}",
        completion_path.display()
    );
    assert!(
        raw_result_path.is_file(),
        "missing {}",
        raw_result_path.display()
    );
    assert!(
        stage_result_path.is_file(),
        "missing {}",
        stage_result_path.display()
    );

    let invocation: Value = read_json(&invocation_path);
    let completion: Value = read_json(&completion_path);
    let raw_result: Value = read_json(&raw_result_path);
    let stage_result: Value = read_json(&stage_result_path);
    assert_eq!(invocation["runner_name"], expected_runner);
    assert_eq!(completion["runner_name"], expected_runner);
    assert_eq!(raw_result["runner_name"], expected_runner);
    assert_eq!(stage_result["runner_name"], expected_runner);
    assert_eq!(stage_result["metadata"]["request_id"], request.request_id);
}

fn write_live_smoke_evidence<T: Serialize>(
    request: &StageRunRequest,
    raw_result: &RunnerRawResult,
    stage_result: &T,
) {
    let run_dir = Path::new(&request.run_dir);
    write_pretty_json(
        &run_dir
            .join("runner_results")
            .join(format!("{}.json", request.request_id)),
        raw_result,
    );
    write_pretty_json(
        &run_dir
            .join("stage_results")
            .join(format!("{}.json", request.request_id)),
        stage_result,
    );
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn write_pretty_json<T: Serialize>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn env_var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn env_list(name: &str) -> Option<Vec<String>> {
    let values = env_var(name)?
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn env_u64(name: &str) -> Option<u64> {
    env_var(name)?.parse().ok()
}

fn codex_permission_from_env() -> CodexPermissionLevel {
    match env_var("MILLRACE_REAL_CODEX_PERMISSION").as_deref() {
        Some("basic") | None => CodexPermissionLevel::Basic,
        Some("elevated") => CodexPermissionLevel::Elevated,
        Some("maximum") => CodexPermissionLevel::Maximum,
        Some(other) => panic!(
            "MILLRACE_REAL_CODEX_PERMISSION must be basic, elevated, or maximum, got {other}"
        ),
    }
}
