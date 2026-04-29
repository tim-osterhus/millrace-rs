use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{
    ContractError, IncidentDecision, IncidentSeverity, LearningRequestAction, LearningStageName,
    Plane, SpecSourceType, StageName, TaskStatusHint, WorkItemKind, stage_plane,
    validate_safe_identifier,
};

/// Canonical work-document schema version used by the Python reference.
pub const WORK_DOCUMENT_SCHEMA_VERSION: &str = "1.0";

/// Typed validation and parsing failures for work documents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkDocumentError {
    /// A lower-level contract validation failed.
    Contract(ContractError),
    /// A required scalar field was missing.
    MissingRequiredField {
        /// Canonical snake_case field name.
        field_name: &'static str,
    },
    /// A required collection was missing or empty.
    EmptyRequiredList {
        /// Canonical snake_case field name.
        field_name: &'static str,
    },
    /// A scalar field could not be parsed or validated.
    InvalidField {
        /// Canonical snake_case field name.
        field_name: &'static str,
        /// Raw invalid value.
        value: String,
        /// Human-readable failure reason.
        message: String,
    },
    /// A document-level invariant failed.
    InvalidDocument {
        /// Human-readable failure reason.
        message: String,
    },
    /// File IO failed while reading a work document.
    Io {
        /// Path that could not be read.
        path: String,
        /// Human-readable IO error.
        message: String,
    },
}

impl fmt::Display for WorkDocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => write!(f, "{error}"),
            Self::MissingRequiredField { field_name } => {
                write!(f, "{field_name} is required")
            }
            Self::EmptyRequiredList { field_name } => {
                write!(f, "{field_name} must include at least one item")
            }
            Self::InvalidField {
                field_name,
                value,
                message,
            } => write!(f, "{field_name} has invalid value `{value}`: {message}"),
            Self::InvalidDocument { message } => f.write_str(message),
            Self::Io { path, message } => write!(f, "failed to read {path}: {message}"),
        }
    }
}

impl std::error::Error for WorkDocumentError {}

impl From<ContractError> for WorkDocumentError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}

/// RFC 3339 timestamp as stored in human-facing work documents.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Timestamp(String);

impl Timestamp {
    /// Parses and preserves a canonical timestamp string.
    pub fn parse(field_name: &'static str, value: &str) -> Result<Self, WorkDocumentError> {
        if value.trim() != value || value.is_empty() {
            return Err(WorkDocumentError::InvalidField {
                field_name,
                value: value.to_owned(),
                message: "must be a non-empty RFC 3339 timestamp without surrounding whitespace"
                    .to_owned(),
            });
        }
        OffsetDateTime::parse(value, &Rfc3339).map_err(|error| {
            WorkDocumentError::InvalidField {
                field_name,
                value: value.to_owned(),
                message: error.to_string(),
            }
        })?;
        Ok(Self(value.to_owned()))
    }

    /// Returns the preserved timestamp string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Timestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse("timestamp", &value).map_err(serde::de::Error::custom)
    }
}

/// Task queue document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDocument {
    pub task_id: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    pub root_idea_id: Option<String>,
    pub root_spec_id: Option<String>,
    pub spec_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub incident_id: Option<String>,
    pub target_paths: Vec<String>,
    pub acceptance: Vec<String>,
    pub required_checks: Vec<String>,
    pub references: Vec<String>,
    pub risk: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub blocks: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub status_hint: Option<TaskStatusHint>,
    pub created_at: Timestamp,
    pub created_by: String,
    pub updated_at: Option<Timestamp>,
}

impl TaskDocument {
    /// Returns the document kind token.
    #[must_use]
    pub const fn kind(&self) -> WorkItemKind {
        WorkItemKind::Task
    }

    /// Returns the fixed schema version.
    #[must_use]
    pub const fn schema_version(&self) -> &'static str {
        WORK_DOCUMENT_SCHEMA_VERSION
    }

    /// Validates the Python-reference task contract.
    pub fn validate(&self) -> Result<(), WorkDocumentError> {
        validate_safe_identifier(&self.task_id, "task_id")?;
        validate_optional_identifier("root_idea_id", &self.root_idea_id)?;
        validate_optional_identifier("root_spec_id", &self.root_spec_id)?;
        validate_optional_identifier("spec_id", &self.spec_id)?;
        validate_optional_identifier("parent_task_id", &self.parent_task_id)?;
        validate_optional_identifier("incident_id", &self.incident_id)?;
        validate_required_list("target_paths", &self.target_paths)?;
        validate_required_list("acceptance", &self.acceptance)?;
        validate_required_list("required_checks", &self.required_checks)?;
        validate_required_list("references", &self.references)?;
        validate_required_list("risk", &self.risk)?;
        Ok(())
    }
}

/// Spec planning document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecDocument {
    pub spec_id: String,
    pub title: String,
    pub summary: String,
    pub source_type: SpecSourceType,
    pub source_id: Option<String>,
    pub parent_spec_id: Option<String>,
    pub root_idea_id: Option<String>,
    pub root_spec_id: Option<String>,
    pub goals: Vec<String>,
    #[serde(default)]
    pub non_goals: Vec<String>,
    #[serde(default)]
    pub scope: Vec<String>,
    pub constraints: Vec<String>,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub target_paths: Vec<String>,
    #[serde(default)]
    pub entrypoints: Vec<String>,
    #[serde(default)]
    pub required_skills: Vec<String>,
    #[serde(default)]
    pub decomposition_hints: Vec<String>,
    pub acceptance: Vec<String>,
    pub references: Vec<String>,
    pub created_at: Timestamp,
    pub created_by: String,
    pub updated_at: Option<Timestamp>,
}

impl SpecDocument {
    /// Returns the document kind token.
    #[must_use]
    pub const fn kind(&self) -> WorkItemKind {
        WorkItemKind::Spec
    }

    /// Returns the fixed schema version.
    #[must_use]
    pub const fn schema_version(&self) -> &'static str {
        WORK_DOCUMENT_SCHEMA_VERSION
    }

    /// Validates the Python-reference spec contract.
    pub fn validate(&self) -> Result<(), WorkDocumentError> {
        validate_safe_identifier(&self.spec_id, "spec_id")?;
        validate_optional_identifier("source_id", &self.source_id)?;
        validate_optional_identifier("parent_spec_id", &self.parent_spec_id)?;
        validate_optional_identifier("root_idea_id", &self.root_idea_id)?;
        validate_optional_identifier("root_spec_id", &self.root_spec_id)?;
        validate_required_list("goals", &self.goals)?;
        validate_required_list("constraints", &self.constraints)?;
        validate_required_list("acceptance", &self.acceptance)?;
        validate_required_list("references", &self.references)?;
        Ok(())
    }
}

/// Incident planning document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncidentDocument {
    pub incident_id: String,
    pub title: String,
    pub summary: String,
    pub root_idea_id: Option<String>,
    pub root_spec_id: Option<String>,
    pub source_task_id: Option<String>,
    pub source_spec_id: Option<String>,
    pub source_stage: StageName,
    pub source_plane: Plane,
    pub failure_class: String,
    pub severity: IncidentSeverity,
    pub needs_planning: bool,
    pub trigger_reason: String,
    pub observed_symptoms: Vec<String>,
    pub failed_attempts: Vec<String>,
    pub consultant_decision: IncidentDecision,
    pub evidence_paths: Vec<String>,
    pub related_run_ids: Vec<String>,
    pub related_stage_results: Vec<String>,
    pub references: Vec<String>,
    pub opened_at: Timestamp,
    pub opened_by: String,
    pub updated_at: Option<Timestamp>,
}

impl IncidentDocument {
    /// Returns the document kind token.
    #[must_use]
    pub const fn kind(&self) -> WorkItemKind {
        WorkItemKind::Incident
    }

    /// Returns the fixed schema version.
    #[must_use]
    pub const fn schema_version(&self) -> &'static str {
        WORK_DOCUMENT_SCHEMA_VERSION
    }

    /// Validates the Python-reference incident contract.
    pub fn validate(&self) -> Result<(), WorkDocumentError> {
        validate_safe_identifier(&self.incident_id, "incident_id")?;
        validate_optional_identifier("root_idea_id", &self.root_idea_id)?;
        validate_optional_identifier("root_spec_id", &self.root_spec_id)?;
        validate_optional_identifier("source_task_id", &self.source_task_id)?;
        validate_optional_identifier("source_spec_id", &self.source_spec_id)?;
        if stage_plane(self.source_stage) != self.source_plane {
            return Err(WorkDocumentError::InvalidDocument {
                message: "source_stage must belong to source_plane".to_owned(),
            });
        }
        Ok(())
    }
}

/// Learning request document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningRequestDocument {
    pub learning_request_id: String,
    pub title: String,
    pub summary: String,
    pub requested_action: LearningRequestAction,
    pub target_skill_id: Option<String>,
    pub target_stage: Option<LearningStageName>,
    pub source_refs: Vec<String>,
    pub preferred_output_paths: Vec<String>,
    pub trigger_metadata: Value,
    pub originating_run_ids: Vec<String>,
    pub artifact_paths: Vec<String>,
    pub references: Vec<String>,
    pub created_at: Timestamp,
    pub created_by: String,
    pub updated_at: Option<Timestamp>,
}

impl LearningRequestDocument {
    /// Returns the document kind token.
    #[must_use]
    pub const fn kind(&self) -> WorkItemKind {
        WorkItemKind::LearningRequest
    }

    /// Returns the fixed schema version.
    #[must_use]
    pub const fn schema_version(&self) -> &'static str {
        WORK_DOCUMENT_SCHEMA_VERSION
    }

    /// Validates the Python-reference learning request contract.
    pub fn validate(&self) -> Result<(), WorkDocumentError> {
        validate_safe_identifier(&self.learning_request_id, "learning_request_id")?;
        validate_optional_identifier("target_skill_id", &self.target_skill_id)?;
        for run_id in &self.originating_run_ids {
            validate_safe_identifier(run_id, "originating_run_ids")?;
        }
        if !self.trigger_metadata.is_object() {
            return Err(WorkDocumentError::InvalidField {
                field_name: "trigger_metadata",
                value: self.trigger_metadata.to_string(),
                message: "must be a JSON object".to_owned(),
            });
        }
        Ok(())
    }
}

/// Persisted closure target state owned by the Arbiter contract surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosureTargetState {
    #[serde(default = "default_schema_version_string")]
    pub schema_version: String,
    #[serde(default = "default_closure_target_state_kind")]
    pub kind: String,
    pub root_spec_id: String,
    pub root_idea_id: String,
    pub root_spec_path: String,
    pub root_idea_path: String,
    pub rubric_path: String,
    #[serde(default)]
    pub latest_verdict_path: Option<String>,
    #[serde(default)]
    pub latest_report_path: Option<String>,
    #[serde(default = "default_true")]
    pub closure_open: bool,
    #[serde(default)]
    pub closure_blocked_by_lineage_work: bool,
    #[serde(default)]
    pub blocking_work_ids: Vec<String>,
    pub opened_at: Timestamp,
    #[serde(default)]
    pub closed_at: Option<Timestamp>,
    #[serde(default)]
    pub last_arbiter_run_id: Option<String>,
}

impl ClosureTargetState {
    /// Returns the fixed schema version.
    #[must_use]
    pub const fn schema_version(&self) -> &'static str {
        WORK_DOCUMENT_SCHEMA_VERSION
    }

    /// Validates the Python-reference closure target contract.
    pub fn validate(&self) -> Result<(), WorkDocumentError> {
        validate_literal(
            "schema_version",
            &self.schema_version,
            WORK_DOCUMENT_SCHEMA_VERSION,
        )?;
        validate_literal("kind", &self.kind, "closure_target_state")?;
        validate_safe_identifier(&self.root_spec_id, "root_spec_id")?;
        validate_safe_identifier(&self.root_idea_id, "root_idea_id")?;
        validate_optional_identifier("last_arbiter_run_id", &self.last_arbiter_run_id)?;
        for work_item_id in &self.blocking_work_ids {
            validate_safe_identifier(work_item_id, "blocking_work_ids")?;
        }
        if let Some(closed_at) = &self.closed_at {
            let opened_at = parse_timestamp_for_compare("opened_at", self.opened_at.as_str())?;
            let closed_at = parse_timestamp_for_compare("closed_at", closed_at.as_str())?;
            if closed_at < opened_at {
                return Err(WorkDocumentError::InvalidDocument {
                    message: "closed_at cannot precede opened_at".to_owned(),
                });
            }
            if self.closure_open {
                return Err(WorkDocumentError::InvalidDocument {
                    message: "closed closure target cannot remain open".to_owned(),
                });
            }
        }
        if !self.blocking_work_ids.is_empty() && !self.closure_blocked_by_lineage_work {
            return Err(WorkDocumentError::InvalidDocument {
                message: "blocking_work_ids require closure_blocked_by_lineage_work=true"
                    .to_owned(),
            });
        }
        Ok(())
    }
}

/// Any headed markdown work document supported by the queue surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkDocument {
    Task(TaskDocument),
    Spec(SpecDocument),
    Incident(IncidentDocument),
    LearningRequest(LearningRequestDocument),
}

impl WorkDocument {
    /// Returns the document kind token.
    #[must_use]
    pub const fn kind(&self) -> WorkItemKind {
        match self {
            Self::Task(document) => document.kind(),
            Self::Spec(document) => document.kind(),
            Self::Incident(document) => document.kind(),
            Self::LearningRequest(document) => document.kind(),
        }
    }

    /// Returns the fixed schema version.
    #[must_use]
    pub const fn schema_version(&self) -> &'static str {
        WORK_DOCUMENT_SCHEMA_VERSION
    }

    /// Returns the document title.
    #[must_use]
    pub fn title(&self) -> &str {
        match self {
            Self::Task(document) => &document.title,
            Self::Spec(document) => &document.title,
            Self::Incident(document) => &document.title,
            Self::LearningRequest(document) => &document.title,
        }
    }

    /// Validates the wrapped document.
    pub fn validate(&self) -> Result<(), WorkDocumentError> {
        match self {
            Self::Task(document) => document.validate(),
            Self::Spec(document) => document.validate(),
            Self::Incident(document) => document.validate(),
            Self::LearningRequest(document) => document.validate(),
        }
    }
}

impl From<TaskDocument> for WorkDocument {
    fn from(value: TaskDocument) -> Self {
        Self::Task(value)
    }
}

impl From<SpecDocument> for WorkDocument {
    fn from(value: SpecDocument) -> Self {
        Self::Spec(value)
    }
}

impl From<IncidentDocument> for WorkDocument {
    fn from(value: IncidentDocument) -> Self {
        Self::Incident(value)
    }
}

impl From<LearningRequestDocument> for WorkDocument {
    fn from(value: LearningRequestDocument) -> Self {
        Self::LearningRequest(value)
    }
}

fn validate_optional_identifier(
    field_name: &str,
    value: &Option<String>,
) -> Result<(), WorkDocumentError> {
    if let Some(value) = value {
        validate_safe_identifier(value, field_name)?;
    }
    Ok(())
}

fn validate_required_list(
    field_name: &'static str,
    values: &[String],
) -> Result<(), WorkDocumentError> {
    if values.is_empty() {
        Err(WorkDocumentError::EmptyRequiredList { field_name })
    } else {
        Ok(())
    }
}

fn validate_literal(
    field_name: &'static str,
    value: &str,
    expected: &'static str,
) -> Result<(), WorkDocumentError> {
    if value == expected {
        return Ok(());
    }
    Err(WorkDocumentError::InvalidField {
        field_name,
        value: value.to_owned(),
        message: format!("must be literal `{expected}`"),
    })
}

fn parse_timestamp_for_compare(
    field_name: &'static str,
    value: &str,
) -> Result<OffsetDateTime, WorkDocumentError> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|error| WorkDocumentError::InvalidField {
        field_name,
        value: value.to_owned(),
        message: error.to_string(),
    })
}

fn default_schema_version_string() -> String {
    WORK_DOCUMENT_SCHEMA_VERSION.to_owned()
}

fn default_closure_target_state_kind() -> String {
    "closure_target_state".to_owned()
}

const fn default_true() -> bool {
    true
}
