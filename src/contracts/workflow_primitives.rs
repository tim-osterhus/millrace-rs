//! Typed contracts for data-driven workflow primitive definitions.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use super::{Plane, RuntimeJsonContract, RuntimeJsonError, validate_safe_identifier};

const SCHEMA_VERSION: &str = "1.0";

pub type WorkflowPrimitiveId = String;
pub type WorkItemFamilyId = String;
pub type DocumentAdapterId = String;
pub type QueueClaimPolicyId = String;
pub type TerminalActionId = String;
pub type LifecycleMutationPlanId = String;
pub type RuntimeEffectHandlerId = String;
pub type RuntimeEffectRuleId = String;
pub type RequestContextProfileId = String;
pub type ArtifactContractId = String;

macro_rules! impl_primitive_contract {
    ($type:ty, $artifact:literal, $kind:literal, $id_field:ident) => {
        impl RuntimeJsonContract for $type {
            const ARTIFACT: &'static str = $artifact;

            fn validate_contract(&mut self) -> Result<(), RuntimeJsonError> {
                validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
                validate_literal("kind", &self.kind, $kind)?;
                validate_safe_identifier(&self.$id_field, stringify!($id_field))?;
                self.validate_definition()
            }
        }

        impl $type {
            /// Deserializes and validates this workflow primitive definition.
            pub fn from_json_value(value: serde_json::Value) -> Result<Self, RuntimeJsonError> {
                <Self as RuntimeJsonContract>::from_json_value(value)
            }
        }
    };
}

/// Artifact payload formats used by filename adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFormat {
    Json,
    Markdown,
    Text,
    Directory,
}

/// Runtime-effect mutation phase categories used by failure policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEffectMutationPhase {
    PreMutation,
    PartialMutation,
    Unknown,
}

/// Parser/renderer binding for one artifact filename.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactFilenameAdapterDefinition {
    pub filename: String,
    pub format: ArtifactFormat,
    pub parser_id: String,
    #[serde(default)]
    pub renderer_id: Option<String>,
}

impl ArtifactFilenameAdapterDefinition {
    fn validate(&self) -> Result<(), RuntimeJsonError> {
        validate_filename("filename", &self.filename)?;
        validate_safe_identifier(&self.parser_id, "parser_id")?;
        validate_optional_id("renderer_id", &self.renderer_id)
    }
}

/// Artifact contract registry definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactContractDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_artifact_contract_kind")]
    pub kind: String,
    pub artifact_id: ArtifactContractId,
    pub canonical_filename: String,
    #[serde(default)]
    pub accepted_filenames: Vec<String>,
    pub preferred_format: ArtifactFormat,
    pub schema_id: String,
    pub filename_adapters: Vec<ArtifactFilenameAdapterDefinition>,
    #[serde(default)]
    pub producer_stage_kind_ids: Vec<String>,
    #[serde(default)]
    pub consumer_handler_ids: Vec<RuntimeEffectHandlerId>,
    #[serde(default)]
    pub destination_family_id: Option<WorkItemFamilyId>,
}

impl ArtifactContractDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_filename("canonical_filename", &self.canonical_filename)?;
        validate_id_list("accepted_filenames", &self.accepted_filenames, false, false)?;
        validate_safe_identifier(&self.schema_id, "schema_id")?;
        validate_id_list(
            "producer_stage_kind_ids",
            &self.producer_stage_kind_ids,
            false,
            true,
        )?;
        validate_id_list(
            "consumer_handler_ids",
            &self.consumer_handler_ids,
            false,
            true,
        )?;
        validate_optional_id("destination_family_id", &self.destination_family_id)?;
        if self.filename_adapters.is_empty() {
            return invalid_field("filename_adapters", "must not be empty");
        }
        for adapter in &self.filename_adapters {
            adapter.validate()?;
        }
        let mut filenames = vec![self.canonical_filename.as_str()];
        filenames.extend(self.accepted_filenames.iter().map(String::as_str));
        let adapter_names: Vec<_> = self
            .filename_adapters
            .iter()
            .map(|adapter| adapter.filename.as_str())
            .collect();
        for filename in filenames {
            if !adapter_names.contains(&filename) {
                return invalid_document("filename_adapters must cover every declared filename");
            }
        }
        Ok(())
    }
}

impl_primitive_contract!(
    ArtifactContractDefinition,
    "artifact_contract",
    "artifact_contract",
    artifact_id
);

/// Queue directories for one work-item family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkItemQueueDirs {
    pub queue: String,
    pub active: String,
    pub done: String,
    pub blocked: String,
    #[serde(default)]
    pub canceled: Option<String>,
    #[serde(default)]
    pub superseded: Option<String>,
}

impl WorkItemQueueDirs {
    fn validate(&self) -> Result<(), RuntimeJsonError> {
        for (field, value) in [
            ("queue", &self.queue),
            ("active", &self.active),
            ("done", &self.done),
            ("blocked", &self.blocked),
        ] {
            validate_runtime_relative_path(field, value)?;
        }
        validate_optional_runtime_path("canceled", &self.canceled)?;
        validate_optional_runtime_path("superseded", &self.superseded)
    }
}

/// Work-item family registry definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkItemFamilyDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_work_item_family_kind")]
    pub kind: String,
    pub family_id: WorkItemFamilyId,
    pub plane: Plane,
    pub entry_key: String,
    pub display_name: String,
    pub document_kind: String,
    pub runtime_relative_dir: String,
    #[serde(default = "default_json_extension")]
    pub file_extension: String,
    pub schema_id: String,
    pub document_adapter_id: DocumentAdapterId,
    pub queue_dirs: WorkItemQueueDirs,
    pub lifecycle_states: Vec<String>,
    #[serde(default = "default_queued")]
    pub claimable_state: String,
    #[serde(default = "default_active")]
    pub active_state: String,
    #[serde(default = "default_done")]
    pub done_state: String,
    #[serde(default = "default_blocked")]
    pub blocked_state: String,
    #[serde(default)]
    pub canceled_state: Option<String>,
    #[serde(default)]
    pub closure_blocking_states: Vec<String>,
    #[serde(default)]
    pub default_entry_key: Option<String>,
    #[serde(default)]
    pub id_field: Option<String>,
    #[serde(default = "default_created_at_field")]
    pub created_at_field: String,
    #[serde(default)]
    pub lineage_fields: Vec<String>,
    #[serde(default)]
    pub dependency_field: Option<String>,
    #[serde(default = "default_plane_scope")]
    pub one_active_policy: String,
    #[serde(default = "default_fail_policy")]
    pub duplicate_policy: String,
    #[serde(default = "default_block_source_policy")]
    pub invalid_artifact_policy: String,
    #[serde(default = "default_created_at_asc")]
    pub sort_policy: String,
    #[serde(default)]
    pub operator_capabilities: Vec<String>,
}

impl WorkItemFamilyDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_safe_identifier(&self.entry_key, "entry_key")?;
        require_non_blank("display_name", &self.display_name)?;
        validate_safe_identifier(&self.document_kind, "document_kind")?;
        validate_runtime_relative_path("runtime_relative_dir", &self.runtime_relative_dir)?;
        validate_file_extension("file_extension", &self.file_extension)?;
        validate_safe_identifier(&self.schema_id, "schema_id")?;
        validate_safe_identifier(&self.document_adapter_id, "document_adapter_id")?;
        self.queue_dirs.validate()?;
        validate_id_list("lifecycle_states", &self.lifecycle_states, true, true)?;
        for (field, state) in [
            ("claimable_state", &self.claimable_state),
            ("active_state", &self.active_state),
            ("done_state", &self.done_state),
            ("blocked_state", &self.blocked_state),
        ] {
            validate_safe_identifier(state, field)?;
            if !self.lifecycle_states.contains(state) {
                return invalid_field(field, "must be declared in lifecycle_states");
            }
        }
        validate_optional_id("canceled_state", &self.canceled_state)?;
        validate_id_list(
            "closure_blocking_states",
            &self.closure_blocking_states,
            false,
            true,
        )?;
        validate_optional_id("default_entry_key", &self.default_entry_key)?;
        validate_optional_id("id_field", &self.id_field)?;
        validate_safe_identifier(&self.created_at_field, "created_at_field")?;
        validate_id_list("lineage_fields", &self.lineage_fields, false, true)?;
        validate_optional_id("dependency_field", &self.dependency_field)?;
        validate_id_list(
            "operator_capabilities",
            &self.operator_capabilities,
            false,
            true,
        )
    }
}

impl_primitive_contract!(
    WorkItemFamilyDefinition,
    "work_item_family",
    "work_item_family",
    family_id
);

/// Work-item document adapter registry definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkItemDocumentAdapterDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_document_adapter_kind")]
    pub kind: String,
    pub adapter_id: DocumentAdapterId,
    pub schema_id: String,
    pub supported_file_extensions: Vec<String>,
    pub family_ids: Vec<WorkItemFamilyId>,
    pub can_parse: bool,
    pub can_render: bool,
    pub can_summarize: bool,
    pub supports_dependencies: bool,
    pub supports_lineage: bool,
}

impl WorkItemDocumentAdapterDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_safe_identifier(&self.schema_id, "schema_id")?;
        if self.supported_file_extensions.is_empty() {
            return invalid_field("supported_file_extensions", "must not be empty");
        }
        for extension in &self.supported_file_extensions {
            validate_file_extension("supported_file_extensions", extension)?;
        }
        validate_id_list("family_ids", &self.family_ids, true, true)?;
        if !(self.can_parse || self.can_render || self.can_summarize) {
            return invalid_document("document adapter must support at least one operation");
        }
        Ok(())
    }
}

impl_primitive_contract!(
    WorkItemDocumentAdapterDefinition,
    "work_item_document_adapter",
    "work_item_document_adapter",
    adapter_id
);

/// Work-item partition selector registry definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkItemPartitionSelectorDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_partition_selector_kind")]
    pub kind: String,
    pub selector_id: String,
    pub family_id: WorkItemFamilyId,
    pub output_kind: String,
    pub supports_static_compile_check: bool,
}

impl WorkItemPartitionSelectorDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_safe_identifier(&self.family_id, "family_id")?;
        validate_safe_identifier(&self.output_kind, "output_kind")?;
        Ok(())
    }
}

impl_primitive_contract!(
    WorkItemPartitionSelectorDefinition,
    "work_item_partition_selector",
    "work_item_partition_selector",
    selector_id
);

/// Queue claim policy for a runtime plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaneQueueClaimPolicyDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_queue_claim_policy_kind")]
    pub kind: String,
    pub policy_id: QueueClaimPolicyId,
    pub plane: Plane,
    #[serde(default)]
    pub family_order: Vec<WorkItemFamilyId>,
    #[serde(default = "default_defer_unrelated")]
    pub closure_lineage_policy: String,
    #[serde(default = "default_idle")]
    pub empty_behavior: String,
}

impl PlaneQueueClaimPolicyDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_id_list("family_order", &self.family_order, false, true)
    }
}

impl_primitive_contract!(
    PlaneQueueClaimPolicyDefinition,
    "plane_queue_claim_policy",
    "plane_queue_claim_policy",
    policy_id
);

/// Scheduler lane definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLaneDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_workflow_lane_kind")]
    pub kind: String,
    pub lane_id: String,
    pub plane: Plane,
    #[serde(alias = "accepted_family_ids")]
    pub allowed_family_ids: Vec<WorkItemFamilyId>,
    pub claim_policy_id: QueueClaimPolicyId,
    #[serde(default = "default_one")]
    pub max_active_runs: u64,
    #[serde(default = "default_plane_scope")]
    pub one_active_scope: String,
    #[serde(default)]
    pub partition_selector_id: Option<String>,
    #[serde(default = "default_plane_scope")]
    pub mutation_lock_scope: String,
    #[serde(default = "default_single_writer_serialized")]
    pub result_application_policy: String,
    #[serde(default)]
    pub conflict_policy_id: Option<String>,
}

impl WorkflowLaneDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_safe_identifier(&self.lane_id, "lane_id")?;
        validate_id_list("allowed_family_ids", &self.allowed_family_ids, true, true)?;
        validate_safe_identifier(&self.claim_policy_id, "claim_policy_id")?;
        validate_optional_id("partition_selector_id", &self.partition_selector_id)?;
        validate_optional_id("conflict_policy_id", &self.conflict_policy_id)?;
        if self.max_active_runs == 0 {
            return invalid_field("max_active_runs", "must be >= 1");
        }
        Ok(())
    }
}

impl_primitive_contract!(
    WorkflowLaneDefinition,
    "workflow_lane",
    "workflow_lane",
    lane_id
);

/// Lane conflict policy definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaneConflictPolicyDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_lane_conflict_policy_kind")]
    pub kind: String,
    pub policy_id: String,
    #[serde(default)]
    pub lane_ids: Vec<String>,
    #[serde(default)]
    pub concurrent_with_lane_ids: Vec<String>,
    pub conflict_scopes: Vec<String>,
    #[serde(default)]
    pub lock_acquisition_order: Vec<String>,
    #[serde(default = "default_after_result_application")]
    pub release_policy: String,
    #[serde(default = "default_reject_compile")]
    pub missing_lock_policy: String,
}

impl LaneConflictPolicyDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_id_list("lane_ids", &self.lane_ids, true, true)?;
        validate_id_list(
            "concurrent_with_lane_ids",
            &self.concurrent_with_lane_ids,
            true,
            true,
        )?;
        validate_id_list("conflict_scopes", &self.conflict_scopes, true, true)?;
        validate_id_list(
            "lock_acquisition_order",
            &self.lock_acquisition_order,
            false,
            true,
        )
    }
}

impl_primitive_contract!(
    LaneConflictPolicyDefinition,
    "lane_conflict_policy",
    "lane_conflict_policy",
    policy_id
);

/// Terminal action definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalActionDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_terminal_action_kind")]
    pub kind: String,
    pub terminal_action_id: TerminalActionId,
    pub terminal_class: String,
    #[serde(default)]
    pub lifecycle_mutation_plan_id: Option<LifecycleMutationPlanId>,
    #[serde(default)]
    pub effect_rule_ids: Vec<RuntimeEffectRuleId>,
    #[serde(default)]
    pub status_marker: Option<String>,
    #[serde(default)]
    pub operator_summary_template: Option<String>,
    #[serde(default)]
    pub non_mutating: bool,
}

impl TerminalActionDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_safe_identifier(&self.terminal_class, "terminal_class")?;
        validate_optional_id(
            "lifecycle_mutation_plan_id",
            &self.lifecycle_mutation_plan_id,
        )?;
        validate_id_list("effect_rule_ids", &self.effect_rule_ids, false, true)?;
        validate_optional_non_blank("status_marker", &self.status_marker)?;
        validate_optional_non_blank("operator_summary_template", &self.operator_summary_template)?;
        if !self.non_mutating && self.lifecycle_mutation_plan_id.is_none() {
            return invalid_document(
                "lifecycle_mutation_plan_id is required for mutating terminal actions",
            );
        }
        Ok(())
    }
}

impl_primitive_contract!(
    TerminalActionDefinition,
    "terminal_action",
    "terminal_action",
    terminal_action_id
);

/// Lifecycle mutation plan definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleMutationPlanDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_lifecycle_mutation_plan_kind")]
    pub kind: String,
    pub plan_id: LifecycleMutationPlanId,
    pub source_node_id: String,
    pub outcome_id: String,
    pub source_family_id: WorkItemFamilyId,
    pub owner: String,
    pub source_from_state: String,
    #[serde(default)]
    pub source_to_state: Option<String>,
    pub ordering: String,
    #[serde(default)]
    pub lifecycle_action_id: Option<String>,
}

impl LifecycleMutationPlanDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_safe_identifier(&self.source_node_id, "source_node_id")?;
        validate_safe_identifier(&self.outcome_id, "outcome_id")?;
        validate_safe_identifier(&self.source_family_id, "source_family_id")?;
        validate_safe_identifier(&self.owner, "owner")?;
        validate_safe_identifier(&self.source_from_state, "source_from_state")?;
        validate_optional_id("source_to_state", &self.source_to_state)?;
        validate_safe_identifier(&self.ordering, "ordering")?;
        validate_optional_id("lifecycle_action_id", &self.lifecycle_action_id)
    }
}

impl_primitive_contract!(
    LifecycleMutationPlanDefinition,
    "lifecycle_mutation_plan",
    "lifecycle_mutation_plan",
    plan_id
);

/// Runtime effect handler definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEffectHandlerDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_runtime_effect_handler_kind")]
    pub kind: String,
    pub handler_id: RuntimeEffectHandlerId,
    pub source_planes: Vec<Plane>,
    #[serde(default)]
    pub allowed_source_families: Vec<WorkItemFamilyId>,
    #[serde(default)]
    pub destination_kinds: Vec<String>,
    #[serde(default)]
    pub required_artifacts: Vec<String>,
    #[serde(default)]
    pub optional_artifacts: Vec<String>,
    pub returns_source_lifecycle_intent: bool,
    pub requires_lifecycle_mutation_plan: bool,
    pub creates_work_items: bool,
    #[serde(default)]
    pub creates_incidents: bool,
    #[serde(default)]
    pub creates_closure_targets: bool,
    pub failure_classes: Vec<String>,
}

impl RuntimeEffectHandlerDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        if self.source_planes.is_empty() {
            return invalid_field("source_planes", "must not be empty");
        }
        validate_id_list(
            "allowed_source_families",
            &self.allowed_source_families,
            false,
            true,
        )?;
        validate_id_list("destination_kinds", &self.destination_kinds, false, true)?;
        validate_id_list("required_artifacts", &self.required_artifacts, false, true)?;
        validate_id_list("optional_artifacts", &self.optional_artifacts, false, true)?;
        validate_id_list("failure_classes", &self.failure_classes, true, true)?;
        if self.returns_source_lifecycle_intent && !self.requires_lifecycle_mutation_plan {
            return invalid_document(
                "requires_lifecycle_mutation_plan must be true when lifecycle intent is returned",
            );
        }
        if self.creates_work_items && self.destination_kinds.is_empty() {
            return invalid_field(
                "destination_kinds",
                "required when creates_work_items is true",
            );
        }
        Ok(())
    }
}

impl_primitive_contract!(
    RuntimeEffectHandlerDefinition,
    "runtime_effect_handler",
    "runtime_effect_handler",
    handler_id
);

/// Runtime effect rule definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEffectRuleDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_runtime_effect_rule_kind")]
    pub kind: String,
    pub rule_id: RuntimeEffectRuleId,
    pub effect_operation_id: String,
    pub source_node_id: String,
    pub on_outcomes: Vec<String>,
    pub handler_id: RuntimeEffectHandlerId,
    #[serde(default)]
    pub required_run_artifacts: Vec<String>,
    #[serde(default)]
    pub destination_family_id: Option<WorkItemFamilyId>,
    #[serde(default)]
    pub creates_work_items: bool,
    pub duplicate_policy: String,
    pub partial_commit_policy: String,
    pub replay_policy: String,
    pub lineage_policy: String,
    pub applies_before_route: bool,
    #[serde(default)]
    pub lifecycle_mutation_plan_id: Option<LifecycleMutationPlanId>,
}

impl RuntimeEffectRuleDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_safe_identifier(&self.effect_operation_id, "effect_operation_id")?;
        validate_safe_identifier(&self.source_node_id, "source_node_id")?;
        validate_id_list("on_outcomes", &self.on_outcomes, true, true)?;
        validate_safe_identifier(&self.handler_id, "handler_id")?;
        validate_id_list(
            "required_run_artifacts",
            &self.required_run_artifacts,
            false,
            true,
        )?;
        validate_optional_id("destination_family_id", &self.destination_family_id)?;
        validate_safe_identifier(&self.duplicate_policy, "duplicate_policy")?;
        validate_safe_identifier(&self.partial_commit_policy, "partial_commit_policy")?;
        validate_safe_identifier(&self.replay_policy, "replay_policy")?;
        validate_safe_identifier(&self.lineage_policy, "lineage_policy")?;
        validate_optional_id(
            "lifecycle_mutation_plan_id",
            &self.lifecycle_mutation_plan_id,
        )?;
        if self.creates_work_items && self.destination_family_id.is_none() {
            return invalid_document(
                "destination_family_id is required when creates_work_items is true",
            );
        }
        Ok(())
    }
}

impl_primitive_contract!(
    RuntimeEffectRuleDefinition,
    "runtime_effect_rule",
    "runtime_effect_rule",
    rule_id
);

/// Outcome artifact declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeArtifactDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_outcome_artifact_kind")]
    pub kind: String,
    pub outcome_id: String,
    #[serde(default)]
    pub required_artifacts: Vec<String>,
    #[serde(default)]
    pub optional_artifacts: Vec<String>,
}

impl OutcomeArtifactDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_id_list("required_artifacts", &self.required_artifacts, false, true)?;
        validate_id_list("optional_artifacts", &self.optional_artifacts, false, true)
    }
}

impl_primitive_contract!(
    OutcomeArtifactDefinition,
    "outcome_artifact",
    "outcome_artifact",
    outcome_id
);

/// Request-context profile definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestContextProfileDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_request_context_profile_kind")]
    pub kind: String,
    pub profile_id: RequestContextProfileId,
    pub request_kind: String,
    pub required_providers: Vec<String>,
    #[serde(default)]
    pub optional_providers: Vec<String>,
    #[serde(default)]
    pub output_path_preferences: BTreeMap<String, String>,
    pub visibility_policy: String,
}

impl RequestContextProfileDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_safe_identifier(&self.request_kind, "request_kind")?;
        validate_id_list("required_providers", &self.required_providers, true, true)?;
        validate_id_list("optional_providers", &self.optional_providers, false, true)?;
        for (key, path) in &self.output_path_preferences {
            validate_safe_identifier(key, "output_path_preferences key")?;
            validate_runtime_relative_path("output_path_preferences", path)?;
        }
        validate_safe_identifier(&self.visibility_policy, "visibility_policy")?;
        Ok(())
    }
}

impl_primitive_contract!(
    RequestContextProfileDefinition,
    "request_context_profile",
    "request_context_profile",
    profile_id
);

/// Request-context render plan definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestContextRenderPlan {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_request_context_render_plan_kind")]
    pub kind: String,
    pub render_plan_id: String,
    pub profile_id: RequestContextProfileId,
    pub bundle_schema_version: String,
    pub section_order: Vec<String>,
    pub artifact_ref_policy: String,
    pub redaction_policy_id: String,
    #[serde(default)]
    pub max_inline_bytes_by_role: BTreeMap<String, u64>,
    pub missing_optional_provider_policy: String,
}

impl RequestContextRenderPlan {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_safe_identifier(&self.profile_id, "profile_id")?;
        require_non_blank("bundle_schema_version", &self.bundle_schema_version)?;
        validate_id_list("section_order", &self.section_order, true, true)?;
        validate_safe_identifier(&self.artifact_ref_policy, "artifact_ref_policy")?;
        validate_safe_identifier(&self.redaction_policy_id, "redaction_policy_id")?;
        for key in self.max_inline_bytes_by_role.keys() {
            validate_safe_identifier(key, "max_inline_bytes_by_role key")?;
        }
        validate_safe_identifier(
            &self.missing_optional_provider_policy,
            "missing_optional_provider_policy",
        )?;
        Ok(())
    }
}

impl_primitive_contract!(
    RequestContextRenderPlan,
    "request_context_render_plan",
    "request_context_render_plan",
    render_plan_id
);

/// Workflow completion behavior definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCompletionBehaviorDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_completion_behavior_kind")]
    pub kind: String,
    pub behavior_id: String,
    pub plane: Plane,
    #[serde(default = "default_backlog_drained")]
    pub trigger: String,
    pub target_scope: String,
    pub readiness_handler_ids: Vec<String>,
    pub target_entry_key: String,
    pub target_node_id: String,
    pub request_context_profile_id: RequestContextProfileId,
    pub target_selector: String,
    pub backpressure_policy: String,
    #[serde(default = "default_true")]
    pub skip_if_already_closed: bool,
    pub terminal_action_by_outcome: BTreeMap<String, TerminalActionId>,
}

impl WorkflowCompletionBehaviorDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_safe_identifier(&self.trigger, "trigger")?;
        validate_safe_identifier(&self.target_scope, "target_scope")?;
        validate_id_list(
            "readiness_handler_ids",
            &self.readiness_handler_ids,
            true,
            true,
        )?;
        validate_safe_identifier(&self.target_entry_key, "target_entry_key")?;
        validate_safe_identifier(&self.target_node_id, "target_node_id")?;
        validate_safe_identifier(
            &self.request_context_profile_id,
            "request_context_profile_id",
        )?;
        validate_safe_identifier(&self.target_selector, "target_selector")?;
        validate_safe_identifier(&self.backpressure_policy, "backpressure_policy")?;
        if self.terminal_action_by_outcome.is_empty() {
            return invalid_field("terminal_action_by_outcome", "must not be empty");
        }
        for (outcome, action) in &self.terminal_action_by_outcome {
            validate_safe_identifier(outcome, "terminal_action_by_outcome outcome")?;
            validate_safe_identifier(action, "terminal_action_by_outcome action")?;
        }
        Ok(())
    }
}

impl_primitive_contract!(
    WorkflowCompletionBehaviorDefinition,
    "workflow_completion_behavior",
    "workflow_completion_behavior",
    behavior_id
);

/// Workflow recovery policy definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRecoveryPolicyDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_recovery_policy_kind")]
    pub kind: String,
    pub policy_id: String,
    pub source_node_ids: Vec<String>,
    pub on_outcomes: Vec<String>,
    pub counter_name: String,
    pub threshold: u64,
    #[serde(default)]
    pub retry_target_node_id: Option<String>,
    #[serde(default)]
    pub exhausted_target_node_id: Option<String>,
    #[serde(default)]
    pub exhausted_terminal_state_id: Option<String>,
    pub failure_class_template: String,
}

impl WorkflowRecoveryPolicyDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_id_list("source_node_ids", &self.source_node_ids, true, true)?;
        validate_id_list("on_outcomes", &self.on_outcomes, true, true)?;
        validate_safe_identifier(&self.counter_name, "counter_name")?;
        if self.threshold == 0 {
            return invalid_field("threshold", "must be >= 1");
        }
        validate_optional_id("retry_target_node_id", &self.retry_target_node_id)?;
        validate_optional_id("exhausted_target_node_id", &self.exhausted_target_node_id)?;
        validate_optional_id(
            "exhausted_terminal_state_id",
            &self.exhausted_terminal_state_id,
        )?;
        validate_safe_identifier(&self.failure_class_template, "failure_class_template")?;
        if self.exhausted_target_node_id.is_some() == self.exhausted_terminal_state_id.is_some() {
            return invalid_document("recovery policy must declare exactly one exhausted target");
        }
        Ok(())
    }
}

impl_primitive_contract!(
    WorkflowRecoveryPolicyDefinition,
    "workflow_recovery_policy",
    "workflow_recovery_policy",
    policy_id
);

/// Runtime failure policy definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFailurePolicyDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_runtime_failure_policy_kind")]
    pub kind: String,
    pub policy_id: String,
    pub applies_to_origins: Vec<String>,
    pub applies_to_planes: Vec<Plane>,
    #[serde(default)]
    pub applies_to_families: Vec<WorkItemFamilyId>,
    #[serde(default)]
    pub applies_to_failure_classes: Vec<String>,
    #[serde(default)]
    pub applies_to_mutation_phases: Vec<RuntimeEffectMutationPhase>,
    #[serde(default)]
    pub applies_to_handler_ids: Vec<RuntimeEffectHandlerId>,
    #[serde(default)]
    pub applies_to_source_node_ids: Vec<String>,
    #[serde(default)]
    pub applies_to_source_terminal_state_ids: Vec<String>,
    pub action: String,
    #[serde(default)]
    pub threshold: Option<u64>,
    #[serde(default)]
    pub counter_name: Option<String>,
    pub failure_class_template: String,
    #[serde(default)]
    pub recovery_node_id: Option<String>,
    #[serde(default)]
    pub target_node_id: Option<String>,
    #[serde(default)]
    pub target_terminal_state_id: Option<String>,
    #[serde(default)]
    pub max_attempts: Option<u64>,
    #[serde(default)]
    pub incident_severity: Option<String>,
}

impl RuntimeFailurePolicyDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_id_list("applies_to_origins", &self.applies_to_origins, true, true)?;
        if self.applies_to_planes.is_empty() {
            return invalid_field("applies_to_planes", "must not be empty");
        }
        validate_id_list(
            "applies_to_families",
            &self.applies_to_families,
            false,
            true,
        )?;
        validate_id_list(
            "applies_to_failure_classes",
            &self.applies_to_failure_classes,
            false,
            true,
        )?;
        validate_id_list(
            "applies_to_handler_ids",
            &self.applies_to_handler_ids,
            false,
            true,
        )?;
        validate_id_list(
            "applies_to_source_node_ids",
            &self.applies_to_source_node_ids,
            false,
            true,
        )?;
        validate_id_list(
            "applies_to_source_terminal_state_ids",
            &self.applies_to_source_terminal_state_ids,
            false,
            true,
        )?;
        validate_safe_identifier(&self.action, "action")?;
        validate_optional_id("counter_name", &self.counter_name)?;
        validate_safe_identifier(&self.failure_class_template, "failure_class_template")?;
        validate_optional_id("recovery_node_id", &self.recovery_node_id)?;
        validate_optional_id("target_node_id", &self.target_node_id)?;
        validate_optional_id("target_terminal_state_id", &self.target_terminal_state_id)?;
        validate_optional_id("incident_severity", &self.incident_severity)?;
        if self.threshold.is_some() != self.counter_name.is_some() {
            return invalid_document("threshold and counter_name must be declared together");
        }
        Ok(())
    }
}

impl_primitive_contract!(
    RuntimeFailurePolicyDefinition,
    "runtime_failure_policy",
    "runtime_failure_policy",
    policy_id
);

/// Scheduler policy for workflow planes and lanes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowPlaneSchedulerPolicyDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_scheduler_policy_kind")]
    pub kind: String,
    pub policy_id: String,
    pub plane_order: Vec<Plane>,
    #[serde(default)]
    pub concurrency_policy_id: Option<String>,
    pub lanes: Vec<WorkflowLaneDefinition>,
    pub claim_policies_by_plane: HashMap<Plane, PlaneQueueClaimPolicyDefinition>,
    #[serde(default)]
    pub completion_check_order: Vec<Plane>,
    #[serde(default)]
    pub experimental_multi_lane: bool,
    #[serde(default)]
    pub lane_conflict_policies: Vec<LaneConflictPolicyDefinition>,
}

impl WorkflowPlaneSchedulerPolicyDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        if self.plane_order.is_empty() {
            return invalid_field("plane_order", "must not be empty");
        }
        validate_optional_id("concurrency_policy_id", &self.concurrency_policy_id)?;
        if self.lanes.is_empty() {
            return invalid_field("lanes", "must not be empty");
        }
        for lane in &self.lanes {
            let mut lane = lane.clone();
            lane.validate_contract()?;
        }
        for (plane, policy) in &self.claim_policies_by_plane {
            if policy.plane != *plane {
                return invalid_document("claim_policies_by_plane keys must match policy plane");
            }
        }
        for policy in &self.lane_conflict_policies {
            let mut policy = policy.clone();
            policy.validate_contract()?;
        }
        Ok(())
    }
}

impl_primitive_contract!(
    WorkflowPlaneSchedulerPolicyDefinition,
    "workflow_plane_scheduler_policy",
    "workflow_plane_scheduler_policy",
    policy_id
);

/// Operator control capability definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorControlCapabilityDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_operator_control_capability_kind")]
    pub kind: String,
    pub capability_id: String,
    pub action: String,
    pub target_type: String,
    #[serde(default)]
    pub plane: Option<Plane>,
    #[serde(default)]
    pub family_ids: Vec<WorkItemFamilyId>,
    #[serde(default)]
    pub lane_ids: Vec<String>,
    #[serde(default)]
    pub allowed_lifecycle_states: Vec<String>,
}

impl OperatorControlCapabilityDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_safe_identifier(&self.action, "action")?;
        validate_safe_identifier(&self.target_type, "target_type")?;
        validate_id_list("family_ids", &self.family_ids, false, true)?;
        validate_id_list("lane_ids", &self.lane_ids, false, true)?;
        validate_id_list(
            "allowed_lifecycle_states",
            &self.allowed_lifecycle_states,
            false,
            true,
        )?;
        Ok(())
    }
}

impl_primitive_contract!(
    OperatorControlCapabilityDefinition,
    "operator_control_capability",
    "operator_control_capability",
    capability_id
);

/// Workspace schema epoch definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSchemaEpochDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_workspace_schema_epoch_kind")]
    pub kind: String,
    pub epoch_id: String,
    pub minimum_supported_epoch_id: String,
    #[serde(default)]
    pub archive_required_from_epoch_ids: Vec<String>,
    pub reset_command: String,
    #[serde(default)]
    pub compatibility_notes: Vec<String>,
}

impl WorkspaceSchemaEpochDefinition {
    fn validate_definition(&self) -> Result<(), RuntimeJsonError> {
        validate_safe_identifier(
            &self.minimum_supported_epoch_id,
            "minimum_supported_epoch_id",
        )?;
        validate_id_list(
            "archive_required_from_epoch_ids",
            &self.archive_required_from_epoch_ids,
            false,
            true,
        )?;
        require_non_blank("reset_command", &self.reset_command)?;
        validate_non_empty_entries("compatibility_notes", &self.compatibility_notes)
    }
}

impl_primitive_contract!(
    WorkspaceSchemaEpochDefinition,
    "workspace_schema_epoch",
    "workspace_schema_epoch",
    epoch_id
);

/// Discovered built-in workflow primitive bundle.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkflowPrimitiveBundle {
    pub artifact_contracts: Vec<ArtifactContractDefinition>,
    pub request_context_profiles: Vec<RequestContextProfileDefinition>,
    pub work_item_families: Vec<WorkItemFamilyDefinition>,
    pub document_adapters: Vec<WorkItemDocumentAdapterDefinition>,
    pub queue_claim_policies: Vec<PlaneQueueClaimPolicyDefinition>,
    pub terminal_actions: Vec<TerminalActionDefinition>,
    pub lifecycle_mutation_plans: Vec<LifecycleMutationPlanDefinition>,
    pub runtime_effect_handlers: Vec<RuntimeEffectHandlerDefinition>,
    pub runtime_effect_rules: Vec<RuntimeEffectRuleDefinition>,
    pub recovery_policies: Vec<WorkflowRecoveryPolicyDefinition>,
    pub runtime_failure_policies: Vec<RuntimeFailurePolicyDefinition>,
    pub workspace_schema_epoch: Option<WorkspaceSchemaEpochDefinition>,
}

fn default_schema_version() -> String {
    SCHEMA_VERSION.to_owned()
}

fn default_artifact_contract_kind() -> String {
    "artifact_contract".to_owned()
}
fn default_work_item_family_kind() -> String {
    "work_item_family".to_owned()
}
fn default_document_adapter_kind() -> String {
    "work_item_document_adapter".to_owned()
}
fn default_partition_selector_kind() -> String {
    "work_item_partition_selector".to_owned()
}
fn default_queue_claim_policy_kind() -> String {
    "plane_queue_claim_policy".to_owned()
}
fn default_workflow_lane_kind() -> String {
    "workflow_lane".to_owned()
}
fn default_lane_conflict_policy_kind() -> String {
    "lane_conflict_policy".to_owned()
}
fn default_terminal_action_kind() -> String {
    "terminal_action".to_owned()
}
fn default_lifecycle_mutation_plan_kind() -> String {
    "lifecycle_mutation_plan".to_owned()
}
fn default_runtime_effect_handler_kind() -> String {
    "runtime_effect_handler".to_owned()
}
fn default_runtime_effect_rule_kind() -> String {
    "runtime_effect_rule".to_owned()
}
fn default_outcome_artifact_kind() -> String {
    "outcome_artifact".to_owned()
}
fn default_request_context_profile_kind() -> String {
    "request_context_profile".to_owned()
}
fn default_request_context_render_plan_kind() -> String {
    "request_context_render_plan".to_owned()
}
fn default_completion_behavior_kind() -> String {
    "workflow_completion_behavior".to_owned()
}
fn default_recovery_policy_kind() -> String {
    "workflow_recovery_policy".to_owned()
}
fn default_runtime_failure_policy_kind() -> String {
    "runtime_failure_policy".to_owned()
}
fn default_scheduler_policy_kind() -> String {
    "workflow_plane_scheduler_policy".to_owned()
}
fn default_operator_control_capability_kind() -> String {
    "operator_control_capability".to_owned()
}
fn default_workspace_schema_epoch_kind() -> String {
    "workspace_schema_epoch".to_owned()
}

fn default_json_extension() -> String {
    ".json".to_owned()
}
fn default_queued() -> String {
    "queued".to_owned()
}
fn default_active() -> String {
    "active".to_owned()
}
fn default_done() -> String {
    "done".to_owned()
}
fn default_blocked() -> String {
    "blocked".to_owned()
}
fn default_created_at_field() -> String {
    "created_at".to_owned()
}
fn default_plane_scope() -> String {
    "plane".to_owned()
}
fn default_fail_policy() -> String {
    "fail".to_owned()
}
fn default_block_source_policy() -> String {
    "block_source".to_owned()
}
fn default_created_at_asc() -> String {
    "created_at_asc".to_owned()
}
fn default_defer_unrelated() -> String {
    "defer_unrelated".to_owned()
}
fn default_idle() -> String {
    "idle".to_owned()
}
const fn default_one() -> u64 {
    1
}
fn default_single_writer_serialized() -> String {
    "single_writer_serialized".to_owned()
}
fn default_after_result_application() -> String {
    "after_result_application".to_owned()
}
fn default_reject_compile() -> String {
    "reject_compile".to_owned()
}
fn default_backlog_drained() -> String {
    "backlog_drained".to_owned()
}
const fn default_true() -> bool {
    true
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

fn validate_optional_id(
    field_name: &'static str,
    value: &Option<String>,
) -> Result<(), RuntimeJsonError> {
    if let Some(value) = value {
        validate_safe_identifier(value, field_name)?;
    }
    Ok(())
}

fn validate_optional_non_blank(
    field_name: &'static str,
    value: &Option<String>,
) -> Result<(), RuntimeJsonError> {
    if let Some(value) = value {
        require_non_blank(field_name, value)?;
    }
    Ok(())
}

fn validate_id_list(
    field_name: &'static str,
    values: &[String],
    require_non_empty: bool,
    check_safe_id: bool,
) -> Result<(), RuntimeJsonError> {
    if require_non_empty && values.is_empty() {
        return invalid_field(field_name, "must not be empty");
    }
    let mut seen = std::collections::HashSet::new();
    for value in values {
        if check_safe_id {
            validate_safe_identifier(value, field_name)?;
        } else {
            require_non_blank(field_name, value)?;
        }
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

fn validate_non_empty_entries(
    field_name: &'static str,
    values: &[String],
) -> Result<(), RuntimeJsonError> {
    for value in values {
        require_non_blank(field_name, value)?;
    }
    Ok(())
}

fn validate_filename(field_name: &'static str, value: &str) -> Result<(), RuntimeJsonError> {
    require_non_blank(field_name, value)?;
    if value.contains('/') || value.contains('\\') || value == "." || value == ".." {
        return invalid_field(field_name, "must be a filename, not a path");
    }
    Ok(())
}

fn validate_file_extension(field_name: &'static str, value: &str) -> Result<(), RuntimeJsonError> {
    if value.starts_with('.') && value.len() > 1 && !value.contains('/') && !value.contains('\\') {
        Ok(())
    } else {
        invalid_field(field_name, "must be a file extension such as `.md`")
    }
}

fn validate_runtime_relative_path(
    field_name: &'static str,
    value: &str,
) -> Result<(), RuntimeJsonError> {
    require_non_blank(field_name, value)?;
    if value.starts_with('/') || value.split('/').any(|part| part == "..") {
        return invalid_field(field_name, "must be a runtime-relative path");
    }
    Ok(())
}

fn validate_optional_runtime_path(
    field_name: &'static str,
    value: &Option<String>,
) -> Result<(), RuntimeJsonError> {
    if let Some(value) = value {
        validate_runtime_relative_path(field_name, value)?;
    }
    Ok(())
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
