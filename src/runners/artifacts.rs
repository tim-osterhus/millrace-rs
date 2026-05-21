//! Runner invocation and completion artifact contracts.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    contracts::{
        CapabilitySupportDecision, ExecutionCapabilityGrant, StageName, Timestamp, TokenUsage,
    },
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
    pub lane_id: Option<String>,
    pub launch_plan_id: Option<String>,
    pub request_context_profile_id: Option<String>,
    pub context_bundle_path: Option<String>,
    #[serde(default)]
    pub context_artifact_refs: Vec<String>,
    pub context_render_plan_id: Option<String>,
    pub rendered_prompt_context_path: Option<String>,
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
    #[serde(default)]
    pub execution_capability_grants: Vec<ExecutionCapabilityGrant>,
    #[serde(default)]
    pub capability_support_decisions: Vec<CapabilitySupportDecision>,
    #[serde(default)]
    pub capability_evidence_refs: Vec<String>,
    #[serde(default)]
    pub missing_capability_evidence_refs: Vec<String>,
    pub failure_capability_class: Option<String>,
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
        validate_optional_non_blank("lane_id", &self.lane_id, "runner_invocation")?;
        validate_optional_non_blank("launch_plan_id", &self.launch_plan_id, "runner_invocation")?;
        validate_optional_non_blank(
            "request_context_profile_id",
            &self.request_context_profile_id,
            "runner_invocation",
        )?;
        validate_optional_non_blank(
            "context_bundle_path",
            &self.context_bundle_path,
            "runner_invocation",
        )?;
        validate_optional_non_blank(
            "context_render_plan_id",
            &self.context_render_plan_id,
            "runner_invocation",
        )?;
        validate_optional_non_blank(
            "rendered_prompt_context_path",
            &self.rendered_prompt_context_path,
            "runner_invocation",
        )?;
        validate_non_blank_values(
            "context_artifact_refs",
            &self.context_artifact_refs,
            "runner_invocation",
        )?;
        require_command(&self.command, "runner_invocation")?;
        validate_capability_artifact_fields(
            "runner_invocation",
            &self.capability_evidence_refs,
            &self.missing_capability_evidence_refs,
            &self.failure_capability_class,
        )?;
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
    pub lane_id: Option<String>,
    pub launch_plan_id: Option<String>,
    pub request_context_profile_id: Option<String>,
    pub context_bundle_path: Option<String>,
    #[serde(default)]
    pub context_artifact_refs: Vec<String>,
    pub context_render_plan_id: Option<String>,
    pub rendered_prompt_context_path: Option<String>,
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
    pub failure_capability_class: Option<String>,
    #[serde(default)]
    pub execution_capability_grants: Vec<ExecutionCapabilityGrant>,
    #[serde(default)]
    pub capability_support_decisions: Vec<CapabilitySupportDecision>,
    #[serde(default)]
    pub capability_evidence_refs: Vec<String>,
    #[serde(default)]
    pub missing_capability_evidence_refs: Vec<String>,
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
        validate_optional_non_blank("lane_id", &self.lane_id, "runner_completion")?;
        validate_optional_non_blank("launch_plan_id", &self.launch_plan_id, "runner_completion")?;
        validate_optional_non_blank(
            "request_context_profile_id",
            &self.request_context_profile_id,
            "runner_completion",
        )?;
        validate_optional_non_blank(
            "context_bundle_path",
            &self.context_bundle_path,
            "runner_completion",
        )?;
        validate_optional_non_blank(
            "context_render_plan_id",
            &self.context_render_plan_id,
            "runner_completion",
        )?;
        validate_optional_non_blank(
            "rendered_prompt_context_path",
            &self.rendered_prompt_context_path,
            "runner_completion",
        )?;
        validate_non_blank_values(
            "context_artifact_refs",
            &self.context_artifact_refs,
            "runner_completion",
        )?;
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
        validate_capability_artifact_fields(
            "runner_completion",
            &self.capability_evidence_refs,
            &self.missing_capability_evidence_refs,
            &self.failure_capability_class,
        )?;
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
        lane_id: request.lane_id.clone(),
        launch_plan_id: request.launch_plan_id.clone(),
        request_context_profile_id: request.request_context_profile_id.clone(),
        context_bundle_path: request.context_bundle_path.clone(),
        context_artifact_refs: request.context_artifact_refs.clone(),
        context_render_plan_id: request.context_render_plan_id.clone(),
        rendered_prompt_context_path: request.rendered_prompt_context_path.clone(),
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
        execution_capability_grants: request.execution_capability_grants.clone(),
        capability_support_decisions: request.capability_support_decisions.clone(),
        capability_evidence_refs: capability_evidence_refs_for_request(request),
        missing_capability_evidence_refs: Vec::new(),
        failure_capability_class: None,
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
        lane_id: request.lane_id.clone(),
        launch_plan_id: request.launch_plan_id.clone(),
        request_context_profile_id: request.request_context_profile_id.clone(),
        context_bundle_path: request.context_bundle_path.clone(),
        context_artifact_refs: request.context_artifact_refs.clone(),
        context_render_plan_id: request.context_render_plan_id.clone(),
        rendered_prompt_context_path: request.rendered_prompt_context_path.clone(),
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
        failure_capability_class: raw_result.failure_capability_class.clone(),
        execution_capability_grants: request.execution_capability_grants.clone(),
        capability_support_decisions: if raw_result.capability_support_decisions.is_empty() {
            request.capability_support_decisions.clone()
        } else {
            raw_result.capability_support_decisions.clone()
        },
        capability_evidence_refs: raw_result.capability_evidence_refs.clone(),
        missing_capability_evidence_refs: raw_result.missing_capability_evidence_refs.clone(),
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

/// Returns the runner-owned capability evidence refs available after completion.
#[must_use]
pub fn capability_evidence_refs_for_request(request: &StageRunRequest) -> Vec<String> {
    if request.execution_capability_grants.is_empty() {
        return Vec::new();
    }
    vec![
        format!("runner_invocation:{}", request.request_id),
        format!("runner_completion:{}", request.request_id),
    ]
}

/// Returns required evidence refs not produced by normal runner invocation/completion artifacts.
#[must_use]
pub fn missing_capability_evidence_refs_for_request(request: &StageRunRequest) -> Vec<String> {
    if request.execution_capability_grants.is_empty() {
        return Vec::new();
    }
    let available = ["runner_invocation", "runner_completion"];
    let mut missing = Vec::new();
    for grant in &request.execution_capability_grants {
        if !grant.required {
            continue;
        }
        for requirement in &grant.evidence_requirements {
            if available.contains(&requirement.as_str()) {
                continue;
            }
            let reference = format!("{}:{requirement}", grant.grant_id);
            if !missing.contains(&reference) {
                missing.push(reference);
            }
        }
    }
    missing
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

fn validate_optional_non_blank(
    field_name: &'static str,
    value: &Option<String>,
    artifact: &'static str,
) -> RunnerResult<()> {
    if value
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        Err(RunnerError::InvalidRunnerArtifact {
            message: format!("{artifact}.{field_name} must not be blank when set"),
        })
    } else {
        Ok(())
    }
}

fn validate_non_blank_values(
    field_name: &'static str,
    values: &[String],
    artifact: &'static str,
) -> RunnerResult<()> {
    if values.iter().any(|value| value.trim().is_empty()) {
        Err(RunnerError::InvalidRunnerArtifact {
            message: format!("{artifact}.{field_name} must not contain blank values"),
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

fn validate_capability_artifact_fields(
    artifact: &'static str,
    capability_evidence_refs: &[String],
    missing_capability_evidence_refs: &[String],
    failure_capability_class: &Option<String>,
) -> RunnerResult<()> {
    if capability_evidence_refs
        .iter()
        .chain(missing_capability_evidence_refs)
        .any(|value| value.trim().is_empty())
    {
        return Err(RunnerError::InvalidRunnerArtifact {
            message: format!("{artifact} capability evidence refs must not be blank"),
        });
    }
    if failure_capability_class
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(RunnerError::InvalidRunnerArtifact {
            message: format!("{artifact}.failure_capability_class must not be blank"),
        });
    }
    Ok(())
}
