//! Built-in work-item document adapters for generic queue operations.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    contracts::{Timestamp, WorkItemKind, validate_safe_identifier},
    work_documents::{
        parse_incident_document_with_source, parse_learning_request_document_with_source,
        parse_probe_document_with_source, parse_spec_document_with_source,
        parse_task_document_with_source,
    },
};

use super::{QueueStoreError, QueueStoreResult, WorkspacePaths};

/// Built-in markdown document adapter metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkItemDocumentAdapter {
    pub family_id: &'static str,
    pub work_item_kind: WorkItemKind,
    pub id_field: &'static str,
    pub timestamp_field: &'static str,
    pub supports_root_filter: bool,
}

impl WorkItemDocumentAdapter {
    /// Queue directory for this adapter.
    #[must_use]
    pub fn queue_dir(self, paths: &WorkspacePaths) -> &Path {
        match self.work_item_kind {
            WorkItemKind::Task => &paths.tasks_queue_dir,
            WorkItemKind::Probe => &paths.probes_queue_dir,
            WorkItemKind::Spec => &paths.specs_queue_dir,
            WorkItemKind::Incident => &paths.incidents_incoming_dir,
            WorkItemKind::LearningRequest => &paths.learning_requests_queue_dir,
            WorkItemKind::BlueprintDraft => &paths.runtime_root,
        }
    }

    /// Active directory for this adapter.
    #[must_use]
    pub fn active_dir(self, paths: &WorkspacePaths) -> &Path {
        match self.work_item_kind {
            WorkItemKind::Task => &paths.tasks_active_dir,
            WorkItemKind::Probe => &paths.probes_active_dir,
            WorkItemKind::Spec => &paths.specs_active_dir,
            WorkItemKind::Incident => &paths.incidents_active_dir,
            WorkItemKind::LearningRequest => &paths.learning_requests_active_dir,
            WorkItemKind::BlueprintDraft => &paths.runtime_root,
        }
    }

    /// Done directory for this adapter.
    #[must_use]
    pub fn done_dir(self, paths: &WorkspacePaths) -> &Path {
        match self.work_item_kind {
            WorkItemKind::Task => &paths.tasks_done_dir,
            WorkItemKind::Probe => &paths.probes_done_dir,
            WorkItemKind::Spec => &paths.specs_done_dir,
            WorkItemKind::Incident => &paths.incidents_resolved_dir,
            WorkItemKind::LearningRequest => &paths.learning_requests_done_dir,
            WorkItemKind::BlueprintDraft => &paths.runtime_root,
        }
    }

    /// Blocked directory for this adapter.
    #[must_use]
    pub fn blocked_dir(self, paths: &WorkspacePaths) -> &Path {
        match self.work_item_kind {
            WorkItemKind::Task => &paths.tasks_blocked_dir,
            WorkItemKind::Probe => &paths.probes_blocked_dir,
            WorkItemKind::Spec => &paths.specs_blocked_dir,
            WorkItemKind::Incident => &paths.incidents_blocked_dir,
            WorkItemKind::LearningRequest => &paths.learning_requests_blocked_dir,
            WorkItemKind::BlueprintDraft => &paths.runtime_root,
        }
    }
}

/// Parsed identity fields from one adapted work document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterParsedDocument {
    pub work_item_id: String,
    pub title: String,
    pub created_at: Timestamp,
}

const TASK_ADAPTER: WorkItemDocumentAdapter = WorkItemDocumentAdapter {
    family_id: "task",
    work_item_kind: WorkItemKind::Task,
    id_field: "task_id",
    timestamp_field: "created_at",
    supports_root_filter: true,
};
const PROBE_ADAPTER: WorkItemDocumentAdapter = WorkItemDocumentAdapter {
    family_id: "probe",
    work_item_kind: WorkItemKind::Probe,
    id_field: "probe_id",
    timestamp_field: "created_at",
    supports_root_filter: false,
};
const SPEC_ADAPTER: WorkItemDocumentAdapter = WorkItemDocumentAdapter {
    family_id: "spec",
    work_item_kind: WorkItemKind::Spec,
    id_field: "spec_id",
    timestamp_field: "created_at",
    supports_root_filter: true,
};
const INCIDENT_ADAPTER: WorkItemDocumentAdapter = WorkItemDocumentAdapter {
    family_id: "incident",
    work_item_kind: WorkItemKind::Incident,
    id_field: "incident_id",
    timestamp_field: "opened_at",
    supports_root_filter: true,
};
const LEARNING_REQUEST_ADAPTER: WorkItemDocumentAdapter = WorkItemDocumentAdapter {
    family_id: "learning_request",
    work_item_kind: WorkItemKind::LearningRequest,
    id_field: "learning_request_id",
    timestamp_field: "created_at",
    supports_root_filter: false,
};

const BUILTIN_ADAPTERS: &[WorkItemDocumentAdapter] = &[
    TASK_ADAPTER,
    PROBE_ADAPTER,
    SPEC_ADAPTER,
    INCIDENT_ADAPTER,
    LEARNING_REQUEST_ADAPTER,
];

/// Return built-in markdown work-item adapters in deterministic family order.
#[must_use]
pub fn builtin_work_item_adapters() -> &'static [WorkItemDocumentAdapter] {
    BUILTIN_ADAPTERS
}

/// Look up a built-in adapter by legacy work-item kind.
#[must_use]
pub fn adapter_for_kind(kind: WorkItemKind) -> Option<WorkItemDocumentAdapter> {
    BUILTIN_ADAPTERS
        .iter()
        .copied()
        .find(|adapter| adapter.work_item_kind == kind)
}

/// Look up a built-in adapter by work-item family id.
pub fn adapter_for_family_id(family_id: &str) -> QueueStoreResult<WorkItemDocumentAdapter> {
    let normalized = validate_safe_identifier(family_id, "family_id").map_err(|error| {
        QueueStoreError::InvalidState {
            message: error.to_string(),
        }
    })?;
    BUILTIN_ADAPTERS
        .iter()
        .copied()
        .find(|adapter| adapter.family_id == normalized)
        .ok_or_else(|| QueueStoreError::InvalidState {
            message: format!("unknown work item family: {normalized}"),
        })
}

/// Parse one work document through an adapter.
pub fn parse_with_adapter(
    adapter: WorkItemDocumentAdapter,
    text: &str,
    path: &Path,
) -> QueueStoreResult<AdapterParsedDocument> {
    let source = path.display().to_string();
    match adapter.work_item_kind {
        WorkItemKind::Task => {
            let document = parse_task_document_with_source(text, &source).map_err(|source| {
                QueueStoreError::WorkDocument {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
            Ok(AdapterParsedDocument {
                work_item_id: document.task_id,
                title: document.title,
                created_at: document.created_at,
            })
        }
        WorkItemKind::Probe => {
            let document = parse_probe_document_with_source(text, &source).map_err(|source| {
                QueueStoreError::WorkDocument {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
            Ok(AdapterParsedDocument {
                work_item_id: document.probe_id,
                title: document.title,
                created_at: document.created_at,
            })
        }
        WorkItemKind::Spec => {
            let document = parse_spec_document_with_source(text, &source).map_err(|source| {
                QueueStoreError::WorkDocument {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
            Ok(AdapterParsedDocument {
                work_item_id: document.spec_id,
                title: document.title,
                created_at: document.created_at,
            })
        }
        WorkItemKind::Incident => {
            let document =
                parse_incident_document_with_source(text, &source).map_err(|source| {
                    QueueStoreError::WorkDocument {
                        path: path.to_path_buf(),
                        source,
                    }
                })?;
            Ok(AdapterParsedDocument {
                work_item_id: document.incident_id,
                title: document.title,
                created_at: document.opened_at,
            })
        }
        WorkItemKind::LearningRequest => {
            let document =
                parse_learning_request_document_with_source(text, &source).map_err(|source| {
                    QueueStoreError::WorkDocument {
                        path: path.to_path_buf(),
                        source,
                    }
                })?;
            Ok(AdapterParsedDocument {
                work_item_id: document.learning_request_id,
                title: document.title,
                created_at: document.created_at,
            })
        }
        WorkItemKind::BlueprintDraft => Err(QueueStoreError::InvalidState {
            message: "blueprint_draft is not backed by a built-in markdown adapter".to_owned(),
        }),
    }
}

/// Validate that a work-document filename agrees with its parsed id field.
pub fn validate_filename_with_adapter(
    adapter: WorkItemDocumentAdapter,
    path: &Path,
) -> QueueStoreResult<AdapterParsedDocument> {
    let raw = fs::read_to_string(path).map_err(|error| QueueStoreError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let parsed = parse_with_adapter(adapter, &raw, path)?;
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if stem != parsed.work_item_id {
        return Err(QueueStoreError::InvalidState {
            message: format!(
                "filename stem does not match {}: expected {}, found {}",
                adapter.id_field, parsed.work_item_id, stem
            ),
        });
    }
    Ok(parsed)
}

/// Enqueue already-rendered markdown through an adapter after generic uniqueness checks.
pub fn enqueue_rendered_with_adapter(
    paths: &WorkspacePaths,
    adapter: WorkItemDocumentAdapter,
    item_id: &str,
    rendered: &str,
) -> QueueStoreResult<PathBuf> {
    validate_safe_identifier(item_id, adapter.id_field).map_err(|error| {
        QueueStoreError::InvalidState {
            message: error.to_string(),
        }
    })?;
    ensure_unique_with_adapter(paths, adapter, item_id)?;
    let destination = adapter.queue_dir(paths).join(format!("{item_id}.md"));
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| QueueStoreError::Io {
            path: parent.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    fs::write(&destination, rendered).map_err(|error| QueueStoreError::Io {
        path: destination.clone(),
        message: error.to_string(),
    })?;
    validate_filename_with_adapter(adapter, &destination)?;
    Ok(destination)
}

fn ensure_unique_with_adapter(
    paths: &WorkspacePaths,
    adapter: WorkItemDocumentAdapter,
    item_id: &str,
) -> QueueStoreResult<()> {
    let filename = format!("{item_id}.md");
    for directory in [
        adapter.queue_dir(paths),
        adapter.active_dir(paths),
        adapter.done_dir(paths),
        adapter.blocked_dir(paths),
    ] {
        if directory.join(&filename).exists() {
            return Err(QueueStoreError::InvalidState {
                message: format!("{} {item_id} already exists", adapter.family_id),
            });
        }
    }
    Ok(())
}
