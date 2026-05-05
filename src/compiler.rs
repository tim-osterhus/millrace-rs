//! Compiler-facing contract models and validation helpers.

pub mod assets;
pub mod contracts;
pub mod graph_exports;
pub mod materialization;
pub mod persistence;

pub use assets::{
    CompilerAssetError, CompilerAssetResult, DEFAULT_MODE_ID, EffectiveCodexCompileConfig,
    EffectiveCompileConfig, EffectiveRecoveryCompileConfig, EffectiveRunnersCompileConfig,
    EffectiveRuntimeCompileConfig, EffectiveStageCompileConfig, MISSING_ASSET_TOKEN,
    ResolvedCompileAssetSet, ResolvedGraphLoopAsset, ResolvedStageKindAsset,
    build_compile_input_fingerprint, canonical_mode_id, compile_input_fingerprint_for_workspace,
    fingerprint_effective_compile_config, fingerprint_resolved_assets,
    load_effective_compile_config, load_effective_compile_config_from_path, resolve_compile_assets,
    resolve_compile_assets_with_config, resolve_mode_id,
};
pub use contracts::{
    CompileInputFingerprint, CompileOutcome, CompiledGraphCompletionEntryPlan,
    CompiledGraphEntryPlan, CompiledGraphResumePolicyPlan, CompiledGraphThresholdPolicyPlan,
    CompiledGraphTransitionPlan, CompiledPlanCurrentness, CompiledPlanCurrentnessState,
    CompiledRunPlan, CompilerContract, CompilerContractError, FrozenGraphPlanePlan,
    GraphLoopCompletionBehaviorDefinition, GraphLoopCounterName, GraphLoopDefinition,
    GraphLoopDynamicPoliciesDefinition, GraphLoopEntryDefinition, GraphLoopEntryKey,
    GraphLoopNodeDefinition, GraphLoopTerminalClass, GraphLoopTerminalStateDefinition,
    LearningTriggerRuleDefinition, MaterializedGraphNodePlan, ModeDefinition,
    PlaneConcurrencyPolicyDefinition, RecoveryRole, RegisteredStageKindDefinition,
    ResolvedAssetRef, StageIdempotencePolicy, validate_graph_stage_kind_references,
};
pub use graph_exports::{
    CompilerGraphExportError, CompilerGraphExportResult, export_compiled_stage_graph,
    export_compiled_stage_graph_at, export_compiled_stage_graphs, export_compiled_stage_graphs_at,
};
pub use materialization::{
    CompilerMaterializationError, CompilerMaterializationResult, DEFAULT_STAGE_TIMEOUT_SECONDS,
    build_compiled_plan_id, build_graph_source_refs, compile_compiled_run_plan,
    compile_graph_resume_policies, compile_graph_threshold_policies, compile_graph_transitions,
    materialize_compiled_run_plan, materialize_graph_node_plan, materialize_graph_plane_plan,
    resolved_threshold_for_policy, selected_stages_for_graph_loops, stage_name_for_identifier,
};
pub use persistence::{
    CompileWorkspaceOptions, CompilerPersistenceError, CompilerPersistenceResult,
    compile_and_persist_workspace_plan, compile_and_persist_workspace_plan_for_paths,
    compile_and_persist_workspace_plan_with_options, inspect_workspace_plan_currentness,
    inspect_workspace_plan_currentness_for_paths, load_persisted_compile_diagnostics,
    load_persisted_compiled_plan, save_compile_diagnostics, save_compiled_plan,
};

pub use crate::contracts::CompileDiagnostics;
