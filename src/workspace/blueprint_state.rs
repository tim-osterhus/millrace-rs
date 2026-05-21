//! JSON-backed state helpers for Blueprint Planning artifacts.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    contracts::{
        BlueprintCritiqueDocument, BlueprintDraftDocument, BlueprintDraftStatus,
        BlueprintEvaluationDocument, BlueprintManifestDocument, BlueprintPacketDocument,
        BlueprintPromotionRecord, ClosureBlockingWorkRef, RuntimeJsonContract, WorkItemKind,
        validate_safe_identifier,
    },
    workspace::{QueueClaim, QueueStoreError, QueueStoreResult, WorkspacePaths},
};

type ManifestEntry = (PathBuf, BlueprintManifestDocument);

/// Persist a Blueprint manifest keyed by `manifest_id`, while tolerating legacy root-keyed files.
pub fn write_blueprint_manifest(
    paths: &WorkspacePaths,
    manifest: &BlueprintManifestDocument,
) -> QueueStoreResult<PathBuf> {
    validate_model(manifest)?;
    let destination = blueprint_manifest_path(paths, &manifest.manifest_id)?;
    let entries = blueprint_manifest_entries_for_id(paths, &manifest.manifest_id)?;
    if !entries.is_empty() {
        let _existing = resolve_blueprint_manifest_entry(&manifest.manifest_id, &entries)?;
        let expected = normalized_json(manifest)?;
        for (_path, existing) in &entries {
            if normalized_json(existing)? != expected {
                return Err(QueueStoreError::InvalidState {
                    message: duplicate_manifest_message(&manifest.manifest_id, &entries),
                });
            }
        }
        if destination.exists() {
            return Ok(destination);
        }
    }
    write_unique_json_document(&destination, manifest)
}

/// Canonical manifest path for a Blueprint manifest id.
pub fn blueprint_manifest_path(
    paths: &WorkspacePaths,
    manifest_id: &str,
) -> QueueStoreResult<PathBuf> {
    validate_safe_identifier(manifest_id, "manifest_id").map_err(invalid_state)?;
    Ok(blueprint_manifest_dir(paths).join(format!("{manifest_id}.json")))
}

/// Load one Blueprint manifest by embedded manifest id.
pub fn read_blueprint_manifest(
    paths: &WorkspacePaths,
    manifest_id: &str,
) -> QueueStoreResult<BlueprintManifestDocument> {
    let entries = blueprint_manifest_entries_for_id(paths, manifest_id)?;
    if entries.is_empty() {
        return Err(QueueStoreError::InvalidState {
            message: format!("blueprint_manifest_missing: {manifest_id}"),
        });
    }
    Ok(resolve_blueprint_manifest_entry(manifest_id, &entries)?.1)
}

/// Resolve one Blueprint manifest path by embedded manifest id.
pub fn resolve_blueprint_manifest_path(
    paths: &WorkspacePaths,
    manifest_id: &str,
) -> QueueStoreResult<PathBuf> {
    let entries = blueprint_manifest_entries_for_id(paths, manifest_id)?;
    if entries.is_empty() {
        return Err(QueueStoreError::InvalidState {
            message: format!("blueprint_manifest_missing: {manifest_id}"),
        });
    }
    Ok(resolve_blueprint_manifest_entry(manifest_id, &entries)?.0)
}

/// List all readable Blueprint manifests, grouped by embedded manifest id.
pub fn list_blueprint_manifests(
    paths: &WorkspacePaths,
) -> QueueStoreResult<Vec<BlueprintManifestDocument>> {
    let mut grouped: BTreeMap<String, Vec<ManifestEntry>> = BTreeMap::new();
    for path in list_json_files(&blueprint_manifest_dir(paths))? {
        let manifest = read_json_contract::<BlueprintManifestDocument>(&path)?;
        grouped
            .entry(manifest.manifest_id.clone())
            .or_default()
            .push((path, manifest));
    }
    let mut manifests = Vec::new();
    for (manifest_id, entries) in grouped {
        manifests.push(resolve_blueprint_manifest_entry(&manifest_id, &entries)?.1);
    }
    Ok(manifests)
}

/// List manifests for one root spec.
pub fn list_blueprint_manifests_for_root(
    paths: &WorkspacePaths,
    root_spec_id: &str,
) -> QueueStoreResult<Vec<BlueprintManifestDocument>> {
    Ok(list_blueprint_manifests(paths)?
        .into_iter()
        .filter(|manifest| manifest.root_spec_id == root_spec_id)
        .collect())
}

/// Enqueue one Blueprint draft.
pub fn enqueue_blueprint_draft(
    paths: &WorkspacePaths,
    draft: &BlueprintDraftDocument,
) -> QueueStoreResult<PathBuf> {
    ensure_unique_blueprint_draft(paths, &draft.draft_id)?;
    let mut queued = draft.clone();
    queued.status = BlueprintDraftStatus::Queued;
    write_unique_json_document(
        &draft_state_dir(paths, "queue").join(format!("{}.json", queued.draft_id)),
        &queued,
    )
}

/// Claim the next eligible Blueprint draft, respecting strict draft dependencies.
pub fn claim_next_blueprint_draft(
    paths: &WorkspacePaths,
    root_spec_id: Option<&str>,
) -> QueueStoreResult<Option<QueueClaim>> {
    let active = list_json_files(&draft_state_dir(paths, "active"))?;
    if active.len() > 1 {
        return Err(QueueStoreError::InvalidState {
            message: "multiple active blueprint drafts found".to_owned(),
        });
    }
    if !active.is_empty() {
        return Ok(None);
    }

    loop {
        let Some((mut draft, source)) = select_next_eligible_draft(paths, root_spec_id)? else {
            return Ok(None);
        };
        let destination =
            draft_state_dir(paths, "active").join(source.file_name().ok_or_else(|| {
                QueueStoreError::InvalidState {
                    message: "queued blueprint draft path is missing a filename".to_owned(),
                }
            })?);
        draft.status = BlueprintDraftStatus::Active;
        if !source.exists() {
            continue;
        }
        match write_unique_json_document(&destination, &draft) {
            Ok(path) => match fs::remove_file(&source) {
                Ok(()) => {
                    return Ok(Some(
                        QueueClaim::for_builtin(WorkItemKind::BlueprintDraft, draft.draft_id, path)
                            .with_source("queue", source)
                            .with_claim_policy("planning.default", 1),
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let _ = fs::remove_file(&destination);
                    continue;
                }
                Err(error) => return Err(QueueStoreError::io(&source, error)),
            },
            Err(QueueStoreError::InvalidState { message })
                if message.contains("already exists") =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Return the number of active Blueprint draft artifacts.
pub fn active_blueprint_draft_count(paths: &WorkspacePaths) -> QueueStoreResult<usize> {
    Ok(list_json_files(&draft_state_dir(paths, "active"))?.len())
}

/// Move an active Blueprint draft to approved.
pub fn approve_active_blueprint_draft(
    paths: &WorkspacePaths,
    draft_id: &str,
) -> QueueStoreResult<PathBuf> {
    move_blueprint_draft(
        paths,
        draft_id,
        "active",
        "approved",
        BlueprintDraftStatus::Approved,
    )
}

/// Move an active Blueprint draft to blocked.
pub fn block_active_blueprint_draft(
    paths: &WorkspacePaths,
    draft_id: &str,
) -> QueueStoreResult<PathBuf> {
    move_blueprint_draft(
        paths,
        draft_id,
        "active",
        "blocked",
        BlueprintDraftStatus::Blocked,
    )
}

/// Move an active Blueprint draft back to queue.
pub fn requeue_active_blueprint_draft(
    paths: &WorkspacePaths,
    draft_id: &str,
) -> QueueStoreResult<PathBuf> {
    move_blueprint_draft(
        paths,
        draft_id,
        "active",
        "queue",
        BlueprintDraftStatus::Queued,
    )
}

/// Cancel a queued, active, or blocked Blueprint draft.
pub fn cancel_blueprint_draft(paths: &WorkspacePaths, draft_id: &str) -> QueueStoreResult<PathBuf> {
    for source_state in ["queue", "active", "blocked"] {
        if draft_state_dir(paths, source_state)
            .join(format!("{draft_id}.json"))
            .exists()
        {
            return move_blueprint_draft(
                paths,
                draft_id,
                source_state,
                "canceled",
                BlueprintDraftStatus::Canceled,
            );
        }
    }
    Err(QueueStoreError::InvalidState {
        message: format!("blueprint draft {draft_id} not found"),
    })
}

/// Read an active Blueprint draft.
pub fn read_active_blueprint_draft(
    paths: &WorkspacePaths,
    draft_id: &str,
) -> QueueStoreResult<BlueprintDraftDocument> {
    read_blueprint_draft(&draft_state_dir(paths, "active").join(format!("{draft_id}.json")))
}

/// Read one Blueprint draft file.
pub fn read_blueprint_draft(path: &Path) -> QueueStoreResult<BlueprintDraftDocument> {
    read_json_contract(path)
}

/// Replace an active Blueprint draft, preserving active status.
pub fn update_active_blueprint_draft(
    paths: &WorkspacePaths,
    draft: &BlueprintDraftDocument,
) -> QueueStoreResult<PathBuf> {
    let destination = draft_state_dir(paths, "active").join(format!("{}.json", draft.draft_id));
    if !destination.exists() {
        return Err(QueueStoreError::InvalidState {
            message: format!("active blueprint draft {} not found", draft.draft_id),
        });
    }
    let mut active = draft.clone();
    active.status = BlueprintDraftStatus::Active;
    write_json_document(&destination, &active)
}

/// Persist a Blueprint packet in the requested packet state.
pub fn persist_blueprint_packet(
    paths: &WorkspacePaths,
    packet: &BlueprintPacketDocument,
    packet_state: &str,
) -> QueueStoreResult<PathBuf> {
    write_unique_json_document(
        &blueprints_dir(paths)
            .join("packets")
            .join(packet_state)
            .join(format!("{}.json", packet.blueprint_id)),
        packet,
    )
}

/// Move a candidate Blueprint packet to an evaluator-owned target state.
pub fn move_candidate_blueprint_packet(
    paths: &WorkspacePaths,
    blueprint_id: &str,
    target_state: &str,
) -> QueueStoreResult<PathBuf> {
    let source = blueprints_dir(paths)
        .join("packets")
        .join("candidates")
        .join(format!("{blueprint_id}.json"));
    if !source.exists() {
        return Err(QueueStoreError::InvalidState {
            message: format!("candidate blueprint packet {blueprint_id} not found"),
        });
    }
    let destination = blueprints_dir(paths)
        .join("packets")
        .join(target_state)
        .join(format!("{blueprint_id}.json"));
    move_unique_file(&source, &destination)
}

/// Persist a Blueprint critique.
pub fn persist_blueprint_critique(
    paths: &WorkspacePaths,
    critique: &BlueprintCritiqueDocument,
    critique_state: &str,
) -> QueueStoreResult<PathBuf> {
    write_unique_json_document(
        &blueprints_dir(paths)
            .join("critiques")
            .join(critique_state)
            .join(format!("{}.json", critique.critique_id)),
        critique,
    )
}

/// Persist a Blueprint evaluation.
pub fn persist_blueprint_evaluation(
    paths: &WorkspacePaths,
    evaluation: &BlueprintEvaluationDocument,
) -> QueueStoreResult<PathBuf> {
    write_unique_json_document(
        &blueprints_dir(paths)
            .join("evaluations")
            .join(format!("{}.json", evaluation.evaluation_id)),
        evaluation,
    )
}

/// Persist a Blueprint promotion record.
pub fn persist_blueprint_promotion(
    paths: &WorkspacePaths,
    promotion: &BlueprintPromotionRecord,
) -> QueueStoreResult<PathBuf> {
    write_unique_json_document(
        &blueprints_dir(paths)
            .join("promotions")
            .join(format!("{}.json", promotion.promotion_id)),
        promotion,
    )
}

/// Return a runtime-root-relative Blueprint artifact reference.
pub fn blueprint_artifact_ref(paths: &WorkspacePaths, path: &Path) -> QueueStoreResult<String> {
    path.strip_prefix(&paths.runtime_root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .map_err(|_| QueueStoreError::InvalidState {
            message: format!(
                "Blueprint artifact is outside runtime root: {}",
                path.display()
            ),
        })
}

/// Legacy closure-blocker ids for open Blueprint lineage work.
pub fn list_open_blueprint_lineage_work_ids(
    paths: &WorkspacePaths,
    root_spec_id: &str,
) -> QueueStoreResult<Vec<String>> {
    Ok(list_open_blueprint_lineage_work_refs(paths, root_spec_id)?
        .iter()
        .map(legacy_blocker_id)
        .collect())
}

/// Structured closure-blocker refs for open Blueprint lineage work.
pub fn list_open_blueprint_lineage_work_refs(
    paths: &WorkspacePaths,
    root_spec_id: &str,
) -> QueueStoreResult<Vec<ClosureBlockingWorkRef>> {
    let mut blockers = Vec::new();
    let mut seen = BTreeSet::new();

    for state in ["active", "queue", "blocked"] {
        for path in list_json_files(&draft_state_dir(paths, state))? {
            match read_blueprint_draft(&path) {
                Ok(draft) if draft.root_spec_id == root_spec_id => push_unique_ref(
                    &mut blockers,
                    &mut seen,
                    ClosureBlockingWorkRef {
                        blocker_type: "blueprint_draft".to_owned(),
                        reason: "open_blueprint_draft".to_owned(),
                        work_item_family_id: Some("blueprint_draft".to_owned()),
                        work_item_kind: Some(WorkItemKind::BlueprintDraft),
                        work_item_id: Some(draft.draft_id),
                        state: Some(state.to_owned()),
                        root_spec_id: Some(draft.root_spec_id),
                        root_idea_id: Some(draft.root_idea_id),
                        artifact_path: Some(runtime_relative(paths, &path)),
                        detail: None,
                    },
                ),
                Ok(_) => {}
                Err(_) => push_unique_ref(&mut blockers, &mut seen, invalid_ref(paths, &path)),
            }
        }
    }

    let mut approved_blueprints = BTreeMap::new();
    for path in list_json_files(&blueprints_dir(paths).join("packets/approved"))? {
        match read_json_contract::<BlueprintPacketDocument>(&path) {
            Ok(packet) if packet.root_spec_id == root_spec_id => {
                approved_blueprints.insert(
                    packet.blueprint_id,
                    (runtime_relative(paths, &path), packet.root_idea_id),
                );
            }
            Ok(_) => {}
            Err(_) => push_unique_ref(&mut blockers, &mut seen, invalid_ref(paths, &path)),
        }
    }

    let mut promoted_blueprint_ids = BTreeSet::new();
    for path in list_json_files(&blueprints_dir(paths).join("promotions"))? {
        match read_json_contract::<BlueprintPromotionRecord>(&path) {
            Ok(promotion) if promotion.root_spec_id == root_spec_id => {
                promoted_blueprint_ids.insert(promotion.blueprint_id.clone());
                if generated_task_done(paths, &promotion.generated_task_id)
                    || generated_task_open(paths, &promotion.generated_task_id)
                {
                    continue;
                }
                push_unique_ref(
                    &mut blockers,
                    &mut seen,
                    ClosureBlockingWorkRef {
                        blocker_type: "blueprint_promotion".to_owned(),
                        reason: "missing_generated_task".to_owned(),
                        work_item_family_id: Some("blueprint_promotion".to_owned()),
                        work_item_kind: None,
                        work_item_id: Some(promotion.promotion_id),
                        state: Some("promoted".to_owned()),
                        root_spec_id: Some(promotion.root_spec_id),
                        root_idea_id: Some(promotion.root_idea_id),
                        artifact_path: Some(runtime_relative(paths, &path)),
                        detail: Some(format!("generated_task_id={}", promotion.generated_task_id)),
                    },
                );
            }
            Ok(_) => {}
            Err(_) => push_unique_ref(&mut blockers, &mut seen, invalid_ref(paths, &path)),
        }
    }

    for (blueprint_id, (artifact_path, root_idea_id)) in approved_blueprints
        .iter()
        .filter(|(blueprint_id, _)| !promoted_blueprint_ids.contains(*blueprint_id))
    {
        push_unique_ref(
            &mut blockers,
            &mut seen,
            ClosureBlockingWorkRef {
                blocker_type: "blueprint_approved".to_owned(),
                reason: "missing_promotion".to_owned(),
                work_item_family_id: Some("blueprint_packet".to_owned()),
                work_item_kind: None,
                work_item_id: Some(blueprint_id.clone()),
                state: Some("approved".to_owned()),
                root_spec_id: Some(root_spec_id.to_owned()),
                root_idea_id: Some(root_idea_id.clone()),
                artifact_path: Some(artifact_path.clone()),
                detail: None,
            },
        );
    }

    for path in list_json_files(&blueprints_dir(paths).join("packets/candidates"))? {
        match read_json_contract::<BlueprintPacketDocument>(&path) {
            Ok(packet) if packet.root_spec_id == root_spec_id => push_unique_ref(
                &mut blockers,
                &mut seen,
                ClosureBlockingWorkRef {
                    blocker_type: "blueprint_candidate".to_owned(),
                    reason: "candidate_packet".to_owned(),
                    work_item_family_id: Some("blueprint_packet".to_owned()),
                    work_item_kind: None,
                    work_item_id: Some(packet.blueprint_id),
                    state: Some("candidates".to_owned()),
                    root_spec_id: Some(packet.root_spec_id),
                    root_idea_id: Some(packet.root_idea_id),
                    artifact_path: Some(runtime_relative(paths, &path)),
                    detail: None,
                },
            ),
            Ok(_) => {}
            Err(_) => push_unique_ref(&mut blockers, &mut seen, invalid_ref(paths, &path)),
        }
    }

    Ok(blockers)
}

fn blueprint_manifest_entries_for_id(
    paths: &WorkspacePaths,
    manifest_id: &str,
) -> QueueStoreResult<Vec<ManifestEntry>> {
    validate_safe_identifier(manifest_id, "manifest_id").map_err(invalid_state)?;
    let canonical = blueprint_manifest_path(paths, manifest_id)?;
    let mut entries = Vec::new();
    if canonical.exists() {
        let manifest = read_json_contract::<BlueprintManifestDocument>(&canonical)?;
        if manifest.manifest_id == manifest_id {
            entries.push((canonical.clone(), manifest));
        }
    }
    for path in list_json_files(&blueprint_manifest_dir(paths))? {
        if path == canonical {
            continue;
        }
        let Ok(manifest) = read_json_contract::<BlueprintManifestDocument>(&path) else {
            continue;
        };
        if manifest.manifest_id == manifest_id {
            entries.push((path, manifest));
        }
    }
    Ok(entries)
}

fn resolve_blueprint_manifest_entry(
    manifest_id: &str,
    entries: &[ManifestEntry],
) -> QueueStoreResult<ManifestEntry> {
    let mut normalized = BTreeSet::new();
    for (_path, manifest) in entries {
        normalized.insert(normalized_json(manifest)?);
    }
    if normalized.len() > 1 {
        return Err(QueueStoreError::InvalidState {
            message: duplicate_manifest_message(manifest_id, entries),
        });
    }
    if let Some(entry) = entries.iter().find(|(path, _manifest)| {
        path.file_stem().and_then(|value| value.to_str()) == Some(manifest_id)
    }) {
        return Ok(entry.clone());
    }
    entries
        .first()
        .cloned()
        .ok_or_else(|| QueueStoreError::InvalidState {
            message: format!("blueprint_manifest_missing: {manifest_id}"),
        })
}

fn duplicate_manifest_message(manifest_id: &str, entries: &[ManifestEntry]) -> String {
    let locations = entries
        .iter()
        .map(|(path, _manifest)| path.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "blueprint_manifest_duplicate: manifest_id={manifest_id} has divergent content in {locations}"
    )
}

fn select_next_eligible_draft(
    paths: &WorkspacePaths,
    root_spec_id: Option<&str>,
) -> QueueStoreResult<Option<(BlueprintDraftDocument, PathBuf)>> {
    let completed = completed_dependency_draft_ids(paths)?;
    let mut candidates = Vec::new();
    for path in list_json_files(&draft_state_dir(paths, "queue"))? {
        let draft = read_blueprint_draft(&path)?;
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if stem != draft.draft_id {
            return Err(QueueStoreError::InvalidState {
                message: format!(
                    "filename stem does not match draft_id: expected {}, found {stem}",
                    draft.draft_id
                ),
            });
        }
        if root_spec_id.is_some_and(|root| draft.root_spec_id != root) {
            continue;
        }
        if !draft
            .depends_on_draft_ids
            .iter()
            .all(|draft_id| completed.contains(draft_id))
        {
            continue;
        }
        candidates.push((
            draft.draft_index,
            draft.created_at.as_str().to_owned(),
            draft.draft_id.clone(),
            draft,
            path,
        ));
    }
    candidates
        .sort_by(|left, right| (&left.0, &left.1, &left.2).cmp(&(&right.0, &right.1, &right.2)));
    Ok(candidates
        .into_iter()
        .next()
        .map(|(_index, _created_at, _draft_id, draft, path)| (draft, path)))
}

fn move_blueprint_draft(
    paths: &WorkspacePaths,
    draft_id: &str,
    source_state: &str,
    target_state: &str,
    status: BlueprintDraftStatus,
) -> QueueStoreResult<PathBuf> {
    let source = draft_state_dir(paths, source_state).join(format!("{draft_id}.json"));
    if !source.exists() {
        return Err(QueueStoreError::InvalidState {
            message: format!("active blueprint draft {draft_id} not found"),
        });
    }
    let destination = draft_state_dir(paths, target_state).join(format!("{draft_id}.json"));
    if destination.exists() {
        return Err(QueueStoreError::InvalidState {
            message: format!("blueprint draft {draft_id} already exists"),
        });
    }
    let mut draft = read_blueprint_draft(&source)?;
    draft.status = status;
    write_unique_json_document(&destination, &draft)?;
    fs::remove_file(&source).map_err(|error| QueueStoreError::io(&source, error))?;
    Ok(destination)
}

fn ensure_unique_blueprint_draft(paths: &WorkspacePaths, draft_id: &str) -> QueueStoreResult<()> {
    validate_safe_identifier(draft_id, "draft_id").map_err(invalid_state)?;
    for state in [
        "queue",
        "active",
        "approved",
        "blocked",
        "canceled",
        "superseded",
    ] {
        if draft_state_dir(paths, state)
            .join(format!("{draft_id}.json"))
            .exists()
        {
            return Err(QueueStoreError::InvalidState {
                message: format!("blueprint draft {draft_id} already exists"),
            });
        }
    }
    Ok(())
}

fn completed_dependency_draft_ids(paths: &WorkspacePaths) -> QueueStoreResult<BTreeSet<String>> {
    let mut ids = BTreeSet::new();
    for state in ["approved", "canceled", "superseded"] {
        for path in list_json_files(&draft_state_dir(paths, state))? {
            if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
                ids.insert(stem.to_owned());
            }
        }
    }
    Ok(ids)
}

fn generated_task_done(paths: &WorkspacePaths, task_id: &str) -> bool {
    paths.tasks_done_dir.join(format!("{task_id}.md")).exists()
}

fn generated_task_open(paths: &WorkspacePaths, task_id: &str) -> bool {
    ["queue", "active", "blocked"].iter().any(|state| {
        let dir = match *state {
            "queue" => &paths.tasks_queue_dir,
            "active" => &paths.tasks_active_dir,
            _ => &paths.tasks_blocked_dir,
        };
        dir.join(format!("{task_id}.md")).exists()
    })
}

fn invalid_ref(paths: &WorkspacePaths, path: &Path) -> ClosureBlockingWorkRef {
    ClosureBlockingWorkRef {
        blocker_type: "blueprint_invalid".to_owned(),
        reason: "invalid_artifact".to_owned(),
        work_item_family_id: None,
        work_item_kind: None,
        work_item_id: None,
        state: None,
        root_spec_id: None,
        root_idea_id: None,
        artifact_path: Some(runtime_relative(paths, path)),
        detail: None,
    }
}

fn legacy_blocker_id(reference: &ClosureBlockingWorkRef) -> String {
    match (
        reference.blocker_type.as_str(),
        reference.work_item_id.as_deref(),
    ) {
        ("blueprint_draft", Some(id)) => format!("blueprint_draft:{id}"),
        ("blueprint_candidate", Some(id)) => format!("blueprint_candidate:{id}"),
        ("blueprint_promotion", Some(id)) => {
            format!("blueprint_promotion:{id}:missing_generated_task")
        }
        ("blueprint_approved", Some(id)) => format!("blueprint_approved:{id}:missing_promotion"),
        ("blueprint_invalid", _) => reference
            .artifact_path
            .as_ref()
            .map(|path| format!("blueprint_invalid:{path}"))
            .unwrap_or_else(|| "blueprint_invalid".to_owned()),
        (_, Some(id)) => id.to_owned(),
        _ => reference
            .artifact_path
            .clone()
            .unwrap_or_else(|| "blueprint_invalid".to_owned()),
    }
}

fn push_unique_ref(
    refs: &mut Vec<ClosureBlockingWorkRef>,
    seen: &mut BTreeSet<(String, Option<String>, String, Option<String>)>,
    reference: ClosureBlockingWorkRef,
) {
    let key = (
        reference.blocker_type.clone(),
        reference.work_item_id.clone(),
        reference.reason.clone(),
        reference.artifact_path.clone(),
    );
    if seen.insert(key) {
        refs.push(reference);
    }
}

fn read_json_contract<T>(path: &Path) -> QueueStoreResult<T>
where
    T: RuntimeJsonContract,
{
    let raw = fs::read_to_string(path).map_err(|error| QueueStoreError::io(path, error))?;
    T::from_json_str(&raw).map_err(|error| QueueStoreError::InvalidState {
        message: format!("Blueprint artifact {} is invalid: {error}", path.display()),
    })
}

fn validate_model<T>(document: &T) -> QueueStoreResult<()>
where
    T: RuntimeJsonContract + Clone,
{
    let mut copy = document.clone();
    copy.validate_contract()
        .map_err(|error| QueueStoreError::InvalidState {
            message: error.to_string(),
        })
}

fn write_unique_json_document<T>(destination: &Path, document: &T) -> QueueStoreResult<PathBuf>
where
    T: RuntimeJsonContract + Serialize + Clone,
{
    if destination.exists() {
        return Err(QueueStoreError::InvalidState {
            message: format!(
                "Blueprint artifact already exists: {}",
                destination.display()
            ),
        });
    }
    write_json_document(destination, document)
}

fn write_json_document<T>(destination: &Path, document: &T) -> QueueStoreResult<PathBuf>
where
    T: RuntimeJsonContract + Serialize + Clone,
{
    validate_model(document)?;
    let value = serde_json::to_value(document).map_err(|error| QueueStoreError::InvalidState {
        message: error.to_string(),
    })?;
    let payload = serde_json::to_string_pretty(&sort_json_value(value)).map_err(|error| {
        QueueStoreError::InvalidState {
            message: error.to_string(),
        }
    })? + "\n";
    write_text_atomically(destination, &payload)?;
    Ok(destination.to_path_buf())
}

fn move_unique_file(source: &Path, destination: &Path) -> QueueStoreResult<PathBuf> {
    if destination.exists() {
        return Err(QueueStoreError::InvalidState {
            message: format!(
                "Blueprint artifact already exists: {}",
                destination.display()
            ),
        });
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| QueueStoreError::io(parent, error))?;
    }
    fs::rename(source, destination).map_err(|error| QueueStoreError::io(source, error))?;
    Ok(destination.to_path_buf())
}

fn write_text_atomically(destination: &Path, payload: &str) -> QueueStoreResult<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| QueueStoreError::io(parent, error))?;
    }
    let temp = destination.with_file_name(format!(
        ".{}.tmp",
        destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("blueprint")
    ));
    fs::write(&temp, payload).map_err(|error| QueueStoreError::io(&temp, error))?;
    fs::rename(&temp, destination).map_err(|error| QueueStoreError::io(destination, error))
}

fn normalized_json<T>(document: &T) -> QueueStoreResult<String>
where
    T: Serialize,
{
    let value = serde_json::to_value(document).map_err(|error| QueueStoreError::InvalidState {
        message: error.to_string(),
    })?;
    serde_json::to_string(&sort_json_value(value)).map_err(|error| QueueStoreError::InvalidState {
        message: error.to_string(),
    })
}

fn sort_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted = map
                .into_iter()
                .map(|(key, value)| (key, sort_json_value(value)))
                .collect::<Map<_, _>>();
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(sort_json_value).collect()),
        value => value,
    }
}

fn list_json_files(directory: &Path) -> QueueStoreResult<Vec<PathBuf>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| QueueStoreError::io(directory, error))? {
        let entry = entry.map_err(|error| QueueStoreError::io(directory, error))?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn runtime_relative(paths: &WorkspacePaths, path: &Path) -> String {
    path.strip_prefix(&paths.runtime_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn blueprints_dir(paths: &WorkspacePaths) -> PathBuf {
    paths.runtime_root.join("blueprints")
}

fn blueprint_manifest_dir(paths: &WorkspacePaths) -> PathBuf {
    blueprints_dir(paths).join("manifests")
}

fn draft_state_dir(paths: &WorkspacePaths, state: &str) -> PathBuf {
    blueprints_dir(paths).join("drafts").join(state)
}

fn invalid_state(error: impl ToString) -> QueueStoreError {
    QueueStoreError::InvalidState {
        message: error.to_string(),
    }
}
