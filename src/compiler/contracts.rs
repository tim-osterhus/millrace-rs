//! Serde-backed contracts for compiler inputs and frozen compiled plans.

use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::Value;

use crate::contracts::{
    CompileDiagnostics, ContractError, LearningRequestAction, LearningStageName, LoopEdgeKind,
    Plane, ResultClass, StageName, Timestamp, legal_terminal_results, stage_name_for_value,
    stage_plane, terminal_result_for_plane, validate_safe_identifier,
};

const SCHEMA_VERSION: &str = "1.0";

macro_rules! compiler_string_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident => $value:literal,
            )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $(
                $(#[$variant_meta])*
                $variant,
            )+
        }

        impl $name {
            /// Every canonical value for this enum.
            pub const ALL: &'static [Self] = &[
                $(Self::$variant,)+
            ];

            /// Returns the canonical serialized value.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value,)+
                }
            }

            /// Parses a canonical serialized value.
            pub fn from_value(value: &str) -> Result<Self, CompilerContractError> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(CompilerContractError::UnknownValue {
                        field_name: stringify!($name),
                        value: value.to_owned(),
                    }),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::from_value(&value).map_err(serde::de::Error::custom)
            }
        }
    };
}

compiler_string_enum! {
    /// Entry keys accepted by graph loop definitions.
    pub enum GraphLoopEntryKey {
        Task => "task",
        Spec => "spec",
        Incident => "incident",
        ClosureTarget => "closure_target",
        LearningRequest => "learning_request",
    }
}

compiler_string_enum! {
    /// Terminal classes declared by graph loop terminal states.
    pub enum GraphLoopTerminalClass {
        Success => "success",
        FollowupNeeded => "followup_needed",
        Blocked => "blocked",
        EscalatePlanning => "escalate_planning",
    }
}

compiler_string_enum! {
    /// Runtime counters used by compiled graph threshold policies.
    pub enum GraphLoopCounterName {
        FixCycleCount => "fix_cycle_count",
        TroubleshootAttemptCount => "troubleshoot_attempt_count",
        MechanicAttemptCount => "mechanic_attempt_count",
    }
}

compiler_string_enum! {
    /// Stage-kind idempotence policies from the registry.
    pub enum StageIdempotencePolicy {
        Idempotent => "idempotent",
        RetrySafeWithKey => "retry_safe_with_key",
        SingleAttemptOnly => "single_attempt_only",
    }
}

compiler_string_enum! {
    /// Recovery role annotations from the stage-kind registry.
    pub enum RecoveryRole {
        LocalRepair => "local_repair",
        Escalation => "escalation",
    }
}

compiler_string_enum! {
    /// Currentness states for persisted compiled plans.
    pub enum CompiledPlanCurrentnessState {
        Current => "current",
        Stale => "stale",
        Missing => "missing",
        Unknown => "unknown",
    }
}

/// Typed failures produced while decoding or validating compiler contracts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompilerContractError {
    /// A lower-level shared contract validation failed.
    Contract(ContractError),
    /// JSON syntax, required-field, type, or enum decoding failed.
    Json {
        /// Artifact type being decoded.
        artifact: &'static str,
        /// Serde error message.
        message: String,
    },
    /// A string value did not match a compiler-local enum.
    UnknownValue {
        /// Field or enum name.
        field_name: &'static str,
        /// Invalid value.
        value: String,
    },
    /// A literal schema or kind field had the wrong value.
    InvalidLiteral {
        /// Field name.
        field_name: &'static str,
        /// Expected literal value.
        expected: &'static str,
        /// Actual value.
        actual: String,
    },
    /// A scalar field failed validation.
    InvalidField {
        /// Field name.
        field_name: &'static str,
        /// Human-readable failure reason.
        message: String,
    },
    /// A document-level invariant failed.
    InvalidDocument {
        /// Human-readable failure reason.
        message: String,
    },
}

impl fmt::Display for CompilerContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => write!(f, "{error}"),
            Self::Json { artifact, message } => {
                write!(f, "failed to decode {artifact}: {message}")
            }
            Self::UnknownValue { field_name, value } => {
                write!(f, "{field_name} has unknown value: {value}")
            }
            Self::InvalidLiteral {
                field_name,
                expected,
                actual,
            } => write!(
                f,
                "{field_name} must be literal `{expected}`, got `{actual}`"
            ),
            Self::InvalidField {
                field_name,
                message,
            } => write!(f, "{field_name} is invalid: {message}"),
            Self::InvalidDocument { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for CompilerContractError {}

impl From<ContractError> for CompilerContractError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}

/// Common helpers for compiler JSON artifact contracts.
pub trait CompilerContract: Sized + Serialize + DeserializeOwned {
    /// Human-readable artifact name used in decode errors.
    const ARTIFACT: &'static str;

    /// Validates and normalizes this decoded artifact.
    fn validate_contract(&mut self) -> Result<(), CompilerContractError>;

    /// Deserializes and validates a JSON value.
    fn from_json_value(value: Value) -> Result<Self, CompilerContractError> {
        let mut decoded: Self = decode_json(Self::ARTIFACT, value)?;
        decoded.validate_contract()?;
        Ok(decoded)
    }

    /// Deserializes and validates a JSON string.
    fn from_json_str(raw: &str) -> Result<Self, CompilerContractError> {
        let value = serde_json::from_str(raw).map_err(|error| CompilerContractError::Json {
            artifact: Self::ARTIFACT,
            message: error.to_string(),
        })?;
        Self::from_json_value(value)
    }
}

/// Learning trigger declared by a mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningTriggerRuleDefinition {
    pub rule_id: String,
    pub source_plane: Plane,
    pub source_stage: StageName,
    #[serde(default)]
    pub on_terminal_results: Vec<String>,
    pub target_stage: LearningStageName,
    #[serde(default = "default_learning_request_action")]
    pub requested_action: LearningRequestAction,
}

impl LearningTriggerRuleDefinition {
    /// Validates trigger stage alignment and terminal outcomes.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        validate_safe_identifier(&self.rule_id, "rule_id")?;
        if stage_plane(self.source_stage) != self.source_plane {
            return Err(CompilerContractError::InvalidDocument {
                message: "source_stage must belong to source_plane".to_owned(),
            });
        }
        if self.source_plane == Plane::Learning {
            return Err(CompilerContractError::InvalidDocument {
                message: "learning triggers must originate outside the learning plane".to_owned(),
            });
        }
        normalize_status_values(&mut self.on_terminal_results, "on_terminal_results", false)?;
        if self.on_terminal_results.is_empty() {
            return Err(CompilerContractError::InvalidField {
                field_name: "on_terminal_results",
                message: "must not be empty".to_owned(),
            });
        }

        let legal = legal_terminal_results(self.source_stage);
        for outcome in &self.on_terminal_results {
            let result = terminal_result_for_plane(self.source_plane, outcome)?;
            if !legal.contains(&result) {
                return Err(CompilerContractError::InvalidDocument {
                    message: format!(
                        "on_terminal_results contains value illegal for source_stage: {outcome}"
                    ),
                });
            }
        }
        Ok(())
    }
}

/// Concurrency policy declared by a mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PlaneConcurrencyPolicyDefinition {
    #[serde(default)]
    pub mutually_exclusive_planes: Vec<Vec<Plane>>,
    #[serde(default)]
    pub may_run_concurrently: Vec<Vec<Plane>>,
}

impl PlaneConcurrencyPolicyDefinition {
    /// Validates concurrency policy tuples.
    pub fn validate(&self) -> Result<(), CompilerContractError> {
        validate_plane_groups("mutually_exclusive_planes", &self.mutually_exclusive_planes)?;
        validate_plane_groups("may_run_concurrently", &self.may_run_concurrently)
    }
}

/// Mode selection and compile binding contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModeDefinition {
    pub schema_version: String,
    pub kind: String,
    pub mode_id: String,
    pub loop_ids_by_plane: HashMap<Plane, String>,
    #[serde(default)]
    pub stage_entrypoint_overrides: HashMap<StageName, String>,
    #[serde(default)]
    pub stage_skill_additions: HashMap<StageName, Vec<String>>,
    #[serde(default)]
    pub stage_model_bindings: HashMap<StageName, String>,
    #[serde(default)]
    pub stage_runner_bindings: HashMap<StageName, String>,
    #[serde(default)]
    pub stage_thinking_bindings: HashMap<StageName, Option<String>>,
    pub concurrency_policy: Option<PlaneConcurrencyPolicyDefinition>,
    #[serde(default)]
    pub learning_trigger_rules: Vec<LearningTriggerRuleDefinition>,
}

impl<'de> Deserialize<'de> for ModeDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        ModeDefinitionRaw::deserialize(deserializer)?
            .try_into_mode_definition()
            .map_err(serde::de::Error::custom)
    }
}

impl CompilerContract for ModeDefinition {
    const ARTIFACT: &'static str = "mode";

    fn validate_contract(&mut self) -> Result<(), CompilerContractError> {
        self.validate()
    }
}

impl ModeDefinition {
    /// Validates mode loop bindings and learning trigger invariants.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
        validate_literal("kind", &self.kind, "mode")?;
        require_non_blank("mode_id", &self.mode_id)?;
        if !self.loop_ids_by_plane.contains_key(&Plane::Execution) {
            return Err(CompilerContractError::InvalidDocument {
                message: "loop_ids_by_plane must include execution".to_owned(),
            });
        }
        if !self.loop_ids_by_plane.contains_key(&Plane::Planning) {
            return Err(CompilerContractError::InvalidDocument {
                message: "loop_ids_by_plane must include planning".to_owned(),
            });
        }
        for (plane, loop_id) in &self.loop_ids_by_plane {
            require_non_blank("loop_id", loop_id)?;
            let expected_prefix = format!("{}.", plane.as_str());
            if !loop_id.starts_with(&expected_prefix) {
                return Err(CompilerContractError::InvalidDocument {
                    message: format!(
                        "loop id for plane {} must start with {expected_prefix:?}",
                        plane.as_str()
                    ),
                });
            }
        }
        normalize_stage_string_map(&mut self.stage_entrypoint_overrides)?;
        normalize_stage_vec_map(&mut self.stage_skill_additions)?;
        normalize_stage_string_map(&mut self.stage_model_bindings)?;
        normalize_stage_string_map(&mut self.stage_runner_bindings)?;
        normalize_stage_optional_string_map(&mut self.stage_thinking_bindings)?;
        if let Some(policy) = &self.concurrency_policy {
            policy.validate()?;
        }
        for rule in &mut self.learning_trigger_rules {
            rule.validate()?;
        }
        if !self.learning_trigger_rules.is_empty()
            && !self.loop_ids_by_plane.contains_key(&Plane::Learning)
        {
            return Err(CompilerContractError::InvalidDocument {
                message: "learning_trigger_rules require a learning loop binding".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModeDefinitionRaw {
    #[serde(default = "default_schema_version")]
    schema_version: String,
    #[serde(default = "default_mode_kind")]
    kind: String,
    mode_id: String,
    #[serde(default)]
    loop_ids_by_plane: HashMap<Plane, String>,
    execution_loop_id: Option<String>,
    planning_loop_id: Option<String>,
    #[serde(default)]
    stage_entrypoint_overrides: HashMap<StageName, String>,
    #[serde(default)]
    stage_skill_additions: HashMap<StageName, Vec<String>>,
    #[serde(default)]
    stage_model_bindings: HashMap<StageName, String>,
    #[serde(default)]
    stage_runner_bindings: HashMap<StageName, String>,
    #[serde(default)]
    stage_thinking_bindings: HashMap<StageName, Option<String>>,
    concurrency_policy: Option<PlaneConcurrencyPolicyDefinition>,
    #[serde(default)]
    learning_trigger_rules: Vec<LearningTriggerRuleDefinition>,
}

impl ModeDefinitionRaw {
    fn try_into_mode_definition(self) -> Result<ModeDefinition, CompilerContractError> {
        let mut loop_ids_by_plane = self.loop_ids_by_plane;
        if let Some(loop_id) = self.execution_loop_id {
            loop_ids_by_plane.insert(Plane::Execution, loop_id);
        }
        if let Some(loop_id) = self.planning_loop_id {
            loop_ids_by_plane.insert(Plane::Planning, loop_id);
        }
        let mut mode = ModeDefinition {
            schema_version: self.schema_version,
            kind: self.kind,
            mode_id: self.mode_id,
            loop_ids_by_plane,
            stage_entrypoint_overrides: self.stage_entrypoint_overrides,
            stage_skill_additions: self.stage_skill_additions,
            stage_model_bindings: self.stage_model_bindings,
            stage_runner_bindings: self.stage_runner_bindings,
            stage_thinking_bindings: self.stage_thinking_bindings,
            concurrency_policy: self.concurrency_policy,
            learning_trigger_rules: self.learning_trigger_rules,
        };
        mode.validate()?;
        Ok(mode)
    }
}

/// Graph node declared by an authoritative graph loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphLoopNodeDefinition {
    pub node_id: String,
    pub stage_kind_id: String,
    pub entrypoint_path: Option<String>,
    #[serde(default)]
    pub attached_skill_additions: Vec<String>,
    pub runner_name: Option<String>,
    pub model_name: Option<String>,
    #[serde(default)]
    pub thinking_level: Option<String>,
    pub timeout_seconds: Option<u64>,
}

impl GraphLoopNodeDefinition {
    /// Validates and normalizes graph node fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        normalize_canonical_id(&mut self.node_id, "node_id")?;
        normalize_canonical_id(&mut self.stage_kind_id, "stage_kind_id")?;
        validate_optional_markdown_asset_path(
            "entrypoint_path",
            &mut self.entrypoint_path,
            "entrypoints/",
        )?;
        normalize_markdown_asset_paths(
            "attached_skill_additions",
            &mut self.attached_skill_additions,
            "skills/",
        )?;
        require_optional_non_blank("runner_name", &self.runner_name)?;
        require_optional_non_blank("model_name", &self.model_name)?;
        require_optional_non_blank("thinking_level", &self.thinking_level)?;
        if self.timeout_seconds == Some(0) {
            return Err(CompilerContractError::InvalidField {
                field_name: "timeout_seconds",
                message: "must be >= 1 when set".to_owned(),
            });
        }
        Ok(())
    }
}

/// Named graph entry point for a work item category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphLoopEntryDefinition {
    pub entry_key: GraphLoopEntryKey,
    pub node_id: String,
}

impl GraphLoopEntryDefinition {
    /// Validates and normalizes graph entry fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        normalize_canonical_id(&mut self.node_id, "node_id")
    }
}

/// Graph terminal state definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphLoopTerminalStateDefinition {
    pub terminal_state_id: String,
    pub terminal_class: GraphLoopTerminalClass,
    pub writes_status: String,
    #[serde(default)]
    pub emits_artifacts: Vec<String>,
    #[serde(default = "default_true")]
    pub ends_plane_run: bool,
}

impl GraphLoopTerminalStateDefinition {
    /// Validates and normalizes terminal state fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        normalize_canonical_id(&mut self.terminal_state_id, "terminal_state_id")?;
        normalize_status(&mut self.writes_status, "writes_status")?;
        normalize_canonical_ids(&mut self.emits_artifacts, "emits_artifacts")
    }
}

/// Directed graph edge definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphLoopEdgeDefinition {
    pub edge_id: String,
    pub from_node_id: String,
    pub to_node_id: Option<String>,
    pub terminal_state_id: Option<String>,
    #[serde(default)]
    pub on_outcomes: Vec<String>,
    #[serde(default = "default_loop_edge_kind")]
    pub kind: LoopEdgeKind,
    #[serde(default = "default_priority")]
    pub priority: i64,
    pub description: Option<String>,
    pub max_attempts: Option<u64>,
}

impl GraphLoopEdgeDefinition {
    /// Validates and normalizes graph edge fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        normalize_canonical_id(&mut self.edge_id, "edge_id")?;
        normalize_canonical_id(&mut self.from_node_id, "from_node_id")?;
        normalize_optional_canonical_id(&mut self.to_node_id, "to_node_id")?;
        normalize_optional_canonical_id(&mut self.terminal_state_id, "terminal_state_id")?;
        normalize_status_values(&mut self.on_outcomes, "on_outcomes", true)?;
        validate_optional_nonempty_text("description", &mut self.description)?;
        if self.max_attempts == Some(0) {
            return Err(CompilerContractError::InvalidField {
                field_name: "max_attempts",
                message: "must be >= 1 when set".to_owned(),
            });
        }
        validate_edge_target_shape(
            "edge",
            self.kind,
            self.to_node_id.is_some(),
            self.terminal_state_id.is_some(),
            self.max_attempts,
        )
    }
}

/// Completion behavior declared by a graph loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphLoopCompletionBehaviorDefinition {
    pub trigger: String,
    pub readiness_rule: String,
    pub target_node_id: String,
    pub request_kind: String,
    pub target_selector: String,
    pub rubric_policy: String,
    pub blocked_work_policy: String,
    #[serde(default = "default_true")]
    pub skip_if_already_closed: bool,
    pub on_pass_terminal_state_id: String,
    pub on_gap_terminal_state_id: String,
    #[serde(default)]
    pub create_incident_on_gap: bool,
}

impl GraphLoopCompletionBehaviorDefinition {
    /// Validates and normalizes completion behavior fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        validate_literal("trigger", &self.trigger, "backlog_drained")?;
        validate_literal(
            "readiness_rule",
            &self.readiness_rule,
            "no_open_lineage_work",
        )?;
        validate_literal("request_kind", &self.request_kind, "closure_target")?;
        validate_literal(
            "target_selector",
            &self.target_selector,
            "active_closure_target",
        )?;
        validate_literal("rubric_policy", &self.rubric_policy, "reuse_or_create")?;
        validate_literal("blocked_work_policy", &self.blocked_work_policy, "suppress")?;
        normalize_canonical_id(&mut self.target_node_id, "target_node_id")?;
        normalize_canonical_id(
            &mut self.on_pass_terminal_state_id,
            "on_pass_terminal_state_id",
        )?;
        normalize_canonical_id(
            &mut self.on_gap_terminal_state_id,
            "on_gap_terminal_state_id",
        )?;
        if self.on_pass_terminal_state_id == self.on_gap_terminal_state_id {
            return Err(CompilerContractError::InvalidDocument {
                message: "completion behavior pass/gap terminal states must differ".to_owned(),
            });
        }
        Ok(())
    }
}

/// Resume policy declared by a graph loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphLoopResumePolicyDefinition {
    pub policy_id: String,
    pub source_node_id: String,
    pub on_outcome: String,
    pub default_target_node_id: String,
    #[serde(default)]
    pub metadata_stage_keys: Vec<String>,
    #[serde(default)]
    pub disallowed_target_node_ids: Vec<String>,
}

impl GraphLoopResumePolicyDefinition {
    /// Validates and normalizes resume policy fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        normalize_canonical_id(&mut self.policy_id, "policy_id")?;
        normalize_canonical_id(&mut self.source_node_id, "source_node_id")?;
        normalize_status(&mut self.on_outcome, "on_outcome")?;
        normalize_canonical_id(&mut self.default_target_node_id, "default_target_node_id")?;
        normalize_canonical_ids(&mut self.metadata_stage_keys, "metadata_stage_keys")?;
        normalize_canonical_ids(
            &mut self.disallowed_target_node_ids,
            "disallowed_target_node_ids",
        )
    }
}

/// Threshold policy declared by a graph loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphLoopThresholdPolicyDefinition {
    pub policy_id: String,
    #[serde(default)]
    pub source_node_ids: Vec<String>,
    pub on_outcome: String,
    pub counter_name: GraphLoopCounterName,
    pub threshold: u64,
    pub exhausted_target_node_id: Option<String>,
    pub exhausted_terminal_state_id: Option<String>,
}

impl GraphLoopThresholdPolicyDefinition {
    /// Validates and normalizes threshold policy fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        normalize_canonical_id(&mut self.policy_id, "policy_id")?;
        normalize_canonical_ids(&mut self.source_node_ids, "source_node_ids")?;
        if self.source_node_ids.is_empty() {
            return Err(CompilerContractError::InvalidField {
                field_name: "source_node_ids",
                message: "must not be empty".to_owned(),
            });
        }
        normalize_status(&mut self.on_outcome, "on_outcome")?;
        if self.threshold == 0 {
            return Err(CompilerContractError::InvalidField {
                field_name: "threshold",
                message: "must be >= 1".to_owned(),
            });
        }
        normalize_optional_canonical_id(
            &mut self.exhausted_target_node_id,
            "exhausted_target_node_id",
        )?;
        normalize_optional_canonical_id(
            &mut self.exhausted_terminal_state_id,
            "exhausted_terminal_state_id",
        )?;
        let target_count = self.exhausted_target_node_id.is_some() as u8
            + self.exhausted_terminal_state_id.is_some() as u8;
        if target_count != 1 {
            return Err(CompilerContractError::InvalidDocument {
                message:
                    "threshold policies must define exactly one exhausted target node or terminal state"
                        .to_owned(),
            });
        }
        Ok(())
    }
}

/// Optional dynamic graph policies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct GraphLoopDynamicPoliciesDefinition {
    #[serde(default)]
    pub resume_policies: Vec<GraphLoopResumePolicyDefinition>,
    #[serde(default)]
    pub threshold_policies: Vec<GraphLoopThresholdPolicyDefinition>,
}

impl GraphLoopDynamicPoliciesDefinition {
    /// Validates dynamic policies and unique ids.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        for policy in &mut self.resume_policies {
            policy.validate()?;
        }
        for policy in &mut self.threshold_policies {
            policy.validate()?;
        }
        let mut ids = HashSet::new();
        for policy_id in self
            .resume_policies
            .iter()
            .map(|policy| policy.policy_id.as_str())
            .chain(
                self.threshold_policies
                    .iter()
                    .map(|policy| policy.policy_id.as_str()),
            )
        {
            if !ids.insert(policy_id) {
                return Err(CompilerContractError::InvalidDocument {
                    message: "graph loops may not contain duplicate dynamic policy ids".to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// Authoritative graph loop asset contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphLoopDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_graph_loop_kind")]
    pub kind: String,
    pub loop_id: String,
    pub plane: Plane,
    #[serde(default)]
    pub nodes: Vec<GraphLoopNodeDefinition>,
    #[serde(default)]
    pub edges: Vec<GraphLoopEdgeDefinition>,
    #[serde(default)]
    pub entry_nodes: Vec<GraphLoopEntryDefinition>,
    #[serde(default)]
    pub terminal_states: Vec<GraphLoopTerminalStateDefinition>,
    pub dynamic_policies: Option<GraphLoopDynamicPoliciesDefinition>,
    pub completion_behavior: Option<GraphLoopCompletionBehaviorDefinition>,
}

impl CompilerContract for GraphLoopDefinition {
    const ARTIFACT: &'static str = "graph_loop";

    fn validate_contract(&mut self) -> Result<(), CompilerContractError> {
        self.validate()
    }
}

impl GraphLoopDefinition {
    /// Validates graph integrity and reference shape.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
        validate_literal("kind", &self.kind, "graph_loop")?;
        normalize_canonical_id(&mut self.loop_id, "loop_id")?;
        require_non_empty("nodes", self.nodes.len())?;
        require_non_empty("edges", self.edges.len())?;
        require_non_empty("entry_nodes", self.entry_nodes.len())?;
        require_non_empty("terminal_states", self.terminal_states.len())?;

        for node in &mut self.nodes {
            node.validate()?;
        }
        for entry in &mut self.entry_nodes {
            entry.validate()?;
        }
        for edge in &mut self.edges {
            edge.validate()?;
        }
        for terminal_state in &mut self.terminal_states {
            terminal_state.validate()?;
        }
        if let Some(policies) = &mut self.dynamic_policies {
            policies.validate()?;
        }
        if let Some(completion) = &mut self.completion_behavior {
            completion.validate()?;
        }

        let node_ids = unique_ids(
            "graph loops may not contain duplicate node ids",
            self.nodes.iter().map(|node| node.node_id.as_str()),
        )?;
        let terminal_state_ids = unique_ids(
            "graph loops may not contain duplicate terminal state ids",
            self.terminal_states
                .iter()
                .map(|state| state.terminal_state_id.as_str()),
        )?;
        unique_ids(
            "graph loops may not contain duplicate edge ids",
            self.edges.iter().map(|edge| edge.edge_id.as_str()),
        )?;
        unique_ids(
            "graph loops may not contain duplicate entry keys",
            self.entry_nodes
                .iter()
                .map(|entry| entry.entry_key.as_str()),
        )?;

        for entry in &self.entry_nodes {
            if !node_ids.contains(entry.node_id.as_str()) {
                return Err(CompilerContractError::InvalidDocument {
                    message: format!(
                        "entry key {} references unknown node_id {}",
                        entry.entry_key.as_str(),
                        entry.node_id
                    ),
                });
            }
        }
        for edge in &self.edges {
            if !node_ids.contains(edge.from_node_id.as_str()) {
                return Err(CompilerContractError::InvalidDocument {
                    message: format!(
                        "edge {} references unknown from_node_id {}",
                        edge.edge_id, edge.from_node_id
                    ),
                });
            }
            if let Some(to_node_id) = &edge.to_node_id {
                if !node_ids.contains(to_node_id.as_str()) {
                    return Err(CompilerContractError::InvalidDocument {
                        message: format!(
                            "edge {} references unknown to_node_id {}",
                            edge.edge_id, to_node_id
                        ),
                    });
                }
            }
            if let Some(terminal_state_id) = &edge.terminal_state_id {
                if !terminal_state_ids.contains(terminal_state_id.as_str()) {
                    return Err(CompilerContractError::InvalidDocument {
                        message: format!(
                            "edge {} references unknown terminal_state_id {}",
                            edge.edge_id, terminal_state_id
                        ),
                    });
                }
            }
        }
        if let Some(policies) = &self.dynamic_policies {
            for policy in &policies.resume_policies {
                require_known_node(
                    "resume policy",
                    &policy.policy_id,
                    "source_node_id",
                    &policy.source_node_id,
                    &node_ids,
                )?;
                require_known_node(
                    "resume policy",
                    &policy.policy_id,
                    "default_target_node_id",
                    &policy.default_target_node_id,
                    &node_ids,
                )?;
                for node_id in &policy.disallowed_target_node_ids {
                    require_known_node(
                        "resume policy",
                        &policy.policy_id,
                        "disallowed_target_node_ids",
                        node_id,
                        &node_ids,
                    )?;
                }
            }
            for policy in &policies.threshold_policies {
                for source_node_id in &policy.source_node_ids {
                    require_known_node(
                        "threshold policy",
                        &policy.policy_id,
                        "source_node_ids",
                        source_node_id,
                        &node_ids,
                    )?;
                }
                if let Some(node_id) = &policy.exhausted_target_node_id {
                    require_known_node(
                        "threshold policy",
                        &policy.policy_id,
                        "exhausted_target_node_id",
                        node_id,
                        &node_ids,
                    )?;
                }
                if let Some(terminal_state_id) = &policy.exhausted_terminal_state_id {
                    require_known_terminal_state(
                        "threshold policy",
                        &policy.policy_id,
                        "exhausted_terminal_state_id",
                        terminal_state_id,
                        &terminal_state_ids,
                    )?;
                }
            }
        }
        if let Some(completion) = &self.completion_behavior {
            require_known_node(
                "completion behavior",
                "completion_behavior",
                "target_node_id",
                &completion.target_node_id,
                &node_ids,
            )?;
            require_known_terminal_state(
                "completion behavior",
                "completion_behavior",
                "on_pass_terminal_state_id",
                &completion.on_pass_terminal_state_id,
                &terminal_state_ids,
            )?;
            require_known_terminal_state(
                "completion behavior",
                "completion_behavior",
                "on_gap_terminal_state_id",
                &completion.on_gap_terminal_state_id,
                &terminal_state_ids,
            )?;
        }
        Ok(())
    }
}

/// Validates graph node references against a loaded stage-kind registry.
pub fn validate_graph_stage_kind_references(
    graph: &GraphLoopDefinition,
    stage_kinds: &HashMap<String, RegisteredStageKindDefinition>,
) -> Result<(), CompilerContractError> {
    for node in &graph.nodes {
        let Some(stage_kind) = stage_kinds.get(&node.stage_kind_id) else {
            return Err(CompilerContractError::InvalidDocument {
                message: format!(
                    "graph node {} references unknown stage_kind_id {}",
                    node.node_id, node.stage_kind_id
                ),
            });
        };
        if stage_kind.plane != graph.plane {
            return Err(CompilerContractError::InvalidDocument {
                message: format!(
                    "graph node {} references stage_kind_id {} from plane {}",
                    node.node_id,
                    node.stage_kind_id,
                    stage_kind.plane.as_str()
                ),
            });
        }
    }
    Ok(())
}

/// Stage-kind registry entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredStageKindDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_registered_stage_kind")]
    pub kind: String,
    pub stage_kind_id: String,
    pub plane: Plane,
    pub display_name: String,
    pub default_entrypoint_path: String,
    #[serde(default)]
    pub required_skill_paths: Vec<String>,
    #[serde(default)]
    pub suggested_skill_paths: Vec<String>,
    pub running_status_marker: String,
    #[serde(default)]
    pub legal_outcomes: Vec<String>,
    #[serde(default)]
    pub success_outcomes: Vec<String>,
    #[serde(default)]
    pub failure_outcomes: Vec<String>,
    #[serde(default)]
    pub allowed_result_classes_by_outcome: HashMap<String, Vec<ResultClass>>,
    #[serde(default)]
    pub allowed_input_artifacts: Vec<String>,
    #[serde(default)]
    pub declared_output_artifacts: Vec<String>,
    #[serde(default = "default_stage_idempotence_policy")]
    pub idempotence_policy: StageIdempotencePolicy,
    #[serde(default)]
    pub allowed_overrides: Vec<String>,
    #[serde(default)]
    pub can_start_tasks: bool,
    #[serde(default)]
    pub can_start_specs: bool,
    #[serde(default)]
    pub can_start_incidents: bool,
    #[serde(default)]
    pub can_start_learning_requests: bool,
    pub recovery_role: Option<RecoveryRole>,
    #[serde(default)]
    pub closure_role: bool,
}

impl CompilerContract for RegisteredStageKindDefinition {
    const ARTIFACT: &'static str = "registered_stage_kind";

    fn validate_contract(&mut self) -> Result<(), CompilerContractError> {
        self.validate()
    }
}

impl RegisteredStageKindDefinition {
    /// Validates stage-kind registry semantics.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
        validate_literal("kind", &self.kind, "registered_stage_kind")?;
        normalize_canonical_id(&mut self.stage_kind_id, "stage_kind_id")?;
        if let Ok(stage) = stage_name_for_value(&self.stage_kind_id) {
            if stage_plane(stage) != self.plane {
                return Err(CompilerContractError::InvalidDocument {
                    message: "stage_kind_id stage must belong to stage-kind plane".to_owned(),
                });
            }
        }
        normalize_nonempty_text(&mut self.display_name, "display_name")?;
        validate_markdown_asset_path(
            "default_entrypoint_path",
            &mut self.default_entrypoint_path,
            "entrypoints/",
        )?;
        normalize_markdown_asset_paths(
            "required_skill_paths",
            &mut self.required_skill_paths,
            "skills/",
        )?;
        require_non_empty("required_skill_paths", self.required_skill_paths.len())?;
        normalize_markdown_asset_paths(
            "suggested_skill_paths",
            &mut self.suggested_skill_paths,
            "skills/",
        )?;
        normalize_status(&mut self.running_status_marker, "running_status_marker")?;
        normalize_status_values(&mut self.legal_outcomes, "legal_outcomes", true)?;
        normalize_status_values(&mut self.success_outcomes, "success_outcomes", false)?;
        normalize_status_values(&mut self.failure_outcomes, "failure_outcomes", false)?;
        normalize_canonical_ids(&mut self.allowed_input_artifacts, "allowed_input_artifacts")?;
        normalize_canonical_ids(
            &mut self.declared_output_artifacts,
            "declared_output_artifacts",
        )?;
        normalize_override_names(&mut self.allowed_overrides, "allowed_overrides")?;
        normalize_allowed_result_classes(&mut self.allowed_result_classes_by_outcome)?;
        self.validate_outcome_sets()
    }

    fn validate_outcome_sets(&self) -> Result<(), CompilerContractError> {
        let legal: HashSet<&str> = self.legal_outcomes.iter().map(String::as_str).collect();
        for outcome in &self.success_outcomes {
            if !legal.contains(outcome.as_str()) {
                return Err(CompilerContractError::InvalidDocument {
                    message: "success_outcomes must be a subset of legal_outcomes".to_owned(),
                });
            }
        }
        for outcome in &self.failure_outcomes {
            if !legal.contains(outcome.as_str()) {
                return Err(CompilerContractError::InvalidDocument {
                    message: "failure_outcomes must be a subset of legal_outcomes".to_owned(),
                });
            }
        }
        let allowed_keys: HashSet<&str> = self
            .allowed_result_classes_by_outcome
            .keys()
            .map(String::as_str)
            .collect();
        if allowed_keys != legal {
            return Err(CompilerContractError::InvalidDocument {
                message:
                    "allowed_result_classes_by_outcome must define one entry for every legal outcome"
                        .to_owned(),
            });
        }
        for (outcome, result_classes) in &self.allowed_result_classes_by_outcome {
            if result_classes.is_empty() {
                return Err(CompilerContractError::InvalidDocument {
                    message:
                        "allowed_result_classes_by_outcome entries must declare at least one result class"
                            .to_owned(),
                });
            }
            if outcome == "BLOCKED" {
                for result_class in result_classes {
                    if !matches!(
                        result_class,
                        ResultClass::Blocked | ResultClass::RecoverableFailure
                    ) {
                        return Err(CompilerContractError::InvalidDocument {
                            message:
                                "BLOCKED outcome may only allow blocked or recoverable_failure result classes"
                                    .to_owned(),
                        });
                    }
                }
            } else if result_classes.len() != 1 {
                return Err(CompilerContractError::InvalidDocument {
                    message: "non-BLOCKED outcomes must map to exactly one allowed result class"
                        .to_owned(),
                });
            }
        }
        for outcome in &self.success_outcomes {
            if !self.allowed_result_classes_by_outcome[outcome].contains(&ResultClass::Success) {
                return Err(CompilerContractError::InvalidDocument {
                    message: "success outcomes must allow result class success".to_owned(),
                });
            }
        }
        for outcome in &self.failure_outcomes {
            if self.allowed_result_classes_by_outcome[outcome].contains(&ResultClass::Success) {
                return Err(CompilerContractError::InvalidDocument {
                    message: "failure outcomes may not allow result class success".to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// Compiled entry plan for a graph entry key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledGraphEntryPlan {
    pub entry_key: GraphLoopEntryKey,
    pub node_id: String,
    pub stage_kind_id: String,
    pub plane: Plane,
}

impl CompiledGraphEntryPlan {
    /// Validates and normalizes compiled entry fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        normalize_canonical_id(&mut self.node_id, "node_id")?;
        normalize_canonical_id(&mut self.stage_kind_id, "stage_kind_id")
    }
}

/// Compiled closure-target entry for planning completion behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledGraphCompletionEntryPlan {
    #[serde(default = "default_closure_target_entry_key")]
    pub entry_key: GraphLoopEntryKey,
    pub node_id: String,
    pub stage_kind_id: String,
    pub plane: Plane,
    pub trigger: String,
    pub readiness_rule: String,
    pub request_kind: String,
    pub target_selector: String,
    pub rubric_policy: String,
    pub blocked_work_policy: String,
    #[serde(default = "default_true")]
    pub skip_if_already_closed: bool,
    pub on_pass_terminal_state_id: String,
    pub on_gap_terminal_state_id: String,
    #[serde(default)]
    pub create_incident_on_gap: bool,
}

impl CompiledGraphCompletionEntryPlan {
    /// Validates and normalizes compiled completion entry fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        if self.entry_key != GraphLoopEntryKey::ClosureTarget {
            return Err(CompilerContractError::InvalidLiteral {
                field_name: "entry_key",
                expected: "closure_target",
                actual: self.entry_key.as_str().to_owned(),
            });
        }
        normalize_canonical_id(&mut self.node_id, "node_id")?;
        normalize_canonical_id(&mut self.stage_kind_id, "stage_kind_id")?;
        validate_literal("trigger", &self.trigger, "backlog_drained")?;
        validate_literal(
            "readiness_rule",
            &self.readiness_rule,
            "no_open_lineage_work",
        )?;
        validate_literal("request_kind", &self.request_kind, "closure_target")?;
        validate_literal(
            "target_selector",
            &self.target_selector,
            "active_closure_target",
        )?;
        validate_literal("rubric_policy", &self.rubric_policy, "reuse_or_create")?;
        validate_literal("blocked_work_policy", &self.blocked_work_policy, "suppress")?;
        normalize_canonical_id(
            &mut self.on_pass_terminal_state_id,
            "on_pass_terminal_state_id",
        )?;
        normalize_canonical_id(
            &mut self.on_gap_terminal_state_id,
            "on_gap_terminal_state_id",
        )?;
        if self.on_pass_terminal_state_id == self.on_gap_terminal_state_id {
            return Err(CompilerContractError::InvalidDocument {
                message: "compiled completion pass/gap terminal states must differ".to_owned(),
            });
        }
        Ok(())
    }
}

/// Compiled transition for one concrete outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledGraphTransitionPlan {
    pub edge_id: String,
    pub source_node_id: String,
    pub outcome: String,
    pub target_node_id: Option<String>,
    pub terminal_state_id: Option<String>,
    #[serde(default = "default_loop_edge_kind")]
    pub kind: LoopEdgeKind,
    #[serde(default = "default_priority")]
    pub priority: i64,
    pub max_attempts: Option<u64>,
}

impl CompiledGraphTransitionPlan {
    /// Validates and normalizes compiled transition fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        normalize_canonical_id(&mut self.edge_id, "edge_id")?;
        normalize_canonical_id(&mut self.source_node_id, "source_node_id")?;
        normalize_status(&mut self.outcome, "outcome")?;
        normalize_optional_canonical_id(&mut self.target_node_id, "target_node_id")?;
        normalize_optional_canonical_id(&mut self.terminal_state_id, "terminal_state_id")?;
        if self.max_attempts == Some(0) {
            return Err(CompilerContractError::InvalidField {
                field_name: "max_attempts",
                message: "must be >= 1 when set".to_owned(),
            });
        }
        validate_edge_target_shape(
            "compiled graph transition",
            self.kind,
            self.target_node_id.is_some(),
            self.terminal_state_id.is_some(),
            self.max_attempts,
        )
    }
}

/// Compiled resume policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledGraphResumePolicyPlan {
    pub policy_id: String,
    pub source_node_id: String,
    pub on_outcome: String,
    pub default_target_node_id: String,
    #[serde(default)]
    pub metadata_stage_keys: Vec<String>,
    #[serde(default)]
    pub disallowed_target_node_ids: Vec<String>,
}

impl CompiledGraphResumePolicyPlan {
    /// Validates and normalizes compiled resume policy fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        normalize_canonical_id(&mut self.policy_id, "policy_id")?;
        normalize_canonical_id(&mut self.source_node_id, "source_node_id")?;
        normalize_status(&mut self.on_outcome, "on_outcome")?;
        normalize_canonical_id(&mut self.default_target_node_id, "default_target_node_id")?;
        normalize_canonical_ids(&mut self.metadata_stage_keys, "metadata_stage_keys")?;
        normalize_canonical_ids(
            &mut self.disallowed_target_node_ids,
            "disallowed_target_node_ids",
        )
    }
}

/// Compiled threshold policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledGraphThresholdPolicyPlan {
    pub policy_id: String,
    #[serde(default)]
    pub source_node_ids: Vec<String>,
    pub on_outcome: String,
    pub counter_name: GraphLoopCounterName,
    pub threshold: u64,
    pub exhausted_target_node_id: Option<String>,
    pub exhausted_terminal_state_id: Option<String>,
}

impl CompiledGraphThresholdPolicyPlan {
    /// Validates and normalizes compiled threshold policy fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        normalize_canonical_id(&mut self.policy_id, "policy_id")?;
        normalize_canonical_ids(&mut self.source_node_ids, "source_node_ids")?;
        require_non_empty("source_node_ids", self.source_node_ids.len())?;
        normalize_status(&mut self.on_outcome, "on_outcome")?;
        if self.threshold == 0 {
            return Err(CompilerContractError::InvalidField {
                field_name: "threshold",
                message: "must be >= 1".to_owned(),
            });
        }
        normalize_optional_canonical_id(
            &mut self.exhausted_target_node_id,
            "exhausted_target_node_id",
        )?;
        normalize_optional_canonical_id(
            &mut self.exhausted_terminal_state_id,
            "exhausted_terminal_state_id",
        )?;
        let target_count = self.exhausted_target_node_id.is_some() as u8
            + self.exhausted_terminal_state_id.is_some() as u8;
        if target_count != 1 {
            return Err(CompilerContractError::InvalidDocument {
                message:
                    "compiled threshold policy must target exactly one exhausted node or terminal state"
                        .to_owned(),
            });
        }
        Ok(())
    }
}

/// Materialized graph node frozen into a compiled plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedGraphNodePlan {
    pub node_id: String,
    pub stage_kind_id: String,
    pub plane: Plane,
    pub entrypoint_path: String,
    pub entrypoint_contract_id: Option<String>,
    pub running_status_marker: String,
    #[serde(default)]
    pub allowed_result_classes_by_outcome: HashMap<String, Vec<ResultClass>>,
    #[serde(default)]
    pub declared_output_artifacts: Vec<String>,
    #[serde(default)]
    pub required_skill_paths: Vec<String>,
    #[serde(default)]
    pub attached_skill_additions: Vec<String>,
    pub runner_name: Option<String>,
    pub model_name: Option<String>,
    pub thinking_level: Option<String>,
    pub model_reasoning_effort: Option<String>,
    #[serde(default)]
    pub timeout_seconds: u64,
}

impl MaterializedGraphNodePlan {
    /// Validates and normalizes materialized node fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        normalize_canonical_id(&mut self.node_id, "node_id")?;
        normalize_canonical_id(&mut self.stage_kind_id, "stage_kind_id")?;
        validate_markdown_asset_path("entrypoint_path", &mut self.entrypoint_path, "entrypoints/")?;
        require_optional_non_blank("entrypoint_contract_id", &self.entrypoint_contract_id)?;
        normalize_status(&mut self.running_status_marker, "running_status_marker")?;
        normalize_allowed_result_classes(&mut self.allowed_result_classes_by_outcome)?;
        normalize_canonical_ids(
            &mut self.declared_output_artifacts,
            "declared_output_artifacts",
        )?;
        normalize_markdown_asset_paths(
            "required_skill_paths",
            &mut self.required_skill_paths,
            "skills/",
        )?;
        normalize_markdown_asset_paths(
            "attached_skill_additions",
            &mut self.attached_skill_additions,
            "skills/",
        )?;
        require_optional_non_blank("runner_name", &self.runner_name)?;
        require_optional_non_blank("model_name", &self.model_name)?;
        require_optional_non_blank("thinking_level", &self.thinking_level)?;
        require_optional_non_blank("model_reasoning_effort", &self.model_reasoning_effort)
    }
}

/// Frozen graph plan for one runtime plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenGraphPlanePlan {
    pub loop_id: String,
    pub plane: Plane,
    #[serde(default)]
    pub nodes: Vec<MaterializedGraphNodePlan>,
    #[serde(default)]
    pub entry_nodes: Vec<GraphLoopEntryDefinition>,
    #[serde(default)]
    pub transitions: Vec<GraphLoopEdgeDefinition>,
    #[serde(default)]
    pub compiled_entries: Vec<CompiledGraphEntryPlan>,
    pub compiled_completion_entry: Option<CompiledGraphCompletionEntryPlan>,
    #[serde(default)]
    pub compiled_transitions: Vec<CompiledGraphTransitionPlan>,
    #[serde(default)]
    pub compiled_resume_policies: Vec<CompiledGraphResumePolicyPlan>,
    #[serde(default)]
    pub compiled_threshold_policies: Vec<CompiledGraphThresholdPolicyPlan>,
    #[serde(default)]
    pub terminal_states: Vec<GraphLoopTerminalStateDefinition>,
    pub completion_behavior: Option<GraphLoopCompletionBehaviorDefinition>,
}

impl FrozenGraphPlanePlan {
    /// Validates graph plane alignment and compiled graph subcontracts.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        normalize_canonical_id(&mut self.loop_id, "loop_id")?;
        require_non_empty("nodes", self.nodes.len())?;
        require_non_empty("entry_nodes", self.entry_nodes.len())?;
        require_non_empty("transitions", self.transitions.len())?;
        require_non_empty("compiled_entries", self.compiled_entries.len())?;
        require_non_empty("compiled_transitions", self.compiled_transitions.len())?;
        require_non_empty("terminal_states", self.terminal_states.len())?;

        for node in &mut self.nodes {
            node.validate()?;
            if node.plane != self.plane {
                return Err(CompilerContractError::InvalidDocument {
                    message: "all graph nodes must belong to graph plane".to_owned(),
                });
            }
        }
        for entry in &mut self.entry_nodes {
            entry.validate()?;
        }
        for transition in &mut self.transitions {
            transition.validate()?;
        }
        for entry in &mut self.compiled_entries {
            entry.validate()?;
            if entry.plane != self.plane {
                return Err(CompilerContractError::InvalidDocument {
                    message: "all compiled graph entries must belong to graph plane".to_owned(),
                });
            }
        }
        if let Some(entry) = &mut self.compiled_completion_entry {
            entry.validate()?;
            if entry.plane != self.plane {
                return Err(CompilerContractError::InvalidDocument {
                    message: "compiled completion entry must belong to graph plane".to_owned(),
                });
            }
        }
        for transition in &mut self.compiled_transitions {
            transition.validate()?;
        }
        for policy in &mut self.compiled_resume_policies {
            policy.validate()?;
        }
        for policy in &mut self.compiled_threshold_policies {
            policy.validate()?;
        }
        for terminal_state in &mut self.terminal_states {
            terminal_state.validate()?;
        }
        if let Some(completion) = &mut self.completion_behavior {
            completion.validate()?;
        }
        match (&self.completion_behavior, &self.compiled_completion_entry) {
            (None, Some(_)) => {
                return Err(CompilerContractError::InvalidDocument {
                    message: "compiled completion entry requires completion_behavior".to_owned(),
                });
            }
            (Some(_), None) => {
                return Err(CompilerContractError::InvalidDocument {
                    message:
                        "graphs with completion_behavior must define compiled completion entry"
                            .to_owned(),
                });
            }
            (Some(completion), Some(entry)) => {
                if completion.target_node_id != entry.node_id {
                    return Err(CompilerContractError::InvalidDocument {
                        message:
                            "compiled completion entry must target completion_behavior.target_node_id"
                                .to_owned(),
                    });
                }
            }
            (None, None) => {}
        }
        Ok(())
    }
}

/// Three-part fingerprint of inputs used for one compile attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileInputFingerprint {
    pub mode_id: String,
    pub config_fingerprint: String,
    pub assets_fingerprint: String,
}

impl CompileInputFingerprint {
    /// Validates fingerprint identity fields.
    pub fn validate(&self) -> Result<(), CompilerContractError> {
        require_non_blank("mode_id", &self.mode_id)?;
        require_non_blank("config_fingerprint", &self.config_fingerprint)?;
        require_non_blank("assets_fingerprint", &self.assets_fingerprint)
    }
}

/// One asset whose content participates in compile currentness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedAssetRef {
    pub asset_family: String,
    pub logical_id: String,
    pub compile_time_path: String,
    pub content_sha256: String,
}

impl ResolvedAssetRef {
    /// Validates resolved asset reference fields.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        require_non_blank("asset_family", &self.asset_family)?;
        require_non_blank("logical_id", &self.logical_id)?;
        require_non_blank("compile_time_path", &self.compile_time_path)?;
        require_non_blank("content_sha256", &self.content_sha256)
    }
}

/// Runtime-authoritative compiled plan persisted as `compiled_plan.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledRunPlan {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_compiled_run_plan_kind")]
    pub kind: String,
    pub compiled_plan_id: String,
    pub compile_input_fingerprint: CompileInputFingerprint,
    pub mode_id: String,
    pub loop_ids_by_plane: HashMap<Plane, String>,
    pub execution_loop_id: String,
    pub planning_loop_id: String,
    pub learning_loop_id: Option<String>,
    pub graphs_by_plane: HashMap<Plane, FrozenGraphPlanePlan>,
    pub execution_graph: FrozenGraphPlanePlan,
    pub planning_graph: FrozenGraphPlanePlan,
    pub learning_graph: Option<FrozenGraphPlanePlan>,
    pub concurrency_policy: Option<PlaneConcurrencyPolicyDefinition>,
    #[serde(default)]
    pub learning_trigger_rules: Vec<LearningTriggerRuleDefinition>,
    pub compiled_at: Timestamp,
    #[serde(default)]
    pub resolved_assets: Vec<ResolvedAssetRef>,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

impl CompilerContract for CompiledRunPlan {
    const ARTIFACT: &'static str = "compiled_run_plan";

    fn validate_contract(&mut self) -> Result<(), CompilerContractError> {
        self.validate()
    }
}

impl CompiledRunPlan {
    /// Validates compiled plan graph aliases and plane alignment.
    pub fn validate(&mut self) -> Result<(), CompilerContractError> {
        validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
        validate_literal("kind", &self.kind, "compiled_run_plan")?;
        require_non_blank("compiled_plan_id", &self.compiled_plan_id)?;
        self.compile_input_fingerprint.validate()?;
        require_non_blank("mode_id", &self.mode_id)?;
        require_non_blank("execution_loop_id", &self.execution_loop_id)?;
        require_non_blank("planning_loop_id", &self.planning_loop_id)?;
        require_optional_non_blank("learning_loop_id", &self.learning_loop_id)?;
        if self.loop_ids_by_plane.get(&Plane::Execution) != Some(&self.execution_loop_id) {
            return Err(CompilerContractError::InvalidDocument {
                message: "loop_ids_by_plane execution id must match execution_loop_id".to_owned(),
            });
        }
        if self.loop_ids_by_plane.get(&Plane::Planning) != Some(&self.planning_loop_id) {
            return Err(CompilerContractError::InvalidDocument {
                message: "loop_ids_by_plane planning id must match planning_loop_id".to_owned(),
            });
        }
        if let Some(learning_loop_id) = &self.learning_loop_id {
            if self.loop_ids_by_plane.get(&Plane::Learning) != Some(learning_loop_id) {
                return Err(CompilerContractError::InvalidDocument {
                    message: "loop_ids_by_plane learning id must match learning_loop_id".to_owned(),
                });
            }
        } else if self.loop_ids_by_plane.contains_key(&Plane::Learning) {
            return Err(CompilerContractError::InvalidDocument {
                message: "learning loop binding requires learning_loop_id".to_owned(),
            });
        }

        self.execution_graph.validate()?;
        self.planning_graph.validate()?;
        if let Some(graph) = &mut self.learning_graph {
            graph.validate()?;
        }
        if self.execution_graph.plane != Plane::Execution {
            return Err(CompilerContractError::InvalidDocument {
                message: "execution_graph must declare plane=execution".to_owned(),
            });
        }
        if self.planning_graph.plane != Plane::Planning {
            return Err(CompilerContractError::InvalidDocument {
                message: "planning_graph must declare plane=planning".to_owned(),
            });
        }
        if let Some(graph) = &self.learning_graph {
            if graph.plane != Plane::Learning {
                return Err(CompilerContractError::InvalidDocument {
                    message: "learning_graph must declare plane=learning".to_owned(),
                });
            }
            if Some(&graph.loop_id) != self.learning_loop_id.as_ref() {
                return Err(CompilerContractError::InvalidDocument {
                    message: "learning_loop_id must match learning_graph.loop_id".to_owned(),
                });
            }
        }
        if self.execution_graph.loop_id != self.execution_loop_id {
            return Err(CompilerContractError::InvalidDocument {
                message: "execution_loop_id must match execution_graph.loop_id".to_owned(),
            });
        }
        if self.planning_graph.loop_id != self.planning_loop_id {
            return Err(CompilerContractError::InvalidDocument {
                message: "planning_loop_id must match planning_graph.loop_id".to_owned(),
            });
        }
        if self.graphs_by_plane.get(&Plane::Execution) != Some(&self.execution_graph) {
            return Err(CompilerContractError::InvalidDocument {
                message: "graphs_by_plane execution graph must match execution_graph".to_owned(),
            });
        }
        if self.graphs_by_plane.get(&Plane::Planning) != Some(&self.planning_graph) {
            return Err(CompilerContractError::InvalidDocument {
                message: "graphs_by_plane planning graph must match planning_graph".to_owned(),
            });
        }
        if let Some(graph) = &self.learning_graph {
            if self.graphs_by_plane.get(&Plane::Learning) != Some(graph) {
                return Err(CompilerContractError::InvalidDocument {
                    message: "graphs_by_plane learning graph must match learning_graph".to_owned(),
                });
            }
        } else if self.graphs_by_plane.contains_key(&Plane::Learning) {
            return Err(CompilerContractError::InvalidDocument {
                message: "learning graph binding requires learning_graph".to_owned(),
            });
        }
        if let Some(policy) = &self.concurrency_policy {
            policy.validate()?;
        }
        for rule in &mut self.learning_trigger_rules {
            rule.validate()?;
        }
        if !self.learning_trigger_rules.is_empty() && self.learning_graph.is_none() {
            return Err(CompilerContractError::InvalidDocument {
                message: "learning_trigger_rules require learning_graph".to_owned(),
            });
        }
        for asset in &mut self.resolved_assets {
            asset.validate()?;
        }
        normalize_non_blank_vec("source_refs", &mut self.source_refs)
    }
}

/// Result of one compile attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileOutcome {
    pub active_plan: Option<CompiledRunPlan>,
    pub diagnostics: CompileDiagnostics,
    pub used_last_known_good: bool,
    pub compiled_plan_id: Option<String>,
    pub resolved_assets: Vec<ResolvedAssetRef>,
    pub compile_input_fingerprint: Option<CompileInputFingerprint>,
}

/// Read-only comparison between expected and persisted compile inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledPlanCurrentness {
    pub state: CompiledPlanCurrentnessState,
    pub expected_fingerprint: CompileInputFingerprint,
    pub persisted_plan_id: Option<String>,
    pub persisted_fingerprint: Option<CompileInputFingerprint>,
}

impl CompiledPlanCurrentness {
    /// Validates currentness fields.
    pub fn validate(&self) -> Result<(), CompilerContractError> {
        self.expected_fingerprint.validate()?;
        require_optional_non_blank("persisted_plan_id", &self.persisted_plan_id)?;
        if let Some(fingerprint) = &self.persisted_fingerprint {
            fingerprint.validate()?;
        }
        if self.state == CompiledPlanCurrentnessState::Missing
            && (self.persisted_plan_id.is_some() || self.persisted_fingerprint.is_some())
        {
            return Err(CompilerContractError::InvalidDocument {
                message: "missing currentness cannot include persisted plan identity".to_owned(),
            });
        }
        Ok(())
    }
}

fn decode_json<T>(artifact: &'static str, value: Value) -> Result<T, CompilerContractError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value).map_err(|error| CompilerContractError::Json {
        artifact,
        message: error.to_string(),
    })
}

fn validate_literal(
    field_name: &'static str,
    actual: &str,
    expected: &'static str,
) -> Result<(), CompilerContractError> {
    if actual == expected {
        Ok(())
    } else {
        Err(CompilerContractError::InvalidLiteral {
            field_name,
            expected,
            actual: actual.to_owned(),
        })
    }
}

fn require_non_blank(field_name: &'static str, value: &str) -> Result<(), CompilerContractError> {
    if value.trim().is_empty() {
        Err(CompilerContractError::InvalidField {
            field_name,
            message: "is required".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn require_optional_non_blank(
    field_name: &'static str,
    value: &Option<String>,
) -> Result<(), CompilerContractError> {
    if let Some(value) = value {
        require_non_blank(field_name, value)?;
    }
    Ok(())
}

fn require_non_empty(field_name: &'static str, len: usize) -> Result<(), CompilerContractError> {
    if len == 0 {
        Err(CompilerContractError::InvalidField {
            field_name,
            message: "must not be empty".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn normalize_canonical_id(
    value: &mut String,
    field_name: &'static str,
) -> Result<(), CompilerContractError> {
    let normalized = value.trim().to_ascii_lowercase();
    if !is_canonical_id(&normalized) {
        return Err(CompilerContractError::InvalidField {
            field_name,
            message: "must use lowercase canonical tokens separated by '.', '_', or '-'".to_owned(),
        });
    }
    *value = normalized;
    Ok(())
}

fn normalize_optional_canonical_id(
    value: &mut Option<String>,
    field_name: &'static str,
) -> Result<(), CompilerContractError> {
    if let Some(value) = value {
        normalize_canonical_id(value, field_name)?;
    }
    Ok(())
}

fn normalize_canonical_ids(
    values: &mut Vec<String>,
    field_name: &'static str,
) -> Result<(), CompilerContractError> {
    for value in values.iter_mut() {
        normalize_canonical_id(value, field_name)?;
    }
    dedupe(values);
    Ok(())
}

fn is_canonical_id(value: &str) -> bool {
    let mut previous_was_separator = false;
    let mut saw_char = false;
    for (index, byte) in value.bytes().enumerate() {
        let is_alnum = byte.is_ascii_lowercase() || byte.is_ascii_digit();
        let is_separator = matches!(byte, b'.' | b'_' | b'-');
        if !is_alnum && !is_separator {
            return false;
        }
        if index == 0 && is_separator {
            return false;
        }
        if is_separator && previous_was_separator {
            return false;
        }
        previous_was_separator = is_separator;
        saw_char = true;
    }
    saw_char && !previous_was_separator
}

fn normalize_status(
    value: &mut String,
    field_name: &'static str,
) -> Result<(), CompilerContractError> {
    let normalized = value.trim().to_ascii_uppercase();
    if !is_status(&normalized) {
        return Err(CompilerContractError::InvalidField {
            field_name,
            message: "must use uppercase status tokens".to_owned(),
        });
    }
    *value = normalized;
    Ok(())
}

fn normalize_status_values(
    values: &mut Vec<String>,
    field_name: &'static str,
    require_values: bool,
) -> Result<(), CompilerContractError> {
    for value in values.iter_mut() {
        normalize_status(value, field_name)?;
    }
    dedupe(values);
    if require_values && values.is_empty() {
        return Err(CompilerContractError::InvalidField {
            field_name,
            message: "must not be empty".to_owned(),
        });
    }
    Ok(())
}

fn is_status(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn normalize_nonempty_text(
    value: &mut String,
    field_name: &'static str,
) -> Result<(), CompilerContractError> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Err(CompilerContractError::InvalidField {
            field_name,
            message: "may not be empty".to_owned(),
        });
    }
    *value = normalized;
    Ok(())
}

fn validate_optional_nonempty_text(
    field_name: &'static str,
    value: &mut Option<String>,
) -> Result<(), CompilerContractError> {
    if let Some(value) = value {
        normalize_nonempty_text(value, field_name)?;
    }
    Ok(())
}

fn validate_markdown_asset_path(
    field_name: &'static str,
    value: &mut String,
    required_prefix: &'static str,
) -> Result<(), CompilerContractError> {
    let normalized = value.trim().replace('\\', "/");
    if !is_markdown_asset_path(&normalized, required_prefix) {
        return Err(CompilerContractError::InvalidField {
            field_name,
            message: format!(
                "must be a markdown asset path under {required_prefix:?} without traversal"
            ),
        });
    }
    *value = normalized;
    Ok(())
}

fn validate_optional_markdown_asset_path(
    field_name: &'static str,
    value: &mut Option<String>,
    required_prefix: &'static str,
) -> Result<(), CompilerContractError> {
    if let Some(value) = value {
        validate_markdown_asset_path(field_name, value, required_prefix)?;
    }
    Ok(())
}

fn normalize_markdown_asset_paths(
    field_name: &'static str,
    values: &mut Vec<String>,
    required_prefix: &'static str,
) -> Result<(), CompilerContractError> {
    for value in values.iter_mut() {
        validate_markdown_asset_path(field_name, value, required_prefix)?;
    }
    dedupe(values);
    Ok(())
}

fn is_markdown_asset_path(value: &str, required_prefix: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && value.starts_with(required_prefix)
        && value.ends_with(".md")
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "..")
}

fn normalize_override_names(
    values: &mut Vec<String>,
    field_name: &'static str,
) -> Result<(), CompilerContractError> {
    for value in values.iter_mut() {
        let normalized = value.trim().to_ascii_lowercase();
        if !is_override_name(&normalized) {
            return Err(CompilerContractError::InvalidField {
                field_name,
                message: "must use lowercase snake_case override names".to_owned(),
            });
        }
        *value = normalized;
    }
    dedupe(values);
    Ok(())
}

fn is_override_name(value: &str) -> bool {
    let mut previous_was_separator = false;
    let mut saw_char = false;
    for (index, byte) in value.bytes().enumerate() {
        let is_alnum = byte.is_ascii_lowercase() || byte.is_ascii_digit();
        let is_separator = byte == b'_';
        if !is_alnum && !is_separator {
            return false;
        }
        if index == 0 && is_separator {
            return false;
        }
        if is_separator && previous_was_separator {
            return false;
        }
        previous_was_separator = is_separator;
        saw_char = true;
    }
    saw_char && !previous_was_separator
}

fn normalize_allowed_result_classes(
    mapping: &mut HashMap<String, Vec<ResultClass>>,
) -> Result<(), CompilerContractError> {
    if mapping.is_empty() {
        return Err(CompilerContractError::InvalidField {
            field_name: "allowed_result_classes_by_outcome",
            message: "must not be empty".to_owned(),
        });
    }
    let mut normalized = HashMap::new();
    for (outcome, result_classes) in std::mem::take(mapping) {
        let mut normalized_outcome = outcome;
        normalize_status(&mut normalized_outcome, "allowed_result_classes_by_outcome")?;
        if normalized
            .insert(normalized_outcome, deduped(result_classes))
            .is_some()
        {
            return Err(CompilerContractError::InvalidDocument {
                message:
                    "allowed_result_classes_by_outcome may not declare duplicate normalized outcomes"
                        .to_owned(),
            });
        }
    }
    *mapping = normalized;
    Ok(())
}

fn normalize_stage_string_map(
    mapping: &mut HashMap<StageName, String>,
) -> Result<(), CompilerContractError> {
    for value in mapping.values() {
        require_non_blank("stage binding", value)?;
    }
    Ok(())
}

fn normalize_stage_optional_string_map(
    mapping: &mut HashMap<StageName, Option<String>>,
) -> Result<(), CompilerContractError> {
    for value in mapping.values() {
        require_optional_non_blank("stage binding", value)?;
    }
    Ok(())
}

fn normalize_stage_vec_map(
    mapping: &mut HashMap<StageName, Vec<String>>,
) -> Result<(), CompilerContractError> {
    for values in mapping.values_mut() {
        for value in values.iter() {
            require_non_blank("stage binding", value)?;
        }
        dedupe(values);
    }
    Ok(())
}

fn normalize_non_blank_vec(
    field_name: &'static str,
    values: &mut Vec<String>,
) -> Result<(), CompilerContractError> {
    for value in values.iter() {
        require_non_blank(field_name, value)?;
    }
    dedupe(values);
    Ok(())
}

fn validate_plane_groups(
    field_name: &'static str,
    groups: &[Vec<Plane>],
) -> Result<(), CompilerContractError> {
    for group in groups {
        if group.is_empty() {
            return Err(CompilerContractError::InvalidField {
                field_name,
                message: "plane groups must not be empty".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_edge_target_shape(
    label: &'static str,
    kind: LoopEdgeKind,
    has_node_target: bool,
    has_terminal_target: bool,
    max_attempts: Option<u64>,
) -> Result<(), CompilerContractError> {
    let target_count = has_node_target as u8 + has_terminal_target as u8;
    if target_count != 1 {
        return Err(CompilerContractError::InvalidDocument {
            message: format!("{label} must target exactly one node or terminal_state_id"),
        });
    }
    if kind == LoopEdgeKind::Terminal && !has_terminal_target {
        return Err(CompilerContractError::InvalidDocument {
            message: format!("{label} with kind=terminal must target a terminal_state_id"),
        });
    }
    if kind == LoopEdgeKind::Retry && max_attempts.is_none() {
        return Err(CompilerContractError::InvalidDocument {
            message: format!("{label} with kind=retry must declare max_attempts"),
        });
    }
    if kind != LoopEdgeKind::Retry && max_attempts.is_some() {
        return Err(CompilerContractError::InvalidDocument {
            message: format!("{label} may only declare max_attempts when kind=retry"),
        });
    }
    Ok(())
}

fn unique_ids<'a>(
    duplicate_message: &'static str,
    ids: impl Iterator<Item = &'a str>,
) -> Result<HashSet<&'a str>, CompilerContractError> {
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(CompilerContractError::InvalidDocument {
                message: duplicate_message.to_owned(),
            });
        }
    }
    Ok(seen)
}

fn require_known_node(
    label: &'static str,
    policy_id: &str,
    field_name: &'static str,
    node_id: &str,
    node_ids: &HashSet<&str>,
) -> Result<(), CompilerContractError> {
    if node_ids.contains(node_id) {
        Ok(())
    } else {
        Err(CompilerContractError::InvalidDocument {
            message: format!("{label} {policy_id} references unknown {field_name} {node_id}"),
        })
    }
}

fn require_known_terminal_state(
    label: &'static str,
    policy_id: &str,
    field_name: &'static str,
    terminal_state_id: &str,
    terminal_state_ids: &HashSet<&str>,
) -> Result<(), CompilerContractError> {
    if terminal_state_ids.contains(terminal_state_id) {
        Ok(())
    } else {
        Err(CompilerContractError::InvalidDocument {
            message: format!(
                "{label} {policy_id} references unknown {field_name} {terminal_state_id}"
            ),
        })
    }
}

fn dedupe<T>(values: &mut Vec<T>)
where
    T: Eq + std::hash::Hash + Clone,
{
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn deduped<T>(mut values: Vec<T>) -> Vec<T>
where
    T: Eq + std::hash::Hash + Clone,
{
    dedupe(&mut values);
    values
}

fn default_schema_version() -> String {
    SCHEMA_VERSION.to_owned()
}

fn default_mode_kind() -> String {
    "mode".to_owned()
}

fn default_graph_loop_kind() -> String {
    "graph_loop".to_owned()
}

fn default_registered_stage_kind() -> String {
    "registered_stage_kind".to_owned()
}

fn default_compiled_run_plan_kind() -> String {
    "compiled_run_plan".to_owned()
}

fn default_learning_request_action() -> LearningRequestAction {
    LearningRequestAction::Improve
}

fn default_loop_edge_kind() -> LoopEdgeKind {
    LoopEdgeKind::Normal
}

fn default_priority() -> i64 {
    100
}

fn default_true() -> bool {
    true
}

fn default_stage_idempotence_policy() -> StageIdempotencePolicy {
    StageIdempotencePolicy::RetrySafeWithKey
}

fn default_closure_target_entry_key() -> GraphLoopEntryKey {
    GraphLoopEntryKey::ClosureTarget
}
