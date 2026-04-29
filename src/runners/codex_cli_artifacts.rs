//! Codex CLI artifact path and materialization helpers.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::runtime::StageRunRequest;

use super::{RunnerError, RunnerResult, prompting::legal_terminal_markers};

/// Canonical Codex CLI artifact paths for one stage request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexCliArtifactPaths {
    pub prompt_path: PathBuf,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
    pub event_log_path: PathBuf,
    pub output_last_message_path: PathBuf,
    pub invocation_path: PathBuf,
    pub completion_path: PathBuf,
}

/// Returns Python-compatible Codex CLI artifact paths.
#[must_use]
pub fn codex_cli_artifact_paths(run_dir: &Path, request_id: &str) -> CodexCliArtifactPaths {
    CodexCliArtifactPaths {
        prompt_path: run_dir.join(format!("runner_prompt.{request_id}.md")),
        stdout_path: run_dir.join(format!("runner_stdout.{request_id}.txt")),
        stderr_path: run_dir.join(format!("runner_stderr.{request_id}.txt")),
        event_log_path: run_dir.join(format!("runner_events.{request_id}.jsonl")),
        output_last_message_path: run_dir.join(format!("runner_last_message.{request_id}.txt")),
        invocation_path: run_dir.join(format!("runner_invocation.{request_id}.json")),
        completion_path: run_dir.join(format!("runner_completion.{request_id}.json")),
    }
}

/// Preserves raw Codex JSONL stdout as the event-log artifact when stdout exists.
pub fn persist_event_log(
    stdout_path: &Path,
    event_log_path: &Path,
) -> RunnerResult<Option<PathBuf>> {
    if !stdout_path.exists() {
        return Ok(None);
    }
    if let Some(parent) = event_log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| RunnerError::Io {
            path: parent.display().to_string(),
            message: error.to_string(),
        })?;
    }
    let raw = fs::read(stdout_path).map_err(|error| RunnerError::Io {
        path: stdout_path.display().to_string(),
        message: error.to_string(),
    })?;
    fs::write(event_log_path, raw).map_err(|error| RunnerError::Io {
        path: event_log_path.display().to_string(),
        message: error.to_string(),
    })?;
    fs::remove_file(stdout_path).map_err(|error| RunnerError::Io {
        path: stdout_path.display().to_string(),
        message: error.to_string(),
    })?;
    Ok(Some(event_log_path.to_path_buf()))
}

/// Writes the final assistant text to the public runner stdout artifact.
pub fn materialize_stdout_artifact(
    stdout_path: &Path,
    output_last_message_path: &Path,
    event_log_path: Option<&Path>,
) -> RunnerResult<Option<PathBuf>> {
    if output_last_message_path.exists() {
        let text =
            fs::read_to_string(output_last_message_path).map_err(|error| RunnerError::Io {
                path: output_last_message_path.display().to_string(),
                message: error.to_string(),
            })?;
        write_text(stdout_path, &text)?;
        return Ok(Some(stdout_path.to_path_buf()));
    }
    if let Some(event_log_path) = event_log_path.filter(|path| path.exists()) {
        let text = fs::read_to_string(event_log_path).map_err(|error| RunnerError::Io {
            path: event_log_path.display().to_string(),
            message: error.to_string(),
        })?;
        write_text(stdout_path, &text)?;
        return Ok(Some(stdout_path.to_path_buf()));
    }
    Ok(None)
}

/// Returns a legal final terminal marker observed in Codex output-last-message text.
#[must_use]
pub fn reconciled_timeout_terminal_marker(
    request: &StageRunRequest,
    output_last_message_path: &Path,
) -> Option<String> {
    let raw = fs::read_to_string(output_last_message_path).ok()?;
    let stripped_nonempty = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if stripped_nonempty.is_empty() {
        return None;
    }
    let legal_markers = legal_terminal_markers(request)
        .iter()
        .filter_map(|marker| marker.strip_prefix("### ").map(str::trim))
        .collect::<Vec<_>>();
    let observed = raw
        .lines()
        .filter_map(|line| terminal_marker_token(line.trim()))
        .filter(|marker| legal_markers.contains(&marker.as_str()))
        .collect::<Vec<_>>();
    if observed.len() != 1 {
        return None;
    }
    let marker = observed[0].clone();
    if stripped_nonempty.last().copied() != Some(format!("### {marker}").as_str()) {
        return None;
    }
    Some(marker)
}

fn terminal_marker_token(line: &str) -> Option<String> {
    let rest = line.strip_prefix("###")?.trim();
    if rest.is_empty() || !rest.chars().all(|ch| ch == '_' || ch.is_ascii_uppercase()) {
        return None;
    }
    Some(rest.to_owned())
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
