//! Runtime effect handlers for the Blueprint Planning loop.

use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::Value;

use crate::{
    compiler::CompiledRunPlan,
    contracts::{
        ArtifactFormat, BlueprintCritiqueDocument, BlueprintDraftDocument,
        BlueprintEvaluationDecision, BlueprintEvaluationDocument, BlueprintManifestDocument,
        BlueprintPacketDocument, BlueprintPromotionRecord, BlueprintSourceWorkItemKind,
        RuntimeEffectMutationPhase, RuntimeEffectRuleDefinition, RuntimeJsonContract,
        RuntimeJsonError, StageResultEnvelope, TaskDocument, WorkItemKind,
    },
    runtime::RuntimeStartupSession,
    work_documents::{parse_task_document_with_source, parse_task_json_import_with_source},
    workspace::{
        QueueStoreError, SourceLifecycleAction, SourceLifecycleIntent, WorkspacePaths,
        enqueue_blueprint_draft, enqueue_task, move_candidate_blueprint_packet,
        persist_blueprint_critique, persist_blueprint_evaluation, persist_blueprint_packet,
        persist_blueprint_promotion, read_active_blueprint_draft, read_blueprint_draft,
        read_blueprint_manifest, update_active_blueprint_draft, write_blueprint_manifest,
    },
};

use super::{
    RuntimeEffectDecision, RuntimeEffectResult, RuntimeTickResult,
    lifecycle::source_lifecycle_intent_for_effect,
};

pub(crate) const MANAGER_BLUEPRINT_HANDLER_ID: &str =
    "manager_blueprint_manifest_to_blueprint_drafts";
pub(crate) const CONTRACTOR_BLUEPRINT_HANDLER_ID: &str = "contractor_blueprint_candidate_persist";
pub(crate) const EVALUATOR_BLUEPRINT_APPROVAL_HANDLER_ID: &str =
    "evaluator_blueprint_approved_to_task";
pub(crate) const EVALUATOR_BLUEPRINT_REJECTION_HANDLER_ID: &str =
    "evaluator_blueprint_rejected_to_draft_revision";

/// Dispatch a packaged Blueprint runtime-effect handler.
pub(crate) fn run_blueprint_effect_handler(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    run_dir: &Path,
    rule: &RuntimeEffectRuleDefinition,
) -> RuntimeTickResult<Option<RuntimeEffectResult>> {
    let result = match rule.handler_id.as_str() {
        MANAGER_BLUEPRINT_HANDLER_ID => {
            manager_blueprint_manifest_to_blueprint_drafts(&session.paths, stage_result, run_dir)
        }
        CONTRACTOR_BLUEPRINT_HANDLER_ID => {
            contractor_blueprint_candidate_persist(&session.paths, stage_result, run_dir)
        }
        EVALUATOR_BLUEPRINT_APPROVAL_HANDLER_ID => evaluator_blueprint_approved_to_task(
            &session.paths,
            &session.compiled_plan,
            stage_result,
            run_dir,
        ),
        EVALUATOR_BLUEPRINT_REJECTION_HANDLER_ID => evaluator_blueprint_rejected_to_draft_revision(
            &session.paths,
            &session.compiled_plan,
            stage_result,
            run_dir,
        ),
        _ => return Ok(None),
    };
    Ok(Some(result))
}

fn manager_blueprint_manifest_to_blueprint_drafts(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
    run_dir: &Path,
) -> RuntimeEffectResult {
    let mut created_paths = Vec::new();
    let manifest = match read_manager_json_model::<BlueprintManifestDocument>(
        &run_dir.join("blueprint_manifest.json"),
        "blueprint_manifest_missing",
        "blueprint_manifest_parse_error",
        "blueprint_manifest_schema_invalid",
    ) {
        Ok(manifest) => manifest,
        Err(error) => {
            return manager_failure(
                stage_result,
                error.class,
                error.message,
                created_paths,
                true,
            );
        }
    };
    let drafts = match read_manager_json_model_list::<BlueprintDraftDocument>(
        &run_dir.join("blueprint_drafts.json"),
        "blueprint_drafts_missing",
        "blueprint_drafts_parse_error",
        "blueprint_drafts_schema_invalid",
    ) {
        Ok(drafts) => drafts,
        Err(error) => {
            return manager_failure(
                stage_result,
                error.class,
                error.message,
                created_paths,
                true,
            );
        }
    };
    if let Err(message) = validate_manager_output(stage_result, &manifest, &drafts) {
        return manager_failure(
            stage_result,
            "blueprint_manifest_draft_mismatch".to_owned(),
            message,
            created_paths,
            true,
        );
    }

    let manifest_exists = match manager_manifest_exists_equivalent(paths, &manifest) {
        Ok(exists) => exists,
        Err(error) => {
            return manager_failure(
                stage_result,
                error.class,
                error.message,
                created_paths,
                true,
            );
        }
    };
    let mut existing_draft_ids = BTreeSet::new();
    for draft in &drafts {
        match manager_draft_exists_equivalent(paths, draft) {
            Ok(true) => {
                existing_draft_ids.insert(draft.draft_id.clone());
            }
            Ok(false) => {}
            Err(error) => {
                return manager_failure(
                    stage_result,
                    error.class,
                    error.message,
                    created_paths,
                    true,
                );
            }
        }
    }

    let source_state = manager_source_lifecycle_state(paths, stage_result);
    let all_outputs_exist = manifest_exists && existing_draft_ids.len() == drafts.len();
    if all_outputs_exist {
        return match source_state {
            SourceState::Active => manager_success(
                created_paths,
                Some(source_lifecycle_intent_for_effect(
                    stage_result,
                    complete_lifecycle_plan_id(stage_result.work_item_kind),
                    SourceLifecycleAction::Complete,
                )),
                format!("queued {} blueprint draft(s)", drafts.len()),
            ),
            SourceState::Target => manager_success(
                created_paths,
                None,
                format!(
                    "blueprint draft output already exists for {}",
                    manifest.manifest_id
                ),
            ),
            SourceState::Invalid => manager_failure(
                stage_result,
                "blueprint_source_lifecycle_invalid".to_owned(),
                format!(
                    "source work item is not active: {}",
                    stage_result.work_item_id
                ),
                created_paths,
                false,
            ),
        };
    }
    if source_state != SourceState::Active {
        return manager_failure(
            stage_result,
            "blueprint_source_lifecycle_invalid".to_owned(),
            format!(
                "source work item is not active: {}",
                stage_result.work_item_id
            ),
            created_paths,
            false,
        );
    }

    if !manifest_exists {
        match write_blueprint_manifest(paths, &manifest) {
            Ok(path) => created_paths.push(effect_path(paths, &path)),
            Err(error) => {
                return manager_failure(
                    stage_result,
                    manager_write_failure_class(&error, &created_paths),
                    error.to_string(),
                    created_paths,
                    true,
                );
            }
        }
    }
    for draft in &drafts {
        if existing_draft_ids.contains(&draft.draft_id) {
            continue;
        }
        match enqueue_blueprint_draft(paths, draft) {
            Ok(path) => created_paths.push(effect_path(paths, &path)),
            Err(error) => {
                return manager_failure(
                    stage_result,
                    manager_write_failure_class(&error, &created_paths),
                    error.to_string(),
                    created_paths,
                    true,
                );
            }
        }
    }

    manager_success(
        created_paths,
        Some(source_lifecycle_intent_for_effect(
            stage_result,
            complete_lifecycle_plan_id(stage_result.work_item_kind),
            SourceLifecycleAction::Complete,
        )),
        format!("queued {} blueprint draft(s)", drafts.len()),
    )
}

fn contractor_blueprint_candidate_persist(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
    run_dir: &Path,
) -> RuntimeEffectResult {
    let mut created_paths = Vec::new();
    let result = (|| -> Result<RuntimeEffectResult, String> {
        let mut draft = active_draft_for_stage_result(paths, stage_result)
            .map_err(|error| error.to_string())?;
        let packet =
            read_json_model::<BlueprintPacketDocument>(&run_dir.join("blueprint_packet.json"))
                .map_err(|error| error.to_string())?;
        packet
            .ensure_matches_draft(&draft)
            .map_err(|error| error.to_string())?;
        validate_packet_critique_reference(&draft, &packet)?;

        let packet_path = persist_blueprint_packet(paths, &packet, "candidates")
            .map_err(|error| error.to_string())?;
        created_paths.push(effect_path(paths, &packet_path));
        let markdown_path = persist_candidate_markdown(paths, &packet.blueprint_id, run_dir)
            .map_err(|error| error.to_string())?;
        created_paths.push(effect_path(paths, &markdown_path));

        draft.latest_blueprint_id = Some(packet.blueprint_id.clone());
        update_active_blueprint_draft(paths, &draft).map_err(|error| error.to_string())?;
        Ok(RuntimeEffectResult {
            handler_id: CONTRACTOR_BLUEPRINT_HANDLER_ID.to_owned(),
            decision: RuntimeEffectDecision::ContinueRoute,
            created_paths: created_paths.clone(),
            source_lifecycle_intent: None,
            failure_class: None,
            message: Some(format!(
                "persisted candidate blueprint {}",
                packet.blueprint_id
            )),
            mutation_phase: RuntimeEffectMutationPhase::Unknown,
        })
    })();

    result.unwrap_or_else(|message| {
        blueprint_failure(
            CONTRACTOR_BLUEPRINT_HANDLER_ID,
            stage_result,
            if created_paths.is_empty() {
                "blueprint_candidate_invalid"
            } else {
                "blueprint_partial_mutation"
            },
            message,
            created_paths,
        )
    })
}

fn evaluator_blueprint_approved_to_task(
    paths: &WorkspacePaths,
    compiled_plan: &CompiledRunPlan,
    stage_result: &StageResultEnvelope,
    run_dir: &Path,
) -> RuntimeEffectResult {
    let mut created_paths = Vec::new();
    let result = (|| -> Result<RuntimeEffectResult, BlueprintEffectError> {
        let draft = active_draft_for_stage_result(paths, stage_result).map_err(|error| {
            BlueprintEffectError::new("blueprint_evaluation_invalid", error.to_string())
        })?;
        let packet = candidate_packet_for_draft(paths, &draft).map_err(|error| {
            BlueprintEffectError::new("blueprint_evaluation_invalid", error.to_string())
        })?;
        let evaluation = read_json_model::<BlueprintEvaluationDocument>(
            &run_dir.join("blueprint_evaluation.json"),
        )
        .map_err(|error| {
            BlueprintEffectError::new("blueprint_evaluation_invalid", error.to_string())
        })?;
        validate_approval(&evaluation, &packet).map_err(|message| {
            BlueprintEffectError::new("blueprint_evaluation_invalid", message)
        })?;
        let mut task = read_generated_task(compiled_plan, run_dir)?;
        validate_generated_task(&task, &draft, &packet)?;
        ensure_task_id_unused(paths, &task.task_id)?;
        ensure_promotion_id_unused(paths, &evaluation.evaluation_id)?;

        let evaluation_path = persist_blueprint_evaluation(paths, &evaluation)?;
        created_paths.push(effect_path(paths, &evaluation_path));
        let approved_packet_path =
            move_candidate_blueprint_packet(paths, &packet.blueprint_id, "approved")?;
        created_paths.push(effect_path(paths, &approved_packet_path));
        if let Some(markdown_path) =
            move_candidate_markdown(paths, &packet.blueprint_id, "approved")?
        {
            created_paths.push(effect_path(paths, &markdown_path));
        }
        add_unique_ref(
            &mut task.references,
            effect_path(paths, &approved_packet_path),
        );
        add_unique_ref(&mut task.references, effect_path(paths, &evaluation_path));
        let task_path = enqueue_task(paths, &task)?;
        created_paths.push(effect_path(paths, &task_path));
        let promotion = BlueprintPromotionRecord {
            schema_version: "1.0".to_owned(),
            kind: "blueprint_promotion".to_owned(),
            promotion_id: promotion_id(&evaluation.evaluation_id),
            blueprint_id: packet.blueprint_id.clone(),
            evaluation_id: evaluation.evaluation_id.clone(),
            draft_id: draft.draft_id.clone(),
            manifest_id: draft.manifest_id.clone(),
            root_spec_id: draft.root_spec_id.clone(),
            root_idea_id: draft.root_idea_id.clone(),
            generated_task_id: task.task_id.clone(),
            generated_task_path: effect_path(paths, &task_path),
            approved_blueprint_path: effect_path(paths, &approved_packet_path),
            evaluation_path: effect_path(paths, &evaluation_path),
            promoted_at: evaluation.created_at.clone(),
            promoted_by: "runtime".to_owned(),
        };
        let promotion_path = persist_blueprint_promotion(paths, &promotion)?;
        created_paths.push(effect_path(paths, &promotion_path));
        Ok(RuntimeEffectResult {
            handler_id: EVALUATOR_BLUEPRINT_APPROVAL_HANDLER_ID.to_owned(),
            decision: RuntimeEffectDecision::RequestCompleteSource,
            created_paths: created_paths.clone(),
            source_lifecycle_intent: Some(SourceLifecycleIntent::for_builtin(
                "approve_blueprint_draft_after_effect",
                SourceLifecycleAction::Complete,
                WorkItemKind::BlueprintDraft,
                draft.draft_id.clone(),
            )),
            failure_class: None,
            message: Some(format!(
                "promoted blueprint {} to task {}",
                packet.blueprint_id, task.task_id
            )),
            mutation_phase: RuntimeEffectMutationPhase::Unknown,
        })
    })();

    result.unwrap_or_else(|error| {
        blueprint_failure(
            EVALUATOR_BLUEPRINT_APPROVAL_HANDLER_ID,
            stage_result,
            approval_failure_class(&error, &created_paths),
            error.message,
            created_paths,
        )
    })
}

fn evaluator_blueprint_rejected_to_draft_revision(
    paths: &WorkspacePaths,
    compiled_plan: &CompiledRunPlan,
    stage_result: &StageResultEnvelope,
    run_dir: &Path,
) -> RuntimeEffectResult {
    let mut created_paths = Vec::new();
    let result = (|| -> Result<RuntimeEffectResult, BlueprintEffectError> {
        let mut draft = active_draft_for_stage_result(paths, stage_result).map_err(|error| {
            BlueprintEffectError::new("blueprint_critique_invalid", error.to_string())
        })?;
        let packet = candidate_packet_for_draft(paths, &draft).map_err(|error| {
            BlueprintEffectError::new("blueprint_critique_invalid", error.to_string())
        })?;
        let evaluation = read_json_model::<BlueprintEvaluationDocument>(
            &run_dir.join("blueprint_evaluation.json"),
        )
        .map_err(|error| {
            BlueprintEffectError::new("blueprint_critique_invalid", error.to_string())
        })?;
        let critique = read_blueprint_critique(compiled_plan, run_dir)?;
        validate_rejection(&evaluation, &critique, &packet)?;

        let evaluation_path = persist_blueprint_evaluation(paths, &evaluation)?;
        created_paths.push(effect_path(paths, &evaluation_path));
        let rejected_packet_path =
            move_candidate_blueprint_packet(paths, &packet.blueprint_id, "rejected")?;
        created_paths.push(effect_path(paths, &rejected_packet_path));
        if let Some(markdown_path) =
            move_candidate_markdown(paths, &packet.blueprint_id, "rejected")?
        {
            created_paths.push(effect_path(paths, &markdown_path));
        }
        let critique_path = persist_blueprint_critique(paths, &critique, "open")?;
        created_paths.push(effect_path(paths, &critique_path));

        draft.current_revision = packet.revision;
        draft.latest_critique_id = Some(critique.critique_id.clone());
        draft.latest_blueprint_id = Some(packet.blueprint_id.clone());
        draft.updated_at = Some(evaluation.created_at.clone());
        update_active_blueprint_draft(paths, &draft)?;
        Ok(RuntimeEffectResult {
            handler_id: EVALUATOR_BLUEPRINT_REJECTION_HANDLER_ID.to_owned(),
            decision: RuntimeEffectDecision::ContinueRoute,
            created_paths: created_paths.clone(),
            source_lifecycle_intent: None,
            failure_class: None,
            message: Some(format!(
                "recorded rejection critique {}",
                critique.critique_id
            )),
            mutation_phase: RuntimeEffectMutationPhase::Unknown,
        })
    })();

    result.unwrap_or_else(|error| {
        blueprint_failure(
            EVALUATOR_BLUEPRINT_REJECTION_HANDLER_ID,
            stage_result,
            if created_paths.is_empty() {
                error.class.as_str()
            } else {
                "blueprint_partial_mutation"
            },
            error.message,
            created_paths,
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlueprintEffectError {
    class: String,
    message: String,
}

impl BlueprintEffectError {
    fn new(class: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            class: class.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for BlueprintEffectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.class, self.message)
    }
}

impl From<QueueStoreError> for BlueprintEffectError {
    fn from(value: QueueStoreError) -> Self {
        Self::new("blueprint_state_error", value.to_string())
    }
}

fn read_manager_json_model<T>(
    path: &Path,
    missing_class: &str,
    parse_class: &str,
    schema_class: &str,
) -> Result<T, BlueprintEffectError>
where
    T: RuntimeJsonContract,
{
    if !path.exists() {
        return Err(BlueprintEffectError::new(
            missing_class,
            format!(
                "required Blueprint artifact is missing: {}",
                path_name(path)
            ),
        ));
    }
    let raw = fs::read_to_string(path).map_err(|error| {
        BlueprintEffectError::new(
            missing_class,
            format!(
                "required Blueprint artifact could not be read: {}: {error}",
                path_name(path)
            ),
        )
    })?;
    T::from_json_str(&raw).map_err(|error| {
        let class = if matches!(error, RuntimeJsonError::Json { .. }) {
            parse_class
        } else {
            schema_class
        };
        BlueprintEffectError::new(
            class,
            format!("{} failed schema validation: {error}", path_name(path)),
        )
    })
}

fn read_manager_json_model_list<T>(
    path: &Path,
    missing_class: &str,
    parse_class: &str,
    schema_class: &str,
) -> Result<Vec<T>, BlueprintEffectError>
where
    T: RuntimeJsonContract,
{
    if !path.exists() {
        return Err(BlueprintEffectError::new(
            missing_class,
            format!(
                "required Blueprint artifact is missing: {}",
                path_name(path)
            ),
        ));
    }
    let raw = fs::read_to_string(path).map_err(|error| {
        BlueprintEffectError::new(
            missing_class,
            format!(
                "required Blueprint artifact could not be read: {}: {error}",
                path_name(path)
            ),
        )
    })?;
    let payload: Value = serde_json::from_str(&raw).map_err(|error| {
        BlueprintEffectError::new(
            parse_class,
            format!("{} is not valid JSON: {error}", path_name(path)),
        )
    })?;
    let Value::Array(values) = payload else {
        return Err(BlueprintEffectError::new(
            schema_class,
            format!("{} must be a JSON list", path_name(path)),
        ));
    };
    values
        .into_iter()
        .map(|value| {
            T::from_json_value(value).map_err(|error| {
                BlueprintEffectError::new(
                    schema_class,
                    format!("{} failed schema validation: {error}", path_name(path)),
                )
            })
        })
        .collect()
}

fn read_json_model<T>(path: &Path) -> Result<T, BlueprintEffectError>
where
    T: RuntimeJsonContract,
{
    if !path.exists() {
        return Err(BlueprintEffectError::new(
            "artifact_missing",
            format!(
                "required Blueprint artifact is missing: {}",
                path_name(path)
            ),
        ));
    }
    let raw = fs::read_to_string(path).map_err(|error| {
        BlueprintEffectError::new("artifact_missing", format!("{}: {error}", path.display()))
    })?;
    T::from_json_str(&raw)
        .map_err(|error| BlueprintEffectError::new("json_model_parse", error.to_string()))
}

fn validate_manager_output(
    stage_result: &StageResultEnvelope,
    manifest: &BlueprintManifestDocument,
    drafts: &[BlueprintDraftDocument],
) -> Result<(), String> {
    let expected_kind = match stage_result.work_item_kind {
        WorkItemKind::Spec => BlueprintSourceWorkItemKind::Spec,
        WorkItemKind::Incident => BlueprintSourceWorkItemKind::Incident,
        _ => return Err("manifest source_work_item_kind does not match active source".to_owned()),
    };
    if manifest.source_work_item_kind != expected_kind {
        return Err("manifest source_work_item_kind does not match active source".to_owned());
    }
    if manifest.source_work_item_id != stage_result.work_item_id {
        return Err("manifest source_work_item_id does not match active source".to_owned());
    }
    if drafts
        .iter()
        .map(|draft| draft.draft_id.clone())
        .collect::<Vec<_>>()
        != manifest.draft_ids
    {
        return Err("draft order must match manifest draft_ids".to_owned());
    }
    if drafts.len() as u64 != manifest.draft_count {
        return Err("draft count must match manifest".to_owned());
    }
    let mut previous_ids = BTreeSet::new();
    let mut previous_id: Option<String> = None;
    for (index, draft) in drafts.iter().enumerate() {
        let expected_index = (index + 1) as u64;
        if draft.draft_index != expected_index {
            return Err("draft indexes must be contiguous starting at 1".to_owned());
        }
        ensure_draft_matches_manifest(draft, manifest)?;
        if draft.draft_index == 1 && !draft.depends_on_draft_ids.is_empty() {
            return Err("first Blueprint draft cannot declare dependencies".to_owned());
        }
        if draft.draft_index > 1
            && draft.depends_on_draft_ids != vec![previous_id.clone().unwrap_or_default()]
        {
            return Err(
                "strict Blueprint sequence requires dependency on previous draft".to_owned(),
            );
        }
        if !draft
            .depends_on_draft_ids
            .iter()
            .all(|id| previous_ids.contains(id))
        {
            return Err("Blueprint draft dependencies must refer to earlier drafts".to_owned());
        }
        previous_ids.insert(draft.draft_id.clone());
        previous_id = Some(draft.draft_id.clone());
    }
    Ok(())
}

fn ensure_draft_matches_manifest(
    draft: &BlueprintDraftDocument,
    manifest: &BlueprintManifestDocument,
) -> Result<(), String> {
    if draft.manifest_id != manifest.manifest_id {
        return Err("draft manifest_id does not match manifest".to_owned());
    }
    if draft.root_spec_id != manifest.root_spec_id {
        return Err("draft root_spec_id does not match manifest".to_owned());
    }
    if draft.root_idea_id != manifest.root_idea_id {
        return Err("draft root_idea_id does not match manifest".to_owned());
    }
    if draft.source_spec_id != manifest.source_spec_id {
        return Err("draft source_spec_id does not match manifest".to_owned());
    }
    Ok(())
}

fn manager_manifest_exists_equivalent(
    paths: &WorkspacePaths,
    manifest: &BlueprintManifestDocument,
) -> Result<bool, BlueprintEffectError> {
    match read_blueprint_manifest(paths, &manifest.manifest_id) {
        Ok(existing) => {
            if normalized_json(&existing)? != normalized_json(manifest)? {
                return Err(BlueprintEffectError::new(
                    "blueprint_manifest_duplicate",
                    format!(
                        "blueprint_manifest_duplicate: manifest_id={}",
                        manifest.manifest_id
                    ),
                ));
            }
            Ok(true)
        }
        Err(error) if error.to_string().contains("blueprint_manifest_missing") => Ok(false),
        Err(error) => Err(BlueprintEffectError::new(
            "blueprint_manifest_duplicate",
            error.to_string(),
        )),
    }
}

fn manager_draft_exists_equivalent(
    paths: &WorkspacePaths,
    draft: &BlueprintDraftDocument,
) -> Result<bool, BlueprintEffectError> {
    let mut entries = Vec::new();
    for state in [
        "queue",
        "active",
        "approved",
        "blocked",
        "canceled",
        "superseded",
    ] {
        let path = paths
            .runtime_root
            .join("blueprints")
            .join("drafts")
            .join(state)
            .join(format!("{}.json", draft.draft_id));
        if !path.exists() {
            continue;
        }
        let existing = read_blueprint_draft(&path).map_err(|error| {
            BlueprintEffectError::new(
                "blueprint_draft_duplicate",
                format!(
                    "existing draft {} cannot be validated: {error}",
                    draft.draft_id
                ),
            )
        })?;
        if normalized_draft_identity(&existing)? != normalized_draft_identity(draft)? {
            return Err(BlueprintEffectError::new(
                "blueprint_draft_duplicate",
                format!("blueprint_draft_duplicate: draft_id={}", draft.draft_id),
            ));
        }
        entries.push(path);
    }
    if entries.len() > 1 {
        let locations = entries
            .iter()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(BlueprintEffectError::new(
            "blueprint_draft_duplicate",
            format!(
                "blueprint_draft_duplicate: draft_id={} in {locations}",
                draft.draft_id
            ),
        ));
    }
    Ok(!entries.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceState {
    Active,
    Target,
    Invalid,
}

fn manager_source_lifecycle_state(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
) -> SourceState {
    match stage_result.work_item_kind {
        WorkItemKind::Spec => {
            if paths
                .specs_active_dir
                .join(format!("{}.md", stage_result.work_item_id))
                .exists()
            {
                SourceState::Active
            } else if paths
                .specs_done_dir
                .join(format!("{}.md", stage_result.work_item_id))
                .exists()
            {
                SourceState::Target
            } else {
                SourceState::Invalid
            }
        }
        WorkItemKind::Incident => {
            if paths
                .incidents_active_dir
                .join(format!("{}.md", stage_result.work_item_id))
                .exists()
            {
                SourceState::Active
            } else if paths
                .incidents_resolved_dir
                .join(format!("{}.md", stage_result.work_item_id))
                .exists()
            {
                SourceState::Target
            } else {
                SourceState::Invalid
            }
        }
        _ => SourceState::Invalid,
    }
}

fn manager_success(
    created_paths: Vec<String>,
    source_lifecycle_intent: Option<SourceLifecycleIntent>,
    message: String,
) -> RuntimeEffectResult {
    RuntimeEffectResult {
        handler_id: MANAGER_BLUEPRINT_HANDLER_ID.to_owned(),
        decision: RuntimeEffectDecision::RequestCompleteSource,
        created_paths,
        source_lifecycle_intent,
        failure_class: None,
        message: Some(message),
        mutation_phase: RuntimeEffectMutationPhase::Unknown,
    }
}

fn manager_failure(
    stage_result: &StageResultEnvelope,
    failure_class: String,
    message: String,
    created_paths: Vec<String>,
    include_source_lifecycle_intent: bool,
) -> RuntimeEffectResult {
    RuntimeEffectResult {
        handler_id: MANAGER_BLUEPRINT_HANDLER_ID.to_owned(),
        decision: RuntimeEffectDecision::RequestBlockSource,
        created_paths: created_paths.clone(),
        source_lifecycle_intent: include_source_lifecycle_intent.then(|| {
            source_lifecycle_intent_for_effect(
                stage_result,
                block_lifecycle_plan_id(stage_result.work_item_kind),
                SourceLifecycleAction::Block,
            )
        }),
        failure_class: Some(failure_class),
        message: Some(message),
        mutation_phase: if created_paths.is_empty() {
            RuntimeEffectMutationPhase::PreMutation
        } else {
            RuntimeEffectMutationPhase::PartialMutation
        },
    }
}

fn manager_write_failure_class(error: &QueueStoreError, created_paths: &[String]) -> String {
    if !created_paths.is_empty() {
        return "blueprint_partial_mutation".to_owned();
    }
    let message = error.to_string();
    if message.contains("blueprint_manifest_duplicate")
        || (message.contains("Blueprint artifact already exists") && message.contains("manifests"))
    {
        return "blueprint_manifest_duplicate".to_owned();
    }
    if (message.contains("blueprint draft") && message.contains("already exists"))
        || message.contains("blueprint_draft_duplicate")
    {
        return "blueprint_draft_duplicate".to_owned();
    }
    "blueprint_partial_mutation".to_owned()
}

fn active_draft_for_stage_result(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
) -> Result<BlueprintDraftDocument, QueueStoreError> {
    if stage_result.work_item_kind != WorkItemKind::BlueprintDraft {
        return Err(QueueStoreError::InvalidState {
            message: format!(
                "Blueprint handler requires blueprint_draft source, got {}",
                stage_result.work_item_kind.as_str()
            ),
        });
    }
    read_active_blueprint_draft(paths, &stage_result.work_item_id)
}

fn validate_packet_critique_reference(
    draft: &BlueprintDraftDocument,
    packet: &BlueprintPacketDocument,
) -> Result<(), String> {
    let Some(critique_id) = draft.latest_critique_id.as_deref() else {
        return Ok(());
    };
    if packet
        .references
        .iter()
        .any(|reference| reference.contains(critique_id))
    {
        Ok(())
    } else {
        Err("candidate Blueprint must reference latest open critique".to_owned())
    }
}

fn persist_candidate_markdown(
    paths: &WorkspacePaths,
    blueprint_id: &str,
    run_dir: &Path,
) -> io::Result<PathBuf> {
    let source = run_dir.join("blueprint.md");
    if !source.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "required Blueprint artifact is missing: blueprint.md",
        ));
    }
    let destination = paths
        .runtime_root
        .join("blueprints/packets/candidates")
        .join(format!("{blueprint_id}.md"));
    copy_unique_file(&source, &destination)?;
    Ok(destination)
}

fn candidate_packet_for_draft(
    paths: &WorkspacePaths,
    draft: &BlueprintDraftDocument,
) -> Result<BlueprintPacketDocument, BlueprintEffectError> {
    let blueprint_id = draft.latest_blueprint_id.as_deref().ok_or_else(|| {
        BlueprintEffectError::new(
            "blueprint_evaluation_invalid",
            "active draft has no latest candidate blueprint",
        )
    })?;
    let path = paths
        .runtime_root
        .join("blueprints/packets/candidates")
        .join(format!("{blueprint_id}.json"));
    let packet = read_json_model::<BlueprintPacketDocument>(&path)?;
    packet.ensure_matches_draft(draft).map_err(|error| {
        BlueprintEffectError::new("blueprint_evaluation_invalid", error.to_string())
    })?;
    Ok(packet)
}

fn validate_approval(
    evaluation: &BlueprintEvaluationDocument,
    packet: &BlueprintPacketDocument,
) -> Result<(), String> {
    if evaluation.decision != BlueprintEvaluationDecision::Approved {
        return Err("approval handler requires decision=approved".to_owned());
    }
    evaluation
        .ensure_matches_packet(packet)
        .map_err(|error| error.to_string())
}

fn validate_rejection(
    evaluation: &BlueprintEvaluationDocument,
    critique: &BlueprintCritiqueDocument,
    packet: &BlueprintPacketDocument,
) -> Result<(), BlueprintEffectError> {
    if evaluation.decision != BlueprintEvaluationDecision::Rejected {
        return Err(BlueprintEffectError::new(
            "blueprint_critique_invalid",
            "rejection handler requires decision=rejected",
        ));
    }
    evaluation.ensure_matches_packet(packet).map_err(|error| {
        BlueprintEffectError::new("blueprint_critique_invalid", error.to_string())
    })?;
    critique.ensure_matches_packet(packet).map_err(|error| {
        BlueprintEffectError::new("blueprint_critique_invalid", error.to_string())
    })?;
    if evaluation.critique_id.as_deref() != Some(critique.critique_id.as_str()) {
        return Err(BlueprintEffectError::new(
            "blueprint_critique_invalid",
            "evaluation critique_id must match critique packet",
        ));
    }
    Ok(())
}

fn read_generated_task(
    plan: &CompiledRunPlan,
    run_dir: &Path,
) -> Result<TaskDocument, BlueprintEffectError> {
    let resolved = resolve_run_artifact(plan, "generated_task", run_dir)?;
    let raw = fs::read_to_string(&resolved.path)
        .map_err(|error| BlueprintEffectError::new("generated_task_invalid", error.to_string()))?;
    let source_name = resolved.path.display().to_string();
    match resolved.format {
        ArtifactFormat::Json => parse_task_json_import_with_source(&raw, &source_name),
        ArtifactFormat::Markdown => parse_task_document_with_source(&raw, &source_name),
        _ => {
            return Err(BlueprintEffectError::new(
                "generated_task_invalid",
                "generated_task must be JSON or markdown",
            ));
        }
    }
    .map_err(|error| BlueprintEffectError::new("generated_task_invalid", error.to_string()))
}

fn read_blueprint_critique(
    plan: &CompiledRunPlan,
    run_dir: &Path,
) -> Result<BlueprintCritiqueDocument, BlueprintEffectError> {
    let resolved = resolve_run_artifact(plan, "blueprint_critique", run_dir)?;
    if resolved.format != ArtifactFormat::Json {
        return Err(BlueprintEffectError::new(
            "blueprint_critique_invalid",
            "blueprint_critique must be JSON",
        ));
    }
    read_json_model::<BlueprintCritiqueDocument>(&resolved.path)
        .map_err(|error| BlueprintEffectError::new("blueprint_critique_invalid", error.message))
}

fn validate_generated_task(
    task: &TaskDocument,
    draft: &BlueprintDraftDocument,
    packet: &BlueprintPacketDocument,
) -> Result<(), BlueprintEffectError> {
    if task.root_spec_id.as_deref() != Some(draft.root_spec_id.as_str()) {
        return Err(BlueprintEffectError::new(
            "generated_task_invalid",
            "generated task root_spec_id must match Blueprint draft",
        ));
    }
    if task.root_idea_id.as_deref() != Some(draft.root_idea_id.as_str()) {
        return Err(BlueprintEffectError::new(
            "generated_task_invalid",
            "generated task root_idea_id must match Blueprint draft",
        ));
    }
    if task.spec_id.as_deref() != Some(draft.source_spec_id.as_str()) {
        return Err(BlueprintEffectError::new(
            "generated_task_invalid",
            "generated task spec_id must match Blueprint source spec",
        ));
    }
    ensure_contains_all(
        &task.acceptance,
        &packet.task_acceptance,
        "generated task acceptance",
    )?;
    ensure_contains_all(
        &task.required_checks,
        &packet.required_checks,
        "generated task required checks",
    )?;
    let allowed = packet
        .intended_files
        .iter()
        .chain(draft.target_paths.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    if !task.target_paths.iter().all(|path| allowed.contains(path)) {
        return Err(BlueprintEffectError::new(
            "generated_task_invalid",
            "generated task target_paths must stay within Blueprint scope",
        ));
    }
    Ok(())
}

fn ensure_contains_all(
    actual: &[String],
    expected: &[String],
    field_name: &str,
) -> Result<(), BlueprintEffectError> {
    let missing = expected
        .iter()
        .filter(|item| !actual.contains(item))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(BlueprintEffectError::new(
            "generated_task_invalid",
            format!(
                "{field_name} missing Blueprint item(s): {}",
                missing.join(", ")
            ),
        ))
    }
}

fn ensure_task_id_unused(
    paths: &WorkspacePaths,
    task_id: &str,
) -> Result<(), BlueprintEffectError> {
    let filename = format!("{task_id}.md");
    for directory in [
        &paths.tasks_queue_dir,
        &paths.tasks_active_dir,
        &paths.tasks_done_dir,
        &paths.tasks_blocked_dir,
    ] {
        if directory.join(&filename).exists() {
            return Err(BlueprintEffectError::new(
                "blueprint_task_duplicate",
                format!("task {task_id} already exists"),
            ));
        }
    }
    Ok(())
}

fn ensure_promotion_id_unused(
    paths: &WorkspacePaths,
    evaluation_id: &str,
) -> Result<(), BlueprintEffectError> {
    let promotion = paths
        .runtime_root
        .join("blueprints/promotions")
        .join(format!("{}.json", promotion_id(evaluation_id)));
    if promotion.exists() {
        return Err(BlueprintEffectError::new(
            "blueprint_evaluation_invalid",
            format!("Blueprint promotion already exists for evaluation {evaluation_id}"),
        ));
    }
    Ok(())
}

fn move_candidate_markdown(
    paths: &WorkspacePaths,
    blueprint_id: &str,
    target_state: &str,
) -> Result<Option<PathBuf>, QueueStoreError> {
    let source = paths
        .runtime_root
        .join("blueprints/packets/candidates")
        .join(format!("{blueprint_id}.md"));
    if !source.exists() {
        return Ok(None);
    }
    let destination = paths
        .runtime_root
        .join("blueprints/packets")
        .join(target_state)
        .join(format!("{blueprint_id}.md"));
    if destination.exists() {
        return Err(QueueStoreError::InvalidState {
            message: format!("blueprint markdown {blueprint_id} already exists at destination"),
        });
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| QueueStoreError::Io {
            path: parent.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    fs::rename(&source, &destination).map_err(|error| QueueStoreError::Io {
        path: source.clone(),
        message: error.to_string(),
    })?;
    Ok(Some(destination))
}

fn blueprint_failure(
    handler_id: &str,
    stage_result: &StageResultEnvelope,
    failure_class: &str,
    message: String,
    created_paths: Vec<String>,
) -> RuntimeEffectResult {
    RuntimeEffectResult {
        handler_id: handler_id.to_owned(),
        decision: RuntimeEffectDecision::RequestBlockSource,
        created_paths: created_paths.clone(),
        source_lifecycle_intent: Some(source_lifecycle_intent_for_effect(
            stage_result,
            block_lifecycle_plan_id(stage_result.work_item_kind),
            SourceLifecycleAction::Block,
        )),
        failure_class: Some(failure_class.to_owned()),
        message: Some(message),
        mutation_phase: if created_paths.is_empty() {
            RuntimeEffectMutationPhase::PreMutation
        } else {
            RuntimeEffectMutationPhase::PartialMutation
        },
    }
}

fn approval_failure_class(error: &BlueprintEffectError, created_paths: &[String]) -> &'static str {
    if !created_paths.is_empty() {
        return "blueprint_partial_mutation";
    }
    match error.class.as_str() {
        "generated_task_missing" => "generated_task_missing",
        "generated_task_invalid" => "generated_task_invalid",
        "blueprint_task_duplicate" => "blueprint_task_duplicate",
        _ if error.message.contains("generated task") => "generated_task_invalid",
        _ => "blueprint_evaluation_invalid",
    }
}

fn complete_lifecycle_plan_id(kind: WorkItemKind) -> &'static str {
    match kind {
        WorkItemKind::Spec => "complete_spec_source_after_blueprint_effect",
        WorkItemKind::Incident => "complete_incident_source_after_blueprint_effect",
        WorkItemKind::BlueprintDraft => "approve_blueprint_draft_after_effect",
        _ => "complete_source_after_effect",
    }
}

fn block_lifecycle_plan_id(kind: WorkItemKind) -> &'static str {
    match kind {
        WorkItemKind::Spec => "block_spec_source_after_blueprint_effect",
        WorkItemKind::Incident => "block_incident_source_after_blueprint_effect",
        WorkItemKind::BlueprintDraft => "block_blueprint_draft_after_effect",
        _ => "block_source_after_effect",
    }
}

struct ResolvedRunArtifact {
    path: PathBuf,
    format: ArtifactFormat,
}

fn resolve_run_artifact(
    plan: &CompiledRunPlan,
    artifact_id: &str,
    run_dir: &Path,
) -> Result<ResolvedRunArtifact, BlueprintEffectError> {
    let contract = plan
        .workflow_primitives
        .artifact_contracts
        .iter()
        .find(|contract| contract.artifact_id == artifact_id)
        .ok_or_else(|| {
            BlueprintEffectError::new(
                format!("{artifact_id}_invalid"),
                "compiled plan does not declare artifact contract",
            )
        })?;
    let canonical = run_dir.join(&contract.canonical_filename);
    if canonical.exists() {
        return Ok(ResolvedRunArtifact {
            path: canonical,
            format: adapter_format(contract, &contract.canonical_filename)?,
        });
    }
    for filename in &contract.accepted_filenames {
        let candidate = run_dir.join(filename);
        if candidate.exists() {
            return Ok(ResolvedRunArtifact {
                path: candidate,
                format: adapter_format(contract, filename)?,
            });
        }
    }
    let class = if artifact_id == "generated_task" {
        "generated_task_missing"
    } else {
        "artifact_missing"
    };
    Err(BlueprintEffectError::new(
        class,
        format!(
            "no declared run artifact found in {}; expected {}",
            run_dir.display(),
            artifact_filenames(contract).join(", ")
        ),
    ))
}

fn adapter_format(
    contract: &crate::contracts::ArtifactContractDefinition,
    filename: &str,
) -> Result<ArtifactFormat, BlueprintEffectError> {
    contract
        .filename_adapters
        .iter()
        .find(|adapter| adapter.filename == filename)
        .map(|adapter| adapter.format)
        .ok_or_else(|| {
            BlueprintEffectError::new(
                "artifact_filename_unsupported",
                format!("filename {filename} is not declared by contract"),
            )
        })
}

fn artifact_filenames(contract: &crate::contracts::ArtifactContractDefinition) -> Vec<String> {
    let mut filenames = vec![contract.canonical_filename.clone()];
    filenames.extend(contract.accepted_filenames.clone());
    filenames
}

fn copy_unique_file(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "Blueprint artifact already exists: {}",
                destination.display()
            ),
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = destination.with_file_name(format!(
        ".{}.tmp",
        destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("blueprint")
    ));
    fs::copy(source, &temp)?;
    fs::rename(temp, destination)
}

fn add_unique_ref(references: &mut Vec<String>, reference: String) {
    if !references.contains(&reference) {
        references.push(reference);
    }
}

fn effect_path(paths: &WorkspacePaths, path: &Path) -> String {
    path.strip_prefix(&paths.root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn path_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("<artifact>")
        .to_owned()
}

fn promotion_id(evaluation_id: &str) -> String {
    format!("promotion-{evaluation_id}")
}

fn normalized_json<T>(document: &T) -> Result<String, BlueprintEffectError>
where
    T: Serialize,
{
    serde_json::to_string(
        &serde_json::to_value(document)
            .map_err(|error| BlueprintEffectError::new("json_model_parse", error.to_string()))?,
    )
    .map_err(|error| BlueprintEffectError::new("json_model_parse", error.to_string()))
}

fn normalized_draft_identity(
    draft: &BlueprintDraftDocument,
) -> Result<String, BlueprintEffectError> {
    let mut value = serde_json::to_value(draft)
        .map_err(|error| BlueprintEffectError::new("json_model_parse", error.to_string()))?;
    if let Value::Object(map) = &mut value {
        for key in [
            "status",
            "current_revision",
            "latest_blueprint_id",
            "latest_critique_id",
            "updated_at",
        ] {
            map.remove(key);
        }
    }
    serde_json::to_string(&value)
        .map_err(|error| BlueprintEffectError::new("json_model_parse", error.to_string()))
}
