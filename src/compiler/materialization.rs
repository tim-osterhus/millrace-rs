//! Deterministic materialization of resolved compiler assets into frozen plans.

use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::{
    assets::{
        CompilerAssetError, EffectiveCompileConfig, ResolvedCompileAssetSet, resolve_compile_assets,
    },
    contracts::{
        CompiledGraphCompletionEntryPlan, CompiledGraphEntryPlan, CompiledGraphResumePolicyPlan,
        CompiledGraphThresholdPolicyPlan, CompiledGraphTransitionPlan, CompiledRunPlan,
        CompilerContractError, FrozenGraphPlanePlan, GraphLoopCounterName, GraphLoopDefinition,
        GraphLoopEdgeDefinition, GraphLoopResumePolicyDefinition,
        GraphLoopThresholdPolicyDefinition, MaterializedGraphNodePlan, ModeDefinition,
        RegisteredStageKindDefinition,
    },
};
use crate::{
    contracts::{Plane, StageName, Timestamp},
    workspace::WorkspacePaths,
};

/// Default timeout applied to materialized stage nodes when no node or config override exists.
pub const DEFAULT_STAGE_TIMEOUT_SECONDS: u64 = 3600;

/// Result type for compiler graph materialization.
pub type CompilerMaterializationResult<T> = Result<T, CompilerMaterializationError>;

/// Failures produced while freezing resolved compile assets into executable graph plans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompilerMaterializationError {
    /// Asset resolution failed before graph materialization could begin.
    Asset(CompilerAssetError),
    /// A resolved mode did not include a required graph binding.
    MissingLoopBinding {
        /// Runtime plane with the missing binding.
        plane: Plane,
    },
    /// A resolved asset set did not include a required graph.
    MissingGraph {
        /// Runtime plane with the missing graph.
        plane: Plane,
        /// Expected loop id when known.
        loop_id: Option<String>,
    },
    /// A graph node referenced a stage kind absent from the selected registry.
    UnknownStageKind {
        /// Graph node id.
        node_id: String,
        /// Referenced stage kind id.
        stage_kind_id: String,
    },
    /// A graph entry referenced a node absent from the materialized node set.
    UnknownEntryNode {
        /// Entry key value.
        entry_key: String,
        /// Referenced node id.
        node_id: String,
    },
    /// A planning completion behavior referenced a node absent from the materialized node set.
    UnknownCompletionTarget {
        /// Completion target node id.
        node_id: String,
    },
    /// A graph reference contradicts the selected stage-kind registry.
    InvalidStageKindReference {
        /// Graph loop id.
        graph_loop_id: String,
        /// Human-readable failure reason.
        message: String,
    },
    /// A learning trigger references a stage outside the selected graph set.
    InvalidLearningTrigger {
        /// Rule id.
        rule_id: String,
        /// Human-readable failure reason.
        message: String,
    },
    /// A frozen compiler contract failed validation after materialization.
    Contract {
        /// Artifact type being validated.
        artifact: &'static str,
        /// Contract error text.
        message: String,
    },
}

impl fmt::Display for CompilerMaterializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Asset(error) => write!(f, "{error}"),
            Self::MissingLoopBinding { plane } => {
                write!(f, "mode does not bind a {} graph loop", plane.as_str())
            }
            Self::MissingGraph { plane, loop_id } => {
                if let Some(loop_id) = loop_id {
                    write!(
                        f,
                        "resolved assets do not include {} graph loop {loop_id}",
                        plane.as_str()
                    )
                } else {
                    write!(
                        f,
                        "resolved assets do not include a {} graph loop",
                        plane.as_str()
                    )
                }
            }
            Self::UnknownStageKind {
                node_id,
                stage_kind_id,
            } => write!(
                f,
                "graph node {node_id} references unknown stage_kind_id {stage_kind_id}"
            ),
            Self::UnknownEntryNode { entry_key, node_id } => write!(
                f,
                "graph entry {entry_key} references unknown node_id {node_id}"
            ),
            Self::UnknownCompletionTarget { node_id } => {
                write!(
                    f,
                    "completion behavior references unknown target_node_id {node_id}"
                )
            }
            Self::InvalidStageKindReference {
                graph_loop_id,
                message,
            } => write!(
                f,
                "graph loop {graph_loop_id} has invalid stage-kind reference: {message}"
            ),
            Self::InvalidLearningTrigger { rule_id, message } => {
                write!(f, "learning trigger {rule_id} is invalid: {message}")
            }
            Self::Contract { artifact, message } => {
                write!(f, "invalid materialized {artifact}: {message}")
            }
        }
    }
}

impl std::error::Error for CompilerMaterializationError {}

impl From<CompilerAssetError> for CompilerMaterializationError {
    fn from(value: CompilerAssetError) -> Self {
        Self::Asset(value)
    }
}

/// Resolve workspace compiler assets and materialize a Python-compatible frozen run plan.
pub fn compile_compiled_run_plan(
    paths: &WorkspacePaths,
    requested_mode_id: Option<&str>,
    compiled_at: Timestamp,
) -> CompilerMaterializationResult<CompiledRunPlan> {
    let resolved = resolve_compile_assets(paths, requested_mode_id)?;
    materialize_compiled_run_plan(&resolved, compiled_at)
}

/// Materialize a frozen run plan from an already resolved compile asset set.
pub fn materialize_compiled_run_plan(
    resolved: &ResolvedCompileAssetSet,
    compiled_at: Timestamp,
) -> CompilerMaterializationResult<CompiledRunPlan> {
    let stage_kinds: HashMap<_, _> = resolved
        .stage_kinds
        .iter()
        .map(|stage_kind| {
            (
                stage_kind.stage_kind_id.clone(),
                stage_kind.definition.clone(),
            )
        })
        .collect();

    let mut graphs_by_plane = HashMap::new();
    for graph_asset in &resolved.graph_loops {
        let graph_plan = materialize_graph_plane_plan(
            &graph_asset.graph_loop,
            &resolved.mode,
            &resolved.config,
            &stage_kinds,
        )?;
        graphs_by_plane.insert(graph_plan.plane, graph_plan);
    }

    let selected_stages =
        selected_stages_for_graph_loops(resolved.graph_loops.iter().map(|asset| &asset.graph_loop));
    validate_learning_trigger_rules(&resolved.mode, &selected_stages)?;

    let execution_loop_id = required_loop_id(&resolved.mode, Plane::Execution)?;
    let planning_loop_id = required_loop_id(&resolved.mode, Plane::Planning)?;
    let learning_loop_id = resolved
        .mode
        .loop_ids_by_plane
        .get(&Plane::Learning)
        .cloned();

    let execution_graph = graph_for_plane(&graphs_by_plane, Plane::Execution, &execution_loop_id)?;
    let planning_graph = graph_for_plane(&graphs_by_plane, Plane::Planning, &planning_loop_id)?;
    let learning_graph = match &learning_loop_id {
        Some(loop_id) => Some(graph_for_plane(&graphs_by_plane, Plane::Learning, loop_id)?),
        None => None,
    };

    let compiled_plan_id = build_compiled_plan_id(
        &resolved.mode_id,
        &resolved.mode.loop_ids_by_plane,
        &graphs_by_plane,
        &resolved.mode.concurrency_policy,
        &resolved.mode.learning_trigger_rules,
    );
    let source_refs = build_graph_source_refs(
        &resolved.mode_id,
        &graphs_by_plane,
        planning_graph.completion_behavior.is_some(),
    );

    let mut plan = CompiledRunPlan {
        schema_version: "1.0".to_owned(),
        kind: "compiled_run_plan".to_owned(),
        compiled_plan_id,
        compile_input_fingerprint: resolved.compile_input_fingerprint.clone(),
        mode_id: resolved.mode_id.clone(),
        loop_ids_by_plane: resolved.mode.loop_ids_by_plane.clone(),
        execution_loop_id,
        planning_loop_id,
        learning_loop_id,
        graphs_by_plane,
        execution_graph,
        planning_graph,
        learning_graph,
        concurrency_policy: resolved.mode.concurrency_policy.clone(),
        learning_trigger_rules: resolved.mode.learning_trigger_rules.clone(),
        compiled_at,
        resolved_assets: resolved.resolved_assets.clone(),
        source_refs,
    };
    plan.validate()
        .map_err(|error| contract_error("compiled_run_plan", error))?;
    Ok(plan)
}

/// Materialize one frozen graph-plane plan from a resolved graph loop and stage-kind registry.
pub fn materialize_graph_plane_plan(
    graph_loop: &GraphLoopDefinition,
    mode: &ModeDefinition,
    config: &EffectiveCompileConfig,
    stage_kinds: &HashMap<String, RegisteredStageKindDefinition>,
) -> CompilerMaterializationResult<FrozenGraphPlanePlan> {
    validate_graph_stage_kind_semantics(graph_loop, stage_kinds)?;

    let mut nodes = Vec::with_capacity(graph_loop.nodes.len());
    for node in &graph_loop.nodes {
        nodes.push(materialize_graph_node_plan(
            node,
            graph_loop.plane,
            mode,
            config,
            stage_kinds,
        )?);
    }
    let node_plan_by_id: HashMap<_, _> = nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect();

    let mut compiled_entries = Vec::with_capacity(graph_loop.entry_nodes.len());
    for entry in &graph_loop.entry_nodes {
        let Some(node_plan) = node_plan_by_id.get(entry.node_id.as_str()) else {
            return Err(CompilerMaterializationError::UnknownEntryNode {
                entry_key: entry.entry_key.as_str().to_owned(),
                node_id: entry.node_id.clone(),
            });
        };
        compiled_entries.push(CompiledGraphEntryPlan {
            entry_key: entry.entry_key,
            node_id: entry.node_id.clone(),
            stage_kind_id: node_plan.stage_kind_id.clone(),
            plane: graph_loop.plane,
        });
    }

    let compiled_completion_entry = compile_graph_completion_entry(graph_loop, &node_plan_by_id)?;
    let compiled_transitions = compile_graph_transitions(&graph_loop.edges);
    let compiled_resume_policies = graph_loop
        .dynamic_policies
        .as_ref()
        .map(|policies| compile_graph_resume_policies(&policies.resume_policies))
        .unwrap_or_default();
    let compiled_threshold_policies = graph_loop
        .dynamic_policies
        .as_ref()
        .map(|policies| compile_graph_threshold_policies(&policies.threshold_policies, config))
        .unwrap_or_default();

    let mut plan = FrozenGraphPlanePlan {
        loop_id: graph_loop.loop_id.clone(),
        plane: graph_loop.plane,
        nodes,
        entry_nodes: graph_loop.entry_nodes.clone(),
        transitions: graph_loop.edges.clone(),
        compiled_entries,
        compiled_completion_entry,
        compiled_transitions,
        compiled_resume_policies,
        compiled_threshold_policies,
        terminal_states: graph_loop.terminal_states.clone(),
        completion_behavior: graph_loop.completion_behavior.clone(),
    };
    plan.validate()
        .map_err(|error| contract_error("frozen_graph_plane_plan", error))?;
    Ok(plan)
}

/// Materialize one graph node using graph, mode, stage registry, and effective config authority.
pub fn materialize_graph_node_plan(
    node: &super::contracts::GraphLoopNodeDefinition,
    plane: Plane,
    mode: &ModeDefinition,
    config: &EffectiveCompileConfig,
    stage_kinds: &HashMap<String, RegisteredStageKindDefinition>,
) -> CompilerMaterializationResult<MaterializedGraphNodePlan> {
    let stage_kind = stage_kinds.get(&node.stage_kind_id).ok_or_else(|| {
        CompilerMaterializationError::UnknownStageKind {
            node_id: node.node_id.clone(),
            stage_kind_id: node.stage_kind_id.clone(),
        }
    })?;
    let stage_name = stage_name_for_identifier(&node.stage_kind_id);
    let stage_config = config.stages.get(&node.stage_kind_id);

    let mut entrypoint_path = stage_kind.default_entrypoint_path.clone();
    if let Some(node_entrypoint_path) = &node.entrypoint_path {
        entrypoint_path = node_entrypoint_path.clone();
    }
    if let Some(stage_name) = stage_name {
        if let Some(mode_entrypoint_path) = mode.stage_entrypoint_overrides.get(&stage_name) {
            entrypoint_path = mode_entrypoint_path.clone();
        }
    }

    let mut attached_skill_additions = node.attached_skill_additions.clone();
    if let Some(stage_name) = stage_name {
        if let Some(mode_skill_paths) = mode.stage_skill_additions.get(&stage_name) {
            for skill_path in mode_skill_paths {
                push_unique(&mut attached_skill_additions, skill_path.clone());
            }
        }
    }

    let mut runner_name = node.runner_name.clone();
    if let Some(stage_config) = stage_config {
        if let Some(runner) = &stage_config.runner {
            runner_name = Some(runner.clone());
        }
    }
    if let Some(stage_name) = stage_name {
        if let Some(mode_runner) = mode.stage_runner_bindings.get(&stage_name) {
            runner_name = Some(mode_runner.clone());
        }
    }

    let mut model_name = node.model_name.clone();
    if let Some(stage_config) = stage_config {
        if let Some(model) = &stage_config.model {
            model_name = Some(model.clone());
        }
    }
    if let Some(stage_name) = stage_name {
        if let Some(mode_model) = mode.stage_model_bindings.get(&stage_name) {
            model_name = Some(mode_model.clone());
        }
    }

    let mut model_reasoning_effort = config.runners.codex.model_reasoning_effort.clone();
    if let Some(stage_config) = stage_config {
        if let Some(reasoning_effort) = &stage_config.model_reasoning_effort {
            model_reasoning_effort = Some(reasoning_effort.clone());
        }
    }

    let mut timeout_seconds = node
        .timeout_seconds
        .unwrap_or(DEFAULT_STAGE_TIMEOUT_SECONDS);
    if let Some(stage_config) = stage_config {
        if let Some(config_timeout) = stage_config.timeout_seconds {
            timeout_seconds = config_timeout;
        }
    }

    let mut plan = MaterializedGraphNodePlan {
        node_id: node.node_id.clone(),
        stage_kind_id: node.stage_kind_id.clone(),
        plane,
        entrypoint_path,
        entrypoint_contract_id: Some(format!("{}.contract.v1", node.node_id)),
        running_status_marker: stage_kind.running_status_marker.clone(),
        allowed_result_classes_by_outcome: stage_kind.allowed_result_classes_by_outcome.clone(),
        declared_output_artifacts: stage_kind.declared_output_artifacts.clone(),
        required_skill_paths: stage_kind.required_skill_paths.clone(),
        attached_skill_additions,
        runner_name,
        model_name,
        model_reasoning_effort,
        timeout_seconds,
    };
    plan.validate()
        .map_err(|error| contract_error("materialized_graph_node_plan", error))?;
    Ok(plan)
}

/// Compile graph edges into one concrete transition per declared outcome.
#[must_use]
pub fn compile_graph_transitions(
    edges: &[GraphLoopEdgeDefinition],
) -> Vec<CompiledGraphTransitionPlan> {
    let mut compiled = Vec::new();
    for edge in edges {
        for outcome in &edge.on_outcomes {
            compiled.push(CompiledGraphTransitionPlan {
                edge_id: edge.edge_id.clone(),
                source_node_id: edge.from_node_id.clone(),
                outcome: outcome.clone(),
                target_node_id: edge.to_node_id.clone(),
                terminal_state_id: edge.terminal_state_id.clone(),
                kind: edge.kind,
                priority: edge.priority,
                max_attempts: edge.max_attempts,
            });
        }
    }
    compiled
}

/// Compile graph resume policies without changing their declared targets.
#[must_use]
pub fn compile_graph_resume_policies(
    policies: &[GraphLoopResumePolicyDefinition],
) -> Vec<CompiledGraphResumePolicyPlan> {
    policies
        .iter()
        .map(|policy| CompiledGraphResumePolicyPlan {
            policy_id: policy.policy_id.clone(),
            source_node_id: policy.source_node_id.clone(),
            on_outcome: policy.on_outcome.clone(),
            default_target_node_id: policy.default_target_node_id.clone(),
            metadata_stage_keys: policy.metadata_stage_keys.clone(),
            disallowed_target_node_ids: policy.disallowed_target_node_ids.clone(),
        })
        .collect()
}

/// Compile graph threshold policies with configured recovery thresholds.
#[must_use]
pub fn compile_graph_threshold_policies(
    policies: &[GraphLoopThresholdPolicyDefinition],
    config: &EffectiveCompileConfig,
) -> Vec<CompiledGraphThresholdPolicyPlan> {
    policies
        .iter()
        .map(|policy| CompiledGraphThresholdPolicyPlan {
            policy_id: policy.policy_id.clone(),
            source_node_ids: policy.source_node_ids.clone(),
            on_outcome: policy.on_outcome.clone(),
            counter_name: policy.counter_name,
            threshold: resolved_threshold_for_policy(policy, config),
            exhausted_target_node_id: policy.exhausted_target_node_id.clone(),
            exhausted_terminal_state_id: policy.exhausted_terminal_state_id.clone(),
        })
        .collect()
}

/// Resolve the configured threshold value for a graph threshold policy.
#[must_use]
pub fn resolved_threshold_for_policy(
    policy: &GraphLoopThresholdPolicyDefinition,
    config: &EffectiveCompileConfig,
) -> u64 {
    match policy.counter_name {
        GraphLoopCounterName::FixCycleCount => config.recovery.max_fix_cycles,
        GraphLoopCounterName::TroubleshootAttemptCount => {
            config.recovery.max_troubleshoot_attempts_before_consult
        }
        GraphLoopCounterName::MechanicAttemptCount => config.recovery.max_mechanic_attempts,
    }
}

/// Return known stage names selected by graph-loop node stage kind ids.
#[must_use]
pub fn selected_stages_for_graph_loops<'a>(
    graph_loops: impl IntoIterator<Item = &'a GraphLoopDefinition>,
) -> HashSet<StageName> {
    let mut selected_stages = HashSet::new();
    for graph_loop in graph_loops {
        for node in &graph_loop.nodes {
            if let Some(stage_name) = stage_name_for_identifier(&node.stage_kind_id) {
                selected_stages.insert(stage_name);
            }
        }
    }
    selected_stages
}

/// Resolve a stage name from a canonical stage-kind identifier when it names a built-in stage.
#[must_use]
pub fn stage_name_for_identifier(identifier: &str) -> Option<StageName> {
    StageName::from_value(identifier).ok()
}

/// Build the deterministic identity for a materialized compiled plan.
#[must_use]
pub fn build_compiled_plan_id(
    mode_id: &str,
    loop_ids_by_plane: &HashMap<Plane, String>,
    graphs_by_plane: &HashMap<Plane, FrozenGraphPlanePlan>,
    concurrency_policy: &Option<super::contracts::PlaneConcurrencyPolicyDefinition>,
    learning_trigger_rules: &[super::contracts::LearningTriggerRuleDefinition],
) -> String {
    let mut payload = Map::new();
    payload.insert("mode_id".to_owned(), Value::String(mode_id.to_owned()));
    payload.insert(
        "loop_ids_by_plane".to_owned(),
        plane_string_map_value(loop_ids_by_plane),
    );
    payload.insert(
        "graphs_by_plane".to_owned(),
        plane_graph_map_value(graphs_by_plane),
    );
    payload.insert(
        "concurrency_policy".to_owned(),
        serde_json::to_value(concurrency_policy).expect("concurrency policy is serializable"),
    );
    payload.insert(
        "learning_trigger_rules".to_owned(),
        serde_json::to_value(learning_trigger_rules)
            .expect("learning trigger rules are serializable"),
    );
    let encoded =
        serde_json::to_vec(&Value::Object(payload)).expect("compiled plan id payload serializes");
    format!("plan-{mode_id}-{}", hex_prefix(Sha256::digest(encoded), 12))
}

/// Build stable source references for a materialized plan.
#[must_use]
pub fn build_graph_source_refs(
    mode_id: &str,
    graphs_by_plane: &HashMap<Plane, FrozenGraphPlanePlan>,
    has_planning_completion_behavior: bool,
) -> Vec<String> {
    let mut refs = vec![format!("mode:{mode_id}")];
    for (plane, graph) in sorted_plane_entries(graphs_by_plane) {
        let _ = plane;
        refs.push(format!("graph_loop:{}", graph.loop_id));
    }
    if has_planning_completion_behavior {
        if let Some(graph) = graphs_by_plane.get(&Plane::Planning) {
            refs.push(format!("graph_completion_behavior:{}", graph.loop_id));
        }
    }
    refs
}

fn compile_graph_completion_entry(
    graph_loop: &GraphLoopDefinition,
    node_plan_by_id: &HashMap<&str, &MaterializedGraphNodePlan>,
) -> CompilerMaterializationResult<Option<CompiledGraphCompletionEntryPlan>> {
    let Some(completion_behavior) = &graph_loop.completion_behavior else {
        return Ok(None);
    };
    let Some(node_plan) = node_plan_by_id.get(completion_behavior.target_node_id.as_str()) else {
        return Err(CompilerMaterializationError::UnknownCompletionTarget {
            node_id: completion_behavior.target_node_id.clone(),
        });
    };
    Ok(Some(CompiledGraphCompletionEntryPlan {
        entry_key: super::contracts::GraphLoopEntryKey::ClosureTarget,
        node_id: completion_behavior.target_node_id.clone(),
        stage_kind_id: node_plan.stage_kind_id.clone(),
        plane: graph_loop.plane,
        trigger: completion_behavior.trigger.clone(),
        readiness_rule: completion_behavior.readiness_rule.clone(),
        request_kind: completion_behavior.request_kind.clone(),
        target_selector: completion_behavior.target_selector.clone(),
        rubric_policy: completion_behavior.rubric_policy.clone(),
        blocked_work_policy: completion_behavior.blocked_work_policy.clone(),
        skip_if_already_closed: completion_behavior.skip_if_already_closed,
        on_pass_terminal_state_id: completion_behavior.on_pass_terminal_state_id.clone(),
        on_gap_terminal_state_id: completion_behavior.on_gap_terminal_state_id.clone(),
        create_incident_on_gap: completion_behavior.create_incident_on_gap,
    }))
}

fn validate_graph_stage_kind_semantics(
    graph_loop: &GraphLoopDefinition,
    stage_kinds: &HashMap<String, RegisteredStageKindDefinition>,
) -> CompilerMaterializationResult<()> {
    let node_by_id: HashMap<_, _> = graph_loop
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect();

    for node in &graph_loop.nodes {
        let stage_kind = stage_kind_for_node(node, stage_kinds)?;
        if stage_kind.plane != graph_loop.plane {
            return Err(invalid_stage_kind_reference(
                graph_loop,
                format!(
                    "node {} uses stage kind {} from plane {}",
                    node.node_id,
                    node.stage_kind_id,
                    stage_kind.plane.as_str()
                ),
            ));
        }
        for override_name in declared_override_names(node) {
            if !stage_kind
                .allowed_overrides
                .iter()
                .any(|allowed| allowed == override_name)
            {
                return Err(invalid_stage_kind_reference(
                    graph_loop,
                    format!(
                        "node {} declares unsupported override {override_name} for stage kind {}",
                        node.node_id, node.stage_kind_id
                    ),
                ));
            }
        }
    }

    for edge in &graph_loop.edges {
        let Some(source_node) = node_by_id.get(edge.from_node_id.as_str()) else {
            continue;
        };
        let source_stage_kind = stage_kind_for_node(source_node, stage_kinds)?;
        for outcome in &edge.on_outcomes {
            if !source_stage_kind.legal_outcomes.contains(outcome) {
                return Err(invalid_stage_kind_reference(
                    graph_loop,
                    format!(
                        "edge {} declares illegal outcome {outcome} for stage kind {}",
                        edge.edge_id, source_stage_kind.stage_kind_id
                    ),
                ));
            }
        }
    }

    if let Some(policies) = &graph_loop.dynamic_policies {
        for policy in &policies.resume_policies {
            let Some(source_node) = node_by_id.get(policy.source_node_id.as_str()) else {
                continue;
            };
            let source_stage_kind = stage_kind_for_node(source_node, stage_kinds)?;
            if !source_stage_kind
                .legal_outcomes
                .contains(&policy.on_outcome)
            {
                return Err(invalid_stage_kind_reference(
                    graph_loop,
                    format!(
                        "resume policy {} declares illegal outcome {} for stage kind {}",
                        policy.policy_id, policy.on_outcome, source_stage_kind.stage_kind_id
                    ),
                ));
            }
        }

        for policy in &policies.threshold_policies {
            for source_node_id in &policy.source_node_ids {
                let Some(source_node) = node_by_id.get(source_node_id.as_str()) else {
                    continue;
                };
                let source_stage_kind = stage_kind_for_node(source_node, stage_kinds)?;
                if !source_stage_kind
                    .legal_outcomes
                    .contains(&policy.on_outcome)
                {
                    return Err(invalid_stage_kind_reference(
                        graph_loop,
                        format!(
                            "threshold policy {} declares illegal outcome {} for stage kind {}",
                            policy.policy_id, policy.on_outcome, source_stage_kind.stage_kind_id
                        ),
                    ));
                }
            }
        }
    }

    if let Some(completion) = &graph_loop.completion_behavior {
        let Some(target_node) = node_by_id.get(completion.target_node_id.as_str()) else {
            return Err(CompilerMaterializationError::UnknownCompletionTarget {
                node_id: completion.target_node_id.clone(),
            });
        };
        let target_stage_kind = stage_kind_for_node(target_node, stage_kinds)?;
        if !target_stage_kind.closure_role {
            return Err(invalid_stage_kind_reference(
                graph_loop,
                format!(
                    "completion behavior targets non-closure stage kind {}",
                    target_stage_kind.stage_kind_id
                ),
            ));
        }
    }

    Ok(())
}

fn stage_kind_for_node<'a>(
    node: &super::contracts::GraphLoopNodeDefinition,
    stage_kinds: &'a HashMap<String, RegisteredStageKindDefinition>,
) -> CompilerMaterializationResult<&'a RegisteredStageKindDefinition> {
    stage_kinds.get(&node.stage_kind_id).ok_or_else(|| {
        CompilerMaterializationError::UnknownStageKind {
            node_id: node.node_id.clone(),
            stage_kind_id: node.stage_kind_id.clone(),
        }
    })
}

fn declared_override_names(node: &super::contracts::GraphLoopNodeDefinition) -> Vec<&'static str> {
    let mut names = Vec::new();
    if node.entrypoint_path.is_some() {
        names.push("entrypoint_path");
    }
    if !node.attached_skill_additions.is_empty() {
        names.push("attached_skill_additions");
    }
    if node.runner_name.is_some() {
        names.push("runner_name");
    }
    if node.model_name.is_some() {
        names.push("model_name");
    }
    if node.timeout_seconds.is_some() {
        names.push("timeout_seconds");
    }
    names
}

fn invalid_stage_kind_reference(
    graph_loop: &GraphLoopDefinition,
    message: String,
) -> CompilerMaterializationError {
    CompilerMaterializationError::InvalidStageKindReference {
        graph_loop_id: graph_loop.loop_id.clone(),
        message,
    }
}

fn validate_learning_trigger_rules(
    mode: &ModeDefinition,
    selected_stages: &HashSet<StageName>,
) -> CompilerMaterializationResult<()> {
    for rule in &mode.learning_trigger_rules {
        if !selected_stages.contains(&rule.source_stage) {
            return Err(CompilerMaterializationError::InvalidLearningTrigger {
                rule_id: rule.rule_id.clone(),
                message: format!(
                    "source_stage {} is outside selected loops",
                    rule.source_stage.as_str()
                ),
            });
        }
        let target_stage = StageName::from(rule.target_stage);
        if !selected_stages.contains(&target_stage) {
            return Err(CompilerMaterializationError::InvalidLearningTrigger {
                rule_id: rule.rule_id.clone(),
                message: format!(
                    "target_stage {} is outside selected loops",
                    target_stage.as_str()
                ),
            });
        }
    }
    Ok(())
}

fn required_loop_id(mode: &ModeDefinition, plane: Plane) -> CompilerMaterializationResult<String> {
    mode.loop_ids_by_plane
        .get(&plane)
        .cloned()
        .ok_or(CompilerMaterializationError::MissingLoopBinding { plane })
}

fn graph_for_plane(
    graphs_by_plane: &HashMap<Plane, FrozenGraphPlanePlan>,
    plane: Plane,
    loop_id: &str,
) -> CompilerMaterializationResult<FrozenGraphPlanePlan> {
    graphs_by_plane
        .get(&plane)
        .cloned()
        .ok_or_else(|| CompilerMaterializationError::MissingGraph {
            plane,
            loop_id: Some(loop_id.to_owned()),
        })
}

fn plane_string_map_value(loop_ids_by_plane: &HashMap<Plane, String>) -> Value {
    let mut map = Map::new();
    for (plane, loop_id) in sorted_plane_entries(loop_ids_by_plane) {
        map.insert(plane.as_str().to_owned(), Value::String(loop_id.clone()));
    }
    Value::Object(map)
}

fn plane_graph_map_value(graphs_by_plane: &HashMap<Plane, FrozenGraphPlanePlan>) -> Value {
    let mut map = Map::new();
    for (plane, graph) in sorted_plane_entries(graphs_by_plane) {
        map.insert(
            plane.as_str().to_owned(),
            serde_json::to_value(graph).expect("frozen graph plan is serializable"),
        );
    }
    Value::Object(map)
}

fn sorted_plane_entries<T>(mapping: &HashMap<Plane, T>) -> Vec<(Plane, &T)> {
    let mut entries: Vec<_> = mapping
        .iter()
        .map(|(plane, value)| (*plane, value))
        .collect();
    entries.sort_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
    entries
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn contract_error(
    artifact: &'static str,
    error: CompilerContractError,
) -> CompilerMaterializationError {
    CompilerMaterializationError::Contract {
        artifact,
        message: error.to_string(),
    }
}

fn hex_prefix(digest: impl AsRef<[u8]>, hex_len: usize) -> String {
    let bytes = digest.as_ref();
    let mut rendered = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered.truncate(hex_len);
    rendered
}
