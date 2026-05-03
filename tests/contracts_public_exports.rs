use std::any::type_name;

use serde_json::json;

use millrace_ai::contracts::{
    ActiveRunRequestKind, ActiveRunState, CompileDiagnostics, ContractError, ExecutionStageName,
    ExecutionTerminalResult, IdentifierErrorReason, IncidentDecision, IncidentDocument,
    IncidentSeverity, LearningRequestAction, LearningRequestDocument, LearningStageName,
    LearningTerminalResult, LoopEdgeKind, MailboxAddIdeaPayload, MailboxCommand,
    MailboxCommandEnvelope, OutcomeResultClasses, PauseSource, Plane, PlanningStageName,
    PlanningTerminalResult, RecoveryCounterEntry, RecoveryCounters, ReloadOutcome, ResultClass,
    RuntimeErrorCode, RuntimeErrorContext, RuntimeJsonContract, RuntimeJsonError, RuntimeMode,
    RuntimeSnapshot, SAFE_ID_PATTERN_DESCRIPTION, STAGE_LEGAL_TERMINAL_RESULTS,
    STAGE_METADATA_BY_VALUE, STAGE_NAME_BY_VALUE, STAGE_TO_PLANE, SpecDocument, SpecSourceType,
    StageMetadata, StageName, StageResultEnvelope, TaskDocument, TaskStatusHint, TerminalResult,
    Timestamp, TokenUsage, WORK_DOCUMENT_SCHEMA_VERSION, WatcherMode, WorkDocument,
    WorkDocumentError, WorkItemKind, allowed_result_classes_by_outcome, blocked_terminal_for_plane,
    known_stage_values, known_stage_values_for_plane, legal_terminal_markers,
    legal_terminal_results, parse_terminal_marker_for_plane, running_status_marker, stage_metadata,
    stage_metadata_for_value, stage_name_for_plane, stage_name_for_value, stage_plane,
    terminal_result_for_plane, validate_safe_identifier, validate_stage_result_class,
    validate_terminal_marker_for_stage,
};
use millrace_ai::work_documents::{
    parse_task_document, parse_work_document_with_source, render_task_document,
    render_work_document,
};
use millrace_ai::{
    AllowedResultClassPolicy, AllowedResultClassesByOutcome, CodexCliArtifactPaths, CodexCliConfig,
    CodexCliRunnerAdapter, CodexPermissionLevel, CodexProcessError, CodexProcessExecutor,
    CodexProcessRequest, FakeRunner, FakeRunnerConfig, FakeRunnerOutput, FakeRunnerResult,
    PiEventLogPolicy, PiRpcArtifactPaths, PiRpcClientCreateRequest, PiRpcClientError,
    PiRpcClientFactory, PiRpcConfig, PiRpcJsonlClient, PiRpcPromptClient, PiRpcRunnerAdapter,
    PiRpcSessionResult, PiRpcStreamEvent, PiRpcTransport, ProcessExecutionResult, ProcessExitKind,
    RequestKind, RunnerCompletionArtifact, RunnerCompletionArtifactContext, RunnerEnvironmentDelta,
    RunnerError, RunnerExitKind, RunnerInvocationArtifact, RunnerRawResult, RunnerRegistry,
    RunnerResult, RuntimeStartupSession, StageRunRequest, StageRunRequestError, StageRunnerAdapter,
    StageRunnerDispatcher, SubprocessCodexExecutor, SubprocessPiRpcClientFactory,
    SubprocessPiRpcTransport, WorkspaceError, WorkspacePaths, WorkspaceResult,
    build_codex_cli_command, build_pi_rpc_command, build_runtime_runner_dispatcher,
    build_stage_prompt, codex_cli_artifact_paths, completion_artifact_from_raw_result,
    extract_token_usage, invocation_artifact_from_request, materialize_stdout_artifact,
    normalize_stage_result, permission_flags, persist_event_log, persistable_event_lines,
    pi_rpc_artifact_paths, reconciled_timeout_terminal_marker, render_stage_request_context_lines,
    resolve_permission_level, runner_prompt_path, should_persist_event_log, token_usage_from_line,
    token_usage_from_payload, token_usage_from_stats_payload, workspace_paths,
    write_runner_completion, write_runner_invocation, write_stage_prompt_artifact,
};

const NOW: &str = "2026-04-15T00:00:00Z";

type InvocationArtifactBuilder = fn(
    &StageRunRequest,
    String,
    Vec<String>,
    String,
    RunnerEnvironmentDelta,
    String,
    Timestamp,
) -> RunnerResult<RunnerInvocationArtifact>;

fn assert_runtime_contract<T: RuntimeJsonContract>() {}

#[test]
fn public_contract_exports_remain_importable() {
    let public_type_names = [
        type_name::<ActiveRunRequestKind>(),
        type_name::<ActiveRunState>(),
        type_name::<CompileDiagnostics>(),
        type_name::<ContractError>(),
        type_name::<ExecutionStageName>(),
        type_name::<ExecutionTerminalResult>(),
        type_name::<IdentifierErrorReason>(),
        type_name::<IncidentDecision>(),
        type_name::<IncidentDocument>(),
        type_name::<IncidentSeverity>(),
        type_name::<LearningRequestAction>(),
        type_name::<LearningRequestDocument>(),
        type_name::<LearningStageName>(),
        type_name::<LearningTerminalResult>(),
        type_name::<LoopEdgeKind>(),
        type_name::<MailboxAddIdeaPayload>(),
        type_name::<MailboxCommand>(),
        type_name::<MailboxCommandEnvelope>(),
        type_name::<OutcomeResultClasses>(),
        type_name::<PauseSource>(),
        type_name::<Plane>(),
        type_name::<PlanningStageName>(),
        type_name::<PlanningTerminalResult>(),
        type_name::<RecoveryCounterEntry>(),
        type_name::<RecoveryCounters>(),
        type_name::<ReloadOutcome>(),
        type_name::<ResultClass>(),
        type_name::<RuntimeErrorCode>(),
        type_name::<RuntimeErrorContext>(),
        type_name::<RuntimeJsonError>(),
        type_name::<RuntimeMode>(),
        type_name::<RuntimeSnapshot>(),
        type_name::<SpecDocument>(),
        type_name::<SpecSourceType>(),
        type_name::<StageMetadata>(),
        type_name::<StageName>(),
        type_name::<StageResultEnvelope>(),
        type_name::<StageRunRequest>(),
        type_name::<StageRunRequestError>(),
        type_name::<TaskDocument>(),
        type_name::<TaskStatusHint>(),
        type_name::<TerminalResult>(),
        type_name::<Timestamp>(),
        type_name::<TokenUsage>(),
        type_name::<WatcherMode>(),
        type_name::<WorkspaceError>(),
        type_name::<WorkspacePaths>(),
        type_name::<WorkspaceResult<WorkspacePaths>>(),
        type_name::<WorkDocument>(),
        type_name::<WorkDocumentError>(),
        type_name::<WorkItemKind>(),
        type_name::<AllowedResultClassPolicy>(),
        type_name::<AllowedResultClassesByOutcome>(),
        type_name::<CodexCliArtifactPaths>(),
        type_name::<CodexCliConfig>(),
        type_name::<CodexCliRunnerAdapter>(),
        type_name::<CodexPermissionLevel>(),
        type_name::<CodexProcessError>(),
        type_name::<CodexProcessRequest>(),
        type_name::<SubprocessCodexExecutor>(),
        type_name::<FakeRunner>(),
        type_name::<FakeRunnerConfig>(),
        type_name::<FakeRunnerOutput>(),
        type_name::<FakeRunnerResult>(),
        type_name::<PiEventLogPolicy>(),
        type_name::<PiRpcArtifactPaths>(),
        type_name::<PiRpcClientCreateRequest>(),
        type_name::<PiRpcClientError>(),
        type_name::<PiRpcConfig>(),
        type_name::<PiRpcJsonlClient<SubprocessPiRpcTransport>>(),
        type_name::<PiRpcRunnerAdapter>(),
        type_name::<PiRpcSessionResult>(),
        type_name::<PiRpcStreamEvent>(),
        type_name::<SubprocessPiRpcClientFactory>(),
        type_name::<SubprocessPiRpcTransport>(),
        type_name::<RequestKind>(),
        type_name::<ProcessExecutionResult>(),
        type_name::<ProcessExitKind>(),
        type_name::<RunnerCompletionArtifact>(),
        type_name::<RunnerEnvironmentDelta>(),
        type_name::<RunnerError>(),
        type_name::<RunnerExitKind>(),
        type_name::<RunnerInvocationArtifact>(),
        type_name::<RunnerRawResult>(),
        type_name::<RunnerRegistry>(),
        type_name::<RunnerResult<RunnerRawResult>>(),
        type_name::<StageRunnerDispatcher>(),
    ];

    assert!(
        public_type_names
            .iter()
            .all(|name| name.contains("millrace_ai"))
    );

    assert_runtime_contract::<CompileDiagnostics>();
    assert_runtime_contract::<MailboxCommandEnvelope>();
    assert_runtime_contract::<RecoveryCounters>();
    assert_runtime_contract::<RuntimeSnapshot>();
    assert_runtime_contract::<TokenUsage>();

    let _adapter: Option<&dyn StageRunnerAdapter> = None;
    let _codex_executor: Option<&dyn CodexProcessExecutor> = None;
    let _pi_factory: Option<&dyn PiRpcClientFactory> = None;
    let _pi_client: Option<&dyn PiRpcPromptClient> = None;
    let _pi_transport: Option<&dyn PiRpcTransport> = None;
    let _dispatcher: fn(RunnerRegistry) -> StageRunnerDispatcher = StageRunnerDispatcher::new;
    let _runtime_dispatcher: fn(&RuntimeStartupSession) -> RunnerResult<StageRunnerDispatcher> =
        build_runtime_runner_dispatcher;
    let _context_renderer: fn(&StageRunRequest) -> Vec<String> = render_stage_request_context_lines;
    let _prompt_builder: fn(&StageRunRequest) -> String = build_stage_prompt;
    let _prompt_path: fn(&StageRunRequest) -> std::path::PathBuf = runner_prompt_path;
    let _prompt_writer: fn(&StageRunRequest) -> RunnerResult<std::path::PathBuf> =
        write_stage_prompt_artifact;
    let _normalizer: fn(&StageRunRequest, &RunnerRawResult) -> RunnerResult<StageResultEnvelope> =
        normalize_stage_result;
    let _invocation_builder: InvocationArtifactBuilder = invocation_artifact_from_request;
    let _completion_builder: fn(
        &StageRunRequest,
        &RunnerRawResult,
        RunnerCompletionArtifactContext,
    ) -> RunnerResult<RunnerCompletionArtifact> = completion_artifact_from_raw_result;
    let _invocation_writer: fn(&std::path::Path, &RunnerInvocationArtifact) -> RunnerResult<()> =
        write_runner_invocation;
    let _completion_writer: fn(&std::path::Path, &RunnerCompletionArtifact) -> RunnerResult<()> =
        write_runner_completion;
    let _codex_command: fn(
        &CodexCliConfig,
        &std::path::Path,
        &StageRunRequest,
        &str,
        &std::path::Path,
    ) -> Vec<String> = build_codex_cli_command;
    let _codex_paths: fn(&std::path::Path, &str) -> CodexCliArtifactPaths =
        codex_cli_artifact_paths;
    let _codex_event_log: fn(
        &std::path::Path,
        &std::path::Path,
    ) -> RunnerResult<Option<std::path::PathBuf>> = persist_event_log;
    let _codex_stdout: fn(
        &std::path::Path,
        &std::path::Path,
        Option<&std::path::Path>,
    ) -> RunnerResult<Option<std::path::PathBuf>> = materialize_stdout_artifact;
    let _codex_reconcile: fn(&StageRunRequest, &std::path::Path) -> Option<String> =
        reconciled_timeout_terminal_marker;
    let _codex_tokens: fn(Option<&std::path::Path>) -> Option<TokenUsage> = extract_token_usage;
    let _codex_token_line: fn(&str) -> Option<TokenUsage> = token_usage_from_line;
    let _codex_token_payload: fn(&serde_json::Value) -> Option<TokenUsage> =
        token_usage_from_payload;
    let _codex_permissions: fn(&CodexCliConfig, &StageRunRequest) -> CodexPermissionLevel =
        resolve_permission_level;
    let _codex_permission_flags: fn(CodexPermissionLevel) -> &'static [&'static str] =
        permission_flags;
    let _pi_command: fn(&PiRpcConfig, &StageRunRequest) -> RunnerResult<Vec<String>> =
        build_pi_rpc_command;
    let _pi_paths: fn(&std::path::Path, &str) -> PiRpcArtifactPaths = pi_rpc_artifact_paths;
    let _pi_persistable: fn(&[String]) -> Vec<String> = persistable_event_lines;
    let _pi_event_policy: fn(PiEventLogPolicy, &PiRpcSessionResult, &[String]) -> bool =
        should_persist_event_log;
    let _pi_stats_tokens: fn(Option<&serde_json::Value>) -> Option<TokenUsage> =
        token_usage_from_stats_payload;

    let root = std::env::temp_dir().join("millrace-public-export-paths");
    assert_eq!(
        workspace_paths(&root).runtime_root,
        root.join("millrace-agents")
    );
}

#[test]
fn public_metadata_helpers_expose_the_stage_contract_boundary() {
    assert_eq!(millrace_ai::PACKAGE_NAME, "millrace-ai");
    assert_eq!(millrace_ai::CRATE_NAME, "millrace_ai");
    assert_eq!(millrace_ai::CLI_NAME, "millrace");
    assert_eq!(millrace_ai::STABILITY, "experimental");
    assert_eq!(
        millrace_ai::runtime_status().version,
        env!("CARGO_PKG_VERSION")
    );

    assert_eq!(SAFE_ID_PATTERN_DESCRIPTION, "^[A-Za-z0-9][A-Za-z0-9._-]*$");
    assert_eq!(WORK_DOCUMENT_SCHEMA_VERSION, "1.0");
    assert_eq!(STAGE_METADATA_BY_VALUE.len(), StageName::ALL.len());
    assert_eq!(STAGE_NAME_BY_VALUE.len(), StageName::ALL.len());
    assert_eq!(STAGE_TO_PLANE.len(), StageName::ALL.len());
    assert_eq!(STAGE_LEGAL_TERMINAL_RESULTS.len(), StageName::ALL.len());

    let metadata = stage_metadata(StageName::Builder);
    assert_eq!(metadata.stage, StageName::Builder);
    assert_eq!(stage_metadata_for_value("builder").unwrap(), metadata);
    assert_eq!(stage_plane(StageName::Builder), Plane::Execution);
    assert_eq!(
        stage_name_for_plane(Plane::Execution, "builder").unwrap(),
        StageName::Builder
    );
    assert_eq!(stage_name_for_value("curator").unwrap(), StageName::Curator);
    assert_eq!(running_status_marker(StageName::Builder), "BUILDER_RUNNING");
    assert_eq!(
        legal_terminal_markers(StageName::Builder),
        vec!["### BUILDER_COMPLETE".to_owned(), "### BLOCKED".to_owned()]
    );
    assert_eq!(
        legal_terminal_results(StageName::Builder),
        &[
            TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete),
            TerminalResult::Execution(ExecutionTerminalResult::Blocked),
        ]
    );
    assert_eq!(
        allowed_result_classes_by_outcome(StageName::Builder)[0].result_classes,
        &[ResultClass::Success]
    );
    assert_eq!(
        known_stage_values_for_plane(Plane::Learning),
        ["analyst", "professor", "curator"]
    );
    assert!(known_stage_values().contains(&"builder"));
    assert_eq!(
        terminal_result_for_plane(Plane::Learning, "BLOCKED").unwrap(),
        blocked_terminal_for_plane(Plane::Learning)
    );
    assert_eq!(
        parse_terminal_marker_for_plane(Plane::Execution, "### BLOCKED").unwrap(),
        TerminalResult::Execution(ExecutionTerminalResult::Blocked)
    );
    assert_eq!(
        validate_terminal_marker_for_stage(StageName::Builder, "### BUILDER_COMPLETE").unwrap(),
        TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete)
    );
    validate_stage_result_class(
        StageName::Builder,
        TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete),
        ResultClass::Success,
    )
    .unwrap();
    assert_eq!(
        validate_safe_identifier("task-001.alpha_beta", "task_id").unwrap(),
        "task-001.alpha_beta"
    );
}

#[test]
fn public_work_document_helpers_expose_parse_and_render_boundary() {
    let raw = format!(
        "# Public task\n\n\
         Task-ID: public-task\n\
         Title: Public task\n\
         Created-At: {NOW}\n\
         Created-By: tests\n\n\
         Target-Paths:\n\
         - src/contracts/\n\n\
         Acceptance:\n\
         - public helpers parse\n\n\
         Required-Checks:\n\
         - cargo test\n\n\
         References:\n\
         - docs/rust-port-roadmap.md\n\n\
         Risk:\n\
         - export drift\n"
    );

    let task = parse_task_document(&raw).unwrap();
    let rendered_task = render_task_document(&task);
    let document = parse_work_document_with_source(&rendered_task, "public-task.md").unwrap();

    assert_eq!(document.kind(), WorkItemKind::Task);
    assert_eq!(render_work_document(&document), rendered_task);
}

#[test]
fn accepted_and_rejected_public_json_examples_are_deterministic() {
    let mut add_idea = MailboxAddIdeaPayload::from_json_value(json!({
        "source_name": "idea-001.md",
        "markdown": "# Idea\n"
    }))
    .unwrap();
    add_idea.validate().unwrap();

    let bad_payload = MailboxAddIdeaPayload::from_json_value(json!({
        "source_name": "nested/idea.md",
        "markdown": "# Idea\n"
    }))
    .unwrap_err();
    assert!(bad_payload.to_string().contains("single relative filename"));

    let bad_marker =
        validate_terminal_marker_for_stage(StageName::Builder, "### CHECKER_PASS").unwrap_err();
    assert!(matches!(
        bad_marker,
        ContractError::TerminalResultNotAllowed { .. }
    ));
}
