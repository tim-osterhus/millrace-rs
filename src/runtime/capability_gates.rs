//! Pre-dispatch runtime gates for execution capability grants.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::{
    contracts::{
        CapabilityDecisionState, CapabilityEnforcementMode, CapabilityEvidenceStatus,
        CapabilitySupportDecision, CapabilitySupportState, ExecutionCapabilityGrant, Timestamp,
    },
    runners::{RunnerExitKind, RunnerRawResult, StageRunnerAdapter},
    workspace::{WorkspacePaths, atomic_write_text},
};

use super::{
    RuntimeTickError, RuntimeTickResult, StageRunRequest,
    approvals::{
        ApprovalStorageError, ExecutionCapabilityApprovalRequest,
        ExecutionCapabilityApprovalStatus, ensure_execution_capability_approval,
        find_approval_for_grant,
    },
    write_runtime_event,
};

/// Evaluates support for one execution capability grant.
pub type CapabilitySupportEvaluator<'a> =
    dyn Fn(&ExecutionCapabilityGrant, &StageRunRequest) -> CapabilitySupportDecision + 'a;

/// One blocked grant and the normalized runtime-policy failure class it caused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityGateBlockedGrant {
    pub grant_id: String,
    pub failure_class: String,
    pub reason: String,
}

/// Result of checking one stage request before dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityGateResult {
    pub allowed: bool,
    pub request: StageRunRequest,
    pub support_decisions: Vec<CapabilitySupportDecision>,
    pub approval_ids: Vec<String>,
    pub blocked_grant_ids: Vec<String>,
    pub blocked_grants: Vec<CapabilityGateBlockedGrant>,
    pub failure_class: Option<String>,
    pub reason: String,
}

/// Evaluate required grants before invoking a runner or marking the stage running.
pub fn evaluate_stage_request_capabilities(
    paths: &WorkspacePaths,
    request: &StageRunRequest,
    support_evaluator: &CapabilitySupportEvaluator<'_>,
    now: &Timestamp,
) -> RuntimeTickResult<CapabilityGateResult> {
    let mut support_decisions = Vec::new();
    let mut approval_ids = Vec::new();
    let mut blocked_grants = Vec::new();

    for grant in &request.execution_capability_grants {
        if !grant.required {
            continue;
        }

        match grant.decision_state {
            CapabilityDecisionState::Denied => {
                push_blocked(
                    &mut blocked_grants,
                    grant,
                    "capability_grant_denied",
                    "grant decision denied",
                );
            }
            CapabilityDecisionState::Unsupported => {
                push_blocked(
                    &mut blocked_grants,
                    grant,
                    "capability_grant_unsupported",
                    "grant decision unsupported",
                );
            }
            CapabilityDecisionState::ApprovalRequired => {
                let approval = approval_for_request(paths, request, grant, now)?;
                approval_ids.push(approval.approval_id.clone());
                if approval.status != ExecutionCapabilityApprovalStatus::Approved {
                    push_blocked(
                        &mut blocked_grants,
                        grant,
                        "capability_approval_required",
                        "approval is not resolved as approved",
                    );
                }
            }
            CapabilityDecisionState::Granted => {
                if matches!(
                    grant.evidence_status,
                    CapabilityEvidenceStatus::Missing | CapabilityEvidenceStatus::Violated
                ) {
                    push_blocked(
                        &mut blocked_grants,
                        grant,
                        "capability_evidence_missing",
                        "grant evidence status is missing or violated",
                    );
                    continue;
                }

                let support = support_evaluator(grant, request);
                support
                    .validate()
                    .map_err(|error| RuntimeTickError::InvalidState {
                        message: format!("capability support decision failed validation: {error}"),
                    })?;
                if support.support_state == CapabilitySupportState::Unsupported {
                    push_blocked(
                        &mut blocked_grants,
                        grant,
                        "capability_grant_unsupported",
                        "runner reported unsupported capability grant",
                    );
                } else if grant.enforcement_mode != CapabilityEnforcementMode::AdvisoryOnly
                    && support.enforcement_mode == CapabilityEnforcementMode::AdvisoryOnly
                {
                    push_blocked(
                        &mut blocked_grants,
                        grant,
                        "capability_grant_unsupported",
                        "runner could not provide required non-advisory enforcement",
                    );
                } else if required_evidence_missing(grant, &support) {
                    push_blocked(
                        &mut blocked_grants,
                        grant,
                        "capability_evidence_missing",
                        "runner support is missing required capability evidence",
                    );
                }
                support_decisions.push(support);
            }
        }
    }

    let blocked_grant_ids = blocked_grants
        .iter()
        .map(|blocked| blocked.grant_id.clone())
        .collect::<Vec<_>>();
    let failure_class = failure_class_for_blocks(&blocked_grants);
    let allowed = blocked_grants.is_empty();
    let mut updated_request = request.clone();
    updated_request.capability_support_decisions = support_decisions.clone();
    let reason = if allowed {
        "all required capability grants satisfied".to_owned()
    } else {
        format!(
            "blocked capability grants: {}",
            blocked_grant_ids.join(", ")
        )
    };

    Ok(CapabilityGateResult {
        allowed,
        request: updated_request,
        support_decisions,
        approval_ids,
        blocked_grant_ids,
        blocked_grants,
        failure_class,
        reason,
    })
}

/// Evaluate a request using the runner adapter support hook.
pub fn evaluate_stage_request_capabilities_with_runner(
    paths: &WorkspacePaths,
    request: &StageRunRequest,
    runner: &impl StageRunnerAdapter,
    now: &Timestamp,
) -> RuntimeTickResult<CapabilityGateResult> {
    evaluate_stage_request_capabilities(
        paths,
        request,
        &|grant, request| runner.evaluate_capability_grant(grant, request),
        now,
    )
}

/// Persist the gate artifact and runtime event for one evaluated request.
pub fn record_capability_gate_result(
    paths: &WorkspacePaths,
    request: &StageRunRequest,
    gate_result: &CapabilityGateResult,
    now: &Timestamp,
) -> RuntimeTickResult<PathBuf> {
    let run_dir = PathBuf::from(&request.run_dir);
    fs::create_dir_all(&run_dir).map_err(|error| RuntimeTickError::Io {
        path: run_dir.clone(),
        message: error.to_string(),
    })?;
    let artifact_path = run_dir.join(format!("capability_gate.{}.json", request.request_id));
    let artifact = CapabilityGateArtifact {
        schema_version: "1.0",
        kind: "capability_gate_result",
        request_id: &request.request_id,
        run_id: &request.run_id,
        plane: request.plane.as_str(),
        stage: request.stage.as_str(),
        node_id: &request.node_id,
        stage_kind_id: &request.stage_kind_id,
        allowed: gate_result.allowed,
        failure_class: gate_result.failure_class.as_deref(),
        reason: &gate_result.reason,
        blocked_grant_ids: &gate_result.blocked_grant_ids,
        blocked_grants: &gate_result.blocked_grants,
        approval_ids: &gate_result.approval_ids,
        support_decisions: &gate_result.support_decisions,
    };
    let rendered = serde_json::to_string_pretty(&artifact).map_err(|error| {
        RuntimeTickError::InvalidState {
            message: error.to_string(),
        }
    })? + "\n";
    atomic_write_text(&artifact_path, &rendered).map_err(|error| RuntimeTickError::Io {
        path: artifact_path.clone(),
        message: error.to_string(),
    })?;

    write_runtime_event(
        paths,
        "capability_gate_evaluated",
        capability_gate_event_data(request, gate_result, &artifact_path),
        now,
    )?;
    Ok(artifact_path)
}

/// Build a raw runner result for a gate block without invoking the runner.
pub fn capability_gate_failure_result(
    request: &StageRunRequest,
    gate_result: &CapabilityGateResult,
    now: &Timestamp,
) -> RuntimeTickResult<RunnerRawResult> {
    let raw_result = RunnerRawResult {
        request_id: request.request_id.clone(),
        run_id: request.run_id.clone(),
        stage: request.stage,
        runner_name: request
            .runner_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("runtime")
            .to_owned(),
        model_name: request.model_name.clone(),
        thinking_level: request.thinking_level.clone(),
        model_reasoning_effort: request.model_reasoning_effort.clone(),
        exit_kind: RunnerExitKind::RunnerError,
        exit_code: Some(1),
        observed_exit_kind: None,
        observed_exit_code: None,
        stdout_path: None,
        stderr_path: None,
        terminal_result_path: None,
        event_log_path: None,
        token_usage: None,
        failure_capability_class: Some(
            gate_result
                .failure_class
                .clone()
                .unwrap_or_else(|| "capability_gate_blocked".to_owned()),
        ),
        capability_support_decisions: gate_result.support_decisions.clone(),
        capability_evidence_refs: Vec::new(),
        missing_capability_evidence_refs: gate_result.blocked_grant_ids.clone(),
        started_at: now.clone(),
        ended_at: now.clone(),
    };
    raw_result.validate().map_err(RuntimeTickError::from)?;
    Ok(raw_result)
}

fn approval_for_request(
    paths: &WorkspacePaths,
    request: &StageRunRequest,
    grant: &ExecutionCapabilityGrant,
    now: &Timestamp,
) -> RuntimeTickResult<super::approvals::ExecutionCapabilityApproval> {
    if let Some(existing) =
        find_approval_for_grant(paths, &request.run_id, &request.request_id, &grant.grant_id)
            .map_err(approval_error)?
    {
        return Ok(existing);
    }

    ensure_execution_capability_approval(
        paths,
        ExecutionCapabilityApprovalRequest {
            request_id: &request.request_id,
            run_id: &request.run_id,
            plane: request.plane,
            node_id: &request.node_id,
            stage_kind_id: &request.stage_kind_id,
            work_item_kind: request.active_work_item_kind,
            work_item_id: request.active_work_item_id.as_deref(),
            grant,
            now,
            requested_by: "runtime",
        },
    )
    .map_err(approval_error)
}

fn push_blocked(
    blocked_grants: &mut Vec<CapabilityGateBlockedGrant>,
    grant: &ExecutionCapabilityGrant,
    failure_class: &str,
    reason: &str,
) {
    if blocked_grants
        .iter()
        .any(|blocked| blocked.grant_id == grant.grant_id)
    {
        return;
    }
    blocked_grants.push(CapabilityGateBlockedGrant {
        grant_id: grant.grant_id.clone(),
        failure_class: failure_class.to_owned(),
        reason: reason.to_owned(),
    });
}

fn required_evidence_missing(
    grant: &ExecutionCapabilityGrant,
    support: &CapabilitySupportDecision,
) -> bool {
    let available = support
        .evidence_available
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    grant
        .evidence_requirements
        .iter()
        .any(|requirement| !available.contains(requirement.as_str()))
}

fn failure_class_for_blocks(blocked_grants: &[CapabilityGateBlockedGrant]) -> Option<String> {
    for failure_class in [
        "capability_grant_denied",
        "capability_approval_required",
        "capability_grant_unsupported",
        "capability_evidence_missing",
    ] {
        if blocked_grants
            .iter()
            .any(|blocked| blocked.failure_class == failure_class)
        {
            return Some(failure_class.to_owned());
        }
    }
    None
}

fn capability_gate_event_data(
    request: &StageRunRequest,
    gate_result: &CapabilityGateResult,
    artifact_path: &Path,
) -> Map<String, Value> {
    let mut data = Map::new();
    data.insert("request_id".to_owned(), json!(request.request_id));
    data.insert("run_id".to_owned(), json!(request.run_id));
    data.insert("plane".to_owned(), json!(request.plane.as_str()));
    data.insert("stage".to_owned(), json!(request.stage.as_str()));
    data.insert("node_id".to_owned(), json!(request.node_id));
    data.insert("stage_kind_id".to_owned(), json!(request.stage_kind_id));
    data.insert("allowed".to_owned(), json!(gate_result.allowed));
    data.insert("failure_class".to_owned(), json!(gate_result.failure_class));
    data.insert(
        "blocked_grant_ids".to_owned(),
        json!(gate_result.blocked_grant_ids),
    );
    data.insert("approval_ids".to_owned(), json!(gate_result.approval_ids));
    data.insert(
        "capability_gate_artifact_path".to_owned(),
        json!(artifact_path.display().to_string()),
    );
    data
}

fn approval_error(error: ApprovalStorageError) -> RuntimeTickError {
    RuntimeTickError::InvalidState {
        message: error.to_string(),
    }
}

#[derive(Serialize)]
struct CapabilityGateArtifact<'a> {
    schema_version: &'static str,
    kind: &'static str,
    request_id: &'a str,
    run_id: &'a str,
    plane: &'a str,
    stage: &'a str,
    node_id: &'a str,
    stage_kind_id: &'a str,
    allowed: bool,
    failure_class: Option<&'a str>,
    reason: &'a str,
    blocked_grant_ids: &'a [String],
    blocked_grants: &'a [CapabilityGateBlockedGrant],
    approval_ids: &'a [String],
    support_decisions: &'a [CapabilitySupportDecision],
}
