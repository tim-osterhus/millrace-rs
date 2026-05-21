//! Runtime failure-policy interpretation for effect-handler failures.

use crate::contracts::{Plane, RuntimeEffectMutationPhase, RuntimeFailurePolicyDefinition};

/// Runtime-effect failure data matched against compiled runtime failure policies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEffectFailurePolicyInput {
    pub failure_class: Option<String>,
    pub mutation_phase: RuntimeEffectMutationPhase,
    pub handler_id: Option<String>,
    pub source_node_id: Option<String>,
    pub source_terminal_state_id: Option<String>,
    pub source_plane: Option<Plane>,
    pub source_family_id: Option<String>,
    pub created_paths: Vec<String>,
    pub message: Option<String>,
}

/// Result of matching one runtime effect failure against a compiled policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFailurePolicyInterpretation {
    pub policy_id: String,
    pub action: String,
    pub failure_class: String,
    pub target_node_id: Option<String>,
    pub target_terminal_state_id: Option<String>,
    pub max_attempts: Option<u64>,
    pub incident_severity: Option<String>,
}

/// Resolve a runtime-effect failure through ordered compiled policies.
#[must_use]
pub fn interpret_runtime_effect_failure_policy(
    policies: &[RuntimeFailurePolicyDefinition],
    failure: &RuntimeEffectFailurePolicyInput,
) -> Option<RuntimeFailurePolicyInterpretation> {
    let failure_class = failure.failure_class.as_deref()?.trim();
    let handler_id = failure.handler_id.as_deref()?.trim();
    let source_node_id = failure.source_node_id.as_deref()?.trim();
    let _source_plane = failure.source_plane?;
    let source_family_id = failure.source_family_id.as_deref()?.trim();
    if failure_class.is_empty()
        || handler_id.is_empty()
        || source_node_id.is_empty()
        || source_family_id.is_empty()
    {
        return None;
    }

    for policy in policies {
        if !policy_matches_effect_failure(policy, failure) {
            continue;
        }
        if failure.mutation_phase == RuntimeEffectMutationPhase::PartialMutation
            && policy.action == "route_to_node"
        {
            continue;
        }
        return Some(RuntimeFailurePolicyInterpretation {
            policy_id: policy.policy_id.clone(),
            action: policy.action.clone(),
            failure_class: failure_class.to_owned(),
            target_node_id: policy.target_node_id.clone(),
            target_terminal_state_id: policy.target_terminal_state_id.clone(),
            max_attempts: policy.max_attempts,
            incident_severity: policy.incident_severity.clone(),
        });
    }
    None
}

fn policy_matches_effect_failure(
    policy: &RuntimeFailurePolicyDefinition,
    failure: &RuntimeEffectFailurePolicyInput,
) -> bool {
    string_list_matches_required(&policy.applies_to_origins, Some("runtime_effect"))
        && plane_list_matches_required(&policy.applies_to_planes, failure.source_plane)
        && string_list_matches_optional(
            &policy.applies_to_families,
            failure.source_family_id.as_deref(),
        )
        && string_list_matches_optional(
            &policy.applies_to_failure_classes,
            failure.failure_class.as_deref(),
        )
        && phase_list_matches_optional(&policy.applies_to_mutation_phases, failure.mutation_phase)
        && string_list_matches_optional(
            &policy.applies_to_handler_ids,
            failure.handler_id.as_deref(),
        )
        && string_list_matches_optional(
            &policy.applies_to_source_node_ids,
            failure.source_node_id.as_deref(),
        )
        && string_list_matches_optional(
            &policy.applies_to_source_terminal_state_ids,
            failure.source_terminal_state_id.as_deref(),
        )
}

fn string_list_matches_required(values: &[String], candidate: Option<&str>) -> bool {
    let Some(candidate) = candidate else {
        return false;
    };
    values.iter().any(|value| value == candidate)
}

fn plane_list_matches_required(values: &[Plane], candidate: Option<Plane>) -> bool {
    let Some(candidate) = candidate else {
        return false;
    };
    values.contains(&candidate)
}

fn string_list_matches_optional(values: &[String], candidate: Option<&str>) -> bool {
    values.is_empty()
        || candidate.is_some_and(|candidate| values.iter().any(|value| value == candidate))
}

fn phase_list_matches_optional(
    values: &[RuntimeEffectMutationPhase],
    candidate: RuntimeEffectMutationPhase,
) -> bool {
    values.is_empty() || values.contains(&candidate)
}
