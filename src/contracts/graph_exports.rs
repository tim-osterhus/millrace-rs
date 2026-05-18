//! Public compiled-stage-graph export contracts.

use std::{collections::HashMap, fmt};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::{
    ContractError, ExecutionCapabilityGrant, ExecutionCapabilityWarning, Plane, ResultClass,
    Timestamp, validate_safe_identifier,
};

const SCHEMA_VERSION: &str = "1.0";
const COMPILED_STAGE_GRAPH_KIND: &str = "compiled_stage_graph";

/// Typed failures produced while decoding or validating graph export contracts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphExportContractError {
    /// A lower-level shared contract validation failed.
    Contract(ContractError),
    /// JSON syntax, required-field, type, or enum decoding failed.
    Json {
        /// Artifact type being decoded.
        artifact: &'static str,
        /// Serde error message.
        message: String,
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

impl fmt::Display for GraphExportContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => write!(f, "{error}"),
            Self::Json { artifact, message } => {
                write!(f, "failed to decode {artifact}: {message}")
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

impl std::error::Error for GraphExportContractError {}

impl From<ContractError> for GraphExportContractError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}

/// Common helpers for graph export JSON artifact contracts.
pub trait GraphExportContract: Sized + Serialize + DeserializeOwned {
    /// Human-readable artifact name used in decode errors.
    const ARTIFACT: &'static str;

    /// Validates this decoded artifact.
    fn validate_contract(&self) -> Result<(), GraphExportContractError>;

    /// Deserializes and validates a JSON value.
    fn from_json_value(value: Value) -> Result<Self, GraphExportContractError> {
        let decoded: Self =
            serde_json::from_value(value).map_err(|error| GraphExportContractError::Json {
                artifact: Self::ARTIFACT,
                message: error.to_string(),
            })?;
        decoded.validate_contract()?;
        Ok(decoded)
    }

    /// Deserializes and validates a JSON string.
    fn from_json_str(raw: &str) -> Result<Self, GraphExportContractError> {
        let value = serde_json::from_str(raw).map_err(|error| GraphExportContractError::Json {
            artifact: Self::ARTIFACT,
            message: error.to_string(),
        })?;
        Self::from_json_value(value)
    }
}

/// Exported materialized node binding for one compiled stage graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphExportNode {
    pub node_id: String,
    pub plane: Plane,
    pub stage_kind_id: String,
    pub entrypoint_path: String,
    pub entrypoint_contract_id: Option<String>,
    pub running_status_marker: String,
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
    pub allowed_result_classes_by_outcome: HashMap<String, Vec<ResultClass>>,
    #[serde(default)]
    pub declared_output_artifacts: Vec<String>,
    #[serde(default)]
    pub execution_capability_grants: Vec<ExecutionCapabilityGrant>,
    #[serde(default)]
    pub execution_capability_warnings: Vec<ExecutionCapabilityWarning>,
    #[serde(default)]
    pub execution_capability_policy_fingerprint: String,
}

impl GraphExportNode {
    /// Validates node field shape without changing exported values.
    pub fn validate(&self) -> Result<(), GraphExportContractError> {
        validate_safe_identifier(&self.node_id, "node_id")?;
        validate_safe_identifier(&self.stage_kind_id, "stage_kind_id")?;
        require_non_blank("entrypoint_path", &self.entrypoint_path)?;
        require_optional_non_blank("entrypoint_contract_id", &self.entrypoint_contract_id)?;
        validate_safe_identifier(&self.running_status_marker, "running_status_marker")?;
        require_non_blank_vec("required_skill_paths", &self.required_skill_paths)?;
        require_non_blank_vec("attached_skill_additions", &self.attached_skill_additions)?;
        require_optional_non_blank("runner_name", &self.runner_name)?;
        require_optional_non_blank("model_name", &self.model_name)?;
        require_optional_non_blank("thinking_level", &self.thinking_level)?;
        require_optional_non_blank("model_reasoning_effort", &self.model_reasoning_effort)?;
        validate_allowed_result_classes(&self.allowed_result_classes_by_outcome)?;
        require_non_blank_vec("declared_output_artifacts", &self.declared_output_artifacts)?;
        for grant in &self.execution_capability_grants {
            grant
                .validate()
                .map_err(|error| GraphExportContractError::InvalidField {
                    field_name: "execution_capability_grants",
                    message: error.to_string(),
                })?;
        }
        for warning in &self.execution_capability_warnings {
            warning
                .validate()
                .map_err(|error| GraphExportContractError::InvalidField {
                    field_name: "execution_capability_warnings",
                    message: error.to_string(),
                })?;
        }
        if !self.execution_capability_policy_fingerprint.is_empty() {
            require_prefixed_fingerprint(
                "execution_capability_policy_fingerprint",
                &self.execution_capability_policy_fingerprint,
                "cap-pol-",
            )?;
        }
        Ok(())
    }
}

/// Exported compiled edge for one concrete terminal outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphExportEdge {
    pub edge_id: String,
    pub source_node_id: String,
    pub outcome: String,
    pub target_node_id: Option<String>,
    pub terminal_state_id: Option<String>,
    pub kind: String,
    pub priority: i64,
    pub max_attempts: Option<u64>,
}

impl GraphExportEdge {
    /// Validates edge field shape without changing exported values.
    pub fn validate(&self) -> Result<(), GraphExportContractError> {
        validate_safe_identifier(&self.edge_id, "edge_id")?;
        validate_safe_identifier(&self.source_node_id, "source_node_id")?;
        validate_safe_identifier(&self.outcome, "outcome")?;
        require_optional_safe_identifier("target_node_id", &self.target_node_id)?;
        require_optional_safe_identifier("terminal_state_id", &self.terminal_state_id)?;
        validate_safe_identifier(&self.kind, "kind")?;
        if self.max_attempts == Some(0) {
            return Err(GraphExportContractError::InvalidField {
                field_name: "max_attempts",
                message: "must be >= 1 when set".to_owned(),
            });
        }
        Ok(())
    }
}

/// Exported named graph entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphExportEntry {
    pub entry_key: String,
    pub node_id: String,
    pub stage_kind_id: String,
    pub plane: Plane,
}

impl GraphExportEntry {
    /// Validates entry field shape without changing exported values.
    pub fn validate(&self) -> Result<(), GraphExportContractError> {
        validate_safe_identifier(&self.entry_key, "entry_key")?;
        validate_safe_identifier(&self.node_id, "node_id")?;
        validate_safe_identifier(&self.stage_kind_id, "stage_kind_id")?;
        Ok(())
    }
}

/// Exported terminal-state contract for one compiled stage graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphExportTerminalState {
    pub terminal_state_id: String,
    pub terminal_class: String,
    pub writes_status: String,
    #[serde(default)]
    pub emits_artifacts: Vec<String>,
    #[serde(default = "default_true")]
    pub ends_plane_run: bool,
}

impl GraphExportTerminalState {
    /// Validates terminal-state field shape without changing exported values.
    pub fn validate(&self) -> Result<(), GraphExportContractError> {
        validate_safe_identifier(&self.terminal_state_id, "terminal_state_id")?;
        validate_safe_identifier(&self.terminal_class, "terminal_class")?;
        validate_safe_identifier(&self.writes_status, "writes_status")?;
        require_non_blank_vec("emits_artifacts", &self.emits_artifacts)
    }
}

/// Public compiled-stage-graph export for one selected runtime plane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledStageGraphExport {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_compiled_stage_graph_kind")]
    pub kind: String,
    pub compiled_plan_id: String,
    pub mode_id: String,
    pub loop_id: String,
    pub plane: Plane,
    #[serde(default)]
    pub nodes: Vec<GraphExportNode>,
    #[serde(default)]
    pub edges: Vec<GraphExportEdge>,
    #[serde(default)]
    pub entries: Vec<GraphExportEntry>,
    #[serde(default)]
    pub terminal_states: Vec<GraphExportTerminalState>,
    #[serde(default)]
    pub source_refs: Vec<String>,
    pub exported_at: Timestamp,
}

impl GraphExportContract for CompiledStageGraphExport {
    const ARTIFACT: &'static str = "compiled_stage_graph";

    fn validate_contract(&self) -> Result<(), GraphExportContractError> {
        self.validate()
    }
}

impl CompiledStageGraphExport {
    /// Deserializes and validates a JSON value.
    pub fn from_json_value(value: Value) -> Result<Self, GraphExportContractError> {
        <Self as GraphExportContract>::from_json_value(value)
    }

    /// Deserializes and validates a JSON string.
    pub fn from_json_str(raw: &str) -> Result<Self, GraphExportContractError> {
        <Self as GraphExportContract>::from_json_str(raw)
    }

    /// Validates the graph export contract and graph-plane alignment.
    pub fn validate(&self) -> Result<(), GraphExportContractError> {
        validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
        validate_literal("kind", &self.kind, COMPILED_STAGE_GRAPH_KIND)?;
        validate_safe_identifier(&self.compiled_plan_id, "compiled_plan_id")?;
        validate_safe_identifier(&self.mode_id, "mode_id")?;
        validate_safe_identifier(&self.loop_id, "loop_id")?;
        require_non_empty("nodes", self.nodes.len())?;
        require_non_empty("edges", self.edges.len())?;
        require_non_empty("entries", self.entries.len())?;
        require_non_empty("terminal_states", self.terminal_states.len())?;

        for node in &self.nodes {
            node.validate()?;
            if node.plane != self.plane {
                return Err(GraphExportContractError::InvalidDocument {
                    message: "all graph export nodes must belong to graph plane".to_owned(),
                });
            }
        }
        for edge in &self.edges {
            edge.validate()?;
        }
        for entry in &self.entries {
            entry.validate()?;
            if entry.plane != self.plane {
                return Err(GraphExportContractError::InvalidDocument {
                    message: "all graph export entries must belong to graph plane".to_owned(),
                });
            }
        }
        for terminal_state in &self.terminal_states {
            terminal_state.validate()?;
        }
        require_non_blank_vec("source_refs", &self.source_refs)
    }
}

fn validate_literal(
    field_name: &'static str,
    actual: &str,
    expected: &'static str,
) -> Result<(), GraphExportContractError> {
    if actual == expected {
        Ok(())
    } else {
        Err(GraphExportContractError::InvalidLiteral {
            field_name,
            expected,
            actual: actual.to_owned(),
        })
    }
}

fn require_non_empty(field_name: &'static str, len: usize) -> Result<(), GraphExportContractError> {
    if len == 0 {
        Err(GraphExportContractError::InvalidField {
            field_name,
            message: "must not be empty".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn require_non_blank(
    field_name: &'static str,
    value: &str,
) -> Result<(), GraphExportContractError> {
    if value.trim() == value && !value.is_empty() {
        Ok(())
    } else {
        Err(GraphExportContractError::InvalidField {
            field_name,
            message: "must not be blank or include surrounding whitespace".to_owned(),
        })
    }
}

fn require_optional_non_blank(
    field_name: &'static str,
    value: &Option<String>,
) -> Result<(), GraphExportContractError> {
    if let Some(value) = value {
        require_non_blank(field_name, value)?;
    }
    Ok(())
}

fn require_optional_safe_identifier(
    field_name: &'static str,
    value: &Option<String>,
) -> Result<(), GraphExportContractError> {
    if let Some(value) = value {
        validate_safe_identifier(value, field_name)?;
    }
    Ok(())
}

fn require_non_blank_vec(
    field_name: &'static str,
    values: &[String],
) -> Result<(), GraphExportContractError> {
    for value in values {
        require_non_blank(field_name, value)?;
    }
    Ok(())
}

fn require_prefixed_fingerprint(
    field_name: &'static str,
    value: &str,
    prefix: &'static str,
) -> Result<(), GraphExportContractError> {
    let Some(suffix) = value.strip_prefix(prefix) else {
        return Err(GraphExportContractError::InvalidField {
            field_name,
            message: format!("must use the {prefix}<hex> format"),
        });
    };
    if suffix.len() != 12 || !suffix.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(GraphExportContractError::InvalidField {
            field_name,
            message: format!("must use the {prefix}<hex> format"),
        });
    }
    Ok(())
}

fn validate_allowed_result_classes(
    mapping: &HashMap<String, Vec<ResultClass>>,
) -> Result<(), GraphExportContractError> {
    if mapping.is_empty() {
        return Err(GraphExportContractError::InvalidField {
            field_name: "allowed_result_classes_by_outcome",
            message: "must not be empty".to_owned(),
        });
    }
    for (outcome, result_classes) in mapping {
        validate_safe_identifier(outcome, "allowed_result_classes_by_outcome")?;
        if result_classes.is_empty() {
            return Err(GraphExportContractError::InvalidDocument {
                message: "allowed result-class mappings must not be empty".to_owned(),
            });
        }
    }
    Ok(())
}

fn default_schema_version() -> String {
    SCHEMA_VERSION.to_owned()
}

fn default_compiled_stage_graph_kind() -> String {
    COMPILED_STAGE_GRAPH_KIND.to_owned()
}

fn default_true() -> bool {
    true
}
