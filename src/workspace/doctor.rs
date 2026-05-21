//! Deterministic workspace doctor checks for initialized workspace health.

use std::{
    fmt, fs, io,
    path::{Component, Path, PathBuf},
};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    contracts::{
        BlueprintCritiqueDocument, BlueprintDraftDocument, BlueprintEvaluationDocument,
        BlueprintManifestDocument, BlueprintPacketDocument, BlueprintPromotionRecord,
        RuntimeJsonContract,
    },
    work_documents::{
        parse_incident_document_with_source, parse_learning_request_document_with_source,
        parse_spec_document_with_source, parse_task_document_with_source,
    },
};

use super::{
    BaselineManifest, CURRENT_WORKSPACE_SCHEMA_EPOCH, RuntimeOwnershipLockState, WorkspacePaths,
    find_duplicate_task_lifecycle_ids, inspect_runtime_ownership_lock, load_baseline_manifest,
    load_execution_status, load_learning_status, load_planning_status, load_recovery_counters,
    load_snapshot, load_workspace_schema_epoch_marker, workspace_paths,
};

/// One workspace doctor finding with deterministic code and optional path context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorIssue {
    /// Stable issue code.
    pub code: String,
    /// Human-readable issue detail.
    pub message: String,
    /// Path involved in the issue, when applicable.
    pub path: Option<PathBuf>,
}

impl DoctorIssue {
    fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        path: impl Into<Option<PathBuf>>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            path: path.into(),
        }
    }
}

/// Aggregated doctor findings for one workspace check pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    /// True when no error-level issues were found.
    pub ok: bool,
    /// Error-level findings.
    pub errors: Vec<DoctorIssue>,
    /// Warning-level findings.
    pub warnings: Vec<DoctorIssue>,
    /// RFC 3339 timestamp for the check pass.
    pub checked_at: String,
}

/// Run deterministic workspace checks without mutating workspace state.
#[must_use]
pub fn run_workspace_doctor(root: impl AsRef<Path>) -> DoctorReport {
    let paths = workspace_paths(root);
    run_workspace_doctor_for_paths(&paths)
}

/// Run deterministic workspace checks against already resolved paths.
#[must_use]
pub fn run_workspace_doctor_for_paths(paths: &WorkspacePaths) -> DoctorReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    validate_workspace_layout(paths, &mut errors);
    let baseline_manifest = validate_baseline_manifest(paths, &mut errors);
    validate_status_and_state(paths, &mut errors);
    validate_schema_epoch_marker(paths, &mut warnings);
    validate_runtime_ownership_lock(paths, &mut errors, &mut warnings);
    validate_queue_parseability(paths, &mut errors);
    validate_blueprint_artifacts(paths, &mut errors);
    validate_task_lifecycle_uniqueness(paths, &mut errors);
    if let Some(manifest) = baseline_manifest.as_ref() {
        validate_manifest_tracked_managed_files(paths, manifest, &mut errors);
    }

    sort_issues(&mut errors);
    sort_issues(&mut warnings);

    DoctorReport {
        ok: errors.is_empty(),
        errors,
        warnings,
        checked_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned()),
    }
}

fn validate_blueprint_artifacts(paths: &WorkspacePaths, errors: &mut Vec<DoctorIssue>) {
    for target in blueprint_artifact_targets(paths) {
        validate_blueprint_artifact_directory(&target, errors);
    }
}

fn validate_schema_epoch_marker(paths: &WorkspacePaths, warnings: &mut Vec<DoctorIssue>) {
    match load_workspace_schema_epoch_marker(paths) {
        Ok(marker) if marker.epoch_id == CURRENT_WORKSPACE_SCHEMA_EPOCH => {}
        Ok(marker) => warnings.push(DoctorIssue::new(
            "workspace_schema_epoch_incompatible",
            format!(
                "workspace schema epoch {} differs from expected {}",
                marker.epoch_id, CURRENT_WORKSPACE_SCHEMA_EPOCH
            ),
            super::workspace_schema_epoch_marker_path(paths),
        )),
        Err(error) => warnings.push(DoctorIssue::new(
            "workspace_schema_epoch_marker_invalid",
            error.to_string(),
            super::workspace_schema_epoch_marker_path(paths),
        )),
    }
}

fn validate_workspace_layout(paths: &WorkspacePaths, errors: &mut Vec<DoctorIssue>) {
    for directory in paths.directories() {
        if directory.is_dir() {
            continue;
        }
        errors.push(DoctorIssue::new(
            "missing_directory",
            "required workspace directory is missing",
            directory.to_path_buf(),
        ));
    }

    for file_path in required_workspace_files(paths) {
        if file_path.is_file() {
            continue;
        }
        errors.push(DoctorIssue::new(
            "missing_file",
            "required workspace file is missing",
            file_path.to_path_buf(),
        ));
    }
}

fn required_workspace_files(paths: &WorkspacePaths) -> [&Path; 9] {
    [
        &paths.outline_file,
        &paths.historylog_file,
        &paths.runtime_config_file,
        &paths.execution_status_file,
        &paths.planning_status_file,
        &paths.learning_status_file,
        &paths.runtime_snapshot_file,
        &paths.recovery_counters_file,
        &paths.learning_events_file,
    ]
}

fn validate_baseline_manifest(
    paths: &WorkspacePaths,
    errors: &mut Vec<DoctorIssue>,
) -> Option<BaselineManifest> {
    if !paths.baseline_manifest_file.is_file() {
        errors.push(DoctorIssue::new(
            "baseline_manifest_missing",
            "baseline manifest is missing",
            paths.baseline_manifest_file.clone(),
        ));
        return None;
    }

    let manifest = match load_baseline_manifest(paths) {
        Ok(manifest) => manifest,
        Err(error) => {
            errors.push(DoctorIssue::new(
                "baseline_manifest_invalid",
                error.to_string(),
                paths.baseline_manifest_file.clone(),
            ));
            return None;
        }
    };

    if let Err(message) = validate_manifest_contract(&manifest) {
        errors.push(DoctorIssue::new(
            "baseline_manifest_invalid",
            message,
            paths.baseline_manifest_file.clone(),
        ));
        return None;
    }

    Some(manifest)
}

fn validate_manifest_contract(manifest: &BaselineManifest) -> Result<(), String> {
    if manifest.schema_version != "1.0" {
        return Err(format!(
            "schema_version must be 1.0, got {}",
            manifest.schema_version
        ));
    }
    if manifest.manifest_id.trim().is_empty() {
        return Err("manifest_id must be a non-empty string".to_owned());
    }
    if manifest.seed_package_version.trim().is_empty() {
        return Err("seed_package_version must be a non-empty string".to_owned());
    }

    let mut previous_path: Option<&str> = None;
    for entry in &manifest.entries {
        validate_manifest_relative_path(&entry.relative_path)?;
        if entry.asset_family.trim().is_empty() {
            return Err(format!(
                "asset_family must be non-empty for {}",
                entry.relative_path
            ));
        }
        let Some((family, _)) = entry.relative_path.split_once('/') else {
            return Err(format!(
                "relative_path must include an asset family: {}",
                entry.relative_path
            ));
        };
        if entry.asset_family != family {
            return Err(format!(
                "asset_family {} does not match relative_path family {}",
                entry.asset_family, family
            ));
        }
        if !is_sha256_hex(&entry.original_sha256) {
            return Err(format!(
                "original_sha256 must be a SHA-256 hex digest for {}",
                entry.relative_path
            ));
        }
        if let Some(previous) = previous_path {
            if previous >= entry.relative_path.as_str() {
                return Err("manifest entries must be sorted and unique".to_owned());
            }
        }
        previous_path = Some(&entry.relative_path);
    }

    Ok(())
}

fn validate_manifest_relative_path(relative_path: &str) -> Result<(), String> {
    if relative_path.trim().is_empty() {
        return Err("relative_path must be non-empty".to_owned());
    }
    let path = Path::new(relative_path);
    if path.is_absolute() {
        return Err(format!(
            "relative_path must not be absolute: {relative_path}"
        ));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!(
            "relative_path must stay under the runtime root: {relative_path}"
        ));
    }
    Ok(())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_manifest_tracked_managed_files(
    paths: &WorkspacePaths,
    manifest: &BaselineManifest,
    errors: &mut Vec<DoctorIssue>,
) {
    for entry in &manifest.entries {
        let candidate = paths.runtime_root.join(&entry.relative_path);
        if candidate.is_file() {
            continue;
        }
        errors.push(DoctorIssue::new(
            "baseline_manifest_managed_file_missing",
            "manifest-tracked managed file is missing",
            candidate,
        ));
    }
}

fn validate_status_and_state(paths: &WorkspacePaths, errors: &mut Vec<DoctorIssue>) {
    if let Err(error) = load_execution_status(paths) {
        errors.push(DoctorIssue::new(
            "execution_status_invalid",
            error.to_string(),
            paths.execution_status_file.clone(),
        ));
    }
    if let Err(error) = load_planning_status(paths) {
        errors.push(DoctorIssue::new(
            "planning_status_invalid",
            error.to_string(),
            paths.planning_status_file.clone(),
        ));
    }
    if let Err(error) = load_learning_status(paths) {
        errors.push(DoctorIssue::new(
            "learning_status_invalid",
            error.to_string(),
            paths.learning_status_file.clone(),
        ));
    }
    if let Err(error) = load_snapshot(paths) {
        errors.push(DoctorIssue::new(
            "snapshot_invalid",
            error.to_string(),
            paths.runtime_snapshot_file.clone(),
        ));
    }
    if let Err(error) = load_recovery_counters(paths) {
        errors.push(DoctorIssue::new(
            "recovery_counters_invalid",
            error.to_string(),
            paths.recovery_counters_file.clone(),
        ));
    }
}

fn validate_runtime_ownership_lock(
    paths: &WorkspacePaths,
    errors: &mut Vec<DoctorIssue>,
    warnings: &mut Vec<DoctorIssue>,
) {
    let status = inspect_runtime_ownership_lock(paths);
    match status.state {
        RuntimeOwnershipLockState::Absent => {}
        RuntimeOwnershipLockState::Active => warnings.push(DoctorIssue::new(
            "runtime_ownership_lock_active",
            status.detail,
            status.lock_path,
        )),
        RuntimeOwnershipLockState::Stale => errors.push(DoctorIssue::new(
            "runtime_ownership_lock_stale",
            status.detail,
            status.lock_path,
        )),
        RuntimeOwnershipLockState::Invalid => errors.push(DoctorIssue::new(
            "runtime_ownership_lock_invalid",
            status.detail,
            status.lock_path,
        )),
    }
}

fn validate_queue_parseability(paths: &WorkspacePaths, errors: &mut Vec<DoctorIssue>) {
    for target in queue_targets(paths) {
        validate_queue_directory(target, errors);
    }
}

fn blueprint_artifact_targets(paths: &WorkspacePaths) -> Vec<BlueprintArtifactTarget> {
    let blueprints = paths.runtime_root.join("blueprints");
    vec![
        BlueprintArtifactTarget::new(
            blueprints.join("manifests"),
            BlueprintArtifactKind::Manifest,
        ),
        BlueprintArtifactTarget::new(
            blueprints.join("drafts/queue"),
            BlueprintArtifactKind::Draft,
        ),
        BlueprintArtifactTarget::new(
            blueprints.join("drafts/active"),
            BlueprintArtifactKind::Draft,
        ),
        BlueprintArtifactTarget::new(
            blueprints.join("drafts/approved"),
            BlueprintArtifactKind::Draft,
        ),
        BlueprintArtifactTarget::new(
            blueprints.join("drafts/blocked"),
            BlueprintArtifactKind::Draft,
        ),
        BlueprintArtifactTarget::new(
            blueprints.join("drafts/canceled"),
            BlueprintArtifactKind::Draft,
        ),
        BlueprintArtifactTarget::new(
            blueprints.join("packets/candidates"),
            BlueprintArtifactKind::Packet,
        ),
        BlueprintArtifactTarget::new(
            blueprints.join("packets/approved"),
            BlueprintArtifactKind::Packet,
        ),
        BlueprintArtifactTarget::new(
            blueprints.join("packets/rejected"),
            BlueprintArtifactKind::Packet,
        ),
        BlueprintArtifactTarget::new(
            blueprints.join("critiques"),
            BlueprintArtifactKind::Critique,
        ),
        BlueprintArtifactTarget::new(
            blueprints.join("evaluations"),
            BlueprintArtifactKind::Evaluation,
        ),
        BlueprintArtifactTarget::new(
            blueprints.join("promotions"),
            BlueprintArtifactKind::Promotion,
        ),
    ]
}

struct BlueprintArtifactTarget {
    directory: PathBuf,
    kind: BlueprintArtifactKind,
}

impl BlueprintArtifactTarget {
    fn new(directory: PathBuf, kind: BlueprintArtifactKind) -> Self {
        Self { directory, kind }
    }
}

#[derive(Debug, Clone, Copy)]
enum BlueprintArtifactKind {
    Manifest,
    Draft,
    Packet,
    Critique,
    Evaluation,
    Promotion,
}

impl BlueprintArtifactKind {
    fn label(self) -> &'static str {
        match self {
            Self::Manifest => "blueprint_manifest",
            Self::Draft => "blueprint_draft",
            Self::Packet => "blueprint_packet",
            Self::Critique => "blueprint_critique",
            Self::Evaluation => "blueprint_evaluation",
            Self::Promotion => "blueprint_promotion",
        }
    }
}

fn validate_blueprint_artifact_directory(
    target: &BlueprintArtifactTarget,
    errors: &mut Vec<DoctorIssue>,
) {
    if !target.directory.exists() {
        return;
    }
    let mut entries = match fs::read_dir(&target.directory) {
        Ok(entries) => match entries.collect::<Result<Vec<_>, io::Error>>() {
            Ok(entries) => entries,
            Err(error) => {
                errors.push(DoctorIssue::new(
                    "blueprint_artifact_invalid",
                    format!("failed to read {} directory: {error}", target.kind.label()),
                    target.directory.clone(),
                ));
                return;
            }
        },
        Err(error) => {
            errors.push(DoctorIssue::new(
                "blueprint_artifact_invalid",
                format!("failed to read {} directory: {error}", target.kind.label()),
                target.directory.clone(),
            ));
            return;
        }
    };
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        validate_blueprint_artifact(&path, target.kind, errors);
    }
}

fn validate_blueprint_artifact(
    path: &Path,
    kind: BlueprintArtifactKind,
    errors: &mut Vec<DoctorIssue>,
) {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) => {
            errors.push(DoctorIssue::new(
                "blueprint_artifact_invalid",
                format!("failed to read {} artifact: {error}", kind.label()),
                path.to_path_buf(),
            ));
            return;
        }
    };
    let parsed_id = match parse_blueprint_artifact(kind, &raw) {
        Ok(parsed_id) => parsed_id,
        Err(error) => {
            errors.push(DoctorIssue::new(
                "blueprint_artifact_invalid",
                format!("invalid {} artifact: {error}", kind.label()),
                path.to_path_buf(),
            ));
            return;
        }
    };
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if stem != parsed_id {
        errors.push(DoctorIssue::new(
            "blueprint_artifact_invalid",
            format!(
                "filename stem does not match {} id: expected {}, found {}",
                kind.label(),
                parsed_id,
                stem
            ),
            path.to_path_buf(),
        ));
    }
}

fn parse_blueprint_artifact(kind: BlueprintArtifactKind, raw: &str) -> Result<String, String> {
    match kind {
        BlueprintArtifactKind::Manifest => BlueprintManifestDocument::from_json_str(raw)
            .map(|document| document.manifest_id)
            .map_err(|error| error.to_string()),
        BlueprintArtifactKind::Draft => BlueprintDraftDocument::from_json_str(raw)
            .map(|document| document.draft_id)
            .map_err(|error| error.to_string()),
        BlueprintArtifactKind::Packet => BlueprintPacketDocument::from_json_str(raw)
            .map(|document| document.blueprint_id)
            .map_err(|error| error.to_string()),
        BlueprintArtifactKind::Critique => BlueprintCritiqueDocument::from_json_str(raw)
            .map(|document| document.critique_id)
            .map_err(|error| error.to_string()),
        BlueprintArtifactKind::Evaluation => BlueprintEvaluationDocument::from_json_str(raw)
            .map(|document| document.evaluation_id)
            .map_err(|error| error.to_string()),
        BlueprintArtifactKind::Promotion => BlueprintPromotionRecord::from_json_str(raw)
            .map(|document| document.promotion_id)
            .map_err(|error| error.to_string()),
    }
}

fn validate_task_lifecycle_uniqueness(paths: &WorkspacePaths, errors: &mut Vec<DoctorIssue>) {
    let duplicates = match find_duplicate_task_lifecycle_ids(paths) {
        Ok(duplicates) => duplicates,
        Err(error) => {
            errors.push(DoctorIssue::new(
                "task_lifecycle_scan_failed",
                error.to_string(),
                paths.tasks_dir.clone(),
            ));
            return;
        }
    };

    for duplicate in duplicates {
        let state_summary = duplicate
            .state_paths
            .iter()
            .map(|(state, path)| format!("{}:{}", state, workspace_relative_path(paths, path)))
            .collect::<Vec<_>>()
            .join(", ");
        let primary_path = duplicate
            .state_paths
            .first()
            .map(|(_state, path)| path.clone());
        errors.push(DoctorIssue::new(
            "duplicate_task_lifecycle_state",
            format!(
                "task {} appears in multiple lifecycle states: {}",
                duplicate.task_id, state_summary
            ),
            primary_path,
        ));
    }
}

fn queue_targets(paths: &WorkspacePaths) -> Vec<QueueTarget<'_>> {
    vec![
        QueueTarget::task(&paths.tasks_queue_dir),
        QueueTarget::task(&paths.tasks_active_dir),
        QueueTarget::task(&paths.tasks_done_dir),
        QueueTarget::task(&paths.tasks_blocked_dir),
        QueueTarget::spec(&paths.specs_queue_dir),
        QueueTarget::spec(&paths.specs_active_dir),
        QueueTarget::spec(&paths.specs_done_dir),
        QueueTarget::spec(&paths.specs_blocked_dir),
        QueueTarget::incident(&paths.incidents_incoming_dir),
        QueueTarget::incident(&paths.incidents_active_dir),
        QueueTarget::incident(&paths.incidents_resolved_dir),
        QueueTarget::incident(&paths.incidents_blocked_dir),
        QueueTarget::learning_request(&paths.learning_requests_queue_dir),
        QueueTarget::learning_request(&paths.learning_requests_active_dir),
        QueueTarget::learning_request(&paths.learning_requests_done_dir),
        QueueTarget::learning_request(&paths.learning_requests_blocked_dir),
    ]
}

struct QueueTarget<'a> {
    directory: &'a Path,
    kind: QueueDocumentKind,
}

impl<'a> QueueTarget<'a> {
    fn task(directory: &'a Path) -> Self {
        Self {
            directory,
            kind: QueueDocumentKind::Task,
        }
    }

    fn spec(directory: &'a Path) -> Self {
        Self {
            directory,
            kind: QueueDocumentKind::Spec,
        }
    }

    fn incident(directory: &'a Path) -> Self {
        Self {
            directory,
            kind: QueueDocumentKind::Incident,
        }
    }

    fn learning_request(directory: &'a Path) -> Self {
        Self {
            directory,
            kind: QueueDocumentKind::LearningRequest,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum QueueDocumentKind {
    Task,
    Spec,
    Incident,
    LearningRequest,
}

impl QueueDocumentKind {
    fn id_field(self) -> &'static str {
        match self {
            Self::Task => "task_id",
            Self::Spec => "spec_id",
            Self::Incident => "incident_id",
            Self::LearningRequest => "learning_request_id",
        }
    }
}

impl fmt::Display for QueueDocumentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Task => f.write_str("task"),
            Self::Spec => f.write_str("spec"),
            Self::Incident => f.write_str("incident"),
            Self::LearningRequest => f.write_str("learning request"),
        }
    }
}

fn validate_queue_directory(target: QueueTarget<'_>, errors: &mut Vec<DoctorIssue>) {
    let mut entries = match fs::read_dir(target.directory) {
        Ok(entries) => entries
            .collect::<Result<Vec<_>, io::Error>>()
            .map_err(|error| (target.directory.to_path_buf(), error)),
        Err(error) => Err((target.directory.to_path_buf(), error)),
    };

    let entries = match entries.as_mut() {
        Ok(entries) => entries,
        Err((path, error)) => {
            errors.push(DoctorIssue::new(
                "queue_artifact_invalid",
                format!("failed to read queue directory: {error}"),
                path.clone(),
            ));
            return;
        }
    };
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        validate_queue_artifact(&path, target.kind, errors);
    }
}

fn validate_queue_artifact(path: &Path, kind: QueueDocumentKind, errors: &mut Vec<DoctorIssue>) {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) => {
            errors.push(DoctorIssue::new(
                "queue_artifact_invalid",
                format!("failed to read {kind} queue artifact: {error}"),
                path.to_path_buf(),
            ));
            return;
        }
    };
    let source = path.display().to_string();
    let parsed_id = match kind {
        QueueDocumentKind::Task => {
            parse_task_document_with_source(&raw, &source).map(|document| document.task_id)
        }
        QueueDocumentKind::Spec => {
            parse_spec_document_with_source(&raw, &source).map(|document| document.spec_id)
        }
        QueueDocumentKind::Incident => {
            parse_incident_document_with_source(&raw, &source).map(|document| document.incident_id)
        }
        QueueDocumentKind::LearningRequest => {
            parse_learning_request_document_with_source(&raw, &source)
                .map(|document| document.learning_request_id)
        }
    };

    let parsed_id = match parsed_id {
        Ok(parsed_id) => parsed_id,
        Err(error) => {
            errors.push(DoctorIssue::new(
                "queue_artifact_invalid",
                error.to_string(),
                path.to_path_buf(),
            ));
            return;
        }
    };
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if stem != parsed_id {
        errors.push(DoctorIssue::new(
            "queue_artifact_invalid",
            format!(
                "filename stem does not match {}: expected {}, found {}",
                kind.id_field(),
                parsed_id,
                stem
            ),
            path.to_path_buf(),
        ));
    }
}

fn sort_issues(issues: &mut [DoctorIssue]) {
    issues.sort_by(|left, right| {
        issue_path_key(left)
            .cmp(&issue_path_key(right))
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
}

fn issue_path_key(issue: &DoctorIssue) -> String {
    issue
        .path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn workspace_relative_path(paths: &WorkspacePaths, path: &Path) -> String {
    path.strip_prefix(&paths.root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
