//! Blueprint Planning loop document contracts.

use serde::{Deserialize, Serialize};

use super::{RuntimeJsonContract, RuntimeJsonError, Timestamp, validate_safe_identifier};

const SCHEMA_VERSION: &str = "1.0";

macro_rules! impl_blueprint_contract {
    ($type:ty, $artifact:literal, $kind:literal, $validate:ident) => {
        impl RuntimeJsonContract for $type {
            const ARTIFACT: &'static str = $artifact;

            fn validate_contract(&mut self) -> Result<(), RuntimeJsonError> {
                validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
                validate_literal("kind", &self.kind, $kind)?;
                self.$validate()
            }
        }

        impl $type {
            /// Deserializes and validates this Blueprint document from a JSON value.
            pub fn from_json_value(value: serde_json::Value) -> Result<Self, RuntimeJsonError> {
                <Self as RuntimeJsonContract>::from_json_value(value)
            }
        }
    };
}

/// Status values used by Blueprint draft documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintDraftStatus {
    Queued,
    Active,
    CandidateReady,
    Rejected,
    Approved,
    Canceled,
    Blocked,
    Superseded,
}

/// Evaluation decisions produced by Blueprint evaluator stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintEvaluationDecision {
    Approved,
    Rejected,
    Blocked,
}

/// Source work item kinds that can seed a Blueprint manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintSourceWorkItemKind {
    Spec,
    Incident,
}

/// Manager-produced manifest for a sequence of Blueprint drafts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlueprintManifestDocument {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_blueprint_manifest_kind")]
    pub kind: String,
    pub manifest_id: String,
    pub root_spec_id: String,
    pub root_idea_id: String,
    pub source_work_item_kind: BlueprintSourceWorkItemKind,
    pub source_work_item_id: String,
    pub source_spec_id: String,
    pub draft_ids: Vec<String>,
    pub draft_count: u64,
    #[serde(default = "default_true")]
    pub strict_sequence: bool,
    pub spec_summary: String,
    pub decomposition_strategy: String,
    pub global_acceptance_intent: Vec<String>,
    #[serde(default)]
    pub integration_boundary_notes: Vec<String>,
    #[serde(default)]
    pub risk_notes: Vec<String>,
    #[serde(default)]
    pub references: Vec<String>,
    pub created_at: Timestamp,
    #[serde(default = "default_manager_blueprint")]
    pub created_by: String,
}

impl BlueprintManifestDocument {
    fn validate_manifest(&self) -> Result<(), RuntimeJsonError> {
        validate_ids(&[
            ("manifest_id", &self.manifest_id),
            ("root_spec_id", &self.root_spec_id),
            ("root_idea_id", &self.root_idea_id),
            ("source_work_item_id", &self.source_work_item_id),
            ("source_spec_id", &self.source_spec_id),
        ])?;
        validate_id_list("draft_ids", &self.draft_ids, true)?;
        if self.draft_count != self.draft_ids.len() as u64 {
            return invalid_document("draft_count must equal draft_ids length");
        }
        if !self.strict_sequence {
            return invalid_document("strict_sequence must be true");
        }
        require_non_blank("spec_summary", &self.spec_summary)?;
        require_non_blank("decomposition_strategy", &self.decomposition_strategy)?;
        require_non_empty_texts("global_acceptance_intent", &self.global_acceptance_intent)
    }
}

impl_blueprint_contract!(
    BlueprintManifestDocument,
    "blueprint_manifest",
    "blueprint_manifest",
    validate_manifest
);

/// One manager-produced Blueprint draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlueprintDraftDocument {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_blueprint_draft_kind")]
    pub kind: String,
    pub draft_id: String,
    pub manifest_id: String,
    pub root_spec_id: String,
    pub root_idea_id: String,
    pub source_spec_id: String,
    #[serde(alias = "sequence_number")]
    pub draft_index: u64,
    #[serde(default, alias = "dependency_draft_ids")]
    pub depends_on_draft_ids: Vec<String>,
    pub title: String,
    #[serde(alias = "scope_summary")]
    pub summary: String,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub non_goals: Vec<String>,
    pub target_paths: Vec<String>,
    #[serde(alias = "acceptance")]
    pub acceptance_intent: Vec<String>,
    #[serde(default)]
    pub verification_intent: Vec<String>,
    #[serde(default)]
    pub dependency_notes: Vec<String>,
    #[serde(default)]
    pub integration_boundary_notes: Vec<String>,
    pub context_excerpt: String,
    #[serde(alias = "revision")]
    pub current_revision: u64,
    #[serde(default)]
    pub latest_blueprint_id: Option<String>,
    #[serde(default)]
    pub latest_critique_id: Option<String>,
    #[serde(default = "default_blueprint_draft_status")]
    pub status: BlueprintDraftStatus,
    #[serde(default)]
    pub references: Vec<String>,
    pub created_at: Timestamp,
    #[serde(default = "default_manager_blueprint")]
    pub created_by: String,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
}

impl BlueprintDraftDocument {
    fn validate_draft(&self) -> Result<(), RuntimeJsonError> {
        validate_ids(&[
            ("draft_id", &self.draft_id),
            ("manifest_id", &self.manifest_id),
            ("root_spec_id", &self.root_spec_id),
            ("root_idea_id", &self.root_idea_id),
            ("source_spec_id", &self.source_spec_id),
        ])?;
        validate_optional_id("latest_blueprint_id", &self.latest_blueprint_id)?;
        validate_optional_id("latest_critique_id", &self.latest_critique_id)?;
        validate_id_list("depends_on_draft_ids", &self.depends_on_draft_ids, false)?;
        if self.depends_on_draft_ids.contains(&self.draft_id) {
            return invalid_document("depends_on_draft_ids cannot include draft_id");
        }
        require_non_blank("title", &self.title)?;
        require_non_blank("summary", &self.summary)?;
        require_non_blank("context_excerpt", &self.context_excerpt)?;
        require_non_empty_texts("target_paths", &self.target_paths)?;
        require_non_empty_texts("acceptance_intent", &self.acceptance_intent)
    }
}

impl_blueprint_contract!(
    BlueprintDraftDocument,
    "blueprint_draft",
    "blueprint_draft",
    validate_draft
);

/// Contractor-produced Blueprint packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlueprintPacketDocument {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_blueprint_packet_kind")]
    pub kind: String,
    #[serde(alias = "packet_id")]
    pub blueprint_id: String,
    pub draft_id: String,
    pub manifest_id: String,
    pub root_spec_id: String,
    pub root_idea_id: String,
    pub revision: u64,
    pub title: String,
    pub implementation_scope: Vec<String>,
    pub intended_files: Vec<String>,
    pub design_decisions: Vec<String>,
    #[serde(default)]
    pub non_goals: Vec<String>,
    #[serde(default)]
    pub dependency_assumptions: Vec<String>,
    pub verification_plan: Vec<String>,
    pub task_acceptance: Vec<String>,
    pub required_checks: Vec<String>,
    pub risk_notes: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub references: Vec<String>,
    pub created_at: Timestamp,
    #[serde(default = "default_contractor_blueprint")]
    pub created_by: String,
}

impl BlueprintPacketDocument {
    fn validate_packet(&self) -> Result<(), RuntimeJsonError> {
        validate_ids(&[
            ("blueprint_id", &self.blueprint_id),
            ("draft_id", &self.draft_id),
            ("manifest_id", &self.manifest_id),
            ("root_spec_id", &self.root_spec_id),
            ("root_idea_id", &self.root_idea_id),
        ])?;
        if self.revision == 0 {
            return invalid_field("revision", "must be >= 1");
        }
        require_non_blank("title", &self.title)?;
        for (field, values) in [
            ("implementation_scope", &self.implementation_scope),
            ("intended_files", &self.intended_files),
            ("design_decisions", &self.design_decisions),
            ("verification_plan", &self.verification_plan),
            ("task_acceptance", &self.task_acceptance),
            ("required_checks", &self.required_checks),
            ("risk_notes", &self.risk_notes),
        ] {
            require_non_empty_texts(field, values)?;
        }
        Ok(())
    }

    /// Validates packet identity and revision against the source draft.
    pub fn ensure_matches_draft(
        &self,
        draft: &BlueprintDraftDocument,
    ) -> Result<(), RuntimeJsonError> {
        ensure_equal("draft_id", &self.draft_id, &draft.draft_id)?;
        ensure_equal("manifest_id", &self.manifest_id, &draft.manifest_id)?;
        ensure_equal("root_spec_id", &self.root_spec_id, &draft.root_spec_id)?;
        ensure_equal("root_idea_id", &self.root_idea_id, &draft.root_idea_id)?;
        if self.revision != draft.current_revision + 1 {
            return invalid_document("revision must equal draft current_revision + 1");
        }
        Ok(())
    }
}

impl_blueprint_contract!(
    BlueprintPacketDocument,
    "blueprint_packet",
    "blueprint_packet",
    validate_packet
);

/// Evaluator critique for a rejected Blueprint packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlueprintCritiqueDocument {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_blueprint_critique_kind")]
    pub kind: String,
    pub critique_id: String,
    pub evaluation_id: String,
    pub blueprint_id: String,
    pub draft_id: String,
    pub manifest_id: String,
    pub root_spec_id: String,
    pub root_idea_id: String,
    pub revision: u64,
    #[serde(default)]
    pub required_changes: Vec<String>,
    #[serde(default)]
    pub scope_issues: Vec<String>,
    #[serde(default)]
    pub dependency_issues: Vec<String>,
    #[serde(default)]
    pub verification_issues: Vec<String>,
    #[serde(default)]
    pub acceptance_issues: Vec<String>,
    #[serde(default)]
    pub risk_issues: Vec<String>,
    pub blocking_reason: String,
    #[serde(default)]
    pub resolved_by_blueprint_id: Option<String>,
    #[serde(default)]
    pub resolved_at: Option<Timestamp>,
    #[serde(default)]
    pub references: Vec<String>,
    pub created_at: Timestamp,
    #[serde(default = "default_evaluator_blueprint")]
    pub created_by: String,
}

impl BlueprintCritiqueDocument {
    fn validate_critique(&self) -> Result<(), RuntimeJsonError> {
        validate_ids(&[
            ("critique_id", &self.critique_id),
            ("evaluation_id", &self.evaluation_id),
            ("blueprint_id", &self.blueprint_id),
            ("draft_id", &self.draft_id),
            ("manifest_id", &self.manifest_id),
            ("root_spec_id", &self.root_spec_id),
            ("root_idea_id", &self.root_idea_id),
        ])?;
        if self.revision == 0 {
            return invalid_field("revision", "must be >= 1");
        }
        validate_optional_id("resolved_by_blueprint_id", &self.resolved_by_blueprint_id)?;
        let issue_lists = [
            &self.required_changes,
            &self.scope_issues,
            &self.dependency_issues,
            &self.verification_issues,
            &self.acceptance_issues,
            &self.risk_issues,
        ];
        if !issue_lists.iter().any(|values| !values.is_empty()) {
            return invalid_document("at least one issue list is required");
        }
        for (field, values) in [
            ("required_changes", &self.required_changes),
            ("scope_issues", &self.scope_issues),
            ("dependency_issues", &self.dependency_issues),
            ("verification_issues", &self.verification_issues),
            ("acceptance_issues", &self.acceptance_issues),
            ("risk_issues", &self.risk_issues),
        ] {
            validate_non_empty_entries(field, values)?;
        }
        require_non_blank("blocking_reason", &self.blocking_reason)?;
        if self.resolved_by_blueprint_id.is_some() != self.resolved_at.is_some() {
            return invalid_document(
                "resolved_by_blueprint_id and resolved_at must be set together",
            );
        }
        Ok(())
    }

    /// Validates critique identity and revision against the candidate packet.
    pub fn ensure_matches_packet(
        &self,
        packet: &BlueprintPacketDocument,
    ) -> Result<(), RuntimeJsonError> {
        ensure_equal("blueprint_id", &self.blueprint_id, &packet.blueprint_id)?;
        ensure_equal("draft_id", &self.draft_id, &packet.draft_id)?;
        ensure_equal("manifest_id", &self.manifest_id, &packet.manifest_id)?;
        ensure_equal("root_spec_id", &self.root_spec_id, &packet.root_spec_id)?;
        ensure_equal("root_idea_id", &self.root_idea_id, &packet.root_idea_id)?;
        if self.revision != packet.revision {
            return invalid_document("revision mismatch");
        }
        Ok(())
    }
}

impl_blueprint_contract!(
    BlueprintCritiqueDocument,
    "blueprint_critique",
    "blueprint_critique",
    validate_critique
);

/// Evaluator decision for a Blueprint packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlueprintEvaluationDocument {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_blueprint_evaluation_kind")]
    pub kind: String,
    pub evaluation_id: String,
    pub blueprint_id: String,
    pub draft_id: String,
    pub manifest_id: String,
    pub root_spec_id: String,
    pub root_idea_id: String,
    pub decision: BlueprintEvaluationDecision,
    pub rubric_findings: Vec<String>,
    #[serde(default)]
    pub lineage_consistency_findings: Vec<String>,
    #[serde(default)]
    pub dependency_findings: Vec<String>,
    #[serde(default)]
    pub verification_findings: Vec<String>,
    #[serde(default)]
    pub overlap_findings: Vec<String>,
    #[serde(default)]
    pub required_task_fields: Vec<String>,
    #[serde(default)]
    pub critique_id: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
    pub created_at: Timestamp,
    #[serde(default = "default_evaluator_blueprint")]
    pub created_by: String,
}

impl BlueprintEvaluationDocument {
    fn validate_evaluation(&self) -> Result<(), RuntimeJsonError> {
        validate_ids(&[
            ("evaluation_id", &self.evaluation_id),
            ("blueprint_id", &self.blueprint_id),
            ("draft_id", &self.draft_id),
            ("manifest_id", &self.manifest_id),
            ("root_spec_id", &self.root_spec_id),
            ("root_idea_id", &self.root_idea_id),
        ])?;
        validate_optional_id("critique_id", &self.critique_id)?;
        require_non_empty_texts("rubric_findings", &self.rubric_findings)?;
        if self.decision == BlueprintEvaluationDecision::Approved {
            require_non_empty_texts("required_task_fields", &self.required_task_fields)?;
        }
        if self.decision == BlueprintEvaluationDecision::Rejected && self.critique_id.is_none() {
            return invalid_document("rejected evaluations require critique_id");
        }
        Ok(())
    }

    /// Validates evaluation identity against the candidate packet.
    pub fn ensure_matches_packet(
        &self,
        packet: &BlueprintPacketDocument,
    ) -> Result<(), RuntimeJsonError> {
        ensure_equal("blueprint_id", &self.blueprint_id, &packet.blueprint_id)?;
        ensure_equal("draft_id", &self.draft_id, &packet.draft_id)?;
        ensure_equal("manifest_id", &self.manifest_id, &packet.manifest_id)?;
        ensure_equal("root_spec_id", &self.root_spec_id, &packet.root_spec_id)?;
        ensure_equal("root_idea_id", &self.root_idea_id, &packet.root_idea_id)
    }
}

impl_blueprint_contract!(
    BlueprintEvaluationDocument,
    "blueprint_evaluation",
    "blueprint_evaluation",
    validate_evaluation
);

/// Runtime promotion record for an approved Blueprint packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlueprintPromotionRecord {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_blueprint_promotion_kind")]
    pub kind: String,
    pub promotion_id: String,
    pub blueprint_id: String,
    pub evaluation_id: String,
    pub draft_id: String,
    pub manifest_id: String,
    pub root_spec_id: String,
    pub root_idea_id: String,
    pub generated_task_id: String,
    pub generated_task_path: String,
    pub approved_blueprint_path: String,
    pub evaluation_path: String,
    pub promoted_at: Timestamp,
    #[serde(default = "default_runtime")]
    pub promoted_by: String,
}

impl BlueprintPromotionRecord {
    fn validate_promotion(&self) -> Result<(), RuntimeJsonError> {
        validate_ids(&[
            ("promotion_id", &self.promotion_id),
            ("blueprint_id", &self.blueprint_id),
            ("evaluation_id", &self.evaluation_id),
            ("draft_id", &self.draft_id),
            ("manifest_id", &self.manifest_id),
            ("root_spec_id", &self.root_spec_id),
            ("root_idea_id", &self.root_idea_id),
            ("generated_task_id", &self.generated_task_id),
        ])?;
        require_path_contains(
            "generated_task_path",
            &self.generated_task_path,
            "/tasks/queue/",
        )?;
        require_path_contains(
            "approved_blueprint_path",
            &self.approved_blueprint_path,
            "/blueprints/packets/approved/",
        )?;
        require_path_contains(
            "evaluation_path",
            &self.evaluation_path,
            "/blueprints/evaluations/",
        )?;
        if !self.generated_task_path.contains(&self.generated_task_id) {
            return invalid_document("generated_task_path must reference generated_task_id");
        }
        if !self.approved_blueprint_path.contains(&self.blueprint_id) {
            return invalid_document("approved_blueprint_path must reference blueprint_id");
        }
        if !self.evaluation_path.contains(&self.evaluation_id) {
            return invalid_document("evaluation_path must reference evaluation_id");
        }
        Ok(())
    }

    /// Validates promotion identity against the approving evaluation.
    pub fn ensure_matches_evaluation(
        &self,
        evaluation: &BlueprintEvaluationDocument,
    ) -> Result<(), RuntimeJsonError> {
        ensure_equal("blueprint_id", &self.blueprint_id, &evaluation.blueprint_id)?;
        ensure_equal(
            "evaluation_id",
            &self.evaluation_id,
            &evaluation.evaluation_id,
        )?;
        ensure_equal("draft_id", &self.draft_id, &evaluation.draft_id)?;
        ensure_equal("manifest_id", &self.manifest_id, &evaluation.manifest_id)?;
        ensure_equal("root_spec_id", &self.root_spec_id, &evaluation.root_spec_id)?;
        ensure_equal("root_idea_id", &self.root_idea_id, &evaluation.root_idea_id)
    }
}

impl_blueprint_contract!(
    BlueprintPromotionRecord,
    "blueprint_promotion",
    "blueprint_promotion",
    validate_promotion
);

fn default_schema_version() -> String {
    SCHEMA_VERSION.to_owned()
}

fn default_blueprint_manifest_kind() -> String {
    "blueprint_manifest".to_owned()
}

fn default_blueprint_draft_kind() -> String {
    "blueprint_draft".to_owned()
}

fn default_blueprint_packet_kind() -> String {
    "blueprint_packet".to_owned()
}

fn default_blueprint_critique_kind() -> String {
    "blueprint_critique".to_owned()
}

fn default_blueprint_evaluation_kind() -> String {
    "blueprint_evaluation".to_owned()
}

fn default_blueprint_promotion_kind() -> String {
    "blueprint_promotion".to_owned()
}

fn default_manager_blueprint() -> String {
    "manager_blueprint".to_owned()
}

fn default_contractor_blueprint() -> String {
    "contractor_blueprint".to_owned()
}

fn default_evaluator_blueprint() -> String {
    "evaluator_blueprint".to_owned()
}

fn default_runtime() -> String {
    "runtime".to_owned()
}

const fn default_true() -> bool {
    true
}

const fn default_blueprint_draft_status() -> BlueprintDraftStatus {
    BlueprintDraftStatus::Queued
}

fn validate_literal(
    field_name: &'static str,
    actual: &str,
    expected: &'static str,
) -> Result<(), RuntimeJsonError> {
    if actual == expected {
        Ok(())
    } else {
        Err(RuntimeJsonError::InvalidLiteral {
            field_name,
            expected,
            actual: actual.to_owned(),
        })
    }
}

fn validate_ids(values: &[(&'static str, &str)]) -> Result<(), RuntimeJsonError> {
    for (field, value) in values {
        validate_safe_identifier(value, field)?;
    }
    Ok(())
}

fn validate_optional_id(
    field_name: &'static str,
    value: &Option<String>,
) -> Result<(), RuntimeJsonError> {
    if let Some(value) = value {
        validate_safe_identifier(value, field_name)?;
    }
    Ok(())
}

fn validate_id_list(
    field_name: &'static str,
    values: &[String],
    require_non_empty: bool,
) -> Result<(), RuntimeJsonError> {
    if require_non_empty && values.is_empty() {
        return invalid_field(field_name, "must not be empty");
    }
    let mut seen = std::collections::HashSet::new();
    for value in values {
        validate_safe_identifier(value, field_name)?;
        if !seen.insert(value) {
            return invalid_field(field_name, "must not contain duplicates");
        }
    }
    Ok(())
}

fn require_non_blank(field_name: &'static str, value: &str) -> Result<(), RuntimeJsonError> {
    if value.trim().is_empty() {
        invalid_field(field_name, "must not be empty")
    } else {
        Ok(())
    }
}

fn require_non_empty_texts(
    field_name: &'static str,
    values: &[String],
) -> Result<(), RuntimeJsonError> {
    if values.is_empty() {
        return invalid_field(field_name, "must not be empty");
    }
    validate_non_empty_entries(field_name, values)
}

fn validate_non_empty_entries(
    field_name: &'static str,
    values: &[String],
) -> Result<(), RuntimeJsonError> {
    for value in values {
        require_non_blank(field_name, value)?;
    }
    Ok(())
}

fn require_path_contains(
    field_name: &'static str,
    value: &str,
    expected: &str,
) -> Result<(), RuntimeJsonError> {
    require_non_blank(field_name, value)?;
    let comparable = format!("/{}", value.trim_start_matches('/'));
    if comparable.contains(expected) {
        Ok(())
    } else {
        invalid_field(field_name, "does not contain expected path segment")
    }
}

fn ensure_equal(
    field_name: &'static str,
    actual: &str,
    expected: &str,
) -> Result<(), RuntimeJsonError> {
    if actual == expected {
        Ok(())
    } else {
        invalid_field(field_name, "mismatch")
    }
}

fn invalid_field<T>(field_name: &'static str, message: &str) -> Result<T, RuntimeJsonError> {
    Err(RuntimeJsonError::InvalidField {
        field_name,
        message: message.to_owned(),
    })
}

fn invalid_document<T>(message: &str) -> Result<T, RuntimeJsonError> {
    Err(RuntimeJsonError::InvalidDocument {
        message: message.to_owned(),
    })
}
