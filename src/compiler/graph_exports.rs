//! Projection helpers for public compiled-stage-graph exports.

use std::{collections::HashSet, fmt};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::contracts::{CompiledRunPlan, FrozenGraphPlanePlan};
use crate::contracts::{
    CompiledStageGraphExport, GraphExportContractError, GraphExportEdge, GraphExportEntry,
    GraphExportNode, GraphExportTerminalState, Plane, Timestamp,
};

/// Result type for compiled-stage-graph export projection.
pub type CompilerGraphExportResult<T> = Result<T, CompilerGraphExportError>;

/// Failures produced while projecting compiled-stage-graph exports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompilerGraphExportError {
    /// The frozen plan does not include the requested plane graph.
    MissingPlane {
        /// Runtime plane missing from the compiled plan.
        plane: Plane,
    },
    /// The current UTC timestamp could not be rendered or parsed.
    Time {
        /// Human-readable failure reason.
        message: String,
    },
    /// The projected graph export failed its public contract validation.
    Contract {
        /// Contract validation failure.
        message: String,
    },
}

impl fmt::Display for CompilerGraphExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPlane { plane } => {
                write!(
                    f,
                    "compiled plan does not include plane: {}",
                    plane.as_str()
                )
            }
            Self::Time { message } => {
                write!(f, "failed to create graph export timestamp: {message}")
            }
            Self::Contract { message } => {
                write!(f, "invalid compiled stage graph export: {message}")
            }
        }
    }
}

impl std::error::Error for CompilerGraphExportError {}

/// Project every selected plane graph from a compiled plan in stable plane order.
pub fn export_compiled_stage_graphs(
    plan: &CompiledRunPlan,
) -> CompilerGraphExportResult<Vec<CompiledStageGraphExport>> {
    let exported_at = utc_now_timestamp()?;
    export_compiled_stage_graphs_at(plan, exported_at)
}

/// Project every selected plane graph from a compiled plan using a supplied timestamp.
pub fn export_compiled_stage_graphs_at(
    plan: &CompiledRunPlan,
    exported_at: Timestamp,
) -> CompilerGraphExportResult<Vec<CompiledStageGraphExport>> {
    sorted_plane_graphs(plan)
        .into_iter()
        .map(|(_, graph)| export_graph(plan, graph, exported_at.clone()))
        .collect()
}

/// Project one selected plane graph from a compiled plan.
pub fn export_compiled_stage_graph(
    plan: &CompiledRunPlan,
    plane: Plane,
) -> CompilerGraphExportResult<CompiledStageGraphExport> {
    let exported_at = utc_now_timestamp()?;
    export_compiled_stage_graph_at(plan, plane, exported_at)
}

/// Project one selected plane graph from a compiled plan using a supplied timestamp.
pub fn export_compiled_stage_graph_at(
    plan: &CompiledRunPlan,
    plane: Plane,
    exported_at: Timestamp,
) -> CompilerGraphExportResult<CompiledStageGraphExport> {
    let graph = plan
        .graphs_by_plane
        .get(&plane)
        .ok_or(CompilerGraphExportError::MissingPlane { plane })?;
    export_graph(plan, graph, exported_at)
}

fn export_graph(
    plan: &CompiledRunPlan,
    graph: &FrozenGraphPlanePlan,
    exported_at: Timestamp,
) -> CompilerGraphExportResult<CompiledStageGraphExport> {
    let nodes_by_id: HashSet<&str> = graph
        .nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect();
    let export = CompiledStageGraphExport {
        schema_version: "1.0".to_owned(),
        kind: "compiled_stage_graph".to_owned(),
        compiled_plan_id: plan.compiled_plan_id.clone(),
        mode_id: plan.mode_id.clone(),
        loop_id: graph.loop_id.clone(),
        plane: graph.plane,
        nodes: graph
            .nodes
            .iter()
            .map(|node| GraphExportNode {
                node_id: node.node_id.clone(),
                plane: node.plane,
                stage_kind_id: node.stage_kind_id.clone(),
                lane_id: node.lane_id.clone(),
                entrypoint_path: node.entrypoint_path.clone(),
                entrypoint_contract_id: node.entrypoint_contract_id.clone(),
                running_status_marker: node.running_status_marker.clone(),
                required_skill_paths: node.required_skill_paths.clone(),
                attached_skill_additions: node.attached_skill_additions.clone(),
                runner_name: node.runner_name.clone(),
                model_name: node.model_name.clone(),
                thinking_level: node.thinking_level.clone(),
                model_reasoning_effort: node.model_reasoning_effort.clone(),
                timeout_seconds: node.timeout_seconds,
                allowed_result_classes_by_outcome: node.allowed_result_classes_by_outcome.clone(),
                declared_output_artifacts: node.declared_output_artifacts.clone(),
                execution_capability_grants: node.execution_capability_grants.clone(),
                execution_capability_warnings: node.execution_capability_warnings.clone(),
                execution_capability_policy_fingerprint: node
                    .execution_capability_policy_fingerprint
                    .clone(),
                request_context_profile_id: node.request_context_profile_id.clone(),
                terminal_action_mappings: node.terminal_action_mappings.clone(),
                runtime_effect_rule_selections: node.runtime_effect_rule_selections.clone(),
            })
            .collect(),
        edges: graph
            .compiled_transitions
            .iter()
            .map(|edge| GraphExportEdge {
                edge_id: edge.edge_id.clone(),
                source_node_id: edge.source_node_id.clone(),
                outcome: edge.outcome.clone(),
                target_node_id: edge.target_node_id.clone(),
                terminal_state_id: edge.terminal_state_id.clone(),
                kind: edge.kind.as_str().to_owned(),
                priority: edge.priority,
                max_attempts: edge.max_attempts,
            })
            .collect(),
        entries: graph
            .compiled_entries
            .iter()
            .filter(|entry| nodes_by_id.contains(entry.node_id.as_str()))
            .map(|entry| GraphExportEntry {
                entry_key: entry.entry_key.as_str().to_owned(),
                node_id: entry.node_id.clone(),
                stage_kind_id: entry.stage_kind_id.clone(),
                plane: entry.plane,
            })
            .collect(),
        terminal_states: graph
            .terminal_states
            .iter()
            .map(|state| GraphExportTerminalState {
                terminal_state_id: state.terminal_state_id.clone(),
                terminal_class: state.terminal_class.as_str().to_owned(),
                writes_status: state.writes_status.clone(),
                emits_artifacts: state.emits_artifacts.clone(),
                ends_plane_run: state.ends_plane_run,
            })
            .collect(),
        lane_policy: plan.lane_policy.clone(),
        workspace_schema_epoch: plan.workspace_schema_epoch.clone(),
        workflow_primitive_fingerprints: plan.workflow_primitive_fingerprints.clone(),
        source_refs: plan.source_refs.clone(),
        exported_at,
    };
    export.validate().map_err(contract_error)?;
    Ok(export)
}

fn sorted_plane_graphs(plan: &CompiledRunPlan) -> Vec<(Plane, &FrozenGraphPlanePlan)> {
    let mut entries: Vec<_> = plan
        .graphs_by_plane
        .iter()
        .map(|(plane, graph)| (*plane, graph))
        .collect();
    entries.sort_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
    entries
}

fn utc_now_timestamp() -> CompilerGraphExportResult<Timestamp> {
    let rendered = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| CompilerGraphExportError::Time {
            message: error.to_string(),
        })?;
    Timestamp::parse("exported_at", &rendered).map_err(|error| CompilerGraphExportError::Time {
        message: error.to_string(),
    })
}

fn contract_error(error: GraphExportContractError) -> CompilerGraphExportError {
    CompilerGraphExportError::Contract {
        message: error.to_string(),
    }
}
