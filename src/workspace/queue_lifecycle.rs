//! Queue lifecycle interpreter for built-in and compiled work-item families.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::json;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::contracts::{
    WorkItemFamilyDefinition, WorkItemKind, coerce_family_and_kind, family_id_for_work_item_kind,
    validate_safe_identifier,
};

use super::{
    QueueStore, QueueStoreError, QueueStoreResult, WorkspacePaths, approve_active_blueprint_draft,
    block_active_blueprint_draft, requeue_active_blueprint_draft,
};

/// Runtime-owned lifecycle action selected from compiled authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceLifecycleAction {
    /// Move the active source to its done/resolved/approved lifecycle state.
    Complete,
    /// Move the active source to its blocked lifecycle state.
    Block,
    /// Move the active source back to its claimable queue state.
    Requeue,
}

impl SourceLifecycleAction {
    /// Stable lifecycle action id.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Block => "block",
            Self::Requeue => "requeue",
        }
    }
}

/// Source lifecycle intent produced by runtime-owned terminal/effect authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLifecycleIntent {
    pub lifecycle_plan_id: String,
    pub action: SourceLifecycleAction,
    pub work_item_family_id: Option<String>,
    pub work_item_kind: Option<WorkItemKind>,
    pub work_item_id: String,
    pub reason: Option<String>,
}

impl SourceLifecycleIntent {
    /// Build a lifecycle intent for a built-in work-item kind.
    #[must_use]
    pub fn for_builtin(
        lifecycle_plan_id: impl Into<String>,
        action: SourceLifecycleAction,
        work_item_kind: WorkItemKind,
        work_item_id: impl Into<String>,
    ) -> Self {
        Self {
            lifecycle_plan_id: lifecycle_plan_id.into(),
            action,
            work_item_family_id: Some(family_id_for_work_item_kind(work_item_kind).to_owned()),
            work_item_kind: Some(work_item_kind),
            work_item_id: work_item_id.into(),
            reason: None,
        }
    }

    /// Attach a requeue or audit reason.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// Applies lifecycle intents to workspace queue state.
#[derive(Debug, Clone)]
pub struct QueueLifecycleInterpreter {
    paths: WorkspacePaths,
    work_item_families: Vec<WorkItemFamilyDefinition>,
}

impl QueueLifecycleInterpreter {
    /// Build an interpreter from resolved paths and optional compiled families.
    #[must_use]
    pub fn new(paths: WorkspacePaths, work_item_families: Vec<WorkItemFamilyDefinition>) -> Self {
        Self {
            paths,
            work_item_families,
        }
    }

    /// Apply one source lifecycle intent.
    pub fn apply(&self, intent: &SourceLifecycleIntent) -> QueueStoreResult<PathBuf> {
        validate_intent(intent)?;
        let (family_id, kind) =
            coerce_family_and_kind(intent.work_item_family_id.as_deref(), intent.work_item_kind)
                .map_err(|error| QueueStoreError::InvalidState {
                    message: error.to_string(),
                })?;
        if let Some(kind) = kind {
            return self.apply_builtin(kind, intent);
        }
        let family_id = family_id.ok_or_else(|| QueueStoreError::InvalidState {
            message: "source lifecycle intent requires work_item_family_id or work_item_kind"
                .to_owned(),
        })?;
        self.apply_generic_family(&family_id, intent)
    }

    fn apply_builtin(
        &self,
        kind: WorkItemKind,
        intent: &SourceLifecycleIntent,
    ) -> QueueStoreResult<PathBuf> {
        let queue = QueueStore::from_paths(self.paths.clone());
        match (kind, intent.action) {
            (WorkItemKind::Task, SourceLifecycleAction::Complete) => {
                queue.mark_task_done(&intent.work_item_id)
            }
            (WorkItemKind::Task, SourceLifecycleAction::Block) => {
                queue.mark_task_blocked(&intent.work_item_id)
            }
            (WorkItemKind::Task, SourceLifecycleAction::Requeue) => {
                queue.requeue_task(&intent.work_item_id, required_reason(intent)?)
            }
            (WorkItemKind::Probe, SourceLifecycleAction::Complete) => {
                queue.mark_probe_done(&intent.work_item_id)
            }
            (WorkItemKind::Probe, SourceLifecycleAction::Block) => {
                queue.mark_probe_blocked(&intent.work_item_id)
            }
            (WorkItemKind::Probe, SourceLifecycleAction::Requeue) => {
                queue.requeue_probe(&intent.work_item_id, required_reason(intent)?)
            }
            (WorkItemKind::Spec, SourceLifecycleAction::Complete) => {
                queue.mark_spec_done(&intent.work_item_id)
            }
            (WorkItemKind::Spec, SourceLifecycleAction::Block) => {
                queue.mark_spec_blocked(&intent.work_item_id)
            }
            (WorkItemKind::Spec, SourceLifecycleAction::Requeue) => {
                queue.requeue_spec(&intent.work_item_id, required_reason(intent)?)
            }
            (WorkItemKind::Incident, SourceLifecycleAction::Complete) => {
                queue.mark_incident_resolved(&intent.work_item_id)
            }
            (WorkItemKind::Incident, SourceLifecycleAction::Block) => {
                queue.mark_incident_blocked(&intent.work_item_id)
            }
            (WorkItemKind::Incident, SourceLifecycleAction::Requeue) => {
                queue.requeue_incident(&intent.work_item_id, required_reason(intent)?)
            }
            (WorkItemKind::LearningRequest, SourceLifecycleAction::Complete) => {
                queue.mark_learning_request_done(&intent.work_item_id)
            }
            (WorkItemKind::LearningRequest, SourceLifecycleAction::Block) => {
                queue.mark_learning_request_blocked(&intent.work_item_id)
            }
            (WorkItemKind::LearningRequest, SourceLifecycleAction::Requeue) => {
                queue.requeue_learning_request(&intent.work_item_id, required_reason(intent)?)
            }
            (WorkItemKind::BlueprintDraft, SourceLifecycleAction::Complete) => {
                approve_active_blueprint_draft(&self.paths, &intent.work_item_id)
            }
            (WorkItemKind::BlueprintDraft, SourceLifecycleAction::Block) => {
                block_active_blueprint_draft(&self.paths, &intent.work_item_id)
            }
            (WorkItemKind::BlueprintDraft, SourceLifecycleAction::Requeue) => {
                let _reason = required_reason(intent)?;
                requeue_active_blueprint_draft(&self.paths, &intent.work_item_id)
            }
        }
    }

    fn apply_generic_family(
        &self,
        family_id: &str,
        intent: &SourceLifecycleIntent,
    ) -> QueueStoreResult<PathBuf> {
        let family = self
            .work_item_families
            .iter()
            .find(|family| family.family_id == family_id)
            .ok_or_else(|| QueueStoreError::InvalidState {
                message: format!("unsupported active work item family: {family_id}"),
            })?;
        move_generic_active_family(&self.paths, family, intent)
    }
}

/// Apply one lifecycle intent with no additional compiled generic families.
pub fn apply_source_lifecycle_intent(
    paths: &WorkspacePaths,
    intent: &SourceLifecycleIntent,
) -> QueueStoreResult<PathBuf> {
    QueueLifecycleInterpreter::new(paths.clone(), Vec::new()).apply(intent)
}

/// Requeue one active source item.
pub fn requeue_active_work_item(
    paths: &WorkspacePaths,
    work_item_family_id: Option<&str>,
    work_item_kind: Option<WorkItemKind>,
    work_item_id: &str,
    reason: &str,
    work_item_families: Vec<WorkItemFamilyDefinition>,
) -> QueueStoreResult<PathBuf> {
    let intent = SourceLifecycleIntent {
        lifecycle_plan_id: "requeue_work_item".to_owned(),
        action: SourceLifecycleAction::Requeue,
        work_item_family_id: work_item_family_id.map(ToOwned::to_owned),
        work_item_kind,
        work_item_id: work_item_id.to_owned(),
        reason: Some(reason.to_owned()),
    };
    QueueLifecycleInterpreter::new(paths.clone(), work_item_families).apply(&intent)
}

/// Requeue all active artifacts from supplied compiled work-item families.
pub fn requeue_all_active_work_items(
    paths: &WorkspacePaths,
    reason: &str,
    work_item_families: Vec<WorkItemFamilyDefinition>,
) -> QueueStoreResult<usize> {
    let mut count = 0;
    for family in &work_item_families {
        let active_dir = paths.runtime_root.join(&family.queue_dirs.active);
        if !active_dir.is_dir() {
            continue;
        }
        let mut entries = fs::read_dir(&active_dir)
            .map_err(|error| QueueStoreError::Io {
                path: active_dir.clone(),
                message: error.to_string(),
            })?
            .collect::<Result<Vec<_>, std::io::Error>>()
            .map_err(|error| QueueStoreError::Io {
                path: active_dir.clone(),
                message: error.to_string(),
            })?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            if !path.is_file()
                || path.extension().and_then(|value| value.to_str())
                    != extension_without_dot(&family.file_extension)
            {
                continue;
            }
            let Some(item_id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            requeue_active_work_item(
                paths,
                Some(&family.family_id),
                None,
                item_id,
                reason,
                work_item_families.clone(),
            )?;
            count += 1;
        }
    }
    Ok(count)
}

fn move_generic_active_family(
    paths: &WorkspacePaths,
    family: &WorkItemFamilyDefinition,
    intent: &SourceLifecycleIntent,
) -> QueueStoreResult<PathBuf> {
    let source = paths
        .runtime_root
        .join(&family.queue_dirs.active)
        .join(format!("{}{}", intent.work_item_id, family.file_extension));
    let target_dir = match intent.action {
        SourceLifecycleAction::Complete => &family.queue_dirs.done,
        SourceLifecycleAction::Block => &family.queue_dirs.blocked,
        SourceLifecycleAction::Requeue => &family.queue_dirs.queue,
    };
    let destination = paths
        .runtime_root
        .join(target_dir)
        .join(
            source
                .file_name()
                .ok_or_else(|| QueueStoreError::InvalidState {
                    message: "active generic work item path is missing filename".to_owned(),
                })?,
        );
    if !source.is_file() {
        return Err(QueueStoreError::InvalidState {
            message: format!("{} {} is not active", family.family_id, intent.work_item_id),
        });
    }
    if destination.exists() {
        return Err(QueueStoreError::InvalidState {
            message: format!(
                "{} {} already exists at destination",
                family.family_id, intent.work_item_id
            ),
        });
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| QueueStoreError::Io {
            path: parent.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    fs::rename(&source, &destination).map_err(|error| QueueStoreError::Io {
        path: source,
        message: error.to_string(),
    })?;
    if intent.action == SourceLifecycleAction::Requeue {
        append_family_requeue_reason(
            destination.parent().unwrap_or_else(|| Path::new(".")),
            &intent.work_item_id,
            &family.family_id,
            required_reason(intent)?,
        )?;
    }
    Ok(destination)
}

fn append_family_requeue_reason(
    destination_dir: &Path,
    work_item_id: &str,
    family_id: &str,
    reason: &str,
) -> QueueStoreResult<()> {
    let cleaned = reason.trim();
    if cleaned.is_empty() {
        return Err(QueueStoreError::InvalidState {
            message: "requeue reason is required".to_owned(),
        });
    }
    let path = destination_dir.join(format!("{work_item_id}.requeue.jsonl"));
    let payload = json!({
        "at": now_rfc3339(),
        "family_id": family_id,
        "reason": cleaned,
    })
    .to_string()
        + "\n";
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| QueueStoreError::Io {
            path: path.clone(),
            message: error.to_string(),
        })?;
    file.write_all(payload.as_bytes())
        .map_err(|error| QueueStoreError::Io {
            path,
            message: error.to_string(),
        })
}

fn validate_intent(intent: &SourceLifecycleIntent) -> QueueStoreResult<()> {
    validate_safe_identifier(&intent.lifecycle_plan_id, "lifecycle_plan_id").map_err(|error| {
        QueueStoreError::InvalidState {
            message: error.to_string(),
        }
    })?;
    validate_safe_identifier(&intent.work_item_id, "work_item_id").map_err(|error| {
        QueueStoreError::InvalidState {
            message: error.to_string(),
        }
    })?;
    if let Some(family_id) = &intent.work_item_family_id {
        validate_safe_identifier(family_id, "work_item_family_id").map_err(|error| {
            QueueStoreError::InvalidState {
                message: error.to_string(),
            }
        })?;
    }
    Ok(())
}

fn required_reason(intent: &SourceLifecycleIntent) -> QueueStoreResult<&str> {
    intent
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .ok_or_else(|| QueueStoreError::InvalidState {
            message: "requeue reason is required".to_owned(),
        })
}

fn extension_without_dot(extension: &str) -> Option<&str> {
    extension.strip_prefix('.')
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}
