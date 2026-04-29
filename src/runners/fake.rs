use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::contracts::{ResultClass, StageName, Timestamp, TokenUsage};
use crate::runtime::StageRunRequest;

use super::{
    RunnerEnvironmentDelta, RunnerError, RunnerExitKind, RunnerRawResult, RunnerResult,
    StageRunnerAdapter, completion_artifact_from_raw_result, invocation_artifact_from_request,
    write_runner_completion, write_runner_invocation, write_stage_prompt_artifact,
};

const DEFAULT_FAKE_START: &str = "2026-04-15T00:00:00Z";
const DEFAULT_FAKE_END: &str = "2026-04-15T00:00:01Z";

/// Deterministic fake-runner configuration for tests and serial runtime slices.
#[derive(Debug, Clone)]
pub struct FakeRunnerConfig {
    /// Runner name recorded in raw results when the request does not specify one.
    pub runner_name: String,
    /// Fixed start timestamp used for every fake result.
    pub fixed_started_at: Timestamp,
    /// Fixed end timestamp used for every fake result.
    pub fixed_ended_at: Timestamp,
    /// Default result when no mapping matches.
    pub default_result: FakeRunnerResult,
    /// Highest-precedence mapping by request id.
    pub by_request_id: BTreeMap<String, FakeRunnerResult>,
    /// Second-precedence mapping by compiled graph node id.
    pub by_node_id: BTreeMap<String, FakeRunnerResult>,
    /// Third-precedence mapping by stage value.
    pub by_stage: BTreeMap<String, FakeRunnerResult>,
}

impl FakeRunnerConfig {
    /// Builds a deterministic fake-runner config.
    pub fn new(default_result: FakeRunnerResult) -> RunnerResult<Self> {
        Ok(Self {
            runner_name: "fake_runner".to_owned(),
            fixed_started_at: parse_fake_timestamp(DEFAULT_FAKE_START)?,
            fixed_ended_at: parse_fake_timestamp(DEFAULT_FAKE_END)?,
            default_result,
            by_request_id: BTreeMap::new(),
            by_node_id: BTreeMap::new(),
            by_stage: BTreeMap::new(),
        })
    }

    /// Adds or replaces a request-id mapping.
    #[must_use]
    pub fn with_request_result(
        mut self,
        request_id: impl Into<String>,
        result: FakeRunnerResult,
    ) -> Self {
        self.by_request_id.insert(request_id.into(), result);
        self
    }

    /// Adds or replaces a node-id mapping.
    #[must_use]
    pub fn with_node_result(
        mut self,
        node_id: impl Into<String>,
        result: FakeRunnerResult,
    ) -> Self {
        self.by_node_id.insert(node_id.into(), result);
        self
    }

    /// Adds or replaces a stage mapping.
    #[must_use]
    pub fn with_stage_result(mut self, stage: StageName, result: FakeRunnerResult) -> Self {
        self.by_stage.insert(stage.as_str().to_owned(), result);
        self
    }
}

/// A deterministic fake stage runner.
#[derive(Debug, Clone)]
pub struct FakeRunner {
    config: FakeRunnerConfig,
}

impl FakeRunner {
    /// Builds a fake runner from explicit configuration.
    #[must_use]
    pub fn new(config: FakeRunnerConfig) -> Self {
        Self { config }
    }

    /// Builds a fake runner with a single default result.
    pub fn with_default(default_result: FakeRunnerResult) -> RunnerResult<Self> {
        Ok(Self::new(FakeRunnerConfig::new(default_result)?))
    }

    fn selected_result(&self, request: &StageRunRequest) -> &FakeRunnerResult {
        self.config
            .by_request_id
            .get(&request.request_id)
            .or_else(|| self.config.by_node_id.get(&request.node_id))
            .or_else(|| self.config.by_stage.get(request.stage.as_str()))
            .unwrap_or(&self.config.default_result)
    }
}

impl StageRunnerAdapter for FakeRunner {
    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        let result = self.selected_result(request);
        let run_dir = Path::new(&request.run_dir);
        fs::create_dir_all(run_dir).map_err(|error| RunnerError::Io {
            path: request.run_dir.clone(),
            message: error.to_string(),
        })?;
        let prompt_path = write_stage_prompt_artifact(request)?;
        let paths = fake_runner_artifact_paths(run_dir, &request.request_id);
        let runner_name = request
            .runner_name
            .clone()
            .unwrap_or_else(|| self.config.runner_name.clone());
        let command = vec![runner_name.clone(), "run-stage".to_owned()];
        let environment_delta = RunnerEnvironmentDelta::default();
        let mut invocation = invocation_artifact_from_request(
            request,
            runner_name.clone(),
            command.clone(),
            request.run_dir.clone(),
            environment_delta.clone(),
            prompt_path.display().to_string(),
            self.config.fixed_started_at.clone(),
        )?;
        invocation.stdout_path = Some(paths.stdout_path.display().to_string());
        invocation.stderr_path = Some(paths.stderr_path.display().to_string());
        invocation.event_log_path = Some(paths.event_log_path.display().to_string());
        invocation.validate()?;
        write_runner_invocation(&paths.invocation_path, &invocation)?;

        let (stdout_path, terminal_result_path) = write_fake_output(
            &paths.stdout_path,
            &paths.terminal_result_path,
            &result.output,
        )?;
        let stderr_path = write_optional_artifact(&paths.stderr_path, result.stderr.as_deref())?;
        let event_log_path =
            write_optional_artifact(&paths.event_log_path, result.event_log.as_deref())?;

        let raw_result = RunnerRawResult {
            request_id: request.request_id.clone(),
            run_id: request.run_id.clone(),
            stage: request.stage,
            runner_name: runner_name.clone(),
            model_name: request.model_name.clone(),
            model_reasoning_effort: request.model_reasoning_effort.clone(),
            exit_kind: result.exit_kind,
            exit_code: result.exit_code,
            observed_exit_kind: result.observed_exit_kind,
            observed_exit_code: result.observed_exit_code,
            stdout_path,
            stderr_path,
            terminal_result_path,
            event_log_path,
            token_usage: result.token_usage.clone(),
            started_at: self.config.fixed_started_at.clone(),
            ended_at: self.config.fixed_ended_at.clone(),
        };
        raw_result.validate()?;
        let completion = completion_artifact_from_raw_result(
            request,
            runner_name,
            &raw_result,
            command,
            request.run_dir.clone(),
            environment_delta,
            Some(prompt_path.display().to_string()),
            self.config.fixed_ended_at.clone(),
            fake_failure_class(raw_result.exit_kind).map(str::to_owned),
            vec!["deterministic fake runner result".to_owned()],
        )?;
        write_runner_completion(&paths.completion_path, &completion)?;
        Ok(raw_result)
    }
}

/// Fake-runner output mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeRunnerOutput {
    /// Writes stdout containing one terminal marker line.
    TerminalMarker(String),
    /// Writes stdout exactly as provided.
    Stdout(String),
    /// Produces no stdout and no structured terminal result path.
    MissingTerminalOutput,
    /// Writes a structured terminal-result JSON artifact.
    StructuredTerminalResult {
        terminal_result: String,
        result_class: Option<ResultClass>,
        summary_artifact_paths: Vec<String>,
    },
}

/// Fake-runner result template selected for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeRunnerResult {
    pub output: FakeRunnerOutput,
    pub exit_kind: RunnerExitKind,
    pub exit_code: Option<i32>,
    pub observed_exit_kind: Option<RunnerExitKind>,
    pub observed_exit_code: Option<i32>,
    pub stderr: Option<String>,
    pub event_log: Option<String>,
    pub token_usage: Option<TokenUsage>,
}

impl FakeRunnerResult {
    /// Returns a completed fake result whose stdout contains the given marker.
    #[must_use]
    pub fn terminal_marker(marker: impl Into<String>) -> Self {
        Self {
            output: FakeRunnerOutput::TerminalMarker(marker.into()),
            exit_kind: RunnerExitKind::Completed,
            exit_code: Some(0),
            observed_exit_kind: None,
            observed_exit_code: None,
            stderr: None,
            event_log: None,
            token_usage: None,
        }
    }

    /// Returns a completed fake result whose stdout is malformed for terminal extraction.
    #[must_use]
    pub fn malformed_stdout(stdout: impl Into<String>) -> Self {
        Self {
            output: FakeRunnerOutput::Stdout(stdout.into()),
            exit_kind: RunnerExitKind::Completed,
            exit_code: Some(0),
            observed_exit_kind: None,
            observed_exit_code: None,
            stderr: None,
            event_log: None,
            token_usage: None,
        }
    }

    /// Returns a completed fake result with no terminal output.
    #[must_use]
    pub fn missing_terminal_output() -> Self {
        Self {
            output: FakeRunnerOutput::MissingTerminalOutput,
            exit_kind: RunnerExitKind::Completed,
            exit_code: Some(0),
            observed_exit_kind: None,
            observed_exit_code: None,
            stderr: None,
            event_log: None,
            token_usage: None,
        }
    }

    /// Returns a completed fake result backed by structured terminal JSON.
    #[must_use]
    pub fn structured_terminal_result(
        terminal_result: impl Into<String>,
        result_class: Option<ResultClass>,
    ) -> Self {
        Self {
            output: FakeRunnerOutput::StructuredTerminalResult {
                terminal_result: terminal_result.into(),
                result_class,
                summary_artifact_paths: Vec::new(),
            },
            exit_kind: RunnerExitKind::Completed,
            exit_code: Some(0),
            observed_exit_kind: None,
            observed_exit_code: None,
            stderr: None,
            event_log: None,
            token_usage: None,
        }
    }

    /// Adds token usage to the fake result.
    #[must_use]
    pub fn with_token_usage(mut self, token_usage: TokenUsage) -> Self {
        self.token_usage = Some(token_usage);
        self
    }

    /// Overrides exit kind and exit code.
    #[must_use]
    pub fn with_exit(mut self, exit_kind: RunnerExitKind, exit_code: Option<i32>) -> Self {
        self.exit_kind = exit_kind;
        self.exit_code = exit_code;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FakeRunnerArtifactPaths {
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    event_log_path: PathBuf,
    terminal_result_path: PathBuf,
    invocation_path: PathBuf,
    completion_path: PathBuf,
}

fn fake_runner_artifact_paths(run_dir: &Path, request_id: &str) -> FakeRunnerArtifactPaths {
    FakeRunnerArtifactPaths {
        stdout_path: run_dir.join(format!("runner_stdout.{request_id}.txt")),
        stderr_path: run_dir.join(format!("runner_stderr.{request_id}.txt")),
        event_log_path: run_dir.join(format!("runner_events.{request_id}.jsonl")),
        terminal_result_path: run_dir.join(format!("runner_terminal_result.{request_id}.json")),
        invocation_path: run_dir.join(format!("runner_invocation.{request_id}.json")),
        completion_path: run_dir.join(format!("runner_completion.{request_id}.json")),
    }
}

fn write_fake_output(
    stdout_path: &Path,
    terminal_result_path: &Path,
    output: &FakeRunnerOutput,
) -> RunnerResult<(Option<String>, Option<String>)> {
    match output {
        FakeRunnerOutput::TerminalMarker(marker) => {
            write_text(stdout_path, &format!("fake runner output\n{marker}\n"))?;
            Ok((Some(stdout_path.display().to_string()), None))
        }
        FakeRunnerOutput::Stdout(stdout) => {
            write_text(stdout_path, stdout)?;
            Ok((Some(stdout_path.display().to_string()), None))
        }
        FakeRunnerOutput::MissingTerminalOutput => Ok((None, None)),
        FakeRunnerOutput::StructuredTerminalResult {
            terminal_result,
            result_class,
            summary_artifact_paths,
        } => {
            let payload = json!({
                "terminal_result": terminal_result,
                "result_class": result_class,
                "summary_artifact_paths": summary_artifact_paths,
            });
            write_text(
                terminal_result_path,
                &(serde_json::to_string_pretty(&payload).map_err(|error| RunnerError::Json {
                    artifact: "fake_terminal_result",
                    message: error.to_string(),
                })? + "\n"),
            )?;
            Ok((None, Some(terminal_result_path.display().to_string())))
        }
    }
}

fn write_optional_artifact(path: &Path, contents: Option<&str>) -> RunnerResult<Option<String>> {
    let Some(contents) = contents else {
        return Ok(None);
    };
    write_text(path, contents)?;
    Ok(Some(path.display().to_string()))
}

fn write_text(path: &Path, contents: &str) -> RunnerResult<()> {
    fs::write(path, contents).map_err(|error| RunnerError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

fn parse_fake_timestamp(value: &str) -> RunnerResult<Timestamp> {
    Timestamp::parse("timestamp", value).map_err(|error| RunnerError::InvalidRawResult {
        message: error.to_string(),
    })
}

fn fake_failure_class(exit_kind: RunnerExitKind) -> Option<&'static str> {
    match exit_kind {
        RunnerExitKind::Completed => None,
        RunnerExitKind::Timeout => Some("runner_timeout"),
        RunnerExitKind::ProviderError => Some("provider_failure"),
        RunnerExitKind::RunnerError | RunnerExitKind::Interrupted => {
            Some("runner_transport_failure")
        }
    }
}
