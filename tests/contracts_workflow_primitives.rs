use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use millrace_ai::contracts::{
    ArtifactContractDefinition, ArtifactFormat, LaneConflictPolicyDefinition,
    LifecycleMutationPlanDefinition, OperatorControlCapabilityDefinition, Plane,
    PlaneQueueClaimPolicyDefinition, RequestContextProfileDefinition, RequestContextRenderPlan,
    RuntimeEffectHandlerDefinition, RuntimeEffectRuleDefinition, RuntimeFailurePolicyDefinition,
    RuntimeJsonContract, TerminalActionDefinition, WorkItemDocumentAdapterDefinition,
    WorkItemFamilyDefinition, WorkItemKind, WorkflowLaneDefinition,
    WorkflowPlaneSchedulerPolicyDefinition, WorkspaceSchemaEpochDefinition,
    family_id_for_work_item_kind, legacy_work_item_kind_for_family_id,
    plane_for_work_item_family_id,
};

fn definitions<T>(raw: &str) -> Vec<T>
where
    T: RuntimeJsonContract + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let value: Value = serde_json::from_str(raw).unwrap();
    let items = value.get("definitions").cloned().unwrap_or(value);
    match items {
        Value::Array(values) => values
            .into_iter()
            .map(|value| round_trip::<T>(value))
            .collect(),
        value => vec![round_trip::<T>(value)],
    }
}

fn round_trip<T>(value: Value) -> T
where
    T: RuntimeJsonContract + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let decoded = T::from_json_value(value).unwrap();
    let serialized = serde_json::to_value(&decoded).unwrap();
    let decoded_again = T::from_json_value(serialized).unwrap();
    assert_eq!(decoded_again, decoded);
    decoded
}

#[test]
fn workflow_primitive_registry_assets_parse_as_inert_contract_data() {
    let artifact_contracts = definitions::<ArtifactContractDefinition>(include_str!(
        "../src/assets/baseline/registry/artifact_contracts/default_artifact_contracts.json"
    ));
    assert!(
        artifact_contracts
            .iter()
            .any(|contract| contract.artifact_id == "blueprint_manifest"
                && contract.preferred_format == ArtifactFormat::Json)
    );

    let families = [
        include_str!("../src/assets/baseline/registry/work_item_families/task.json"),
        include_str!("../src/assets/baseline/registry/work_item_families/spec.json"),
        include_str!("../src/assets/baseline/registry/work_item_families/probe.json"),
        include_str!("../src/assets/baseline/registry/work_item_families/incident.json"),
        include_str!("../src/assets/baseline/registry/work_item_families/learning_request.json"),
        include_str!("../src/assets/baseline/registry/work_item_families/blueprint_draft.json"),
    ]
    .into_iter()
    .flat_map(definitions::<WorkItemFamilyDefinition>)
    .collect::<Vec<_>>();
    assert_eq!(
        families
            .iter()
            .map(|family| family.family_id.as_str())
            .collect::<Vec<_>>(),
        [
            "task",
            "spec",
            "probe",
            "incident",
            "learning_request",
            "blueprint_draft"
        ]
    );
    assert_eq!(
        families
            .iter()
            .find(|family| family.family_id == "blueprint_draft")
            .unwrap()
            .queue_dirs
            .queue,
        "blueprints/drafts/queue"
    );

    let adapters = [
        include_str!("../src/assets/baseline/registry/document_adapters/builtin_markdown_v1.json"),
        include_str!(
            "../src/assets/baseline/registry/document_adapters/blueprint_draft_markdown_v1.json"
        ),
    ]
    .into_iter()
    .flat_map(definitions::<WorkItemDocumentAdapterDefinition>)
    .collect::<Vec<_>>();
    assert!(
        adapters
            .iter()
            .any(|adapter| adapter.family_ids == ["blueprint_draft"])
    );

    let claim_policies = definitions::<PlaneQueueClaimPolicyDefinition>(include_str!(
        "../src/assets/baseline/registry/queue_claim_policies/default_queue_claim_policies.json"
    ));
    assert!(
        claim_policies
            .iter()
            .any(|policy| policy.policy_id == "planning.default"
                && policy.family_order.contains(&"blueprint_draft".to_owned()))
    );

    let terminal_actions = definitions::<TerminalActionDefinition>(include_str!(
        "../src/assets/baseline/registry/terminal_actions/default_terminal_actions.json"
    ));
    assert!(terminal_actions.iter().any(|action| action.non_mutating));

    let lifecycle_plans = definitions::<LifecycleMutationPlanDefinition>(include_str!(
        "../src/assets/baseline/registry/lifecycle_mutation_plans/default_lifecycle_mutations.json"
    ));
    assert!(
        lifecycle_plans
            .iter()
            .any(|plan| plan.plan_id == "approve_blueprint_draft_after_effect")
    );

    let profiles = definitions::<RequestContextProfileDefinition>(include_str!(
        "../src/assets/baseline/registry/request_context_profiles/default_request_context_profiles.json"
    ));
    assert!(
        profiles
            .iter()
            .any(|profile| profile.profile_id == "contractor_blueprint.default")
    );

    let handlers = definitions::<RuntimeEffectHandlerDefinition>(include_str!(
        "../src/assets/baseline/registry/runtime_effect_handlers/default_effect_handlers.json"
    ));
    assert!(handlers.iter().any(|handler| handler.handler_id
        == "evaluator_blueprint_approved_to_task"
        && handler.creates_work_items));

    let mut rules = definitions::<RuntimeEffectRuleDefinition>(include_str!(
        "../src/assets/baseline/registry/runtime_effect_rules/blueprint_effect_rules.json"
    ));
    rules.extend(definitions::<RuntimeEffectRuleDefinition>(include_str!(
        "../src/assets/baseline/registry/runtime_effect_rules/planner_effect_rules.json"
    )));
    assert!(
        rules
            .iter()
            .any(|rule| rule.destination_family_id.as_deref() == Some("blueprint_draft"))
    );

    let failure_policies = definitions::<RuntimeFailurePolicyDefinition>(include_str!(
        "../src/assets/baseline/registry/runtime_failure_policies/default_runtime_failure_policies.json"
    ));
    assert!(failure_policies.iter().any(|policy| {
        policy
            .applies_to_handler_ids
            .contains(&"manager_blueprint_manifest_to_blueprint_drafts".to_owned())
    }));

    let epoch = definitions::<WorkspaceSchemaEpochDefinition>(include_str!(
        "../src/assets/baseline/registry/workspace_schema_epochs/current.json"
    ))
    .remove(0);
    assert_eq!(epoch.epoch_id, "v0.20");
}

#[test]
fn lane_context_and_operator_control_primitives_round_trip_as_contract_data() {
    let render_plan = round_trip::<RequestContextRenderPlan>(json!({
        "schema_version": "1.0",
        "kind": "request_context_render_plan",
        "render_plan_id": "stage_request.default.v1",
        "profile_id": "builder.default",
        "bundle_schema_version": "1.0",
        "section_order": ["active_work_item", "runtime_snapshot"],
        "artifact_ref_policy": "visible_refs_only",
        "redaction_policy_id": "operator_private_paths",
        "max_inline_bytes_by_role": {"operator": 4096},
        "missing_optional_provider_policy": "omit"
    }));
    assert_eq!(render_plan.profile_id, "builder.default");

    let lane = round_trip::<WorkflowLaneDefinition>(json!({
        "schema_version": "1.0",
        "kind": "workflow_lane",
        "lane_id": "planning.main",
        "plane": "planning",
        "accepted_family_ids": ["spec", "blueprint_draft"],
        "claim_policy_id": "planning.default",
        "max_active_runs": 1,
        "one_active_scope": "plane",
        "mutation_lock_scope": "plane",
        "result_application_policy": "single_writer_serialized"
    }));
    assert_eq!(lane.allowed_family_ids, ["spec", "blueprint_draft"]);

    let conflict = round_trip::<LaneConflictPolicyDefinition>(json!({
        "schema_version": "1.0",
        "kind": "lane_conflict_policy",
        "policy_id": "planning.execution.serialized",
        "lane_ids": ["planning.main"],
        "concurrent_with_lane_ids": ["execution.main"],
        "conflict_scopes": ["runtime_state"],
        "lock_acquisition_order": ["planning.main", "execution.main"],
        "release_policy": "after_result_application",
        "missing_lock_policy": "reject_compile"
    }));
    assert_eq!(conflict.policy_id, "planning.execution.serialized");

    let scheduler = round_trip::<WorkflowPlaneSchedulerPolicyDefinition>(json!({
        "schema_version": "1.0",
        "kind": "workflow_plane_scheduler_policy",
        "policy_id": "runtime.default",
        "plane_order": ["planning", "execution"],
        "lanes": [serde_json::to_value(&lane).unwrap()],
        "claim_policies_by_plane": {
            "planning": {
                "schema_version": "1.0",
                "kind": "plane_queue_claim_policy",
                "policy_id": "planning.default",
                "plane": "planning",
                "family_order": ["spec", "blueprint_draft"],
                "closure_lineage_policy": "defer_unrelated",
                "empty_behavior": "idle"
            }
        },
        "completion_check_order": ["planning"],
        "experimental_multi_lane": false,
        "lane_conflict_policies": [serde_json::to_value(&conflict).unwrap()]
    }));
    assert_eq!(scheduler.plane_order, [Plane::Planning, Plane::Execution]);

    let operator_capability = round_trip::<OperatorControlCapabilityDefinition>(json!({
        "schema_version": "1.0",
        "kind": "operator_control_capability",
        "capability_id": "cancel.blueprint_draft",
        "action": "cancel",
        "target_type": "work_item",
        "plane": "planning",
        "family_ids": ["blueprint_draft"],
        "lane_ids": ["planning.main"],
        "allowed_lifecycle_states": ["queued", "active", "blocked"]
    }));
    assert_eq!(operator_capability.family_ids, ["blueprint_draft"]);
}

#[test]
fn work_refs_preserve_legacy_family_kind_mapping() {
    assert_eq!(
        family_id_for_work_item_kind(WorkItemKind::BlueprintDraft),
        "blueprint_draft"
    );
    assert_eq!(
        legacy_work_item_kind_for_family_id("blueprint_draft").unwrap(),
        Some(WorkItemKind::BlueprintDraft)
    );
    assert_eq!(
        plane_for_work_item_family_id("blueprint_draft")
            .unwrap()
            .unwrap()
            .as_str(),
        "planning"
    );
    assert_eq!(
        legacy_work_item_kind_for_family_id("blueprint_packet").unwrap(),
        None
    );
}

#[test]
fn runtime_effect_handler_references_remain_unvalidated_contract_data() {
    let unknown_handler_rule = round_trip::<RuntimeEffectRuleDefinition>(json!({
        "schema_version": "1.0",
        "kind": "runtime_effect_rule",
        "rule_id": "test_unknown_handler",
        "effect_operation_id": "test_unknown_handler",
        "source_node_id": "planner",
        "on_outcomes": ["PLANNER_COMPLETE"],
        "handler_id": "unknown_handler",
        "duplicate_policy": "fail",
        "partial_commit_policy": "block_source",
        "replay_policy": "resume_idempotently",
        "lineage_policy": "preserve_root",
        "applies_before_route": true
    }));
    let duplicate_handler_rule = round_trip::<RuntimeEffectRuleDefinition>(json!({
        "schema_version": "1.0",
        "kind": "runtime_effect_rule",
        "rule_id": "test_duplicate_handler",
        "effect_operation_id": "test_duplicate_handler",
        "source_node_id": "planner",
        "on_outcomes": ["PLANNER_COMPLETE"],
        "handler_id": unknown_handler_rule.handler_id,
        "duplicate_policy": "fail",
        "partial_commit_policy": "block_source",
        "replay_policy": "resume_idempotently",
        "lineage_policy": "preserve_root",
        "applies_before_route": true
    }));

    assert_eq!(duplicate_handler_rule.handler_id, "unknown_handler");
}
