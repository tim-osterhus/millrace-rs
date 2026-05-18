//! Pi RPC runner adapter.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    contracts::{
        CapabilityDecisionState, CapabilityEnforcementMode, CapabilitySupportDecision,
        CapabilitySupportState, ExecutionCapabilityGrant, Timestamp,
    },
    runtime::StageRunRequest,
};

use super::{
    RunnerEnvironmentDelta, RunnerError, RunnerExitKind, RunnerRawResult, RunnerResult,
    StageRunnerAdapter,
    artifacts::{
        RunnerCompletionArtifactContext, completion_artifact_from_raw_result,
        invocation_artifact_from_request, write_runner_completion, write_runner_invocation,
    },
    capability_evidence_refs_for_request, missing_capability_evidence_refs_for_request,
    pi_rpc_client::{
        PiRpcClientCreateRequest, PiRpcClientError, PiRpcClientFactory, PiRpcSessionResult,
        SubprocessPiRpcClientFactory,
    },
    prompting::build_stage_prompt,
};

const PI_RESERVED_ARGS: [&str; 7] = [
    "--mode",
    "--no-session",
    "--provider",
    "--model",
    "--thinking",
    "--no-context-files",
    "--no-skills",
];

/// Pi RPC event-log persistence policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiEventLogPolicy {
    /// Persist filtered event logs only for non-completed sessions.
    FailureFull,
    /// Persist filtered event logs for successful and failed sessions.
    Full,
}

impl PiEventLogPolicy {
    /// Returns the stable serialized policy value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FailureFull => "failure_full",
            Self::Full => "full",
        }
    }
}

/// Runtime Pi RPC settings consumed by the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiRpcConfig {
    pub command: String,
    pub args: Vec<String>,
    pub provider: Option<String>,
    pub thinking: Option<String>,
    pub disable_context_files: bool,
    pub disable_skills: bool,
    pub event_log_policy: PiEventLogPolicy,
    pub env: BTreeMap<String, String>,
}

impl Default for PiRpcConfig {
    fn default() -> Self {
        Self {
            command: "pi".to_owned(),
            args: Vec::new(),
            provider: None,
            thinking: None,
            disable_context_files: true,
            disable_skills: true,
            event_log_policy: PiEventLogPolicy::FailureFull,
            env: BTreeMap::new(),
        }
    }
}

impl PiRpcConfig {
    /// Validates Python-reserved transport flags.
    pub fn validate(&self) -> RunnerResult<()> {
        let conflicts = reserved_arg_conflicts(&self.args);
        if conflicts.is_empty() {
            return Ok(());
        }
        Err(RunnerError::InvalidRunnerArtifact {
            message: format!(
                "reserved pi runner flags are not allowed in runners.pi.args: {}",
                conflicts.join(", ")
            ),
        })
    }
}

/// Canonical Pi RPC artifact paths for one stage request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiRpcArtifactPaths {
    pub prompt_path: PathBuf,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
    pub event_log_path: PathBuf,
    pub invocation_path: PathBuf,
    pub completion_path: PathBuf,
}

/// Returns Python-compatible Pi RPC artifact paths.
#[must_use]
pub fn pi_rpc_artifact_paths(run_dir: &Path, request_id: &str) -> PiRpcArtifactPaths {
    PiRpcArtifactPaths {
        prompt_path: run_dir.join(format!("runner_prompt.{request_id}.md")),
        stdout_path: run_dir.join(format!("runner_stdout.{request_id}.txt")),
        stderr_path: run_dir.join(format!("runner_stderr.{request_id}.txt")),
        event_log_path: run_dir.join(format!("runner_events.{request_id}.jsonl")),
        invocation_path: run_dir.join(format!("runner_invocation.{request_id}.json")),
        completion_path: run_dir.join(format!("runner_completion.{request_id}.json")),
    }
}

/// Stage runner adapter that invokes Pi RPC.
pub struct PiRpcRunnerAdapter {
    config: PiRpcConfig,
    workspace_root: PathBuf,
    client_factory: Box<dyn PiRpcClientFactory + Send + Sync>,
}

impl std::fmt::Debug for PiRpcRunnerAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PiRpcRunnerAdapter")
            .field("config", &self.config)
            .field("workspace_root", &self.workspace_root)
            .finish_non_exhaustive()
    }
}

impl PiRpcRunnerAdapter {
    /// Builds a Pi RPC adapter backed by a real subprocess JSONL client.
    #[must_use]
    pub fn new(config: PiRpcConfig, workspace_root: impl Into<PathBuf>) -> Self {
        Self::with_client_factory(config, workspace_root, SubprocessPiRpcClientFactory::new())
    }

    /// Builds a Pi RPC adapter with an injected client factory.
    #[must_use]
    pub fn with_client_factory<F>(
        config: PiRpcConfig,
        workspace_root: impl Into<PathBuf>,
        client_factory: F,
    ) -> Self
    where
        F: PiRpcClientFactory + Send + Sync + 'static,
    {
        Self {
            config,
            workspace_root: normalize_workspace_root(workspace_root.into()),
            client_factory: Box::new(client_factory),
        }
    }

    /// Returns the adapter name used by the runner registry.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        "pi_rpc"
    }

    /// Returns the configured workspace root.
    #[must_use]
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Builds the exact Pi RPC argv for a stage request.
    pub fn build_command(&self, request: &StageRunRequest) -> RunnerResult<Vec<String>> {
        build_pi_rpc_command(&self.config, request)
    }

    /// Reports Pi RPC support for one execution capability grant.
    #[must_use]
    pub fn evaluate_capability_grant(
        &self,
        grant: &ExecutionCapabilityGrant,
        request: &StageRunRequest,
    ) -> CapabilitySupportDecision {
        pi_rpc_capability_support_decision(self.name(), grant, request)
    }
}

impl StageRunnerAdapter for PiRpcRunnerAdapter {
    fn evaluate_capability_grant(
        &self,
        grant: &ExecutionCapabilityGrant,
        request: &StageRunRequest,
    ) -> CapabilitySupportDecision {
        self.evaluate_capability_grant(grant, request)
    }

    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        let request = self.request_with_capability_support(request);
        let request = &request;
        self.config.validate()?;
        let emitted_at = now_timestamp("emitted_at")?;
        let run_dir = Path::new(&request.run_dir);
        fs::create_dir_all(run_dir).map_err(|error| RunnerError::Io {
            path: run_dir.display().to_string(),
            message: error.to_string(),
        })?;
        let artifact_paths = pi_rpc_artifact_paths(run_dir, &request.request_id);

        let prompt = build_stage_prompt(request);
        write_text(&artifact_paths.prompt_path, &prompt)?;
        let command = self.build_command(request)?;
        let environment_delta = RunnerEnvironmentDelta {
            set: self.config.env.clone(),
            unset: Vec::new(),
        };

        let mut invocation = invocation_artifact_from_request(
            request,
            self.name(),
            command.clone(),
            self.workspace_root.display().to_string(),
            environment_delta.clone(),
            artifact_paths.prompt_path.display().to_string(),
            emitted_at.clone(),
        )?;
        invocation.stdout_path = Some(artifact_paths.stdout_path.display().to_string());
        invocation.stderr_path = Some(artifact_paths.stderr_path.display().to_string());
        invocation.event_log_path = Some(artifact_paths.event_log_path.display().to_string());
        invocation.validate()?;
        write_runner_invocation(&artifact_paths.invocation_path, &invocation)?;

        let create_request = PiRpcClientCreateRequest {
            command: command.clone(),
            cwd: self.workspace_root.clone(),
            environment_delta: environment_delta.clone(),
        };
        let mut client = match self.client_factory.create(create_request) {
            Ok(client) => client,
            Err(error) => {
                return self.error_result(
                    request,
                    &artifact_paths,
                    command,
                    environment_delta,
                    emitted_at,
                    error,
                );
            }
        };
        let session_result = match client.run_prompt(&prompt, effective_timeout_seconds(request)) {
            Ok(result) => result,
            Err(error) => {
                return self.error_result(
                    request,
                    &artifact_paths,
                    command,
                    environment_delta,
                    emitted_at,
                    error,
                );
            }
        };

        self.session_result(
            request,
            &artifact_paths,
            command,
            environment_delta,
            session_result,
        )
    }
}

impl PiRpcRunnerAdapter {
    #[allow(clippy::too_many_arguments)]
    fn error_result(
        &self,
        request: &StageRunRequest,
        artifact_paths: &PiRpcArtifactPaths,
        command: Vec<String>,
        environment_delta: RunnerEnvironmentDelta,
        started_at: Timestamp,
        error: PiRpcClientError,
    ) -> RunnerResult<RunnerRawResult> {
        let (exit_code, failure_class, notes, stderr_text) = match error {
            PiRpcClientError::BinaryNotFound { binary } => (
                Some(127),
                Some("runner_binary_not_found".to_owned()),
                vec!["runner executable missing".to_owned()],
                format!("runner binary not found: {binary}\n"),
            ),
            PiRpcClientError::Transport { message } | PiRpcClientError::InvalidJson { message } => {
                (
                    Some(1),
                    Some("runner_transport_failure".to_owned()),
                    vec![message.clone()],
                    format!("runner process error: {message}\n"),
                )
            }
        };
        write_text(&artifact_paths.stderr_path, &stderr_text)?;
        let raw_result = RunnerRawResult {
            request_id: request.request_id.clone(),
            run_id: request.run_id.clone(),
            stage: request.stage,
            runner_name: self.name().to_owned(),
            model_name: request.model_name.clone(),
            thinking_level: request.thinking_level.clone(),
            model_reasoning_effort: request.model_reasoning_effort.clone(),
            exit_kind: RunnerExitKind::RunnerError,
            exit_code,
            observed_exit_kind: None,
            observed_exit_code: None,
            stdout_path: None,
            stderr_path: Some(artifact_paths.stderr_path.display().to_string()),
            terminal_result_path: None,
            event_log_path: None,
            token_usage: None,
            failure_capability_class: failure_capability_class_for_request(request),
            capability_support_decisions: request.capability_support_decisions.clone(),
            capability_evidence_refs: capability_evidence_refs_for_request(request),
            missing_capability_evidence_refs: missing_capability_evidence_refs_for_request(request),
            started_at,
            ended_at: now_timestamp("ended_at")?,
        };
        raw_result.validate()?;
        write_completion(
            request,
            self.name(),
            &raw_result,
            command,
            &self.workspace_root,
            environment_delta,
            Some(artifact_paths.prompt_path.display().to_string()),
            &artifact_paths.completion_path,
            failure_class,
            notes,
        )?;
        Ok(raw_result)
    }

    #[allow(clippy::too_many_arguments)]
    fn session_result(
        &self,
        request: &StageRunRequest,
        artifact_paths: &PiRpcArtifactPaths,
        command: Vec<String>,
        environment_delta: RunnerEnvironmentDelta,
        session_result: PiRpcSessionResult,
    ) -> RunnerResult<RunnerRawResult> {
        let persisted_event_lines = persistable_event_lines(&session_result.event_lines);
        let event_log_path = if should_persist_event_log(
            self.config.event_log_policy,
            &session_result,
            &persisted_event_lines,
        ) {
            write_lines(&artifact_paths.event_log_path, &persisted_event_lines)?;
            Some(artifact_paths.event_log_path.display().to_string())
        } else {
            None
        };

        if let Some(assistant_text) = session_result.assistant_text.as_ref() {
            write_text(&artifact_paths.stdout_path, assistant_text)?;
        }
        write_text(&artifact_paths.stderr_path, &session_result.stderr_text)?;

        let stdout_path = artifact_paths
            .stdout_path
            .exists()
            .then(|| artifact_paths.stdout_path.display().to_string());
        let stderr_path = Some(artifact_paths.stderr_path.display().to_string());
        let raw_result = RunnerRawResult {
            request_id: request.request_id.clone(),
            run_id: request.run_id.clone(),
            stage: request.stage,
            runner_name: self.name().to_owned(),
            model_name: request.model_name.clone(),
            thinking_level: request.thinking_level.clone(),
            model_reasoning_effort: request.model_reasoning_effort.clone(),
            exit_kind: session_result.exit_kind,
            exit_code: session_result.exit_code,
            observed_exit_kind: session_result.observed_exit_kind,
            observed_exit_code: session_result.observed_exit_code,
            stdout_path,
            stderr_path,
            terminal_result_path: None,
            event_log_path,
            token_usage: session_result.token_usage,
            failure_capability_class: failure_capability_class_for_request(request),
            capability_support_decisions: request.capability_support_decisions.clone(),
            capability_evidence_refs: capability_evidence_refs_for_request(request),
            missing_capability_evidence_refs: missing_capability_evidence_refs_for_request(request),
            started_at: session_result.started_at,
            ended_at: session_result.ended_at,
        };
        raw_result.validate()?;
        write_completion(
            request,
            self.name(),
            &raw_result,
            command,
            &self.workspace_root,
            environment_delta,
            Some(artifact_paths.prompt_path.display().to_string()),
            &artifact_paths.completion_path,
            session_result.failure_class,
            session_result.notes,
        )?;
        Ok(raw_result)
    }
}

/// Builds Python-compatible Pi RPC argv.
pub fn build_pi_rpc_command(
    config: &PiRpcConfig,
    request: &StageRunRequest,
) -> RunnerResult<Vec<String>> {
    config.validate()?;
    let mut command = Vec::new();
    command.push(config.command.clone());
    command.extend(config.args.iter().cloned());
    command.extend(["--mode".to_owned(), "rpc".to_owned()]);
    command.push("--no-session".to_owned());
    if let Some(provider) = config.provider.as_ref() {
        command.extend(["--provider".to_owned(), provider.clone()]);
    }
    if let Some(model_name) = request.model_name.as_ref() {
        command.extend(["--model".to_owned(), model_name.clone()]);
    }
    let thinking = request.thinking_level.as_ref().or(config.thinking.as_ref());
    if let Some(thinking) = thinking {
        command.extend(["--thinking".to_owned(), thinking.clone()]);
    }
    if config.disable_context_files {
        command.push("--no-context-files".to_owned());
    }
    if config.disable_skills {
        command.push("--no-skills".to_owned());
    }
    Ok(command)
}

/// Filters out noisy Pi `message_update` events before persistence.
#[must_use]
pub fn persistable_event_lines(event_lines: &[String]) -> Vec<String> {
    event_lines
        .iter()
        .filter(|line| !is_message_update_event(line))
        .cloned()
        .collect()
}

/// Returns true when a Pi event log should be persisted.
#[must_use]
pub fn should_persist_event_log(
    policy: PiEventLogPolicy,
    session_result: &PiRpcSessionResult,
    persisted_event_lines: &[String],
) -> bool {
    if persisted_event_lines.is_empty() {
        return false;
    }
    policy == PiEventLogPolicy::Full || session_result.exit_kind != RunnerExitKind::Completed
}

fn pi_rpc_capability_support_decision(
    runner_id: &str,
    grant: &ExecutionCapabilityGrant,
    request: &StageRunRequest,
) -> CapabilitySupportDecision {
    if grant.decision_state != CapabilityDecisionState::Granted {
        return CapabilitySupportDecision {
            runner_id: runner_id.to_owned(),
            invocation_context_ref: request.stage.as_str().to_owned(),
            grant_id: grant.grant_id.clone(),
            support_state: CapabilitySupportState::Unsupported,
            enforcement_mode: CapabilityEnforcementMode::NotApplicable,
            limitations: Vec::new(),
            evidence_available: Vec::new(),
            reason: format!("grant decision is {}", grant.decision_state.as_str()),
        };
    }
    if matches!(
        grant.capability_id.as_str(),
        "runner.invoke" | "artifact.read" | "artifact.write" | "evidence.emit"
    ) {
        return CapabilitySupportDecision {
            runner_id: runner_id.to_owned(),
            invocation_context_ref: request.stage.as_str().to_owned(),
            grant_id: grant.grant_id.clone(),
            support_state: CapabilitySupportState::Supported,
            enforcement_mode: grant.enforcement_mode,
            limitations: Vec::new(),
            evidence_available: vec![
                "runner_invocation".to_owned(),
                "runner_completion".to_owned(),
            ],
            reason: "Millrace runtime records Pi RPC invocation and artifacts".to_owned(),
        };
    }
    CapabilitySupportDecision {
        runner_id: runner_id.to_owned(),
        invocation_context_ref: request.stage.as_str().to_owned(),
        grant_id: grant.grant_id.clone(),
        support_state: CapabilitySupportState::PartiallySupported,
        enforcement_mode: CapabilityEnforcementMode::AdvisoryOnly,
        limitations: vec![
            "Pi RPC cannot prove filesystem or network enforcement boundaries".to_owned(),
        ],
        evidence_available: vec![
            "runner_invocation".to_owned(),
            "runner_completion".to_owned(),
        ],
        reason: "Pi RPC support is advisory for this capability".to_owned(),
    }
}

fn failure_capability_class_for_request(request: &StageRunRequest) -> Option<String> {
    (!missing_capability_evidence_refs_for_request(request).is_empty())
        .then(|| "capability_evidence_missing".to_owned())
}

fn is_message_update_event(raw_line: &str) -> bool {
    serde_json::from_str::<Value>(raw_line)
        .ok()
        .and_then(|payload| {
            payload
                .as_object()
                .and_then(|object| object.get("type"))
                .and_then(Value::as_str)
                .map(|event_type| event_type == "message_update")
        })
        .unwrap_or(false)
}

fn reserved_arg_conflicts(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|arg| {
            PI_RESERVED_ARGS.iter().any(|reserved| {
                arg.as_str() == *reserved || arg.starts_with(&format!("{reserved}="))
            })
        })
        .cloned()
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn write_completion(
    request: &StageRunRequest,
    runner_name: &str,
    raw_result: &RunnerRawResult,
    command: Vec<String>,
    cwd: &Path,
    environment_delta: RunnerEnvironmentDelta,
    prompt_path: Option<String>,
    completion_path: &Path,
    failure_class: Option<String>,
    notes: Vec<String>,
) -> RunnerResult<()> {
    let emitted_at = now_timestamp("emitted_at")?;
    let context = RunnerCompletionArtifactContext::new(
        runner_name,
        command,
        cwd.display().to_string(),
        environment_delta,
        prompt_path,
        emitted_at,
    )
    .with_failure_class(failure_class)
    .with_notes(notes);
    let artifact = completion_artifact_from_raw_result(request, raw_result, context)?;
    write_runner_completion(completion_path, &artifact)
}

fn write_text(path: &Path, text: &str) -> RunnerResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| RunnerError::Io {
            path: parent.display().to_string(),
            message: error.to_string(),
        })?;
    }
    fs::write(path, text).map_err(|error| RunnerError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

fn write_lines(path: &Path, lines: &[String]) -> RunnerResult<()> {
    let mut payload = lines.join("\n");
    payload.push('\n');
    write_text(path, &payload)
}

fn effective_timeout_seconds(request: &StageRunRequest) -> u64 {
    if request.timeout_seconds == 0 {
        3600
    } else {
        request.timeout_seconds
    }
}

fn normalize_workspace_root(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn now_timestamp(field_name: &'static str) -> RunnerResult<Timestamp> {
    let rendered = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| RunnerError::InvalidRawResult {
            message: format!("{field_name} could not be formatted: {error}"),
        })?;
    Timestamp::parse(field_name, &rendered).map_err(|error| RunnerError::InvalidRawResult {
        message: error.to_string(),
    })
}
