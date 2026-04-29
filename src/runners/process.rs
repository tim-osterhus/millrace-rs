//! Shared process-result models used by concrete runner adapters.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::contracts::Timestamp;

use super::{RunnerError, RunnerExitKind, RunnerResult};

/// Environment changes applied by a runner adapter before process execution.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerEnvironmentDelta {
    /// Environment variables set or overwritten for the process.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub set: BTreeMap<String, String>,
    /// Environment variable names removed from the inherited environment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unset: Vec<String>,
}

impl RunnerEnvironmentDelta {
    /// Returns true when no environment changes are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.unset.is_empty()
    }
}

/// Canonical process outcome before stage-result normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessExitKind {
    /// Process exited without runner-level transport failure.
    Completed,
    /// Process exceeded its timeout.
    Timeout,
    /// Process was interrupted before a normal exit.
    Interrupted,
    /// Process was killed after cancellation or timeout handling.
    Killed,
    /// Process failed before or during transport.
    TransportError,
}

impl ProcessExitKind {
    /// Returns the canonical serialized value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Timeout => "timeout",
            Self::Interrupted => "interrupted",
            Self::Killed => "killed",
            Self::TransportError => "transport_error",
        }
    }

    /// Maps process outcomes into the raw runner-result exit categories.
    #[must_use]
    pub const fn as_runner_exit_kind(self) -> RunnerExitKind {
        match self {
            Self::Completed => RunnerExitKind::Completed,
            Self::Timeout => RunnerExitKind::Timeout,
            Self::Interrupted | Self::Killed => RunnerExitKind::Interrupted,
            Self::TransportError => RunnerExitKind::RunnerError,
        }
    }
}

/// Explicit process execution result shared by subprocess-backed adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessExecutionResult {
    pub schema_version: String,
    pub kind: String,
    pub command: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub environment_delta: RunnerEnvironmentDelta,
    pub stdout_path: Option<String>,
    pub stderr_path: Option<String>,
    pub event_log_path: Option<String>,
    pub exit_kind: ProcessExitKind,
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub interrupted: bool,
    #[serde(default)]
    pub killed: bool,
    pub transport_error: Option<String>,
    pub started_at: Timestamp,
    pub ended_at: Timestamp,
    pub duration_seconds: f64,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl ProcessExecutionResult {
    /// Builds a process result and validates the shared invariants.
    pub fn new(
        command: Vec<String>,
        cwd: impl Into<String>,
        environment_delta: RunnerEnvironmentDelta,
        exit_kind: ProcessExitKind,
        exit_code: Option<i32>,
        started_at: Timestamp,
        ended_at: Timestamp,
    ) -> RunnerResult<Self> {
        let duration_seconds = duration_seconds_between(&started_at, &ended_at)?;
        let mut result = Self {
            schema_version: "1.0".to_owned(),
            kind: "process_execution_result".to_owned(),
            command,
            cwd: cwd.into(),
            environment_delta,
            stdout_path: None,
            stderr_path: None,
            event_log_path: None,
            exit_kind,
            exit_code,
            timed_out: exit_kind == ProcessExitKind::Timeout,
            interrupted: exit_kind == ProcessExitKind::Interrupted,
            killed: exit_kind == ProcessExitKind::Killed,
            transport_error: None,
            started_at,
            ended_at,
            duration_seconds,
            notes: Vec::new(),
        };
        result.validate()?;
        Ok(result)
    }

    /// Validates process-result identity, timing, and outcome consistency.
    pub fn validate(&mut self) -> RunnerResult<()> {
        require_literal("schema_version", &self.schema_version, "1.0")?;
        require_literal("kind", &self.kind, "process_execution_result")?;
        if self.command.is_empty() || self.command.iter().any(|part| part.trim().is_empty()) {
            return Err(RunnerError::InvalidProcessResult {
                message: "command must contain non-empty argv parts".to_owned(),
            });
        }
        if self.cwd.trim().is_empty() {
            return Err(RunnerError::InvalidProcessResult {
                message: "cwd is required".to_owned(),
            });
        }
        let computed_duration = duration_seconds_between(&self.started_at, &self.ended_at)?;
        if self.duration_seconds < 0.0 {
            return Err(RunnerError::InvalidProcessResult {
                message: "duration_seconds must be >= 0".to_owned(),
            });
        }
        if (self.duration_seconds - computed_duration).abs() > 0.001 {
            return Err(RunnerError::InvalidProcessResult {
                message: "duration_seconds must match started_at and ended_at".to_owned(),
            });
        }
        if self.timed_out != (self.exit_kind == ProcessExitKind::Timeout) {
            return Err(RunnerError::InvalidProcessResult {
                message: "timed_out must match timeout exit_kind".to_owned(),
            });
        }
        if self.interrupted != (self.exit_kind == ProcessExitKind::Interrupted) {
            return Err(RunnerError::InvalidProcessResult {
                message: "interrupted must match interrupted exit_kind".to_owned(),
            });
        }
        if self.killed != (self.exit_kind == ProcessExitKind::Killed) {
            return Err(RunnerError::InvalidProcessResult {
                message: "killed must match killed exit_kind".to_owned(),
            });
        }
        if self.exit_kind == ProcessExitKind::TransportError
            && self
                .transport_error
                .as_deref()
                .is_none_or(|message| message.trim().is_empty())
        {
            return Err(RunnerError::InvalidProcessResult {
                message: "transport_error is required for transport_error exit_kind".to_owned(),
            });
        }
        Ok(())
    }
}

fn require_literal(field_name: &'static str, value: &str, expected: &str) -> RunnerResult<()> {
    if value == expected {
        Ok(())
    } else {
        Err(RunnerError::InvalidProcessResult {
            message: format!("{field_name} must be {expected:?}"),
        })
    }
}

pub(crate) fn duration_seconds_between(
    started_at: &Timestamp,
    ended_at: &Timestamp,
) -> RunnerResult<f64> {
    let started = parse_time("started_at", started_at)?;
    let ended = parse_time("ended_at", ended_at)?;
    if ended < started {
        return Err(RunnerError::InvalidProcessResult {
            message: "ended_at cannot precede started_at".to_owned(),
        });
    }
    Ok((ended - started).as_seconds_f64())
}

fn parse_time(field_name: &'static str, timestamp: &Timestamp) -> RunnerResult<OffsetDateTime> {
    OffsetDateTime::parse(timestamp.as_str(), &Rfc3339).map_err(|error| {
        RunnerError::InvalidProcessResult {
            message: format!("{field_name} is invalid: {error}"),
        }
    })
}
