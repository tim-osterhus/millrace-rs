use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::contracts::{
    BlockedOrigin, FailureClassifierCode, FailureScope, ResultClass, RunnerFailureClass,
    RunnerFailureMetadata, StageResultEnvelope, TerminalResult, WorkItemKind,
    blocked_terminal_for_plane, terminal_result_for_plane,
};
use crate::runtime::{RequestKind, StageRunRequest};

use super::{RunnerError, RunnerExitKind, RunnerRawResult, RunnerResult};

const MAX_CLASSIFIER_TEXT_CHARS: usize = 16_000;

/// Normalizes one raw runner output into a deterministic stage result envelope.
pub fn normalize_stage_result(
    request: &StageRunRequest,
    raw_result: &RunnerRawResult,
) -> RunnerResult<StageResultEnvelope> {
    raw_result.validate()?;

    let identity_notes = identity_mismatch_notes(request, raw_result);
    if !identity_notes.is_empty() {
        return failure_envelope(
            request,
            raw_result,
            RunnerFailureClass::RunnerTransportFailure,
            identity_notes,
            None,
            Vec::new(),
            None,
        );
    }

    if raw_result.exit_kind != RunnerExitKind::Completed
        && let Some(classification) = classify_raw_exit_failure(raw_result)
    {
        return failure_envelope(
            request,
            raw_result,
            classification.failure_class,
            vec![format!(
                "runner exited with {}",
                raw_result.exit_kind.as_str()
            )],
            None,
            Vec::new(),
            Some(classification),
        );
    }
    if raw_result.exit_kind == RunnerExitKind::Completed
        && raw_result.exit_code.is_some_and(|code| code != 0)
    {
        let classification = classify_raw_exit_failure(raw_result).unwrap_or_else(|| {
            classification_for_failure_class(RunnerFailureClass::RunnerTransportFailure)
        });
        return failure_envelope(
            request,
            raw_result,
            classification.failure_class,
            vec!["runner completed with non-zero exit code".to_owned()],
            None,
            Vec::new(),
            Some(classification),
        );
    }

    let extraction = extract_terminal_result(request, raw_result);
    if !extraction.ok() {
        return failure_envelope(
            request,
            raw_result,
            extraction
                .failure_class
                .unwrap_or(RunnerFailureClass::IllegalTerminalResult),
            extraction.notes,
            extraction.detected_marker,
            extraction.artifact_paths,
            None,
        );
    }

    let terminal_result = extraction
        .terminal_result
        .expect("ok extraction has terminal_result");
    let result_class = extraction
        .result_class
        .expect("ok extraction has result_class");
    let report_artifact = resolved_report_artifact(request);
    let mut notes = extraction.notes;
    notes.extend(transport_reconciliation_notes(raw_result));

    let envelope = StageResultEnvelope {
        schema_version: "1.0".to_owned(),
        kind: "stage_result".to_owned(),
        run_id: request.run_id.clone(),
        plane: request.plane,
        stage: request.stage,
        node_id: request.node_id.clone(),
        stage_kind_id: request.stage_kind_id.clone(),
        work_item_kind: request_result_identity(request)?.0,
        work_item_id: request_result_identity(request)?.1,
        terminal_result,
        result_class,
        summary_status_marker: terminal_result.marker(),
        success: result_class == ResultClass::Success,
        retryable: false,
        exit_code: raw_result.exit_code.unwrap_or(0),
        duration_seconds: raw_result.duration_seconds()?,
        prompt_artifact: None,
        report_artifact: report_artifact.clone(),
        artifact_paths: merge_artifact_paths(
            extraction.artifact_paths,
            [report_artifact, raw_result.event_log_path.clone()],
        ),
        detected_marker: extraction.detected_marker,
        stdout_path: raw_result.stdout_path.clone(),
        stderr_path: raw_result.stderr_path.clone(),
        runner_name: Some(raw_result.runner_name.clone()),
        model_name: raw_result.model_name.clone(),
        thinking_level: resolved_thinking_level(request, raw_result),
        model_reasoning_effort: raw_result
            .model_reasoning_effort
            .clone()
            .or_else(|| request.model_reasoning_effort.clone()),
        token_usage: raw_result.token_usage.clone(),
        notes,
        metadata: request_metadata(
            request,
            if raw_result.terminal_result_path.is_some() {
                "structured_result_file"
            } else {
                "stdout_terminal_token"
            },
            None,
            true,
            raw_result,
        ),
        started_at: raw_result.started_at.clone(),
        completed_at: raw_result.ended_at.clone(),
    };
    validate_envelope(envelope)
}

#[derive(Debug)]
struct TerminalExtraction {
    terminal_result: Option<TerminalResult>,
    result_class: Option<ResultClass>,
    detected_marker: Option<String>,
    artifact_paths: Vec<String>,
    failure_class: Option<RunnerFailureClass>,
    notes: Vec<String>,
}

impl TerminalExtraction {
    fn ok(&self) -> bool {
        self.failure_class.is_none()
            && self.terminal_result.is_some()
            && self.result_class.is_some()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredTerminalResultPayload {
    stage: Option<String>,
    terminal_result: String,
    result_class: Option<ResultClass>,
    #[serde(default)]
    summary_artifact_paths: Vec<String>,
}

fn extract_terminal_result(
    request: &StageRunRequest,
    raw_result: &RunnerRawResult,
) -> TerminalExtraction {
    if let Some(path) = &raw_result.terminal_result_path {
        return extract_from_structured_result_file(request, Path::new(path));
    }
    extract_from_stdout_tokens(request, raw_result.stdout_path.as_deref())
}

fn extract_from_structured_result_file(
    request: &StageRunRequest,
    terminal_result_path: &Path,
) -> TerminalExtraction {
    if !terminal_result_path.exists() {
        return TerminalExtraction {
            terminal_result: None,
            result_class: None,
            detected_marker: None,
            artifact_paths: Vec::new(),
            failure_class: Some(RunnerFailureClass::MissingTerminalResult),
            notes: vec![format!(
                "structured terminal result file is missing: {}",
                terminal_result_path.display()
            )],
        };
    }

    let raw_payload = match fs::read_to_string(terminal_result_path) {
        Ok(raw) => raw,
        Err(error) => {
            return TerminalExtraction {
                terminal_result: None,
                result_class: None,
                detected_marker: None,
                artifact_paths: Vec::new(),
                failure_class: Some(RunnerFailureClass::IllegalTerminalResult),
                notes: vec![format!(
                    "failed to parse structured terminal result: {error}"
                )],
            };
        }
    };

    let payload = match serde_json::from_str::<Value>(&raw_payload)
        .and_then(serde_json::from_value::<StructuredTerminalResultPayload>)
    {
        Ok(payload) => payload,
        Err(error) => {
            return TerminalExtraction {
                terminal_result: None,
                result_class: None,
                detected_marker: None,
                artifact_paths: Vec::new(),
                failure_class: Some(RunnerFailureClass::IllegalTerminalResult),
                notes: vec![format!(
                    "structured terminal result payload is invalid: {error}"
                )],
            };
        }
    };

    if payload
        .stage
        .as_deref()
        .is_some_and(|stage| stage != request.stage.as_str())
    {
        return TerminalExtraction {
            terminal_result: None,
            result_class: None,
            detected_marker: None,
            artifact_paths: payload.summary_artifact_paths,
            failure_class: Some(RunnerFailureClass::IllegalTerminalResult),
            notes: vec![
                "structured terminal result stage does not match run request stage".to_owned(),
            ],
        };
    }

    let terminal_result = match terminal_result_for_request(request, &payload.terminal_result) {
        Some(terminal_result) => terminal_result,
        None => {
            return TerminalExtraction {
                terminal_result: None,
                result_class: None,
                detected_marker: None,
                artifact_paths: payload.summary_artifact_paths,
                failure_class: Some(RunnerFailureClass::IllegalTerminalResult),
                notes: vec![format!(
                    "terminal result {:?} is illegal for request node {}",
                    payload.terminal_result, request.node_id
                )],
            };
        }
    };

    let result_class =
        match resolve_result_class(request, &payload.terminal_result, payload.result_class) {
            Some(result_class) => result_class,
            None => {
                return TerminalExtraction {
                    terminal_result: None,
                    result_class: None,
                    detected_marker: None,
                    artifact_paths: payload.summary_artifact_paths,
                    failure_class: Some(RunnerFailureClass::IllegalTerminalResult),
                    notes: vec![
                        "structured terminal result class is incompatible with terminal_result"
                            .to_owned(),
                    ],
                };
            }
        };

    let missing_artifacts = payload
        .summary_artifact_paths
        .iter()
        .filter(|candidate| !artifact_exists(&request.run_dir, candidate))
        .cloned()
        .collect::<Vec<_>>();
    if !missing_artifacts.is_empty() {
        return TerminalExtraction {
            terminal_result: None,
            result_class: None,
            detected_marker: None,
            artifact_paths: payload.summary_artifact_paths,
            failure_class: Some(RunnerFailureClass::MissingRequiredArtifact),
            notes: vec![format!(
                "missing required summary artifacts: {}",
                missing_artifacts.join(", ")
            )],
        };
    }

    TerminalExtraction {
        terminal_result: Some(terminal_result),
        result_class: Some(result_class),
        detected_marker: Some(terminal_result.marker()),
        artifact_paths: payload.summary_artifact_paths,
        failure_class: None,
        notes: vec!["terminal result resolved from structured result file".to_owned()],
    }
}

fn extract_from_stdout_tokens(
    request: &StageRunRequest,
    stdout_path: Option<&str>,
) -> TerminalExtraction {
    let Some(stdout_path) = stdout_path else {
        return TerminalExtraction {
            terminal_result: None,
            result_class: None,
            detected_marker: None,
            artifact_paths: Vec::new(),
            failure_class: Some(RunnerFailureClass::MissingTerminalResult),
            notes: vec![
                "stdout path is missing and no structured terminal result was provided".to_owned(),
            ],
        };
    };

    let path = Path::new(stdout_path);
    if !path.exists() {
        return TerminalExtraction {
            terminal_result: None,
            result_class: None,
            detected_marker: None,
            artifact_paths: Vec::new(),
            failure_class: Some(RunnerFailureClass::MissingTerminalResult),
            notes: vec![format!("stdout file is missing: {stdout_path}")],
        };
    }

    let raw_stdout = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) => {
            return TerminalExtraction {
                terminal_result: None,
                result_class: None,
                detected_marker: None,
                artifact_paths: Vec::new(),
                failure_class: Some(RunnerFailureClass::RunnerTransportFailure),
                notes: vec![format!("failed reading stdout file: {error}")],
            };
        }
    };

    let tokens = raw_stdout
        .lines()
        .filter_map(terminal_token_from_line)
        .map(str::to_owned)
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        return TerminalExtraction {
            terminal_result: None,
            result_class: None,
            detected_marker: None,
            artifact_paths: Vec::new(),
            failure_class: Some(RunnerFailureClass::MissingTerminalResult),
            notes: vec!["no terminal token found in stdout".to_owned()],
        };
    }

    let unique_tokens = tokens.iter().collect::<BTreeSet<_>>();
    if unique_tokens.len() > 1 {
        return TerminalExtraction {
            terminal_result: None,
            result_class: None,
            detected_marker: tokens.last().map(|token| format!("### {token}")),
            artifact_paths: Vec::new(),
            failure_class: Some(RunnerFailureClass::ConflictingTerminalResults),
            notes: vec!["stdout contains conflicting terminal tokens".to_owned()],
        };
    }

    let final_token = tokens.last().expect("tokens not empty");
    let terminal_result = match terminal_result_for_request(request, final_token) {
        Some(terminal_result) => terminal_result,
        None => {
            return TerminalExtraction {
                terminal_result: None,
                result_class: None,
                detected_marker: Some(format!("### {final_token}")),
                artifact_paths: Vec::new(),
                failure_class: Some(RunnerFailureClass::IllegalTerminalResult),
                notes: vec![format!(
                    "terminal token {:?} is illegal for request node {}",
                    final_token, request.node_id
                )],
            };
        }
    };

    let result_class = resolve_result_class(request, final_token, None)
        .expect("request policy already matched terminal token");
    TerminalExtraction {
        terminal_result: Some(terminal_result),
        result_class: Some(result_class),
        detected_marker: Some(format!("### {final_token}")),
        artifact_paths: Vec::new(),
        failure_class: None,
        notes: vec!["terminal result resolved from stdout token".to_owned()],
    }
}

fn terminal_token_from_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("###")?;
    if rest.is_empty() || !rest.as_bytes()[0].is_ascii_whitespace() {
        return None;
    }
    let token = rest.trim();
    if token.is_empty()
        || token.chars().any(char::is_whitespace)
        || !token.chars().all(|ch| ch.is_ascii_uppercase() || ch == '_')
    {
        return None;
    }
    Some(token)
}

fn classify_raw_exit_failure(raw_result: &RunnerRawResult) -> Option<RunnerFailureMetadata> {
    if raw_result.exit_kind == RunnerExitKind::Completed
        && raw_result.exit_code.is_none_or(|code| code == 0)
    {
        return None;
    }
    if raw_result.exit_kind == RunnerExitKind::Timeout {
        return Some(RunnerFailureMetadata::new(
            RunnerFailureClass::RunnerTimeout,
            BlockedOrigin::RunnerFailure,
            FailureScope::Environment,
            true,
            FailureClassifierCode::ExitTimeout,
        ));
    }

    let evidence = raw_failure_evidence(raw_result);
    if let Some(classification) = classify_failure_evidence(&evidence, BlockedOrigin::RunnerFailure)
    {
        return Some(classification);
    }
    if raw_result.exit_kind == RunnerExitKind::ProviderError {
        return Some(RunnerFailureMetadata::new(
            RunnerFailureClass::ProviderUnavailable,
            BlockedOrigin::RunnerFailure,
            FailureScope::Provider,
            true,
            FailureClassifierCode::ProviderDefaultUnavailable,
        ));
    }
    Some(classification_for_failure_class(
        RunnerFailureClass::RunnerTransportFailure,
    ))
}

fn raw_failure_evidence(raw_result: &RunnerRawResult) -> String {
    let mut parts = Vec::new();
    for raw_path in [&raw_result.stderr_path, &raw_result.stdout_path]
        .into_iter()
        .flatten()
    {
        let path = Path::new(raw_path);
        if !path.is_file() {
            continue;
        }
        if let Ok(bytes) = fs::read(path) {
            parts.push(String::from_utf8_lossy(&bytes).into_owned());
        }
    }
    let evidence = parts.join("\n").to_lowercase();
    let char_count = evidence.chars().count();
    if char_count <= MAX_CLASSIFIER_TEXT_CHARS {
        evidence
    } else {
        evidence
            .chars()
            .skip(char_count - MAX_CLASSIFIER_TEXT_CHARS)
            .collect()
    }
}

fn classify_failure_evidence(
    evidence: &str,
    blocked_origin: BlockedOrigin,
) -> Option<RunnerFailureMetadata> {
    if evidence.trim().is_empty() {
        return None;
    }

    if contains_any(
        evidence,
        &[
            "runner binary not found",
            "executable missing",
            "no such file or directory",
            "command not found",
        ],
    ) {
        return Some(RunnerFailureMetadata::new(
            RunnerFailureClass::RunnerBinaryMissing,
            blocked_origin,
            FailureScope::LocalConfiguration,
            false,
            FailureClassifierCode::RunnerBinaryMissing,
        ));
    }
    if contains_any(
        evidence,
        &[
            "unauthorized",
            "authentication",
            "not authenticated",
            "login required",
            "invalid api key",
            "api key",
            "401",
        ],
    ) {
        return Some(RunnerFailureMetadata::new(
            RunnerFailureClass::AuthMissingOrInvalid,
            blocked_origin,
            FailureScope::LocalConfiguration,
            false,
            FailureClassifierCode::AuthMissingOrInvalid,
        ));
    }
    if contains_any(
        evidence,
        &["rate limit", "rate_limit", "too many requests", "429"],
    ) {
        return Some(RunnerFailureMetadata::new(
            RunnerFailureClass::ProviderRateLimited,
            blocked_origin,
            FailureScope::Provider,
            true,
            FailureClassifierCode::ProviderRateLimited,
        ));
    }
    if contains_any(
        evidence,
        &[
            "could not resolve",
            "temporary failure in name resolution",
            "dns",
            "network is unreachable",
            "connection refused",
            "connection reset",
            "no route to host",
            "offline",
            "internet",
        ],
    ) {
        return Some(RunnerFailureMetadata::new(
            RunnerFailureClass::NetworkUnavailable,
            blocked_origin,
            FailureScope::Environment,
            true,
            FailureClassifierCode::NetworkUnavailable,
        ));
    }
    if contains_any(
        evidence,
        &[
            "service unavailable",
            "temporarily unavailable",
            "provider unavailable",
            "provider overloaded",
            "overloaded",
            "503",
        ],
    ) {
        return Some(RunnerFailureMetadata::new(
            RunnerFailureClass::ProviderUnavailable,
            blocked_origin,
            FailureScope::Provider,
            true,
            FailureClassifierCode::ProviderUnavailable,
        ));
    }
    None
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn classification_for_failure_class(failure_class: RunnerFailureClass) -> RunnerFailureMetadata {
    match failure_class {
        RunnerFailureClass::RunnerTimeout => RunnerFailureMetadata::new(
            failure_class,
            BlockedOrigin::RunnerFailure,
            FailureScope::Environment,
            true,
            FailureClassifierCode::RunnerTimeout,
        ),
        RunnerFailureClass::NetworkUnavailable => RunnerFailureMetadata::new(
            failure_class,
            BlockedOrigin::RunnerFailure,
            FailureScope::Environment,
            true,
            FailureClassifierCode::NetworkUnavailable,
        ),
        RunnerFailureClass::ProviderUnavailable => RunnerFailureMetadata::new(
            failure_class,
            BlockedOrigin::RunnerFailure,
            FailureScope::Provider,
            true,
            FailureClassifierCode::ProviderUnavailable,
        ),
        RunnerFailureClass::ProviderRateLimited => RunnerFailureMetadata::new(
            failure_class,
            BlockedOrigin::RunnerFailure,
            FailureScope::Provider,
            true,
            FailureClassifierCode::ProviderRateLimited,
        ),
        RunnerFailureClass::RunnerBinaryMissing => RunnerFailureMetadata::new(
            failure_class,
            BlockedOrigin::RunnerFailure,
            FailureScope::LocalConfiguration,
            false,
            FailureClassifierCode::RunnerBinaryMissing,
        ),
        RunnerFailureClass::AuthMissingOrInvalid => RunnerFailureMetadata::new(
            failure_class,
            BlockedOrigin::RunnerFailure,
            FailureScope::LocalConfiguration,
            false,
            FailureClassifierCode::AuthMissingOrInvalid,
        ),
        RunnerFailureClass::MissingTerminalResult => RunnerFailureMetadata::new(
            failure_class,
            BlockedOrigin::StageTerminal,
            FailureScope::Contract,
            false,
            FailureClassifierCode::MissingTerminalResult,
        ),
        RunnerFailureClass::IllegalTerminalResult => RunnerFailureMetadata::new(
            failure_class,
            BlockedOrigin::StageTerminal,
            FailureScope::Contract,
            false,
            FailureClassifierCode::IllegalTerminalResult,
        ),
        RunnerFailureClass::ConflictingTerminalResults => RunnerFailureMetadata::new(
            failure_class,
            BlockedOrigin::StageTerminal,
            FailureScope::Contract,
            false,
            FailureClassifierCode::ConflictingTerminalResults,
        ),
        RunnerFailureClass::MissingRequiredArtifact => RunnerFailureMetadata::new(
            failure_class,
            BlockedOrigin::StageTerminal,
            FailureScope::Contract,
            false,
            FailureClassifierCode::MissingRequiredArtifact,
        ),
        RunnerFailureClass::RunnerTransportFailure => RunnerFailureMetadata::new(
            failure_class,
            BlockedOrigin::RunnerFailure,
            FailureScope::Unknown,
            false,
            FailureClassifierCode::UnclassifiedFailure,
        ),
    }
}

fn identity_mismatch_notes(request: &StageRunRequest, raw_result: &RunnerRawResult) -> Vec<String> {
    let mut notes = Vec::new();
    if raw_result.request_id != request.request_id {
        notes.push("raw result request_id does not match stage run request".to_owned());
    }
    if raw_result.run_id != request.run_id {
        notes.push("raw result run_id does not match stage run request".to_owned());
    }
    if raw_result.stage != request.stage {
        notes.push("raw result stage does not match stage run request".to_owned());
    }
    notes
}

fn failure_envelope(
    request: &StageRunRequest,
    raw_result: &RunnerRawResult,
    failure_class: RunnerFailureClass,
    notes: Vec<String>,
    detected_marker: Option<String>,
    artifact_paths: Vec<String>,
    classification: Option<RunnerFailureMetadata>,
) -> RunnerResult<StageResultEnvelope> {
    let blocked_terminal = blocked_terminal_for_plane(request.plane);
    let (detected_marker, raw_detected_marker) =
        failure_detected_marker(blocked_terminal, detected_marker);
    let report_artifact = resolved_report_artifact(request);
    let (work_item_kind, work_item_id) = request_result_identity(request)?;
    let mut metadata = request_metadata(request, "failure", Some(failure_class), false, raw_result);
    classification
        .unwrap_or_else(|| classification_for_failure_class(failure_class))
        .insert_into_map(&mut metadata);
    if let Some(raw_detected_marker) = raw_detected_marker {
        metadata.insert("raw_detected_marker".to_owned(), json!(raw_detected_marker));
    }
    let envelope = StageResultEnvelope {
        schema_version: "1.0".to_owned(),
        kind: "stage_result".to_owned(),
        run_id: request.run_id.clone(),
        plane: request.plane,
        stage: request.stage,
        node_id: request.node_id.clone(),
        stage_kind_id: request.stage_kind_id.clone(),
        work_item_kind,
        work_item_id,
        terminal_result: blocked_terminal,
        result_class: ResultClass::RecoverableFailure,
        summary_status_marker: "### BLOCKED".to_owned(),
        success: false,
        retryable: true,
        exit_code: raw_result.exit_code.unwrap_or(1),
        duration_seconds: raw_result.duration_seconds()?,
        prompt_artifact: None,
        report_artifact: report_artifact.clone(),
        artifact_paths: merge_artifact_paths(
            artifact_paths,
            [report_artifact, raw_result.event_log_path.clone()],
        ),
        detected_marker,
        stdout_path: raw_result.stdout_path.clone(),
        stderr_path: raw_result.stderr_path.clone(),
        runner_name: Some(raw_result.runner_name.clone()),
        model_name: raw_result.model_name.clone(),
        thinking_level: resolved_thinking_level(request, raw_result),
        model_reasoning_effort: raw_result
            .model_reasoning_effort
            .clone()
            .or_else(|| request.model_reasoning_effort.clone()),
        token_usage: raw_result.token_usage.clone(),
        notes,
        metadata,
        started_at: raw_result.started_at.clone(),
        completed_at: raw_result.ended_at.clone(),
    };
    validate_envelope(envelope)
}

fn failure_detected_marker(
    blocked_terminal: TerminalResult,
    detected_marker: Option<String>,
) -> (Option<String>, Option<String>) {
    let Some(marker) = detected_marker else {
        return (None, None);
    };
    if marker == blocked_terminal.marker() {
        (Some(marker), None)
    } else {
        (None, Some(marker))
    }
}

fn terminal_result_for_request(request: &StageRunRequest, token: &str) -> Option<TerminalResult> {
    if !request
        .legal_terminal_markers
        .iter()
        .any(|marker| marker == &format!("### {token}"))
    {
        return None;
    }
    terminal_result_for_plane(request.plane, token).ok()
}

fn resolve_result_class(
    request: &StageRunRequest,
    terminal_token: &str,
    raw_result_class: Option<ResultClass>,
) -> Option<ResultClass> {
    let allowed = request
        .allowed_result_classes_by_outcome
        .result_classes_for(terminal_token)?;
    match raw_result_class {
        None if allowed.len() == 1 => allowed.first().copied(),
        None if terminal_token == "BLOCKED" && allowed.contains(&ResultClass::Blocked) => {
            Some(ResultClass::Blocked)
        }
        None => None,
        Some(result_class) if allowed.contains(&result_class) => Some(result_class),
        Some(_) => None,
    }
}

fn request_result_identity(request: &StageRunRequest) -> RunnerResult<(WorkItemKind, String)> {
    match request.request_kind {
        RequestKind::ClosureTarget => {
            let Some(root_spec_id) = &request.closure_target_root_spec_id else {
                return Err(RunnerError::InvalidRequest {
                    message: "closure_target_root_spec_id is required for closure_target requests"
                        .to_owned(),
                });
            };
            Ok((WorkItemKind::Spec, root_spec_id.clone()))
        }
        RequestKind::LearningRequest => {
            let Some(work_item_id) = &request.active_work_item_id else {
                return Err(RunnerError::InvalidRequest {
                    message: "active_work_item_id is required for learning_request requests"
                        .to_owned(),
                });
            };
            Ok((WorkItemKind::LearningRequest, work_item_id.clone()))
        }
        RequestKind::ActiveWorkItem => {
            let (Some(kind), Some(id)) =
                (request.active_work_item_kind, &request.active_work_item_id)
            else {
                return Err(RunnerError::InvalidRequest {
                    message:
                        "active_work_item_kind and active_work_item_id are required to normalize stage results"
                            .to_owned(),
                });
            };
            Ok((kind, id.clone()))
        }
    }
}

fn merge_artifact_paths<const N: usize>(
    artifact_paths: Vec<String>,
    additional_artifacts: [Option<String>; N],
) -> Vec<String> {
    let mut merged = artifact_paths;
    for artifact in additional_artifacts.into_iter().flatten() {
        if !merged.contains(&artifact) {
            merged.push(artifact);
        }
    }
    merged
}

fn artifact_exists(run_dir: &str, candidate_path: &str) -> bool {
    let Ok(run_root) = Path::new(run_dir).canonicalize() else {
        return false;
    };
    let candidate = PathBuf::from(candidate_path);
    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        run_root.join(candidate)
    };
    let Ok(candidate) = candidate.canonicalize() else {
        return false;
    };
    candidate.starts_with(&run_root) && candidate.exists()
}

fn resolved_report_artifact(request: &StageRunRequest) -> Option<String> {
    [
        request.preferred_report_path.as_deref(),
        request.preferred_troubleshoot_report_path.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find(|candidate| artifact_exists(&request.run_dir, candidate))
    .map(str::to_owned)
}

fn raw_exit_kind(raw_result: &RunnerRawResult) -> RunnerExitKind {
    raw_result
        .observed_exit_kind
        .unwrap_or(raw_result.exit_kind)
}

fn raw_exit_code(raw_result: &RunnerRawResult) -> Option<i32> {
    raw_result.observed_exit_code.or(raw_result.exit_code)
}

fn timeout_reconciled(raw_result: &RunnerRawResult) -> bool {
    raw_result.observed_exit_kind == Some(RunnerExitKind::Timeout)
        && raw_result.exit_kind == RunnerExitKind::Completed
}

fn transport_reconciliation_notes(raw_result: &RunnerRawResult) -> Vec<String> {
    if timeout_reconciled(raw_result) {
        vec!["runner timeout was reconciled after a final terminal marker was captured".to_owned()]
    } else {
        Vec::new()
    }
}

fn resolved_thinking_level(
    request: &StageRunRequest,
    raw_result: &RunnerRawResult,
) -> Option<String> {
    raw_result
        .thinking_level
        .clone()
        .or_else(|| request.thinking_level.clone())
        .or_else(|| raw_result.model_reasoning_effort.clone())
        .or_else(|| request.model_reasoning_effort.clone())
}

fn request_metadata(
    request: &StageRunRequest,
    normalization_source: &str,
    failure_class: Option<RunnerFailureClass>,
    valid_terminal_result: bool,
    raw_result: &RunnerRawResult,
) -> Map<String, Value> {
    let mut metadata = Map::new();
    metadata.insert("request_id".to_owned(), json!(request.request_id));
    metadata.insert(
        "request_kind".to_owned(),
        json!(request.request_kind.as_str()),
    );
    metadata.insert("mode_id".to_owned(), json!(request.mode_id));
    metadata.insert(
        "compiled_plan_id".to_owned(),
        json!(request.compiled_plan_id),
    );
    metadata.insert(
        "closure_target_root_spec_id".to_owned(),
        json!(request.closure_target_root_spec_id),
    );
    metadata.insert(
        "closure_target_root_idea_id".to_owned(),
        json!(request.closure_target_root_idea_id),
    );
    metadata.insert(
        "preferred_rubric_path".to_owned(),
        json!(request.preferred_rubric_path),
    );
    metadata.insert(
        "preferred_verdict_path".to_owned(),
        json!(request.preferred_verdict_path),
    );
    metadata.insert(
        "preferred_report_path".to_owned(),
        json!(request.preferred_report_path),
    );
    metadata.insert(
        "active_work_item_kind".to_owned(),
        json!(request.active_work_item_kind.map(|kind| kind.as_str())),
    );
    metadata.insert(
        "active_work_item_id".to_owned(),
        json!(request.active_work_item_id),
    );
    metadata.insert(
        "active_work_item_path".to_owned(),
        json!(request.active_work_item_path),
    );
    metadata.insert(
        "skill_revision_evidence_path".to_owned(),
        json!(request.skill_revision_evidence_path),
    );
    metadata.insert("thinking_level".to_owned(), json!(request.thinking_level));
    metadata.insert(
        "model_reasoning_effort".to_owned(),
        json!(request.model_reasoning_effort),
    );
    metadata.insert(
        "normalization_source".to_owned(),
        json!(normalization_source),
    );
    metadata.insert(
        "failure_class".to_owned(),
        json!(failure_class.map(|failure_class| failure_class.as_str())),
    );
    metadata.insert(
        "valid_terminal_result".to_owned(),
        json!(valid_terminal_result),
    );
    metadata.insert(
        "raw_exit_kind".to_owned(),
        json!(raw_exit_kind(raw_result).as_str()),
    );
    metadata.insert("raw_exit_code".to_owned(), json!(raw_exit_code(raw_result)));
    metadata.insert(
        "timeout_reconciled".to_owned(),
        json!(timeout_reconciled(raw_result)),
    );
    metadata
}

fn validate_envelope(mut envelope: StageResultEnvelope) -> RunnerResult<StageResultEnvelope> {
    envelope
        .validate()
        .map_err(|error| RunnerError::StageResultEnvelope {
            message: error.to_string(),
        })?;
    Ok(envelope)
}
