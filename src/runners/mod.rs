//! Runner request/result boundary and deterministic fake runner support.

pub mod artifacts;
pub mod codex_cli;
pub mod codex_cli_artifacts;
pub mod codex_cli_tokens;
pub mod contracts;
pub mod dispatcher;
pub mod fake;
pub mod normalization;
pub mod pi_rpc;
pub mod pi_rpc_client;
pub mod process;
pub mod prompting;
pub mod registry;

pub use artifacts::{
    RunnerCompletionArtifact, RunnerCompletionArtifactContext, RunnerInvocationArtifact,
    capability_evidence_refs_for_request, completion_artifact_from_raw_result,
    invocation_artifact_from_request, missing_capability_evidence_refs_for_request,
    write_runner_completion, write_runner_invocation,
};
pub use codex_cli::{
    CodexCliConfig, CodexCliRunnerAdapter, CodexPermissionLevel, CodexProcessError,
    CodexProcessExecutor, CodexProcessRequest, SubprocessCodexExecutor, build_codex_cli_command,
    permission_flags, resolve_permission_level,
};
pub use codex_cli_artifacts::{
    CodexCliArtifactPaths, codex_cli_artifact_paths, materialize_stdout_artifact,
    persist_event_log, reconciled_timeout_terminal_marker,
};
pub use codex_cli_tokens::{extract_token_usage, token_usage_from_line, token_usage_from_payload};
pub use contracts::{
    RunnerError, RunnerExitKind, RunnerRawResult, RunnerResult, StageRunnerAdapter,
};
pub use dispatcher::StageRunnerDispatcher;
pub use fake::{FakeRunner, FakeRunnerConfig, FakeRunnerOutput, FakeRunnerResult};
pub use normalization::normalize_stage_result;
pub use pi_rpc::{
    PiEventLogPolicy, PiRpcArtifactPaths, PiRpcConfig, PiRpcRunnerAdapter, build_pi_rpc_command,
    persistable_event_lines, pi_rpc_artifact_paths, should_persist_event_log,
};
pub use pi_rpc_client::{
    PiRpcClientCreateRequest, PiRpcClientError, PiRpcClientFactory, PiRpcJsonlClient,
    PiRpcPromptClient, PiRpcSessionResult, PiRpcStreamEvent, PiRpcTransport,
    SubprocessPiRpcClientFactory, SubprocessPiRpcTransport, token_usage_from_stats_payload,
};
pub use process::{ProcessExecutionResult, ProcessExitKind, RunnerEnvironmentDelta};
pub use prompting::{
    build_stage_prompt, legal_terminal_markers, runner_prompt_path, write_stage_prompt_artifact,
};
pub use registry::RunnerRegistry;
