//! Deterministic materialization of resolved compiler assets into frozen plans.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
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
        CompilerContractError, ExecutionCapabilitySummary, FrozenGraphPlanePlan,
        GraphLoopCounterName, GraphLoopDefinition, GraphLoopEdgeDefinition,
        GraphLoopResumePolicyDefinition, GraphLoopThresholdPolicyDefinition,
        MaterializedGraphNodePlan, ModeDefinition, RegisteredStageKindDefinition,
    },
    workflow_primitives::materialize_workflow_primitive_graph_authority,
};
use crate::{
    contracts::{
        ApprovalPolicyRef, CapabilityDecisionState, CapabilityEnforcementMode,
        CapabilityEvidenceStatus, CapabilityPolicyDecision, CapabilityPolicyOverride,
        CapabilityRequest, CapabilityScope, ExecutionCapabilityGrant, ExecutionCapabilityWarning,
        LearningStageName, Plane, StageName, Timestamp, capability_grant_fingerprint,
    },
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
    /// Capability requests or policy could not be sealed into per-node grants.
    CapabilityGrant {
        /// Graph node id.
        node_id: String,
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
            Self::CapabilityGrant { node_id, message } => {
                write!(
                    f,
                    "graph node {node_id} has invalid execution capability grants: {message}"
                )
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
        let mut graph_plan = materialize_graph_plane_plan(
            &graph_asset.graph_loop,
            &resolved.mode,
            &resolved.config,
            &stage_kinds,
        )?;
        materialize_workflow_primitive_graph_authority(
            &mut graph_plan,
            &resolved.workflow_primitive_authority.workflow_primitives,
            &resolved.workflow_primitive_authority.lane_policy,
        )
        .map_err(|error| contract_error("workflow_primitive_graph_authority", error))?;
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
    let execution_capability_summaries_by_plane =
        execution_capability_summaries_by_plane(&graphs_by_plane);
    let execution_capability_summary =
        execution_capability_summary_from_graphs(graphs_by_plane.values());

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
        execution_capability_summary,
        execution_capability_summaries_by_plane,
        concurrency_policy: resolved.mode.concurrency_policy.clone(),
        learning_trigger_rules: resolved.mode.learning_trigger_rules.clone(),
        workflow_primitives: resolved
            .workflow_primitive_authority
            .workflow_primitives
            .clone(),
        workflow_primitive_fingerprints: resolved
            .workflow_primitive_authority
            .workflow_primitive_fingerprints
            .clone(),
        lane_policy: Some(resolved.workflow_primitive_authority.lane_policy.clone()),
        workspace_schema_epoch: resolved
            .workflow_primitive_authority
            .workflow_primitives
            .workspace_schema_epoch
            .clone(),
        pending_compiled_plan: None,
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
    let execution_capability_summary = execution_capability_summary_from_nodes(&nodes);

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
        execution_capability_summary,
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

    let mut thinking_level = node.thinking_level.clone();
    if let Some(stage_config) = stage_config {
        if let Some(config_thinking_level) = &stage_config.thinking_level {
            thinking_level = Some(config_thinking_level.clone());
        }
    }
    if let Some(stage_name) = stage_name {
        if let Some(mode_thinking_level) = mode.stage_thinking_bindings.get(&stage_name) {
            thinking_level = mode_thinking_level.clone();
        }
    }

    let model_reasoning_effort = if runner_name.as_deref() == Some("codex_cli") {
        thinking_level.clone()
    } else {
        None
    };

    let mut timeout_seconds = node
        .timeout_seconds
        .unwrap_or(DEFAULT_STAGE_TIMEOUT_SECONDS);
    if let Some(stage_config) = stage_config {
        if let Some(config_timeout) = stage_config.timeout_seconds {
            timeout_seconds = config_timeout;
        }
    }
    let capability_context = compile_execution_capability_context(
        node,
        stage_kind,
        stage_name,
        mode,
        config,
        runner_name.as_deref(),
    )?;

    let mut plan = MaterializedGraphNodePlan {
        node_id: node.node_id.clone(),
        stage_kind_id: node.stage_kind_id.clone(),
        plane,
        lane_id: None,
        entrypoint_path,
        entrypoint_contract_id: Some(format!("{}.contract.v1", node.node_id)),
        running_status_marker: stage_kind.running_status_marker.clone(),
        allowed_result_classes_by_outcome: stage_kind.allowed_result_classes_by_outcome.clone(),
        declared_output_artifacts: stage_kind.declared_output_artifacts.clone(),
        required_skill_paths: stage_kind.required_skill_paths.clone(),
        attached_skill_additions,
        runner_name,
        model_name,
        thinking_level,
        model_reasoning_effort,
        timeout_seconds,
        execution_capability_grants: capability_context.grants,
        execution_capability_warnings: capability_context.warnings,
        execution_capability_policy_fingerprint: capability_context.policy_fingerprint,
        request_context_profile_id: None,
        terminal_action_mappings: BTreeMap::new(),
        runtime_effect_rule_selections: Vec::new(),
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

struct CompiledCapabilityContext {
    grants: Vec<ExecutionCapabilityGrant>,
    warnings: Vec<ExecutionCapabilityWarning>,
    policy_fingerprint: String,
}

#[derive(Clone)]
struct CapabilityPolicyCandidate {
    source: &'static str,
    override_: CapabilityPolicyOverride,
}

fn compile_execution_capability_context(
    node: &super::contracts::GraphLoopNodeDefinition,
    stage_kind: &RegisteredStageKindDefinition,
    stage_name: Option<StageName>,
    mode: &ModeDefinition,
    config: &EffectiveCompileConfig,
    runner_name: Option<&str>,
) -> CompilerMaterializationResult<CompiledCapabilityContext> {
    if !config.execution_capabilities.enabled {
        return Ok(CompiledCapabilityContext {
            grants: Vec::new(),
            warnings: Vec::new(),
            policy_fingerprint: capability_policy_fingerprint(node, &[], &[], config),
        });
    }

    let mut requests = default_framework_capability_requests(node, runner_name);
    append_capability_requests(
        &mut requests,
        &stage_kind.execution_capability_requests,
        "stage_kind",
    );
    append_capability_requests(
        &mut requests,
        &node.execution_capability_requests,
        "graph_node",
    );
    append_capability_requests(&mut requests, &mode.execution_capability_requests, "mode");
    if let Some(stage_name) = stage_name {
        if let Some(mode_requests) = mode.stage_execution_capability_requests.get(&stage_name) {
            append_capability_requests(&mut requests, mode_requests, "mode");
        }
    }
    let requests = dedupe_capability_requests(requests);

    let mut policies = Vec::new();
    append_policy_candidates(
        &mut policies,
        &stage_kind.execution_capability_policy_overrides,
        "stage_kind",
    );
    append_policy_candidates(&mut policies, &mode.execution_capability_policies, "mode");
    if let Some(stage_name) = stage_name {
        if let Some(mode_overrides) = mode
            .stage_execution_capability_policy_overrides
            .get(&stage_name)
        {
            append_policy_candidates(&mut policies, mode_overrides, "mode");
        }
    }
    append_policy_candidates(
        &mut policies,
        &node.execution_capability_policies,
        "graph_node",
    );
    append_policy_candidates(
        &mut policies,
        &node.execution_capability_policy_overrides,
        "graph_node",
    );

    let policy_fingerprint = capability_policy_fingerprint(node, &requests, &policies, config);
    let mut grants = Vec::with_capacity(requests.len());
    let mut warnings = Vec::new();
    for (index, request) in requests.iter().enumerate() {
        let grant = resolve_execution_capability_grant(node, request, &policies, config, index)?;
        warnings.extend(warnings_for_grant(node, request, &grant));
        grants.push(grant);
    }
    Ok(CompiledCapabilityContext {
        grants,
        warnings,
        policy_fingerprint,
    })
}

fn default_framework_capability_requests(
    node: &super::contracts::GraphLoopNodeDefinition,
    runner_name: Option<&str>,
) -> Vec<CapabilityRequest> {
    vec![
        framework_request(
            node,
            "runner.invoke",
            "execute",
            CapabilityScope {
                kind: "runner".to_owned(),
                value: runner_name.unwrap_or("default").to_owned(),
                metadata: Map::new(),
            },
        ),
        framework_request(
            node,
            "workspace.read",
            "read",
            CapabilityScope {
                kind: "workspace".to_owned(),
                value: "workspace".to_owned(),
                metadata: Map::new(),
            },
        ),
        framework_request(
            node,
            "artifact.write",
            "write",
            CapabilityScope {
                kind: "artifact_kind".to_owned(),
                value: "stage_result".to_owned(),
                metadata: Map::new(),
            },
        ),
    ]
}

fn framework_request(
    node: &super::contracts::GraphLoopNodeDefinition,
    capability_id: &str,
    access: &str,
    scope: CapabilityScope,
) -> CapabilityRequest {
    CapabilityRequest {
        request_id: format!(
            "{}.framework.{}",
            node.node_id,
            capability_id.replace('.', "_")
        ),
        capability_id: capability_id.to_owned(),
        access: access.to_owned(),
        scope,
        required: true,
        requires_enforcement: false,
        reason: "default framework capability required to dispatch a stage".to_owned(),
        requested_by: "stage_kind_default".to_owned(),
        policy_source: Some("stage_kind_default".to_owned()),
    }
}

fn append_capability_requests(
    target: &mut Vec<CapabilityRequest>,
    requests: &[CapabilityRequest],
    source: &'static str,
) {
    for request in requests {
        let mut request = request.clone();
        if request.requested_by == "stage" {
            request.requested_by = source.to_owned();
        }
        if request.policy_source.is_none() {
            request.policy_source = Some(source.to_owned());
        }
        target.push(request);
    }
}

fn append_policy_candidates(
    target: &mut Vec<CapabilityPolicyCandidate>,
    overrides: &[CapabilityPolicyOverride],
    source: &'static str,
) {
    target.extend(
        overrides
            .iter()
            .cloned()
            .map(|override_| CapabilityPolicyCandidate { source, override_ }),
    );
}

fn dedupe_capability_requests(requests: Vec<CapabilityRequest>) -> Vec<CapabilityRequest> {
    let mut deduped: Vec<CapabilityRequest> = Vec::new();
    for request in requests {
        if let Some(existing) = deduped
            .iter_mut()
            .find(|existing| capability_request_key(existing) == capability_request_key(&request))
        {
            existing.required |= request.required;
            existing.requires_enforcement |= request.requires_enforcement;
            if existing.reason.trim().is_empty() && !request.reason.trim().is_empty() {
                existing.reason = request.reason;
            }
            if existing.policy_source.is_none() {
                existing.policy_source = request.policy_source;
            }
            continue;
        }
        deduped.push(request);
    }
    deduped
}

fn capability_request_key(request: &CapabilityRequest) -> (String, String, String, String) {
    (
        request.capability_id.clone(),
        request.access.clone(),
        request.scope.kind.clone(),
        request.scope.value.clone(),
    )
}

fn resolve_execution_capability_grant(
    node: &super::contracts::GraphLoopNodeDefinition,
    request: &CapabilityRequest,
    policies: &[CapabilityPolicyCandidate],
    config: &EffectiveCompileConfig,
    index: usize,
) -> CompilerMaterializationResult<ExecutionCapabilityGrant> {
    let (decision, resolved_by, decision_reason) =
        resolve_capability_decision(request, policies, config);
    let mut decision_state = match decision {
        CapabilityPolicyDecision::Allow => CapabilityDecisionState::Granted,
        CapabilityPolicyDecision::Deny => CapabilityDecisionState::Denied,
        CapabilityPolicyDecision::ApprovalRequired => CapabilityDecisionState::ApprovalRequired,
    };
    let mut enforcement_mode = if decision_state == CapabilityDecisionState::Granted {
        if is_runtime_enforced_capability(&request.capability_id) {
            CapabilityEnforcementMode::RuntimeEnforced
        } else {
            CapabilityEnforcementMode::AdvisoryOnly
        }
    } else {
        CapabilityEnforcementMode::NotApplicable
    };
    let mut approval_policy_ref = None;
    if decision_state == CapabilityDecisionState::ApprovalRequired {
        approval_policy_ref = Some(ApprovalPolicyRef {
            policy_id: format!("operator.{}", request.capability_id.replace('.', "_")),
            gate_scope: "stage".to_owned(),
            expiration_seconds: None,
            required_decision: "approved".to_owned(),
        });
    }
    if decision_state == CapabilityDecisionState::Granted
        && enforcement_mode == CapabilityEnforcementMode::AdvisoryOnly
        && request.required
        && (request.requires_enforcement || config.execution_capabilities.fail_required_advisory)
    {
        return Err(CompilerMaterializationError::CapabilityGrant {
            node_id: node.node_id.clone(),
            message: format!(
                "required advisory grant rejected for capability {} by requires_enforcement or fail_required_advisory",
                request.capability_id
            ),
        });
    }
    if decision_state == CapabilityDecisionState::Granted
        && enforcement_mode == CapabilityEnforcementMode::AdvisoryOnly
        && request.required
        && !config.execution_capabilities.allow_advisory_grants
    {
        decision_state = CapabilityDecisionState::Unsupported;
        enforcement_mode = CapabilityEnforcementMode::NotApplicable;
    }

    let mut grant = ExecutionCapabilityGrant {
        grant_id: format!(
            "grant-{}-{}-{}",
            node.node_id,
            request.capability_id.replace('.', "-"),
            index
        ),
        request_id: request.request_id.clone(),
        capability_id: request.capability_id.clone(),
        access: request.access.clone(),
        scope: request.scope.clone(),
        required: request.required,
        decision_state,
        enforcement_mode,
        approval_policy_ref,
        evidence_requirements: evidence_requirements_for_grant(request, enforcement_mode),
        evidence_status: CapabilityEvidenceStatus::NotRequired,
        decision_reason,
        resolved_by: resolved_by.to_owned(),
        fingerprint: String::new(),
    };
    if grant.decision_state == CapabilityDecisionState::Granted
        && !grant.evidence_requirements.is_empty()
    {
        grant.evidence_status = CapabilityEvidenceStatus::Pending;
    }
    grant.fingerprint = capability_grant_fingerprint(&grant);
    grant
        .validate()
        .map_err(|error| CompilerMaterializationError::CapabilityGrant {
            node_id: node.node_id.clone(),
            message: error.to_string(),
        })?;
    Ok(grant)
}

fn resolve_capability_decision<'a>(
    request: &CapabilityRequest,
    policies: &'a [CapabilityPolicyCandidate],
    config: &'a EffectiveCompileConfig,
) -> (CapabilityPolicyDecision, &'a str, String) {
    if let Some(decision) = config
        .execution_capabilities
        .defaults
        .get(&request.capability_id)
    {
        return (
            *decision,
            "runtime_config",
            format!(
                "runtime config default for capability {} resolved to {}",
                request.capability_id,
                decision.as_str()
            ),
        );
    }
    for source in ["graph_node", "mode", "stage_kind"] {
        if let Some(candidate) = policies.iter().rev().find(|candidate| {
            candidate.source == source
                && candidate.override_.capability_id == request.capability_id
                && scope_override_matches_request(&candidate.override_, request)
        }) {
            let reason = if candidate.override_.reason.trim().is_empty() {
                format!(
                    "{source} override for capability {} resolved to {}",
                    request.capability_id,
                    candidate.override_.decision.as_str()
                )
            } else {
                candidate.override_.reason.clone()
            };
            return (candidate.override_.decision, candidate.source, reason);
        }
    }
    if is_default_framework_request(request) {
        return (
            CapabilityPolicyDecision::Allow,
            "stage_kind_default",
            "default framework grants are allowed by compiler policy".to_owned(),
        );
    }
    (
        config.execution_capabilities.default_unknown_capability,
        "runtime_config",
        format!(
            "runtime config default_unknown_capability resolved capability {} to {}",
            request.capability_id,
            config
                .execution_capabilities
                .default_unknown_capability
                .as_str()
        ),
    )
}

fn scope_override_matches_request(
    override_: &CapabilityPolicyOverride,
    request: &CapabilityRequest,
) -> bool {
    match &override_.scope {
        Some(scope) => scope.kind == request.scope.kind && scope.value == request.scope.value,
        None => true,
    }
}

fn evidence_requirements_for_grant(
    request: &CapabilityRequest,
    enforcement_mode: CapabilityEnforcementMode,
) -> Vec<String> {
    if enforcement_mode == CapabilityEnforcementMode::NotApplicable {
        return Vec::new();
    }
    let mut requirements = vec![
        "runner_invocation".to_owned(),
        "runner_completion".to_owned(),
    ];
    if request.requires_enforcement {
        requirements.push("capability_evidence".to_owned());
    }
    requirements
}

fn warnings_for_grant(
    node: &super::contracts::GraphLoopNodeDefinition,
    request: &CapabilityRequest,
    grant: &ExecutionCapabilityGrant,
) -> Vec<ExecutionCapabilityWarning> {
    let mut warnings = Vec::new();
    match grant.decision_state {
        CapabilityDecisionState::Granted
            if grant.enforcement_mode == CapabilityEnforcementMode::AdvisoryOnly =>
        {
            warnings.push(capability_warning(
                node,
                &grant.capability_id,
                "advisory",
                format!(
                    "capability {} is granted as advisory-only",
                    grant.capability_id
                ),
            ));
        }
        CapabilityDecisionState::Denied => warnings.push(capability_warning(
            node,
            &grant.capability_id,
            "denied",
            format!("capability {} is denied", grant.capability_id),
        )),
        CapabilityDecisionState::ApprovalRequired => warnings.push(capability_warning(
            node,
            &grant.capability_id,
            "approval_required",
            format!("capability {} requires approval", grant.capability_id),
        )),
        CapabilityDecisionState::Unsupported => warnings.push(capability_warning(
            node,
            &grant.capability_id,
            "unsupported",
            format!(
                "capability {} cannot satisfy required enforcement without advisory grants",
                grant.capability_id
            ),
        )),
        _ => {}
    }
    if request.required && grant.enforcement_mode == CapabilityEnforcementMode::AdvisoryOnly {
        warnings.push(capability_warning(
            node,
            &grant.capability_id,
            "required_advisory",
            format!(
                "required capability {} resolved to advisory-only",
                grant.capability_id
            ),
        ));
    }
    warnings
}

fn capability_warning(
    node: &super::contracts::GraphLoopNodeDefinition,
    capability_id: &str,
    severity: &str,
    message: String,
) -> ExecutionCapabilityWarning {
    ExecutionCapabilityWarning {
        warning_id: format!(
            "{}.{}.{}",
            node.node_id,
            capability_id.replace('.', "_"),
            severity
        ),
        capability_id: capability_id.to_owned(),
        severity: severity.to_owned(),
        message,
    }
}

fn is_default_framework_request(request: &CapabilityRequest) -> bool {
    request.policy_source.as_deref() == Some("stage_kind_default")
        && matches!(
            request.capability_id.as_str(),
            "runner.invoke" | "workspace.read" | "artifact.write"
        )
}

fn is_runtime_enforced_capability(capability_id: &str) -> bool {
    matches!(
        capability_id,
        "runner.invoke" | "artifact.read" | "artifact.write" | "evidence.emit" | "runtime.control"
    )
}

fn capability_policy_fingerprint(
    node: &super::contracts::GraphLoopNodeDefinition,
    requests: &[CapabilityRequest],
    policies: &[CapabilityPolicyCandidate],
    config: &EffectiveCompileConfig,
) -> String {
    let mut payload = Map::new();
    payload.insert("node_id".to_owned(), Value::String(node.node_id.clone()));
    payload.insert(
        "requests".to_owned(),
        serde_json::to_value(requests).expect("capability requests are serializable"),
    );
    payload.insert(
        "policy_overrides".to_owned(),
        serde_json::to_value(
            policies
                .iter()
                .map(|policy| {
                    let mut value = Map::new();
                    value.insert("source".to_owned(), Value::String(policy.source.to_owned()));
                    value.insert(
                        "override".to_owned(),
                        serde_json::to_value(&policy.override_)
                            .expect("capability policy override is serializable"),
                    );
                    Value::Object(value)
                })
                .collect::<Vec<_>>(),
        )
        .expect("capability policies are serializable"),
    );
    payload.insert(
        "execution_capabilities".to_owned(),
        serde_json::to_value(&config.execution_capabilities)
            .expect("execution capability config is serializable"),
    );
    let encoded =
        serde_json::to_vec(&Value::Object(payload)).expect("capability policy payload serializes");
    format!("cap-pol-{}", hex_prefix(Sha256::digest(encoded), 12))
}

fn execution_capability_summary_from_nodes(
    nodes: &[MaterializedGraphNodePlan],
) -> ExecutionCapabilitySummary {
    let grants = nodes
        .iter()
        .flat_map(|node| node.execution_capability_grants.iter());
    execution_capability_summary_from_grants(grants)
}

fn execution_capability_summary_from_graphs<'a>(
    graphs: impl IntoIterator<Item = &'a FrozenGraphPlanePlan>,
) -> ExecutionCapabilitySummary {
    let grants = graphs
        .into_iter()
        .flat_map(|graph| graph.nodes.iter())
        .flat_map(|node| node.execution_capability_grants.iter());
    execution_capability_summary_from_grants(grants)
}

fn execution_capability_summaries_by_plane(
    graphs_by_plane: &HashMap<Plane, FrozenGraphPlanePlan>,
) -> HashMap<Plane, ExecutionCapabilitySummary> {
    sorted_plane_entries(graphs_by_plane)
        .into_iter()
        .map(|(plane, graph)| (plane, graph.execution_capability_summary.clone()))
        .collect()
}

fn execution_capability_summary_from_grants<'a>(
    grants: impl IntoIterator<Item = &'a ExecutionCapabilityGrant>,
) -> ExecutionCapabilitySummary {
    let mut summary = ExecutionCapabilitySummary::default();
    for grant in grants {
        summary.total_grants += 1;
        *summary
            .by_decision
            .entry(grant.decision_state.as_str().to_owned())
            .or_insert(0) += 1;
        *summary
            .by_enforcement
            .entry(grant.enforcement_mode.as_str().to_owned())
            .or_insert(0) += 1;
    }
    summary
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
    refs.push("workflow_primitives:v0.20".to_owned());
    refs.push("workspace_schema_epoch:v0.20".to_owned());
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
        if rule.target_stage == LearningStageName::Curator
            && rule.target_skill_id.is_none()
            && rule.preferred_output_paths.is_empty()
        {
            return Err(CompilerMaterializationError::InvalidLearningTrigger {
                rule_id: rule.rule_id.clone(),
                message: "targets curator without a safe destination: direct curator triggers require target_skill_id or preferred_output_paths; route vague learning through analyst".to_owned(),
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
