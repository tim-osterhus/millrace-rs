//! Filesystem-backed queue store for canonical work documents.

use std::{
    collections::BTreeSet,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    contracts::{
        IncidentDocument, LearningRequestDocument, SpecDocument, SpecSourceType, TaskDocument,
        WorkDocumentError, WorkItemKind,
    },
    work_documents::{
        parse_incident_document_with_source, parse_learning_request_document_with_source,
        parse_spec_document_with_source, parse_task_document_with_source, render_incident_document,
        render_learning_request_document, render_spec_document, render_task_document,
    },
};

use super::{WorkspaceError, WorkspacePaths, initialize_workspace};

/// Result type for queue store operations.
pub type QueueStoreResult<T> = Result<T, QueueStoreError>;

/// Filesystem queue store failures.
#[derive(Debug)]
pub enum QueueStoreError {
    /// Workspace initialization or path handling failed.
    Workspace(WorkspaceError),
    /// A filesystem operation failed.
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Underlying IO error message.
        message: String,
    },
    /// A work document failed contract parsing or validation.
    WorkDocument {
        /// Source path being parsed or written.
        path: PathBuf,
        /// Typed work-document contract error.
        source: WorkDocumentError,
    },
    /// Queue state is internally inconsistent for the requested transition.
    InvalidState {
        /// Human-readable state error.
        message: String,
    },
}

impl QueueStoreError {
    fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: error.to_string(),
        }
    }

    fn invalid_state(message: impl Into<String>) -> Self {
        Self::InvalidState {
            message: message.into(),
        }
    }

    fn work_document(path: impl Into<PathBuf>, source: WorkDocumentError) -> Self {
        Self::WorkDocument {
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for QueueStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(error) => write!(f, "{error}"),
            Self::Io { path, message } => {
                write!(f, "queue filesystem error at {}: {message}", path.display())
            }
            Self::WorkDocument { path, source } => {
                write!(f, "queue document error at {}: {source}", path.display())
            }
            Self::InvalidState { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for QueueStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::WorkDocument { source, .. } => Some(source),
            Self::Io { .. } | Self::InvalidState { .. } => None,
        }
    }
}

impl From<WorkspaceError> for QueueStoreError {
    fn from(value: WorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

/// Claimed queue ownership for one work item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueClaim {
    /// Kind of work item claimed.
    pub work_item_kind: WorkItemKind,
    /// Canonical work item id.
    pub work_item_id: String,
    /// Active path now owning the work item.
    pub path: PathBuf,
}

/// Stale active artifact detection result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleActiveState {
    /// True when stale or contradictory active state was detected.
    pub is_stale: bool,
    /// Deterministic reason codes.
    pub reasons: Vec<String>,
}

/// Read-only queue inspection entry for one canonical work document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueInspectionEntry {
    /// Kind of work item found.
    pub work_item_kind: WorkItemKind,
    /// Queue state directory the document was found in.
    pub work_item_state: String,
    /// Canonical work item id parsed from the document.
    pub work_item_id: String,
    /// Human-facing work item title.
    pub title: String,
    /// Canonical filesystem path to the work document.
    pub path: PathBuf,
}

/// Queue store facade rooted at one initialized workspace.
#[derive(Debug, Clone)]
pub struct QueueStore {
    /// Resolved workspace paths.
    pub paths: WorkspacePaths,
}

impl QueueStore {
    /// Initialize or open a queue store rooted at the provided workspace.
    pub fn new(root: impl AsRef<Path>) -> QueueStoreResult<Self> {
        let paths = initialize_workspace(root)?;
        Ok(Self { paths })
    }

    /// Build a queue store from already resolved workspace paths.
    #[must_use]
    pub fn from_paths(paths: WorkspacePaths) -> Self {
        Self { paths }
    }

    /// Enqueue a task document.
    pub fn enqueue_task(&self, document: &TaskDocument) -> QueueStoreResult<PathBuf> {
        enqueue_task(&self.paths, document)
    }

    /// Enqueue a spec document.
    pub fn enqueue_spec(&self, document: &SpecDocument) -> QueueStoreResult<PathBuf> {
        enqueue_spec(&self.paths, document)
    }

    /// Enqueue an incident document.
    pub fn enqueue_incident(&self, document: &IncidentDocument) -> QueueStoreResult<PathBuf> {
        enqueue_incident(&self.paths, document)
    }

    /// Enqueue a learning request document.
    pub fn enqueue_learning_request(
        &self,
        document: &LearningRequestDocument,
    ) -> QueueStoreResult<PathBuf> {
        enqueue_learning_request(&self.paths, document)
    }

    /// Claim the next eligible execution task.
    pub fn claim_next_execution_task(
        &self,
        root_spec_id: Option<&str>,
    ) -> QueueStoreResult<Option<QueueClaim>> {
        claim_next_execution_task(&self.paths, root_spec_id)
    }

    /// Claim the next planning item, preferring incidents before specs.
    pub fn claim_next_planning_item(
        &self,
        root_spec_id: Option<&str>,
    ) -> QueueStoreResult<Option<QueueClaim>> {
        claim_next_planning_item(&self.paths, root_spec_id)
    }

    /// Claim the next learning request.
    pub fn claim_next_learning_request(&self) -> QueueStoreResult<Option<QueueClaim>> {
        claim_next_learning_request(&self.paths)
    }

    /// Mark an active task done.
    pub fn mark_task_done(&self, task_id: &str) -> QueueStoreResult<PathBuf> {
        mark_task_done(&self.paths, task_id)
    }

    /// Mark an active task blocked.
    pub fn mark_task_blocked(&self, task_id: &str) -> QueueStoreResult<PathBuf> {
        mark_task_blocked(&self.paths, task_id)
    }

    /// Mark an active spec done.
    pub fn mark_spec_done(&self, spec_id: &str) -> QueueStoreResult<PathBuf> {
        mark_spec_done(&self.paths, spec_id)
    }

    /// Mark an active spec blocked.
    pub fn mark_spec_blocked(&self, spec_id: &str) -> QueueStoreResult<PathBuf> {
        mark_spec_blocked(&self.paths, spec_id)
    }

    /// Mark an active incident resolved.
    pub fn mark_incident_resolved(&self, incident_id: &str) -> QueueStoreResult<PathBuf> {
        mark_incident_resolved(&self.paths, incident_id)
    }

    /// Mark an active incident blocked.
    pub fn mark_incident_blocked(&self, incident_id: &str) -> QueueStoreResult<PathBuf> {
        mark_incident_blocked(&self.paths, incident_id)
    }

    /// Mark an active learning request done.
    pub fn mark_learning_request_done(
        &self,
        learning_request_id: &str,
    ) -> QueueStoreResult<PathBuf> {
        mark_learning_request_done(&self.paths, learning_request_id)
    }

    /// Mark an active learning request blocked.
    pub fn mark_learning_request_blocked(
        &self,
        learning_request_id: &str,
    ) -> QueueStoreResult<PathBuf> {
        mark_learning_request_blocked(&self.paths, learning_request_id)
    }

    /// Requeue an active task and record the reason.
    pub fn requeue_task(&self, task_id: &str, reason: &str) -> QueueStoreResult<PathBuf> {
        requeue_task(&self.paths, task_id, reason)
    }

    /// Requeue an active spec and record the reason.
    pub fn requeue_spec(&self, spec_id: &str, reason: &str) -> QueueStoreResult<PathBuf> {
        requeue_spec(&self.paths, spec_id, reason)
    }

    /// Requeue an active incident and record the reason.
    pub fn requeue_incident(&self, incident_id: &str, reason: &str) -> QueueStoreResult<PathBuf> {
        requeue_incident(&self.paths, incident_id, reason)
    }

    /// Requeue an active learning request and record the reason.
    pub fn requeue_learning_request(
        &self,
        learning_request_id: &str,
        reason: &str,
    ) -> QueueStoreResult<PathBuf> {
        requeue_learning_request(&self.paths, learning_request_id, reason)
    }

    /// Detect stale active execution queue state against a snapshot identity.
    pub fn detect_execution_stale_state(
        &self,
        snapshot_active_task_id: Option<&str>,
    ) -> QueueStoreResult<StaleActiveState> {
        detect_execution_stale_state(&self.paths, snapshot_active_task_id)
    }

    /// Detect stale active planning queue state against a snapshot identity.
    pub fn detect_planning_stale_state(
        &self,
        snapshot_active_kind: Option<WorkItemKind>,
        snapshot_active_item_id: Option<&str>,
    ) -> QueueStoreResult<StaleActiveState> {
        detect_planning_stale_state(&self.paths, snapshot_active_kind, snapshot_active_item_id)
    }

    /// Detect stale active learning queue state against a snapshot identity.
    pub fn detect_learning_stale_state(
        &self,
        snapshot_active_learning_request_id: Option<&str>,
    ) -> QueueStoreResult<StaleActiveState> {
        detect_learning_stale_state(&self.paths, snapshot_active_learning_request_id)
    }

    /// Inspect all canonical queue work documents without mutating queue state.
    pub fn inspect_work_items(&self) -> QueueStoreResult<Vec<QueueInspectionEntry>> {
        inspect_queue_items(&self.paths)
    }

    /// Locate and parse one canonical queue work document by id without mutating queue state.
    pub fn find_work_item(
        &self,
        work_item_id: &str,
    ) -> QueueStoreResult<Option<QueueInspectionEntry>> {
        find_queue_item(&self.paths, work_item_id)
    }
}

/// Inspect all canonical queue work documents without mutating queue state.
pub fn inspect_queue_items(paths: &WorkspacePaths) -> QueueStoreResult<Vec<QueueInspectionEntry>> {
    let mut entries = Vec::new();

    push_task_entries(&mut entries, &paths.tasks_queue_dir, "queue")?;
    push_task_entries(&mut entries, &paths.tasks_active_dir, "active")?;
    push_task_entries(&mut entries, &paths.tasks_done_dir, "done")?;
    push_task_entries(&mut entries, &paths.tasks_blocked_dir, "blocked")?;

    push_spec_entries(&mut entries, &paths.specs_queue_dir, "queue")?;
    push_spec_entries(&mut entries, &paths.specs_active_dir, "active")?;
    push_spec_entries(&mut entries, &paths.specs_done_dir, "done")?;
    push_spec_entries(&mut entries, &paths.specs_blocked_dir, "blocked")?;

    push_incident_entries(&mut entries, &paths.incidents_incoming_dir, "incoming")?;
    push_incident_entries(&mut entries, &paths.incidents_active_dir, "active")?;
    push_incident_entries(&mut entries, &paths.incidents_resolved_dir, "resolved")?;
    push_incident_entries(&mut entries, &paths.incidents_blocked_dir, "blocked")?;

    push_learning_request_entries(&mut entries, &paths.learning_requests_queue_dir, "queue")?;
    push_learning_request_entries(&mut entries, &paths.learning_requests_active_dir, "active")?;
    push_learning_request_entries(&mut entries, &paths.learning_requests_done_dir, "done")?;
    push_learning_request_entries(
        &mut entries,
        &paths.learning_requests_blocked_dir,
        "blocked",
    )?;

    entries.sort_by(|left, right| {
        (
            left.work_item_kind.as_str(),
            left.work_item_state.as_str(),
            left.work_item_id.as_str(),
        )
            .cmp(&(
                right.work_item_kind.as_str(),
                right.work_item_state.as_str(),
                right.work_item_id.as_str(),
            ))
    });
    Ok(entries)
}

/// Locate and parse one canonical queue work document by id without mutating queue state.
pub fn find_queue_item(
    paths: &WorkspacePaths,
    work_item_id: &str,
) -> QueueStoreResult<Option<QueueInspectionEntry>> {
    Ok(inspect_queue_items(paths)?
        .into_iter()
        .find(|entry| entry.work_item_id == work_item_id))
}

/// Enqueue a task document in the execution queue.
pub fn enqueue_task(paths: &WorkspacePaths, document: &TaskDocument) -> QueueStoreResult<PathBuf> {
    document.validate().map_err(|source| {
        QueueStoreError::work_document(task_path(&paths.tasks_queue_dir, &document.task_id), source)
    })?;
    ensure_unique_id(
        &document.task_id,
        &[
            &paths.tasks_queue_dir,
            &paths.tasks_active_dir,
            &paths.tasks_done_dir,
            &paths.tasks_blocked_dir,
        ],
        WorkItemKind::Task,
    )?;
    let destination = task_path(&paths.tasks_queue_dir, &document.task_id);
    write_document(&destination, &render_task_document(document))?;
    Ok(destination)
}

/// Enqueue a spec document in the planning spec queue.
pub fn enqueue_spec(paths: &WorkspacePaths, document: &SpecDocument) -> QueueStoreResult<PathBuf> {
    document.validate().map_err(|source| {
        QueueStoreError::work_document(spec_path(&paths.specs_queue_dir, &document.spec_id), source)
    })?;
    ensure_unique_id(
        &document.spec_id,
        &[
            &paths.specs_queue_dir,
            &paths.specs_active_dir,
            &paths.specs_done_dir,
            &paths.specs_blocked_dir,
        ],
        WorkItemKind::Spec,
    )?;
    let destination = spec_path(&paths.specs_queue_dir, &document.spec_id);
    write_document(&destination, &render_spec_document(document))?;
    Ok(destination)
}

/// Enqueue an incident document in the planning incoming queue.
pub fn enqueue_incident(
    paths: &WorkspacePaths,
    document: &IncidentDocument,
) -> QueueStoreResult<PathBuf> {
    document.validate().map_err(|source| {
        QueueStoreError::work_document(
            incident_path(&paths.incidents_incoming_dir, &document.incident_id),
            source,
        )
    })?;
    ensure_unique_id(
        &document.incident_id,
        &[
            &paths.incidents_incoming_dir,
            &paths.incidents_active_dir,
            &paths.incidents_resolved_dir,
            &paths.incidents_blocked_dir,
        ],
        WorkItemKind::Incident,
    )?;
    let destination = incident_path(&paths.incidents_incoming_dir, &document.incident_id);
    write_document(&destination, &render_incident_document(document))?;
    Ok(destination)
}

/// Enqueue a learning request document in the learning queue.
pub fn enqueue_learning_request(
    paths: &WorkspacePaths,
    document: &LearningRequestDocument,
) -> QueueStoreResult<PathBuf> {
    document.validate().map_err(|source| {
        QueueStoreError::work_document(
            learning_request_path(
                &paths.learning_requests_queue_dir,
                &document.learning_request_id,
            ),
            source,
        )
    })?;
    ensure_unique_id(
        &document.learning_request_id,
        &[
            &paths.learning_requests_queue_dir,
            &paths.learning_requests_active_dir,
            &paths.learning_requests_done_dir,
            &paths.learning_requests_blocked_dir,
        ],
        WorkItemKind::LearningRequest,
    )?;
    let destination = learning_request_path(
        &paths.learning_requests_queue_dir,
        &document.learning_request_id,
    );
    write_document(&destination, &render_learning_request_document(document))?;
    Ok(destination)
}

/// Claim the oldest eligible execution task, respecting task dependencies.
pub fn claim_next_execution_task(
    paths: &WorkspacePaths,
    root_spec_id: Option<&str>,
) -> QueueStoreResult<Option<QueueClaim>> {
    let active = list_markdown_files(&paths.tasks_active_dir)?;
    if active.len() > 1 {
        return Err(QueueStoreError::invalid_state(
            "multiple active execution tasks found",
        ));
    }
    if !active.is_empty() {
        return Ok(None);
    }

    loop {
        let Some((task_id, source)) = select_oldest_eligible_task(paths, root_spec_id)? else {
            return Ok(None);
        };
        let destination = paths
            .tasks_active_dir
            .join(source.file_name().ok_or_else(|| {
                QueueStoreError::invalid_state("queued task path is missing a filename")
            })?);
        match fs::rename(&source, &destination) {
            Ok(()) => {
                return Ok(Some(QueueClaim {
                    work_item_kind: WorkItemKind::Task,
                    work_item_id: task_id,
                    path: destination,
                }));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(QueueStoreError::io(&source, error)),
        }
    }
}

/// Claim the oldest planning incident, or the oldest spec when no incident is eligible.
pub fn claim_next_planning_item(
    paths: &WorkspacePaths,
    root_spec_id: Option<&str>,
) -> QueueStoreResult<Option<QueueClaim>> {
    let active_specs = list_markdown_files(&paths.specs_active_dir)?;
    let active_incidents = list_markdown_files(&paths.incidents_active_dir)?;
    if active_specs.len() + active_incidents.len() > 1 {
        return Err(QueueStoreError::invalid_state(
            "multiple active planning items found",
        ));
    }
    if !active_specs.is_empty() || !active_incidents.is_empty() {
        return Ok(None);
    }

    loop {
        if let Some((incident_id, source)) =
            select_oldest_incident(&paths.incidents_incoming_dir, root_spec_id)?
        {
            let destination = paths
                .incidents_active_dir
                .join(source.file_name().ok_or_else(|| {
                    QueueStoreError::invalid_state("queued incident path is missing a filename")
                })?);
            match fs::rename(&source, &destination) {
                Ok(()) => {
                    return Ok(Some(QueueClaim {
                        work_item_kind: WorkItemKind::Incident,
                        work_item_id: incident_id,
                        path: destination,
                    }));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(QueueStoreError::io(&source, error)),
            }
        }

        let Some((spec_id, source)) = select_oldest_spec(&paths.specs_queue_dir, root_spec_id)?
        else {
            return Ok(None);
        };
        let destination = paths
            .specs_active_dir
            .join(source.file_name().ok_or_else(|| {
                QueueStoreError::invalid_state("queued spec path is missing a filename")
            })?);
        match fs::rename(&source, &destination) {
            Ok(()) => {
                return Ok(Some(QueueClaim {
                    work_item_kind: WorkItemKind::Spec,
                    work_item_id: spec_id,
                    path: destination,
                }));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(QueueStoreError::io(&source, error)),
        }
    }
}

/// Claim the oldest learning request.
pub fn claim_next_learning_request(paths: &WorkspacePaths) -> QueueStoreResult<Option<QueueClaim>> {
    let active = list_markdown_files(&paths.learning_requests_active_dir)?;
    if active.len() > 1 {
        return Err(QueueStoreError::invalid_state(
            "multiple active learning requests found",
        ));
    }
    if !active.is_empty() {
        return Ok(None);
    }

    loop {
        let Some((learning_request_id, source)) =
            select_oldest_learning_request(&paths.learning_requests_queue_dir)?
        else {
            return Ok(None);
        };
        let destination = paths
            .learning_requests_active_dir
            .join(source.file_name().ok_or_else(|| {
                QueueStoreError::invalid_state("queued learning request path is missing a filename")
            })?);
        match fs::rename(&source, &destination) {
            Ok(()) => {
                return Ok(Some(QueueClaim {
                    work_item_kind: WorkItemKind::LearningRequest,
                    work_item_id: learning_request_id,
                    path: destination,
                }));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(QueueStoreError::io(&source, error)),
        }
    }
}

/// Mark an active task done.
pub fn mark_task_done(paths: &WorkspacePaths, task_id: &str) -> QueueStoreResult<PathBuf> {
    move_item(
        &paths.tasks_active_dir,
        &paths.tasks_done_dir,
        task_id,
        WorkItemKind::Task,
    )
}

/// Mark an active task blocked.
pub fn mark_task_blocked(paths: &WorkspacePaths, task_id: &str) -> QueueStoreResult<PathBuf> {
    move_item(
        &paths.tasks_active_dir,
        &paths.tasks_blocked_dir,
        task_id,
        WorkItemKind::Task,
    )
}

/// Mark an active spec done.
pub fn mark_spec_done(paths: &WorkspacePaths, spec_id: &str) -> QueueStoreResult<PathBuf> {
    move_item(
        &paths.specs_active_dir,
        &paths.specs_done_dir,
        spec_id,
        WorkItemKind::Spec,
    )
}

/// Mark an active spec blocked.
pub fn mark_spec_blocked(paths: &WorkspacePaths, spec_id: &str) -> QueueStoreResult<PathBuf> {
    move_item(
        &paths.specs_active_dir,
        &paths.specs_blocked_dir,
        spec_id,
        WorkItemKind::Spec,
    )
}

/// Mark an active incident resolved.
pub fn mark_incident_resolved(
    paths: &WorkspacePaths,
    incident_id: &str,
) -> QueueStoreResult<PathBuf> {
    move_item(
        &paths.incidents_active_dir,
        &paths.incidents_resolved_dir,
        incident_id,
        WorkItemKind::Incident,
    )
}

/// Mark an active incident blocked.
pub fn mark_incident_blocked(
    paths: &WorkspacePaths,
    incident_id: &str,
) -> QueueStoreResult<PathBuf> {
    move_item(
        &paths.incidents_active_dir,
        &paths.incidents_blocked_dir,
        incident_id,
        WorkItemKind::Incident,
    )
}

/// Mark an active learning request done.
pub fn mark_learning_request_done(
    paths: &WorkspacePaths,
    learning_request_id: &str,
) -> QueueStoreResult<PathBuf> {
    move_item(
        &paths.learning_requests_active_dir,
        &paths.learning_requests_done_dir,
        learning_request_id,
        WorkItemKind::LearningRequest,
    )
}

/// Mark an active learning request blocked.
pub fn mark_learning_request_blocked(
    paths: &WorkspacePaths,
    learning_request_id: &str,
) -> QueueStoreResult<PathBuf> {
    move_item(
        &paths.learning_requests_active_dir,
        &paths.learning_requests_blocked_dir,
        learning_request_id,
        WorkItemKind::LearningRequest,
    )
}

/// Move an active task back to the execution queue and record the reason.
pub fn requeue_task(
    paths: &WorkspacePaths,
    task_id: &str,
    reason: &str,
) -> QueueStoreResult<PathBuf> {
    let destination = move_item(
        &paths.tasks_active_dir,
        &paths.tasks_queue_dir,
        task_id,
        WorkItemKind::Task,
    )?;
    append_requeue_reason(&paths.tasks_queue_dir, task_id, WorkItemKind::Task, reason)?;
    Ok(destination)
}

/// Move an active spec back to the planning queue and record the reason.
pub fn requeue_spec(
    paths: &WorkspacePaths,
    spec_id: &str,
    reason: &str,
) -> QueueStoreResult<PathBuf> {
    let destination = move_item(
        &paths.specs_active_dir,
        &paths.specs_queue_dir,
        spec_id,
        WorkItemKind::Spec,
    )?;
    append_requeue_reason(&paths.specs_queue_dir, spec_id, WorkItemKind::Spec, reason)?;
    Ok(destination)
}

/// Move an active incident back to the incoming incident queue and record the reason.
pub fn requeue_incident(
    paths: &WorkspacePaths,
    incident_id: &str,
    reason: &str,
) -> QueueStoreResult<PathBuf> {
    let destination = move_item(
        &paths.incidents_active_dir,
        &paths.incidents_incoming_dir,
        incident_id,
        WorkItemKind::Incident,
    )?;
    append_requeue_reason(
        &paths.incidents_incoming_dir,
        incident_id,
        WorkItemKind::Incident,
        reason,
    )?;
    Ok(destination)
}

/// Move an active learning request back to the learning queue and record the reason.
pub fn requeue_learning_request(
    paths: &WorkspacePaths,
    learning_request_id: &str,
    reason: &str,
) -> QueueStoreResult<PathBuf> {
    let destination = move_item(
        &paths.learning_requests_active_dir,
        &paths.learning_requests_queue_dir,
        learning_request_id,
        WorkItemKind::LearningRequest,
    )?;
    append_requeue_reason(
        &paths.learning_requests_queue_dir,
        learning_request_id,
        WorkItemKind::LearningRequest,
        reason,
    )?;
    Ok(destination)
}

/// Detect stale execution active state.
pub fn detect_execution_stale_state(
    paths: &WorkspacePaths,
    snapshot_active_task_id: Option<&str>,
) -> QueueStoreResult<StaleActiveState> {
    let active_ids = ids_in_directory(&paths.tasks_active_dir)?;
    let queued_ids = ids_in_directory(&paths.tasks_queue_dir)?;
    let mut reasons = Vec::new();

    if active_ids.len() > 1 {
        reasons.push("multiple_active_items");
    }
    if !active_ids.is_empty() && snapshot_active_task_id.is_none() {
        reasons.push("active_without_snapshot");
    }
    if let Some(snapshot_active_task_id) = snapshot_active_task_id {
        if queued_ids.contains(&snapshot_active_task_id.to_owned()) {
            reasons.push("snapshot_points_to_queued_item");
        }
        if active_ids.is_empty() {
            reasons.push("snapshot_without_active_artifact");
        } else if active_ids.len() == 1 && active_ids[0] != snapshot_active_task_id {
            reasons.push("snapshot_active_id_mismatch");
        }
    }

    Ok(stale_state(reasons))
}

/// Detect stale planning active state.
pub fn detect_planning_stale_state(
    paths: &WorkspacePaths,
    snapshot_active_kind: Option<WorkItemKind>,
    snapshot_active_item_id: Option<&str>,
) -> QueueStoreResult<StaleActiveState> {
    if snapshot_active_kind.is_some() != snapshot_active_item_id.is_some() {
        return Err(QueueStoreError::invalid_state(
            "snapshot_active_kind and snapshot_active_item_id must be set together",
        ));
    }
    if let Some(kind) = snapshot_active_kind {
        if !matches!(kind, WorkItemKind::Spec | WorkItemKind::Incident) {
            return Err(QueueStoreError::invalid_state(
                "planning stale-state checks only support spec and incident kinds",
            ));
        }
    }

    let mut active_items = Vec::new();
    for item_id in ids_in_directory(&paths.specs_active_dir)? {
        active_items.push((WorkItemKind::Spec, item_id));
    }
    for item_id in ids_in_directory(&paths.incidents_active_dir)? {
        active_items.push((WorkItemKind::Incident, item_id));
    }

    let mut reasons = Vec::new();
    if active_items.len() > 1 {
        reasons.push("multiple_active_items");
    }
    if !active_items.is_empty() && snapshot_active_item_id.is_none() {
        reasons.push("active_without_snapshot");
    }
    if let (Some(kind), Some(item_id)) = (snapshot_active_kind, snapshot_active_item_id) {
        let queued_ids = match kind {
            WorkItemKind::Spec => ids_in_directory(&paths.specs_queue_dir)?,
            WorkItemKind::Incident => ids_in_directory(&paths.incidents_incoming_dir)?,
            WorkItemKind::Task | WorkItemKind::LearningRequest => Vec::new(),
        };
        if queued_ids.contains(&item_id.to_owned()) {
            reasons.push("snapshot_points_to_queued_item");
        }
        if active_items.is_empty() {
            reasons.push("snapshot_without_active_artifact");
        } else if active_items.len() == 1 {
            let (active_kind, active_id) = &active_items[0];
            if *active_kind != kind || active_id != item_id {
                reasons.push("snapshot_active_id_mismatch");
            }
        }
    }

    Ok(stale_state(reasons))
}

/// Detect stale learning active state.
pub fn detect_learning_stale_state(
    paths: &WorkspacePaths,
    snapshot_active_learning_request_id: Option<&str>,
) -> QueueStoreResult<StaleActiveState> {
    let active_ids = ids_in_directory(&paths.learning_requests_active_dir)?;
    let queued_ids = ids_in_directory(&paths.learning_requests_queue_dir)?;
    let mut reasons = Vec::new();

    if active_ids.len() > 1 {
        reasons.push("multiple_active_items");
    }
    if !active_ids.is_empty() && snapshot_active_learning_request_id.is_none() {
        reasons.push("active_without_snapshot");
    }
    if let Some(snapshot_active_learning_request_id) = snapshot_active_learning_request_id {
        if queued_ids.contains(&snapshot_active_learning_request_id.to_owned()) {
            reasons.push("snapshot_points_to_queued_item");
        }
        if active_ids.is_empty() {
            reasons.push("snapshot_without_active_artifact");
        } else if active_ids.len() == 1 && active_ids[0] != snapshot_active_learning_request_id {
            reasons.push("snapshot_active_id_mismatch");
        }
    }

    Ok(stale_state(reasons))
}

fn select_oldest_eligible_task(
    paths: &WorkspacePaths,
    root_spec_id: Option<&str>,
) -> QueueStoreResult<Option<(String, PathBuf)>> {
    let completed_task_ids: BTreeSet<String> = ids_in_directory(&paths.tasks_done_dir)?
        .into_iter()
        .collect();
    let mut candidates = Vec::new();
    for path in list_markdown_files(&paths.tasks_queue_dir)? {
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(QueueStoreError::io(&path, error)),
        };
        let document = match parse_task_document_with_source(&raw, &path.display().to_string()) {
            Ok(document) => document,
            Err(error) => {
                quarantine_invalid_artifact(&paths.tasks_queue_dir, &path, &error.to_string())?;
                continue;
            }
        };
        if path_stem(&path)? != document.task_id {
            quarantine_invalid_artifact(
                &paths.tasks_queue_dir,
                &path,
                &format!(
                    "filename stem does not match task_id: expected {}, found {}",
                    document.task_id,
                    path_stem(&path)?
                ),
            )?;
            continue;
        }
        if root_spec_id
            .is_some_and(|expected| effective_root_spec_task(&document) != Some(expected))
        {
            continue;
        }
        if !document
            .depends_on
            .iter()
            .all(|dependency| completed_task_ids.contains(dependency))
        {
            continue;
        }
        candidates.push((
            document.created_at.as_str().to_owned(),
            document.task_id.clone(),
            path,
        ));
    }
    candidates.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    Ok(candidates
        .into_iter()
        .next()
        .map(|(_timestamp, item_id, path)| (item_id, path)))
}

fn push_task_entries(
    entries: &mut Vec<QueueInspectionEntry>,
    directory: &Path,
    state: &str,
) -> QueueStoreResult<()> {
    for path in list_markdown_files(directory)? {
        let raw = fs::read_to_string(&path).map_err(|error| QueueStoreError::io(&path, error))?;
        let document = parse_task_document_with_source(&raw, &path.display().to_string())
            .map_err(|error| QueueStoreError::work_document(&path, error))?;
        ensure_filename_matches(&path, "task_id", &document.task_id)?;
        entries.push(QueueInspectionEntry {
            work_item_kind: WorkItemKind::Task,
            work_item_state: state.to_owned(),
            work_item_id: document.task_id,
            title: document.title,
            path,
        });
    }
    Ok(())
}

fn push_spec_entries(
    entries: &mut Vec<QueueInspectionEntry>,
    directory: &Path,
    state: &str,
) -> QueueStoreResult<()> {
    for path in list_markdown_files(directory)? {
        let raw = fs::read_to_string(&path).map_err(|error| QueueStoreError::io(&path, error))?;
        let document = parse_spec_document_with_source(&raw, &path.display().to_string())
            .map_err(|error| QueueStoreError::work_document(&path, error))?;
        ensure_filename_matches(&path, "spec_id", &document.spec_id)?;
        entries.push(QueueInspectionEntry {
            work_item_kind: WorkItemKind::Spec,
            work_item_state: state.to_owned(),
            work_item_id: document.spec_id,
            title: document.title,
            path,
        });
    }
    Ok(())
}

fn push_incident_entries(
    entries: &mut Vec<QueueInspectionEntry>,
    directory: &Path,
    state: &str,
) -> QueueStoreResult<()> {
    for path in list_markdown_files(directory)? {
        let raw = fs::read_to_string(&path).map_err(|error| QueueStoreError::io(&path, error))?;
        let document = parse_incident_document_with_source(&raw, &path.display().to_string())
            .map_err(|error| QueueStoreError::work_document(&path, error))?;
        ensure_filename_matches(&path, "incident_id", &document.incident_id)?;
        entries.push(QueueInspectionEntry {
            work_item_kind: WorkItemKind::Incident,
            work_item_state: state.to_owned(),
            work_item_id: document.incident_id,
            title: document.title,
            path,
        });
    }
    Ok(())
}

fn push_learning_request_entries(
    entries: &mut Vec<QueueInspectionEntry>,
    directory: &Path,
    state: &str,
) -> QueueStoreResult<()> {
    for path in list_markdown_files(directory)? {
        let raw = fs::read_to_string(&path).map_err(|error| QueueStoreError::io(&path, error))?;
        let document =
            parse_learning_request_document_with_source(&raw, &path.display().to_string())
                .map_err(|error| QueueStoreError::work_document(&path, error))?;
        ensure_filename_matches(&path, "learning_request_id", &document.learning_request_id)?;
        entries.push(QueueInspectionEntry {
            work_item_kind: WorkItemKind::LearningRequest,
            work_item_state: state.to_owned(),
            work_item_id: document.learning_request_id,
            title: document.title,
            path,
        });
    }
    Ok(())
}

fn ensure_filename_matches(
    path: &Path,
    field_name: &str,
    document_id: &str,
) -> QueueStoreResult<()> {
    let filename_id = path_stem(path)?;
    if filename_id == document_id {
        return Ok(());
    }
    Err(QueueStoreError::invalid_state(format!(
        "filename stem does not match {field_name}: expected {document_id}, found {filename_id}"
    )))
}

fn select_oldest_spec(
    directory: &Path,
    root_spec_id: Option<&str>,
) -> QueueStoreResult<Option<(String, PathBuf)>> {
    let mut candidates = Vec::new();
    for path in list_markdown_files(directory)? {
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(QueueStoreError::io(&path, error)),
        };
        let document = match parse_spec_document_with_source(&raw, &path.display().to_string()) {
            Ok(document) => document,
            Err(error) => {
                quarantine_invalid_artifact(directory, &path, &error.to_string())?;
                continue;
            }
        };
        if path_stem(&path)? != document.spec_id {
            quarantine_invalid_artifact(
                directory,
                &path,
                &format!(
                    "filename stem does not match spec_id: expected {}, found {}",
                    document.spec_id,
                    path_stem(&path)?
                ),
            )?;
            continue;
        }
        if root_spec_id
            .is_some_and(|expected| effective_root_spec_spec(&document) != Some(expected))
        {
            continue;
        }
        candidates.push((
            document.created_at.as_str().to_owned(),
            document.spec_id.clone(),
            path,
        ));
    }
    candidates.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    Ok(candidates
        .into_iter()
        .next()
        .map(|(_timestamp, item_id, path)| (item_id, path)))
}

fn select_oldest_incident(
    directory: &Path,
    root_spec_id: Option<&str>,
) -> QueueStoreResult<Option<(String, PathBuf)>> {
    let mut candidates = Vec::new();
    for path in list_markdown_files(directory)? {
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(QueueStoreError::io(&path, error)),
        };
        let document = match parse_incident_document_with_source(&raw, &path.display().to_string())
        {
            Ok(document) => document,
            Err(error) => {
                quarantine_invalid_artifact(directory, &path, &error.to_string())?;
                continue;
            }
        };
        if path_stem(&path)? != document.incident_id {
            quarantine_invalid_artifact(
                directory,
                &path,
                &format!(
                    "filename stem does not match incident_id: expected {}, found {}",
                    document.incident_id,
                    path_stem(&path)?
                ),
            )?;
            continue;
        }
        if root_spec_id
            .is_some_and(|expected| effective_root_spec_incident(&document) != Some(expected))
        {
            continue;
        }
        candidates.push((
            document.opened_at.as_str().to_owned(),
            document.incident_id.clone(),
            path,
        ));
    }
    candidates.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    Ok(candidates
        .into_iter()
        .next()
        .map(|(_timestamp, item_id, path)| (item_id, path)))
}

fn select_oldest_learning_request(directory: &Path) -> QueueStoreResult<Option<(String, PathBuf)>> {
    let mut candidates = Vec::new();
    for path in list_markdown_files(directory)? {
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(QueueStoreError::io(&path, error)),
        };
        let document =
            match parse_learning_request_document_with_source(&raw, &path.display().to_string()) {
                Ok(document) => document,
                Err(error) => {
                    quarantine_invalid_artifact(directory, &path, &error.to_string())?;
                    continue;
                }
            };
        if path_stem(&path)? != document.learning_request_id {
            quarantine_invalid_artifact(
                directory,
                &path,
                &format!(
                    "filename stem does not match learning_request_id: expected {}, found {}",
                    document.learning_request_id,
                    path_stem(&path)?
                ),
            )?;
            continue;
        }
        candidates.push((
            document.created_at.as_str().to_owned(),
            document.learning_request_id.clone(),
            path,
        ));
    }
    candidates.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    Ok(candidates
        .into_iter()
        .next()
        .map(|(_timestamp, item_id, path)| (item_id, path)))
}

fn move_item(
    source_dir: &Path,
    destination_dir: &Path,
    item_id: &str,
    kind: WorkItemKind,
) -> QueueStoreResult<PathBuf> {
    let source = source_dir.join(format!("{item_id}.md"));
    if !source.exists() {
        return Err(QueueStoreError::invalid_state(format!(
            "{} {item_id} is not active",
            kind.as_str()
        )));
    }
    fs::create_dir_all(destination_dir)
        .map_err(|error| QueueStoreError::io(destination_dir, error))?;
    let destination =
        destination_dir.join(source.file_name().ok_or_else(|| {
            QueueStoreError::invalid_state("active item path is missing a filename")
        })?);
    if destination.exists() {
        return Err(QueueStoreError::invalid_state(format!(
            "{} {item_id} already exists at destination",
            kind.as_str()
        )));
    }
    fs::rename(&source, &destination).map_err(|error| QueueStoreError::io(&source, error))?;
    Ok(destination)
}

fn ensure_unique_id(
    item_id: &str,
    directories: &[&PathBuf],
    kind: WorkItemKind,
) -> QueueStoreResult<()> {
    let filename = format!("{item_id}.md");
    for directory in directories {
        if directory.join(&filename).exists() {
            return Err(QueueStoreError::invalid_state(format!(
                "{} {item_id} already exists",
                kind.as_str()
            )));
        }
    }
    Ok(())
}

fn write_document(destination: &Path, raw: &str) -> QueueStoreResult<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| QueueStoreError::io(parent, error))?;
    }
    fs::write(destination, raw).map_err(|error| QueueStoreError::io(destination, error))
}

fn append_requeue_reason(
    destination_dir: &Path,
    item_id: &str,
    kind: WorkItemKind,
    reason: &str,
) -> QueueStoreResult<()> {
    let cleaned = reason.trim();
    if cleaned.is_empty() {
        return Err(QueueStoreError::invalid_state("requeue reason is required"));
    }
    fs::create_dir_all(destination_dir)
        .map_err(|error| QueueStoreError::io(destination_dir, error))?;
    let log_path = destination_dir.join(format!("{item_id}.requeue.jsonl"));
    let payload = serde_json::json!({
        "at": current_timestamp(),
        "kind": kind.as_str(),
        "reason": cleaned,
    });
    let line = serde_json::to_string(&payload).map_err(|error| QueueStoreError::InvalidState {
        message: error.to_string(),
    })? + "\n";
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&log_path)
        .map_err(|error| QueueStoreError::io(&log_path, error))?;
    file.write_all(line.as_bytes())
        .map_err(|error| QueueStoreError::io(&log_path, error))
}

fn quarantine_invalid_artifact(
    directory: &Path,
    source_path: &Path,
    error: &str,
) -> QueueStoreResult<()> {
    let mut destination = source_path.with_extension(format!(
        "{}.invalid",
        source_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
    ));
    let mut suffix_index = 1;
    while destination.exists() {
        destination = source_path.with_extension(format!(
            "{}.invalid.{suffix_index}",
            source_path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
        ));
        suffix_index += 1;
    }
    match fs::rename(source_path, &destination) {
        Ok(()) => {}
        Err(rename_error) if rename_error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(rename_error) => return Err(QueueStoreError::io(source_path, rename_error)),
    }

    let log_path = directory.join("invalid-artifacts.jsonl");
    let payload = serde_json::json!({
        "at": current_timestamp(),
        "error": error,
        "quarantine_name": destination.file_name().and_then(|value| value.to_str()).unwrap_or_default(),
        "source_name": source_path.file_name().and_then(|value| value.to_str()).unwrap_or_default(),
    });
    let line = serde_json::to_string(&payload).map_err(|error| QueueStoreError::InvalidState {
        message: error.to_string(),
    })? + "\n";
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&log_path)
        .map_err(|error| QueueStoreError::io(&log_path, error))?;
    file.write_all(line.as_bytes())
        .map_err(|error| QueueStoreError::io(&log_path, error))
}

fn list_markdown_files(directory: &Path) -> QueueStoreResult<Vec<PathBuf>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| QueueStoreError::io(directory, error))? {
        let entry = entry.map_err(|error| QueueStoreError::io(directory, error))?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("md") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn ids_in_directory(directory: &Path) -> QueueStoreResult<Vec<String>> {
    list_markdown_files(directory)?
        .into_iter()
        .map(|path| path_stem(&path))
        .collect()
}

fn path_stem(path: &Path) -> QueueStoreResult<String> {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            QueueStoreError::invalid_state(format!(
                "path has no UTF-8 filename stem: {}",
                path.display()
            ))
        })
}

fn stale_state(reasons: Vec<&'static str>) -> StaleActiveState {
    let reasons: BTreeSet<String> = reasons.into_iter().map(str::to_owned).collect();
    StaleActiveState {
        is_stale: !reasons.is_empty(),
        reasons: reasons.into_iter().collect(),
    }
}

fn task_path(directory: &Path, task_id: &str) -> PathBuf {
    directory.join(format!("{task_id}.md"))
}

fn spec_path(directory: &Path, spec_id: &str) -> PathBuf {
    directory.join(format!("{spec_id}.md"))
}

fn incident_path(directory: &Path, incident_id: &str) -> PathBuf {
    directory.join(format!("{incident_id}.md"))
}

fn learning_request_path(directory: &Path, learning_request_id: &str) -> PathBuf {
    directory.join(format!("{learning_request_id}.md"))
}

fn effective_root_spec_task(document: &TaskDocument) -> Option<&str> {
    document
        .root_spec_id
        .as_deref()
        .or(document.spec_id.as_deref())
}

fn effective_root_spec_spec(document: &SpecDocument) -> Option<&str> {
    document.root_spec_id.as_deref().or_else(|| {
        if matches!(
            document.source_type,
            SpecSourceType::Idea | SpecSourceType::Manual
        ) {
            Some(document.spec_id.as_str())
        } else {
            None
        }
    })
}

fn effective_root_spec_incident(document: &IncidentDocument) -> Option<&str> {
    document
        .root_spec_id
        .as_deref()
        .or(document.source_spec_id.as_deref())
}

fn current_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}
