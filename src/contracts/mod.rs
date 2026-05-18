//! Typed contracts shared across Millrace runtime artifacts.

mod capabilities;
mod enums;
mod error;
mod graph_exports;
mod recon;
mod run_trace;
mod runtime_json;
mod stage_metadata;
mod work_documents;

pub use capabilities::{
    ApprovalPolicyRef, BASE_EXECUTION_CAPABILITY_IDS, CapabilityContractError,
    CapabilityPolicyOverride, CapabilityRequest, CapabilityScope, CapabilitySupportDecision,
    ExecutionCapabilityGrant, ExecutionCapabilityWarning, capability_grant_fingerprint,
    capability_key_aliases, is_base_execution_capability_id, normalize_capability_id,
    validate_capability_id,
};
pub use enums::{
    CapabilityDecisionState, CapabilityEnforcementMode, CapabilityEvidenceStatus,
    CapabilityPolicyDecision, CapabilitySupportState, ExecutionStageName, ExecutionTerminalResult,
    IncidentDecision, IncidentSeverity, LearningRequestAction, LearningStageName,
    LearningTerminalResult, LoopEdgeKind, MailboxCommand, Plane, PlanningStageName,
    PlanningTerminalResult, ProbeStatusHint, ReloadOutcome, ResultClass, RootIntakeKind,
    RuntimeErrorCode, RuntimeMode, SpecSourceType, StageName, TaskStatusHint, TerminalResult,
    WatcherMode, WorkItemKind,
};
pub use error::{ContractError, IdentifierErrorReason};
pub use graph_exports::{
    CompiledStageGraphExport, GraphExportContract, GraphExportContractError, GraphExportEdge,
    GraphExportEntry, GraphExportNode, GraphExportTerminalState,
};
pub use recon::{
    ReconConfidence, ReconDecision, ReconHandoffTarget, ReconPacketDocument, ReconPacketError,
    ReconPathFinding, ReconRiskLevel, ReconVerificationPlan,
};
pub use run_trace::{
    RunTraceArtifactRef, RunTraceEdge, RunTraceGraph, RunTraceNode, RunTraceSpawnedWorkKind,
    RunTraceSpawnedWorkRef, RunTraceStatus,
};
pub use runtime_json::{
    ActiveRunRequestKind, ActiveRunState, AutoRecoveryPreRecoverySnapshot,
    BlockedDependencyAutoRecoveryDiagnostic, BlockedItemMetadata, BlockedOrigin,
    BlockedTaskRequeueResult, CompileDiagnostics, FailureClassifierCode, FailureScope,
    LatestOperatorIntervention, MailboxAddIdeaPayload, MailboxAddProbePayload,
    MailboxAddSpecPayload, MailboxAddTaskPayload, MailboxArchiveBlockedTaskPayload,
    MailboxArchiveInvalidIncidentPayload, MailboxCancelWorkItemPayload, MailboxCommandEnvelope,
    MailboxExecutionCapabilityApprovalPayload, MailboxIncidentInterventionPayload,
    MailboxRetargetTaskDependencyPayload, MailboxSupersedeCascade, MailboxSupersedeTaskPayload,
    PauseSource, ReadOnlyStatusPayload, RecoveryCounterEntry, RecoveryCounters, RunnerFailureClass,
    RunnerFailureMetadata, RuntimeErrorContext, RuntimeJsonContract, RuntimeJsonError,
    RuntimeSnapshot, StageResultEnvelope, StrandedBlockedDependency, SubscriptionQuotaStatus,
    SubscriptionQuotaTelemetryState, SubscriptionQuotaWindowReading, TokenUsage,
    UsageGovernanceBlocker, UsageGovernanceBlockerSource, UsageGovernanceDegradedPolicy,
    UsageGovernanceEvaluationBoundary, UsageGovernanceLedgerEntry,
    UsageGovernanceRuntimeTokenMetric, UsageGovernanceRuntimeTokenWindow, UsageGovernanceState,
    UsageGovernanceSubscriptionProvider, UsageGovernanceSubscriptionWindow,
    failure_class_allows_auto_requeue,
};
pub use stage_metadata::{
    OutcomeResultClasses, SAFE_ID_PATTERN_DESCRIPTION, STAGE_ALLOWED_WORK_ITEM_KINDS,
    STAGE_LEGAL_TERMINAL_RESULTS, STAGE_METADATA_BY_VALUE, STAGE_NAME_BY_VALUE, STAGE_TO_PLANE,
    StageMetadata, allowed_result_classes_by_outcome, allowed_work_item_kinds,
    blocked_terminal_for_plane, known_stage_values, known_stage_values_for_plane,
    legal_terminal_markers, legal_terminal_results, parse_terminal_marker_for_plane,
    running_status_marker, stage_allows_work_item_kind, stage_metadata, stage_metadata_for_value,
    stage_name_for_plane, stage_name_for_value, stage_plane, terminal_result_for_plane,
    validate_safe_identifier, validate_stage_result_class, validate_terminal_marker_for_stage,
};
pub use work_documents::{
    ClosureTargetState, IncidentDocument, LearningRequestDocument, ProbeDocument, SpecDocument,
    TaskDocument, Timestamp, WORK_DOCUMENT_SCHEMA_VERSION, WorkDocument, WorkDocumentError,
};
