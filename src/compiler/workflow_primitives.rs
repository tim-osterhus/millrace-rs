//! Compiler-owned workflow primitive loading, validation, and materialization.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
};

use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    assets::{CompilerAssetError, CompilerAssetResult},
    contracts::{
        CompiledWorkflowPrimitiveBundle, FrozenGraphPlanePlan, MaterializedGraphNodePlan,
        ResolvedAssetRef,
    },
};
use crate::contracts::{
    ArtifactContractDefinition, LaneConflictPolicyDefinition, LifecycleMutationPlanDefinition,
    Plane, PlaneQueueClaimPolicyDefinition, RequestContextProfileDefinition,
    RuntimeEffectHandlerDefinition, RuntimeEffectRuleDefinition, RuntimeFailurePolicyDefinition,
    RuntimeJsonContract, TerminalActionDefinition, WorkItemDocumentAdapterDefinition,
    WorkItemFamilyDefinition, WorkflowLaneDefinition, WorkflowPlaneSchedulerPolicyDefinition,
    WorkflowRecoveryPolicyDefinition, WorkspaceSchemaEpochDefinition,
};
use crate::workspace::WorkspacePaths;

/// Current workspace-schema epoch expected by the Rust v0.20.0 compiler authority surface.
pub const WORKSPACE_SCHEMA_EPOCH_ID: &str = "v0.20";

const RUNTIME_EFFECT_HANDLER_IMPLEMENTATIONS: &[&str] = &[
    "planner_disposition",
    "manager_blueprint_manifest_to_blueprint_drafts",
    "contractor_blueprint_candidate_persist",
    "evaluator_blueprint_approved_to_task",
    "evaluator_blueprint_rejected_to_draft_revision",
];

const ARTIFACT_CONTRACTS_PATH: &str = "registry/artifact_contracts/default_artifact_contracts.json";
const BUILTIN_DOCUMENT_ADAPTER_PATH: &str = "registry/document_adapters/builtin_markdown_v1.json";
const BLUEPRINT_DOCUMENT_ADAPTER_PATH: &str =
    "registry/document_adapters/blueprint_draft_markdown_v1.json";
const LIFECYCLE_MUTATION_PLANS_PATH: &str =
    "registry/lifecycle_mutation_plans/default_lifecycle_mutations.json";
const QUEUE_CLAIM_POLICIES_PATH: &str =
    "registry/queue_claim_policies/default_queue_claim_policies.json";
const REQUEST_CONTEXT_PROFILES_PATH: &str =
    "registry/request_context_profiles/default_request_context_profiles.json";
const RUNTIME_EFFECT_HANDLERS_PATH: &str =
    "registry/runtime_effect_handlers/default_effect_handlers.json";
const BLUEPRINT_EFFECT_RULES_PATH: &str =
    "registry/runtime_effect_rules/blueprint_effect_rules.json";
const PLANNER_EFFECT_RULES_PATH: &str = "registry/runtime_effect_rules/planner_effect_rules.json";
const RUNTIME_FAILURE_POLICIES_PATH: &str =
    "registry/runtime_failure_policies/default_runtime_failure_policies.json";
const TERMINAL_ACTIONS_PATH: &str = "registry/terminal_actions/default_terminal_actions.json";
const WORKSPACE_SCHEMA_EPOCH_PATH: &str = "registry/workspace_schema_epochs/current.json";
const RECOVERY_POLICIES_PATH: &str = "registry/recovery_policies/default_recovery_policies.json";

const WORK_ITEM_FAMILY_PATHS: &[&str] = &[
    "registry/work_item_families/blueprint_draft.json",
    "registry/work_item_families/incident.json",
    "registry/work_item_families/learning_request.json",
    "registry/work_item_families/probe.json",
    "registry/work_item_families/spec.json",
    "registry/work_item_families/task.json",
];

/// Resolved workflow primitive authority for one compile selection.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledWorkflowPrimitiveAuthority {
    pub workflow_primitives: CompiledWorkflowPrimitiveBundle,
    pub workflow_primitive_fingerprints: BTreeMap<String, String>,
    pub lane_policy: WorkflowPlaneSchedulerPolicyDefinition,
    pub resolved_assets: Vec<ResolvedAssetRef>,
}

/// Load, select, and validate workflow primitive authority for selected graph loops.
pub fn load_workflow_primitive_authority(
    paths: &WorkspacePaths,
    graph_loops: &[super::assets::ResolvedGraphLoopAsset],
) -> CompilerAssetResult<CompiledWorkflowPrimitiveAuthority> {
    let loaded = load_all_workflow_primitives(paths)?;
    let selected = select_workflow_primitives(&loaded, graph_loops);
    let lane_policy = build_lane_policy(&selected.queue_claim_policies, graph_loops)?;
    validate_workflow_primitive_authority(paths, &selected, &lane_policy, graph_loops)?;

    Ok(CompiledWorkflowPrimitiveAuthority {
        workflow_primitives: selected,
        workflow_primitive_fingerprints: loaded.fingerprints,
        lane_policy,
        resolved_assets: loaded.resolved_assets,
    })
}

/// Apply selected workflow primitive authority to one frozen graph plan.
pub fn materialize_workflow_primitive_graph_authority(
    graph: &mut FrozenGraphPlanePlan,
    primitives: &CompiledWorkflowPrimitiveBundle,
    lane_policy: &WorkflowPlaneSchedulerPolicyDefinition,
) -> Result<(), super::contracts::CompilerContractError> {
    let rules_by_node_outcome = rules_by_node_outcome(&primitives.runtime_effect_rules);
    let terminal_states_by_id: HashMap<_, _> = graph
        .terminal_states
        .iter()
        .map(|state| {
            (
                state.terminal_state_id.clone(),
                state.terminal_class.as_str().to_owned(),
            )
        })
        .collect();
    let compiled_transitions = graph.compiled_transitions.clone();

    for node in &mut graph.nodes {
        node.lane_id = lane_id_for_plane(lane_policy, graph.plane);
        node.request_context_profile_id =
            request_context_profile_id(node, &primitives.request_context_profiles);
        node.runtime_effect_rule_selections =
            runtime_effect_rule_ids_for_node(node, &primitives.runtime_effect_rules);
        node.terminal_action_mappings = terminal_action_mappings_for_node(
            node,
            &compiled_transitions,
            primitives,
            &rules_by_node_outcome,
            &terminal_states_by_id,
        )?;
        node.validate()?;
    }
    graph.validate()
}

#[derive(Default)]
struct LoadedWorkflowPrimitives {
    artifact_contracts: Vec<ArtifactContractDefinition>,
    request_context_profiles: Vec<RequestContextProfileDefinition>,
    work_item_families: Vec<WorkItemFamilyDefinition>,
    document_adapters: Vec<WorkItemDocumentAdapterDefinition>,
    queue_claim_policies: Vec<PlaneQueueClaimPolicyDefinition>,
    terminal_actions: Vec<TerminalActionDefinition>,
    lifecycle_mutation_plans: Vec<LifecycleMutationPlanDefinition>,
    runtime_effect_handlers: Vec<RuntimeEffectHandlerDefinition>,
    runtime_effect_rules: Vec<RuntimeEffectRuleDefinition>,
    recovery_policies: Vec<WorkflowRecoveryPolicyDefinition>,
    runtime_failure_policies: Vec<RuntimeFailurePolicyDefinition>,
    workspace_schema_epoch: Option<WorkspaceSchemaEpochDefinition>,
    resolved_assets: Vec<ResolvedAssetRef>,
    fingerprints: BTreeMap<String, String>,
}

fn load_all_workflow_primitives(
    paths: &WorkspacePaths,
) -> CompilerAssetResult<LoadedWorkflowPrimitives> {
    let mut loaded = LoadedWorkflowPrimitives::default();

    loaded.artifact_contracts = load_collection(
        paths,
        ARTIFACT_CONTRACTS_PATH,
        "artifact_contracts",
        &mut loaded,
    )?;
    let builtin_document_adapters = load_single(
        paths,
        BUILTIN_DOCUMENT_ADAPTER_PATH,
        "document_adapters",
        &mut loaded,
    )?;
    loaded.document_adapters.extend(builtin_document_adapters);
    let blueprint_document_adapters = load_single(
        paths,
        BLUEPRINT_DOCUMENT_ADAPTER_PATH,
        "document_adapters",
        &mut loaded,
    )?;
    loaded.document_adapters.extend(blueprint_document_adapters);
    loaded.lifecycle_mutation_plans = load_collection(
        paths,
        LIFECYCLE_MUTATION_PLANS_PATH,
        "lifecycle_mutation_plans",
        &mut loaded,
    )?;
    loaded.queue_claim_policies = load_collection(
        paths,
        QUEUE_CLAIM_POLICIES_PATH,
        "queue_claim_policies",
        &mut loaded,
    )?;
    loaded.request_context_profiles = load_collection(
        paths,
        REQUEST_CONTEXT_PROFILES_PATH,
        "request_context_profiles",
        &mut loaded,
    )?;
    loaded.runtime_effect_handlers = load_collection(
        paths,
        RUNTIME_EFFECT_HANDLERS_PATH,
        "runtime_effect_handlers",
        &mut loaded,
    )?;
    let planner_effect_rules = load_collection(
        paths,
        PLANNER_EFFECT_RULES_PATH,
        "runtime_effect_rules",
        &mut loaded,
    )?;
    loaded.runtime_effect_rules.extend(planner_effect_rules);
    let blueprint_effect_rules = load_collection(
        paths,
        BLUEPRINT_EFFECT_RULES_PATH,
        "runtime_effect_rules",
        &mut loaded,
    )?;
    loaded.runtime_effect_rules.extend(blueprint_effect_rules);
    loaded.runtime_failure_policies = load_collection(
        paths,
        RUNTIME_FAILURE_POLICIES_PATH,
        "runtime_failure_policies",
        &mut loaded,
    )?;
    loaded.terminal_actions = load_collection(
        paths,
        TERMINAL_ACTIONS_PATH,
        "terminal_actions",
        &mut loaded,
    )?;
    loaded.recovery_policies = load_collection(
        paths,
        RECOVERY_POLICIES_PATH,
        "recovery_policies",
        &mut loaded,
    )?;
    loaded.workspace_schema_epoch = Some(load_single_definition(
        paths,
        WORKSPACE_SCHEMA_EPOCH_PATH,
        "workspace_schema_epoch",
        &mut loaded,
    )?);

    for relative_path in WORK_ITEM_FAMILY_PATHS {
        let families = load_single(paths, relative_path, "work_item_families", &mut loaded)?;
        loaded.work_item_families.extend(families);
    }

    Ok(loaded)
}

fn select_workflow_primitives(
    loaded: &LoadedWorkflowPrimitives,
    graph_loops: &[super::assets::ResolvedGraphLoopAsset],
) -> CompiledWorkflowPrimitiveBundle {
    let selected_planes: HashSet<_> = graph_loops.iter().map(|graph| graph.plane).collect();
    let selected_nodes = selected_node_ids(graph_loops);
    let selected_profile_ids =
        selected_request_context_profile_ids(graph_loops, &loaded.request_context_profiles);
    let selected_rules: Vec<_> = loaded
        .runtime_effect_rules
        .iter()
        .filter(|rule| selected_nodes.contains(rule.source_node_id.as_str()))
        .cloned()
        .collect();
    let selected_handler_ids: BTreeSet<_> = selected_rules
        .iter()
        .map(|rule| rule.handler_id.as_str())
        .collect();
    let selected_recovery_policies = loaded
        .recovery_policies
        .iter()
        .filter(|policy| {
            policy
                .source_node_ids
                .iter()
                .any(|node_id| selected_nodes.contains(node_id.as_str()))
        })
        .cloned()
        .collect();

    CompiledWorkflowPrimitiveBundle {
        artifact_contracts: loaded.artifact_contracts.clone(),
        request_context_profiles: loaded
            .request_context_profiles
            .iter()
            .filter(|profile| selected_profile_ids.contains(profile.profile_id.as_str()))
            .cloned()
            .collect(),
        work_item_families: loaded.work_item_families.clone(),
        document_adapters: loaded.document_adapters.clone(),
        queue_claim_policies: loaded
            .queue_claim_policies
            .iter()
            .filter(|policy| selected_planes.contains(&policy.plane))
            .cloned()
            .collect(),
        terminal_actions: loaded.terminal_actions.clone(),
        lifecycle_mutation_plans: loaded.lifecycle_mutation_plans.clone(),
        runtime_effect_handlers: loaded
            .runtime_effect_handlers
            .iter()
            .filter(|handler| selected_handler_ids.contains(handler.handler_id.as_str()))
            .cloned()
            .collect(),
        runtime_effect_rules: selected_rules,
        recovery_policies: selected_recovery_policies,
        runtime_failure_policies: loaded.runtime_failure_policies.clone(),
        workspace_schema_epoch: loaded.workspace_schema_epoch.clone(),
    }
}

fn validate_workflow_primitive_authority(
    paths: &WorkspacePaths,
    primitives: &CompiledWorkflowPrimitiveBundle,
    lane_policy: &WorkflowPlaneSchedulerPolicyDefinition,
    graph_loops: &[super::assets::ResolvedGraphLoopAsset],
) -> CompilerAssetResult<()> {
    validate_work_item_family_document_adapters(paths, primitives)?;
    validate_runtime_effect_rules(paths, primitives)?;
    validate_lifecycle_and_terminal_actions(paths, primitives)?;
    validate_lane_policy(paths, lane_policy)?;
    validate_workspace_schema_epoch(paths, primitives)?;
    validate_graph_workflow_references(paths, primitives, graph_loops)?;
    Ok(())
}

fn validate_work_item_family_document_adapters(
    paths: &WorkspacePaths,
    primitives: &CompiledWorkflowPrimitiveBundle,
) -> CompilerAssetResult<()> {
    let families: HashMap<_, _> = primitives
        .work_item_families
        .iter()
        .map(|family| (family.family_id.as_str(), family))
        .collect();
    let adapters: HashMap<_, _> = primitives
        .document_adapters
        .iter()
        .map(|adapter| (adapter.adapter_id.as_str(), adapter))
        .collect();
    for family in &primitives.work_item_families {
        let Some(adapter) = adapters.get(family.document_adapter_id.as_str()) else {
            return Err(invalid_primitive(
                paths,
                "invalid work-item family document adapter reference",
            ));
        };
        if !adapter.family_ids.iter().any(|id| id == &family.family_id) {
            return Err(invalid_primitive(
                paths,
                "document adapter family_ids must include referencing work-item family",
            ));
        }
    }
    for adapter in &primitives.document_adapters {
        for family_id in &adapter.family_ids {
            if !families.contains_key(family_id.as_str()) {
                return Err(invalid_primitive(
                    paths,
                    "document adapter references unknown work-item family",
                ));
            }
        }
    }
    Ok(())
}

fn validate_runtime_effect_rules(
    paths: &WorkspacePaths,
    primitives: &CompiledWorkflowPrimitiveBundle,
) -> CompilerAssetResult<()> {
    let handlers: HashSet<_> = primitives
        .runtime_effect_handlers
        .iter()
        .map(|handler| handler.handler_id.as_str())
        .collect();
    let implemented: HashSet<_> = RUNTIME_EFFECT_HANDLER_IMPLEMENTATIONS
        .iter()
        .copied()
        .collect();
    let families: HashSet<_> = primitives
        .work_item_families
        .iter()
        .map(|family| family.family_id.as_str())
        .collect();
    let mut bindings = BTreeMap::new();

    for handler in &primitives.runtime_effect_handlers {
        if !implemented.contains(handler.handler_id.as_str()) {
            return Err(invalid_primitive(
                paths,
                format!(
                    "runtime effect handler {} has no packaged Rust implementation",
                    handler.handler_id
                ),
            ));
        }
        for family_id in &handler.allowed_source_families {
            if !families.contains(family_id.as_str()) {
                return Err(invalid_primitive(
                    paths,
                    "runtime effect handler references unknown source family",
                ));
            }
        }
    }

    for rule in &primitives.runtime_effect_rules {
        if !handlers.contains(rule.handler_id.as_str()) {
            return Err(invalid_primitive(
                paths,
                format!(
                    "runtime effect rule {} references unknown handler {}",
                    rule.rule_id, rule.handler_id
                ),
            ));
        }
        if let Some(destination_family_id) = &rule.destination_family_id {
            if !families.contains(destination_family_id.as_str()) {
                return Err(invalid_primitive(
                    paths,
                    "runtime effect rule references unknown destination family",
                ));
            }
        }
        for outcome in &rule.on_outcomes {
            let key = (rule.source_node_id.as_str(), outcome.as_str());
            if let Some(existing) = bindings.insert(key, rule.rule_id.as_str()) {
                return Err(invalid_primitive(
                    paths,
                    format!(
                        "duplicate runtime-effect binding for {} {outcome}: {existing} and {}",
                        rule.source_node_id, rule.rule_id
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn validate_lifecycle_and_terminal_actions(
    paths: &WorkspacePaths,
    primitives: &CompiledWorkflowPrimitiveBundle,
) -> CompilerAssetResult<()> {
    let lifecycle_plan_ids: HashSet<_> = primitives
        .lifecycle_mutation_plans
        .iter()
        .map(|plan| plan.plan_id.as_str())
        .collect();
    let terminal_action_ids: HashSet<_> = primitives
        .terminal_actions
        .iter()
        .map(|action| action.terminal_action_id.as_str())
        .collect();
    for action in &primitives.terminal_actions {
        if !action.non_mutating {
            let Some(plan_id) = &action.lifecycle_mutation_plan_id else {
                return Err(invalid_primitive(
                    paths,
                    format!(
                        "terminal action {} is mutating without a lifecycle mutation plan",
                        action.terminal_action_id
                    ),
                ));
            };
            if !lifecycle_plan_ids.contains(plan_id.as_str()) {
                return Err(invalid_primitive(
                    paths,
                    "terminal action references unknown lifecycle mutation plan",
                ));
            }
        }
    }
    for rule in &primitives.runtime_effect_rules {
        if let Some(plan_id) = &rule.lifecycle_mutation_plan_id {
            if !lifecycle_plan_ids.contains(plan_id.as_str()) {
                return Err(invalid_primitive(
                    paths,
                    "runtime effect rule references unknown lifecycle mutation plan",
                ));
            }
            if terminal_action_for_lifecycle_plan(primitives, plan_id).is_none() {
                return Err(invalid_primitive(
                    paths,
                    "runtime effect rule lifecycle plan has no terminal action mapping",
                ));
            }
        }
    }
    for action in &primitives.terminal_actions {
        for effect_rule_id in &action.effect_rule_ids {
            if !primitives
                .runtime_effect_rules
                .iter()
                .any(|rule| &rule.rule_id == effect_rule_id)
            {
                return Err(invalid_primitive(
                    paths,
                    "terminal action references unknown runtime effect rule",
                ));
            }
        }
        if !terminal_action_ids.contains(action.terminal_action_id.as_str()) {
            return Err(invalid_primitive(paths, "invalid terminal action"));
        }
    }
    Ok(())
}

fn validate_lane_policy(
    paths: &WorkspacePaths,
    lane_policy: &WorkflowPlaneSchedulerPolicyDefinition,
) -> CompilerAssetResult<()> {
    let conflict_ids: HashSet<_> = lane_policy
        .lane_conflict_policies
        .iter()
        .map(|policy| policy.policy_id.as_str())
        .collect();
    for lane in &lane_policy.lanes {
        let Some(conflict_policy_id) = &lane.conflict_policy_id else {
            return Err(invalid_primitive(
                paths,
                format!("lane {} is missing lane conflict policy", lane.lane_id),
            ));
        };
        if !conflict_ids.contains(conflict_policy_id.as_str()) {
            return Err(invalid_primitive(
                paths,
                format!(
                    "lane {} references unknown lane conflict policy {}",
                    lane.lane_id, conflict_policy_id
                ),
            ));
        }
    }
    Ok(())
}

fn validate_workspace_schema_epoch(
    paths: &WorkspacePaths,
    primitives: &CompiledWorkflowPrimitiveBundle,
) -> CompilerAssetResult<()> {
    let Some(epoch) = &primitives.workspace_schema_epoch else {
        return Err(invalid_primitive(
            paths,
            "missing workspace schema epoch authority",
        ));
    };
    if epoch.epoch_id != WORKSPACE_SCHEMA_EPOCH_ID
        || epoch.minimum_supported_epoch_id != WORKSPACE_SCHEMA_EPOCH_ID
    {
        return Err(invalid_primitive(
            paths,
            format!(
                "stale workspace schema epoch: expected {WORKSPACE_SCHEMA_EPOCH_ID}, got {}",
                epoch.epoch_id
            ),
        ));
    }
    Ok(())
}

fn validate_graph_workflow_references(
    paths: &WorkspacePaths,
    primitives: &CompiledWorkflowPrimitiveBundle,
    graph_loops: &[super::assets::ResolvedGraphLoopAsset],
) -> CompilerAssetResult<()> {
    let family_ids: HashSet<_> = primitives
        .work_item_families
        .iter()
        .map(|family| family.family_id.as_str())
        .collect();
    let claim_policies_by_plane: HashMap<_, _> = primitives
        .queue_claim_policies
        .iter()
        .map(|policy| (policy.plane, policy))
        .collect();
    for graph_asset in graph_loops {
        let graph = &graph_asset.graph_loop;
        for entry in &graph.entry_nodes {
            if !family_ids.contains(entry.entry_key.as_str()) {
                return Err(invalid_primitive(
                    paths,
                    format!(
                        "graph {} entry {} references unknown work-item family",
                        graph.loop_id,
                        entry.entry_key.as_str()
                    ),
                ));
            }
        }
        let Some(claim_policy) = claim_policies_by_plane.get(&graph.plane) else {
            return Err(invalid_primitive(
                paths,
                format!(
                    "missing queue claim policy for plane {}",
                    graph.plane.as_str()
                ),
            ));
        };
        for family_id in &claim_policy.family_order {
            if !family_ids.contains(family_id.as_str()) {
                return Err(invalid_primitive(
                    paths,
                    "queue claim policy references unknown work-item family",
                ));
            }
        }
        if claim_policy.empty_behavior == "check_completion" && graph.completion_behavior.is_none()
        {
            return Err(invalid_primitive(
                paths,
                format!(
                    "graph {} is selected for completion checks but has no completion behavior",
                    graph.loop_id
                ),
            ));
        }
        if graph.loop_id == "planning.blueprint" {
            for required in [
                "blueprint_draft",
                "manager_blueprint",
                "contractor_blueprint",
                "evaluator_blueprint",
                "mechanic_blueprint",
            ] {
                let has_entry = graph
                    .entry_nodes
                    .iter()
                    .any(|entry| entry.entry_key.as_str() == required);
                let has_node = graph
                    .nodes
                    .iter()
                    .any(|node| node.node_id == required || node.stage_kind_id == required);
                if !(has_entry || has_node) {
                    return Err(invalid_primitive(
                        paths,
                        "blueprint graph mode reference drift",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn build_lane_policy(
    claim_policies: &[PlaneQueueClaimPolicyDefinition],
    graph_loops: &[super::assets::ResolvedGraphLoopAsset],
) -> CompilerAssetResult<WorkflowPlaneSchedulerPolicyDefinition> {
    let selected_planes: Vec<_> = graph_loops.iter().map(|graph| graph.plane).collect();
    let selected_lane_ids: Vec<_> = selected_planes
        .iter()
        .map(|plane| format!("{}.main", plane.as_str()))
        .collect();
    let mut claim_policies_by_plane = HashMap::new();
    let mut lanes = Vec::new();
    let mut lane_conflict_policies = Vec::new();
    let mut completion_check_order = Vec::new();

    for plane in selected_planes {
        let claim_policy = claim_policies
            .iter()
            .find(|policy| policy.plane == plane)
            .ok_or_else(|| CompilerAssetError::MissingReferencedAsset {
                asset_family: "queue_claim_policy",
                logical_id: format!("queue_claim_policy:{}", plane.as_str()),
                path: PathBuf::from(QUEUE_CLAIM_POLICIES_PATH),
            })?
            .clone();
        if claim_policy.empty_behavior == "check_completion" {
            completion_check_order.push(plane);
        }
        let lane_id = format!("{}.main", plane.as_str());
        let conflict_policy_id = format!("{}.single_writer", plane.as_str());
        lanes.push(WorkflowLaneDefinition {
            schema_version: "1.0".to_owned(),
            kind: "workflow_lane".to_owned(),
            lane_id: lane_id.clone(),
            plane,
            allowed_family_ids: claim_policy.family_order.clone(),
            claim_policy_id: claim_policy.policy_id.clone(),
            max_active_runs: 1,
            one_active_scope: "plane".to_owned(),
            partition_selector_id: None,
            mutation_lock_scope: "plane".to_owned(),
            result_application_policy: "single_writer_serialized".to_owned(),
            conflict_policy_id: Some(conflict_policy_id.clone()),
        });
        lane_conflict_policies.push(LaneConflictPolicyDefinition {
            schema_version: "1.0".to_owned(),
            kind: "lane_conflict_policy".to_owned(),
            policy_id: conflict_policy_id,
            lane_ids: vec![lane_id.clone()],
            concurrent_with_lane_ids: selected_lane_ids.clone(),
            conflict_scopes: vec!["workspace_write".to_owned()],
            lock_acquisition_order: vec![lane_id],
            release_policy: "after_result_application".to_owned(),
            missing_lock_policy: "reject_compile".to_owned(),
        });
        claim_policies_by_plane.insert(plane, claim_policy);
    }

    Ok(WorkflowPlaneSchedulerPolicyDefinition {
        schema_version: "1.0".to_owned(),
        kind: "workflow_plane_scheduler_policy".to_owned(),
        policy_id: "compiled.default".to_owned(),
        plane_order: graph_loops.iter().map(|graph| graph.plane).collect(),
        concurrency_policy_id: None,
        lanes,
        claim_policies_by_plane,
        completion_check_order,
        experimental_multi_lane: false,
        lane_conflict_policies,
    })
}

fn selected_node_ids(graph_loops: &[super::assets::ResolvedGraphLoopAsset]) -> HashSet<&str> {
    graph_loops
        .iter()
        .flat_map(|graph| graph.graph_loop.nodes.iter())
        .map(|node| node.node_id.as_str())
        .collect()
}

fn selected_request_context_profile_ids<'a>(
    graph_loops: &'a [super::assets::ResolvedGraphLoopAsset],
    profiles: &'a [RequestContextProfileDefinition],
) -> BTreeSet<&'a str> {
    let profile_ids: HashSet<_> = profiles
        .iter()
        .map(|profile| profile.profile_id.as_str())
        .collect();
    let mut selected = BTreeSet::new();
    for graph in graph_loops {
        for node in &graph.graph_loop.nodes {
            let stage_profile = format!("{}.default", node.stage_kind_id);
            if profile_ids.contains(stage_profile.as_str()) {
                selected.insert(profile_id_str(profiles, &stage_profile));
            } else if graph.plane == Plane::Learning && profile_ids.contains("learning.default") {
                selected.insert(profile_id_str(profiles, "learning.default"));
            }
        }
    }
    selected
}

fn profile_id_str<'a>(
    profiles: &'a [RequestContextProfileDefinition],
    profile_id: &str,
) -> &'a str {
    profiles
        .iter()
        .find(|profile| profile.profile_id == profile_id)
        .map(|profile| profile.profile_id.as_str())
        .expect("profile id came from loaded profiles")
}

fn lane_id_for_plane(
    lane_policy: &WorkflowPlaneSchedulerPolicyDefinition,
    plane: Plane,
) -> Option<String> {
    lane_policy
        .lanes
        .iter()
        .find(|lane| lane.plane == plane)
        .map(|lane| lane.lane_id.clone())
}

fn request_context_profile_id(
    node: &MaterializedGraphNodePlan,
    profiles: &[RequestContextProfileDefinition],
) -> Option<String> {
    let profile_ids: HashSet<_> = profiles
        .iter()
        .map(|profile| profile.profile_id.as_str())
        .collect();
    let stage_profile = format!("{}.default", node.stage_kind_id);
    if profile_ids.contains(stage_profile.as_str()) {
        return Some(stage_profile);
    }
    if node.plane == Plane::Learning && profile_ids.contains("learning.default") {
        return Some("learning.default".to_owned());
    }
    None
}

fn runtime_effect_rule_ids_for_node(
    node: &MaterializedGraphNodePlan,
    rules: &[RuntimeEffectRuleDefinition],
) -> Vec<String> {
    rules
        .iter()
        .filter(|rule| rule.source_node_id == node.node_id)
        .map(|rule| rule.rule_id.clone())
        .collect()
}

fn rules_by_node_outcome(
    rules: &[RuntimeEffectRuleDefinition],
) -> BTreeMap<(String, String), Vec<&RuntimeEffectRuleDefinition>> {
    let mut by_key: BTreeMap<(String, String), Vec<&RuntimeEffectRuleDefinition>> = BTreeMap::new();
    for rule in rules {
        for outcome in &rule.on_outcomes {
            by_key
                .entry((rule.source_node_id.clone(), outcome.clone()))
                .or_default()
                .push(rule);
        }
    }
    by_key
}

fn terminal_action_mappings_for_node(
    node: &MaterializedGraphNodePlan,
    compiled_transitions: &[super::contracts::CompiledGraphTransitionPlan],
    primitives: &CompiledWorkflowPrimitiveBundle,
    rules_by_node_outcome: &BTreeMap<(String, String), Vec<&RuntimeEffectRuleDefinition>>,
    terminal_states_by_id: &HashMap<String, String>,
) -> Result<BTreeMap<String, String>, super::contracts::CompilerContractError> {
    let mut mappings = BTreeMap::new();
    for ((source_node_id, outcome), rules) in rules_by_node_outcome {
        if source_node_id != &node.node_id {
            continue;
        }
        for rule in rules {
            if let Some(plan_id) = &rule.lifecycle_mutation_plan_id {
                let action_id = terminal_action_for_lifecycle_plan(primitives, plan_id)
                    .ok_or_else(|| super::contracts::CompilerContractError::InvalidDocument {
                        message: format!(
                            "runtime effect rule {} has no terminal action for lifecycle plan {plan_id}",
                            rule.rule_id
                        ),
                    })?;
                mappings.insert(outcome.clone(), action_id);
            }
        }
    }

    for transition in compiled_transitions {
        if transition.source_node_id != node.node_id {
            continue;
        }
        if mappings.contains_key(&transition.outcome) {
            continue;
        }
        let Some(terminal_state_id) = &transition.terminal_state_id else {
            continue;
        };
        let Some(terminal_class) = terminal_states_by_id.get(terminal_state_id) else {
            continue;
        };
        let action_id = terminal_action_for_class(primitives, terminal_class)
            .ok_or_else(|| super::contracts::CompilerContractError::InvalidDocument {
                message: format!(
                    "terminal state {terminal_state_id} has no terminal action for class {terminal_class}"
                ),
            })?;
        mappings.insert(transition.outcome.clone(), action_id);
    }
    Ok(mappings)
}

fn terminal_action_for_lifecycle_plan(
    primitives: &CompiledWorkflowPrimitiveBundle,
    plan_id: &str,
) -> Option<String> {
    primitives
        .terminal_actions
        .iter()
        .find(|action| action.lifecycle_mutation_plan_id.as_deref() == Some(plan_id))
        .map(|action| action.terminal_action_id.clone())
}

fn terminal_action_for_class(
    primitives: &CompiledWorkflowPrimitiveBundle,
    terminal_class: &str,
) -> Option<String> {
    primitives
        .terminal_actions
        .iter()
        .find(|action| action.terminal_class == terminal_class)
        .map(|action| action.terminal_action_id.clone())
}

fn load_collection<T>(
    paths: &WorkspacePaths,
    relative_path: &str,
    collection_id: &str,
    loaded: &mut LoadedWorkflowPrimitives,
) -> CompilerAssetResult<Vec<T>>
where
    T: RuntimeJsonContract + DeserializeOwned,
{
    let value = load_json_value(paths, relative_path, collection_id, loaded)?;
    let definitions = value
        .get("definitions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            invalid_primitive(paths, format!("{relative_path} must contain definitions"))
        })?;
    definitions
        .iter()
        .cloned()
        .map(|value| {
            T::from_json_value(value).map_err(|error| CompilerAssetError::Contract {
                path: paths.runtime_root.join(relative_path),
                artifact: T::ARTIFACT,
                message: error.to_string(),
            })
        })
        .collect()
}

fn load_single<T>(
    paths: &WorkspacePaths,
    relative_path: &str,
    collection_id: &str,
    loaded: &mut LoadedWorkflowPrimitives,
) -> CompilerAssetResult<Vec<T>>
where
    T: RuntimeJsonContract + DeserializeOwned,
{
    Ok(vec![load_single_definition(
        paths,
        relative_path,
        collection_id,
        loaded,
    )?])
}

fn load_single_definition<T>(
    paths: &WorkspacePaths,
    relative_path: &str,
    collection_id: &str,
    loaded: &mut LoadedWorkflowPrimitives,
) -> CompilerAssetResult<T>
where
    T: RuntimeJsonContract + DeserializeOwned,
{
    let value = load_json_value(paths, relative_path, collection_id, loaded)?;
    T::from_json_value(value).map_err(|error| CompilerAssetError::Contract {
        path: paths.runtime_root.join(relative_path),
        artifact: T::ARTIFACT,
        message: error.to_string(),
    })
}

fn load_json_value(
    paths: &WorkspacePaths,
    relative_path: &str,
    collection_id: &str,
    loaded: &mut LoadedWorkflowPrimitives,
) -> CompilerAssetResult<Value> {
    let path = paths.runtime_root.join(relative_path);
    let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
    let content_sha256 = sha256_hex(&bytes);
    loaded.resolved_assets.push(ResolvedAssetRef {
        asset_family: "workflow_primitive".to_owned(),
        logical_id: format!(
            "workflow_primitive:{collection_id}:{}",
            primitive_asset_id(relative_path)
        ),
        compile_time_path: relative_path.to_owned(),
        content_sha256: content_sha256.clone(),
    });
    loaded.fingerprints.insert(
        format!("{collection_id}.{}", primitive_asset_id(relative_path)),
        content_sha256.clone(),
    );
    let collection_fingerprint = loaded
        .fingerprints
        .get(collection_id)
        .map(|previous| {
            sha256_hex(format!("{previous}\n{relative_path}\n{content_sha256}").as_bytes())
        })
        .unwrap_or_else(|| content_sha256.clone());
    loaded
        .fingerprints
        .insert(collection_id.to_owned(), collection_fingerprint);
    serde_json::from_slice(&bytes).map_err(|error| CompilerAssetError::Contract {
        path,
        artifact: "workflow_primitive",
        message: error.to_string(),
    })
}

fn primitive_asset_id(relative_path: &str) -> String {
    Path::new(relative_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(relative_path)
        .replace('-', "_")
}

fn invalid_primitive(paths: &WorkspacePaths, message: impl Into<String>) -> CompilerAssetError {
    CompilerAssetError::InvalidReferencedAsset {
        asset_family: "workflow_primitive",
        logical_id: "workflow_primitives".to_owned(),
        path: paths.runtime_root.join("registry"),
        message: message.into(),
    }
}

fn io_error(path: &Path, error: io::Error) -> CompilerAssetError {
    CompilerAssetError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

fn sha256_hex(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    let mut rendered = String::with_capacity(digest.len() * 2);
    for byte in digest {
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}
