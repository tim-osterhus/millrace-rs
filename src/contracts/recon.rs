//! Typed Recon packet contracts for probe intake routing.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use super::{ContractError, Timestamp, WORK_DOCUMENT_SCHEMA_VERSION, validate_safe_identifier};

macro_rules! recon_string_enum {
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

            /// Returns the canonical string value used in Recon artifacts.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value,)+
                }
            }

            /// Parses a canonical string value.
            pub fn from_value(value: &str) -> Result<Self, ReconPacketError> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(ReconPacketError::UnknownValue {
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

recon_string_enum! {
    /// Recon routing decision.
    pub enum ReconDecision {
        ToExecution => "to_execution",
        ToPlanning => "to_planning",
        Blocked => "blocked",
        Noop => "noop",
    }
}

impl ReconDecision {
    /// Returns the required handoff target for this decision.
    #[must_use]
    pub const fn handoff_target(self) -> ReconHandoffTarget {
        match self {
            Self::ToExecution => ReconHandoffTarget::Execution,
            Self::ToPlanning => ReconHandoffTarget::Planning,
            Self::Blocked => ReconHandoffTarget::Blocked,
            Self::Noop => ReconHandoffTarget::Noop,
        }
    }
}

recon_string_enum! {
    /// Recon confidence level.
    pub enum ReconConfidence {
        Low => "low",
        Medium => "medium",
        High => "high",
    }
}

recon_string_enum! {
    /// Recon risk level.
    pub enum ReconRiskLevel {
        Low => "low",
        Medium => "medium",
        High => "high",
    }
}

recon_string_enum! {
    /// Recon handoff target derived from a routing decision.
    pub enum ReconHandoffTarget {
        Execution => "execution",
        Planning => "planning",
        Blocked => "blocked",
        Noop => "noop",
    }
}

/// Typed validation and parsing failures for Recon packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconPacketError {
    /// A lower-level contract validation failed.
    Contract(ContractError),
    /// JSON syntax, required-field, type, or enum decoding failed.
    Json {
        /// Artifact type being decoded.
        artifact: &'static str,
        /// Serde error message.
        message: String,
    },
    /// A string value did not match a Recon enum.
    UnknownValue {
        /// Field or enum name.
        field_name: &'static str,
        /// Invalid value.
        value: String,
    },
    /// A required scalar field was missing or blank.
    MissingRequiredField {
        /// Canonical snake_case field name.
        field_name: &'static str,
    },
    /// A required collection was missing or empty.
    EmptyRequiredList {
        /// Canonical snake_case field name.
        field_name: &'static str,
    },
    /// A scalar field failed validation.
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
    /// File IO failed while reading a Recon packet.
    Io {
        /// Path that could not be read.
        path: String,
        /// Human-readable IO error.
        message: String,
    },
}

impl fmt::Display for ReconPacketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => write!(f, "{error}"),
            Self::Json { artifact, message } => {
                write!(f, "failed to decode {artifact}: {message}")
            }
            Self::UnknownValue { field_name, value } => {
                write!(f, "{field_name} has unknown value: {value}")
            }
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

impl std::error::Error for ReconPacketError {}

impl From<ContractError> for ReconPacketError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}

/// One path or test finding in a Recon packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconPathFinding {
    pub path: String,
    pub reason: String,
}

impl ReconPathFinding {
    /// Validates path-finding required fields.
    pub fn validate(&self) -> Result<(), ReconPacketError> {
        require_non_blank("path", &self.path)?;
        require_non_blank("reason", &self.reason)
    }
}

/// Verification commands and fallback checks emitted by Recon.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconVerificationPlan {
    #[serde(default)]
    pub required_commands: Vec<String>,
    #[serde(default)]
    pub focused_checks: Vec<String>,
    #[serde(default)]
    pub fallback_checks: Vec<String>,
}

/// Complete Recon packet document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconPacketDocument {
    #[serde(default = "default_schema_version_string")]
    pub schema_version: String,
    #[serde(default = "default_recon_packet_kind")]
    pub kind: String,
    pub recon_packet_id: String,
    pub probe_id: String,
    pub decision: ReconDecision,
    pub confidence: ReconConfidence,
    pub risk_level: ReconRiskLevel,
    pub request_summary: String,
    pub interpreted_goal: String,
    pub relevant_paths: Vec<ReconPathFinding>,
    #[serde(default)]
    pub relevant_symbols: Vec<String>,
    #[serde(default)]
    pub relevant_tests: Vec<ReconPathFinding>,
    pub semantic_invariants: Vec<String>,
    #[serde(default)]
    pub edge_cases_to_preserve: Vec<String>,
    #[serde(default)]
    pub verification_plan: ReconVerificationPlan,
    #[serde(default)]
    pub open_questions: Vec<String>,
    pub handoff_target: ReconHandoffTarget,
    #[serde(default)]
    pub emitted_task_id: Option<String>,
    #[serde(default)]
    pub emitted_spec_id: Option<String>,
    pub created_at: Timestamp,
    #[serde(default = "default_recon_created_by")]
    pub created_by: String,
}

impl ReconPacketDocument {
    /// Deserializes and validates a Recon packet JSON value.
    pub fn from_json_value(value: Value) -> Result<Self, ReconPacketError> {
        let mut decoded: Self =
            serde_json::from_value(value).map_err(|error| ReconPacketError::Json {
                artifact: "recon_packet",
                message: error.to_string(),
            })?;
        decoded.validate()?;
        Ok(decoded)
    }

    /// Deserializes and validates a Recon packet JSON string.
    pub fn from_json_str(raw: &str) -> Result<Self, ReconPacketError> {
        let value = serde_json::from_str(raw).map_err(|error| ReconPacketError::Json {
            artifact: "recon_packet",
            message: error.to_string(),
        })?;
        Self::from_json_value(value)
    }

    /// Returns the fixed schema version.
    #[must_use]
    pub const fn schema_version(&self) -> &'static str {
        WORK_DOCUMENT_SCHEMA_VERSION
    }

    /// Validates identifier shape, required fields, and handoff invariants.
    pub fn validate(&mut self) -> Result<(), ReconPacketError> {
        validate_literal(
            "schema_version",
            &self.schema_version,
            WORK_DOCUMENT_SCHEMA_VERSION,
        )?;
        validate_literal("kind", &self.kind, "recon_packet")?;
        validate_safe_identifier(&self.recon_packet_id, "recon_packet_id")?;
        validate_safe_identifier(&self.probe_id, "probe_id")?;
        if let Some(emitted_task_id) = &self.emitted_task_id {
            validate_safe_identifier(emitted_task_id, "emitted_task_id")?;
        }
        if let Some(emitted_spec_id) = &self.emitted_spec_id {
            validate_safe_identifier(emitted_spec_id, "emitted_spec_id")?;
        }

        require_non_blank("request_summary", &self.request_summary)?;
        require_non_blank("interpreted_goal", &self.interpreted_goal)?;
        validate_literal("created_by", &self.created_by, "recon")?;
        require_required_list("relevant_paths", &self.relevant_paths)?;
        require_required_list("semantic_invariants", &self.semantic_invariants)?;
        for finding in &self.relevant_paths {
            finding.validate()?;
        }
        for finding in &self.relevant_tests {
            finding.validate()?;
        }
        for invariant in &self.semantic_invariants {
            require_non_blank("semantic_invariants", invariant)?;
        }

        let expected_handoff = self.decision.handoff_target();
        if self.handoff_target != expected_handoff {
            return Err(ReconPacketError::InvalidDocument {
                message: "handoff_target must match decision".to_owned(),
            });
        }
        match self.decision {
            ReconDecision::ToExecution => {
                if self.emitted_task_id.is_none() {
                    return Err(ReconPacketError::InvalidDocument {
                        message: "to_execution decisions require emitted_task_id".to_owned(),
                    });
                }
                if self.emitted_spec_id.is_some() {
                    return Err(ReconPacketError::InvalidDocument {
                        message: "emitted_spec_id is only valid for to_planning decisions"
                            .to_owned(),
                    });
                }
            }
            ReconDecision::ToPlanning => {
                if self.emitted_spec_id.is_none() {
                    return Err(ReconPacketError::InvalidDocument {
                        message: "to_planning decisions require emitted_spec_id".to_owned(),
                    });
                }
                if self.emitted_task_id.is_some() {
                    return Err(ReconPacketError::InvalidDocument {
                        message: "emitted_task_id is only valid for to_execution decisions"
                            .to_owned(),
                    });
                }
            }
            ReconDecision::Blocked | ReconDecision::Noop => {
                if self.emitted_task_id.is_some() {
                    return Err(ReconPacketError::InvalidDocument {
                        message: "emitted_task_id is only valid for to_execution decisions"
                            .to_owned(),
                    });
                }
                if self.emitted_spec_id.is_some() {
                    return Err(ReconPacketError::InvalidDocument {
                        message: "emitted_spec_id is only valid for to_planning decisions"
                            .to_owned(),
                    });
                }
            }
        }
        Ok(())
    }
}

fn require_non_blank(field_name: &'static str, value: &str) -> Result<(), ReconPacketError> {
    if value.trim().is_empty() {
        Err(ReconPacketError::MissingRequiredField { field_name })
    } else {
        Ok(())
    }
}

fn require_required_list<T>(
    field_name: &'static str,
    values: &[T],
) -> Result<(), ReconPacketError> {
    if values.is_empty() {
        Err(ReconPacketError::EmptyRequiredList { field_name })
    } else {
        Ok(())
    }
}

fn validate_literal(
    field_name: &'static str,
    value: &str,
    expected: &'static str,
) -> Result<(), ReconPacketError> {
    if value == expected {
        return Ok(());
    }
    Err(ReconPacketError::InvalidField {
        field_name,
        value: value.to_owned(),
        message: format!("must be literal `{expected}`"),
    })
}

fn default_schema_version_string() -> String {
    WORK_DOCUMENT_SCHEMA_VERSION.to_owned()
}

fn default_recon_packet_kind() -> String {
    "recon_packet".to_owned()
}

fn default_recon_created_by() -> String {
    "recon".to_owned()
}
