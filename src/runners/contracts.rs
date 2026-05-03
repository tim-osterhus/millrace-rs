use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::contracts::{StageName, Timestamp, TokenUsage};
use crate::runtime::StageRunRequest;

/// Runner exit categories captured before stage-result normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerExitKind {
    Completed,
    Timeout,
    RunnerError,
    ProviderError,
    Interrupted,
}

impl RunnerExitKind {
    /// Returns the canonical serialized value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Timeout => "timeout",
            Self::RunnerError => "runner_error",
            Self::ProviderError => "provider_error",
            Self::Interrupted => "interrupted",
        }
    }
}

impl fmt::Display for RunnerExitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Thin raw result emitted by a runner after one stage invocation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunnerRawResult {
    pub request_id: String,
    pub run_id: String,
    pub stage: StageName,
    pub runner_name: String,
    pub model_name: Option<String>,
    pub thinking_level: Option<String>,
    pub model_reasoning_effort: Option<String>,

    pub exit_kind: RunnerExitKind,
    pub exit_code: Option<i32>,
    pub observed_exit_kind: Option<RunnerExitKind>,
    pub observed_exit_code: Option<i32>,

    pub stdout_path: Option<String>,
    pub stderr_path: Option<String>,
    pub terminal_result_path: Option<String>,
    pub event_log_path: Option<String>,
    pub token_usage: Option<TokenUsage>,

    pub started_at: Timestamp,
    pub ended_at: Timestamp,
}

impl<'de> Deserialize<'de> for RunnerRawResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        RunnerRawResultRaw::deserialize(deserializer)?
            .try_into_raw_result()
            .map_err(serde::de::Error::custom)
    }
}

impl RunnerRawResult {
    /// Deserializes and validates a raw runner result JSON value.
    pub fn from_json_value(value: Value) -> RunnerResult<Self> {
        serde_json::from_value(value).map_err(|error| RunnerError::Json {
            artifact: "runner_raw_result",
            message: error.to_string(),
        })
    }

    /// Deserializes and validates a raw runner result JSON string.
    pub fn from_json_str(raw: &str) -> RunnerResult<Self> {
        serde_json::from_str(raw).map_err(|error| RunnerError::Json {
            artifact: "runner_raw_result",
            message: error.to_string(),
        })
    }

    /// Validates identity and timestamp invariants.
    pub fn validate(&self) -> RunnerResult<()> {
        require_non_blank("request_id", &self.request_id)?;
        require_non_blank("run_id", &self.run_id)?;
        require_non_blank("runner_name", &self.runner_name)?;
        if self.ended_time()? < self.started_time()? {
            return Err(RunnerError::InvalidRawResult {
                message: "ended_at cannot precede started_at".to_owned(),
            });
        }
        Ok(())
    }

    /// Returns runner duration in seconds.
    pub fn duration_seconds(&self) -> RunnerResult<f64> {
        Ok((self.ended_time()? - self.started_time()?).as_seconds_f64())
    }

    fn started_time(&self) -> RunnerResult<OffsetDateTime> {
        parse_time("started_at", &self.started_at)
    }

    fn ended_time(&self) -> RunnerResult<OffsetDateTime> {
        parse_time("ended_at", &self.ended_at)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunnerRawResultRaw {
    request_id: String,
    run_id: String,
    stage: StageName,
    runner_name: String,
    model_name: Option<String>,
    thinking_level: Option<String>,
    model_reasoning_effort: Option<String>,

    exit_kind: RunnerExitKind,
    exit_code: Option<i32>,
    observed_exit_kind: Option<RunnerExitKind>,
    observed_exit_code: Option<i32>,

    stdout_path: Option<String>,
    stderr_path: Option<String>,
    terminal_result_path: Option<String>,
    event_log_path: Option<String>,
    token_usage: Option<TokenUsage>,

    started_at: Timestamp,
    ended_at: Timestamp,
}

impl RunnerRawResultRaw {
    fn try_into_raw_result(self) -> RunnerResult<RunnerRawResult> {
        let raw_result = RunnerRawResult {
            request_id: self.request_id,
            run_id: self.run_id,
            stage: self.stage,
            runner_name: self.runner_name,
            model_name: self.model_name,
            thinking_level: self.thinking_level,
            model_reasoning_effort: self.model_reasoning_effort,
            exit_kind: self.exit_kind,
            exit_code: self.exit_code,
            observed_exit_kind: self.observed_exit_kind,
            observed_exit_code: self.observed_exit_code,
            stdout_path: self.stdout_path,
            stderr_path: self.stderr_path,
            terminal_result_path: self.terminal_result_path,
            event_log_path: self.event_log_path,
            token_usage: self.token_usage,
            started_at: self.started_at,
            ended_at: self.ended_at,
        };
        raw_result.validate()?;
        Ok(raw_result)
    }
}

/// Result alias used by runner boundary APIs.
pub type RunnerResult<T> = Result<T, RunnerError>;

/// Stage runner adapter boundary.
pub trait StageRunnerAdapter {
    /// Runs one stage request and returns raw runner output.
    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult>;
}

/// Typed failures emitted by runner contracts and normalization.
#[derive(Debug)]
pub enum RunnerError {
    /// JSON syntax, field, enum, or unknown-field decoding failed.
    Json {
        /// Artifact type being decoded.
        artifact: &'static str,
        /// Serde error message.
        message: String,
    },
    /// A raw runner result violates contract invariants.
    InvalidRawResult {
        /// Human-readable failure reason.
        message: String,
    },
    /// The stage request cannot be normalized into a stage result.
    InvalidRequest {
        /// Human-readable failure reason.
        message: String,
    },
    /// File IO failed while reading or writing runner artifacts.
    Io {
        /// Path involved in the failure.
        path: String,
        /// IO error message.
        message: String,
    },
    /// Stage result envelope validation failed after normalization.
    StageResultEnvelope {
        /// Human-readable failure reason.
        message: String,
    },
    /// A process-result artifact violates contract invariants.
    InvalidProcessResult {
        /// Human-readable failure reason.
        message: String,
    },
    /// A runner-artifact payload violates contract invariants.
    InvalidRunnerArtifact {
        /// Human-readable failure reason.
        message: String,
    },
    /// A registry operation received an invalid adapter name.
    InvalidRunnerName {
        /// Human-readable failure reason.
        message: String,
    },
    /// An adapter name was registered more than once.
    DuplicateRunner {
        /// Duplicate adapter name.
        runner_name: String,
    },
    /// Dispatcher could not resolve the requested runner name.
    UnknownRunner {
        /// Requested runner name after dispatcher resolution.
        requested: String,
        /// Available adapter names at the time of resolution.
        available: Vec<String>,
    },
    /// A runner subprocess executable was not available.
    RunnerBinaryNotFound {
        /// Missing binary name.
        binary: String,
    },
    /// A runner subprocess failed before producing a normal process result.
    RunnerTransport {
        /// Human-readable transport failure.
        message: String,
    },
}

impl fmt::Display for RunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json { artifact, message } => {
                write!(f, "failed to decode {artifact}: {message}")
            }
            Self::InvalidRawResult { message } => f.write_str(message),
            Self::InvalidRequest { message } => f.write_str(message),
            Self::Io { path, message } => {
                write!(f, "runner artifact IO failed at {path}: {message}")
            }
            Self::StageResultEnvelope { message } => {
                write!(f, "normalized stage result is invalid: {message}")
            }
            Self::InvalidProcessResult { message } => f.write_str(message),
            Self::InvalidRunnerArtifact { message } => f.write_str(message),
            Self::InvalidRunnerName { message } => f.write_str(message),
            Self::DuplicateRunner { runner_name } => {
                write!(f, "duplicate stage runner adapter: {runner_name}")
            }
            Self::UnknownRunner {
                requested,
                available,
            } => write!(
                f,
                "Unknown stage runner: {requested}. Available: {}",
                if available.is_empty() {
                    "none".to_owned()
                } else {
                    available.join(", ")
                }
            ),
            Self::RunnerBinaryNotFound { binary } => {
                write!(f, "runner binary not found: {binary}")
            }
            Self::RunnerTransport { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for RunnerError {}

pub(crate) fn require_non_blank(field_name: &'static str, value: &str) -> RunnerResult<()> {
    if value.trim().is_empty() {
        Err(RunnerError::InvalidRawResult {
            message: format!("{field_name} is required"),
        })
    } else {
        Ok(())
    }
}

fn parse_time(field_name: &'static str, timestamp: &Timestamp) -> RunnerResult<OffsetDateTime> {
    OffsetDateTime::parse(timestamp.as_str(), &Rfc3339).map_err(|error| {
        RunnerError::InvalidRawResult {
            message: format!("{field_name} is invalid: {error}"),
        }
    })
}
