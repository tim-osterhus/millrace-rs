//! Runtime effect dispatch selected from compiled workflow primitives.

use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    compiler::{CompiledRunPlan, FrozenGraphPlanePlan, MaterializedGraphNodePlan},
    contracts::{
        ArtifactContractDefinition, ArtifactFormat, Plane, RuntimeEffectMutationPhase,
        RuntimeEffectRuleDefinition, StageResultEnvelope, Timestamp, stage_name_for_plane,
        validate_safe_identifier,
    },
    runtime::{RuntimeStartupSession, StageRunRequest},
    workspace::{
        QueueLifecycleInterpreter, SourceLifecycleAction, SourceLifecycleIntent, WorkspacePaths,
        atomic_write_text,
    },
};

use super::{
    RouterAction, RouterDecision, RuntimeTickError, RuntimeTickResult,
    failure_policy::{
        RuntimeEffectFailurePolicyInput, RuntimeFailurePolicyInterpretation,
        interpret_runtime_effect_failure_policy,
    },
    lifecycle::{source_lifecycle_intent_for_effect, source_work_item_family_id},
    tick::write_runtime_event,
};

const PLANNER_DISPOSITION_HANDLER_ID: &str = "planner_disposition";
const COMPLETE_SOURCE_AFTER_EFFECT_PLAN_ID: &str = "complete_source_after_effect";
const BLOCK_SOURCE_AFTER_EFFECT_PLAN_ID: &str = "block_source_after_effect";

/// Handler-level effect decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEffectDecision {
    ContinueRoute,
    RequestCompleteSource,
    RequestBlockSource,
    RetryRecovery,
}

impl RuntimeEffectDecision {
    /// Returns the canonical serialized token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ContinueRoute => "continue_route",
            Self::RequestCompleteSource => "request_complete_source",
            Self::RequestBlockSource => "request_block_source",
            Self::RetryRecovery => "retry_recovery",
        }
    }
}

/// Normalized result returned by one packaged runtime effect handler.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeEffectResult {
    pub handler_id: String,
    pub decision: RuntimeEffectDecision,
    pub created_paths: Vec<String>,
    pub source_lifecycle_intent: Option<SourceLifecycleIntent>,
    pub failure_class: Option<String>,
    pub message: Option<String>,
    pub mutation_phase: RuntimeEffectMutationPhase,
}

/// Runtime-effect application output that the tick application path consumes.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeEffectApplication {
    pub router_decision: RouterDecision,
    pub spawned_paths: Vec<PathBuf>,
    pub source_lifecycle_applied: bool,
    pub decision_path: Option<PathBuf>,
    pub result_path: Option<PathBuf>,
}

impl RuntimeEffectApplication {
    fn unchanged(router_decision: RouterDecision) -> Self {
        Self {
            router_decision,
            spawned_paths: Vec::new(),
            source_lifecycle_applied: false,
            decision_path: None,
            result_path: None,
        }
    }
}

/// Apply the compiled runtime effect selected for a stage result, when present.
pub fn apply_runtime_effect_for_stage_result(
    session: &RuntimeStartupSession,
    request: &StageRunRequest,
    stage_result: &mut StageResultEnvelope,
    router_decision: RouterDecision,
    stage_result_path: &Path,
) -> RuntimeTickResult<RuntimeEffectApplication> {
    let Some(selection) = selected_runtime_effect_rule(&session.compiled_plan, stage_result)?
    else {
        return Ok(RuntimeEffectApplication::unchanged(router_decision));
    };

    let decision_path =
        write_runtime_effect_decision_artifact(request, stage_result, selection.rule)?;
    let mut effect_result = normalize_effect_failure_phase(run_packaged_handler(
        session,
        stage_result,
        Path::new(&request.run_dir),
        selection.rule,
    )?);
    let failure_policy = runtime_effect_failure_policy_resolution(
        &session.compiled_plan,
        stage_result,
        &effect_result,
        selection.rule,
    );

    let mut source_lifecycle_applied = false;
    let mut spawned_paths = Vec::new();
    let final_decision = if failure_policy
        .as_ref()
        .is_some_and(|policy| policy.action == "route_to_node")
    {
        router_decision_for_failure_policy_route(
            &session.compiled_plan,
            stage_result,
            &effect_result,
            failure_policy.as_ref().expect("checked route policy"),
        )?
        .unwrap_or(router_decision)
    } else {
        effect_result =
            apply_runtime_effect_result(session, effect_result, &stage_result.completed_at)?;
        spawned_paths = spawned_paths_for_effect(session, stage_result, &effect_result)?;
        match effect_result.decision {
            RuntimeEffectDecision::ContinueRoute => router_decision_for_continue_route(
                &session.compiled_plan,
                stage_result,
                router_decision,
            )?,
            RuntimeEffectDecision::RequestCompleteSource
            | RuntimeEffectDecision::RequestBlockSource => {
                source_lifecycle_applied = effect_result.source_lifecycle_intent.is_some();
                router_decision_for_effect(&effect_result, failure_policy.as_ref())
            }
            RuntimeEffectDecision::RetryRecovery => {
                return Err(invalid_state(format!(
                    "{} requested runtime effect recovery: {}",
                    effect_result.handler_id,
                    effect_result
                        .failure_class
                        .as_deref()
                        .unwrap_or("runtime_effect_failed")
                )));
            }
        }
    };

    let result_path = write_runtime_effect_result_artifact(
        request,
        stage_result,
        selection.rule,
        &effect_result,
        failure_policy.as_ref(),
    )?;
    annotate_stage_result_with_effect(
        stage_result,
        selection.rule,
        &effect_result,
        &decision_path,
        &result_path,
        Path::new(&request.run_dir),
        failure_policy.as_ref(),
    );
    let _event_path = emit_runtime_effect_event(
        session,
        stage_result,
        selection.rule,
        &effect_result,
        failure_policy.as_ref(),
    )?;
    write_stage_result_snapshot(stage_result_path, stage_result)?;

    Ok(RuntimeEffectApplication {
        router_decision: final_decision,
        spawned_paths,
        source_lifecycle_applied,
        decision_path: Some(decision_path),
        result_path: Some(result_path),
    })
}

struct RuntimeEffectRuleSelection<'a> {
    rule: &'a RuntimeEffectRuleDefinition,
}

fn selected_runtime_effect_rule<'a>(
    plan: &'a CompiledRunPlan,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<Option<RuntimeEffectRuleSelection<'a>>> {
    let stage_plan = stage_plan_for_result(plan, stage_result)?;
    if stage_plan.runtime_effect_rule_selections.is_empty() {
        return Ok(None);
    }
    let outcome = stage_result.terminal_result.as_str();
    let mut matches = Vec::new();
    for rule_id in &stage_plan.runtime_effect_rule_selections {
        let rule = plan
            .workflow_primitives
            .runtime_effect_rules
            .iter()
            .find(|rule| rule.rule_id == *rule_id)
            .ok_or_else(|| {
                invalid_state(format!(
                    "compiled node {} selected unknown runtime effect rule {rule_id}",
                    stage_plan.node_id
                ))
            })?;
        if rule_matches_stage_result(rule, stage_result, outcome) {
            matches.push(rule);
        }
    }
    match matches.as_slice() {
        [] => Ok(None),
        [rule] => {
            ensure_packaged_handler(plan, &rule.handler_id)?;
            Ok(Some(RuntimeEffectRuleSelection { rule }))
        }
        rules => Err(invalid_state(format!(
            "multiple runtime effect rules matched {}/{}: {}",
            stage_result.node_id,
            outcome,
            rules
                .iter()
                .map(|rule| rule.rule_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn rule_matches_stage_result(
    rule: &RuntimeEffectRuleDefinition,
    stage_result: &StageResultEnvelope,
    outcome: &str,
) -> bool {
    (rule.source_node_id == stage_result.node_id
        || rule.source_node_id == stage_result.stage_kind_id)
        && rule
            .on_outcomes
            .iter()
            .any(|candidate| candidate == outcome)
}

fn ensure_packaged_handler(plan: &CompiledRunPlan, handler_id: &str) -> RuntimeTickResult<()> {
    if plan
        .workflow_primitives
        .runtime_effect_handlers
        .iter()
        .all(|handler| handler.handler_id != handler_id)
    {
        return Err(invalid_state(format!(
            "runtime effect rule references unknown handler {handler_id}"
        )));
    }
    if !matches!(
        handler_id,
        "planner_disposition"
            | "manager_blueprint_manifest_to_blueprint_drafts"
            | "contractor_blueprint_candidate_persist"
            | "evaluator_blueprint_approved_to_task"
            | "evaluator_blueprint_rejected_to_draft_revision"
    ) {
        return Err(invalid_state(format!(
            "runtime effect handler {handler_id} has no packaged Rust implementation"
        )));
    }
    Ok(())
}

fn run_packaged_handler(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    run_dir: &Path,
    rule: &RuntimeEffectRuleDefinition,
) -> RuntimeTickResult<RuntimeEffectResult> {
    if let Some(result) = super::blueprint_effects::run_blueprint_effect_handler(
        session,
        stage_result,
        run_dir,
        rule,
    )? {
        return Ok(result);
    }
    match rule.handler_id.as_str() {
        PLANNER_DISPOSITION_HANDLER_ID => planner_disposition(session, stage_result, run_dir, rule),
        handler_id => Ok(RuntimeEffectResult {
            handler_id: handler_id.to_owned(),
            decision: RuntimeEffectDecision::RequestBlockSource,
            created_paths: Vec::new(),
            source_lifecycle_intent: Some(source_lifecycle_intent_for_effect(
                stage_result,
                rule.lifecycle_mutation_plan_id
                    .clone()
                    .unwrap_or_else(|| BLOCK_SOURCE_AFTER_EFFECT_PLAN_ID.to_owned()),
                SourceLifecycleAction::Block,
            )),
            failure_class: Some("runtime_effect_handler_not_implemented".to_owned()),
            message: Some(format!(
                "runtime effect handler {handler_id} is reserved for the Blueprint runtime slice"
            )),
            mutation_phase: RuntimeEffectMutationPhase::PreMutation,
        }),
    }
}

fn planner_disposition(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    run_dir: &Path,
    rule: &RuntimeEffectRuleDefinition,
) -> RuntimeTickResult<RuntimeEffectResult> {
    let disposition = match parse_planner_disposition(&session.compiled_plan, run_dir) {
        Ok(disposition) => disposition,
        Err(error) if error.failure_class == "artifact_missing" => {
            return Ok(planner_failure_result(
                stage_result,
                rule,
                "planner_disposition_missing",
                "required Planner artifact is missing: planner_disposition.json",
            ));
        }
        Err(error) => {
            return Ok(planner_failure_result(
                stage_result,
                rule,
                "planner_disposition_invalid",
                &error.to_string(),
            ));
        }
    };

    if let Err(message) = disposition.source_mismatch(stage_result) {
        return Ok(planner_failure_result(
            stage_result,
            rule,
            "planner_disposition_source_mismatch",
            &message,
        ));
    }

    let terminal_result = stage_result.terminal_result.as_str();
    match disposition.disposition.as_str() {
        "active_source_ready_for_manager" => {
            if terminal_result != "PLANNER_COMPLETE" {
                return Ok(planner_failure_result(
                    stage_result,
                    rule,
                    "planner_disposition_terminal_mismatch",
                    "active_source_ready_for_manager requires PLANNER_COMPLETE",
                ));
            }
            Ok(RuntimeEffectResult {
                handler_id: PLANNER_DISPOSITION_HANDLER_ID.to_owned(),
                decision: RuntimeEffectDecision::ContinueRoute,
                created_paths: Vec::new(),
                source_lifecycle_intent: None,
                failure_class: None,
                message: Some(
                    "Planner disposition keeps active source on the Manager route".to_owned(),
                ),
                mutation_phase: RuntimeEffectMutationPhase::Unknown,
            })
        }
        "blocked" => {
            if terminal_result == "BLOCKED" {
                return Ok(RuntimeEffectResult {
                    handler_id: PLANNER_DISPOSITION_HANDLER_ID.to_owned(),
                    decision: RuntimeEffectDecision::ContinueRoute,
                    created_paths: Vec::new(),
                    source_lifecycle_intent: None,
                    failure_class: None,
                    message: Some(
                        "Planner disposition preserves graph-declared blocked recovery".to_owned(),
                    ),
                    mutation_phase: RuntimeEffectMutationPhase::Unknown,
                });
            }
            Ok(planner_failure_result(
                stage_result,
                rule,
                "planner_disposition_blocked",
                "blocked disposition requires the BLOCKED terminal result",
            ))
        }
        "emitted_child_specs" => {
            if terminal_result != "PLANNER_COMPLETE" {
                return Ok(planner_failure_result(
                    stage_result,
                    rule,
                    "planner_disposition_terminal_mismatch",
                    "emitted_child_specs requires PLANNER_COMPLETE",
                ));
            }
            let missing = missing_emitted_specs(&session.paths, &disposition.emitted_spec_ids);
            if !missing.is_empty() {
                return Ok(planner_failure_result(
                    stage_result,
                    rule,
                    "planner_disposition_child_spec_missing",
                    &format!(
                        "Planner emitted child spec ids that are not queued: {}",
                        missing.join(", ")
                    ),
                ));
            }
            let created_paths = disposition
                .emitted_spec_ids
                .iter()
                .map(|spec_id| {
                    workspace_relative_path(
                        &session.paths,
                        &session.paths.specs_queue_dir.join(format!("{spec_id}.md")),
                    )
                })
                .collect::<Vec<_>>();
            Ok(RuntimeEffectResult {
                handler_id: PLANNER_DISPOSITION_HANDLER_ID.to_owned(),
                decision: RuntimeEffectDecision::RequestCompleteSource,
                created_paths,
                source_lifecycle_intent: Some(source_lifecycle_intent_for_effect(
                    stage_result,
                    rule.lifecycle_mutation_plan_id
                        .clone()
                        .unwrap_or_else(|| COMPLETE_SOURCE_AFTER_EFFECT_PLAN_ID.to_owned()),
                    SourceLifecycleAction::Complete,
                )),
                failure_class: None,
                message: Some(format!(
                    "Planner disposition resolved active source after emitting child specs: {}",
                    disposition.emitted_spec_ids.join(", ")
                )),
                mutation_phase: RuntimeEffectMutationPhase::PreMutation,
            })
        }
        _ => Ok(planner_failure_result(
            stage_result,
            rule,
            "planner_disposition_invalid",
            "unsupported planner disposition value",
        )),
    }
}

fn planner_failure_result(
    stage_result: &StageResultEnvelope,
    rule: &RuntimeEffectRuleDefinition,
    failure_class: &str,
    message: &str,
) -> RuntimeEffectResult {
    RuntimeEffectResult {
        handler_id: PLANNER_DISPOSITION_HANDLER_ID.to_owned(),
        decision: RuntimeEffectDecision::RequestBlockSource,
        created_paths: Vec::new(),
        source_lifecycle_intent: Some(source_lifecycle_intent_for_effect(
            stage_result,
            rule.lifecycle_mutation_plan_id
                .clone()
                .unwrap_or_else(|| BLOCK_SOURCE_AFTER_EFFECT_PLAN_ID.to_owned()),
            SourceLifecycleAction::Block,
        )),
        failure_class: Some(failure_class.to_owned()),
        message: Some(message.to_owned()),
        mutation_phase: RuntimeEffectMutationPhase::PreMutation,
    }
}

fn parse_planner_disposition(
    plan: &CompiledRunPlan,
    run_dir: &Path,
) -> Result<PlannerDispositionDocument, RuntimeArtifactError> {
    let resolved = resolve_run_artifact(plan, "planner_disposition", run_dir)?;
    if resolved.format != ArtifactFormat::Json {
        return Err(RuntimeArtifactError::new(
            "planner_disposition",
            &resolved.path,
            "artifact_parser_unsupported",
            "planner disposition must be a JSON artifact",
        ));
    }
    let raw = fs::read_to_string(&resolved.path).map_err(|error| RuntimeArtifactError {
        artifact_id: "planner_disposition".to_owned(),
        path: resolved.path.clone(),
        failure_class: "json_model_parse".to_owned(),
        message: error.to_string(),
    })?;
    let mut disposition: PlannerDispositionDocument =
        serde_json::from_str(&raw).map_err(|error| RuntimeArtifactError {
            artifact_id: "planner_disposition".to_owned(),
            path: resolved.path.clone(),
            failure_class: "json_model_parse".to_owned(),
            message: error.to_string(),
        })?;
    disposition
        .validate()
        .map_err(|error| RuntimeArtifactError {
            artifact_id: "planner_disposition".to_owned(),
            path: resolved.path,
            failure_class: "json_model_parse".to_owned(),
            message: error,
        })?;
    Ok(disposition)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlannerDispositionDocument {
    schema_version: String,
    kind: String,
    source_work_item_family_id: String,
    source_work_item_id: String,
    disposition: String,
    emitted_spec_ids: Vec<String>,
    refined_active_source: bool,
    recommended_next_action: String,
    created_at: Timestamp,
    created_by: String,
}

impl PlannerDispositionDocument {
    fn validate(&mut self) -> Result<(), String> {
        if self.schema_version != "1.0" {
            return Err("schema_version must be 1.0".to_owned());
        }
        if self.kind != "planner_disposition" {
            return Err("kind must be planner_disposition".to_owned());
        }
        if !matches!(
            self.source_work_item_family_id.as_str(),
            "spec" | "incident"
        ) {
            return Err("source_work_item_family_id must be spec or incident".to_owned());
        }
        validate_safe_identifier(&self.source_work_item_id, "source_work_item_id")
            .map_err(|error| error.to_string())?;
        for spec_id in &self.emitted_spec_ids {
            validate_safe_identifier(spec_id, "emitted_spec_ids")
                .map_err(|error| error.to_string())?;
        }
        let unique = self.emitted_spec_ids.iter().collect::<BTreeSet<_>>();
        if unique.len() != self.emitted_spec_ids.len() {
            return Err("emitted_spec_ids must be unique".to_owned());
        }
        if !matches!(
            self.disposition.as_str(),
            "active_source_ready_for_manager" | "emitted_child_specs" | "blocked"
        ) {
            return Err("disposition has an unsupported value".to_owned());
        }
        if self.recommended_next_action.trim().is_empty() {
            return Err("recommended_next_action is required".to_owned());
        }
        if self.created_by != "planner" {
            return Err("created_by must be planner".to_owned());
        }
        if self.disposition == "emitted_child_specs" && self.emitted_spec_ids.is_empty() {
            return Err("emitted_child_specs requires emitted_spec_ids".to_owned());
        }
        if self.disposition != "emitted_child_specs" && !self.emitted_spec_ids.is_empty() {
            return Err("emitted_spec_ids are only valid for emitted_child_specs".to_owned());
        }
        let _refined_active_source = self.refined_active_source;
        let _created_at = &self.created_at;
        Ok(())
    }

    fn source_mismatch(&self, stage_result: &StageResultEnvelope) -> Result<(), String> {
        let family_id = source_work_item_family_id(stage_result).unwrap_or_default();
        if family_id != self.source_work_item_family_id {
            return Err(format!(
                "planner disposition source family mismatch: {} != {}",
                self.source_work_item_family_id, family_id
            ));
        }
        if stage_result.work_item_id != self.source_work_item_id {
            return Err(format!(
                "planner disposition source id mismatch: {} != {}",
                self.source_work_item_id, stage_result.work_item_id
            ));
        }
        Ok(())
    }
}

struct ResolvedRunArtifact {
    path: PathBuf,
    format: ArtifactFormat,
}

#[derive(Debug)]
struct RuntimeArtifactError {
    artifact_id: String,
    path: PathBuf,
    failure_class: String,
    message: String,
}

impl RuntimeArtifactError {
    fn new(
        artifact_id: impl Into<String>,
        path: &Path,
        failure_class: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            artifact_id: artifact_id.into(),
            path: path.to_path_buf(),
            failure_class: failure_class.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RuntimeArtifactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "artifact_id={} path={} failure_class={}: {}",
            self.artifact_id,
            self.path.display(),
            self.failure_class,
            self.message
        )
    }
}

fn resolve_run_artifact(
    plan: &CompiledRunPlan,
    artifact_id: &str,
    run_dir: &Path,
) -> Result<ResolvedRunArtifact, RuntimeArtifactError> {
    let contract = artifact_contract(plan, artifact_id)?;
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
    Err(RuntimeArtifactError {
        artifact_id: artifact_id.to_owned(),
        path: run_dir.to_path_buf(),
        failure_class: "artifact_missing".to_owned(),
        message: format!(
            "no declared run artifact found in {}; expected {}",
            run_dir.display(),
            artifact_filenames(contract).join(", ")
        ),
    })
}

fn artifact_contract<'a>(
    plan: &'a CompiledRunPlan,
    artifact_id: &str,
) -> Result<&'a ArtifactContractDefinition, RuntimeArtifactError> {
    plan.workflow_primitives
        .artifact_contracts
        .iter()
        .find(|contract| contract.artifact_id == artifact_id)
        .ok_or_else(|| RuntimeArtifactError {
            artifact_id: artifact_id.to_owned(),
            path: PathBuf::new(),
            failure_class: "artifact_contract_missing".to_owned(),
            message: "compiled plan does not declare artifact contract".to_owned(),
        })
}

fn adapter_format(
    contract: &ArtifactContractDefinition,
    filename: &str,
) -> Result<ArtifactFormat, RuntimeArtifactError> {
    contract
        .filename_adapters
        .iter()
        .find(|adapter| adapter.filename == filename)
        .map(|adapter| adapter.format)
        .ok_or_else(|| RuntimeArtifactError {
            artifact_id: contract.artifact_id.clone(),
            path: PathBuf::from(filename),
            failure_class: "artifact_filename_unsupported".to_owned(),
            message: format!("filename {filename} is not declared by contract"),
        })
}

fn artifact_filenames(contract: &ArtifactContractDefinition) -> Vec<String> {
    let mut filenames = vec![contract.canonical_filename.clone()];
    filenames.extend(contract.accepted_filenames.clone());
    filenames
}

fn missing_emitted_specs(paths: &WorkspacePaths, emitted_spec_ids: &[String]) -> Vec<String> {
    emitted_spec_ids
        .iter()
        .filter(|spec_id| {
            !paths
                .specs_queue_dir
                .join(format!("{spec_id}.md"))
                .is_file()
        })
        .cloned()
        .collect()
}

fn apply_runtime_effect_result(
    session: &RuntimeStartupSession,
    mut effect_result: RuntimeEffectResult,
    occurred_at: &Timestamp,
) -> RuntimeTickResult<RuntimeEffectResult> {
    let missing_paths = effect_result
        .created_paths
        .iter()
        .filter(|path| !effect_path(&session.paths, path).exists())
        .cloned()
        .collect::<Vec<_>>();
    if !missing_paths.is_empty() {
        write_runtime_event(
            &session.paths,
            "runtime_effect_destination_missing",
            json_object([
                (
                    "handler_id",
                    Value::String(effect_result.handler_id.clone()),
                ),
                ("missing_paths", json!(missing_paths)),
                (
                    "work_item_family_id",
                    effect_result
                        .source_lifecycle_intent
                        .as_ref()
                        .and_then(|intent| intent.work_item_family_id.clone())
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "work_item_kind",
                    effect_result
                        .source_lifecycle_intent
                        .as_ref()
                        .and_then(|intent| intent.work_item_kind)
                        .map(|kind| Value::String(kind.as_str().to_owned()))
                        .unwrap_or(Value::Null),
                ),
                (
                    "work_item_id",
                    effect_result
                        .source_lifecycle_intent
                        .as_ref()
                        .map(|intent| Value::String(intent.work_item_id.clone()))
                        .unwrap_or(Value::Null),
                ),
                (
                    "lifecycle_plan_id",
                    effect_result
                        .source_lifecycle_intent
                        .as_ref()
                        .map(|intent| Value::String(intent.lifecycle_plan_id.clone()))
                        .unwrap_or(Value::Null),
                ),
            ]),
            occurred_at,
        )?;
        effect_result.decision = RuntimeEffectDecision::RetryRecovery;
        effect_result.failure_class = Some("runtime_effect_destination_missing".to_owned());
        return Ok(effect_result);
    }

    if let Some(intent) = &effect_result.source_lifecycle_intent {
        QueueLifecycleInterpreter::new(
            session.paths.clone(),
            session
                .compiled_plan
                .workflow_primitives
                .work_item_families
                .clone(),
        )
        .apply(intent)
        .map_err(RuntimeTickError::from)?;
    }
    Ok(effect_result)
}

fn normalize_effect_failure_phase(mut effect_result: RuntimeEffectResult) -> RuntimeEffectResult {
    if effect_result.decision == RuntimeEffectDecision::RequestBlockSource
        && effect_result.mutation_phase == RuntimeEffectMutationPhase::PreMutation
        && !effect_result.created_paths.is_empty()
    {
        effect_result.mutation_phase = RuntimeEffectMutationPhase::PartialMutation;
    }
    effect_result
}

fn runtime_effect_failure_policy_resolution(
    plan: &CompiledRunPlan,
    stage_result: &StageResultEnvelope,
    effect_result: &RuntimeEffectResult,
    effect_rule: &RuntimeEffectRuleDefinition,
) -> Option<RuntimeFailurePolicyInterpretation> {
    if effect_result.decision != RuntimeEffectDecision::RequestBlockSource {
        return None;
    }
    let input = RuntimeEffectFailurePolicyInput {
        failure_class: effect_result.failure_class.clone(),
        mutation_phase: effect_result.mutation_phase,
        handler_id: Some(effect_result.handler_id.clone()),
        source_node_id: Some(stage_result.node_id.clone()),
        source_terminal_state_id: source_terminal_state_id_for_effect(
            plan,
            stage_result,
            effect_rule,
        ),
        source_plane: Some(stage_result.plane),
        source_family_id: source_work_item_family_id(stage_result),
        created_paths: effect_result.created_paths.clone(),
        message: effect_result.message.clone(),
    };
    interpret_runtime_effect_failure_policy(
        &plan.workflow_primitives.runtime_failure_policies,
        &input,
    )
}

fn source_terminal_state_id_for_effect(
    plan: &CompiledRunPlan,
    stage_result: &StageResultEnvelope,
    effect_rule: &RuntimeEffectRuleDefinition,
) -> Option<String> {
    let graph = graph_for_plane(plan, stage_result.plane).ok()?;
    let outcome = stage_result.terminal_result.as_str();
    graph
        .compiled_transitions
        .iter()
        .find(|transition| {
            transition.outcome == outcome
                && (transition.source_node_id == stage_result.node_id
                    || transition.source_node_id == effect_rule.source_node_id)
        })
        .and_then(|transition| transition.terminal_state_id.clone())
}

fn router_decision_for_failure_policy_route(
    plan: &CompiledRunPlan,
    stage_result: &StageResultEnvelope,
    effect_result: &RuntimeEffectResult,
    resolution: &RuntimeFailurePolicyInterpretation,
) -> RuntimeTickResult<Option<RouterDecision>> {
    let Some(target_node_id) = &resolution.target_node_id else {
        return Ok(None);
    };
    let graph = graph_for_plane(plan, stage_result.plane)?;
    let target_node = graph
        .nodes
        .iter()
        .find(|node| node.node_id == *target_node_id)
        .ok_or_else(|| {
            invalid_state(format!(
                "runtime failure policy {} targets unknown node {target_node_id}",
                resolution.policy_id
            ))
        })?;
    Ok(Some(RouterDecision {
        action: RouterAction::RunStage,
        next_plane: Some(graph.plane),
        next_stage: Some(stage_name_for_plane(
            graph.plane,
            &target_node.stage_kind_id,
        )?),
        reason: format!(
            "runtime_effect_failure:{}:{}",
            effect_result.handler_id, resolution.failure_class
        ),
        next_node_id: Some(target_node.node_id.clone()),
        next_stage_kind_id: Some(target_node.stage_kind_id.clone()),
        failure_class: Some(resolution.failure_class.clone()),
        counter_key: None,
        create_incident: false,
    }))
}

fn router_decision_for_continue_route(
    plan: &CompiledRunPlan,
    stage_result: &StageResultEnvelope,
    fallback: RouterDecision,
) -> RuntimeTickResult<RouterDecision> {
    let graph = graph_for_plane(plan, stage_result.plane)?;
    let outcome = stage_result.terminal_result.as_str();
    let Some(transition) = graph.compiled_transitions.iter().find(|transition| {
        transition.outcome == outcome
            && (transition.source_node_id == stage_result.node_id
                || transition.source_node_id == stage_result.stage_kind_id)
            && transition.target_node_id.is_some()
    }) else {
        return Ok(fallback);
    };
    let target_node_id = transition
        .target_node_id
        .as_ref()
        .expect("transition target was checked");
    let target_node = graph
        .nodes
        .iter()
        .find(|node| node.node_id == *target_node_id)
        .ok_or_else(|| {
            invalid_state(format!(
                "runtime effect continue_route targets unknown node {target_node_id}"
            ))
        })?;
    Ok(RouterDecision {
        action: RouterAction::RunStage,
        next_plane: Some(graph.plane),
        next_stage: Some(stage_name_for_plane(
            graph.plane,
            &target_node.stage_kind_id,
        )?),
        reason: if fallback.reason.is_empty() {
            format!("{}:{outcome}", stage_result.stage_kind_id)
        } else {
            fallback.reason
        },
        next_node_id: Some(target_node.node_id.clone()),
        next_stage_kind_id: Some(target_node.stage_kind_id.clone()),
        failure_class: fallback.failure_class,
        counter_key: fallback.counter_key,
        create_incident: fallback.create_incident,
    })
}

fn router_decision_for_effect(
    effect_result: &RuntimeEffectResult,
    failure_policy_resolution: Option<&RuntimeFailurePolicyInterpretation>,
) -> RouterDecision {
    match effect_result.decision {
        RuntimeEffectDecision::RequestCompleteSource => RouterDecision {
            action: RouterAction::Idle,
            next_plane: None,
            next_stage: None,
            reason: effect_result.handler_id.clone(),
            next_node_id: None,
            next_stage_kind_id: None,
            failure_class: None,
            counter_key: None,
            create_incident: false,
        },
        RuntimeEffectDecision::RequestBlockSource => {
            let failure_class = effect_result
                .failure_class
                .clone()
                .or_else(|| failure_policy_resolution.map(|policy| policy.failure_class.clone()));
            let reason = if failure_policy_resolution
                .is_some_and(|policy| policy.action == "require_operator")
            {
                format!(
                    "runtime_effect_requires_operator:{}:{}",
                    effect_result.handler_id,
                    failure_class.as_deref().unwrap_or("runtime_effect_failed")
                )
            } else {
                effect_result.handler_id.clone()
            };
            RouterDecision {
                action: RouterAction::Blocked,
                next_plane: None,
                next_stage: None,
                reason,
                next_node_id: None,
                next_stage_kind_id: None,
                failure_class,
                counter_key: None,
                create_incident: false,
            }
        }
        RuntimeEffectDecision::ContinueRoute | RuntimeEffectDecision::RetryRecovery => {
            RouterDecision {
                action: RouterAction::Blocked,
                next_plane: None,
                next_stage: None,
                reason: effect_result.handler_id.clone(),
                next_node_id: None,
                next_stage_kind_id: None,
                failure_class: effect_result.failure_class.clone(),
                counter_key: None,
                create_incident: false,
            }
        }
    }
}

fn spawned_paths_for_effect(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    effect_result: &RuntimeEffectResult,
) -> RuntimeTickResult<Vec<PathBuf>> {
    let Some(destination_family_id) =
        destination_family_id_for_effect(&session.compiled_plan, stage_result, effect_result)?
    else {
        return Ok(Vec::new());
    };
    let Some(family) = session
        .compiled_plan
        .workflow_primitives
        .work_item_families
        .iter()
        .find(|family| family.family_id == destination_family_id)
    else {
        return Ok(Vec::new());
    };
    let queue_dir = session.paths.runtime_root.join(&family.queue_dirs.queue);
    let mut paths = Vec::new();
    for created_path in &effect_result.created_paths {
        let path = effect_path(&session.paths, created_path);
        if path.starts_with(&queue_dir) {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn destination_family_id_for_effect(
    plan: &CompiledRunPlan,
    stage_result: &StageResultEnvelope,
    effect_result: &RuntimeEffectResult,
) -> RuntimeTickResult<Option<String>> {
    let outcome = stage_result.terminal_result.as_str();
    let matching_rules = plan
        .workflow_primitives
        .runtime_effect_rules
        .iter()
        .filter(|rule| {
            rule.handler_id == effect_result.handler_id
                && rule_matches_stage_result(rule, stage_result, outcome)
        })
        .collect::<Vec<_>>();
    match matching_rules.as_slice() {
        [] => Ok(None),
        [rule] => Ok(rule.destination_family_id.clone()),
        rules => Err(invalid_state(format!(
            "multiple runtime effect rules matched spawned-work destination {}/{}: {}",
            stage_result.node_id,
            outcome,
            rules
                .iter()
                .map(|rule| rule.rule_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn annotate_stage_result_with_effect(
    stage_result: &mut StageResultEnvelope,
    rule: &RuntimeEffectRuleDefinition,
    effect_result: &RuntimeEffectResult,
    decision_path: &Path,
    result_path: &Path,
    run_dir: &Path,
    failure_policy: Option<&RuntimeFailurePolicyInterpretation>,
) {
    stage_result.metadata.insert(
        "runtime_effect_rule_id".to_owned(),
        Value::String(rule.rule_id.clone()),
    );
    stage_result.metadata.insert(
        "runtime_effect_operation_id".to_owned(),
        Value::String(rule.effect_operation_id.clone()),
    );
    stage_result.metadata.insert(
        "runtime_effect_handler_id".to_owned(),
        Value::String(effect_result.handler_id.clone()),
    );
    stage_result.metadata.insert(
        "runtime_effect_decision".to_owned(),
        Value::String(effect_result.decision.as_str().to_owned()),
    );
    stage_result.metadata.insert(
        "runtime_effect_created_paths".to_owned(),
        json!(effect_result.created_paths),
    );
    stage_result.metadata.insert(
        "runtime_effect_failure_class".to_owned(),
        optional_string_json(effect_result.failure_class.as_ref()),
    );
    stage_result.metadata.insert(
        "runtime_effect_failure_message".to_owned(),
        optional_string_json(effect_result.message.as_ref()),
    );
    stage_result.metadata.insert(
        "runtime_effect_mutation_phase".to_owned(),
        Value::String(mutation_phase_str(effect_result.mutation_phase).to_owned()),
    );
    stage_result.metadata.insert(
        "runtime_effect_decision_path".to_owned(),
        Value::String(decision_path.display().to_string()),
    );
    stage_result.metadata.insert(
        "runtime_effect_result_path".to_owned(),
        Value::String(result_path.display().to_string()),
    );
    if let Some(intent) = &effect_result.source_lifecycle_intent {
        stage_result.metadata.insert(
            "runtime_effect_source_lifecycle_plan_id".to_owned(),
            Value::String(intent.lifecycle_plan_id.clone()),
        );
        stage_result.metadata.insert(
            "runtime_effect_source_lifecycle_action".to_owned(),
            Value::String(intent.action.as_str().to_owned()),
        );
    } else {
        stage_result.metadata.insert(
            "runtime_effect_source_lifecycle_plan_id".to_owned(),
            Value::Null,
        );
        stage_result.metadata.insert(
            "runtime_effect_source_lifecycle_action".to_owned(),
            Value::Null,
        );
    }
    if let Some(policy) = failure_policy {
        stage_result.metadata.insert(
            "runtime_effect_failure_policy_id".to_owned(),
            Value::String(policy.policy_id.clone()),
        );
        stage_result.metadata.insert(
            "runtime_effect_recovery_action".to_owned(),
            Value::String(policy.action.clone()),
        );
    }
    if effect_result.failure_class.is_some() {
        stage_result.metadata.insert(
            "failure_origin".to_owned(),
            Value::String("runtime_effect".to_owned()),
        );
        stage_result.metadata.insert(
            "failure_class".to_owned(),
            optional_string_json(effect_result.failure_class.as_ref()),
        );
    }
    push_unique_artifact_path(stage_result, run_relative_path(run_dir, decision_path));
    push_unique_artifact_path(stage_result, run_relative_path(run_dir, result_path));
    for path in &effect_result.created_paths {
        push_unique_artifact_path(stage_result, path.clone());
    }
}

fn write_runtime_effect_decision_artifact(
    request: &StageRunRequest,
    stage_result: &StageResultEnvelope,
    rule: &RuntimeEffectRuleDefinition,
) -> RuntimeTickResult<PathBuf> {
    let path = request_artifact_path(request, "runtime_effect_decisions", "json");
    let payload = json!({
        "schema_version": "1.0",
        "kind": "runtime_effect_decision",
        "request_id": request.request_id,
        "run_id": request.run_id,
        "plane": stage_result.plane.as_str(),
        "stage": stage_result.stage.as_str(),
        "node_id": stage_result.node_id,
        "stage_kind_id": stage_result.stage_kind_id,
        "terminal_result": stage_result.terminal_result.as_str(),
        "rule_id": rule.rule_id,
        "effect_operation_id": rule.effect_operation_id,
        "handler_id": rule.handler_id,
        "required_run_artifacts": rule.required_run_artifacts,
        "lifecycle_mutation_plan_id": rule.lifecycle_mutation_plan_id,
        "applies_before_route": rule.applies_before_route,
    });
    write_pretty_json(&path, &payload)?;
    Ok(path)
}

fn write_runtime_effect_result_artifact(
    request: &StageRunRequest,
    stage_result: &StageResultEnvelope,
    rule: &RuntimeEffectRuleDefinition,
    effect_result: &RuntimeEffectResult,
    failure_policy: Option<&RuntimeFailurePolicyInterpretation>,
) -> RuntimeTickResult<PathBuf> {
    let path = request_artifact_path(request, "runtime_effect_results", "json");
    let intent = effect_result.source_lifecycle_intent.as_ref();
    let payload = json!({
        "schema_version": "1.0",
        "kind": "runtime_effect_result",
        "request_id": request.request_id,
        "run_id": request.run_id,
        "plane": stage_result.plane.as_str(),
        "stage": stage_result.stage.as_str(),
        "node_id": stage_result.node_id,
        "stage_kind_id": stage_result.stage_kind_id,
        "terminal_result": stage_result.terminal_result.as_str(),
        "rule_id": rule.rule_id,
        "handler_id": effect_result.handler_id,
        "decision": effect_result.decision.as_str(),
        "created_paths": effect_result.created_paths,
        "failure_class": effect_result.failure_class,
        "message": effect_result.message,
        "mutation_phase": mutation_phase_str(effect_result.mutation_phase),
        "source_lifecycle_intent": intent.map(|intent| json!({
            "lifecycle_plan_id": intent.lifecycle_plan_id,
            "action": intent.action.as_str(),
            "work_item_family_id": intent.work_item_family_id,
            "work_item_kind": intent.work_item_kind.map(|kind| kind.as_str()),
            "work_item_id": intent.work_item_id,
        })),
        "failure_policy": failure_policy.map(|policy| json!({
            "policy_id": policy.policy_id,
            "action": policy.action,
            "failure_class": policy.failure_class,
            "target_node_id": policy.target_node_id,
            "target_terminal_state_id": policy.target_terminal_state_id,
            "max_attempts": policy.max_attempts,
            "incident_severity": policy.incident_severity,
        })),
    });
    write_pretty_json(&path, &payload)?;
    Ok(path)
}

fn emit_runtime_effect_event(
    session: &RuntimeStartupSession,
    stage_result: &StageResultEnvelope,
    rule: &RuntimeEffectRuleDefinition,
    effect_result: &RuntimeEffectResult,
    failure_policy: Option<&RuntimeFailurePolicyInterpretation>,
) -> RuntimeTickResult<PathBuf> {
    let intent = effect_result.source_lifecycle_intent.as_ref();
    write_runtime_event(
        &session.paths,
        "runtime_effect_applied",
        json_object([
            (
                "handler_id",
                Value::String(effect_result.handler_id.clone()),
            ),
            ("rule_id", Value::String(rule.rule_id.clone())),
            (
                "decision",
                Value::String(effect_result.decision.as_str().to_owned()),
            ),
            (
                "failure_class",
                optional_string_json(effect_result.failure_class.as_ref()),
            ),
            (
                "message",
                optional_string_json(effect_result.message.as_ref()),
            ),
            (
                "mutation_phase",
                Value::String(mutation_phase_str(effect_result.mutation_phase).to_owned()),
            ),
            (
                "failure_policy_id",
                failure_policy
                    .map(|policy| Value::String(policy.policy_id.clone()))
                    .unwrap_or(Value::Null),
            ),
            (
                "failure_policy_action",
                failure_policy
                    .map(|policy| Value::String(policy.action.clone()))
                    .unwrap_or(Value::Null),
            ),
            (
                "stage_kind_id",
                Value::String(stage_result.stage_kind_id.clone()),
            ),
            (
                "terminal_result",
                Value::String(stage_result.terminal_result.as_str().to_owned()),
            ),
            (
                "work_item_family_id",
                source_work_item_family_id(stage_result)
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            ),
            (
                "work_item_kind",
                Value::String(stage_result.work_item_kind.as_str().to_owned()),
            ),
            (
                "work_item_id",
                Value::String(stage_result.work_item_id.clone()),
            ),
            ("created_paths", json!(effect_result.created_paths)),
            (
                "source_lifecycle_plan_id",
                intent
                    .map(|intent| Value::String(intent.lifecycle_plan_id.clone()))
                    .unwrap_or(Value::Null),
            ),
            (
                "source_lifecycle_action",
                intent
                    .map(|intent| Value::String(intent.action.as_str().to_owned()))
                    .unwrap_or(Value::Null),
            ),
        ]),
        &stage_result.completed_at,
    )
}

fn stage_plan_for_result<'a>(
    plan: &'a CompiledRunPlan,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<&'a MaterializedGraphNodePlan> {
    let graph = graph_for_plane(plan, stage_result.plane)?;
    graph
        .nodes
        .iter()
        .find(|node| {
            node.node_id == stage_result.node_id && node.stage_kind_id == stage_result.stage_kind_id
        })
        .or_else(|| {
            graph
                .nodes
                .iter()
                .find(|node| node.node_id == stage_result.node_id)
        })
        .ok_or_else(|| {
            invalid_state(format!(
                "compiled graph is missing stage result node {}",
                stage_result.node_id
            ))
        })
}

fn graph_for_plane(
    plan: &CompiledRunPlan,
    plane: Plane,
) -> RuntimeTickResult<&FrozenGraphPlanePlan> {
    match plane {
        Plane::Execution => Ok(&plan.execution_graph),
        Plane::Planning => Ok(&plan.planning_graph),
        Plane::Learning => plan
            .learning_graph
            .as_ref()
            .ok_or_else(|| invalid_state("compiled plan has no learning graph")),
    }
}

fn write_stage_result_snapshot(
    path: &Path,
    stage_result: &StageResultEnvelope,
) -> RuntimeTickResult<()> {
    let mut validated = stage_result.clone();
    validated
        .validate()
        .map_err(|error| invalid_state(format!("stage result validation failed: {error}")))?;
    write_pretty_json(path, &validated)
}

fn request_artifact_path(request: &StageRunRequest, directory: &str, extension: &str) -> PathBuf {
    Path::new(&request.run_dir)
        .join(directory)
        .join(format!("{}.{}", request.request_id, extension))
}

fn write_pretty_json<T: Serialize>(path: &Path, value: &T) -> RuntimeTickResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    }
    let payload = serde_json::to_string_pretty(value)
        .map_err(|error| invalid_state(error.to_string()))?
        + "\n";
    atomic_write_text(path, &payload)?;
    Ok(())
}

fn push_unique_artifact_path(stage_result: &mut StageResultEnvelope, path: String) {
    if !stage_result
        .artifact_paths
        .iter()
        .any(|existing| existing == &path)
    {
        stage_result.artifact_paths.push(path);
    }
}

fn run_relative_path(run_dir: &Path, path: &Path) -> String {
    path.strip_prefix(run_dir)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
}

fn effect_path(paths: &WorkspacePaths, path: &str) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        paths.root.join(candidate)
    }
}

fn workspace_relative_path(paths: &WorkspacePaths, path: &Path) -> String {
    path.strip_prefix(&paths.root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn optional_string_json(value: Option<&String>) -> Value {
    value.cloned().map(Value::String).unwrap_or(Value::Null)
}

fn json_object<const N: usize>(entries: [(&str, Value); N]) -> Map<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn mutation_phase_str(phase: RuntimeEffectMutationPhase) -> &'static str {
    match phase {
        RuntimeEffectMutationPhase::PreMutation => "pre_mutation",
        RuntimeEffectMutationPhase::PartialMutation => "partial_mutation",
        RuntimeEffectMutationPhase::Unknown => "unknown",
    }
}

fn invalid_state(message: impl Into<String>) -> RuntimeTickError {
    RuntimeTickError::InvalidState {
        message: message.into(),
    }
}

fn io_error(path: &Path, error: io::Error) -> RuntimeTickError {
    RuntimeTickError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}
