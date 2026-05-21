use serde_json::Value;

use millrace_ai::contracts::{Plane, RuntimeEffectMutationPhase, RuntimeFailurePolicyDefinition};
use millrace_ai::{RuntimeEffectFailurePolicyInput, interpret_runtime_effect_failure_policy};

fn bundled_runtime_failure_policies() -> Vec<RuntimeFailurePolicyDefinition> {
    let payload: Value = serde_json::from_str(include_str!(
        "../src/assets/baseline/registry/runtime_failure_policies/default_runtime_failure_policies.json"
    ))
    .unwrap();
    payload["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(RuntimeFailurePolicyDefinition::from_json_value)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn blueprint_failure_input(
    failure_class: &str,
    mutation_phase: RuntimeEffectMutationPhase,
) -> RuntimeEffectFailurePolicyInput {
    RuntimeEffectFailurePolicyInput {
        failure_class: Some(failure_class.to_owned()),
        mutation_phase,
        handler_id: Some("manager_blueprint_manifest_to_blueprint_drafts".to_owned()),
        source_node_id: Some("manager_blueprint".to_owned()),
        source_terminal_state_id: Some("manager_blueprint_complete".to_owned()),
        source_plane: Some(Plane::Planning),
        source_family_id: Some("spec".to_owned()),
        created_paths: Vec::new(),
        message: None,
    }
}

#[test]
fn runtime_effect_failure_policy_routes_pre_mutation_blueprint_artifact_repairs() {
    let policies = bundled_runtime_failure_policies();
    let resolution = interpret_runtime_effect_failure_policy(
        &policies,
        &blueprint_failure_input(
            "blueprint_manifest_missing",
            RuntimeEffectMutationPhase::PreMutation,
        ),
    )
    .unwrap();

    assert_eq!(
        resolution.policy_id,
        "manager_blueprint_pre_mutation_artifact_repair"
    );
    assert_eq!(resolution.action, "route_to_node");
    assert_eq!(
        resolution.target_node_id.as_deref(),
        Some("mechanic_blueprint")
    );
    assert_eq!(resolution.failure_class, "blueprint_manifest_missing");
}

#[test]
fn runtime_effect_failure_policy_blocks_partial_mutations_even_when_route_policy_matches_class() {
    let policies = bundled_runtime_failure_policies();
    let resolution = interpret_runtime_effect_failure_policy(
        &policies,
        &blueprint_failure_input(
            "blueprint_partial_mutation",
            RuntimeEffectMutationPhase::PartialMutation,
        ),
    )
    .unwrap();

    assert_eq!(
        resolution.policy_id,
        "manager_blueprint_partial_mutation_conservative_block"
    );
    assert_eq!(resolution.action, "block_source_work_item");
    assert_eq!(resolution.target_node_id, None);
}

#[test]
fn runtime_effect_failure_policy_requires_handler_id_match() {
    let policies = bundled_runtime_failure_policies();
    let mut input = blueprint_failure_input(
        "blueprint_manifest_missing",
        RuntimeEffectMutationPhase::PreMutation,
    );
    input.handler_id = Some("planner_disposition".to_owned());

    assert!(interpret_runtime_effect_failure_policy(&policies, &input).is_none());
}

#[test]
fn runtime_effect_failure_policy_requires_runtime_effect_origin_match() {
    let policies = bundled_runtime_failure_policies();
    let mut non_runtime_effect_origin_policy = policies
        .iter()
        .find(|policy| policy.policy_id == "manager_blueprint_pre_mutation_artifact_repair")
        .unwrap()
        .clone();
    non_runtime_effect_origin_policy.policy_id =
        "manager_blueprint_non_runtime_effect_origin".to_owned();
    non_runtime_effect_origin_policy.applies_to_origins = vec!["stage_terminal".to_owned()];
    let input = blueprint_failure_input(
        "blueprint_manifest_missing",
        RuntimeEffectMutationPhase::PreMutation,
    );

    assert!(
        interpret_runtime_effect_failure_policy(&[non_runtime_effect_origin_policy], &input)
            .is_none()
    );
}

#[test]
fn runtime_effect_failure_policy_requires_source_terminal_state_match() {
    let policies = bundled_runtime_failure_policies();
    let mut input = blueprint_failure_input(
        "blueprint_manifest_missing",
        RuntimeEffectMutationPhase::PreMutation,
    );
    input.source_terminal_state_id = Some("manager_blueprint_blocked".to_owned());

    assert!(interpret_runtime_effect_failure_policy(&policies, &input).is_none());
}
