//! Run-trace graph persistence and fallback inspection helpers.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    contracts::{
        RunTraceArtifactRef, RunTraceEdge, RunTraceGraph, RunTraceNode, RunTraceSpawnedWorkKind,
        RunTraceSpawnedWorkRef, RunTraceStatus, RuntimeJsonContract, RuntimeJsonError,
        StageResultEnvelope, Timestamp,
    },
    workspace::{WorkspacePaths, atomic_write_text},
};

use super::tick::{RouterAction, RouterDecision};

/// Result type for run-trace inspection and persistence helpers.
pub type RunTraceResult<T> = Result<T, RunTraceError>;

/// Failures produced while reading or deriving run-trace graphs.
#[derive(Debug)]
pub enum RunTraceError {
    /// Filesystem access failed.
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying IO error message.
        message: String,
    },
    /// JSON syntax, required-field, type, or enum decoding failed.
    Json(RuntimeJsonError),
    /// A timestamp could not be produced.
    Time {
        /// Timestamp field being built.
        field_name: &'static str,
        /// Human-readable failure reason.
        message: String,
    },
}

impl fmt::Display for RunTraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => {
                write!(
                    f,
                    "run-trace filesystem error at {}: {message}",
                    path.display()
                )
            }
            Self::Json(error) => write!(f, "{error}"),
            Self::Time {
                field_name,
                message,
            } => write!(f, "failed to build timestamp {field_name}: {message}"),
        }
    }
}

impl std::error::Error for RunTraceError {}

impl From<RuntimeJsonError> for RunTraceError {
    fn from(value: RuntimeJsonError) -> Self {
        Self::Json(value)
    }
}

/// Returns the canonical trace artifact path inside a run directory.
#[must_use]
pub fn trace_path_for_run_dir(run_dir: impl AsRef<Path>) -> PathBuf {
    run_dir.as_ref().join("run_trace.json")
}

/// Read a run trace, deriving a read-only fallback from stage results when needed.
pub fn inspect_run_trace(run_dir: impl AsRef<Path>) -> RunTraceResult<RunTraceGraph> {
    let run_dir = absolute_path(run_dir.as_ref());
    let trace_path = trace_path_for_run_dir(&run_dir);
    if trace_path.is_file() {
        match fs::read_to_string(&trace_path)
            .map_err(|error| io_error(&trace_path, error))
            .and_then(|raw| RunTraceGraph::from_json_str(&raw).map_err(Into::into))
        {
            Ok(trace) => return Ok(trace),
            Err(error) => {
                return derive_run_trace_from_stage_results(
                    &run_dir,
                    RunTraceStatus::Malformed,
                    vec![format!("run_trace.json malformed: {error}")],
                );
            }
        }
    }

    derive_run_trace_from_stage_results(
        &run_dir,
        RunTraceStatus::Incomplete,
        vec!["derived from stage result artifacts".to_owned()],
    )
}

/// Read a run trace by id under a workspace runs directory without mutating the run.
pub fn inspect_run_trace_id(
    paths: &WorkspacePaths,
    run_id: &str,
) -> RunTraceResult<Option<RunTraceGraph>> {
    if run_id.is_empty()
        || run_id == "."
        || run_id == ".."
        || run_id.contains('/')
        || run_id.contains('\\')
    {
        return Ok(None);
    }
    let run_dir = paths.runs_dir.join(run_id);
    if !run_dir.is_dir() {
        return Ok(None);
    }
    inspect_run_trace(run_dir).map(Some)
}

/// Derive a fallback trace graph from persisted stage-result artifacts.
pub fn derive_run_trace_from_stage_results(
    run_dir: impl AsRef<Path>,
    status: RunTraceStatus,
    notes: Vec<String>,
) -> RunTraceResult<RunTraceGraph> {
    let run_dir = absolute_path(run_dir.as_ref());
    let stage_results_dir = run_dir.join("stage_results");
    let mut stage_result_paths = if stage_results_dir.is_dir() {
        json_files(&stage_results_dir)?
    } else {
        Vec::new()
    };
    stage_result_paths.sort();

    let mut nodes = Vec::new();
    let mut collected_notes = notes;
    for (index, stage_result_path) in stage_result_paths.iter().enumerate() {
        match fs::read_to_string(stage_result_path)
            .map_err(|error| io_error(stage_result_path, error))
            .and_then(|raw| StageResultEnvelope::from_json_str(&raw).map_err(Into::into))
        {
            Ok(stage_result) => nodes.push(node_from_stage_result(
                &run_dir,
                &stage_result,
                stage_result_path,
                Some(index + 1),
            )),
            Err(error) => collected_notes.push(format!(
                "{}: invalid stage result: {error}",
                stage_result_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
            )),
        }
    }

    let first = nodes.first();
    let latest = nodes.last();
    let mut graph = RunTraceGraph {
        schema_version: "1.0".to_owned(),
        kind: "run_trace_graph".to_owned(),
        run_id: run_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned(),
        run_dir: run_dir.display().to_string(),
        compiled_plan_id: latest.and_then(|node| node.compiled_plan_id.clone()),
        mode_id: latest.and_then(|node| node.mode_id.clone()),
        request_kind: latest.and_then(|node| node.request_kind.clone()),
        work_item_kind: latest.and_then(|node| node.work_item_kind.clone()),
        work_item_id: latest.and_then(|node| node.work_item_id.clone()),
        closure_target_root_spec_id: latest
            .and_then(|node| node.closure_target_root_spec_id.clone()),
        status,
        started_at: first.map(|node| node.started_at.clone()),
        completed_at: latest.map(|node| node.completed_at.clone()),
        duration_seconds: run_duration_seconds(first, latest),
        nodes,
        edges: Vec::new(),
        notes: collected_notes,
        generated_at: utc_now_timestamp("generated_at")?,
    };
    graph.validate_contract()?;
    Ok(graph)
}

/// Best-effort trace-node update after stage-result persistence.
pub fn upsert_stage_result_trace_node(
    paths: &WorkspacePaths,
    run_dir: &Path,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) {
    if let Err(error) = try_upsert_stage_result_trace_node(run_dir, stage_result, stage_result_path)
    {
        write_run_trace_failure_event(
            paths,
            &stage_result.run_id,
            "node",
            &error,
            &stage_result.completed_at,
        );
    }
}

/// Best-effort trace-edge update after authoritative routing has been applied.
pub fn record_router_decision_trace(
    paths: &WorkspacePaths,
    run_dir: &Path,
    stage_result: &StageResultEnvelope,
    decision: &RouterDecision,
    spawned_work: Vec<RunTraceSpawnedWorkRef>,
) {
    if let Err(error) =
        try_record_router_decision_trace(run_dir, stage_result, decision, spawned_work)
    {
        write_run_trace_failure_event(
            paths,
            &stage_result.run_id,
            "edge",
            &error,
            &stage_result.completed_at,
        );
    }
}

/// Build spawned-work edge evidence from a queue path.
#[must_use]
pub fn spawned_work_ref_from_path(
    path: &Path,
    source_stage_result: &StageResultEnvelope,
    reason: impl Into<String>,
) -> RunTraceSpawnedWorkRef {
    RunTraceSpawnedWorkRef {
        kind: spawned_kind_from_path(path),
        item_id: path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned(),
        path: Some(path.to_string_lossy().replace('\\', "/")),
        reason: Some(reason.into()),
        source_stage_node_id: Some(source_stage_result.node_id.clone()),
        source_terminal_result: Some(source_stage_result.terminal_result.as_str().to_owned()),
    }
}

fn try_upsert_stage_result_trace_node(
    run_dir: &Path,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> RunTraceResult<()> {
    let mut trace = load_or_derive_for_update(run_dir)?;
    let node = node_from_stage_result(run_dir, stage_result, stage_result_path, None);
    trace
        .nodes
        .retain(|existing| existing.trace_node_id != node.trace_node_id);
    trace.nodes.push(node.clone());
    trace.nodes.sort_by(|left, right| {
        (
            left.started_at.as_str(),
            left.completed_at.as_str(),
            left.trace_node_id.as_str(),
        )
            .cmp(&(
                right.started_at.as_str(),
                right.completed_at.as_str(),
                right.trace_node_id.as_str(),
            ))
    });
    for edge in &mut trace.edges {
        link_edge_target(edge, &node);
    }
    sort_trace_nodes(&mut trace.nodes, &trace.edges);
    let notes = without_derived_note(&trace.notes);
    update_trace_header(
        &mut trace,
        stage_result,
        RunTraceStatus::Active,
        Some(notes),
    )?;
    write_trace(&trace_path_for_run_dir(run_dir), &trace)
}

fn try_record_router_decision_trace(
    run_dir: &Path,
    stage_result: &StageResultEnvelope,
    decision: &RouterDecision,
    spawned_work: Vec<RunTraceSpawnedWorkRef>,
) -> RunTraceResult<()> {
    let mut trace = load_or_derive_for_update(run_dir)?;
    let source_trace_node_id = trace_node_id(stage_result, None);
    let edge = edge_from_decision(stage_result, decision, source_trace_node_id, spawned_work);
    trace
        .edges
        .retain(|existing| existing.trace_edge_id != edge.trace_edge_id);
    trace.edges.push(edge);
    sort_trace_nodes(&mut trace.nodes, &trace.edges);
    let notes = without_derived_note(&trace.notes);
    update_trace_header(
        &mut trace,
        stage_result,
        status_from_decision(decision),
        Some(notes),
    )?;
    write_trace(&trace_path_for_run_dir(run_dir), &trace)
}

fn load_or_derive_for_update(run_dir: &Path) -> RunTraceResult<RunTraceGraph> {
    let trace_path = trace_path_for_run_dir(run_dir);
    if trace_path.is_file() {
        if let Ok(raw) = fs::read_to_string(&trace_path) {
            if let Ok(trace) = RunTraceGraph::from_json_str(&raw) {
                return Ok(trace);
            }
        }
        return derive_run_trace_from_stage_results(
            run_dir,
            RunTraceStatus::Malformed,
            vec!["run_trace.json malformed; regenerated from stage result artifacts".to_owned()],
        );
    }
    derive_run_trace_from_stage_results(
        run_dir,
        RunTraceStatus::Incomplete,
        vec!["derived from stage result artifacts".to_owned()],
    )
}

fn node_from_stage_result(
    run_dir: &Path,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
    fallback_index: Option<usize>,
) -> RunTraceNode {
    let request_id = trace_node_id(stage_result, fallback_index);
    RunTraceNode {
        trace_node_id: request_id.clone(),
        run_id: stage_result.run_id.clone(),
        request_id,
        plane: stage_result.plane,
        stage: stage_result.stage.as_str().to_owned(),
        node_id: stage_result.node_id.clone(),
        stage_kind_id: stage_result.stage_kind_id.clone(),
        compiled_plan_id: string_metadata(stage_result, "compiled_plan_id"),
        mode_id: string_metadata(stage_result, "mode_id"),
        request_kind: string_metadata(stage_result, "request_kind"),
        work_item_kind: Some(stage_result.work_item_kind.as_str().to_owned()),
        work_item_id: Some(stage_result.work_item_id.clone()),
        closure_target_root_spec_id: string_metadata(stage_result, "closure_target_root_spec_id"),
        terminal_result: stage_result.terminal_result.as_str().to_owned(),
        result_class: stage_result.result_class,
        failure_class: string_metadata(stage_result, "failure_class"),
        runner_name: stage_result.runner_name.clone(),
        model_name: stage_result.model_name.clone(),
        thinking_level: stage_result.thinking_level.clone(),
        model_reasoning_effort: stage_result.model_reasoning_effort.clone(),
        started_at: stage_result.started_at.clone(),
        completed_at: stage_result.completed_at.clone(),
        duration_seconds: stage_result.duration_seconds,
        token_usage: stage_result.token_usage.clone(),
        artifacts: artifact_refs(run_dir, stage_result, stage_result_path),
    }
}

fn artifact_refs(
    run_dir: &Path,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
) -> Vec<RunTraceArtifactRef> {
    let mut paths = Vec::new();
    paths.push((
        normalize_run_relative_path(run_dir, stage_result_path),
        "stage_result".to_owned(),
    ));
    for (path, kind) in [
        (stage_result.prompt_artifact.as_deref(), "prompt"),
        (stage_result.stdout_path.as_deref(), "stdout"),
        (stage_result.stderr_path.as_deref(), "stderr"),
        (stage_result.report_artifact.as_deref(), "report"),
    ] {
        if let Some(path) = path {
            paths.push((normalize_run_relative_path(run_dir, path), kind.to_owned()));
        }
    }
    for path in &stage_result.artifact_paths {
        paths.push((
            normalize_run_relative_path(run_dir, path),
            artifact_kind(path),
        ));
    }

    let mut refs = Vec::new();
    for (relative_path, kind) in paths {
        if refs
            .iter()
            .any(|reference: &RunTraceArtifactRef| reference.path == relative_path)
        {
            continue;
        }
        let absolute = run_dir.join(&relative_path);
        refs.push(RunTraceArtifactRef {
            path: relative_path,
            kind,
            size_bytes: absolute
                .metadata()
                .ok()
                .filter(|meta| meta.is_file())
                .map(|meta| meta.len()),
            sha256: None,
        });
    }
    refs
}

fn edge_from_decision(
    stage_result: &StageResultEnvelope,
    decision: &RouterDecision,
    source_trace_node_id: String,
    spawned_work: Vec<RunTraceSpawnedWorkRef>,
) -> RunTraceEdge {
    let target_node_id = (decision.action == RouterAction::RunStage)
        .then(|| decision.next_node_id.clone())
        .flatten();
    let terminal_state_id = target_node_id
        .is_none()
        .then(|| terminal_state_id(stage_result, decision))
        .flatten();
    let target_or_terminal = target_node_id.clone().unwrap_or_else(|| {
        format!(
            "terminal:{}",
            terminal_state_id.as_deref().unwrap_or("none")
        )
    });
    RunTraceEdge {
        trace_edge_id: format!(
            "{}--{}--{}",
            source_trace_node_id,
            stage_result.terminal_result.as_str(),
            target_or_terminal
        ),
        source_trace_node_id,
        outcome: stage_result.terminal_result.as_str().to_owned(),
        edge_kind: decision.action.as_str().to_owned(),
        target_node_id,
        target_trace_node_id: None,
        terminal_state_id,
        spawned_work,
        decision_reason: Some(decision.reason.clone()),
        decided_at: stage_result.completed_at.clone(),
    }
}

fn update_trace_header(
    trace: &mut RunTraceGraph,
    stage_result: &StageResultEnvelope,
    status: RunTraceStatus,
    notes: Option<Vec<String>>,
) -> RunTraceResult<()> {
    trace.compiled_plan_id = string_metadata(stage_result, "compiled_plan_id")
        .or_else(|| trace.compiled_plan_id.clone());
    trace.mode_id = string_metadata(stage_result, "mode_id").or_else(|| trace.mode_id.clone());
    trace.request_kind =
        string_metadata(stage_result, "request_kind").or_else(|| trace.request_kind.clone());
    trace.work_item_kind = Some(stage_result.work_item_kind.as_str().to_owned());
    trace.work_item_id = Some(stage_result.work_item_id.clone());
    trace.closure_target_root_spec_id =
        string_metadata(stage_result, "closure_target_root_spec_id")
            .or_else(|| trace.closure_target_root_spec_id.clone());
    trace.status = status;
    trace.started_at = trace.nodes.first().map(|node| node.started_at.clone());
    trace.completed_at = trace.nodes.last().map(|node| node.completed_at.clone());
    trace.duration_seconds = run_duration_seconds(trace.nodes.first(), trace.nodes.last());
    if let Some(notes) = notes {
        trace.notes = notes;
    }
    trace.generated_at = utc_now_timestamp("generated_at")?;
    trace.validate_contract()?;
    Ok(())
}

fn write_trace(trace_path: &Path, trace: &RunTraceGraph) -> RunTraceResult<()> {
    let payload = serde_json::to_string_pretty(trace).map_err(|error| RuntimeJsonError::Json {
        artifact: "run_trace_graph",
        message: error.to_string(),
    })? + "\n";
    atomic_write_text(trace_path, &payload).map_err(|error| RunTraceError::Io {
        path: trace_path.to_path_buf(),
        message: error.to_string(),
    })
}

fn trace_node_id(stage_result: &StageResultEnvelope, fallback_index: Option<usize>) -> String {
    if let Some(request_id) = string_metadata(stage_result, "request_id") {
        return request_id;
    }
    if let Some(index) = fallback_index {
        return format!("stage-result-{index:04}");
    }
    format!(
        "{}-{}",
        stage_result.stage.as_str(),
        stage_result.terminal_result.as_str().to_ascii_lowercase()
    )
}

fn terminal_state_id(
    stage_result: &StageResultEnvelope,
    decision: &RouterDecision,
) -> Option<String> {
    if decision.action == RouterAction::Blocked {
        return Some("blocked".to_owned());
    }
    Some(stage_result.terminal_result.as_str().to_ascii_lowercase())
}

fn status_from_decision(decision: &RouterDecision) -> RunTraceStatus {
    match decision.action {
        RouterAction::RunStage => RunTraceStatus::Active,
        RouterAction::Handoff => RunTraceStatus::Handoff,
        RouterAction::Blocked => RunTraceStatus::Blocked,
        RouterAction::Idle => RunTraceStatus::Complete,
    }
}

fn link_edge_target(edge: &mut RunTraceEdge, node: &RunTraceNode) {
    if edge.target_node_id.as_deref() == Some(node.node_id.as_str())
        && edge.target_trace_node_id.is_none()
    {
        edge.target_trace_node_id = Some(node.trace_node_id.clone());
    }
}

fn sort_trace_nodes(nodes: &mut [RunTraceNode], edges: &[RunTraceEdge]) {
    nodes.sort_by(|left, right| {
        (
            trace_node_depth(left, edges),
            left.started_at.as_str(),
            left.completed_at.as_str(),
            left.trace_node_id.as_str(),
        )
            .cmp(&(
                trace_node_depth(right, edges),
                right.started_at.as_str(),
                right.completed_at.as_str(),
                right.trace_node_id.as_str(),
            ))
    });
}

fn trace_node_depth(node: &RunTraceNode, edges: &[RunTraceEdge]) -> usize {
    let mut depth = 0;
    let mut current = node.trace_node_id.as_str();
    for _ in 0..edges.len() {
        let Some(parent) = edges
            .iter()
            .find(|edge| edge.target_trace_node_id.as_deref() == Some(current))
        else {
            break;
        };
        depth += 1;
        current = parent.source_trace_node_id.as_str();
    }
    depth
}

fn spawned_kind_from_path(path: &Path) -> RunTraceSpawnedWorkKind {
    if path
        .components()
        .any(|component| component.as_os_str().to_string_lossy() == "learning")
    {
        return RunTraceSpawnedWorkKind::LearningRequest;
    }
    if path
        .components()
        .any(|component| component.as_os_str().to_string_lossy() == "incidents")
    {
        return RunTraceSpawnedWorkKind::Incident;
    }
    if path
        .components()
        .any(|component| component.as_os_str().to_string_lossy() == "specs")
    {
        return RunTraceSpawnedWorkKind::Spec;
    }
    RunTraceSpawnedWorkKind::Task
}

fn artifact_kind(path: &str) -> String {
    let name = Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path);
    if name.ends_with(".json") {
        "json".to_owned()
    } else if name.ends_with(".md") {
        "report".to_owned()
    } else if name.contains("stdout") {
        "stdout".to_owned()
    } else if name.contains("stderr") {
        "stderr".to_owned()
    } else {
        "artifact".to_owned()
    }
}

fn normalize_run_relative_path(run_dir: &Path, path_value: impl AsRef<Path>) -> String {
    let original = path_value.as_ref();
    let candidate = if original.is_absolute() {
        original.to_path_buf()
    } else {
        run_dir.join(original)
    };
    let resolved = candidate.canonicalize().unwrap_or(candidate);
    let resolved_run_dir = run_dir
        .canonicalize()
        .unwrap_or_else(|_| run_dir.to_path_buf());
    resolved
        .strip_prefix(&resolved_run_dir)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| original.to_string_lossy().replace('\\', "/"))
}

fn without_derived_note(notes: &[String]) -> Vec<String> {
    notes
        .iter()
        .filter(|note| note.as_str() != "derived from stage result artifacts")
        .cloned()
        .collect()
}

fn string_metadata(stage_result: &StageResultEnvelope, key: &str) -> Option<String> {
    stage_result
        .metadata
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn run_duration_seconds(
    first: Option<&RunTraceNode>,
    latest: Option<&RunTraceNode>,
) -> Option<f64> {
    let (Some(first), Some(latest)) = (first, latest) else {
        return None;
    };
    let started_at = parse_timestamp(&first.started_at).ok()?;
    let completed_at = parse_timestamp(&latest.completed_at).ok()?;
    Some((completed_at - started_at).as_seconds_f64())
}

fn json_files(directory: &Path) -> RunTraceResult<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| io_error(directory, error))? {
        let entry = entry.map_err(|error| io_error(directory, error))?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn absolute_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    })
}

fn utc_now_timestamp(field_name: &'static str) -> RunTraceResult<Timestamp> {
    let value = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| RunTraceError::Time {
            field_name,
            message: error.to_string(),
        })?;
    Timestamp::parse(field_name, &value).map_err(|error| RunTraceError::Time {
        field_name,
        message: error.to_string(),
    })
}

fn parse_timestamp(timestamp: &Timestamp) -> Result<OffsetDateTime, time::error::Parse> {
    OffsetDateTime::parse(timestamp.as_str(), &Rfc3339)
}

fn io_error(path: &Path, error: io::Error) -> RunTraceError {
    RunTraceError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

fn write_run_trace_failure_event(
    paths: &WorkspacePaths,
    run_id: &str,
    phase: &str,
    error: &RunTraceError,
    occurred_at: &Timestamp,
) {
    let event_log = paths.logs_dir.join("runtime_events.jsonl");
    if let Some(parent) = event_log.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let payload = json!({
        "schema_version": "1.0",
        "kind": "runtime_event",
        "event_type": "run_trace_write_failed",
        "occurred_at": occurred_at.as_str(),
        "data": {
            "run_id": run_id,
            "phase": phase,
            "error": error.to_string(),
        },
    });
    let Ok(line) = serde_json::to_string(&payload).map(|line| line + "\n") else {
        return;
    };
    use std::io::Write as _;
    let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(event_log)
    else {
        return;
    };
    let _ = file.write_all(line.as_bytes());
}
