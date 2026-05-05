//! Public run-trace graph contracts.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{Plane, ResultClass, RuntimeJsonContract, RuntimeJsonError, Timestamp, TokenUsage};

const SCHEMA_VERSION: &str = "1.0";

macro_rules! run_trace_string_enum {
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
            pub fn from_value(value: &str) -> Result<Self, RuntimeJsonError> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(RuntimeJsonError::UnknownValue {
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

run_trace_string_enum! {
    /// Work item kinds that can be spawned from a traced router edge.
    pub enum RunTraceSpawnedWorkKind {
        Task => "task",
        Spec => "spec",
        Incident => "incident",
        LearningRequest => "learning_request",
    }
}

run_trace_string_enum! {
    /// Run-trace graph status values.
    pub enum RunTraceStatus {
        Active => "active",
        Complete => "complete",
        Blocked => "blocked",
        Handoff => "handoff",
        Incomplete => "incomplete",
        Malformed => "malformed",
    }
}

/// Artifact reference attached to one trace node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTraceArtifactRef {
    pub path: String,
    pub kind: String,
    pub size_bytes: Option<u64>,
    pub sha256: Option<String>,
}

impl RunTraceArtifactRef {
    fn validate(&self) -> Result<(), RuntimeJsonError> {
        require_non_blank("artifact.path", &self.path)?;
        require_non_blank("artifact.kind", &self.kind)?;
        require_optional_non_blank("artifact.sha256", &self.sha256)
    }
}

/// Work item spawned as evidence from one routed trace edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTraceSpawnedWorkRef {
    pub kind: RunTraceSpawnedWorkKind,
    pub item_id: String,
    pub path: Option<String>,
    pub reason: Option<String>,
    pub source_stage_node_id: Option<String>,
    pub source_terminal_result: Option<String>,
}

impl RunTraceSpawnedWorkRef {
    fn validate(&self) -> Result<(), RuntimeJsonError> {
        require_non_blank("spawned_work.item_id", &self.item_id)?;
        require_optional_non_blank("spawned_work.path", &self.path)?;
        require_optional_non_blank("spawned_work.reason", &self.reason)?;
        require_optional_non_blank(
            "spawned_work.source_stage_node_id",
            &self.source_stage_node_id,
        )?;
        require_optional_non_blank(
            "spawned_work.source_terminal_result",
            &self.source_terminal_result,
        )
    }
}

/// One stage result represented as a run-trace node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTraceNode {
    pub trace_node_id: String,
    pub run_id: String,
    pub request_id: String,
    pub plane: Plane,
    pub stage: String,
    pub node_id: String,
    pub stage_kind_id: String,
    pub compiled_plan_id: Option<String>,
    pub mode_id: Option<String>,
    pub request_kind: Option<String>,
    pub work_item_kind: Option<String>,
    pub work_item_id: Option<String>,
    pub closure_target_root_spec_id: Option<String>,
    pub terminal_result: String,
    pub result_class: ResultClass,
    pub failure_class: Option<String>,
    pub runner_name: Option<String>,
    pub model_name: Option<String>,
    pub thinking_level: Option<String>,
    pub model_reasoning_effort: Option<String>,
    pub started_at: Timestamp,
    pub completed_at: Timestamp,
    pub duration_seconds: f64,
    pub token_usage: Option<TokenUsage>,
    #[serde(default)]
    pub artifacts: Vec<RunTraceArtifactRef>,
}

impl RunTraceNode {
    fn validate(&mut self) -> Result<(), RuntimeJsonError> {
        require_non_blank("trace_node_id", &self.trace_node_id)?;
        require_non_blank("run_id", &self.run_id)?;
        require_non_blank("request_id", &self.request_id)?;
        require_non_blank("stage", &self.stage)?;
        require_non_blank("node_id", &self.node_id)?;
        require_non_blank("stage_kind_id", &self.stage_kind_id)?;
        require_optional_non_blank("compiled_plan_id", &self.compiled_plan_id)?;
        require_optional_non_blank("mode_id", &self.mode_id)?;
        require_optional_non_blank("request_kind", &self.request_kind)?;
        require_optional_non_blank("work_item_kind", &self.work_item_kind)?;
        require_optional_non_blank("work_item_id", &self.work_item_id)?;
        require_optional_non_blank(
            "closure_target_root_spec_id",
            &self.closure_target_root_spec_id,
        )?;
        require_non_blank("terminal_result", &self.terminal_result)?;
        require_optional_non_blank("failure_class", &self.failure_class)?;
        require_optional_non_blank("runner_name", &self.runner_name)?;
        require_optional_non_blank("model_name", &self.model_name)?;
        require_optional_non_blank("thinking_level", &self.thinking_level)?;
        require_optional_non_blank("model_reasoning_effort", &self.model_reasoning_effort)?;
        require_finite_non_negative("duration_seconds", self.duration_seconds)?;
        if parse_time("completed_at", &self.completed_at)?
            < parse_time("started_at", &self.started_at)?
        {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "run trace node completed_at cannot precede started_at".to_owned(),
            });
        }
        if let Some(token_usage) = &mut self.token_usage {
            token_usage.validate_contract()?;
        }
        for artifact in &self.artifacts {
            artifact.validate()?;
        }
        Ok(())
    }
}

/// One graph-authoritative router decision represented as a trace edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTraceEdge {
    pub trace_edge_id: String,
    pub source_trace_node_id: String,
    pub outcome: String,
    pub edge_kind: String,
    pub target_node_id: Option<String>,
    pub target_trace_node_id: Option<String>,
    pub terminal_state_id: Option<String>,
    #[serde(default)]
    pub spawned_work: Vec<RunTraceSpawnedWorkRef>,
    pub decision_reason: Option<String>,
    pub decided_at: Timestamp,
}

impl RunTraceEdge {
    fn validate(&self) -> Result<(), RuntimeJsonError> {
        require_non_blank("trace_edge_id", &self.trace_edge_id)?;
        require_non_blank("source_trace_node_id", &self.source_trace_node_id)?;
        require_non_blank("outcome", &self.outcome)?;
        require_non_blank("edge_kind", &self.edge_kind)?;
        require_optional_non_blank("target_node_id", &self.target_node_id)?;
        require_optional_non_blank("target_trace_node_id", &self.target_trace_node_id)?;
        require_optional_non_blank("terminal_state_id", &self.terminal_state_id)?;
        require_optional_non_blank("decision_reason", &self.decision_reason)?;
        parse_time("decided_at", &self.decided_at)?;
        for spawned in &self.spawned_work {
            spawned.validate()?;
        }
        Ok(())
    }
}

/// Persisted run-trace graph for one runtime run directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTraceGraph {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_run_trace_graph_kind")]
    pub kind: String,
    pub run_id: String,
    pub run_dir: String,
    pub compiled_plan_id: Option<String>,
    pub mode_id: Option<String>,
    pub request_kind: Option<String>,
    pub work_item_kind: Option<String>,
    pub work_item_id: Option<String>,
    pub closure_target_root_spec_id: Option<String>,
    pub status: RunTraceStatus,
    pub started_at: Option<Timestamp>,
    pub completed_at: Option<Timestamp>,
    pub duration_seconds: Option<f64>,
    #[serde(default)]
    pub nodes: Vec<RunTraceNode>,
    #[serde(default)]
    pub edges: Vec<RunTraceEdge>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub generated_at: Timestamp,
}

impl RuntimeJsonContract for RunTraceGraph {
    const ARTIFACT: &'static str = "run_trace_graph";

    fn validate_contract(&mut self) -> Result<(), RuntimeJsonError> {
        validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
        validate_literal("kind", &self.kind, "run_trace_graph")?;
        require_non_blank("run_id", &self.run_id)?;
        require_non_blank("run_dir", &self.run_dir)?;
        require_optional_non_blank("compiled_plan_id", &self.compiled_plan_id)?;
        require_optional_non_blank("mode_id", &self.mode_id)?;
        require_optional_non_blank("request_kind", &self.request_kind)?;
        require_optional_non_blank("work_item_kind", &self.work_item_kind)?;
        require_optional_non_blank("work_item_id", &self.work_item_id)?;
        require_optional_non_blank(
            "closure_target_root_spec_id",
            &self.closure_target_root_spec_id,
        )?;
        if let Some(duration_seconds) = self.duration_seconds {
            require_finite_non_negative("duration_seconds", duration_seconds)?;
        }
        if let (Some(started_at), Some(completed_at)) = (&self.started_at, &self.completed_at) {
            if parse_time("completed_at", completed_at)? < parse_time("started_at", started_at)? {
                return Err(RuntimeJsonError::InvalidDocument {
                    message: "run trace completed_at cannot precede started_at".to_owned(),
                });
            }
        }
        parse_time("generated_at", &self.generated_at)?;
        for note in &self.notes {
            require_non_blank("notes", note)?;
        }

        for node in &mut self.nodes {
            node.validate()?;
        }
        for edge in &self.edges {
            edge.validate()?;
            if !self
                .nodes
                .iter()
                .any(|node| node.trace_node_id == edge.source_trace_node_id)
            {
                return Err(RuntimeJsonError::InvalidDocument {
                    message: "run trace edge source_trace_node_id must reference a node".to_owned(),
                });
            }
            if let Some(target_trace_node_id) = &edge.target_trace_node_id {
                if !self
                    .nodes
                    .iter()
                    .any(|node| node.trace_node_id == *target_trace_node_id)
                {
                    return Err(RuntimeJsonError::InvalidDocument {
                        message: "run trace edge target_trace_node_id must reference a node"
                            .to_owned(),
                    });
                }
            }
        }
        Ok(())
    }
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

fn require_non_blank(field_name: &'static str, value: &str) -> Result<(), RuntimeJsonError> {
    if value.trim().is_empty() {
        Err(RuntimeJsonError::InvalidField {
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
) -> Result<(), RuntimeJsonError> {
    if let Some(value) = value {
        require_non_blank(field_name, value)?;
    }
    Ok(())
}

fn require_finite_non_negative(
    field_name: &'static str,
    value: f64,
) -> Result<(), RuntimeJsonError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(RuntimeJsonError::InvalidField {
            field_name,
            message: "must be a finite number >= 0".to_owned(),
        })
    }
}

fn parse_time(
    field_name: &'static str,
    timestamp: &Timestamp,
) -> Result<OffsetDateTime, RuntimeJsonError> {
    OffsetDateTime::parse(timestamp.as_str(), &Rfc3339).map_err(|error| {
        RuntimeJsonError::InvalidField {
            field_name,
            message: error.to_string(),
        }
    })
}

fn default_schema_version() -> String {
    SCHEMA_VERSION.to_owned()
}

fn default_run_trace_graph_kind() -> String {
    "run_trace_graph".to_owned()
}
