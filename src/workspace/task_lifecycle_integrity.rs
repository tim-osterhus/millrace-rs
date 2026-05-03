//! Task lifecycle duplicate diagnostics and safe reconciliation helpers.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{contracts::TaskDocument, work_documents::parse_task_document_with_source};

use super::{
    WorkspacePaths,
    queue_store::{QueueStoreError, QueueStoreResult},
};

static RETIREMENT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// One task id that appears in more than one lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskLifecycleDuplicate {
    /// Logical task id from the parsed document, or filename stem when parsing fails.
    pub task_id: String,
    /// Lifecycle states and concrete paths where this task id was found.
    pub state_paths: Vec<(String, PathBuf)>,
}

impl TaskLifecycleDuplicate {
    /// Lifecycle state labels in deterministic scan order.
    #[must_use]
    pub fn states(&self) -> Vec<&str> {
        self.state_paths
            .iter()
            .map(|(state, _path)| state.as_str())
            .collect()
    }

    /// Concrete paths in deterministic scan order.
    #[must_use]
    pub fn paths(&self) -> Vec<&Path> {
        self.state_paths
            .iter()
            .map(|(_state, path)| path.as_path())
            .collect()
    }
}

/// Return task ids present in multiple task lifecycle directories.
pub fn find_duplicate_task_lifecycle_ids(
    paths: &WorkspacePaths,
) -> QueueStoreResult<Vec<TaskLifecycleDuplicate>> {
    let mut by_task_id: BTreeMap<String, Vec<(String, PathBuf)>> = BTreeMap::new();

    for (state, directory) in task_lifecycle_directories(paths) {
        for path in list_markdown_files(directory)? {
            let task_id = task_id_for_path(&path);
            by_task_id
                .entry(task_id)
                .or_default()
                .push((state.to_owned(), path));
        }
    }

    Ok(by_task_id
        .into_iter()
        .filter_map(|(task_id, state_paths)| {
            (state_paths.len() > 1).then_some(TaskLifecycleDuplicate {
                task_id,
                state_paths,
            })
        })
        .collect())
}

/// Archive a same-root blocked predecessor once a same-id continuation is done.
pub fn retire_stale_blocked_task_duplicate_after_done(
    paths: &WorkspacePaths,
    task_id: &str,
) -> QueueStoreResult<Option<PathBuf>> {
    let blocked_path = paths.tasks_blocked_dir.join(format!("{task_id}.md"));
    let done_path = paths.tasks_done_dir.join(format!("{task_id}.md"));
    if !blocked_path.is_file() || !done_path.is_file() {
        return Ok(None);
    }

    let Some(blocked_document) = read_task_document_or_none(&blocked_path) else {
        return Ok(None);
    };
    let Some(done_document) = read_task_document_or_none(&done_path) else {
        return Ok(None);
    };

    let Some(blocked_root) = effective_root_spec_id(&blocked_document) else {
        return Ok(None);
    };
    if Some(blocked_root) != effective_root_spec_id(&done_document) {
        return Ok(None);
    }

    let archive_dir = paths.tasks_blocked_dir.join("superseded");
    fs::create_dir_all(&archive_dir).map_err(|error| QueueStoreError::io(&archive_dir, error))?;
    let archived_at = OffsetDateTime::now_utc();
    let archive_path = superseded_task_path(&archive_dir, task_id, archived_at);

    match fs::rename(&blocked_path, &archive_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(QueueStoreError::io(&blocked_path, error)),
    }

    append_retirement_record(
        paths,
        &archive_dir,
        task_id,
        &blocked_path,
        &archive_path,
        archived_at,
        blocked_root,
    )?;
    Ok(Some(archive_path))
}

fn task_lifecycle_directories(paths: &WorkspacePaths) -> [(&'static str, &Path); 4] {
    [
        ("queue", paths.tasks_queue_dir.as_path()),
        ("active", paths.tasks_active_dir.as_path()),
        ("done", paths.tasks_done_dir.as_path()),
        ("blocked", paths.tasks_blocked_dir.as_path()),
    ]
}

fn task_id_for_path(path: &Path) -> String {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => return path_stem_lossy(path),
    };
    parse_task_document_with_source(&raw, &path.display().to_string())
        .map(|document| document.task_id)
        .unwrap_or_else(|_| path_stem_lossy(path))
}

fn read_task_document_or_none(path: &Path) -> Option<TaskDocument> {
    let raw = fs::read_to_string(path).ok()?;
    parse_task_document_with_source(&raw, &path.display().to_string()).ok()
}

fn effective_root_spec_id(document: &TaskDocument) -> Option<&str> {
    document
        .root_spec_id
        .as_deref()
        .or(document.spec_id.as_deref())
}

fn superseded_task_path(archive_dir: &Path, task_id: &str, archived_at: OffsetDateTime) -> PathBuf {
    let timestamp = compact_timestamp(archived_at);
    for _ in 0..100 {
        let suffix = retirement_suffix();
        let candidate = archive_dir.join(format!("{task_id}.{timestamp}.{suffix}.blocked.md"));
        if !candidate.exists() {
            return candidate;
        }
    }
    archive_dir.join(format!(
        "{task_id}.{timestamp}.{}.blocked.md",
        retirement_suffix()
    ))
}

fn append_retirement_record(
    paths: &WorkspacePaths,
    archive_dir: &Path,
    task_id: &str,
    source_path: &Path,
    archive_path: &Path,
    archived_at: OffsetDateTime,
    root_spec_id: &str,
) -> QueueStoreResult<()> {
    let log_path = archive_dir.join("retirements.jsonl");
    let archived_at = archived_at
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());
    let record = BTreeMap::from([
        ("archive_path", workspace_relative_path(paths, archive_path)),
        ("archived_at", archived_at),
        ("reason", "same_id_done_continuation".to_owned()),
        ("root_spec_id", root_spec_id.to_owned()),
        ("source_path", workspace_relative_path(paths, source_path)),
        ("task_id", task_id.to_owned()),
    ]);
    let line = serde_json::to_string(&record).map_err(|error| QueueStoreError::InvalidState {
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

fn compact_timestamp(timestamp: OffsetDateTime) -> String {
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        timestamp.year(),
        u8::from(timestamp.month()),
        timestamp.day(),
        timestamp.hour(),
        timestamp.minute(),
        timestamp.second()
    )
}

fn retirement_suffix() -> String {
    let counter = RETIREMENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = OffsetDateTime::now_utc().unix_timestamp_nanos() as u64;
    format!(
        "{:08x}",
        (now ^ counter ^ u64::from(process::id())) & 0xffff_ffff
    )
}

fn workspace_relative_path(paths: &WorkspacePaths, path: &Path) -> String {
    path.strip_prefix(&paths.root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn path_stem_lossy(path: &Path) -> String {
    path.file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default()
}
