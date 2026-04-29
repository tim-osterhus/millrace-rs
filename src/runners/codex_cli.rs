//! Codex CLI runner adapter.

use std::{
    collections::BTreeMap,
    fs,
    fs::File,
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{contracts::Timestamp, runtime::StageRunRequest};

use super::{
    ProcessExecutionResult, ProcessExitKind, RunnerEnvironmentDelta, RunnerError, RunnerExitKind,
    RunnerRawResult, RunnerResult, StageRunnerAdapter,
    artifacts::{
        completion_artifact_from_raw_result, invocation_artifact_from_request,
        write_runner_completion, write_runner_invocation,
    },
    codex_cli_artifacts::{
        codex_cli_artifact_paths, materialize_stdout_artifact, persist_event_log,
        reconciled_timeout_terminal_marker,
    },
    codex_cli_tokens::extract_token_usage,
    process::duration_seconds_between,
    prompting::build_stage_prompt,
};

/// Python-compatible Codex permission levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexPermissionLevel {
    /// Maps to `--full-auto`.
    Basic,
    /// Maps to `-c approval_policy="never" --sandbox danger-full-access`.
    Elevated,
    /// Maps to `--dangerously-bypass-approvals-and-sandbox`.
    Maximum,
}

impl CodexPermissionLevel {
    /// Returns the stable serialized value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Elevated => "elevated",
            Self::Maximum => "maximum",
        }
    }
}

/// Runtime Codex CLI settings consumed by the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexCliConfig {
    pub command: String,
    pub args: Vec<String>,
    pub profile: Option<String>,
    pub permission_default: CodexPermissionLevel,
    pub permission_by_stage: BTreeMap<String, CodexPermissionLevel>,
    pub permission_by_model: BTreeMap<String, CodexPermissionLevel>,
    pub model_reasoning_effort: Option<String>,
    pub skip_git_repo_check: bool,
    pub extra_config: Vec<String>,
    pub env: BTreeMap<String, String>,
}

impl Default for CodexCliConfig {
    fn default() -> Self {
        Self {
            command: "codex".to_owned(),
            args: vec!["exec".to_owned()],
            profile: None,
            permission_default: CodexPermissionLevel::Maximum,
            permission_by_stage: BTreeMap::new(),
            permission_by_model: BTreeMap::new(),
            model_reasoning_effort: None,
            skip_git_repo_check: true,
            extra_config: Vec::new(),
            env: BTreeMap::new(),
        }
    }
}

/// One Codex subprocess request handed to the executor seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexProcessRequest {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub environment_delta: RunnerEnvironmentDelta,
    pub timeout_seconds: u64,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
}

/// Error returned by a Codex subprocess executor before a process result exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexProcessError {
    /// The executable was missing.
    BinaryNotFound { binary: String },
    /// Process transport failed before a normal result could be returned.
    Transport { message: String },
}

impl std::fmt::Display for CodexProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BinaryNotFound { binary } => write!(f, "runner binary not found: {binary}"),
            Self::Transport { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for CodexProcessError {}

/// Injectable process executor for always-on Codex adapter tests.
pub trait CodexProcessExecutor {
    /// Executes one Codex process request.
    fn execute(
        &self,
        request: &CodexProcessRequest,
    ) -> Result<ProcessExecutionResult, CodexProcessError>;
}

/// Real subprocess-backed Codex executor.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubprocessCodexExecutor;

impl CodexProcessExecutor for SubprocessCodexExecutor {
    fn execute(
        &self,
        request: &CodexProcessRequest,
    ) -> Result<ProcessExecutionResult, CodexProcessError> {
        run_codex_subprocess(request)
    }
}

/// Stage runner adapter that invokes Codex CLI.
pub struct CodexCliRunnerAdapter {
    config: CodexCliConfig,
    workspace_root: PathBuf,
    process_executor: Box<dyn CodexProcessExecutor>,
}

impl std::fmt::Debug for CodexCliRunnerAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexCliRunnerAdapter")
            .field("config", &self.config)
            .field("workspace_root", &self.workspace_root)
            .finish_non_exhaustive()
    }
}

impl CodexCliRunnerAdapter {
    /// Builds a Codex adapter backed by the real subprocess executor.
    #[must_use]
    pub fn new(config: CodexCliConfig, workspace_root: impl Into<PathBuf>) -> Self {
        Self::with_process_executor(config, workspace_root, SubprocessCodexExecutor)
    }

    /// Builds a Codex adapter with an injected process executor.
    #[must_use]
    pub fn with_process_executor<E>(
        config: CodexCliConfig,
        workspace_root: impl Into<PathBuf>,
        process_executor: E,
    ) -> Self
    where
        E: CodexProcessExecutor + 'static,
    {
        let workspace_root = normalize_workspace_root(workspace_root.into());
        Self {
            config,
            workspace_root,
            process_executor: Box::new(process_executor),
        }
    }

    /// Returns the adapter name used by the runner registry.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        "codex_cli"
    }

    /// Returns the configured workspace root.
    #[must_use]
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Builds the exact Codex CLI argv for a stage request.
    pub fn build_command(
        &self,
        request: &StageRunRequest,
        prompt: &str,
        output_last_message_path: &Path,
    ) -> Vec<String> {
        build_codex_cli_command(
            &self.config,
            &self.workspace_root,
            request,
            prompt,
            output_last_message_path,
        )
    }
}

impl StageRunnerAdapter for CodexCliRunnerAdapter {
    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        let emitted_at = now_timestamp("emitted_at")?;
        let run_dir = Path::new(&request.run_dir);
        fs::create_dir_all(run_dir).map_err(|error| RunnerError::Io {
            path: run_dir.display().to_string(),
            message: error.to_string(),
        })?;
        let artifact_paths = codex_cli_artifact_paths(run_dir, &request.request_id);

        let prompt = build_stage_prompt(request);
        write_text(&artifact_paths.prompt_path, &prompt)?;

        let command =
            self.build_command(request, &prompt, &artifact_paths.output_last_message_path);
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

        let process_request = CodexProcessRequest {
            command: command.clone(),
            cwd: self.workspace_root.clone(),
            environment_delta: environment_delta.clone(),
            timeout_seconds: effective_timeout_seconds(request.timeout_seconds),
            stdout_path: artifact_paths.stdout_path.clone(),
            stderr_path: artifact_paths.stderr_path.clone(),
        };

        let process_result = match self.process_executor.execute(&process_request) {
            Ok(result) => result,
            Err(CodexProcessError::BinaryNotFound { binary }) => {
                write_text(
                    &artifact_paths.stderr_path,
                    &format!("runner binary not found: {binary}\n"),
                )?;
                let ended_at = now_timestamp("ended_at")?;
                let raw_result = RunnerRawResult {
                    request_id: request.request_id.clone(),
                    run_id: request.run_id.clone(),
                    stage: request.stage,
                    runner_name: self.name().to_owned(),
                    model_name: request.model_name.clone(),
                    model_reasoning_effort: request.model_reasoning_effort.clone(),
                    exit_kind: RunnerExitKind::RunnerError,
                    exit_code: Some(127),
                    observed_exit_kind: None,
                    observed_exit_code: None,
                    stdout_path: Some(artifact_paths.stdout_path.display().to_string()),
                    stderr_path: Some(artifact_paths.stderr_path.display().to_string()),
                    terminal_result_path: None,
                    event_log_path: None,
                    token_usage: None,
                    started_at: emitted_at,
                    ended_at,
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
                    Some("runner_binary_not_found".to_owned()),
                    vec!["runner executable missing".to_owned()],
                )?;
                return Ok(raw_result);
            }
            Err(CodexProcessError::Transport { message }) => {
                let started_at = now_timestamp("started_at")?;
                let ended_at = now_timestamp("ended_at")?;
                let result = process_result(
                    &process_request,
                    ProcessExitKind::TransportError,
                    Some(1),
                    started_at,
                    ended_at,
                    Some(message),
                )?;
                result
            }
        };

        let process_exit_kind = runner_exit_kind_for_process(&process_result);
        let persisted_event_log_path =
            persist_event_log(&artifact_paths.stdout_path, &artifact_paths.event_log_path)?;
        let token_usage = extract_token_usage(persisted_event_log_path.as_deref());
        let materialized_stdout_path = materialize_stdout_artifact(
            &artifact_paths.stdout_path,
            &artifact_paths.output_last_message_path,
            persisted_event_log_path.as_deref(),
        )?;

        let reconciled_marker = if process_exit_kind == RunnerExitKind::Timeout {
            reconciled_timeout_terminal_marker(request, &artifact_paths.output_last_message_path)
        } else {
            None
        };

        let observed_exit_kind = reconciled_marker.as_ref().map(|_| process_exit_kind);
        let observed_exit_code = reconciled_marker.as_ref().and(process_result.exit_code);
        let (exit_kind, exit_code) = if reconciled_marker.is_some() {
            (RunnerExitKind::Completed, Some(0))
        } else {
            (process_exit_kind, process_result.exit_code)
        };

        let raw_result = RunnerRawResult {
            request_id: request.request_id.clone(),
            run_id: request.run_id.clone(),
            stage: request.stage,
            runner_name: self.name().to_owned(),
            model_name: request.model_name.clone(),
            model_reasoning_effort: request.model_reasoning_effort.clone(),
            exit_kind,
            exit_code,
            observed_exit_kind,
            observed_exit_code,
            stdout_path: materialized_stdout_path
                .as_ref()
                .map(|path| path.display().to_string()),
            stderr_path: Some(artifact_paths.stderr_path.display().to_string()),
            terminal_result_path: None,
            event_log_path: persisted_event_log_path
                .as_ref()
                .map(|path| path.display().to_string()),
            token_usage,
            started_at: process_result.started_at.clone(),
            ended_at: process_result.ended_at.clone(),
        };
        raw_result.validate()?;

        let (failure_class, notes) =
            completion_failure_evidence(&process_result, process_exit_kind, reconciled_marker);
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
}

/// Builds Python-compatible Codex CLI argv.
#[must_use]
pub fn build_codex_cli_command(
    config: &CodexCliConfig,
    workspace_root: &Path,
    request: &StageRunRequest,
    prompt: &str,
    output_last_message_path: &Path,
) -> Vec<String> {
    let mut command = Vec::new();
    command.push(config.command.clone());
    command.extend(config.args.iter().cloned());
    if let Some(profile) = config.profile.as_ref() {
        command.extend(["--profile".to_owned(), profile.clone()]);
    }
    if config.skip_git_repo_check {
        command.push("--skip-git-repo-check".to_owned());
    }
    if let Some(model_name) = request.model_name.as_ref() {
        command.extend(["--model".to_owned(), model_name.clone()]);
    }
    command.extend(
        permission_flags(resolve_permission_level(config, request))
            .into_iter()
            .map(|value| (*value).to_owned()),
    );
    for item in &config.extra_config {
        command.extend(["-c".to_owned(), item.clone()]);
    }
    let model_reasoning_effort = request
        .model_reasoning_effort
        .as_ref()
        .or(config.model_reasoning_effort.as_ref());
    if let Some(model_reasoning_effort) = model_reasoning_effort {
        command.extend([
            "-c".to_owned(),
            format!("model_reasoning_effort=\"{model_reasoning_effort}\""),
        ]);
    }
    command.extend(["--cd".to_owned(), workspace_root.display().to_string()]);
    command.push("--json".to_owned());
    command.extend([
        "--output-last-message".to_owned(),
        output_last_message_path.display().to_string(),
    ]);
    command.push(prompt.to_owned());
    command
}

/// Resolves Codex permission level using stage, then model, then default precedence.
#[must_use]
pub fn resolve_permission_level(
    config: &CodexCliConfig,
    request: &StageRunRequest,
) -> CodexPermissionLevel {
    if let Some(level) = config.permission_by_stage.get(request.stage.as_str()) {
        return *level;
    }
    if let Some(model_name) = request.model_name.as_ref() {
        if let Some(level) = config.permission_by_model.get(model_name) {
            return *level;
        }
    }
    config.permission_default
}

/// Returns Codex CLI flags for one permission level.
#[must_use]
pub const fn permission_flags(level: CodexPermissionLevel) -> &'static [&'static str] {
    match level {
        CodexPermissionLevel::Basic => &["--full-auto"],
        CodexPermissionLevel::Elevated => &[
            "-c",
            "approval_policy=\"never\"",
            "--sandbox",
            "danger-full-access",
        ],
        CodexPermissionLevel::Maximum => &["--dangerously-bypass-approvals-and-sandbox"],
    }
}

fn runner_exit_kind_for_process(result: &ProcessExecutionResult) -> RunnerExitKind {
    if result.timed_out || result.exit_kind == ProcessExitKind::Timeout {
        RunnerExitKind::Timeout
    } else if result.transport_error.is_some()
        || result.exit_kind == ProcessExitKind::TransportError
    {
        RunnerExitKind::RunnerError
    } else if result.exit_code.is_some_and(|code| code != 0) {
        RunnerExitKind::RunnerError
    } else {
        RunnerExitKind::Completed
    }
}

fn completion_failure_evidence(
    process_result: &ProcessExecutionResult,
    process_exit_kind: RunnerExitKind,
    reconciled_marker: Option<String>,
) -> (Option<String>, Vec<String>) {
    if let Some(marker) = reconciled_marker {
        return (
            None,
            vec![format!(
                "runner timeout reconciled after final terminal marker ### {marker}"
            )],
        );
    }
    if process_exit_kind == RunnerExitKind::Timeout {
        return (
            Some("runner_timeout".to_owned()),
            vec!["runner process exceeded timeout".to_owned()],
        );
    }
    if let Some(error) = process_result.transport_error.as_ref() {
        return (
            Some("runner_transport_failure".to_owned()),
            vec![error.clone()],
        );
    }
    if process_result.exit_code.is_some_and(|code| code != 0) {
        return (
            Some("runner_non_zero_exit".to_owned()),
            vec!["runner exited with non-zero status".to_owned()],
        );
    }
    (None, Vec::new())
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
    let artifact = completion_artifact_from_raw_result(
        request,
        runner_name,
        raw_result,
        command,
        cwd.display().to_string(),
        environment_delta,
        prompt_path,
        emitted_at,
        failure_class,
        notes,
    )?;
    write_runner_completion(completion_path, &artifact)
}

fn run_codex_subprocess(
    request: &CodexProcessRequest,
) -> Result<ProcessExecutionResult, CodexProcessError> {
    let started_at = now_timestamp("started_at").map_err(|error| CodexProcessError::Transport {
        message: error.to_string(),
    })?;
    create_parent(&request.stdout_path)?;
    create_parent(&request.stderr_path)?;
    let stdout_file =
        File::create(&request.stdout_path).map_err(|error| CodexProcessError::Transport {
            message: format!(
                "failed to create stdout artifact {}: {error}",
                request.stdout_path.display()
            ),
        })?;
    let stderr_file =
        File::create(&request.stderr_path).map_err(|error| CodexProcessError::Transport {
            message: format!(
                "failed to create stderr artifact {}: {error}",
                request.stderr_path.display()
            ),
        })?;
    let Some((binary, args)) = request.command.split_first() else {
        return Err(CodexProcessError::Transport {
            message: "codex command cannot be empty".to_owned(),
        });
    };

    let mut command = Command::new(binary);
    command
        .args(args)
        .current_dir(&request.cwd)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));
    for (key, value) in &request.environment_delta.set {
        command.env(key, value);
    }
    for key in &request.environment_delta.unset {
        command.env_remove(key);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(CodexProcessError::BinaryNotFound {
                binary: binary.clone(),
            });
        }
        Err(error) => {
            let ended_at = now_timestamp("ended_at").map_err(|timestamp_error| {
                CodexProcessError::Transport {
                    message: timestamp_error.to_string(),
                }
            })?;
            let _ = fs::write(
                &request.stderr_path,
                format!("runner process error: {error}\n"),
            );
            return process_result(
                request,
                ProcessExitKind::TransportError,
                Some(1),
                started_at,
                ended_at,
                Some(error.to_string()),
            )
            .map_err(|result_error| CodexProcessError::Transport {
                message: result_error.to_string(),
            });
        }
    };

    let timeout = Duration::from_secs(request.timeout_seconds.max(1));
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let ended_at =
                    now_timestamp("ended_at").map_err(|error| CodexProcessError::Transport {
                        message: error.to_string(),
                    })?;
                return process_result(
                    request,
                    ProcessExitKind::Completed,
                    status.code(),
                    started_at,
                    ended_at,
                    None,
                )
                .map_err(|error| CodexProcessError::Transport {
                    message: error.to_string(),
                });
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::write(
                    &request.stderr_path,
                    format!(
                        "runner timed out after {} seconds\n",
                        request.timeout_seconds
                    ),
                );
                let ended_at =
                    now_timestamp("ended_at").map_err(|error| CodexProcessError::Transport {
                        message: error.to_string(),
                    })?;
                return process_result(
                    request,
                    ProcessExitKind::Timeout,
                    Some(124),
                    started_at,
                    ended_at,
                    None,
                )
                .map_err(|error| CodexProcessError::Transport {
                    message: error.to_string(),
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                let ended_at = now_timestamp("ended_at").map_err(|timestamp_error| {
                    CodexProcessError::Transport {
                        message: timestamp_error.to_string(),
                    }
                })?;
                let _ = fs::write(
                    &request.stderr_path,
                    format!("runner process error: {error}\n"),
                );
                return process_result(
                    request,
                    ProcessExitKind::TransportError,
                    Some(1),
                    started_at,
                    ended_at,
                    Some(error.to_string()),
                )
                .map_err(|result_error| CodexProcessError::Transport {
                    message: result_error.to_string(),
                });
            }
        }
    }
}

fn process_result(
    request: &CodexProcessRequest,
    exit_kind: ProcessExitKind,
    exit_code: Option<i32>,
    started_at: Timestamp,
    ended_at: Timestamp,
    transport_error: Option<String>,
) -> RunnerResult<ProcessExecutionResult> {
    let duration_seconds = duration_seconds_between(&started_at, &ended_at)?;
    let mut result = ProcessExecutionResult {
        schema_version: "1.0".to_owned(),
        kind: "process_execution_result".to_owned(),
        command: request.command.clone(),
        cwd: request.cwd.display().to_string(),
        environment_delta: request.environment_delta.clone(),
        stdout_path: None,
        stderr_path: None,
        event_log_path: None,
        exit_kind,
        exit_code,
        timed_out: exit_kind == ProcessExitKind::Timeout,
        interrupted: exit_kind == ProcessExitKind::Interrupted,
        killed: exit_kind == ProcessExitKind::Killed,
        transport_error,
        started_at,
        ended_at,
        duration_seconds,
        notes: Vec::new(),
    };
    result.stdout_path = Some(request.stdout_path.display().to_string());
    result.stderr_path = Some(request.stderr_path.display().to_string());
    if result.exit_kind == ProcessExitKind::Timeout {
        result
            .notes
            .push("runner process exceeded timeout".to_owned());
    }
    result.validate()?;
    Ok(result)
}

fn create_parent(path: &Path) -> Result<(), CodexProcessError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| CodexProcessError::Transport {
            message: format!("failed to create {}: {error}", parent.display()),
        })?;
    }
    Ok(())
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

fn effective_timeout_seconds(timeout_seconds: u64) -> u64 {
    if timeout_seconds == 0 {
        3600
    } else {
        timeout_seconds
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
