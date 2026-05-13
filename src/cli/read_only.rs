use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use serde_json::{Number, Value};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    assets,
    compiler::{
        CompiledPlanCurrentness, CompilerContract, ModeDefinition, canonical_mode_id,
        inspect_workspace_plan_currentness_for_paths,
    },
    contracts::{
        ClosureTargetState, PauseSource, Plane, ReadOnlyStatusPayload, RunTraceGraph,
        RuntimeErrorContext, RuntimeSnapshot, StageResultEnvelope, SubscriptionQuotaTelemetryState,
        TokenUsage, UsageGovernanceBlockerSource, WorkDocument, WorkItemKind,
        validate_safe_identifier,
    },
    runtime::{StageRunRequest, inspect_run_trace_id, load_runtime_startup_config},
    work_documents::read_work_document,
    workspace::{
        QueueInspectionEntry, QueueStore, WorkspacePaths, inspect_runtime_ownership_lock,
        list_deferred_root_spec_ids, load_baseline_manifest, load_snapshot,
        load_usage_governance_ledger, load_usage_governance_state,
    },
};

#[derive(Debug, Clone)]
struct InspectedStageResult {
    stage_result_path: String,
    envelope: StageResultEnvelope,
    request_id: Option<String>,
    compiled_plan_id: Option<String>,
    mode_id: Option<String>,
    request_kind: Option<String>,
    closure_target_root_spec_id: Option<String>,
    closure_target_root_idea_id: Option<String>,
    preferred_rubric_path: Option<String>,
    preferred_verdict_path: Option<String>,
    preferred_report_path: Option<String>,
    skill_revision_evidence_path: Option<String>,
    raw_exit_kind: Option<String>,
    raw_exit_code: Option<String>,
    failure_class: Option<String>,
    prompt_artifact: Option<String>,
    stdout_path: Option<String>,
    stderr_path: Option<String>,
    event_log_path: Option<String>,
    report_artifact: Option<String>,
    artifact_paths: Vec<String>,
}

#[derive(Debug, Clone)]
struct InspectedRunnerArtifact {
    kind: String,
    path: String,
    request_id: Option<String>,
    thinking_level: Option<String>,
    model_reasoning_effort: Option<String>,
}

#[derive(Debug, Clone)]
struct InspectedStageRequest {
    stage_request_path: String,
    request_id: String,
    stage: String,
    node_id: String,
    stage_kind_id: String,
    runner_name: Option<String>,
    model_name: Option<String>,
    thinking_level: Option<String>,
    model_reasoning_effort: Option<String>,
    timeout_seconds: u64,
}

#[derive(Debug, Clone)]
struct InspectedRunSummary {
    run_id: String,
    run_dir: PathBuf,
    status: String,
    compiled_plan_id: Option<String>,
    mode_id: Option<String>,
    request_kind: Option<String>,
    closure_target_root_spec_id: Option<String>,
    work_item_kind: Option<WorkItemKind>,
    work_item_id: Option<String>,
    failure_class: Option<String>,
    troubleshoot_report_path: Option<String>,
    primary_prompt_artifact_path: Option<String>,
    primary_stdout_path: Option<String>,
    primary_stderr_path: Option<String>,
    primary_event_log_path: Option<String>,
    primary_runner_invocation_path: Option<String>,
    primary_runner_completion_path: Option<String>,
    primary_skill_revision_evidence_path: Option<String>,
    stage_requests: Vec<InspectedStageRequest>,
    stage_results: Vec<InspectedStageResult>,
    malformed_stage_result_paths: Vec<String>,
    runner_artifacts: Vec<InspectedRunnerArtifact>,
    governance_ledger_stage_result_paths: Vec<String>,
    notes: Vec<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    duration_seconds: Option<f64>,
    token_usage: Option<TokenUsage>,
}

pub fn queue_ls_lines(paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    let store = QueueStore::from_paths(paths.clone());
    let entries = store
        .inspect_work_items()
        .map_err(|error| error.to_string())?;
    let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for entry in &entries {
        *counts
            .entry((
                entry.work_item_kind.as_str().to_owned(),
                entry.work_item_state.clone(),
            ))
            .or_default() += 1;
    }

    let execution_queue_depth = count_kind_state(&counts, "task", "queue");
    let probe_queue_depth = count_kind_state(&counts, "probe", "queue");
    let spec_queue_depth = count_kind_state(&counts, "spec", "queue");
    let incident_queue_depth = count_kind_state(&counts, "incident", "incoming");
    let planning_queue_depth =
        probe_queue_depth + spec_queue_depth + count_kind_state(&counts, "incident", "incoming");
    let learning_queue_depth = count_kind_state(&counts, "learning_request", "queue");
    let execution_active = count_kind_state(&counts, "task", "active");
    let probe_active = count_kind_state(&counts, "probe", "active");
    let spec_active = count_kind_state(&counts, "spec", "active");
    let incident_active = count_kind_state(&counts, "incident", "active");
    let planning_active =
        probe_active + spec_active + count_kind_state(&counts, "incident", "active");
    let learning_active = count_kind_state(&counts, "learning_request", "active");

    let mut lines = vec![
        format!("execution_queue_depth: {execution_queue_depth}"),
        format!("planning_queue_depth: {planning_queue_depth}"),
        format!("learning_queue_depth: {learning_queue_depth}"),
        format!("probe_queue_depth: {probe_queue_depth}"),
        format!("spec_queue_depth: {spec_queue_depth}"),
        format!("incident_queue_depth: {incident_queue_depth}"),
        format!("execution_active: {execution_active}"),
        format!("planning_active: {planning_active}"),
        format!("learning_active: {learning_active}"),
        format!(
            "active_task_count: {}",
            count_kind_state(&counts, "task", "active")
        ),
        format!("active_probe_count: {probe_active}"),
        format!("active_spec_count: {spec_active}"),
        format!("active_incident_count: {incident_active}"),
        format!("active_learning_request_count: {learning_active}"),
        format!(
            "task_queue_count: {}",
            count_kind_state(&counts, "task", "queue")
        ),
        format!(
            "task_done_count: {}",
            count_kind_state(&counts, "task", "done")
        ),
        format!(
            "task_blocked_count: {}",
            count_kind_state(&counts, "task", "blocked")
        ),
        format!("probe_queue_count: {probe_queue_depth}"),
        format!(
            "probe_done_count: {}",
            count_kind_state(&counts, "probe", "done")
        ),
        format!(
            "probe_blocked_count: {}",
            count_kind_state(&counts, "probe", "blocked")
        ),
        format!(
            "spec_queue_count: {}",
            count_kind_state(&counts, "spec", "queue")
        ),
        format!(
            "spec_done_count: {}",
            count_kind_state(&counts, "spec", "done")
        ),
        format!(
            "spec_blocked_count: {}",
            count_kind_state(&counts, "spec", "blocked")
        ),
        format!(
            "incident_incoming_count: {}",
            count_kind_state(&counts, "incident", "incoming")
        ),
        format!(
            "incident_resolved_count: {}",
            count_kind_state(&counts, "incident", "resolved")
        ),
        format!(
            "incident_blocked_count: {}",
            count_kind_state(&counts, "incident", "blocked")
        ),
        format!(
            "learning_request_queue_count: {}",
            count_kind_state(&counts, "learning_request", "queue")
        ),
        format!(
            "learning_request_done_count: {}",
            count_kind_state(&counts, "learning_request", "done")
        ),
        format!(
            "learning_request_blocked_count: {}",
            count_kind_state(&counts, "learning_request", "blocked")
        ),
        format!("work_item_count: {}", entries.len()),
    ];
    for entry in entries {
        lines.push(render_queue_entry(&entry));
    }
    Ok(lines)
}

pub fn queue_show_lines(paths: &WorkspacePaths, work_item_id: &str) -> Result<Vec<String>, String> {
    validate_safe_identifier(work_item_id, "work_item_id")
        .map_err(|error| format!("invalid work item id: {error}"))?;
    let store = QueueStore::from_paths(paths.clone());
    let Some(entry) = store
        .find_work_item(work_item_id)
        .map_err(|error| error.to_string())?
    else {
        return Err(format!("work item not found: {work_item_id}"));
    };

    let document = read_work_document(&entry.path)
        .map_err(|error| format!("queue document error at {}: {error}", entry.path.display()))?;
    let mut lines = vec![
        format!("work_item_id: {}", entry.work_item_id),
        format!("work_item_kind: {}", entry.work_item_kind.as_str()),
        format!("work_item_state: {}", entry.work_item_state),
        format!("path: {}", entry.path.display()),
        format!("title: {}", entry.title),
    ];
    lines.extend(render_queue_document_fields(&document));
    Ok(lines)
}

pub fn status_lines(paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    render_status_lines(paths)
}

pub fn status_json(paths: &WorkspacePaths) -> Result<String, String> {
    let status = inspect_status(paths)?;
    let value = serde_json::to_value(&status.payload)
        .map_err(|error| format!("failed to encode status JSON: {error}"))?;
    serde_json::to_string_pretty(&value)
        .map_err(|error| format!("failed to render status JSON: {error}"))
}

pub fn status_watch_lines(
    paths_list: &[WorkspacePaths],
    max_updates: Option<&str>,
    interval_seconds: Option<&str>,
) -> Result<Vec<String>, String> {
    let max_updates = match max_updates {
        Some(value) => parse_positive_usize("--max-updates", value)?,
        None => 1,
    };
    let interval_seconds = match interval_seconds {
        Some(value) => parse_non_negative_seconds("--interval-seconds", value)?,
        None => 1.0,
    };

    let mut lines = Vec::new();
    for update_index in 0..max_updates {
        if update_index > 0 {
            lines.push(String::new());
        }
        lines.extend(render_statuses(paths_list)?);
        if update_index + 1 < max_updates && interval_seconds > 0.0 {
            thread::sleep(Duration::from_secs_f64(interval_seconds));
        }
    }
    Ok(lines)
}

pub fn runs_ls_lines(paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for (index, summary) in list_runs(paths)?.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend([
            format!("run_id: {}", summary.run_id),
            format!("status: {}", summary.status),
            format!("work_item_kind: {}", option_value(summary.work_item_kind)),
            format!(
                "work_item_id: {}",
                option_str(summary.work_item_id.as_deref())
            ),
            format!(
                "failure_class: {}",
                option_str(summary.failure_class.as_deref())
            ),
            format!("stage_result_count: {}", summary.stage_results.len()),
            format!("runner_artifact_count: {}", summary.runner_artifacts.len()),
        ]);
    }
    Ok(lines)
}

pub fn runs_show_lines(paths: &WorkspacePaths, run_id: &str) -> Result<Vec<String>, String> {
    let Some(summary) = inspect_run_id(paths, run_id)? else {
        return Err(format!("run not found: {run_id}"));
    };
    Ok(render_run_show_lines(&summary))
}

pub fn runs_tail_payload(paths: &WorkspacePaths, run_id: &str) -> Result<String, String> {
    let Some(summary) = inspect_run_id(paths, run_id)? else {
        return Err(format!("run not found: {run_id}"));
    };
    let Some(artifact) = select_primary_run_artifact(&summary) else {
        return Err(format!("no tailable artifact found for run: {run_id}"));
    };
    let artifact_path = resolve_run_artifact_path(&summary.run_dir, &artifact);
    fs::read_to_string(&artifact_path)
        .map_err(|error| format!("failed to read tailable artifact {artifact}: {error}"))
}

pub fn runs_trace_graph(paths: &WorkspacePaths, run_id: &str) -> Result<RunTraceGraph, String> {
    let Some(trace) = inspect_run_trace_id(paths, run_id).map_err(|error| error.to_string())?
    else {
        return Err(format!("run not found: {run_id}"));
    };
    Ok(trace)
}

pub fn modes_list_lines() -> Result<Vec<String>, String> {
    let mut modes = embedded_modes()?;
    modes.sort_by(|left, right| left.mode_id.cmp(&right.mode_id));

    let mut lines = Vec::new();
    for mode in modes {
        lines.push(format!(
            "{}: execution_loop={} planning_loop={}",
            mode.mode_id,
            mode.loop_ids_by_plane
                .get(&Plane::Execution)
                .map(String::as_str)
                .unwrap_or("none"),
            mode.loop_ids_by_plane
                .get(&Plane::Planning)
                .map(String::as_str)
                .unwrap_or("none")
        ));
    }
    lines.push("standard_plain -> default_codex (compatibility alias)".to_owned());
    Ok(lines)
}

pub fn modes_show_lines(mode_id: &str) -> Result<Vec<String>, String> {
    let canonical_mode_id = canonical_mode_id(mode_id);
    let alias_target = (canonical_mode_id != mode_id).then(|| canonical_mode_id.clone());
    let Some(mode) = embedded_modes()?
        .into_iter()
        .find(|mode| mode.mode_id == canonical_mode_id)
    else {
        return Err(format!("unknown mode: {mode_id}"));
    };

    let mut lines = Vec::new();
    if let Some(alias_target) = alias_target {
        lines.push(format!("alias_of: {alias_target}"));
    }
    lines.extend([
        format!("mode_id: {}", mode.mode_id),
        format!(
            "execution_loop_id: {}",
            mode.loop_ids_by_plane
                .get(&Plane::Execution)
                .map(String::as_str)
                .unwrap_or("none")
        ),
        format!(
            "planning_loop_id: {}",
            mode.loop_ids_by_plane
                .get(&Plane::Planning)
                .map(String::as_str)
                .unwrap_or("none")
        ),
        format!(
            "learning_loop_id: {}",
            mode.loop_ids_by_plane
                .get(&Plane::Learning)
                .map(String::as_str)
                .unwrap_or("none")
        ),
    ]);
    Ok(lines)
}

pub fn config_show_lines(
    paths: &WorkspacePaths,
    config_path: Option<&str>,
) -> Result<Vec<String>, String> {
    let resolved_config_path = config_path
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.runtime_config_file.clone());
    let config =
        load_runtime_startup_config(&resolved_config_path).map_err(|error| error.to_string())?;
    let snapshot = load_snapshot(paths).map_err(|error| error.to_string())?;
    Ok(vec![
        format!("default_mode: {}", config.default_mode),
        format!("run_style: {}", config.run_style.as_str()),
        format!(
            "idle_sleep_seconds: {}",
            format_seconds(config.idle_sleep_seconds)
        ),
        format!("runners.default_runner: {}", config.runners.default_runner),
        format!("runners.codex.command: {}", config.runners.codex.command),
        format!(
            "runners.codex.args: {}",
            string_vec_text(&config.runners.codex.args)
        ),
        format!(
            "runners.codex.profile: {}",
            option_str(config.runners.codex.profile.as_deref())
        ),
        format!(
            "runners.codex.permission_default: {}",
            config.runners.codex.permission_default.as_str()
        ),
        format!(
            "runners.codex.permission_by_stage: {}",
            permission_map_text(&config.runners.codex.permission_by_stage)
        ),
        format!(
            "runners.codex.permission_by_model: {}",
            permission_map_text(&config.runners.codex.permission_by_model)
        ),
        format!(
            "runners.codex.model_reasoning_effort: {}",
            option_str(config.runners.codex.model_reasoning_effort.as_deref())
        ),
        format!(
            "runners.codex.skip_git_repo_check: {}",
            bool_text(config.runners.codex.skip_git_repo_check)
        ),
        format!(
            "runners.codex.extra_config: {}",
            string_vec_text(&config.runners.codex.extra_config)
        ),
        format!(
            "runners.codex.env: {}",
            string_map_text(&config.runners.codex.env)
        ),
        format!("runners.pi.command: {}", config.runners.pi.command),
        format!(
            "runners.pi.args: {}",
            string_vec_text(&config.runners.pi.args)
        ),
        format!(
            "runners.pi.provider: {}",
            option_str(config.runners.pi.provider.as_deref())
        ),
        format!(
            "runners.pi.thinking: {}",
            option_str(config.runners.pi.thinking.as_deref())
        ),
        format!(
            "runners.pi.disable_context_files: {}",
            bool_text(config.runners.pi.disable_context_files)
        ),
        format!(
            "runners.pi.disable_skills: {}",
            bool_text(config.runners.pi.disable_skills)
        ),
        format!(
            "runners.pi.event_log_policy: {}",
            config.runners.pi.event_log_policy.as_str()
        ),
        format!(
            "runners.pi.env: {}",
            string_map_text(&config.runners.pi.env)
        ),
        format!("stages.count: {}", config.stages.len()),
        format!("watchers.enabled: {}", bool_text(config.watchers_enabled)),
        format!("watchers.debounce_ms: {}", config.watchers_debounce_ms),
        format!(
            "watchers.watch_ideas_inbox: {}",
            bool_text(config.watchers_watch_ideas_inbox)
        ),
        format!(
            "watchers.watch_specs_queue: {}",
            bool_text(config.watchers_watch_specs_queue)
        ),
        format!(
            "auto_recovery.enabled: {}",
            bool_text(config.auto_recovery.enabled)
        ),
        format!(
            "usage_governance.enabled: {}",
            bool_text(config.usage_governance_enabled)
        ),
        format!(
            "usage_governance.auto_resume: {}",
            bool_text(config.usage_governance.auto_resume)
        ),
        format!(
            "usage_governance.evaluation_boundary: {}",
            config.usage_governance.evaluation_boundary.as_str()
        ),
        format!(
            "usage_governance.calendar_timezone: {}",
            config.usage_governance.calendar_timezone.as_str()
        ),
        format!(
            "usage_governance.runtime_token_rules.enabled: {}",
            bool_text(config.usage_governance.runtime_token_rules.enabled)
        ),
        format!(
            "usage_governance.runtime_token_rules.count: {}",
            config.usage_governance.runtime_token_rules.rules.len()
        ),
        format!(
            "usage_governance.subscription_quota_rules.enabled: {}",
            bool_text(config.usage_governance.subscription_quota_rules.enabled)
        ),
        format!(
            "usage_governance.subscription_quota_rules.degraded_policy: {}",
            config
                .usage_governance
                .subscription_quota_rules
                .degraded_policy
                .as_str()
        ),
        format!("config_version: {}", snapshot.config_version),
        format!(
            "last_reload_outcome: {}",
            option_value(snapshot.last_reload_outcome)
        ),
        format!(
            "last_reload_error: {}",
            option_str(snapshot.last_reload_error.as_deref())
        ),
    ])
}

fn render_statuses(paths_list: &[WorkspacePaths]) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for (index, paths) in paths_list.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(render_status_lines(paths)?);
    }
    Ok(lines)
}

fn render_status_lines(paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    let status = inspect_status(paths)?;
    let payload = &status.payload;
    let mut lines = vec![
        format!("workspace: {}", payload.workspace),
        format!("runtime_mode: {}", payload.runtime_mode.as_str()),
        format!("process_running: {}", bool_text(payload.process_running)),
        format!("runtime_ownership_lock: {}", payload.runtime_ownership_lock),
        format!("paused: {}", bool_text(payload.paused)),
        format!("pause_sources: {}", payload.pause_sources),
        format!("stop_requested: {}", bool_text(payload.stop_requested)),
        format!("active_mode_id: {}", payload.active_mode_id),
        format!("compiled_plan_id: {}", payload.compiled_plan_id),
        format!(
            "compiled_plan_currentness: {}",
            payload.compiled_plan_currentness
        ),
        format!("active_plane: {}", option_value(payload.active_plane)),
        format!("active_stage: {}", option_value(payload.active_stage)),
        format!(
            "active_node_id: {}",
            option_str(payload.active_node_id.as_deref())
        ),
        format!(
            "active_stage_kind_id: {}",
            option_str(payload.active_stage_kind_id.as_deref())
        ),
        format!(
            "active_work_item_kind: {}",
            option_value(payload.active_work_item_kind)
        ),
        format!(
            "active_work_item_id: {}",
            option_str(payload.active_work_item_id.as_deref())
        ),
        format!("active_run_count: {}", payload.active_run_count),
        format!("execution_queue_depth: {}", payload.execution_queue_depth),
        format!("planning_queue_depth: {}", payload.planning_queue_depth),
        format!("learning_queue_depth: {}", payload.learning_queue_depth),
        format!(
            "execution_status_marker: {}",
            payload.execution_status_marker
        ),
        format!("planning_status_marker: {}", payload.planning_status_marker),
        format!("learning_status_marker: {}", payload.learning_status_marker),
        format!("blocked_idle: {}", bool_text(payload.blocked_idle)),
        format!(
            "latest_runtime_error_report_path: {}",
            option_str(payload.latest_runtime_error_report_path.as_deref())
        ),
    ];
    lines.extend(render_active_run_lines(&status.snapshot));
    lines.extend(render_baseline_manifest_lines(
        status.baseline_manifest.as_ref(),
    ));
    lines.extend(render_compile_currentness_lines(
        status.currentness.as_ref(),
    ));
    lines.extend(render_usage_governance_lines(paths, &status.snapshot)?);
    lines.extend(render_closure_target_lines_from_payload(payload));
    if let Some(failure_class) = &status.snapshot.current_failure_class {
        lines.push(format!("current_failure_class: {failure_class}"));
        for (label, count) in [
            (
                "troubleshoot_attempt_count",
                status.snapshot.troubleshoot_attempt_count,
            ),
            (
                "mechanic_attempt_count",
                status.snapshot.mechanic_attempt_count,
            ),
            ("fix_cycle_count", status.snapshot.fix_cycle_count),
            (
                "consultant_invocations",
                status.snapshot.consultant_invocations,
            ),
        ] {
            if count > 0 {
                lines.push(format!("{label}: {count}"));
            }
        }
    }
    Ok(lines)
}

struct StatusInspection {
    snapshot: RuntimeSnapshot,
    baseline_manifest: Option<crate::workspace::BaselineManifest>,
    currentness: Result<CompiledPlanCurrentness, crate::compiler::CompilerPersistenceError>,
    payload: ReadOnlyStatusPayload,
}

fn inspect_status(paths: &WorkspacePaths) -> Result<StatusInspection, String> {
    let snapshot = load_snapshot(paths).map_err(|error| error.to_string())?;
    let baseline_manifest = load_baseline_manifest(paths).ok();
    let currentness = inspect_workspace_plan_currentness_for_paths(paths, None);
    let lock_status = inspect_runtime_ownership_lock(paths);
    let process_running = snapshot.process_running && lock_status.state.as_str() == "active";
    let queue_depths = status_queue_depths(paths)?;
    let closure_status = closure_target_status(paths)?;
    let latest_runtime_error_report_path = latest_runtime_error_report_path(paths)?;
    let active_run_count = snapshot.active_runs_by_plane.len() as u64;
    let blocked_idle = blocked_idle(
        process_running,
        active_run_count,
        &queue_depths,
        &closure_status,
        &snapshot,
    );

    let payload = ReadOnlyStatusPayload {
        workspace: paths.root.display().to_string(),
        runtime_mode: snapshot.runtime_mode,
        process_running,
        runtime_ownership_lock: lock_status.state.as_str().to_owned(),
        paused: snapshot.paused,
        pause_sources: pause_sources_label(&snapshot),
        stop_requested: snapshot.stop_requested,
        active_mode_id: snapshot.active_mode_id.clone(),
        compiled_plan_id: snapshot.compiled_plan_id.clone(),
        compiled_plan_currentness: compiled_plan_currentness_value(currentness.as_ref()).to_owned(),
        active_plane: snapshot.active_plane,
        active_stage: snapshot.active_stage,
        active_node_id: snapshot.active_node_id.clone(),
        active_stage_kind_id: snapshot.active_stage_kind_id.clone(),
        active_work_item_kind: snapshot.active_work_item_kind,
        active_work_item_id: snapshot.active_work_item_id.clone(),
        active_run_count,
        execution_queue_depth: queue_depths.execution,
        planning_queue_depth: queue_depths.planning,
        learning_queue_depth: queue_depths.learning,
        execution_status_marker: snapshot.execution_status_marker.clone(),
        planning_status_marker: snapshot.planning_status_marker.clone(),
        learning_status_marker: snapshot.learning_status_marker.clone(),
        blocked_idle,
        current_failure_class: snapshot.current_failure_class.clone(),
        latest_runtime_error_report_path,
        closure_target_root_spec_id: closure_status.root_spec_id,
        closure_target_open: closure_status.open,
        closure_target_blocked_by_lineage_work: closure_status.blocked_by_lineage_work,
        planning_root_specs_deferred_by_closure_target: closure_status.deferred_root_spec_count,
        closure_target_latest_verdict_path: closure_status.latest_verdict_path,
        closure_target_latest_report_path: closure_status.latest_report_path,
    };

    Ok(StatusInspection {
        snapshot,
        baseline_manifest,
        currentness,
        payload,
    })
}

fn render_active_run_lines(snapshot: &RuntimeSnapshot) -> Vec<String> {
    let mut lines = Vec::new();
    for plane in [Plane::Planning, Plane::Execution, Plane::Learning] {
        let Some(active_run) = snapshot.active_runs_by_plane.get(&plane) else {
            continue;
        };
        lines.push(format!(
            "active_run: plane={} stage={} node={} stage_kind={} request_kind={} work_item_kind={} work_item_id={} run={}",
            active_run.plane.as_str(),
            active_run.stage.as_str(),
            active_run.node_id,
            active_run.stage_kind_id,
            active_run.request_kind.as_str(),
            option_value(active_run.work_item_kind),
            option_str(active_run.work_item_id.as_deref()),
            short_run_handle(&active_run.run_id)
        ));
    }
    lines
}

fn render_baseline_manifest_lines(
    manifest: Option<&crate::workspace::BaselineManifest>,
) -> Vec<String> {
    match manifest {
        Some(manifest) => vec![
            format!("baseline_manifest_id: {}", manifest.manifest_id),
            format!(
                "baseline_seed_package_version: {}",
                manifest.seed_package_version
            ),
        ],
        None => vec![
            "baseline_manifest_id: none".to_owned(),
            "baseline_seed_package_version: none".to_owned(),
        ],
    }
}

fn render_compile_currentness_lines(
    currentness: Result<&CompiledPlanCurrentness, &crate::compiler::CompilerPersistenceError>,
) -> Vec<String> {
    match currentness {
        Ok(currentness) => {
            let mut lines = vec![
                format!(
                    "compile_input.mode_id: {}",
                    currentness.expected_fingerprint.mode_id
                ),
                format!(
                    "compile_input.config_fingerprint: {}",
                    currentness.expected_fingerprint.config_fingerprint
                ),
                format!(
                    "compile_input.assets_fingerprint: {}",
                    currentness.expected_fingerprint.assets_fingerprint
                ),
            ];
            match &currentness.persisted_fingerprint {
                Some(fingerprint) => lines.extend([
                    format!("persisted_compile_input.mode_id: {}", fingerprint.mode_id),
                    format!(
                        "persisted_compile_input.config_fingerprint: {}",
                        fingerprint.config_fingerprint
                    ),
                    format!(
                        "persisted_compile_input.assets_fingerprint: {}",
                        fingerprint.assets_fingerprint
                    ),
                ]),
                None => lines.extend([
                    "persisted_compile_input.mode_id: none".to_owned(),
                    "persisted_compile_input.config_fingerprint: none".to_owned(),
                    "persisted_compile_input.assets_fingerprint: none".to_owned(),
                ]),
            }
            lines
        }
        Err(error) => vec![
            "compile_input.mode_id: none".to_owned(),
            "compile_input.config_fingerprint: none".to_owned(),
            "compile_input.assets_fingerprint: none".to_owned(),
            format!("compile_plan_currentness_error: {error}"),
        ],
    }
}

fn render_usage_governance_lines(
    paths: &WorkspacePaths,
    snapshot: &RuntimeSnapshot,
) -> Result<Vec<String>, String> {
    if !paths.usage_governance_state_file.is_file() {
        return Ok(render_usage_governance_default_lines());
    }
    let state = load_usage_governance_state(paths).map_err(|error| error.to_string())?;
    let governance_paused = state.paused_by_governance
        || snapshot
            .pause_sources
            .contains(&PauseSource::UsageGovernance);
    let mut lines = vec![
        format!("usage_governance_enabled: {}", bool_text(state.enabled)),
        format!("usage_governance_paused: {}", bool_text(governance_paused)),
        format!(
            "usage_governance_blocker_count: {}",
            state.active_blockers.len()
        ),
        format!(
            "usage_governance_auto_resume_possible: {}",
            bool_text(state.auto_resume_possible)
        ),
        format!(
            "usage_governance_next_auto_resume_at: {}",
            option_value(state.next_auto_resume_at.as_ref())
        ),
        format!(
            "usage_governance_subscription_status: {}",
            subscription_status_text(state.subscription_quota_status.state)
        ),
    ];
    for blocker in state.active_blockers {
        let detail = if blocker.detail.is_empty() {
            "none"
        } else {
            blocker.detail.as_str()
        };
        lines.push(format!(
            "usage_governance_blocker: source={} rule={} window={} observed={} threshold={} auto_resume_possible={} next_resume={} detail={}",
            blocker_source_text(blocker.source),
            blocker.rule_id.as_str(),
            blocker.window.as_str(),
            format_metric_value(blocker.observed),
            format_metric_value(blocker.threshold),
            bool_text(blocker.auto_resume_possible),
            option_value(blocker.next_auto_resume_at.as_ref()),
            detail
        ));
    }
    if state.subscription_quota_status.enabled {
        lines.push(format!(
            "usage_governance_subscription_provider: {}",
            state.subscription_quota_status.provider.as_str()
        ));
        lines.push(format!(
            "usage_governance_subscription_detail: {}",
            option_str(state.subscription_quota_status.detail.as_deref())
        ));
    }
    Ok(lines)
}

fn render_usage_governance_default_lines() -> Vec<String> {
    vec![
        "usage_governance_enabled: false".to_owned(),
        "usage_governance_paused: false".to_owned(),
        "usage_governance_blocker_count: 0".to_owned(),
        "usage_governance_auto_resume_possible: false".to_owned(),
        "usage_governance_next_auto_resume_at: none".to_owned(),
        "usage_governance_subscription_status: none".to_owned(),
    ]
}

fn blocker_source_text(source: UsageGovernanceBlockerSource) -> &'static str {
    source.as_str()
}

fn subscription_status_text(state: SubscriptionQuotaTelemetryState) -> &'static str {
    state.as_str()
}

fn format_metric_value(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

struct StatusQueueDepths {
    execution: u64,
    planning: u64,
    learning: u64,
}

fn status_queue_depths(paths: &WorkspacePaths) -> Result<StatusQueueDepths, String> {
    Ok(StatusQueueDepths {
        execution: count_markdown_files(&paths.tasks_queue_dir)? as u64,
        planning: (count_markdown_files(&paths.specs_queue_dir)?
            + count_markdown_files(&paths.probes_queue_dir)?
            + count_markdown_files(&paths.incidents_incoming_dir)?) as u64,
        learning: count_markdown_files(&paths.learning_requests_queue_dir)? as u64,
    })
}

struct ClosureTargetStatus {
    root_spec_id: Value,
    open: Value,
    blocked_by_lineage_work: Value,
    deferred_root_spec_count: Value,
    latest_verdict_path: Value,
    latest_report_path: Value,
}

fn closure_target_default_status() -> ClosureTargetStatus {
    ClosureTargetStatus {
        root_spec_id: Value::Null,
        open: Value::Null,
        blocked_by_lineage_work: Value::Null,
        deferred_root_spec_count: Value::Number(Number::from(0)),
        latest_verdict_path: Value::Null,
        latest_report_path: Value::Null,
    }
}

fn closure_target_invalid_status() -> ClosureTargetStatus {
    ClosureTargetStatus {
        root_spec_id: Value::String("invalid_multiple_actionable_open_targets".to_owned()),
        open: Value::String("invalid".to_owned()),
        blocked_by_lineage_work: Value::String("invalid".to_owned()),
        deferred_root_spec_count: Value::String("invalid".to_owned()),
        latest_verdict_path: Value::Null,
        latest_report_path: Value::Null,
    }
}

fn closure_target_status(paths: &WorkspacePaths) -> Result<ClosureTargetStatus, String> {
    let open_targets = list_open_closure_targets(paths)?;
    let actionable_targets: Vec<&ClosureTargetState> = open_targets
        .iter()
        .filter(|target| !target.closure_blocked_by_lineage_work)
        .collect();
    if actionable_targets.len() > 1 {
        return Ok(closure_target_invalid_status());
    }
    if open_targets.is_empty() {
        return Ok(closure_target_default_status());
    }

    let target = actionable_targets
        .first()
        .copied()
        .unwrap_or_else(|| &open_targets[0]);
    let deferred_root_spec_ids = list_deferred_root_spec_ids(paths, &target.root_spec_id)
        .map_err(|error| error.to_string())?;
    Ok(ClosureTargetStatus {
        root_spec_id: Value::String(target.root_spec_id.clone()),
        open: Value::Bool(target.closure_open),
        blocked_by_lineage_work: Value::Bool(target.closure_blocked_by_lineage_work),
        deferred_root_spec_count: Value::Number(Number::from(deferred_root_spec_ids.len() as u64)),
        latest_verdict_path: option_string_value(target.latest_verdict_path.as_deref()),
        latest_report_path: option_string_value(target.latest_report_path.as_deref()),
    })
}

fn render_closure_target_lines_from_payload(payload: &ReadOnlyStatusPayload) -> Vec<String> {
    vec![
        format!(
            "closure_target_root_spec_id: {}",
            status_value_text(&payload.closure_target_root_spec_id)
        ),
        format!(
            "closure_target_open: {}",
            status_value_text(&payload.closure_target_open)
        ),
        format!(
            "closure_target_blocked_by_lineage_work: {}",
            status_value_text(&payload.closure_target_blocked_by_lineage_work)
        ),
        format!(
            "planning_root_specs_deferred_by_closure_target: {}",
            status_value_text(&payload.planning_root_specs_deferred_by_closure_target)
        ),
        format!(
            "closure_target_latest_verdict_path: {}",
            status_value_text(&payload.closure_target_latest_verdict_path)
        ),
        format!(
            "closure_target_latest_report_path: {}",
            status_value_text(&payload.closure_target_latest_report_path)
        ),
    ]
}

fn status_value_text(value: &Value) -> String {
    match value {
        Value::Null => "none".to_owned(),
        Value::Bool(value) => bool_text(*value).to_owned(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

fn option_string_value(value: Option<&str>) -> Value {
    value
        .map(|value| Value::String(value.to_owned()))
        .unwrap_or(Value::Null)
}

fn latest_runtime_error_report_path(paths: &WorkspacePaths) -> Result<Option<String>, String> {
    if !paths.runtime_error_context_file.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&paths.runtime_error_context_file).map_err(|error| {
        format!(
            "failed to read runtime error context {}: {error}",
            paths.runtime_error_context_file.display()
        )
    })?;
    let context = RuntimeErrorContext::from_json_str(&raw).map_err(|error| {
        format!(
            "failed to decode runtime error context {}: {error}",
            paths.runtime_error_context_file.display()
        )
    })?;
    Ok(Some(context.report_path))
}

fn blocked_idle(
    process_running: bool,
    active_run_count: u64,
    queue_depths: &StatusQueueDepths,
    closure_status: &ClosureTargetStatus,
    snapshot: &RuntimeSnapshot,
) -> bool {
    process_running
        && active_run_count == 0
        && queue_depths.execution == 0
        && queue_depths.planning == 0
        && queue_depths.learning == 0
        && closure_status.open == Value::Bool(true)
        && closure_status.blocked_by_lineage_work == Value::Bool(true)
        && snapshot.planning_status_marker == "### BLOCKED"
        && snapshot.current_failure_class.is_some()
}

fn list_open_closure_targets(paths: &WorkspacePaths) -> Result<Vec<ClosureTargetState>, String> {
    let mut targets = Vec::new();
    if !paths.arbiter_targets_dir.exists() {
        return Ok(targets);
    }
    for path in json_files(&paths.arbiter_targets_dir)? {
        let raw = fs::read_to_string(&path).map_err(|error| {
            format!("failed to read closure target {}: {error}", path.display())
        })?;
        let target: ClosureTargetState = serde_json::from_str(&raw).map_err(|error| {
            format!(
                "failed to decode closure target {}: {error}",
                path.display()
            )
        })?;
        target
            .validate()
            .map_err(|error| format!("invalid closure target {}: {error}", path.display()))?;
        if target.closure_open {
            targets.push(target);
        }
    }
    targets.sort_by(|left, right| left.root_spec_id.cmp(&right.root_spec_id));
    Ok(targets)
}

fn compiled_plan_currentness_value(
    currentness: Result<&CompiledPlanCurrentness, &crate::compiler::CompilerPersistenceError>,
) -> String {
    currentness
        .map(|currentness| currentness.state.as_str().to_owned())
        .unwrap_or_else(|_| "unknown".to_owned())
}

fn inspect_run_id(
    paths: &WorkspacePaths,
    run_id: &str,
) -> Result<Option<InspectedRunSummary>, String> {
    let run_dir = paths.runs_dir.join(run_id);
    if !run_dir.is_dir() {
        return Ok(None);
    }
    inspect_run(paths, &run_dir).map(Some)
}

fn list_runs(paths: &WorkspacePaths) -> Result<Vec<InspectedRunSummary>, String> {
    let mut run_dirs = Vec::new();
    for entry in fs::read_dir(&paths.runs_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            run_dirs.push(path);
        }
    }
    run_dirs.sort();
    run_dirs
        .iter()
        .map(|run_dir| inspect_run(paths, run_dir))
        .collect::<Result<Vec<_>, _>>()
}

fn inspect_run(paths: &WorkspacePaths, run_dir: &Path) -> Result<InspectedRunSummary, String> {
    let run_dir = absolute_path(run_dir);
    let run_id = run_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_owned();
    let stage_results_dir = run_dir.join("stage_results");
    if !stage_results_dir.exists() {
        return incomplete_run_summary(paths, run_id, run_dir, "no stage result artifacts found");
    }

    let mut stage_result_paths = Vec::new();
    for entry in fs::read_dir(&stage_results_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("json") {
            stage_result_paths.push(path);
        }
    }
    stage_result_paths.sort();
    if stage_result_paths.is_empty() {
        return incomplete_run_summary(paths, run_id, run_dir, "no stage result artifacts found");
    }

    let mut status = "valid".to_owned();
    let mut notes = Vec::new();
    let mut stage_results = Vec::new();
    let mut malformed_stage_result_paths = Vec::new();
    for path in stage_result_paths {
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) => {
                status = "malformed".to_owned();
                malformed_stage_result_paths.push(normalize_run_relative_path(&run_dir, &path));
                notes.push(format!(
                    "{}: invalid JSON: {error}",
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                ));
                continue;
            }
        };
        let envelope = match StageResultEnvelope::from_json_str(&raw) {
            Ok(envelope) => envelope,
            Err(error) => {
                status = "malformed".to_owned();
                malformed_stage_result_paths.push(normalize_run_relative_path(&run_dir, &path));
                notes.push(format!(
                    "{}: invalid stage result payload: {error}",
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                ));
                continue;
            }
        };
        let stage_result_path = normalize_run_relative_path(&run_dir, &path);
        let stdout_path =
            normalize_optional_run_relative_path(&run_dir, envelope.stdout_path.as_deref());
        let stderr_path =
            normalize_optional_run_relative_path(&run_dir, envelope.stderr_path.as_deref());
        let report_artifact =
            normalize_optional_run_relative_path(&run_dir, envelope.report_artifact.as_deref());
        let prompt_artifact =
            normalize_optional_run_relative_path(&run_dir, envelope.prompt_artifact.as_deref());
        let artifact_paths: Vec<String> = envelope
            .artifact_paths
            .iter()
            .map(|artifact_path| {
                normalize_optional_run_relative_path(&run_dir, Some(artifact_path))
                    .unwrap_or_else(|| artifact_path.clone())
            })
            .collect();
        stage_results.push(InspectedStageResult {
            request_id: string_metadata(&envelope, "request_id"),
            compiled_plan_id: string_metadata(&envelope, "compiled_plan_id"),
            mode_id: string_metadata(&envelope, "mode_id"),
            request_kind: string_metadata(&envelope, "request_kind"),
            closure_target_root_spec_id: string_metadata(&envelope, "closure_target_root_spec_id"),
            closure_target_root_idea_id: string_metadata(&envelope, "closure_target_root_idea_id"),
            preferred_rubric_path: metadata_path(&run_dir, &envelope, "preferred_rubric_path"),
            preferred_verdict_path: metadata_path(&run_dir, &envelope, "preferred_verdict_path"),
            preferred_report_path: metadata_path(&run_dir, &envelope, "preferred_report_path"),
            skill_revision_evidence_path: metadata_path(
                &run_dir,
                &envelope,
                "skill_revision_evidence_path",
            ),
            raw_exit_kind: string_metadata(&envelope, "raw_exit_kind"),
            raw_exit_code: scalar_metadata(&envelope, "raw_exit_code"),
            failure_class: string_metadata(&envelope, "failure_class"),
            prompt_artifact,
            stage_result_path,
            stdout_path,
            stderr_path,
            event_log_path: stage_event_log_path(&artifact_paths),
            report_artifact,
            artifact_paths,
            envelope,
        });
    }

    stage_results.sort_by(|left, right| {
        (
            left.envelope.completed_at.as_str(),
            left.envelope.started_at.as_str(),
            left.stage_result_path.as_str(),
        )
            .cmp(&(
                right.envelope.completed_at.as_str(),
                right.envelope.started_at.as_str(),
                right.stage_result_path.as_str(),
            ))
    });
    if stage_results.is_empty() && status == "valid" {
        status = "incomplete".to_owned();
        notes.push("no stage result artifacts found".to_owned());
    }

    let stage_requests = inspect_stage_requests(&run_dir, &mut notes)?;
    let runner_artifacts = inspect_runner_artifacts(&run_dir)?;
    let governance_ledger_stage_result_paths =
        governance_ledger_stage_result_paths(paths, &run_id, &mut notes);
    let first = stage_results.first();
    let latest = stage_results.last();
    Ok(InspectedRunSummary {
        run_id,
        run_dir,
        status,
        compiled_plan_id: latest.and_then(|stage| stage.compiled_plan_id.clone()),
        mode_id: latest.and_then(|stage| stage.mode_id.clone()),
        request_kind: latest.and_then(|stage| stage.request_kind.clone()),
        closure_target_root_spec_id: latest
            .and_then(|stage| stage.closure_target_root_spec_id.clone()),
        work_item_kind: latest.map(|stage| stage.envelope.work_item_kind),
        work_item_id: latest.map(|stage| stage.envelope.work_item_id.clone()),
        failure_class: latest.and_then(|stage| stage.failure_class.clone()),
        troubleshoot_report_path: latest.and_then(|stage| stage.report_artifact.clone()),
        primary_prompt_artifact_path: latest
            .and_then(|stage| stage.prompt_artifact.clone())
            .or_else(|| runner_artifact_path(&runner_artifacts, "runner_prompt")),
        primary_stdout_path: latest
            .and_then(|stage| stage.stdout_path.clone())
            .or_else(|| runner_artifact_path(&runner_artifacts, "runner_stdout")),
        primary_stderr_path: latest
            .and_then(|stage| stage.stderr_path.clone())
            .or_else(|| runner_artifact_path(&runner_artifacts, "runner_stderr")),
        primary_event_log_path: latest
            .and_then(|stage| stage.event_log_path.clone())
            .or_else(|| runner_artifact_path(&runner_artifacts, "runner_events")),
        primary_runner_invocation_path: runner_artifact_path(
            &runner_artifacts,
            "runner_invocation",
        ),
        primary_runner_completion_path: runner_artifact_path(
            &runner_artifacts,
            "runner_completion",
        ),
        primary_skill_revision_evidence_path: latest
            .and_then(|stage| stage.skill_revision_evidence_path.clone())
            .or_else(|| runner_artifact_path(&runner_artifacts, "skill_revision_evidence")),
        stage_requests,
        started_at: first.map(|stage| stage.envelope.started_at.as_str().to_owned()),
        completed_at: latest.map(|stage| stage.envelope.completed_at.as_str().to_owned()),
        duration_seconds: run_duration_seconds(first, latest),
        token_usage: aggregate_token_usage(
            stage_results
                .iter()
                .map(|stage| stage.envelope.token_usage.as_ref()),
        ),
        stage_results,
        malformed_stage_result_paths,
        runner_artifacts,
        governance_ledger_stage_result_paths,
        notes,
    })
}

fn incomplete_run_summary(
    paths: &WorkspacePaths,
    run_id: String,
    run_dir: PathBuf,
    note: &str,
) -> Result<InspectedRunSummary, String> {
    let mut notes = vec![note.to_owned()];
    let runner_artifacts = inspect_runner_artifacts(&run_dir)?;
    let stage_requests = inspect_stage_requests(&run_dir, &mut notes)?;
    let governance_ledger_stage_result_paths =
        governance_ledger_stage_result_paths(paths, &run_id, &mut notes);
    Ok(InspectedRunSummary {
        run_id,
        run_dir,
        status: "incomplete".to_owned(),
        compiled_plan_id: None,
        mode_id: None,
        request_kind: None,
        closure_target_root_spec_id: None,
        work_item_kind: None,
        work_item_id: None,
        failure_class: None,
        troubleshoot_report_path: None,
        primary_prompt_artifact_path: runner_artifact_path(&runner_artifacts, "runner_prompt"),
        primary_stdout_path: runner_artifact_path(&runner_artifacts, "runner_stdout"),
        primary_stderr_path: runner_artifact_path(&runner_artifacts, "runner_stderr"),
        primary_event_log_path: runner_artifact_path(&runner_artifacts, "runner_events"),
        primary_runner_invocation_path: runner_artifact_path(
            &runner_artifacts,
            "runner_invocation",
        ),
        primary_runner_completion_path: runner_artifact_path(
            &runner_artifacts,
            "runner_completion",
        ),
        primary_skill_revision_evidence_path: runner_artifact_path(
            &runner_artifacts,
            "skill_revision_evidence",
        ),
        stage_requests,
        stage_results: Vec::new(),
        malformed_stage_result_paths: Vec::new(),
        runner_artifacts,
        governance_ledger_stage_result_paths,
        notes,
        started_at: None,
        completed_at: None,
        duration_seconds: None,
        token_usage: None,
    })
}

fn render_run_show_lines(summary: &InspectedRunSummary) -> Vec<String> {
    let mut lines = vec![
        format!("run_id: {}", summary.run_id),
        format!("status: {}", summary.status),
        format!(
            "compiled_plan_id: {}",
            option_str(summary.compiled_plan_id.as_deref())
        ),
        format!("mode_id: {}", option_str(summary.mode_id.as_deref())),
        format!(
            "request_kind: {}",
            option_str(summary.request_kind.as_deref())
        ),
        format!(
            "closure_target_root_spec_id: {}",
            option_str(summary.closure_target_root_spec_id.as_deref())
        ),
        format!("work_item_kind: {}", option_value(summary.work_item_kind)),
        format!(
            "work_item_id: {}",
            option_str(summary.work_item_id.as_deref())
        ),
        format!(
            "failure_class: {}",
            option_str(summary.failure_class.as_deref())
        ),
        format!("started_at: {}", option_str(summary.started_at.as_deref())),
        format!(
            "completed_at: {}",
            option_str(summary.completed_at.as_deref())
        ),
        format!("duration_seconds: {}", option_f64(summary.duration_seconds)),
        format!(
            "troubleshoot_report_path: {}",
            option_str(summary.troubleshoot_report_path.as_deref())
        ),
        format!(
            "primary_prompt_artifact_path: {}",
            option_str(summary.primary_prompt_artifact_path.as_deref())
        ),
        format!(
            "primary_stdout_path: {}",
            option_str(summary.primary_stdout_path.as_deref())
        ),
        format!(
            "primary_stderr_path: {}",
            option_str(summary.primary_stderr_path.as_deref())
        ),
        format!(
            "primary_event_log_path: {}",
            option_str(summary.primary_event_log_path.as_deref())
        ),
        format!(
            "primary_runner_invocation_path: {}",
            option_str(summary.primary_runner_invocation_path.as_deref())
        ),
        format!(
            "primary_runner_completion_path: {}",
            option_str(summary.primary_runner_completion_path.as_deref())
        ),
        format!(
            "primary_skill_revision_evidence_path: {}",
            option_str(summary.primary_skill_revision_evidence_path.as_deref())
        ),
        format!(
            "primary_tail_artifact: {}",
            option_str(select_primary_run_artifact(summary).as_deref())
        ),
        format!("stage_result_count: {}", summary.stage_results.len()),
        format!(
            "malformed_stage_result_count: {}",
            summary.malformed_stage_result_paths.len()
        ),
        format!("runner_artifact_count: {}", summary.runner_artifacts.len()),
        format!(
            "governance_ledger_stage_result_count: {}",
            summary.governance_ledger_stage_result_paths.len()
        ),
        format!("stage_request_count: {}", summary.stage_requests.len()),
    ];
    lines.extend(render_token_usage_lines(summary.token_usage.as_ref()));
    for path in &summary.malformed_stage_result_paths {
        lines.push(format!("malformed_stage_result_path: {path}"));
    }
    for path in &summary.governance_ledger_stage_result_paths {
        lines.push(format!("governance_ledger_stage_result_path: {path}"));
    }
    for artifact in &summary.runner_artifacts {
        lines.push(format!(
            "runner_artifact: kind={} request_id={} path={} thinking_level={} model_reasoning_effort={}",
            artifact.kind,
            option_str(artifact.request_id.as_deref()),
            artifact.path,
            option_str(artifact.thinking_level.as_deref()),
            option_str(artifact.model_reasoning_effort.as_deref())
        ));
    }
    for note in &summary.notes {
        lines.push(format!("note: {note}"));
    }
    for request in &summary.stage_requests {
        lines.extend([
            format!("stage_request_path: {}", request.stage_request_path),
            format!("stage_request_id: {}", request.request_id),
            format!("stage_request_stage: {}", request.stage),
            format!("stage_request_node_id: {}", request.node_id),
            format!("stage_request_stage_kind_id: {}", request.stage_kind_id),
            format!(
                "stage_request_runner_name: {}",
                option_str(request.runner_name.as_deref())
            ),
            format!(
                "stage_request_model_name: {}",
                option_str(request.model_name.as_deref())
            ),
            format!(
                "stage_request_thinking_level: {}",
                option_str(request.thinking_level.as_deref())
            ),
            format!(
                "stage_request_model_reasoning_effort: {}",
                option_str(request.model_reasoning_effort.as_deref())
            ),
            format!("stage_request_timeout_seconds: {}", request.timeout_seconds),
        ]);
    }
    for stage in &summary.stage_results {
        lines.extend([
            format!("stage_result_path: {}", stage.stage_result_path),
            format!("request_id: {}", option_str(stage.request_id.as_deref())),
            format!(
                "compiled_plan_id: {}",
                option_str(stage.compiled_plan_id.as_deref())
            ),
            format!("mode_id: {}", option_str(stage.mode_id.as_deref())),
            format!("stage: {}", stage.envelope.stage.as_str()),
            format!("node_id: {}", stage.envelope.node_id),
            format!("stage_kind_id: {}", stage.envelope.stage_kind_id),
            format!(
                "request_kind: {}",
                option_str(stage.request_kind.as_deref())
            ),
            format!(
                "closure_target_root_spec_id: {}",
                option_str(stage.closure_target_root_spec_id.as_deref())
            ),
            format!(
                "closure_target_root_idea_id: {}",
                option_str(stage.closure_target_root_idea_id.as_deref())
            ),
            format!(
                "preferred_rubric_path: {}",
                option_str(stage.preferred_rubric_path.as_deref())
            ),
            format!(
                "preferred_verdict_path: {}",
                option_str(stage.preferred_verdict_path.as_deref())
            ),
            format!(
                "preferred_report_path: {}",
                option_str(stage.preferred_report_path.as_deref())
            ),
            format!(
                "skill_revision_evidence_path: {}",
                option_str(stage.skill_revision_evidence_path.as_deref())
            ),
            format!(
                "raw_exit_kind: {}",
                option_str(stage.raw_exit_kind.as_deref())
            ),
            format!(
                "raw_exit_code: {}",
                option_str(stage.raw_exit_code.as_deref())
            ),
            format!(
                "terminal_result: {}",
                stage.envelope.terminal_result.as_str()
            ),
            format!("result_class: {}", stage.envelope.result_class.as_str()),
            format!(
                "runner_name: {}",
                option_str(stage.envelope.runner_name.as_deref())
            ),
            format!(
                "model_name: {}",
                option_str(stage.envelope.model_name.as_deref())
            ),
            format!(
                "thinking_level: {}",
                option_str(stage.envelope.thinking_level.as_deref())
            ),
            format!(
                "model_reasoning_effort: {}",
                option_str(stage.envelope.model_reasoning_effort.as_deref())
            ),
            format!("started_at: {}", stage.envelope.started_at.as_str()),
            format!("completed_at: {}", stage.envelope.completed_at.as_str()),
            format!("duration_seconds: {}", stage.envelope.duration_seconds),
            format!(
                "prompt_artifact: {}",
                option_str(stage.prompt_artifact.as_deref())
            ),
            format!("stdout_path: {}", option_str(stage.stdout_path.as_deref())),
            format!("stderr_path: {}", option_str(stage.stderr_path.as_deref())),
            format!(
                "event_log_path: {}",
                option_str(stage.event_log_path.as_deref())
            ),
            format!(
                "report_artifact: {}",
                option_str(stage.report_artifact.as_deref())
            ),
        ]);
        lines.extend(render_token_usage_lines(
            stage.envelope.token_usage.as_ref(),
        ));
        for reference_path in remediation_reference_paths(stage) {
            lines.push(format!("remediation_reference_path: {reference_path}"));
        }
        for artifact_path in &stage.artifact_paths {
            lines.push(format!("artifact_path: {artifact_path}"));
        }
    }
    lines
}

fn render_token_usage_lines(token_usage: Option<&TokenUsage>) -> Vec<String> {
    let Some(token_usage) = token_usage else {
        return Vec::new();
    };
    vec![
        format!("input_tokens: {}", token_usage.input_tokens),
        format!("cached_input_tokens: {}", token_usage.cached_input_tokens),
        format!("output_tokens: {}", token_usage.output_tokens),
        format!("thinking_tokens: {}", token_usage.thinking_tokens),
        format!("total_tokens: {}", token_usage.total_tokens),
    ]
}

fn remediation_reference_paths(stage: &InspectedStageResult) -> Vec<&str> {
    [
        stage.preferred_rubric_path.as_deref(),
        stage.preferred_verdict_path.as_deref(),
        stage.preferred_report_path.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn select_primary_run_artifact(summary: &InspectedRunSummary) -> Option<String> {
    summary
        .troubleshoot_report_path
        .clone()
        .or_else(|| summary.primary_stdout_path.clone())
        .or_else(|| summary.primary_stderr_path.clone())
        .or_else(|| summary.primary_event_log_path.clone())
        .or_else(|| {
            summary
                .stage_results
                .last()
                .map(|stage| stage.stage_result_path.clone())
        })
}

fn resolve_run_artifact_path(run_dir: &Path, candidate: &str) -> PathBuf {
    let path = PathBuf::from(candidate);
    if path.is_absolute() {
        path
    } else {
        run_dir.join(path)
    }
}

fn inspect_runner_artifacts(run_dir: &Path) -> Result<Vec<InspectedRunnerArtifact>, String> {
    if !run_dir.exists() {
        return Ok(Vec::new());
    }
    let mut artifacts = Vec::new();
    for entry in fs::read_dir(run_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(kind) = runner_artifact_kind(file_name) else {
            continue;
        };
        let (thinking_level, model_reasoning_effort) =
            runner_artifact_thinking_fields(kind, &path).unwrap_or((None, None));
        artifacts.push(InspectedRunnerArtifact {
            kind: kind.to_owned(),
            path: normalize_run_relative_path(run_dir, &path),
            request_id: runner_artifact_request_id(kind, file_name),
            thinking_level,
            model_reasoning_effort,
        });
    }
    artifacts.sort_by(|left, right| {
        (left.kind.as_str(), left.path.as_str()).cmp(&(right.kind.as_str(), right.path.as_str()))
    });
    Ok(artifacts)
}

fn inspect_stage_requests(
    run_dir: &Path,
    notes: &mut Vec<String>,
) -> Result<Vec<InspectedStageRequest>, String> {
    let stage_requests_dir = run_dir.join("stage_requests");
    if !stage_requests_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(&stage_requests_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();

    let mut requests = Vec::new();
    for path in paths {
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) => {
                notes.push(format!(
                    "{}: invalid stage request JSON: {error}",
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                ));
                continue;
            }
        };
        let request = match StageRunRequest::from_json_str(&raw) {
            Ok(request) => request,
            Err(error) => {
                notes.push(format!(
                    "{}: invalid stage request payload: {error}",
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                ));
                continue;
            }
        };
        requests.push(InspectedStageRequest {
            stage_request_path: normalize_run_relative_path(run_dir, &path),
            request_id: request.request_id,
            stage: request.stage.as_str().to_owned(),
            node_id: request.node_id,
            stage_kind_id: request.stage_kind_id,
            runner_name: request.runner_name,
            model_name: request.model_name,
            thinking_level: request.thinking_level,
            model_reasoning_effort: request.model_reasoning_effort,
            timeout_seconds: request.timeout_seconds,
        });
    }
    Ok(requests)
}

fn runner_artifact_thinking_fields(
    kind: &str,
    path: &Path,
) -> Result<(Option<String>, Option<String>), String> {
    if !matches!(kind, "runner_invocation" | "runner_completion") {
        return Ok((None, None));
    }
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let payload: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    Ok((
        payload
            .get("thinking_level")
            .and_then(Value::as_str)
            .map(str::to_owned),
        payload
            .get("model_reasoning_effort")
            .and_then(Value::as_str)
            .map(str::to_owned),
    ))
}

fn runner_artifact_path(artifacts: &[InspectedRunnerArtifact], kind: &str) -> Option<String> {
    artifacts
        .iter()
        .find(|artifact| artifact.kind == kind)
        .map(|artifact| artifact.path.clone())
}

fn runner_artifact_kind(file_name: &str) -> Option<&'static str> {
    [
        ("runner_prompt.", "runner_prompt"),
        ("runner_stdout.", "runner_stdout"),
        ("runner_stderr.", "runner_stderr"),
        ("runner_events.", "runner_events"),
        ("runner_last_message.", "runner_last_message"),
        ("runner_terminal_result.", "runner_terminal_result"),
        ("runner_invocation.", "runner_invocation"),
        ("runner_completion.", "runner_completion"),
        ("skill_revision_evidence.", "skill_revision_evidence"),
    ]
    .into_iter()
    .find_map(|(prefix, kind)| file_name.starts_with(prefix).then_some(kind))
}

fn runner_artifact_request_id(kind: &str, file_name: &str) -> Option<String> {
    let (prefix, suffixes): (&str, &[&str]) = match kind {
        "runner_prompt" => ("runner_prompt.", &[".md"]),
        "runner_stdout" => ("runner_stdout.", &[".txt"]),
        "runner_stderr" => ("runner_stderr.", &[".txt"]),
        "runner_events" => ("runner_events.", &[".jsonl"]),
        "runner_last_message" => ("runner_last_message.", &[".txt"]),
        "runner_terminal_result" => ("runner_terminal_result.", &[".json"]),
        "runner_invocation" => ("runner_invocation.", &[".json"]),
        "runner_completion" => ("runner_completion.", &[".json"]),
        "skill_revision_evidence" => ("skill_revision_evidence.", &[".json"]),
        _ => return None,
    };
    let request_id = file_name.strip_prefix(prefix)?;
    for suffix in suffixes {
        if let Some(stripped) = request_id.strip_suffix(suffix) {
            return Some(stripped.to_owned());
        }
    }
    Some(request_id.to_owned())
}

fn stage_event_log_path(artifact_paths: &[String]) -> Option<String> {
    artifact_paths
        .iter()
        .find(|artifact_path| {
            artifact_path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with("runner_events."))
        })
        .cloned()
}

fn metadata_path(run_dir: &Path, stage_result: &StageResultEnvelope, key: &str) -> Option<String> {
    string_metadata(stage_result, key)
        .and_then(|path| normalize_optional_run_relative_path(run_dir, Some(&path)))
}

fn scalar_metadata(stage_result: &StageResultEnvelope, key: &str) -> Option<String> {
    let value = stage_result.metadata.get(key)?;
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(bool_text(*value).to_owned()),
        _ => None,
    }
}

fn governance_ledger_stage_result_paths(
    paths: &WorkspacePaths,
    run_id: &str,
    notes: &mut Vec<String>,
) -> Vec<String> {
    let entries = match load_usage_governance_ledger(paths) {
        Ok(entries) => entries,
        Err(error) => {
            notes.push(format!("usage governance ledger unavailable: {error}"));
            return Vec::new();
        }
    };
    let mut stage_result_paths = entries
        .into_iter()
        .filter(|entry| entry.run_id == run_id)
        .map(|entry| entry.stage_result_path)
        .collect::<Vec<_>>();
    stage_result_paths.sort();
    stage_result_paths.dedup();
    stage_result_paths
}

fn embedded_modes() -> Result<Vec<ModeDefinition>, String> {
    assets::runtime_assets()
        .iter()
        .filter(|asset| asset.relative_path.starts_with("modes/"))
        .filter(|asset| asset.relative_path.ends_with(".json"))
        .map(|asset| {
            std::str::from_utf8(asset.contents)
                .map_err(|error| error.to_string())
                .and_then(|raw| {
                    ModeDefinition::from_json_str(raw).map_err(|error| error.to_string())
                })
        })
        .collect()
}

fn render_queue_entry(entry: &QueueInspectionEntry) -> String {
    format!(
        "work_item: kind={} state={} id={} path={}",
        entry.work_item_kind.as_str(),
        entry.work_item_state,
        entry.work_item_id,
        entry.path.display()
    )
}

fn render_queue_document_fields(document: &WorkDocument) -> Vec<String> {
    let mut lines = Vec::new();
    match document {
        WorkDocument::Task(document) => {
            push_non_empty_field(&mut lines, "summary", &document.summary);
            push_optional_field(&mut lines, "root_idea_id", document.root_idea_id.as_deref());
            push_optional_field(&mut lines, "root_spec_id", document.root_spec_id.as_deref());
            if let Some(root_intake_kind) = document.root_intake_kind {
                lines.push(format!("root_intake_kind: {}", root_intake_kind.as_str()));
            }
            push_optional_field(
                &mut lines,
                "root_intake_id",
                document.root_intake_id.as_deref(),
            );
            push_optional_field(&mut lines, "spec_id", document.spec_id.as_deref());
            push_optional_field(
                &mut lines,
                "parent_task_id",
                document.parent_task_id.as_deref(),
            );
            push_optional_field(&mut lines, "incident_id", document.incident_id.as_deref());
            if let Some(status_hint) = document.status_hint {
                lines.push(format!("status_hint: {}", status_hint.as_str()));
            }
            lines.push(format!("created_at: {}", document.created_at.as_str()));
            lines.push(format!("created_by: {}", document.created_by));
            push_optional_field(
                &mut lines,
                "updated_at",
                document.updated_at.as_ref().map(|value| value.as_str()),
            );
            push_list_field(&mut lines, "depends_on", &document.depends_on);
            push_list_field(&mut lines, "blocks", &document.blocks);
            push_list_field(&mut lines, "tags", &document.tags);
            push_list_field(&mut lines, "target_paths", &document.target_paths);
            push_list_field(&mut lines, "acceptance", &document.acceptance);
            push_list_field(&mut lines, "required_checks", &document.required_checks);
            push_list_field(&mut lines, "references", &document.references);
            push_list_field(&mut lines, "risk", &document.risk);
        }
        WorkDocument::Probe(document) => {
            push_non_empty_field(&mut lines, "summary", &document.summary);
            lines.push(format!("request: {}", document.request));
            if let Some(status_hint) = document.status_hint {
                lines.push(format!("status_hint: {}", status_hint.as_str()));
            }
            lines.push(format!("created_at: {}", document.created_at.as_str()));
            lines.push(format!("created_by: {}", document.created_by));
            push_optional_field(
                &mut lines,
                "updated_at",
                document.updated_at.as_ref().map(|value| value.as_str()),
            );
            push_list_field(&mut lines, "target_paths", &document.target_paths);
            push_list_field(&mut lines, "constraints", &document.constraints);
            push_list_field(&mut lines, "acceptance", &document.acceptance);
            push_list_field(&mut lines, "risk_notes", &document.risk_notes);
            push_list_field(&mut lines, "references", &document.references);
            push_list_field(&mut lines, "tags", &document.tags);
        }
        WorkDocument::Spec(document) => {
            lines.push(format!("summary: {}", document.summary));
            lines.push(format!("source_type: {}", document.source_type.as_str()));
            push_optional_field(&mut lines, "source_id", document.source_id.as_deref());
            push_optional_field(
                &mut lines,
                "parent_spec_id",
                document.parent_spec_id.as_deref(),
            );
            push_optional_field(&mut lines, "root_idea_id", document.root_idea_id.as_deref());
            push_optional_field(&mut lines, "root_spec_id", document.root_spec_id.as_deref());
            if let Some(root_intake_kind) = document.root_intake_kind {
                lines.push(format!("root_intake_kind: {}", root_intake_kind.as_str()));
            }
            push_optional_field(
                &mut lines,
                "root_intake_id",
                document.root_intake_id.as_deref(),
            );
            lines.push(format!("created_at: {}", document.created_at.as_str()));
            lines.push(format!("created_by: {}", document.created_by));
            push_optional_field(
                &mut lines,
                "updated_at",
                document.updated_at.as_ref().map(|value| value.as_str()),
            );
            push_list_field(&mut lines, "goals", &document.goals);
            push_list_field(&mut lines, "non_goals", &document.non_goals);
            push_list_field(&mut lines, "scope", &document.scope);
            push_list_field(&mut lines, "constraints", &document.constraints);
            push_list_field(&mut lines, "assumptions", &document.assumptions);
            push_list_field(&mut lines, "risks", &document.risks);
            push_list_field(&mut lines, "target_paths", &document.target_paths);
            push_list_field(&mut lines, "entrypoints", &document.entrypoints);
            push_list_field(&mut lines, "required_skills", &document.required_skills);
            push_list_field(
                &mut lines,
                "decomposition_hints",
                &document.decomposition_hints,
            );
            push_list_field(&mut lines, "acceptance", &document.acceptance);
            push_list_field(&mut lines, "references", &document.references);
        }
        WorkDocument::Incident(document) => {
            lines.push(format!("summary: {}", document.summary));
            push_optional_field(&mut lines, "root_idea_id", document.root_idea_id.as_deref());
            push_optional_field(&mut lines, "root_spec_id", document.root_spec_id.as_deref());
            if let Some(root_intake_kind) = document.root_intake_kind {
                lines.push(format!("root_intake_kind: {}", root_intake_kind.as_str()));
            }
            push_optional_field(
                &mut lines,
                "root_intake_id",
                document.root_intake_id.as_deref(),
            );
            push_optional_field(
                &mut lines,
                "source_task_id",
                document.source_task_id.as_deref(),
            );
            push_optional_field(
                &mut lines,
                "source_spec_id",
                document.source_spec_id.as_deref(),
            );
            lines.push(format!("source_stage: {}", document.source_stage.as_str()));
            lines.push(format!("source_plane: {}", document.source_plane.as_str()));
            lines.push(format!("failure_class: {}", document.failure_class));
            lines.push(format!("severity: {}", document.severity.as_str()));
            lines.push(format!(
                "needs_planning: {}",
                bool_text(document.needs_planning)
            ));
            lines.push(format!("trigger_reason: {}", document.trigger_reason));
            lines.push(format!(
                "consultant_decision: {}",
                document.consultant_decision.as_str()
            ));
            lines.push(format!("opened_at: {}", document.opened_at.as_str()));
            lines.push(format!("opened_by: {}", document.opened_by));
            push_optional_field(
                &mut lines,
                "updated_at",
                document.updated_at.as_ref().map(|value| value.as_str()),
            );
            push_list_field(&mut lines, "observed_symptoms", &document.observed_symptoms);
            push_list_field(&mut lines, "failed_attempts", &document.failed_attempts);
            push_list_field(&mut lines, "evidence_paths", &document.evidence_paths);
            push_list_field(&mut lines, "related_run_ids", &document.related_run_ids);
            push_list_field(
                &mut lines,
                "related_stage_results",
                &document.related_stage_results,
            );
            push_list_field(&mut lines, "references", &document.references);
        }
        WorkDocument::LearningRequest(document) => {
            push_non_empty_field(&mut lines, "summary", &document.summary);
            lines.push(format!(
                "requested_action: {}",
                document.requested_action.as_str()
            ));
            push_optional_field(
                &mut lines,
                "target_skill_id",
                document.target_skill_id.as_deref(),
            );
            if let Some(target_stage) = document.target_stage {
                lines.push(format!("target_stage: {}", target_stage.as_str()));
            }
            lines.push(format!("trigger_metadata: {}", document.trigger_metadata));
            lines.push(format!("created_at: {}", document.created_at.as_str()));
            lines.push(format!("created_by: {}", document.created_by));
            push_optional_field(
                &mut lines,
                "updated_at",
                document.updated_at.as_ref().map(|value| value.as_str()),
            );
            push_list_field(&mut lines, "source_refs", &document.source_refs);
            push_list_field(
                &mut lines,
                "preferred_output_paths",
                &document.preferred_output_paths,
            );
            push_list_field(
                &mut lines,
                "originating_run_ids",
                &document.originating_run_ids,
            );
            push_list_field(&mut lines, "artifact_paths", &document.artifact_paths);
            push_list_field(&mut lines, "references", &document.references);
        }
    }
    lines
}

fn push_non_empty_field(lines: &mut Vec<String>, name: &str, value: &str) {
    if !value.is_empty() {
        lines.push(format!("{name}: {value}"));
    }
}

fn push_optional_field(lines: &mut Vec<String>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        lines.push(format!("{name}: {value}"));
    }
}

fn push_list_field(lines: &mut Vec<String>, name: &str, values: &[String]) {
    let rendered = serde_json::to_string(values).unwrap_or_else(|_| "[]".to_owned());
    lines.push(format!("{name}: {rendered}"));
}

fn count_kind_state(counts: &BTreeMap<(String, String), usize>, kind: &str, state: &str) -> usize {
    counts
        .get(&(kind.to_owned(), state.to_owned()))
        .copied()
        .unwrap_or(0)
}

fn count_markdown_files(directory: &Path) -> Result<usize, String> {
    if !directory.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("md") {
            count += 1;
        }
    }
    Ok(count)
}

fn json_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn parse_positive_usize(name: &str, value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{name} must be a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{name} must be a positive integer"));
    }
    Ok(parsed)
}

fn parse_non_negative_seconds(name: &str, value: &str) -> Result<f64, String> {
    let parsed = value
        .parse::<f64>()
        .map_err(|_| format!("{name} must be a non-negative number"))?;
    if parsed < 0.0 {
        return Err(format!("{name} must be a non-negative number"));
    }
    Ok(parsed)
}

fn pause_sources_label(snapshot: &RuntimeSnapshot) -> String {
    if snapshot.pause_sources.is_empty() {
        return "none".to_owned();
    }
    snapshot
        .pause_sources
        .iter()
        .map(|source| source.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn short_run_handle(run_id: &str) -> String {
    let Some(suffix) = run_id.strip_prefix("run-") else {
        return run_id.to_owned();
    };
    if suffix.len() >= 12 && suffix.chars().all(|ch| ch.is_ascii_hexdigit()) {
        suffix[..12].to_owned()
    } else {
        run_id.to_owned()
    }
}

fn string_metadata(stage_result: &StageResultEnvelope, key: &str) -> Option<String> {
    stage_result
        .metadata
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn normalize_optional_run_relative_path(
    run_dir: &Path,
    path_value: Option<&str>,
) -> Option<String> {
    path_value.map(|path| normalize_run_relative_path(run_dir, Path::new(path)))
}

fn normalize_run_relative_path(run_dir: &Path, path_value: impl AsRef<Path>) -> String {
    let original = path_value.as_ref();
    let candidate = if original.is_absolute() {
        original.to_path_buf()
    } else {
        run_dir.join(original)
    };
    let resolved = candidate.canonicalize().unwrap_or(candidate);
    resolved
        .strip_prefix(run_dir)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| original.to_string_lossy().replace('\\', "/"))
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

fn run_duration_seconds(
    first: Option<&InspectedStageResult>,
    latest: Option<&InspectedStageResult>,
) -> Option<f64> {
    let first = first?;
    let latest = latest?;
    let started = OffsetDateTime::parse(first.envelope.started_at.as_str(), &Rfc3339).ok()?;
    let completed = OffsetDateTime::parse(latest.envelope.completed_at.as_str(), &Rfc3339).ok()?;
    Some((completed - started).as_seconds_f64())
}

fn aggregate_token_usage<'a>(
    usages: impl IntoIterator<Item = Option<&'a TokenUsage>>,
) -> Option<TokenUsage> {
    let mut total = TokenUsage {
        input_tokens: 0,
        cached_input_tokens: 0,
        output_tokens: 0,
        thinking_tokens: 0,
        total_tokens: 0,
    };
    let mut found = false;
    for usage in usages.into_iter().flatten() {
        found = true;
        total.input_tokens += usage.input_tokens;
        total.cached_input_tokens += usage.cached_input_tokens;
        total.output_tokens += usage.output_tokens;
        total.thinking_tokens += usage.thinking_tokens;
        total.total_tokens += usage.total_tokens;
    }
    found.then_some(total)
}

fn string_vec_text(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_owned())
}

fn string_map_text(values: &BTreeMap<String, String>) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "{}".to_owned())
}

fn permission_map_text(values: &BTreeMap<String, crate::runners::CodexPermissionLevel>) -> String {
    let rendered = values
        .iter()
        .map(|(key, value)| (key.clone(), value.as_str().to_owned()))
        .collect::<BTreeMap<_, _>>();
    serde_json::to_string(&rendered).unwrap_or_else(|_| "{}".to_owned())
}

fn option_value<T: Copy + ToString>(value: Option<T>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_owned())
}

fn option_str(value: Option<&str>) -> String {
    value.unwrap_or("none").to_owned()
}

fn option_f64(value: Option<f64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_owned())
}

fn format_seconds(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}
