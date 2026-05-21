//! Closure-lineage repair helpers for safe queued/blocked work documents.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    contracts::{
        ClosureTargetState, IncidentDocument, Plane, SpecDocument, SpecSourceType, StageName,
        TaskDocument, Timestamp, WorkDocumentError, WorkItemKind, validate_safe_identifier,
    },
    work_documents::{
        parse_incident_document_with_source, parse_spec_document_with_source,
        parse_task_document_with_source, render_incident_document, render_task_document,
    },
};

use super::{
    RuntimeOwnershipLockState, StateStoreError, WorkspacePaths, atomic_write_text,
    inspect_runtime_ownership_lock, load_snapshot, save_snapshot,
};

static REPORT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Result type for closure-lineage repair helpers.
pub type LineageRepairResult<T> = Result<T, LineageRepairError>;

/// Failures produced while loading, planning, or applying closure-lineage repair.
#[derive(Debug)]
pub enum LineageRepairError {
    /// The root spec id could not be used as a safe target filename.
    InvalidRootSpecId {
        /// Invalid id value.
        value: String,
        /// Human-readable validation failure.
        message: String,
    },
    /// The requested closure target JSON file does not exist.
    MissingClosureTarget {
        /// Expected closure target path.
        path: PathBuf,
    },
    /// A filesystem operation failed.
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying IO error message.
        message: String,
    },
    /// JSON syntax failed before typed validation.
    JsonSyntax {
        /// Path being decoded.
        path: PathBuf,
        /// Serde error message.
        message: String,
    },
    /// JSON did not contain the expected object payload.
    NonObjectPayload {
        /// Path being decoded.
        path: PathBuf,
    },
    /// A closure target violated the typed contract.
    ClosureTargetContract {
        /// Path being decoded.
        path: PathBuf,
        /// Typed work-document contract error.
        source: WorkDocumentError,
    },
    /// A work document failed typed headed-markdown parsing during apply.
    WorkDocument {
        /// Work document path.
        path: PathBuf,
        /// Typed work-document contract error.
        source: WorkDocumentError,
    },
    /// Runtime state persistence failed.
    StateStore(StateStoreError),
    /// Apply was refused because a live daemon owns the workspace.
    ActiveRuntimeOwnershipLock {
        /// Human-readable lock status detail.
        detail: String,
    },
    /// Apply was refused because the runtime snapshot reports an active stage.
    ActiveRuntimeStage {
        /// Active stage reported by the snapshot.
        stage: StageName,
    },
    /// JSON rendering failed before persistence.
    JsonRender {
        /// Path being rendered.
        path: PathBuf,
        /// Serde error message.
        message: String,
    },
    /// Timestamp rendering failed.
    Timestamp {
        /// Human-readable timestamp error.
        message: String,
    },
}

impl LineageRepairError {
    fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: error.to_string(),
        }
    }

    fn work_document(path: impl Into<PathBuf>, source: WorkDocumentError) -> Self {
        Self::WorkDocument {
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for LineageRepairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRootSpecId { value, message } => {
                write!(f, "invalid root spec id `{value}`: {message}")
            }
            Self::MissingClosureTarget { path } => {
                write!(f, "closure target does not exist at {}", path.display())
            }
            Self::Io { path, message } => {
                write!(
                    f,
                    "lineage repair filesystem error at {}: {message}",
                    path.display()
                )
            }
            Self::JsonSyntax { path, message } => {
                write!(
                    f,
                    "failed to decode closure target JSON at {}: {message}",
                    path.display()
                )
            }
            Self::NonObjectPayload { path } => {
                write!(f, "expected object payload in {}", path.display())
            }
            Self::ClosureTargetContract { path, source } => {
                write!(
                    f,
                    "closure target contract error at {}: {source}",
                    path.display()
                )
            }
            Self::WorkDocument { path, source } => {
                write!(
                    f,
                    "lineage repair work document error at {}: {source}",
                    path.display()
                )
            }
            Self::StateStore(error) => write!(f, "{error}"),
            Self::ActiveRuntimeOwnershipLock { detail } => {
                write!(
                    f,
                    "active runtime ownership lock prevents lineage repair: {detail}"
                )
            }
            Self::ActiveRuntimeStage { stage } => {
                write!(f, "active runtime stage prevents lineage repair: {stage}")
            }
            Self::JsonRender { path, message } => {
                write!(
                    f,
                    "failed to render lineage repair JSON at {}: {message}",
                    path.display()
                )
            }
            Self::Timestamp { message } => write!(f, "lineage repair timestamp error: {message}"),
        }
    }
}

impl std::error::Error for LineageRepairError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ClosureTargetContract { source, .. } | Self::WorkDocument { source, .. } => {
                Some(source)
            }
            Self::StateStore(error) => Some(error),
            Self::InvalidRootSpecId { .. }
            | Self::MissingClosureTarget { .. }
            | Self::Io { .. }
            | Self::JsonSyntax { .. }
            | Self::NonObjectPayload { .. }
            | Self::ActiveRuntimeOwnershipLock { .. }
            | Self::ActiveRuntimeStage { .. }
            | Self::JsonRender { .. }
            | Self::Timestamp { .. } => None,
        }
    }
}

impl From<StateStoreError> for LineageRepairError {
    fn from(value: StateStoreError) -> Self {
        Self::StateStore(value)
    }
}

/// Queue state label used by Python lineage diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LineageWorkState {
    /// Queued document.
    #[serde(rename = "queue")]
    Queue,
    /// Active document.
    #[serde(rename = "active")]
    Active,
    /// Blocked document.
    #[serde(rename = "blocked")]
    Blocked,
}

impl LineageWorkState {
    /// Returns the Python-compatible state label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Active => "active",
            Self::Blocked => "blocked",
        }
    }
}

impl fmt::Display for LineageWorkState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Diagnostic reason for a closure-lineage drift finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineageDiagnosticReason {
    /// Same root idea but wrong root spec id.
    #[serde(rename = "same_root_idea_different_root_spec")]
    SameRootIdeaDifferentRootSpec,
    /// Known root-spec alias points at the target.
    #[serde(rename = "known_root_spec_alias")]
    KnownRootSpecAlias,
}

impl LineageDiagnosticReason {
    /// Returns the Python-compatible reason label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SameRootIdeaDifferentRootSpec => "same_root_idea_different_root_spec",
            Self::KnownRootSpecAlias => "known_root_spec_alias",
        }
    }
}

impl fmt::Display for LineageDiagnosticReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One work document attached to the wrong root spec for an open closure target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageDriftFinding {
    pub work_item_kind: WorkItemKind,
    pub work_item_id: String,
    pub state: LineageWorkState,
    pub path: String,
    pub expected_root_spec_id: String,
    #[serde(default)]
    pub actual_root_spec_id: Option<String>,
    #[serde(default)]
    pub root_idea_id: Option<String>,
    #[serde(default)]
    pub spec_id: Option<String>,
    pub diagnostic_reason: LineageDiagnosticReason,
}

/// Durable diagnostic for closure-lineage drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageDriftDiagnostic {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_lineage_drift_kind")]
    pub kind: String,
    pub root_spec_id: String,
    pub root_idea_id: String,
    pub detected_at: Timestamp,
    #[serde(default)]
    pub findings: Vec<LineageDriftFinding>,
    pub recommended_command: String,
}

/// One field-level lineage repair that can be previewed or applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageRepairChange {
    pub work_item_kind: WorkItemKind,
    pub work_item_id: String,
    pub state: LineageWorkState,
    pub path: String,
    pub field_name: String,
    #[serde(default)]
    pub old_value: Option<String>,
    pub new_value: String,
}

/// Preview of safe closure-lineage repairs for queued/blocked documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageRepairPlan {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_lineage_repair_plan_kind")]
    pub kind: String,
    pub root_spec_id: String,
    pub root_idea_id: String,
    pub created_at: Timestamp,
    #[serde(default)]
    pub changes: Vec<LineageRepairChange>,
    #[serde(default)]
    pub skipped_findings: Vec<LineageDriftFinding>,
}

/// Result from a preview or apply repair helper call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureLineageRepairOutcome {
    /// Loaded closure target state.
    pub target: ClosureTargetState,
    /// Planned safe changes and skipped findings.
    pub plan: LineageRepairPlan,
    /// Non-applied report path always written by preview/apply preparation.
    pub preview_report_path: PathBuf,
    /// Applied report path written only after successful apply.
    pub applied_report_path: Option<PathBuf>,
    /// Count of documents repaired by successful apply.
    pub repaired_count: usize,
    /// Runtime event log path written only after successful apply.
    pub event_log_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
struct LineageSurface<'a> {
    directory: &'a Path,
    document_kind: LineageDocumentKind,
    work_item_kind: WorkItemKind,
    state: LineageWorkState,
}

#[derive(Debug, Clone, Copy)]
enum LineageDocumentKind {
    Task,
    Spec,
    Incident,
}

enum SurfaceDocument {
    Task(TaskDocument),
    Spec(SpecDocument),
    Incident(IncidentDocument),
}

impl SurfaceDocument {
    fn root_idea_id(&self) -> Option<&str> {
        match self {
            Self::Task(document) => document.root_idea_id.as_deref(),
            Self::Spec(document) => document.root_idea_id.as_deref(),
            Self::Incident(document) => document.root_idea_id.as_deref(),
        }
    }

    fn effective_root_spec_id(&self) -> Option<&str> {
        match self {
            Self::Task(document) => document
                .root_spec_id
                .as_deref()
                .or(document.spec_id.as_deref()),
            Self::Spec(document) => document.root_spec_id.as_deref().or({
                if matches!(
                    document.source_type,
                    SpecSourceType::Idea | SpecSourceType::Manual
                ) {
                    Some(document.spec_id.as_str())
                } else {
                    None
                }
            }),
            Self::Incident(document) => document
                .root_spec_id
                .as_deref()
                .or(document.source_spec_id.as_deref()),
        }
    }

    fn item_id(&self) -> &str {
        match self {
            Self::Task(document) => &document.task_id,
            Self::Spec(document) => &document.spec_id,
            Self::Incident(document) => &document.incident_id,
        }
    }

    fn document_spec_id(&self) -> Option<&str> {
        match self {
            Self::Task(document) => document.spec_id.as_deref(),
            Self::Spec(document) => Some(document.spec_id.as_str()),
            Self::Incident(document) => document.source_spec_id.as_deref(),
        }
    }
}

/// Return the closure target state path after validating the root spec id.
pub fn closure_target_state_path(
    paths: &WorkspacePaths,
    root_spec_id: &str,
) -> LineageRepairResult<PathBuf> {
    validate_root_spec_id(root_spec_id)?;
    Ok(paths
        .arbiter_targets_dir
        .join(format!("{root_spec_id}.json")))
}

/// Load and validate one closure target state from `arbiter/targets`.
pub fn load_closure_target_state(
    paths: &WorkspacePaths,
    root_spec_id: &str,
) -> LineageRepairResult<ClosureTargetState> {
    let path = closure_target_state_path(paths, root_spec_id)?;
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(LineageRepairError::MissingClosureTarget { path });
        }
        Err(error) => return Err(LineageRepairError::io(&path, error)),
    };
    let value: Value =
        serde_json::from_str(&raw).map_err(|error| LineageRepairError::JsonSyntax {
            path: path.clone(),
            message: error.to_string(),
        })?;
    if !value.is_object() {
        return Err(LineageRepairError::NonObjectPayload { path });
    }
    let state: ClosureTargetState =
        serde_json::from_value(value).map_err(|error| LineageRepairError::JsonSyntax {
            path: path.clone(),
            message: error.to_string(),
        })?;
    state
        .validate()
        .map_err(|source| LineageRepairError::ClosureTargetContract {
            path: path.clone(),
            source,
        })?;
    if state.root_spec_id != root_spec_id {
        return Err(LineageRepairError::ClosureTargetContract {
            path,
            source: WorkDocumentError::InvalidField {
                field_name: "root_spec_id",
                value: state.root_spec_id,
                message: format!("must match requested root spec id `{root_spec_id}`"),
            },
        });
    }
    Ok(state)
}

/// Save one validated closure target state atomically.
pub fn save_closure_target_state(
    paths: &WorkspacePaths,
    target: &ClosureTargetState,
) -> LineageRepairResult<PathBuf> {
    target
        .validate()
        .map_err(|source| LineageRepairError::ClosureTargetContract {
            path: paths
                .arbiter_targets_dir
                .join(format!("{}.json", target.root_spec_id)),
            source,
        })?;
    let path = closure_target_state_path(paths, &target.root_spec_id)?;
    let mut payload =
        serde_json::to_string_pretty(target).map_err(|error| LineageRepairError::JsonRender {
            path: path.clone(),
            message: error.to_string(),
        })?;
    payload.push('\n');
    atomic_write_text(&path, &payload)?;
    Ok(path)
}

/// Return deterministic diagnostic aliases for known watcher-derived root ids.
#[must_use]
pub fn known_root_spec_aliases(root_spec_id: &str) -> Vec<String> {
    if let Some(alias) = root_spec_id.strip_prefix("idea-idea-") {
        let alias = format!("idea-{alias}");
        if alias != root_spec_id {
            return vec![alias];
        }
    }
    Vec::new()
}

/// Find queued/active/blocked work tied to a target but attached to the wrong root.
pub fn scan_closure_lineage_drift(
    paths: &WorkspacePaths,
    target: &ClosureTargetState,
) -> LineageRepairResult<Option<LineageDriftDiagnostic>> {
    let detected_at = current_timestamp()?;
    scan_closure_lineage_drift_at(paths, target, detected_at)
}

/// Find queued/active/blocked work with an injected diagnostic timestamp.
pub fn scan_closure_lineage_drift_at(
    paths: &WorkspacePaths,
    target: &ClosureTargetState,
    detected_at: Timestamp,
) -> LineageRepairResult<Option<LineageDriftDiagnostic>> {
    let aliases: BTreeSet<String> = known_root_spec_aliases(&target.root_spec_id)
        .into_iter()
        .collect();
    let mut findings = Vec::new();

    for surface in lineage_surfaces(paths) {
        for path in list_markdown_files(surface.directory)? {
            let Some(document) = parse_surface_document(&path, surface.document_kind)? else {
                continue;
            };
            let actual_root_spec_id = document.effective_root_spec_id().map(str::to_owned);
            if actual_root_spec_id.as_deref() == Some(target.root_spec_id.as_str()) {
                continue;
            }
            let Some(reason) =
                drift_reason(&document, target, actual_root_spec_id.as_deref(), &aliases)
            else {
                continue;
            };
            findings.push(LineageDriftFinding {
                work_item_kind: surface.work_item_kind,
                work_item_id: document.item_id().to_owned(),
                state: surface.state,
                path: workspace_relative_path(paths, &path),
                expected_root_spec_id: target.root_spec_id.clone(),
                actual_root_spec_id,
                root_idea_id: document.root_idea_id().map(str::to_owned),
                spec_id: document.document_spec_id().map(str::to_owned),
                diagnostic_reason: reason,
            });
        }
    }

    if findings.is_empty() {
        return Ok(None);
    }

    findings.sort_by(|left, right| {
        (&left.path, &left.work_item_id).cmp(&(&right.path, &right.work_item_id))
    });
    Ok(Some(LineageDriftDiagnostic {
        schema_version: default_schema_version(),
        kind: default_lineage_drift_kind(),
        root_spec_id: target.root_spec_id.clone(),
        root_idea_id: target.root_idea_id.clone(),
        detected_at,
        findings,
        recommended_command: format!(
            "millrace queue repair-lineage --workspace <workspace> --root-spec-id {} --apply",
            target.root_spec_id
        ),
    }))
}

/// Return the durable diagnostic path for one closure target.
#[must_use]
pub fn lineage_drift_diagnostic_path(paths: &WorkspacePaths, root_spec_id: &str) -> PathBuf {
    paths
        .arbiter_dir
        .join("diagnostics")
        .join("lineage-drift")
        .join(format!("{root_spec_id}.json"))
}

/// Persist a closure-lineage drift diagnostic atomically.
pub fn write_lineage_drift_diagnostic(
    paths: &WorkspacePaths,
    diagnostic: &LineageDriftDiagnostic,
) -> LineageRepairResult<PathBuf> {
    let path = lineage_drift_diagnostic_path(paths, &diagnostic.root_spec_id);
    let mut rendered = serde_json::to_string_pretty(diagnostic).map_err(|error| {
        LineageRepairError::JsonRender {
            path: path.clone(),
            message: error.to_string(),
        }
    })?;
    rendered.push('\n');
    atomic_write_text(&path, &rendered)?;
    Ok(path)
}

/// Build a preview of safe queued/blocked lineage repair changes.
pub fn build_lineage_repair_plan(
    paths: &WorkspacePaths,
    target: &ClosureTargetState,
) -> LineageRepairResult<LineageRepairPlan> {
    let created_at = current_timestamp()?;
    build_lineage_repair_plan_at(paths, target, created_at)
}

/// Build a preview of safe queued/blocked lineage repair changes with an injected timestamp.
pub fn build_lineage_repair_plan_at(
    paths: &WorkspacePaths,
    target: &ClosureTargetState,
    created_at: Timestamp,
) -> LineageRepairResult<LineageRepairPlan> {
    let aliases: BTreeSet<String> = known_root_spec_aliases(&target.root_spec_id)
        .into_iter()
        .collect();
    let Some(diagnostic) = scan_closure_lineage_drift_at(paths, target, created_at.clone())? else {
        return Ok(LineageRepairPlan {
            schema_version: default_schema_version(),
            kind: default_lineage_repair_plan_kind(),
            root_spec_id: target.root_spec_id.clone(),
            root_idea_id: target.root_idea_id.clone(),
            created_at,
            changes: Vec::new(),
            skipped_findings: Vec::new(),
        });
    };

    let mut changes = Vec::new();
    let mut skipped_findings = Vec::new();
    for finding in diagnostic.findings {
        if !matches!(
            finding.state,
            LineageWorkState::Queue | LineageWorkState::Blocked
        ) {
            skipped_findings.push(finding);
            continue;
        }
        match finding.work_item_kind {
            WorkItemKind::Task => {
                changes.extend(task_repair_changes(&finding, target, &aliases));
            }
            WorkItemKind::Incident => {
                changes.extend(incident_repair_changes(&finding, target));
            }
            WorkItemKind::Probe
            | WorkItemKind::Spec
            | WorkItemKind::LearningRequest
            | WorkItemKind::BlueprintDraft => skipped_findings.push(finding),
        }
    }

    Ok(LineageRepairPlan {
        schema_version: default_schema_version(),
        kind: default_lineage_repair_plan_kind(),
        root_spec_id: target.root_spec_id.clone(),
        root_idea_id: target.root_idea_id.clone(),
        created_at,
        changes,
        skipped_findings,
    })
}

/// Persist a durable repair report.
pub fn write_lineage_repair_report(
    paths: &WorkspacePaths,
    plan: &LineageRepairPlan,
    applied: bool,
) -> LineageRepairResult<PathBuf> {
    let path = lineage_repair_report_path(paths, &plan.created_at)?;
    let mut payload =
        serde_json::to_value(plan).map_err(|error| LineageRepairError::JsonRender {
            path: path.clone(),
            message: error.to_string(),
        })?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| LineageRepairError::JsonRender {
            path: path.clone(),
            message: "lineage repair plan did not render as an object".to_owned(),
        })?;
    object.insert("applied".to_owned(), Value::Bool(applied));
    let mut rendered =
        serde_json::to_string_pretty(&payload).map_err(|error| LineageRepairError::JsonRender {
            path: path.clone(),
            message: error.to_string(),
        })?;
    rendered.push('\n');
    atomic_write_text(&path, &rendered)?;
    Ok(path)
}

/// Apply a safe queued/blocked lineage repair plan.
pub fn apply_lineage_repair_plan(
    paths: &WorkspacePaths,
    plan: &LineageRepairPlan,
) -> LineageRepairResult<usize> {
    let mut changes_by_path: BTreeMap<&str, Vec<&LineageRepairChange>> = BTreeMap::new();
    for change in &plan.changes {
        changes_by_path
            .entry(change.path.as_str())
            .or_default()
            .push(change);
    }

    let mut repaired_paths = 0;
    for (relative_path, changes) in changes_by_path {
        let path = paths.root.join(relative_path);
        match changes[0].work_item_kind {
            WorkItemKind::Task => {
                let raw = fs::read_to_string(&path)
                    .map_err(|error| LineageRepairError::io(&path, error))?;
                let mut document =
                    parse_task_document_with_source(&raw, &path.display().to_string())
                        .map_err(|error| LineageRepairError::work_document(&path, error))?;
                for change in changes {
                    match change.field_name.as_str() {
                        "root_spec_id" => document.root_spec_id = Some(change.new_value.clone()),
                        "spec_id" => document.spec_id = Some(change.new_value.clone()),
                        _ => {}
                    }
                }
                atomic_write_text(&path, &render_task_document(&document))?;
                repaired_paths += 1;
            }
            WorkItemKind::Incident => {
                let raw = fs::read_to_string(&path)
                    .map_err(|error| LineageRepairError::io(&path, error))?;
                let mut document =
                    parse_incident_document_with_source(&raw, &path.display().to_string())
                        .map_err(|error| LineageRepairError::work_document(&path, error))?;
                for change in changes {
                    if change.field_name == "root_spec_id" {
                        document.root_spec_id = Some(change.new_value.clone());
                    }
                }
                atomic_write_text(&path, &render_incident_document(&document))?;
                repaired_paths += 1;
            }
            WorkItemKind::Probe
            | WorkItemKind::Spec
            | WorkItemKind::LearningRequest
            | WorkItemKind::BlueprintDraft => {}
        }
    }

    Ok(repaired_paths)
}

/// Refresh queue-depth snapshot fields after successful repair apply.
pub fn refresh_lineage_repair_snapshot(paths: &WorkspacePaths) -> LineageRepairResult<()> {
    let mut snapshot = load_snapshot(paths)?;
    snapshot.queue_depth_execution = count_markdown_files(&paths.tasks_queue_dir)?;
    snapshot.queue_depth_planning = count_markdown_files(&paths.specs_queue_dir)?
        + count_markdown_files(&paths.incidents_incoming_dir)?;
    snapshot
        .queue_depths_by_plane
        .insert(Plane::Execution, snapshot.queue_depth_execution);
    snapshot
        .queue_depths_by_plane
        .insert(Plane::Planning, snapshot.queue_depth_planning);
    save_snapshot(paths, &snapshot)?;
    Ok(())
}

/// Append the Python-compatible `closure_lineage_repaired` runtime event.
pub fn write_closure_lineage_repaired_event(
    paths: &WorkspacePaths,
    target: &ClosureTargetState,
    repaired_count: usize,
    repair_report_path: &Path,
) -> LineageRepairResult<PathBuf> {
    let log_path = paths.logs_dir.join("runtime_events.jsonl");
    let occurred_at = current_timestamp()?;
    let payload = json!({
        "schema_version": "1.0",
        "kind": "runtime_event",
        "event_type": "closure_lineage_repaired",
        "occurred_at": occurred_at.as_str(),
        "data": {
            "root_spec_id": target.root_spec_id,
            "repair_count": repaired_count,
            "repair_report_path": workspace_relative_path(paths, repair_report_path),
        }
    });
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| LineageRepairError::io(parent, error))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| LineageRepairError::io(&log_path, error))?;
    use std::io::Write as _;
    writeln!(file, "{payload}").map_err(|error| LineageRepairError::io(&log_path, error))?;
    Ok(log_path)
}

/// Preview or apply closure-lineage repair through the full guarded workspace boundary.
pub fn repair_closure_lineage(
    paths: &WorkspacePaths,
    root_spec_id: &str,
    apply: bool,
) -> LineageRepairResult<ClosureLineageRepairOutcome> {
    let target = load_closure_target_state(paths, root_spec_id)?;
    let plan = build_lineage_repair_plan(paths, &target)?;
    let preview_report_path = write_lineage_repair_report(paths, &plan, false)?;
    if !apply {
        return Ok(ClosureLineageRepairOutcome {
            target,
            plan,
            preview_report_path,
            applied_report_path: None,
            repaired_count: 0,
            event_log_path: None,
        });
    }

    let lock_status = inspect_runtime_ownership_lock(paths);
    if lock_status.state == RuntimeOwnershipLockState::Active {
        return Err(LineageRepairError::ActiveRuntimeOwnershipLock {
            detail: lock_status.detail,
        });
    }
    let snapshot = load_snapshot(paths)?;
    if let Some(stage) = snapshot.active_stage {
        return Err(LineageRepairError::ActiveRuntimeStage { stage });
    }

    let repaired_count = apply_lineage_repair_plan(paths, &plan)?;
    refresh_lineage_repair_snapshot(paths)?;
    let applied_report_path = write_lineage_repair_report(paths, &plan, true)?;
    let event_log_path =
        write_closure_lineage_repaired_event(paths, &target, repaired_count, &applied_report_path)?;

    Ok(ClosureLineageRepairOutcome {
        target,
        plan,
        preview_report_path,
        applied_report_path: Some(applied_report_path),
        repaired_count,
        event_log_path: Some(event_log_path),
    })
}

fn validate_root_spec_id(root_spec_id: &str) -> LineageRepairResult<()> {
    validate_safe_identifier(root_spec_id, "root_spec_id")
        .map(|_| ())
        .map_err(|error| LineageRepairError::InvalidRootSpecId {
            value: root_spec_id.to_owned(),
            message: error.to_string(),
        })
}

fn lineage_surfaces(paths: &WorkspacePaths) -> Vec<LineageSurface<'_>> {
    vec![
        LineageSurface {
            directory: &paths.tasks_queue_dir,
            document_kind: LineageDocumentKind::Task,
            work_item_kind: WorkItemKind::Task,
            state: LineageWorkState::Queue,
        },
        LineageSurface {
            directory: &paths.tasks_active_dir,
            document_kind: LineageDocumentKind::Task,
            work_item_kind: WorkItemKind::Task,
            state: LineageWorkState::Active,
        },
        LineageSurface {
            directory: &paths.tasks_blocked_dir,
            document_kind: LineageDocumentKind::Task,
            work_item_kind: WorkItemKind::Task,
            state: LineageWorkState::Blocked,
        },
        LineageSurface {
            directory: &paths.specs_queue_dir,
            document_kind: LineageDocumentKind::Spec,
            work_item_kind: WorkItemKind::Spec,
            state: LineageWorkState::Queue,
        },
        LineageSurface {
            directory: &paths.specs_active_dir,
            document_kind: LineageDocumentKind::Spec,
            work_item_kind: WorkItemKind::Spec,
            state: LineageWorkState::Active,
        },
        LineageSurface {
            directory: &paths.specs_blocked_dir,
            document_kind: LineageDocumentKind::Spec,
            work_item_kind: WorkItemKind::Spec,
            state: LineageWorkState::Blocked,
        },
        LineageSurface {
            directory: &paths.incidents_incoming_dir,
            document_kind: LineageDocumentKind::Incident,
            work_item_kind: WorkItemKind::Incident,
            state: LineageWorkState::Queue,
        },
        LineageSurface {
            directory: &paths.incidents_active_dir,
            document_kind: LineageDocumentKind::Incident,
            work_item_kind: WorkItemKind::Incident,
            state: LineageWorkState::Active,
        },
        LineageSurface {
            directory: &paths.incidents_blocked_dir,
            document_kind: LineageDocumentKind::Incident,
            work_item_kind: WorkItemKind::Incident,
            state: LineageWorkState::Blocked,
        },
    ]
}

fn parse_surface_document(
    path: &Path,
    document_kind: LineageDocumentKind,
) -> LineageRepairResult<Option<SurfaceDocument>> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(LineageRepairError::io(path, error)),
    };
    let source_name = path.display().to_string();
    let parsed = match document_kind {
        LineageDocumentKind::Task => {
            parse_task_document_with_source(&raw, &source_name).map(SurfaceDocument::Task)
        }
        LineageDocumentKind::Spec => {
            parse_spec_document_with_source(&raw, &source_name).map(SurfaceDocument::Spec)
        }
        LineageDocumentKind::Incident => {
            parse_incident_document_with_source(&raw, &source_name).map(SurfaceDocument::Incident)
        }
    };
    match parsed {
        Ok(document) => Ok(Some(document)),
        Err(_error) => Ok(None),
    }
}

fn drift_reason(
    document: &SurfaceDocument,
    target: &ClosureTargetState,
    actual_root_spec_id: Option<&str>,
    aliases: &BTreeSet<String>,
) -> Option<LineageDiagnosticReason> {
    if document.root_idea_id() == Some(target.root_idea_id.as_str()) {
        return Some(LineageDiagnosticReason::SameRootIdeaDifferentRootSpec);
    }
    if actual_root_spec_id.is_some_and(|value| aliases.contains(value)) {
        return Some(LineageDiagnosticReason::KnownRootSpecAlias);
    }
    if document
        .document_spec_id()
        .is_some_and(|value| aliases.contains(value))
    {
        return Some(LineageDiagnosticReason::KnownRootSpecAlias);
    }
    None
}

fn task_repair_changes(
    finding: &LineageDriftFinding,
    target: &ClosureTargetState,
    aliases: &BTreeSet<String>,
) -> Vec<LineageRepairChange> {
    let mut changes = vec![LineageRepairChange {
        work_item_kind: finding.work_item_kind,
        work_item_id: finding.work_item_id.clone(),
        state: finding.state,
        path: finding.path.clone(),
        field_name: "root_spec_id".to_owned(),
        old_value: finding.actual_root_spec_id.clone(),
        new_value: target.root_spec_id.clone(),
    }];

    if finding.spec_id.as_ref().is_some_and(|spec_id| {
        finding.actual_root_spec_id.as_ref() == Some(spec_id) || aliases.contains(spec_id)
    }) {
        changes.push(LineageRepairChange {
            work_item_kind: finding.work_item_kind,
            work_item_id: finding.work_item_id.clone(),
            state: finding.state,
            path: finding.path.clone(),
            field_name: "spec_id".to_owned(),
            old_value: finding.spec_id.clone(),
            new_value: target.root_spec_id.clone(),
        });
    }

    changes
}

fn incident_repair_changes(
    finding: &LineageDriftFinding,
    target: &ClosureTargetState,
) -> Vec<LineageRepairChange> {
    vec![LineageRepairChange {
        work_item_kind: finding.work_item_kind,
        work_item_id: finding.work_item_id.clone(),
        state: finding.state,
        path: finding.path.clone(),
        field_name: "root_spec_id".to_owned(),
        old_value: finding.actual_root_spec_id.clone(),
        new_value: target.root_spec_id.clone(),
    }]
}

fn list_markdown_files(directory: &Path) -> LineageRepairResult<Vec<PathBuf>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(LineageRepairError::io(directory, error)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| LineageRepairError::io(directory, error))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("md") && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn count_markdown_files(directory: &Path) -> LineageRepairResult<u64> {
    Ok(list_markdown_files(directory)?.len() as u64)
}

fn lineage_repair_report_path(
    paths: &WorkspacePaths,
    created_at: &Timestamp,
) -> LineageRepairResult<PathBuf> {
    let timestamp = compact_timestamp(created_at)?;
    let counter = REPORT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let suffix = report_suffix(counter);
    Ok(paths
        .arbiter_dir
        .join("diagnostics")
        .join("lineage-repairs")
        .join(format!("{timestamp}-{suffix}.json")))
}

fn compact_timestamp(timestamp: &Timestamp) -> LineageRepairResult<String> {
    let parsed = OffsetDateTime::parse(timestamp.as_str(), &Rfc3339).map_err(|error| {
        LineageRepairError::Timestamp {
            message: error.to_string(),
        }
    })?;
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

fn report_suffix(counter: u64) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp_nanos() as u64;
    format!(
        "{:08x}",
        (now ^ counter ^ u64::from(process::id())) & 0xffff_ffff
    )
}

fn current_timestamp() -> LineageRepairResult<Timestamp> {
    let value = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| LineageRepairError::Timestamp {
            message: error.to_string(),
        })?;
    Timestamp::parse("timestamp", &value).map_err(|error| LineageRepairError::Timestamp {
        message: error.to_string(),
    })
}

fn workspace_relative_path(paths: &WorkspacePaths, path: &Path) -> String {
    path.strip_prefix(&paths.root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn default_schema_version() -> String {
    "1.0".to_owned()
}

fn default_lineage_drift_kind() -> String {
    "closure_lineage_drift_diagnostic".to_owned()
}

fn default_lineage_repair_plan_kind() -> String {
    "closure_lineage_repair_plan".to_owned()
}
