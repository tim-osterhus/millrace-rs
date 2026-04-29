use std::fmt;

use super::enums::{Plane, ResultClass, StageName, TerminalResult};

/// Describes why a string failed the Millrace safe identifier contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierErrorReason {
    /// The value had leading or trailing whitespace.
    SurroundingWhitespace,
    /// The value was empty after validation.
    Empty,
    /// The value did not match the allowed identifier character set.
    InvalidCharacters,
}

impl fmt::Display for IdentifierErrorReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SurroundingWhitespace => write!(f, "must not include surrounding whitespace"),
            Self::Empty => write!(f, "is required"),
            Self::InvalidCharacters => {
                write!(
                    f,
                    "must start with an ASCII alphanumeric character and then contain only ASCII alphanumeric characters, '.', '_', or '-'"
                )
            }
        }
    }
}

/// Typed validation failures for contract metadata and identifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    /// A raw value did not match a known enum variant.
    UnknownEnumValue {
        /// Name of the enum being parsed.
        enum_name: &'static str,
        /// Invalid raw value.
        value: String,
    },
    /// A stage value did not match any known stage.
    UnknownStageValue {
        /// Invalid stage value.
        value: String,
    },
    /// A known stage was requested for the wrong plane.
    StagePlaneMismatch {
        /// Plane requested by the caller.
        plane: Plane,
        /// Stage that belongs to a different plane.
        stage: StageName,
    },
    /// A terminal result token is not valid for a plane.
    UnknownTerminalResult {
        /// Plane used for the lookup.
        plane: Plane,
        /// Invalid terminal result token.
        value: String,
    },
    /// A terminal marker did not have the canonical `### OUTCOME` form.
    InvalidTerminalMarker {
        /// Invalid marker value.
        marker: String,
    },
    /// A terminal result is valid in general, but not legal for this stage.
    TerminalResultNotAllowed {
        /// Stage being validated.
        stage: StageName,
        /// Terminal result supplied by the caller.
        terminal_result: TerminalResult,
    },
    /// A terminal result/result-class pair is not legal for this stage.
    ResultClassNotAllowed {
        /// Stage being validated.
        stage: StageName,
        /// Terminal result supplied by the caller.
        terminal_result: TerminalResult,
        /// Result class supplied by the caller.
        result_class: ResultClass,
    },
    /// A string did not satisfy Millrace's safe identifier pattern.
    UnsafeIdentifier {
        /// Field name included in the validation error.
        field_name: String,
        /// Invalid value.
        value: String,
        /// Specific reason validation failed.
        reason: IdentifierErrorReason,
    },
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEnumValue { enum_name, value } => {
                write!(f, "unknown {enum_name} value: {value}")
            }
            Self::UnknownStageValue { value } => write!(f, "unknown stage value: {value}"),
            Self::StagePlaneMismatch { plane, stage } => {
                write!(
                    f,
                    "stage {} does not belong to plane {}",
                    stage.as_str(),
                    plane.as_str()
                )
            }
            Self::UnknownTerminalResult { plane, value } => {
                write!(
                    f,
                    "unknown terminal result for plane {}: {value}",
                    plane.as_str()
                )
            }
            Self::InvalidTerminalMarker { marker } => {
                write!(f, "invalid terminal marker: {marker}")
            }
            Self::TerminalResultNotAllowed {
                stage,
                terminal_result,
            } => write!(
                f,
                "terminal result {} is not legal for stage {}",
                terminal_result.as_str(),
                stage.as_str()
            ),
            Self::ResultClassNotAllowed {
                stage,
                terminal_result,
                result_class,
            } => write!(
                f,
                "result class {} is not legal for stage {} outcome {}",
                result_class.as_str(),
                stage.as_str(),
                terminal_result.as_str()
            ),
            Self::UnsafeIdentifier {
                field_name, reason, ..
            } => write!(f, "{field_name} {reason}"),
        }
    }
}

impl std::error::Error for ContractError {}
