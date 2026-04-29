//! Typed contracts shared across Millrace runtime artifacts.

mod enums;
mod error;
mod runtime_json;
mod stage_metadata;
mod work_documents;

pub use enums::{
    ExecutionStageName, ExecutionTerminalResult, IncidentDecision, IncidentSeverity,
    LearningRequestAction, LearningStageName, LearningTerminalResult, LoopEdgeKind, MailboxCommand,
    Plane, PlanningStageName, PlanningTerminalResult, ReloadOutcome, ResultClass, RuntimeErrorCode,
    RuntimeMode, SpecSourceType, StageName, TaskStatusHint, TerminalResult, WatcherMode,
    WorkItemKind,
};
pub use error::{ContractError, IdentifierErrorReason};
pub use runtime_json::{
    ActiveRunRequestKind, ActiveRunState, CompileDiagnostics, MailboxAddIdeaPayload,
    MailboxAddSpecPayload, MailboxAddTaskPayload, MailboxCommandEnvelope, PauseSource,
    RecoveryCounterEntry, RecoveryCounters, RuntimeErrorContext, RuntimeJsonContract,
    RuntimeJsonError, RuntimeSnapshot, StageResultEnvelope, SubscriptionQuotaStatus,
    SubscriptionQuotaTelemetryState, SubscriptionQuotaWindowReading, TokenUsage,
    UsageGovernanceBlocker, UsageGovernanceBlockerSource, UsageGovernanceDegradedPolicy,
    UsageGovernanceEvaluationBoundary, UsageGovernanceLedgerEntry,
    UsageGovernanceRuntimeTokenMetric, UsageGovernanceRuntimeTokenWindow, UsageGovernanceState,
    UsageGovernanceSubscriptionProvider, UsageGovernanceSubscriptionWindow,
};
pub use stage_metadata::{
    OutcomeResultClasses, SAFE_ID_PATTERN_DESCRIPTION, STAGE_LEGAL_TERMINAL_RESULTS,
    STAGE_METADATA_BY_VALUE, STAGE_NAME_BY_VALUE, STAGE_TO_PLANE, StageMetadata,
    allowed_result_classes_by_outcome, blocked_terminal_for_plane, known_stage_values,
    known_stage_values_for_plane, legal_terminal_markers, legal_terminal_results,
    parse_terminal_marker_for_plane, running_status_marker, stage_metadata,
    stage_metadata_for_value, stage_name_for_plane, stage_name_for_value, stage_plane,
    terminal_result_for_plane, validate_safe_identifier, validate_stage_result_class,
    validate_terminal_marker_for_stage,
};
pub use work_documents::{
    ClosureTargetState, IncidentDocument, LearningRequestDocument, SpecDocument, TaskDocument,
    Timestamp, WORK_DOCUMENT_SCHEMA_VERSION, WorkDocument, WorkDocumentError,
};
