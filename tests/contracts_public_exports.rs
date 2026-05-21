use std::{any::type_name, collections::BTreeSet};

use serde_json::{Value, json};

use millrace_ai::contracts::{
    ActiveRunRequestKind, ActiveRunState, ApprovalPolicyRef, ArtifactContractDefinition,
    ArtifactFilenameAdapterDefinition, ArtifactFormat, BASE_EXECUTION_CAPABILITY_IDS,
    BlueprintCritiqueDocument, BlueprintDraftDocument, BlueprintDraftStatus,
    BlueprintEvaluationDecision, BlueprintEvaluationDocument, BlueprintManifestDocument,
    BlueprintPacketDocument, BlueprintPromotionRecord, BlueprintSourceWorkItemKind,
    CapabilityContractError, CapabilityDecisionState, CapabilityEnforcementMode,
    CapabilityEvidenceStatus, CapabilityPolicyDecision, CapabilityPolicyOverride,
    CapabilityRequest, CapabilityScope, CapabilitySupportDecision, CapabilitySupportState,
    ClosureBlockingWorkRef, CompileDiagnostics, CompiledStageGraphExport, ContractError,
    ExecutionCapabilityGrant, ExecutionCapabilityWarning, ExecutionStageName,
    ExecutionTerminalResult, GraphExportContract, GraphExportContractError, GraphExportEdge,
    GraphExportEntry, GraphExportNode, GraphExportTerminalState, IdentifierErrorReason,
    IncidentDecision, IncidentDocument, IncidentSeverity, LaneConflictPolicyDefinition,
    LaneRuntimeState, LaneRuntimeStatus, LearningRequestAction, LearningRequestDocument,
    LearningStageName, LearningTerminalResult, LifecycleMutationPlanDefinition, LoopEdgeKind,
    MailboxAddIdeaPayload, MailboxAddProbePayload, MailboxArchiveBlockedTaskPayload,
    MailboxArchiveInvalidIncidentPayload, MailboxCancelWorkItemPayload, MailboxCommand,
    MailboxCommandEnvelope, MailboxExecutionCapabilityApprovalPayload,
    MailboxIncidentInterventionPayload, MailboxRetargetTaskDependencyPayload,
    MailboxSupersedeCascade, MailboxSupersedeTaskPayload, OperatorControlCapabilityDefinition,
    OutcomeArtifactDefinition, OutcomeResultClasses, PauseSource, Plane,
    PlaneQueueClaimPolicyDefinition, PlanningStageName, PlanningTerminalResult, ProbeDocument,
    ProbeStatusHint, ReconConfidence, ReconDecision, ReconHandoffTarget, ReconPacketDocument,
    ReconPacketError, ReconPathFinding, ReconRiskLevel, ReconVerificationPlan,
    RecoveryCounterEntry, RecoveryCounters, ReloadOutcome, RequestContextProfileDefinition,
    RequestContextRenderPlan, ResultClass, RootIntakeKind, RuntimeEffectHandlerDefinition,
    RuntimeEffectMutationPhase, RuntimeEffectRuleDefinition, RuntimeErrorCode, RuntimeErrorContext,
    RuntimeFailurePolicyDefinition, RuntimeJsonContract, RuntimeJsonError, RuntimeMode,
    RuntimeSnapshot, SAFE_ID_PATTERN_DESCRIPTION, STAGE_LEGAL_TERMINAL_RESULTS,
    STAGE_METADATA_BY_VALUE, STAGE_NAME_BY_VALUE, STAGE_TO_PLANE, SpecDocument, SpecSourceType,
    StageMetadata, StageName, StageResultEnvelope, TaskDocument, TaskStatusHint,
    TerminalActionDefinition, TerminalResult, Timestamp, TokenUsage, WORK_DOCUMENT_SCHEMA_VERSION,
    WatcherMode, WorkDocument, WorkDocumentError, WorkItemDocumentAdapterDefinition,
    WorkItemFamilyDefinition, WorkItemKind, WorkItemPartitionSelectorDefinition, WorkItemQueueDirs,
    WorkflowCompletionBehaviorDefinition, WorkflowLaneDefinition,
    WorkflowPlaneSchedulerPolicyDefinition, WorkflowPrimitiveBundle,
    WorkflowRecoveryPolicyDefinition, WorkspaceSchemaEpochDefinition,
    allowed_result_classes_by_outcome, blocked_terminal_for_plane, capability_grant_fingerprint,
    capability_key_aliases, coerce_family_and_kind, family_id_for_work_item_kind,
    is_base_execution_capability_id, known_stage_values, known_stage_values_for_plane,
    legacy_work_item_kind_for_family_id, legal_terminal_markers, legal_terminal_results,
    normalize_capability_id, normalize_work_item_family_id, parse_terminal_marker_for_plane,
    plane_for_work_item_family_id, running_status_marker, stage_metadata, stage_metadata_for_value,
    stage_name_for_plane, stage_name_for_value, stage_plane, terminal_result_for_plane,
    validate_capability_id, validate_safe_identifier, validate_stage_result_class,
    validate_terminal_marker_for_stage,
};
use millrace_ai::recon_packets::{parse_recon_packet, read_recon_packet, render_recon_packet};
use millrace_ai::work_documents::{
    parse_task_document, parse_work_document_with_source, render_task_document,
    render_work_document,
};
use millrace_ai::{
    AllowedResultClassPolicy, AllowedResultClassesByOutcome, CodexCliArtifactPaths, CodexCliConfig,
    CodexCliRunnerAdapter, CodexPermissionLevel, CodexProcessError, CodexProcessExecutor,
    CodexProcessRequest, ExecutionCapabilitiesConfig, FakeRunner, FakeRunnerConfig,
    FakeRunnerOutput, FakeRunnerResult, PiEventLogPolicy, PiRpcArtifactPaths,
    PiRpcClientCreateRequest, PiRpcClientError, PiRpcClientFactory, PiRpcConfig, PiRpcJsonlClient,
    PiRpcPromptClient, PiRpcRunnerAdapter, PiRpcSessionResult, PiRpcStreamEvent, PiRpcTransport,
    ProcessExecutionResult, ProcessExitKind, RequestKind, RunnerCompletionArtifact,
    RunnerCompletionArtifactContext, RunnerEnvironmentDelta, RunnerError, RunnerExitKind,
    RunnerInvocationArtifact, RunnerRawResult, RunnerRegistry, RunnerResult, RuntimeStartupSession,
    StageRunRequest, StageRunRequestError, StageRunnerAdapter, StageRunnerDispatcher,
    SubprocessCodexExecutor, SubprocessPiRpcClientFactory, SubprocessPiRpcTransport,
    WorkspaceError, WorkspacePaths, WorkspaceResult, build_codex_cli_command, build_pi_rpc_command,
    build_runtime_runner_dispatcher, build_stage_prompt, codex_cli_artifact_paths,
    completion_artifact_from_raw_result, extract_token_usage, invocation_artifact_from_request,
    materialize_stdout_artifact, normalize_stage_result, permission_flags, persist_event_log,
    persistable_event_lines, pi_rpc_artifact_paths, reconciled_timeout_terminal_marker,
    render_stage_request_context_lines, resolve_permission_level, runner_prompt_path,
    should_persist_event_log, token_usage_from_line, token_usage_from_payload,
    token_usage_from_stats_payload, workspace_paths, write_runner_completion,
    write_runner_invocation, write_stage_prompt_artifact,
};

const NOW: &str = "2026-04-15T00:00:00Z";

type InvocationArtifactBuilder = fn(
    &StageRunRequest,
    String,
    Vec<String>,
    String,
    RunnerEnvironmentDelta,
    String,
    Timestamp,
) -> RunnerResult<RunnerInvocationArtifact>;

fn assert_runtime_contract<T: RuntimeJsonContract>() {}

fn assert_graph_export_contract<T: GraphExportContract>() {}

#[test]
fn public_contract_exports_remain_importable() {
    let public_type_names = [
        type_name::<ActiveRunRequestKind>(),
        type_name::<ActiveRunState>(),
        type_name::<ApprovalPolicyRef>(),
        type_name::<ArtifactContractDefinition>(),
        type_name::<ArtifactFilenameAdapterDefinition>(),
        type_name::<ArtifactFormat>(),
        type_name::<BlueprintCritiqueDocument>(),
        type_name::<BlueprintDraftDocument>(),
        type_name::<BlueprintDraftStatus>(),
        type_name::<BlueprintEvaluationDecision>(),
        type_name::<BlueprintEvaluationDocument>(),
        type_name::<BlueprintManifestDocument>(),
        type_name::<BlueprintPacketDocument>(),
        type_name::<BlueprintPromotionRecord>(),
        type_name::<BlueprintSourceWorkItemKind>(),
        type_name::<CapabilityContractError>(),
        type_name::<CapabilityDecisionState>(),
        type_name::<CapabilityEnforcementMode>(),
        type_name::<CapabilityEvidenceStatus>(),
        type_name::<CapabilityPolicyDecision>(),
        type_name::<CapabilityPolicyOverride>(),
        type_name::<CapabilityRequest>(),
        type_name::<CapabilityScope>(),
        type_name::<CapabilitySupportDecision>(),
        type_name::<CapabilitySupportState>(),
        type_name::<CompileDiagnostics>(),
        type_name::<CompiledStageGraphExport>(),
        type_name::<ContractError>(),
        type_name::<ClosureBlockingWorkRef>(),
        type_name::<ExecutionCapabilityGrant>(),
        type_name::<ExecutionCapabilityWarning>(),
        type_name::<ExecutionStageName>(),
        type_name::<ExecutionTerminalResult>(),
        type_name::<GraphExportContractError>(),
        type_name::<GraphExportEdge>(),
        type_name::<GraphExportEntry>(),
        type_name::<GraphExportNode>(),
        type_name::<GraphExportTerminalState>(),
        type_name::<IdentifierErrorReason>(),
        type_name::<IncidentDecision>(),
        type_name::<IncidentDocument>(),
        type_name::<IncidentSeverity>(),
        type_name::<LaneConflictPolicyDefinition>(),
        type_name::<LaneRuntimeState>(),
        type_name::<LaneRuntimeStatus>(),
        type_name::<LearningRequestAction>(),
        type_name::<LearningRequestDocument>(),
        type_name::<LearningStageName>(),
        type_name::<LearningTerminalResult>(),
        type_name::<LifecycleMutationPlanDefinition>(),
        type_name::<LoopEdgeKind>(),
        type_name::<MailboxAddIdeaPayload>(),
        type_name::<MailboxAddProbePayload>(),
        type_name::<MailboxArchiveBlockedTaskPayload>(),
        type_name::<MailboxArchiveInvalidIncidentPayload>(),
        type_name::<MailboxCancelWorkItemPayload>(),
        type_name::<MailboxCommand>(),
        type_name::<MailboxCommandEnvelope>(),
        type_name::<MailboxExecutionCapabilityApprovalPayload>(),
        type_name::<MailboxIncidentInterventionPayload>(),
        type_name::<MailboxRetargetTaskDependencyPayload>(),
        type_name::<MailboxSupersedeCascade>(),
        type_name::<MailboxSupersedeTaskPayload>(),
        type_name::<OperatorControlCapabilityDefinition>(),
        type_name::<OutcomeArtifactDefinition>(),
        type_name::<OutcomeResultClasses>(),
        type_name::<PauseSource>(),
        type_name::<Plane>(),
        type_name::<PlaneQueueClaimPolicyDefinition>(),
        type_name::<PlanningStageName>(),
        type_name::<PlanningTerminalResult>(),
        type_name::<ProbeDocument>(),
        type_name::<ProbeStatusHint>(),
        type_name::<RecoveryCounterEntry>(),
        type_name::<RecoveryCounters>(),
        type_name::<ReconConfidence>(),
        type_name::<ReconDecision>(),
        type_name::<ReconHandoffTarget>(),
        type_name::<ReconPacketDocument>(),
        type_name::<ReconPacketError>(),
        type_name::<ReconPathFinding>(),
        type_name::<ReconRiskLevel>(),
        type_name::<ReconVerificationPlan>(),
        type_name::<ReloadOutcome>(),
        type_name::<RequestContextProfileDefinition>(),
        type_name::<RequestContextRenderPlan>(),
        type_name::<ResultClass>(),
        type_name::<RootIntakeKind>(),
        type_name::<RuntimeEffectHandlerDefinition>(),
        type_name::<RuntimeEffectMutationPhase>(),
        type_name::<RuntimeEffectRuleDefinition>(),
        type_name::<RuntimeErrorCode>(),
        type_name::<RuntimeErrorContext>(),
        type_name::<RuntimeFailurePolicyDefinition>(),
        type_name::<RuntimeJsonError>(),
        type_name::<RuntimeMode>(),
        type_name::<RuntimeSnapshot>(),
        type_name::<SpecDocument>(),
        type_name::<SpecSourceType>(),
        type_name::<StageMetadata>(),
        type_name::<StageName>(),
        type_name::<StageResultEnvelope>(),
        type_name::<StageRunRequest>(),
        type_name::<StageRunRequestError>(),
        type_name::<TaskDocument>(),
        type_name::<TaskStatusHint>(),
        type_name::<TerminalActionDefinition>(),
        type_name::<TerminalResult>(),
        type_name::<Timestamp>(),
        type_name::<TokenUsage>(),
        type_name::<WatcherMode>(),
        type_name::<WorkspaceError>(),
        type_name::<WorkspacePaths>(),
        type_name::<WorkspaceResult<WorkspacePaths>>(),
        type_name::<WorkDocument>(),
        type_name::<WorkDocumentError>(),
        type_name::<WorkItemDocumentAdapterDefinition>(),
        type_name::<WorkItemFamilyDefinition>(),
        type_name::<WorkItemKind>(),
        type_name::<WorkItemPartitionSelectorDefinition>(),
        type_name::<WorkItemQueueDirs>(),
        type_name::<WorkflowCompletionBehaviorDefinition>(),
        type_name::<WorkflowLaneDefinition>(),
        type_name::<WorkflowPlaneSchedulerPolicyDefinition>(),
        type_name::<WorkflowPrimitiveBundle>(),
        type_name::<WorkflowRecoveryPolicyDefinition>(),
        type_name::<WorkspaceSchemaEpochDefinition>(),
        type_name::<AllowedResultClassPolicy>(),
        type_name::<AllowedResultClassesByOutcome>(),
        type_name::<ExecutionCapabilitiesConfig>(),
        type_name::<CodexCliArtifactPaths>(),
        type_name::<CodexCliConfig>(),
        type_name::<CodexCliRunnerAdapter>(),
        type_name::<CodexPermissionLevel>(),
        type_name::<CodexProcessError>(),
        type_name::<CodexProcessRequest>(),
        type_name::<SubprocessCodexExecutor>(),
        type_name::<FakeRunner>(),
        type_name::<FakeRunnerConfig>(),
        type_name::<FakeRunnerOutput>(),
        type_name::<FakeRunnerResult>(),
        type_name::<PiEventLogPolicy>(),
        type_name::<PiRpcArtifactPaths>(),
        type_name::<PiRpcClientCreateRequest>(),
        type_name::<PiRpcClientError>(),
        type_name::<PiRpcConfig>(),
        type_name::<PiRpcJsonlClient<SubprocessPiRpcTransport>>(),
        type_name::<PiRpcRunnerAdapter>(),
        type_name::<PiRpcSessionResult>(),
        type_name::<PiRpcStreamEvent>(),
        type_name::<SubprocessPiRpcClientFactory>(),
        type_name::<SubprocessPiRpcTransport>(),
        type_name::<RequestKind>(),
        type_name::<ProcessExecutionResult>(),
        type_name::<ProcessExitKind>(),
        type_name::<RunnerCompletionArtifact>(),
        type_name::<RunnerEnvironmentDelta>(),
        type_name::<RunnerError>(),
        type_name::<RunnerExitKind>(),
        type_name::<RunnerInvocationArtifact>(),
        type_name::<RunnerRawResult>(),
        type_name::<RunnerRegistry>(),
        type_name::<RunnerResult<RunnerRawResult>>(),
        type_name::<StageRunnerDispatcher>(),
    ];

    assert!(
        public_type_names
            .iter()
            .all(|name| name.contains("millrace_ai"))
    );

    assert_runtime_contract::<CompileDiagnostics>();
    assert_runtime_contract::<ArtifactContractDefinition>();
    assert_runtime_contract::<BlueprintCritiqueDocument>();
    assert_runtime_contract::<BlueprintDraftDocument>();
    assert_runtime_contract::<BlueprintEvaluationDocument>();
    assert_runtime_contract::<BlueprintManifestDocument>();
    assert_runtime_contract::<BlueprintPacketDocument>();
    assert_runtime_contract::<BlueprintPromotionRecord>();
    assert_runtime_contract::<LaneConflictPolicyDefinition>();
    assert_runtime_contract::<LifecycleMutationPlanDefinition>();
    assert_runtime_contract::<MailboxArchiveBlockedTaskPayload>();
    assert_runtime_contract::<MailboxArchiveInvalidIncidentPayload>();
    assert_runtime_contract::<MailboxCancelWorkItemPayload>();
    assert_runtime_contract::<MailboxCommandEnvelope>();
    assert_runtime_contract::<MailboxExecutionCapabilityApprovalPayload>();
    assert_runtime_contract::<MailboxIncidentInterventionPayload>();
    assert_runtime_contract::<MailboxRetargetTaskDependencyPayload>();
    assert_runtime_contract::<MailboxSupersedeTaskPayload>();
    assert_runtime_contract::<RecoveryCounters>();
    assert_runtime_contract::<RequestContextProfileDefinition>();
    assert_runtime_contract::<RequestContextRenderPlan>();
    assert_runtime_contract::<RuntimeEffectHandlerDefinition>();
    assert_runtime_contract::<RuntimeEffectRuleDefinition>();
    assert_runtime_contract::<RuntimeFailurePolicyDefinition>();
    assert_runtime_contract::<RuntimeSnapshot>();
    assert_runtime_contract::<OperatorControlCapabilityDefinition>();
    assert_runtime_contract::<OutcomeArtifactDefinition>();
    assert_runtime_contract::<PlaneQueueClaimPolicyDefinition>();
    assert_runtime_contract::<TerminalActionDefinition>();
    assert_runtime_contract::<TokenUsage>();
    assert_runtime_contract::<WorkItemDocumentAdapterDefinition>();
    assert_runtime_contract::<WorkItemFamilyDefinition>();
    assert_runtime_contract::<WorkItemPartitionSelectorDefinition>();
    assert_runtime_contract::<WorkflowCompletionBehaviorDefinition>();
    assert_runtime_contract::<WorkflowLaneDefinition>();
    assert_runtime_contract::<WorkflowPlaneSchedulerPolicyDefinition>();
    assert_runtime_contract::<WorkflowRecoveryPolicyDefinition>();
    assert_runtime_contract::<WorkspaceSchemaEpochDefinition>();
    assert_graph_export_contract::<CompiledStageGraphExport>();

    let _adapter: Option<&dyn StageRunnerAdapter> = None;
    let _codex_executor: Option<&dyn CodexProcessExecutor> = None;
    let _pi_factory: Option<&dyn PiRpcClientFactory> = None;
    let _pi_client: Option<&dyn PiRpcPromptClient> = None;
    let _pi_transport: Option<&dyn PiRpcTransport> = None;
    let _dispatcher: fn(RunnerRegistry) -> StageRunnerDispatcher = StageRunnerDispatcher::new;
    let _runtime_dispatcher: fn(&RuntimeStartupSession) -> RunnerResult<StageRunnerDispatcher> =
        build_runtime_runner_dispatcher;
    let _context_renderer: fn(&StageRunRequest) -> Vec<String> = render_stage_request_context_lines;
    let _prompt_builder: fn(&StageRunRequest) -> String = build_stage_prompt;
    let _prompt_path: fn(&StageRunRequest) -> std::path::PathBuf = runner_prompt_path;
    let _prompt_writer: fn(&StageRunRequest) -> RunnerResult<std::path::PathBuf> =
        write_stage_prompt_artifact;
    let _normalizer: fn(&StageRunRequest, &RunnerRawResult) -> RunnerResult<StageResultEnvelope> =
        normalize_stage_result;
    let _invocation_builder: InvocationArtifactBuilder = invocation_artifact_from_request;
    let _completion_builder: fn(
        &StageRunRequest,
        &RunnerRawResult,
        RunnerCompletionArtifactContext,
    ) -> RunnerResult<RunnerCompletionArtifact> = completion_artifact_from_raw_result;
    let _invocation_writer: fn(&std::path::Path, &RunnerInvocationArtifact) -> RunnerResult<()> =
        write_runner_invocation;
    let _completion_writer: fn(&std::path::Path, &RunnerCompletionArtifact) -> RunnerResult<()> =
        write_runner_completion;
    let _codex_command: fn(
        &CodexCliConfig,
        &std::path::Path,
        &StageRunRequest,
        &str,
        &std::path::Path,
    ) -> Vec<String> = build_codex_cli_command;
    let _codex_paths: fn(&std::path::Path, &str) -> CodexCliArtifactPaths =
        codex_cli_artifact_paths;
    let _codex_event_log: fn(
        &std::path::Path,
        &std::path::Path,
    ) -> RunnerResult<Option<std::path::PathBuf>> = persist_event_log;
    let _codex_stdout: fn(
        &std::path::Path,
        &std::path::Path,
        Option<&std::path::Path>,
    ) -> RunnerResult<Option<std::path::PathBuf>> = materialize_stdout_artifact;
    let _codex_reconcile: fn(&StageRunRequest, &std::path::Path) -> Option<String> =
        reconciled_timeout_terminal_marker;
    let _codex_tokens: fn(Option<&std::path::Path>) -> Option<TokenUsage> = extract_token_usage;
    let _codex_token_line: fn(&str) -> Option<TokenUsage> = token_usage_from_line;
    let _codex_token_payload: fn(&serde_json::Value) -> Option<TokenUsage> =
        token_usage_from_payload;
    let _codex_permissions: fn(&CodexCliConfig, &StageRunRequest) -> CodexPermissionLevel =
        resolve_permission_level;
    let _codex_permission_flags: fn(CodexPermissionLevel) -> &'static [&'static str] =
        permission_flags;
    let _pi_command: fn(&PiRpcConfig, &StageRunRequest) -> RunnerResult<Vec<String>> =
        build_pi_rpc_command;
    let _pi_paths: fn(&std::path::Path, &str) -> PiRpcArtifactPaths = pi_rpc_artifact_paths;
    let _pi_persistable: fn(&[String]) -> Vec<String> = persistable_event_lines;
    let _pi_event_policy: fn(PiEventLogPolicy, &PiRpcSessionResult, &[String]) -> bool =
        should_persist_event_log;
    let _pi_stats_tokens: fn(Option<&serde_json::Value>) -> Option<TokenUsage> =
        token_usage_from_stats_payload;
    let _recon_parser: fn(&str) -> Result<ReconPacketDocument, ReconPacketError> =
        parse_recon_packet;
    let _recon_reader: fn(&std::path::Path) -> Result<ReconPacketDocument, ReconPacketError> =
        read_recon_packet;
    let _recon_renderer: fn(&ReconPacketDocument) -> String = render_recon_packet;

    let root = std::env::temp_dir().join("millrace-public-export-paths");
    assert_eq!(
        workspace_paths(&root).runtime_root,
        root.join("millrace-agents")
    );
}

#[test]
fn public_exports_v0_19_0_guardrail_fixture_requires_capability_contract_exports() {
    let fixture: Value = serde_json::from_str(include_str!(
        "fixtures/runtime_json/auto_port_v0_19_0_runtime_contract_scout.json"
    ))
    .expect("parse v0.19.0 runtime contract scout");
    assert_eq!(fixture["kind"], "auto_port_v0_19_0_runtime_contract_scout");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.19.0");
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.4.0");

    let contract = &fixture["execution_capability_contract"];
    let contract_models: BTreeSet<_> = contract["contract_models"]
        .as_array()
        .expect("capability contract models are present")
        .iter()
        .map(|value| value.as_str().expect("contract model"))
        .collect();
    for model in [
        "CapabilityScope",
        "ApprovalPolicyRef",
        "CapabilityRequest",
        "CapabilityPolicyOverride",
        "ExecutionCapabilityGrant",
        "CapabilitySupportDecision",
        "MailboxExecutionCapabilityApprovalPayload",
    ] {
        assert!(
            contract_models.contains(model),
            "missing v0.19.0 public capability contract model {model}"
        );
    }

    let targets: BTreeSet<_> = fixture["expected_rust_contract_targets"]
        .as_array()
        .expect("expected Rust contract targets are present")
        .iter()
        .map(|value| value.as_str().expect("expected Rust target"))
        .collect();
    for target in [
        "src/lib.rs",
        "src/contracts/mod.rs",
        "src/contracts/capabilities.rs",
        "src/contracts/runtime_json.rs",
        "tests/contracts_public_exports.rs",
        "tests/contracts_runtime_json.rs",
    ] {
        assert!(
            targets.contains(target),
            "missing v0.19.0 public export target {target}"
        );
    }

    assert_eq!(
        contract["capability_key_aliases"]["git_mutate"],
        "git.mutate"
    );
    assert!(
        contract["support_states"]
            .as_array()
            .expect("support states are present")
            .iter()
            .any(|value| value.as_str() == Some("partially_supported")),
        "missing v0.19.0 partial support state"
    );
}

#[test]
fn public_exports_v0_20_0_guardrail_fixture_requires_workflow_blueprint_and_context_contract_exports()
 {
    let fixture: Value = serde_json::from_str(include_str!(
        "fixtures/runtime_json/auto_port_v0_20_0_runtime_contract_scout.json"
    ))
    .expect("parse v0.20.0 runtime contract scout");
    assert_eq!(fixture["kind"], "auto_port_v0_20_0_runtime_contract_scout");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.20.0");
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.5.0");

    let workflow = &fixture["workflow_primitive_contract"];
    let contract_models: BTreeSet<_> = workflow["contract_models"]
        .as_array()
        .expect("workflow contract models are present")
        .iter()
        .map(|value| value.as_str().expect("contract model"))
        .collect();
    for model in [
        "WorkflowPrimitiveSet",
        "ArtifactContractDefinition",
        "DocumentAdapterDefinition",
        "WorkItemFamilyDefinition",
        "TerminalActionDefinition",
        "RuntimeEffectRuleDefinition",
        "RequestContextProfileDefinition",
        "WorkspaceSchemaEpochDefinition",
        "LanePolicyDefinition",
        "ContextRenderPlanDefinition",
    ] {
        assert!(
            contract_models.contains(model),
            "missing v0.20.0 public workflow contract model {model}"
        );
    }

    let targets: BTreeSet<_> = fixture["expected_rust_contract_targets"]
        .as_array()
        .expect("expected Rust contract targets are present")
        .iter()
        .map(|value| value.as_str().expect("expected Rust target"))
        .collect();
    for target in [
        "src/lib.rs",
        "src/contracts/mod.rs",
        "src/contracts/runtime_json.rs",
        "src/contracts/workflow_primitives.rs",
        "src/contracts/blueprint.rs",
        "src/contracts/work_refs.rs",
        "tests/contracts_public_exports.rs",
        "tests/contracts_runtime_json.rs",
        "tests/contracts_workflow_primitives.rs",
        "tests/contracts_blueprint.rs",
    ] {
        assert!(
            targets.contains(target),
            "missing v0.20.0 public export target {target}"
        );
    }

    assert_eq!(
        fixture["blueprint_contract"]["mode_ids"],
        json!(["blueprint_codex", "blueprint_learning_codex"])
    );
    assert!(
        fixture["schema_epoch_contract"]["required_behaviors"]
            .as_array()
            .expect("schema epoch behaviors are present")
            .iter()
            .any(|value| value.as_str() == Some("no_stale_json_parse_before_compatibility")),
        "missing v0.20.0 schema epoch safety behavior"
    );
    assert!(
        fixture["lane_request_context_contract"]["inspection_fields"]
            .as_array()
            .expect("inspection fields are present")
            .iter()
            .any(|value| value.as_str() == Some("context_bundle_path")),
        "missing v0.20.0 request-context public inspection field"
    );
}

#[test]
fn public_metadata_helpers_expose_the_stage_contract_boundary() {
    assert_eq!(millrace_ai::PACKAGE_NAME, "millrace-ai");
    assert_eq!(millrace_ai::CRATE_NAME, "millrace_ai");
    assert_eq!(millrace_ai::CLI_NAME, "millrace");
    assert_eq!(millrace_ai::STABILITY, "experimental");
    assert_eq!(
        millrace_ai::runtime_status().version,
        env!("CARGO_PKG_VERSION")
    );

    assert_eq!(SAFE_ID_PATTERN_DESCRIPTION, "^[A-Za-z0-9][A-Za-z0-9._-]*$");
    assert_eq!(WORK_DOCUMENT_SCHEMA_VERSION, "1.0");
    assert!(BASE_EXECUTION_CAPABILITY_IDS.contains(&"workspace.write"));
    assert!(is_base_execution_capability_id("package.install"));
    assert_eq!(
        normalize_capability_id("package_install"),
        "package.install"
    );
    assert_eq!(
        capability_key_aliases()["runtime_control"],
        "runtime.control"
    );
    assert!(validate_capability_id("workspace.write").is_ok());
    let grant: ExecutionCapabilityGrant = serde_json::from_value(json!({
        "grant_id": "grant-public-export",
        "request_id": "request-public-export",
        "capability_id": "evidence.emit",
        "access": "emit",
        "scope": {"kind": "artifact_kind", "value": "stage_result"},
        "decision_state": "granted",
        "enforcement_mode": "advisory_only",
        "decision_reason": "public export guard",
        "resolved_by": "test"
    }))
    .unwrap();
    assert_eq!(grant.fingerprint, capability_grant_fingerprint(&grant));
    assert_eq!(MailboxCommand::CancelWorkItem.as_str(), "cancel_work_item");
    assert_eq!(
        MailboxCommand::ApproveExecutionCapability.as_str(),
        "approve_execution_capability"
    );
    assert_eq!(
        MailboxCommand::from_value("archive_invalid_incident").unwrap(),
        MailboxCommand::ArchiveInvalidIncident
    );
    assert_eq!(MailboxSupersedeCascade::Retarget.as_str(), "retarget");
    assert_eq!(
        MailboxSupersedeCascade::from_value("cancel").unwrap(),
        MailboxSupersedeCascade::Cancel
    );
    assert_eq!(STAGE_METADATA_BY_VALUE.len(), StageName::ALL.len());
    assert_eq!(STAGE_NAME_BY_VALUE.len(), StageName::ALL.len());
    assert_eq!(STAGE_TO_PLANE.len(), StageName::ALL.len());
    assert_eq!(STAGE_LEGAL_TERMINAL_RESULTS.len(), StageName::ALL.len());

    let metadata = stage_metadata(StageName::Builder);
    assert_eq!(metadata.stage, StageName::Builder);
    assert_eq!(stage_metadata_for_value("builder").unwrap(), metadata);
    assert_eq!(stage_plane(StageName::Builder), Plane::Execution);
    assert_eq!(
        stage_name_for_plane(Plane::Execution, "builder").unwrap(),
        StageName::Builder
    );
    assert_eq!(
        stage_name_for_value("librarian").unwrap(),
        StageName::Librarian
    );
    assert_eq!(running_status_marker(StageName::Builder), "BUILDER_RUNNING");
    assert_eq!(
        legal_terminal_markers(StageName::Builder),
        vec!["### BUILDER_COMPLETE".to_owned(), "### BLOCKED".to_owned()]
    );
    assert_eq!(
        legal_terminal_results(StageName::Builder),
        &[
            TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete),
            TerminalResult::Execution(ExecutionTerminalResult::Blocked),
        ]
    );
    assert_eq!(
        allowed_result_classes_by_outcome(StageName::Builder)[0].result_classes,
        &[ResultClass::Success]
    );
    assert_eq!(
        known_stage_values_for_plane(Plane::Learning),
        ["analyst", "professor", "curator", "librarian"]
    );
    assert_eq!(
        known_stage_values_for_plane(Plane::Planning),
        [
            "recon",
            "planner",
            "manager",
            "manager_blueprint",
            "contractor_blueprint",
            "evaluator_blueprint",
            "mechanic",
            "mechanic_blueprint",
            "auditor",
            "arbiter"
        ]
    );
    assert!(known_stage_values().contains(&"builder"));
    assert!(known_stage_values().contains(&"recon"));
    assert_eq!(
        terminal_result_for_plane(Plane::Learning, "BLOCKED").unwrap(),
        blocked_terminal_for_plane(Plane::Learning)
    );
    assert_eq!(
        terminal_result_for_plane(Plane::Learning, "ANALYST_NOOP").unwrap(),
        TerminalResult::Learning(LearningTerminalResult::AnalystNoop)
    );
    assert_eq!(
        terminal_result_for_plane(Plane::Learning, "LIBRARIAN_NOOP").unwrap(),
        TerminalResult::Learning(LearningTerminalResult::LibrarianNoop)
    );
    assert_eq!(
        terminal_result_for_plane(Plane::Planning, "RECON_NOOP").unwrap(),
        TerminalResult::Planning(PlanningTerminalResult::ReconNoop)
    );
    assert_eq!(
        parse_terminal_marker_for_plane(Plane::Execution, "### BLOCKED").unwrap(),
        TerminalResult::Execution(ExecutionTerminalResult::Blocked)
    );
    assert_eq!(
        validate_terminal_marker_for_stage(StageName::Builder, "### BUILDER_COMPLETE").unwrap(),
        TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete)
    );
    validate_stage_result_class(
        StageName::Builder,
        TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete),
        ResultClass::Success,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Analyst,
        TerminalResult::Learning(LearningTerminalResult::AnalystNoop),
        ResultClass::NoOp,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Librarian,
        TerminalResult::Learning(LearningTerminalResult::LibrarianNoop),
        ResultClass::NoOp,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Recon,
        TerminalResult::Planning(PlanningTerminalResult::ReconNoop),
        ResultClass::NoOp,
    )
    .unwrap();
    assert_eq!(
        validate_safe_identifier("task-001.alpha_beta", "task_id").unwrap(),
        "task-001.alpha_beta"
    );
    assert_eq!(
        normalize_work_item_family_id("blueprint_draft", "work_item_family_id").unwrap(),
        "blueprint_draft"
    );
    assert_eq!(
        family_id_for_work_item_kind(WorkItemKind::BlueprintDraft),
        "blueprint_draft"
    );
    assert_eq!(
        legacy_work_item_kind_for_family_id("blueprint_draft").unwrap(),
        Some(WorkItemKind::BlueprintDraft)
    );
    assert_eq!(
        plane_for_work_item_family_id("blueprint_draft")
            .unwrap()
            .unwrap(),
        Plane::Planning
    );
    assert_eq!(
        coerce_family_and_kind(Some("blueprint_draft"), None).unwrap(),
        (
            Some("blueprint_draft".to_owned()),
            Some(WorkItemKind::BlueprintDraft)
        )
    );
}

#[test]
fn public_work_document_helpers_expose_parse_and_render_boundary() {
    let raw = format!(
        "# Public task\n\n\
         Task-ID: public-task\n\
         Title: Public task\n\
         Created-At: {NOW}\n\
         Created-By: tests\n\n\
         Target-Paths:\n\
         - src/contracts/\n\n\
         Acceptance:\n\
         - public helpers parse\n\n\
         Required-Checks:\n\
         - cargo test\n\n\
         References:\n\
         - docs/rust-port-roadmap.md\n\n\
         Risk:\n\
         - export drift\n"
    );

    let task = parse_task_document(&raw).unwrap();
    let rendered_task = render_task_document(&task);
    let document = parse_work_document_with_source(&rendered_task, "public-task.md").unwrap();

    assert_eq!(document.kind(), WorkItemKind::Task);
    assert_eq!(render_work_document(&document), rendered_task);
}

#[test]
fn accepted_and_rejected_public_json_examples_are_deterministic() {
    let mut add_idea = MailboxAddIdeaPayload::from_json_value(json!({
        "source_name": "idea-001.md",
        "markdown": "# Idea\n"
    }))
    .unwrap();
    add_idea.validate().unwrap();

    let bad_payload = MailboxAddIdeaPayload::from_json_value(json!({
        "source_name": "nested/idea.md",
        "markdown": "# Idea\n"
    }))
    .unwrap_err();
    assert!(bad_payload.to_string().contains("single relative filename"));

    let bad_marker =
        validate_terminal_marker_for_stage(StageName::Builder, "### CHECKER_PASS").unwrap_err();
    assert!(matches!(
        bad_marker,
        ContractError::TerminalResultNotAllowed { .. }
    ));
}
