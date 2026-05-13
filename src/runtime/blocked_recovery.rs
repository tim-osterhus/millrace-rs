//! Blocked work-item recovery metadata helpers.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use crate::{
    contracts::{
        AutoRecoveryPreRecoverySnapshot, BlockedDependencyAutoRecoveryDiagnostic,
        BlockedItemMetadata, BlockedOrigin, BlockedTaskRequeueResult, FailureClassifierCode,
        FailureScope, RuntimeJsonContract, RuntimeSnapshot, StageResultEnvelope,
        StrandedBlockedDependency, TaskDocument, Timestamp, WorkItemKind,
        failure_class_allows_auto_requeue,
    },
    work_documents::parse_task_document_with_source,
    workspace::{QueueStore, WorkspacePaths, atomic_write_text, load_snapshot, save_snapshot},
};
use serde_json::{Map, Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{AutoRecoveryConfig, RouterDecision, RuntimeTickError, RuntimeTickResult};

const DEFAULT_MAX_AUTO_REQUEUES_PER_WORK_ITEM: u64 = 3;
const AUTO_RECOVERY_ACTOR: &str = "runtime-daemon";
const AUTO_RECOVERY_REASON: &str = "transient blocked dependency auto-recovery";

/// Persisted metadata plus destination path for one blocked item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedItemMetadataRecord {
    /// Filesystem path where the metadata was written.
    pub path: PathBuf,
    /// Metadata payload written to disk.
    pub metadata: BlockedItemMetadata,
}

/// Inputs for moving one blocked task back to queue after retry guards pass.
#[derive(Debug, Clone, Copy)]
pub struct RetryBlockedTaskRequest<'a> {
    /// Blocked task identifier to requeue.
    pub task_id: &'a str,
    /// Human-readable reason recorded in the transition audit log.
    pub reason: &'a str,
    /// Actor recorded in the transition audit log.
    pub actor: &'a str,
    /// Whether the requeue was performed by runtime auto-recovery.
    pub auto: bool,
    /// Whether operator force bypasses retryability and budget checks.
    pub force: bool,
    /// Optional root-spec guard for lineage-safe requeue.
    pub root_spec_id: Option<&'a str>,
    /// Optional diagnostics artifact linked from the requeue result.
    pub diagnostics_path: Option<&'a Path>,
    /// Optional retry budget override. The Python-compatible default is used
    /// when absent.
    pub max_auto_requeues_per_work_item: Option<u64>,
}

/// Returns the canonical blocked metadata path for one work item.
#[must_use]
pub fn blocked_metadata_path(
    paths: &WorkspacePaths,
    kind: WorkItemKind,
    work_item_id: &str,
) -> PathBuf {
    paths
        .runtime_root
        .join("diagnostics")
        .join("blocked")
        .join(format!("{}-{work_item_id}.json", kind.as_str()))
}

/// Returns the canonical blocked metadata path for one task.
#[must_use]
pub fn blocked_task_metadata_path(paths: &WorkspacePaths, task_id: &str) -> PathBuf {
    blocked_metadata_path(paths, WorkItemKind::Task, task_id)
}

/// Load blocked metadata, returning `None` for missing or malformed diagnostics.
pub fn load_blocked_item_metadata(
    paths: &WorkspacePaths,
    kind: WorkItemKind,
    work_item_id: &str,
) -> RuntimeTickResult<Option<BlockedItemMetadata>> {
    let path = blocked_metadata_path(paths, kind, work_item_id);
    load_blocked_item_metadata_at(&path)
}

/// Load blocked task metadata, returning `None` for missing or malformed diagnostics.
pub fn load_blocked_task_metadata(
    paths: &WorkspacePaths,
    task_id: &str,
) -> RuntimeTickResult<Option<BlockedItemMetadata>> {
    load_blocked_item_metadata(paths, WorkItemKind::Task, task_id)
}

/// Returns true when metadata allows an automatic requeue.
#[must_use]
pub fn blocked_metadata_allows_auto_requeue(metadata: Option<&BlockedItemMetadata>) -> bool {
    metadata.is_some_and(BlockedItemMetadata::allows_auto_requeue)
}

/// Move one blocked task back to queue after applying the v0.18.4 retry guards.
pub fn retry_blocked_task(
    paths: &WorkspacePaths,
    request: RetryBlockedTaskRequest<'_>,
) -> RuntimeTickResult<BlockedTaskRequeueResult> {
    let cleaned_reason = request.reason.trim();
    if cleaned_reason.is_empty() {
        return Err(invalid_state("requeue reason is required"));
    }
    let task_id = request.task_id;
    let source_path = paths.tasks_blocked_dir.join(format!("{task_id}.md"));
    let destination_path = paths.tasks_queue_dir.join(format!("{task_id}.md"));
    validate_blocked_task_locations(paths, task_id)?;

    let task = read_task_document(&source_path)?;
    let task_root_spec_id = effective_root_spec_id(&task);
    if let Some(root_spec_id) = request.root_spec_id
        && task_root_spec_id != Some(root_spec_id)
    {
        return Err(invalid_state(format!(
            "blocked task {task_id} does not belong to root spec {root_spec_id}"
        )));
    }

    let metadata = load_blocked_task_metadata(paths, task_id)?;
    if !request.force && !blocked_metadata_allows_auto_requeue(metadata.as_ref()) {
        return Err(invalid_state(
            "blocked task is not retryable; rerun with --force to override",
        ));
    }

    let auto_attempts = count_auto_requeues(paths, task_id)?;
    let max_auto_requeues_per_work_item = request
        .max_auto_requeues_per_work_item
        .unwrap_or(DEFAULT_MAX_AUTO_REQUEUES_PER_WORK_ITEM);
    if !request.force && auto_attempts >= max_auto_requeues_per_work_item {
        return Err(invalid_state("blocked task retry budget is exhausted"));
    }

    let attempt_number = auto_attempts + 1;
    let failure_class = metadata
        .as_ref()
        .map(|metadata| metadata.failure_class.clone());
    QueueStore::from_paths(paths.clone()).requeue_blocked_task(
        task_id,
        cleaned_reason,
        request.actor,
        request.auto,
        failure_class.as_deref(),
        Some(attempt_number),
    )?;
    refresh_snapshot_queue_depths(paths);

    let mut result = BlockedTaskRequeueResult {
        task_id: task_id.to_owned(),
        source_path: source_path.display().to_string(),
        destination_path: destination_path.display().to_string(),
        source_state: "blocked".to_owned(),
        destination_state: "queue".to_owned(),
        actor: request.actor.trim().to_owned(),
        auto: request.auto,
        reason: cleaned_reason.to_owned(),
        failure_class,
        attempt_number,
        diagnostics_path: request
            .diagnostics_path
            .map(|path| path.display().to_string()),
    };
    result
        .validate_contract()
        .map_err(|error| RuntimeTickError::InvalidState {
            message: error.to_string(),
        })?;
    write_blocked_task_requeued_event(paths, &result)?;
    Ok(result)
}

/// Find the first blocked task that strands queued execution dependents.
pub fn find_stranded_blocked_dependency(
    paths: &WorkspacePaths,
) -> RuntimeTickResult<Option<StrandedBlockedDependency>> {
    let completed: BTreeSet<String> = markdown_stems(&paths.tasks_done_dir)?;
    let mut dependents_by_blocked_id: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut root_by_blocked_id: BTreeMap<String, Option<String>> = BTreeMap::new();

    for queued_path in markdown_files(&paths.tasks_queue_dir)? {
        let Some(queued) = read_task_document_or_none(&queued_path) else {
            continue;
        };
        let missing_dependencies: Vec<_> = queued
            .depends_on
            .iter()
            .filter(|dependency| !completed.contains(*dependency))
            .cloned()
            .collect();
        if missing_dependencies.is_empty() {
            continue;
        }
        let queued_root = effective_root_spec_id(&queued).map(str::to_owned);
        for dependency in missing_dependencies {
            let blocked_path = paths.tasks_blocked_dir.join(format!("{dependency}.md"));
            if !blocked_path.is_file() {
                continue;
            }
            let Some(blocked) = read_task_document_or_none(&blocked_path) else {
                continue;
            };
            let blocked_root = effective_root_spec_id(&blocked).map(str::to_owned);
            if queued_root.is_some()
                && blocked_root.is_some()
                && queued_root.as_deref() != blocked_root.as_deref()
            {
                continue;
            }
            dependents_by_blocked_id
                .entry(dependency.clone())
                .or_default()
                .insert(queued.task_id.clone());
            root_by_blocked_id.insert(dependency, blocked_root.or_else(|| queued_root.clone()));
        }
    }

    let Some((blocked_task_id, dependent_ids)) = dependents_by_blocked_id.into_iter().next() else {
        return Ok(None);
    };
    let mut dependency = StrandedBlockedDependency {
        root_spec_id: root_by_blocked_id.remove(&blocked_task_id).flatten(),
        metadata: load_blocked_task_metadata(paths, &blocked_task_id)?,
        blocked_task_id,
        queued_dependent_ids: dependent_ids.into_iter().collect(),
    };
    dependency
        .validate_contract()
        .map_err(|error| RuntimeTickError::InvalidState {
            message: error.to_string(),
        })?;
    Ok(Some(dependency))
}

/// Attempt one daemon idle-cycle blocked-dependency auto-recovery.
pub(crate) fn attempt_stranded_dependency_auto_recovery(
    paths: &WorkspacePaths,
    config: &AutoRecoveryConfig,
    snapshot: &mut RuntimeSnapshot,
    now: &Timestamp,
) -> RuntimeTickResult<Option<BlockedTaskRequeueResult>> {
    if !config.enabled
        || !config.blocked_dependency_retry_enabled
        || snapshot.paused
        || snapshot.stop_requested
        || !snapshot.active_runs_by_plane.is_empty()
    {
        return Ok(None);
    }
    if snapshot.queue_depth_execution == 0 && count_markdown_files(&paths.tasks_queue_dir) == 0 {
        return Ok(None);
    }

    let Some(candidate) = find_auto_recovery_candidate(paths)? else {
        return Ok(None);
    };
    if !candidate.lineage_compatible {
        write_auto_recovery_skip(paths, snapshot, now, &candidate, "root_spec_mismatch", 1)?;
        return Ok(None);
    }

    let metadata = match candidate.dependency.metadata.as_ref() {
        Some(metadata) if metadata.allows_auto_requeue() => metadata,
        Some(_) => {
            write_auto_recovery_skip(
                paths,
                snapshot,
                now,
                &candidate,
                "blocked_dependency_not_retryable",
                1,
            )?;
            return Ok(None);
        }
        None => {
            write_auto_recovery_skip(
                paths,
                snapshot,
                now,
                &candidate,
                "missing_or_invalid_metadata",
                1,
            )?;
            return Ok(None);
        }
    };

    let auto_attempts = count_auto_requeues(paths, &candidate.dependency.blocked_task_id)?;
    let auto_attempt_number = auto_attempts + 1;
    if auto_attempts >= config.max_auto_requeues_per_work_item {
        write_auto_recovery_skip(
            paths,
            snapshot,
            now,
            &candidate,
            "retry_budget_exhausted",
            auto_attempt_number,
        )?;
        return Ok(None);
    }

    if auto_recovery_cooldown_active(config, metadata, auto_attempts, now)? {
        write_auto_recovery_skip(
            paths,
            snapshot,
            now,
            &candidate,
            "cooldown_active",
            auto_attempt_number,
        )?;
        return Ok(None);
    }

    let diagnostics_path = write_auto_recovery_diagnostics(
        paths,
        snapshot,
        now,
        &candidate.dependency,
        "requeue",
        "transient blocked dependency",
        auto_attempt_number,
    )?;
    let result = retry_blocked_task(
        paths,
        RetryBlockedTaskRequest {
            task_id: &candidate.dependency.blocked_task_id,
            reason: AUTO_RECOVERY_REASON,
            actor: AUTO_RECOVERY_ACTOR,
            auto: true,
            force: false,
            root_spec_id: candidate.blocked_root_spec_id.as_deref(),
            diagnostics_path: Some(&diagnostics_path),
            max_auto_requeues_per_work_item: Some(config.max_auto_requeues_per_work_item),
        },
    )?;
    refresh_snapshot_queue_depths_in_place(paths, snapshot, now)?;
    write_blocked_dependency_auto_requeued_event(
        paths,
        now,
        &candidate.dependency,
        metadata.failure_class.as_str(),
        &diagnostics_path,
        result.attempt_number,
    )?;
    Ok(Some(result))
}

#[derive(Debug, Clone)]
struct AutoRecoveryCandidate {
    dependency: StrandedBlockedDependency,
    blocked_root_spec_id: Option<String>,
    lineage_compatible: bool,
}

fn find_auto_recovery_candidate(
    paths: &WorkspacePaths,
) -> RuntimeTickResult<Option<AutoRecoveryCandidate>> {
    let completed: BTreeSet<String> = markdown_stems(&paths.tasks_done_dir)?;
    let mut compatible: BTreeMap<String, CandidateParts> = BTreeMap::new();
    let mut incompatible: BTreeMap<String, CandidateParts> = BTreeMap::new();

    for queued_path in markdown_files(&paths.tasks_queue_dir)? {
        let Some(queued) = read_task_document_or_none(&queued_path) else {
            continue;
        };
        let queued_root = effective_root_spec_id(&queued).map(str::to_owned);
        for dependency_id in queued
            .depends_on
            .iter()
            .filter(|dependency| !completed.contains(*dependency))
        {
            let blocked_path = paths.tasks_blocked_dir.join(format!("{dependency_id}.md"));
            if !blocked_path.is_file() {
                continue;
            }
            let Some(blocked) = read_task_document_or_none(&blocked_path) else {
                continue;
            };
            let blocked_root = effective_root_spec_id(&blocked).map(str::to_owned);
            let root_mismatch = queued_root.is_some()
                && blocked_root.is_some()
                && queued_root.as_deref() != blocked_root.as_deref();
            let target = if root_mismatch {
                &mut incompatible
            } else {
                &mut compatible
            };
            target
                .entry(dependency_id.to_owned())
                .or_insert_with(|| CandidateParts {
                    blocked_root_spec_id: blocked_root.clone(),
                    root_spec_id: blocked_root.clone().or_else(|| queued_root.clone()),
                    queued_dependent_ids: BTreeSet::new(),
                })
                .queued_dependent_ids
                .insert(queued.task_id.clone());
        }
    }

    if let Some(candidate) = build_auto_recovery_candidate(paths, compatible, true).transpose()? {
        return Ok(Some(candidate));
    }
    build_auto_recovery_candidate(paths, incompatible, false).transpose()
}

#[derive(Debug, Clone)]
struct CandidateParts {
    blocked_root_spec_id: Option<String>,
    root_spec_id: Option<String>,
    queued_dependent_ids: BTreeSet<String>,
}

fn build_auto_recovery_candidate(
    paths: &WorkspacePaths,
    mut candidates: BTreeMap<String, CandidateParts>,
    lineage_compatible: bool,
) -> Option<RuntimeTickResult<AutoRecoveryCandidate>> {
    let (blocked_task_id, parts) = candidates.pop_first()?;
    Some((|| {
        let mut dependency = StrandedBlockedDependency {
            metadata: load_blocked_task_metadata(paths, &blocked_task_id)?,
            blocked_task_id,
            queued_dependent_ids: parts.queued_dependent_ids.into_iter().collect(),
            root_spec_id: parts.root_spec_id,
        };
        dependency
            .validate_contract()
            .map_err(|error| RuntimeTickError::InvalidState {
                message: error.to_string(),
            })?;
        Ok(AutoRecoveryCandidate {
            dependency,
            blocked_root_spec_id: parts.blocked_root_spec_id,
            lineage_compatible,
        })
    })())
}

fn auto_recovery_cooldown_active(
    config: &AutoRecoveryConfig,
    metadata: &BlockedItemMetadata,
    auto_attempts: u64,
    now: &Timestamp,
) -> RuntimeTickResult<bool> {
    let now = parse_timestamp(now, "auto_recovery.now")?;
    let blocked_at = parse_timestamp(&metadata.blocked_at, "auto_recovery.blocked_at")?;
    let cooldown_seconds = config
        .cooldown_seconds
        .get(auto_attempts as usize)
        .or_else(|| config.cooldown_seconds.last())
        .copied()
        .unwrap_or(0);
    Ok((now - blocked_at).whole_seconds() < cooldown_seconds as i64)
}

fn write_auto_recovery_skip(
    paths: &WorkspacePaths,
    snapshot: &RuntimeSnapshot,
    now: &Timestamp,
    candidate: &AutoRecoveryCandidate,
    reason: &str,
    auto_attempt_number: u64,
) -> RuntimeTickResult<PathBuf> {
    let diagnostics_path = write_auto_recovery_diagnostics(
        paths,
        snapshot,
        now,
        &candidate.dependency,
        "skip",
        reason,
        auto_attempt_number,
    )?;
    let event_path = write_blocked_dependency_auto_requeue_skipped_event(
        paths,
        now,
        &candidate.dependency,
        reason,
        &diagnostics_path,
    )?;
    write_blocked_lineage_requires_operator_review_event(
        paths,
        now,
        &candidate.dependency,
        reason,
        &diagnostics_path,
    )?;
    Ok(event_path)
}

fn write_auto_recovery_diagnostics(
    paths: &WorkspacePaths,
    snapshot: &RuntimeSnapshot,
    now: &Timestamp,
    candidate: &StrandedBlockedDependency,
    decision: &str,
    reason: &str,
    auto_attempt_number: u64,
) -> RuntimeTickResult<PathBuf> {
    let path = paths
        .runtime_root
        .join("diagnostics")
        .join("auto-recovery")
        .join(format!(
            "{}-{}.json",
            compact_timestamp(now)?,
            candidate.blocked_task_id
        ));
    let mut diagnostic = BlockedDependencyAutoRecoveryDiagnostic {
        schema_version: "1.0".to_owned(),
        kind: "blocked_dependency_auto_recovery".to_owned(),
        decision: decision.to_owned(),
        reason: reason.to_owned(),
        created_at: now.clone(),
        blocked_task_id: candidate.blocked_task_id.clone(),
        queued_dependent_ids: candidate.queued_dependent_ids.clone(),
        root_spec_id: candidate.root_spec_id.clone(),
        auto_attempt_number,
        metadata: candidate.metadata.clone(),
        pre_recovery_snapshot: AutoRecoveryPreRecoverySnapshot {
            process_running: snapshot.process_running,
            paused: snapshot.paused,
            stop_requested: snapshot.stop_requested,
            active_runs_by_plane: snapshot
                .active_runs_by_plane
                .keys()
                .map(|plane| plane.as_str().to_owned())
                .collect(),
            queue_depth_execution: snapshot.queue_depth_execution,
            queue_depth_planning: snapshot.queue_depth_planning,
            queue_depth_learning: snapshot.queue_depth_learning,
        },
    };
    diagnostic
        .validate_contract()
        .map_err(|error| RuntimeTickError::InvalidState {
            message: error.to_string(),
        })?;
    write_pretty_json(&path, &diagnostic)?;
    Ok(path)
}

pub(crate) fn persist_blocked_item_metadata(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
    decision: &RouterDecision,
    stage_result_path: &Path,
) -> RuntimeTickResult<BlockedItemMetadataRecord> {
    let mut metadata =
        build_blocked_item_metadata(paths, stage_result, decision, stage_result_path);
    metadata
        .validate_contract()
        .map_err(|error| RuntimeTickError::InvalidState {
            message: error.to_string(),
        })?;
    let path = blocked_metadata_path(paths, metadata.work_item_kind, &metadata.work_item_id);
    write_pretty_json(&path, &metadata)?;
    Ok(BlockedItemMetadataRecord { path, metadata })
}

fn build_blocked_item_metadata(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
    decision: &RouterDecision,
    stage_result_path: &Path,
) -> BlockedItemMetadata {
    let (root_idea_id, root_spec_id) = blocked_work_item_lineage(paths, stage_result);
    let failure_class = metadata_string(stage_result, "failure_class")
        .or_else(|| decision.failure_class.clone())
        .unwrap_or_else(|| "stage_declared_blocked".to_owned());
    let blocked_origin = blocked_origin_for_stage_result(stage_result);
    let failure_scope = failure_scope_for_stage_result(stage_result, blocked_origin);
    let auto_requeue_candidate = metadata_bool(stage_result, "auto_requeue_candidate")
        && failure_class_allows_auto_requeue(&failure_class);
    let failure_classifier_code = metadata_string(stage_result, "failure_classifier_code")
        .and_then(|value| FailureClassifierCode::from_value(&value).ok());

    BlockedItemMetadata {
        work_item_kind: stage_result.work_item_kind,
        work_item_id: stage_result.work_item_id.clone(),
        root_spec_id,
        root_idea_id,
        blocked_at: stage_result.completed_at.clone(),
        blocked_origin,
        failure_class,
        failure_scope,
        auto_requeue_candidate,
        failure_classifier_code,
        source_run_id: Some(stage_result.run_id.clone()),
        source_plane: Some(stage_result.plane.as_str().to_owned()),
        source_stage: Some(stage_result.stage.as_str().to_owned()),
        terminal_result: Some(stage_result.terminal_result.as_str().to_owned()),
        stage_result_path: Some(path_relative_to_root(paths, stage_result_path)),
        stdout_path: stage_result
            .stdout_path
            .as_deref()
            .map(|path| string_path_relative_to_root(paths, path)),
        stderr_path: stage_result
            .stderr_path
            .as_deref()
            .map(|path| string_path_relative_to_root(paths, path)),
    }
}

fn load_blocked_item_metadata_at(path: &Path) -> RuntimeTickResult<Option<BlockedItemMetadata>> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Ok(None);
    };
    Ok(BlockedItemMetadata::from_json_str(&raw).ok())
}

fn validate_blocked_task_locations(paths: &WorkspacePaths, task_id: &str) -> RuntimeTickResult<()> {
    let blocked_path = paths.tasks_blocked_dir.join(format!("{task_id}.md"));
    if !blocked_path.is_file() {
        return Err(invalid_state(format!("task {task_id} is not blocked")));
    }
    for (state, directory) in [
        ("queue", &paths.tasks_queue_dir),
        ("active", &paths.tasks_active_dir),
        ("done", &paths.tasks_done_dir),
    ] {
        if directory.join(format!("{task_id}.md")).exists() {
            return Err(invalid_state(format!("task {task_id} is already {state}")));
        }
    }
    Ok(())
}

fn count_auto_requeues(paths: &WorkspacePaths, task_id: &str) -> RuntimeTickResult<u64> {
    let log_path = paths
        .tasks_queue_dir
        .join(format!("{task_id}.requeue.jsonl"));
    let Ok(raw) = fs::read_to_string(&log_path) else {
        return Ok(0);
    };
    let mut count = 0;
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(payload) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if payload
            .as_object()
            .and_then(|object| object.get("auto"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            count += 1;
        }
    }
    Ok(count)
}

fn refresh_snapshot_queue_depths(paths: &WorkspacePaths) {
    if !paths.runtime_snapshot_file.is_file() {
        return;
    }
    let Ok(mut snapshot) = load_snapshot(paths) else {
        return;
    };
    snapshot.queue_depth_execution = count_markdown_files(&paths.tasks_queue_dir);
    snapshot.queue_depth_planning = count_markdown_files(&paths.specs_queue_dir)
        + count_markdown_files(&paths.probes_queue_dir)
        + count_markdown_files(&paths.incidents_incoming_dir);
    snapshot.queue_depth_learning = count_markdown_files(&paths.learning_requests_queue_dir);
    snapshot.updated_at = current_timestamp();
    sync_plane_queue_depths(&mut snapshot);
    let _ = save_snapshot(paths, &snapshot);
}

fn refresh_snapshot_queue_depths_in_place(
    paths: &WorkspacePaths,
    snapshot: &mut RuntimeSnapshot,
    now: &Timestamp,
) -> RuntimeTickResult<()> {
    snapshot.queue_depth_execution = count_markdown_files(&paths.tasks_queue_dir);
    snapshot.queue_depth_planning = count_markdown_files(&paths.specs_queue_dir)
        + count_markdown_files(&paths.probes_queue_dir)
        + count_markdown_files(&paths.incidents_incoming_dir);
    snapshot.queue_depth_learning = count_markdown_files(&paths.learning_requests_queue_dir);
    snapshot.updated_at = now.clone();
    sync_plane_queue_depths(snapshot);
    save_snapshot(paths, snapshot)?;
    Ok(())
}

fn sync_plane_queue_depths(snapshot: &mut RuntimeSnapshot) {
    use crate::contracts::Plane;

    snapshot
        .queue_depths_by_plane
        .insert(Plane::Execution, snapshot.queue_depth_execution);
    snapshot
        .queue_depths_by_plane
        .insert(Plane::Planning, snapshot.queue_depth_planning);
    snapshot
        .queue_depths_by_plane
        .insert(Plane::Learning, snapshot.queue_depth_learning);
}

fn count_markdown_files(directory: &Path) -> u64 {
    fs::read_dir(directory)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter(|entry| {
            let path = entry.path();
            path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("md")
        })
        .count() as u64
}

fn write_blocked_task_requeued_event(
    paths: &WorkspacePaths,
    result: &BlockedTaskRequeueResult,
) -> RuntimeTickResult<PathBuf> {
    write_runtime_event(
        paths,
        "blocked_task_requeued",
        json_object([
            ("task_id", Value::String(result.task_id.clone())),
            ("actor", Value::String(result.actor.clone())),
            ("auto", Value::Bool(result.auto)),
            ("reason", Value::String(result.reason.clone())),
            (
                "failure_class",
                result
                    .failure_class
                    .as_ref()
                    .map(|value| Value::String(value.clone()))
                    .unwrap_or(Value::Null),
            ),
            ("attempt_number", json!(result.attempt_number)),
            ("source_state", Value::String(result.source_state.clone())),
            (
                "destination_state",
                Value::String(result.destination_state.clone()),
            ),
        ]),
    )
}

fn write_blocked_dependency_auto_requeued_event(
    paths: &WorkspacePaths,
    now: &Timestamp,
    dependency: &StrandedBlockedDependency,
    failure_class: &str,
    diagnostics_path: &Path,
    attempt_number: u64,
) -> RuntimeTickResult<PathBuf> {
    write_runtime_event_at(
        paths,
        "blocked_dependency_auto_requeued",
        json_object([
            ("task_id", Value::String(dependency.blocked_task_id.clone())),
            ("queued_dependents", json!(dependency.queued_dependent_ids)),
            (
                "root_spec_id",
                dependency
                    .root_spec_id
                    .as_ref()
                    .map(|value| Value::String(value.clone()))
                    .unwrap_or(Value::Null),
            ),
            ("failure_class", Value::String(failure_class.to_owned())),
            ("attempt_number", json!(attempt_number)),
            (
                "diagnostics_path",
                Value::String(path_relative_to_root(paths, diagnostics_path)),
            ),
        ]),
        now,
    )
}

fn write_blocked_dependency_auto_requeue_skipped_event(
    paths: &WorkspacePaths,
    now: &Timestamp,
    dependency: &StrandedBlockedDependency,
    reason: &str,
    diagnostics_path: &Path,
) -> RuntimeTickResult<PathBuf> {
    write_runtime_event_at(
        paths,
        "blocked_dependency_auto_requeue_skipped",
        json_object([
            ("task_id", Value::String(dependency.blocked_task_id.clone())),
            ("queued_dependents", json!(dependency.queued_dependent_ids)),
            (
                "root_spec_id",
                dependency
                    .root_spec_id
                    .as_ref()
                    .map(|value| Value::String(value.clone()))
                    .unwrap_or(Value::Null),
            ),
            ("reason", Value::String(reason.to_owned())),
            (
                "failure_class",
                dependency
                    .metadata
                    .as_ref()
                    .map(|metadata| Value::String(metadata.failure_class.clone()))
                    .unwrap_or(Value::Null),
            ),
            (
                "diagnostics_path",
                Value::String(path_relative_to_root(paths, diagnostics_path)),
            ),
        ]),
        now,
    )
}

fn write_blocked_lineage_requires_operator_review_event(
    paths: &WorkspacePaths,
    now: &Timestamp,
    dependency: &StrandedBlockedDependency,
    reason: &str,
    diagnostics_path: &Path,
) -> RuntimeTickResult<PathBuf> {
    write_runtime_event_at(
        paths,
        "blocked_lineage_requires_operator_review",
        json_object([
            ("task_id", Value::String(dependency.blocked_task_id.clone())),
            ("queued_dependents", json!(dependency.queued_dependent_ids)),
            (
                "root_spec_id",
                dependency
                    .root_spec_id
                    .as_ref()
                    .map(|value| Value::String(value.clone()))
                    .unwrap_or(Value::Null),
            ),
            ("reason", Value::String(reason.to_owned())),
            (
                "diagnostics_path",
                Value::String(path_relative_to_root(paths, diagnostics_path)),
            ),
        ]),
        now,
    )
}

fn write_runtime_event(
    paths: &WorkspacePaths,
    event_type: &str,
    data: Map<String, Value>,
) -> RuntimeTickResult<PathBuf> {
    let occurred_at = current_timestamp();
    write_runtime_event_at(paths, event_type, data, &occurred_at)
}

fn write_runtime_event_at(
    paths: &WorkspacePaths,
    event_type: &str,
    data: Map<String, Value>,
    occurred_at: &Timestamp,
) -> RuntimeTickResult<PathBuf> {
    let log_path = paths.logs_dir.join("runtime_events.jsonl");
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| RuntimeTickError::Io {
            path: parent.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    let payload = json!({
        "schema_version": "1.0",
        "kind": "runtime_event",
        "event_type": event_type,
        "occurred_at": occurred_at.as_str(),
        "data": data,
    });
    let line = serde_json::to_string(&payload).map_err(|error| RuntimeTickError::InvalidState {
        message: error.to_string(),
    })? + "\n";
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| RuntimeTickError::Io {
            path: log_path.clone(),
            message: error.to_string(),
        })?;
    file.write_all(line.as_bytes())
        .map_err(|error| RuntimeTickError::Io {
            path: log_path.clone(),
            message: error.to_string(),
        })?;
    Ok(log_path)
}

fn blocked_work_item_lineage(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
) -> (Option<String>, Option<String>) {
    if stage_result.work_item_kind != WorkItemKind::Task {
        return (None, None);
    }
    let mut candidates = vec![
        paths
            .tasks_blocked_dir
            .join(format!("{}.md", stage_result.work_item_id)),
        paths
            .tasks_active_dir
            .join(format!("{}.md", stage_result.work_item_id)),
    ];
    if let Some(active_path) = metadata_string(stage_result, "active_work_item_path") {
        candidates.push(paths.root.join(active_path));
    }

    for path in candidates {
        let Some(task) = read_task_document_or_none(&path) else {
            continue;
        };
        return (
            task.root_idea_id.clone(),
            effective_root_spec_id(&task).map(str::to_owned),
        );
    }
    (None, None)
}

fn blocked_origin_for_stage_result(stage_result: &StageResultEnvelope) -> BlockedOrigin {
    if let Some(origin) = metadata_string(stage_result, "blocked_origin")
        .and_then(|value| BlockedOrigin::from_value(&value).ok())
    {
        return origin;
    }
    if metadata_string(stage_result, "normalization_source").as_deref() == Some("failure") {
        BlockedOrigin::RunnerFailure
    } else {
        BlockedOrigin::StageTerminal
    }
}

fn failure_scope_for_stage_result(
    stage_result: &StageResultEnvelope,
    blocked_origin: BlockedOrigin,
) -> FailureScope {
    if let Some(scope) = metadata_string(stage_result, "failure_scope")
        .and_then(|value| FailureScope::from_value(&value).ok())
    {
        return scope;
    }
    if blocked_origin == BlockedOrigin::StageTerminal {
        FailureScope::Semantic
    } else {
        FailureScope::Unknown
    }
}

fn metadata_string(stage_result: &StageResultEnvelope, key: &str) -> Option<String> {
    stage_result
        .metadata
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

fn metadata_bool(stage_result: &StageResultEnvelope, key: &str) -> bool {
    stage_result
        .metadata
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn read_task_document_or_none(path: &Path) -> Option<TaskDocument> {
    read_task_document(path).ok()
}

fn read_task_document(path: &Path) -> RuntimeTickResult<TaskDocument> {
    let raw = fs::read_to_string(path).map_err(|error| RuntimeTickError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    parse_task_document_with_source(&raw, &path.display().to_string()).map_err(|source| {
        RuntimeTickError::WorkDocument {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn effective_root_spec_id(document: &TaskDocument) -> Option<&str> {
    document
        .root_spec_id
        .as_deref()
        .or(document.spec_id.as_deref())
}

fn markdown_stems(directory: &Path) -> RuntimeTickResult<BTreeSet<String>> {
    Ok(markdown_files(directory)?
        .into_iter()
        .filter_map(|path| path.file_stem()?.to_str().map(str::to_owned))
        .collect())
}

fn markdown_files(directory: &Path) -> RuntimeTickResult<Vec<PathBuf>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| RuntimeTickError::Io {
        path: directory.to_path_buf(),
        message: error.to_string(),
    })? {
        let entry = entry.map_err(|error| RuntimeTickError::Io {
            path: directory.to_path_buf(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("md") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn path_relative_to_root(paths: &WorkspacePaths, path: &Path) -> String {
    path.strip_prefix(&paths.root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn string_path_relative_to_root(paths: &WorkspacePaths, raw_path: &str) -> String {
    let path = Path::new(raw_path);
    if path.is_absolute() {
        path_relative_to_root(paths, path)
    } else {
        raw_path.to_owned()
    }
}

fn write_pretty_json<T: serde::Serialize>(path: &Path, value: &T) -> RuntimeTickResult<()> {
    let mut payload =
        serde_json::to_string_pretty(value).map_err(|error| RuntimeTickError::InvalidState {
            message: error.to_string(),
        })?;
    payload.push('\n');
    atomic_write_text(path, &payload)?;
    Ok(())
}

fn compact_timestamp(timestamp: &Timestamp) -> RuntimeTickResult<String> {
    let parsed = parse_timestamp(timestamp, "auto_recovery.created_at")?;
    Ok(format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        parsed.year(),
        u8::from(parsed.month()),
        parsed.day(),
        parsed.hour(),
        parsed.minute(),
        parsed.second()
    ))
}

fn parse_timestamp(
    timestamp: &Timestamp,
    field_name: &'static str,
) -> RuntimeTickResult<OffsetDateTime> {
    OffsetDateTime::parse(timestamp.as_str(), &Rfc3339).map_err(|error| RuntimeTickError::Time {
        field_name,
        message: error.to_string(),
    })
}

fn current_timestamp() -> crate::contracts::Timestamp {
    let rendered = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());
    crate::contracts::Timestamp::parse("runtime_event.occurred_at", &rendered).unwrap_or_else(
        |_| crate::contracts::Timestamp::parse("fallback", "1970-01-01T00:00:00Z").unwrap(),
    )
}

fn invalid_state(message: impl Into<String>) -> RuntimeTickError {
    RuntimeTickError::InvalidState {
        message: message.into(),
    }
}

fn json_object(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Map<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}
