use std::{collections::BTreeMap, process::ExitCode};

use crate::contracts::{BlockedTaskRequeueResult, CompiledStageGraphExport, RunTraceGraph};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    exit_code: u8,
    stdout_lines: Vec<String>,
    stderr_lines: Vec<String>,
}

impl CliOutput {
    pub fn success(lines: Vec<String>) -> Self {
        Self {
            exit_code: 0,
            stdout_lines: lines,
            stderr_lines: Vec::new(),
        }
    }

    pub fn stdout_failure(message: impl Into<String>) -> Self {
        Self {
            exit_code: 1,
            stdout_lines: vec![format!("error: {}", message.into())],
            stderr_lines: Vec::new(),
        }
    }

    pub fn stderr_failure(message: impl Into<String>) -> Self {
        Self {
            exit_code: 1,
            stdout_lines: Vec::new(),
            stderr_lines: vec![format!("error: {}", message.into())],
        }
    }

    pub fn parse_error(message: impl Into<String>) -> Self {
        Self {
            exit_code: 2,
            stdout_lines: Vec::new(),
            stderr_lines: vec![format!("error: {}", message.into())],
        }
    }

    pub fn with_exit_code(lines: Vec<String>, exit_code: u8) -> Self {
        Self {
            exit_code,
            stdout_lines: lines,
            stderr_lines: Vec::new(),
        }
    }
}

pub fn render_output(output: CliOutput) -> ExitCode {
    for line in output.stdout_lines {
        println!("{line}");
    }
    for line in output.stderr_lines {
        eprintln!("{line}");
    }
    ExitCode::from(output.exit_code)
}

pub fn compiled_graph_lines(graphs: &[CompiledStageGraphExport]) -> Vec<String> {
    if graphs.is_empty() {
        return vec!["compiled_graphs: none".to_owned()];
    }

    let mut lines = vec![
        format!("compiled_plan_id: {}", graphs[0].compiled_plan_id),
        format!("mode_id: {}", graphs[0].mode_id),
        format!(
            "planes: {}",
            graphs
                .iter()
                .map(|graph| graph.plane.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ];
    if let Some(epoch) = &graphs[0].workspace_schema_epoch {
        lines.push(format!("workspace_schema_epoch: {}", epoch.epoch_id));
    }
    if let Some(lane_policy) = &graphs[0].lane_policy {
        lines.push(format!("lane_policy: {}", lane_policy.policy_id));
    }
    for (collection, fingerprint) in &graphs[0].workflow_primitive_fingerprints {
        lines.push(format!(
            "workflow_primitive_fingerprint.{collection}: {fingerprint}"
        ));
    }
    for graph in graphs {
        lines.push(String::new());
        lines.push(format!("{}:", graph.plane.as_str()));
        for node in &graph.nodes {
            lines.push(format!(
                "  node {} lane={} request_context_profile_id={}",
                node.node_id,
                option_text(node.lane_id.as_deref()),
                option_text(node.request_context_profile_id.as_deref())
            ));
            for rule_id in &node.runtime_effect_rule_selections {
                lines.push(format!(
                    "  runtime_effect_rule_selection {} {}",
                    node.node_id, rule_id
                ));
            }
        }
        for edge in &graph.edges {
            let target = edge.target_node_id.clone().unwrap_or_else(|| {
                format!(
                    "terminal:{}",
                    option_text(edge.terminal_state_id.as_deref())
                )
            });
            lines.push(format!(
                "  {} --{}--> {}",
                edge.source_node_id, edge.outcome, target
            ));
        }
    }
    lines
}

pub fn run_trace_lines(trace: &RunTraceGraph) -> Vec<String> {
    let mut lines = vec![
        format!("run_id: {}", trace.run_id),
        format!("status: {}", trace.status.as_str()),
        format!(
            "compiled_plan_id: {}",
            option_text(trace.compiled_plan_id.as_deref())
        ),
        format!("mode_id: {}", option_text(trace.mode_id.as_deref())),
        format!(
            "request_kind: {}",
            option_text(trace.request_kind.as_deref())
        ),
        format!(
            "work_item_kind: {}",
            option_text(trace.work_item_kind.as_deref())
        ),
        format!(
            "work_item_id: {}",
            option_text(trace.work_item_id.as_deref())
        ),
        format!("node_count: {}", trace.nodes.len()),
        format!("edge_count: {}", trace.edges.len()),
    ];
    for note in &trace.notes {
        lines.push(format!("note: {note}"));
    }
    if trace.edges.is_empty() {
        for node in &trace.nodes {
            lines.push(format!("{} {}", node.stage, node.terminal_result));
        }
        return lines;
    }

    let nodes_by_trace_id: BTreeMap<&str, &str> = trace
        .nodes
        .iter()
        .map(|node| (node.trace_node_id.as_str(), node.stage.as_str()))
        .collect();
    for edge in &trace.edges {
        let source_label = nodes_by_trace_id
            .get(edge.source_trace_node_id.as_str())
            .copied()
            .unwrap_or(edge.source_trace_node_id.as_str());
        let target = edge
            .target_trace_node_id
            .as_deref()
            .or(edge.target_node_id.as_deref())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "terminal:{}",
                    option_text(edge.terminal_state_id.as_deref())
                )
            });
        lines.push(format!("{source_label} {} -> {target}", edge.outcome));
    }
    lines
}

pub fn blocked_task_requeue_lines(result: &BlockedTaskRequeueResult) -> Vec<String> {
    vec![
        format!("requeued_task: {}", result.task_id),
        format!("source_state: {}", result.source_state),
        format!("destination_state: {}", result.destination_state),
        format!("source_path: {}", result.source_path),
        format!("destination_path: {}", result.destination_path),
        format!("actor: {}", result.actor),
        format!("auto: {}", bool_text(result.auto)),
        format!("attempt_number: {}", result.attempt_number),
        format!(
            "failure_class: {}",
            option_text(result.failure_class.as_deref())
        ),
    ]
}

fn option_text(value: Option<&str>) -> &str {
    value.unwrap_or("none")
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}
