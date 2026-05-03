//! Runner invocation and completion artifact contracts.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    contracts::{StageName, Timestamp, TokenUsage},
    runtime::{RequestKind, StageRunRequest},
};

use super::{
    RunnerError, RunnerExitKind, RunnerRawResult, RunnerResult,
    process::{RunnerEnvironmentDelta, duration_seconds_between},
};

/// Runner invocation artifact persisted before a concrete adapter executes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerInvocationArtifact {
    pub schema_version: String,
    pub kind: String,
    pub request_id: String,
    pub run_id: String,
    pub stage: StageName,
    pub request_kind: RequestKind,
    pub active_work_item_id: Option<String>,
    pub closure_target_root_spec_id: Option<String>,
    pub runner_name: String,
    pub model_name: Option<String>,
    pub thinking_level: Option<String>,
    pub model_reasoning_effort: Option<String>,
    pub command: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub environment_delta: RunnerEnvironmentDelta,
    pub prompt_path: String,
    pub stdout_path: Option<String>,
    pub stderr_path: Option<String>,
    pub event_log_path: Option<String>,
    pub emitted_at: Timestamp,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl RunnerInvocationArtifact {
    /// Validates the runner invocation artifact contract.
    pub fn validate(&mut self) -> RunnerResult<()> {
        require_literal(
            "schema_version",
            &self.schema_version,
            "1.0",
            "runner_invocation",
        )?;
        require_literal("kind", &self.kind, "runner_invocation", "runner_invocation")?;
        require_non_blank("request_id", &self.request_id, "runner_invocation")?;
        require_non_blank("run_id", &self.run_id, "runner_invocation")?;
        require_non_blank("runner_name", &self.runner_name, "runner_invocation")?;
        require_non_blank("cwd", &self.cwd, "runner_invocation")?;
        require_non_blank("prompt_path", &self.prompt_path, "runner_invocation")?;
        require_command(&self.command, "runner_invocation")?;
        Ok(())
    }
}

/// Runner completion artifact persisted after adapter execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerCompletionArtifact {
    pub schema_version: String,
    pub kind: String,
    pub request_id: String,
    pub run_id: String,
    pub stage: StageName,
    pub request_kind: RequestKind,
    pub active_work_item_id: Option<String>,
    pub closure_target_root_spec_id: Option<String>,
    pub runner_name: String,
    pub model_name: Option<String>,
    pub thinking_level: Option<String>,
    pub model_reasoning_effort: Option<String>,
    pub command: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub environment_delta: RunnerEnvironmentDelta,
    pub prompt_path: Option<String>,
    pub exit_kind: RunnerExitKind,
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub interrupted: bool,
    #[serde(default)]
    pub killed: bool,
    pub transport_error: Option<String>,
    pub observed_exit_kind: Option<RunnerExitKind>,
    pub observed_exit_code: Option<i32>,
    pub stdout_path: Option<String>,
    pub stderr_path: Option<String>,
    pub terminal_result_path: Option<String>,
    pub event_log_path: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub started_at: Timestamp,
    pub ended_at: Timestamp,
    pub duration_seconds: f64,
    pub failure_class: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub emitted_at: Timestamp,
}

impl RunnerCompletionArtifact {
    /// Validates the runner completion artifact contract.
    pub fn validate(&mut self) -> RunnerResult<()> {
        require_literal(
            "schema_version",
            &self.schema_version,
            "1.0",
            "runner_completion",
        )?;
        require_literal("kind", &self.kind, "runner_completion", "runner_completion")?;
        require_non_blank("request_id", &self.request_id, "runner_completion")?;
        require_non_blank("run_id", &self.run_id, "runner_completion")?;
        require_non_blank("runner_name", &self.runner_name, "runner_completion")?;
        require_non_blank("cwd", &self.cwd, "runner_completion")?;
        require_command(&self.command, "runner_completion")?;
        let computed_duration = duration_seconds_between(&self.started_at, &self.ended_at)?;
        if (self.duration_seconds - computed_duration).abs() > 0.001 {
            return Err(RunnerError::InvalidRunnerArtifact {
                message: "runner_completion duration_seconds must match timestamps".to_owned(),
            });
        }
        if self.timed_out
            != (self.exit_kind == RunnerExitKind::Timeout
                || self.observed_exit_kind == Some(RunnerExitKind::Timeout))
        {
            return Err(RunnerError::InvalidRunnerArtifact {
                message: "runner_completion timed_out must match exit evidence".to_owned(),
            });
        }
        if self.interrupted
            != (self.exit_kind == RunnerExitKind::Interrupted
                || self.observed_exit_kind == Some(RunnerExitKind::Interrupted))
        {
            return Err(RunnerError::InvalidRunnerArtifact {
                message: "runner_completion interrupted must match exit evidence".to_owned(),
            });
        }
        Ok(())
    }
}

/// Input values that are owned by the runner boundary when building completion artifacts.
#[derive(Debug, Clone, PartialEq)]
pub struct RunnerCompletionArtifactContext {
    pub runner_name: String,
    pub command: Vec<String>,
    pub cwd: String,
    pub environment_delta: RunnerEnvironmentDelta,
    pub prompt_path: Option<String>,
    pub emitted_at: Timestamp,
    pub failure_class: Option<String>,
    pub notes: Vec<String>,
}

impl RunnerCompletionArtifactContext {
    /// Build a completion artifact context with no failure class or notes.
    #[must_use]
    pub fn new(
        runner_name: impl Into<String>,
        command: Vec<String>,
        cwd: impl Into<String>,
        environment_delta: RunnerEnvironmentDelta,
        prompt_path: Option<String>,
        emitted_at: Timestamp,
    ) -> Self {
        Self {
            runner_name: runner_name.into(),
            command,
            cwd: cwd.into(),
            environment_delta,
            prompt_path,
            emitted_at,
            failure_class: None,
            notes: Vec::new(),
        }
    }

    /// Attach the failure class recorded by the runner boundary.
    #[must_use]
    pub fn with_failure_class(mut self, failure_class: Option<String>) -> Self {
        self.failure_class = failure_class;
        self
    }

    /// Attach free-form runner notes.
    #[must_use]
    pub fn with_notes(mut self, notes: Vec<String>) -> Self {
        self.notes = notes;
        self
    }
}

/// Builds a runner invocation artifact from a stage request.
pub fn invocation_artifact_from_request(
    request: &StageRunRequest,
    runner_name: impl Into<String>,
    command: Vec<String>,
    cwd: impl Into<String>,
    environment_delta: RunnerEnvironmentDelta,
    prompt_path: impl Into<String>,
    emitted_at: Timestamp,
) -> RunnerResult<RunnerInvocationArtifact> {
    let mut artifact = RunnerInvocationArtifact {
        schema_version: "1.0".to_owned(),
        kind: "runner_invocation".to_owned(),
        request_id: request.request_id.clone(),
        run_id: request.run_id.clone(),
        stage: request.stage,
        request_kind: request.request_kind,
        active_work_item_id: request.active_work_item_id.clone(),
        closure_target_root_spec_id: request.closure_target_root_spec_id.clone(),
        runner_name: runner_name.into(),
        model_name: request.model_name.clone(),
        thinking_level: request.thinking_level.clone(),
        model_reasoning_effort: request.model_reasoning_effort.clone(),
        command,
        cwd: cwd.into(),
        environment_delta,
        prompt_path: prompt_path.into(),
        stdout_path: None,
        stderr_path: None,
        event_log_path: None,
        emitted_at,
        notes: Vec::new(),
    };
    artifact.validate()?;
    Ok(artifact)
}

/// Builds a runner completion artifact from a raw runner result.
pub fn completion_artifact_from_raw_result(
    request: &StageRunRequest,
    raw_result: &RunnerRawResult,
    context: RunnerCompletionArtifactContext,
) -> RunnerResult<RunnerCompletionArtifact> {
    let RunnerCompletionArtifactContext {
        runner_name,
        command,
        cwd,
        environment_delta,
        prompt_path,
        emitted_at,
        failure_class,
        notes,
    } = context;
    let duration_seconds = raw_result.duration_seconds()?;
    let mut artifact = RunnerCompletionArtifact {
        schema_version: "1.0".to_owned(),
        kind: "runner_completion".to_owned(),
        request_id: request.request_id.clone(),
        run_id: request.run_id.clone(),
        stage: request.stage,
        request_kind: request.request_kind,
        active_work_item_id: request.active_work_item_id.clone(),
        closure_target_root_spec_id: request.closure_target_root_spec_id.clone(),
        runner_name,
        model_name: raw_result.model_name.clone(),
        thinking_level: raw_result.thinking_level.clone(),
        model_reasoning_effort: raw_result.model_reasoning_effort.clone(),
        command,
        cwd,
        environment_delta,
        prompt_path,
        exit_kind: raw_result.exit_kind,
        exit_code: raw_result.exit_code,
        timed_out: raw_result.exit_kind == RunnerExitKind::Timeout
            || raw_result.observed_exit_kind == Some(RunnerExitKind::Timeout),
        interrupted: raw_result.exit_kind == RunnerExitKind::Interrupted
            || raw_result.observed_exit_kind == Some(RunnerExitKind::Interrupted),
        killed: false,
        transport_error: None,
        observed_exit_kind: raw_result.observed_exit_kind,
        observed_exit_code: raw_result.observed_exit_code,
        stdout_path: raw_result.stdout_path.clone(),
        stderr_path: raw_result.stderr_path.clone(),
        terminal_result_path: raw_result.terminal_result_path.clone(),
        event_log_path: raw_result.event_log_path.clone(),
        token_usage: raw_result.token_usage.clone(),
        started_at: raw_result.started_at.clone(),
        ended_at: raw_result.ended_at.clone(),
        duration_seconds,
        failure_class,
        notes,
        emitted_at,
    };
    artifact.validate()?;
    Ok(artifact)
}

/// Writes a runner invocation artifact as pretty JSON.
pub fn write_runner_invocation(
    path: &Path,
    artifact: &RunnerInvocationArtifact,
) -> RunnerResult<()> {
    write_artifact(path, artifact)
}

/// Writes a runner completion artifact as pretty JSON.
pub fn write_runner_completion(
    path: &Path,
    artifact: &RunnerCompletionArtifact,
) -> RunnerResult<()> {
    write_artifact(path, artifact)
}

fn write_artifact<T: Serialize>(path: &Path, artifact: &T) -> RunnerResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| RunnerError::Io {
            path: parent.display().to_string(),
            message: error.to_string(),
        })?;
    }
    let payload = serde_json::to_string_pretty(artifact).map_err(|error| RunnerError::Json {
        artifact: "runner_artifact",
        message: error.to_string(),
    })? + "\n";
    fs::write(path, payload).map_err(|error| RunnerError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

fn require_literal(
    field_name: &'static str,
    value: &str,
    expected: &str,
    artifact: &'static str,
) -> RunnerResult<()> {
    if value == expected {
        Ok(())
    } else {
        Err(RunnerError::InvalidRunnerArtifact {
            message: format!("{artifact}.{field_name} must be {expected:?}"),
        })
    }
}

fn require_non_blank(
    field_name: &'static str,
    value: &str,
    artifact: &'static str,
) -> RunnerResult<()> {
    if value.trim().is_empty() {
        Err(RunnerError::InvalidRunnerArtifact {
            message: format!("{artifact}.{field_name} is required"),
        })
    } else {
        Ok(())
    }
}

fn require_command(command: &[String], artifact: &'static str) -> RunnerResult<()> {
    if command.is_empty() || command.iter().any(|part| part.trim().is_empty()) {
        Err(RunnerError::InvalidRunnerArtifact {
            message: format!("{artifact}.command must contain non-empty argv parts"),
        })
    } else {
        Ok(())
    }
}
