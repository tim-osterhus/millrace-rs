use std::{any::type_name, collections::HashMap};

use serde_json::{Value, json};

use millrace_ai::{
    compiler::{
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
    },
    contracts::{CompileDiagnostics, LearningRequestAction, LoopEdgeKind, Plane, ResultClass},
};

fn parse_contract<T>(raw: &str) -> T
where
    T: CompilerContract,
{
    T::from_json_str(raw).unwrap()
}

fn fixture_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap()
}

#[test]
fn public_compiler_contract_exports_remain_importable() {
    let names = [
        type_name::<CompiledGraphCompletionEntryPlan>(),
        type_name::<CompiledGraphEntryPlan>(),
        type_name::<CompiledGraphResumePolicyPlan>(),
        type_name::<CompiledGraphThresholdPolicyPlan>(),
        type_name::<CompiledGraphTransitionPlan>(),
        type_name::<CompiledPlanCurrentness>(),
        type_name::<CompiledPlanCurrentnessState>(),
        type_name::<CompiledRunPlan>(),
        type_name::<CompileDiagnostics>(),
        type_name::<CompileInputFingerprint>(),
        type_name::<CompileOutcome>(),
        type_name::<CompilerContractError>(),
        type_name::<FrozenGraphPlanePlan>(),
        type_name::<GraphLoopCompletionBehaviorDefinition>(),
        type_name::<GraphLoopCounterName>(),
        type_name::<GraphLoopDefinition>(),
        type_name::<GraphLoopDynamicPoliciesDefinition>(),
        type_name::<GraphLoopEntryDefinition>(),
        type_name::<GraphLoopEntryKey>(),
        type_name::<GraphLoopNodeDefinition>(),
        type_name::<GraphLoopTerminalClass>(),
        type_name::<GraphLoopTerminalStateDefinition>(),
        type_name::<LearningTriggerRuleDefinition>(),
        type_name::<MaterializedGraphNodePlan>(),
        type_name::<ModeDefinition>(),
        type_name::<PlaneConcurrencyPolicyDefinition>(),
        type_name::<RecoveryRole>(),
        type_name::<RegisteredStageKindDefinition>(),
        type_name::<ResolvedAssetRef>(),
        type_name::<StageIdempotencePolicy>(),
    ];

    assert!(names.iter().all(|name| name.contains("millrace_ai")));
    assert_eq!(GraphLoopEntryKey::ClosureTarget.as_str(), "closure_target");
    assert_eq!(
        GraphLoopCounterName::FixCycleCount.as_str(),
        "fix_cycle_count"
    );
    assert_eq!(CompiledPlanCurrentnessState::Unknown.as_str(), "unknown");
}

#[test]
fn baseline_mode_graph_and_stage_kind_assets_parse_through_contracts() {
    let default_mode: ModeDefinition = parse_contract(include_str!(
        "../src/assets/baseline/modes/default_codex.json"
    ));
    assert_eq!(default_mode.mode_id, "default_codex");
    assert_eq!(
        default_mode
            .loop_ids_by_plane
            .get(&Plane::Execution)
            .map(String::as_str),
        Some("execution.standard")
    );
    assert_eq!(
        default_mode
            .stage_runner_bindings
            .get(&millrace_ai::contracts::StageName::Builder)
            .map(String::as_str),
        Some("codex_cli")
    );
    assert!(default_mode.stage_thinking_bindings.is_empty());

    let learning_mode: ModeDefinition = parse_contract(include_str!(
        "../src/assets/baseline/modes/learning_codex.json"
    ));
    assert!(learning_mode.concurrency_policy.is_some());
    assert_eq!(learning_mode.learning_trigger_rules.len(), 3);
    assert_eq!(
        learning_mode.learning_trigger_rules[0].requested_action,
        LearningRequestAction::Improve
    );

    let execution_graph: GraphLoopDefinition = parse_contract(include_str!(
        "../src/assets/baseline/graphs/execution/standard.json"
    ));
    assert_eq!(execution_graph.loop_id, "execution.standard");
    assert_eq!(execution_graph.nodes[0].stage_kind_id, "builder");
    assert_eq!(execution_graph.edges[0].kind, LoopEdgeKind::Normal);

    let planning_graph: GraphLoopDefinition = parse_contract(include_str!(
        "../src/assets/baseline/graphs/planning/standard.json"
    ));
    assert_eq!(
        planning_graph
            .completion_behavior
            .as_ref()
            .map(|completion| completion.target_node_id.as_str()),
        Some("arbiter")
    );

    let builder_kind: RegisteredStageKindDefinition = parse_contract(include_str!(
        "../src/assets/baseline/registry/stage_kinds/execution/builder.json"
    ));
    assert_eq!(builder_kind.stage_kind_id, "builder");
    assert_eq!(
        builder_kind.idempotence_policy,
        StageIdempotencePolicy::RetrySafeWithKey
    );
    assert_eq!(
        builder_kind.allowed_result_classes_by_outcome["BLOCKED"],
        [ResultClass::Blocked, ResultClass::RecoverableFailure]
    );
    assert!(
        builder_kind
            .allowed_overrides
            .contains(&"thinking_level".to_owned())
    );
}

#[test]
fn compiled_run_plan_fixture_validates_aliases_completion_and_currentness() {
    let plan: CompiledRunPlan = parse_contract(include_str!(
        "fixtures/compiler_contracts/compiled_run_plan_minimal.json"
    ));

    assert_eq!(plan.mode_id, "default_codex");
    assert_eq!(plan.execution_graph.loop_id, "execution.standard");
    assert_eq!(plan.planning_graph.loop_id, "planning.standard");
    assert_eq!(
        plan.execution_graph.nodes[0].thinking_level.as_deref(),
        Some("medium")
    );
    assert_eq!(
        plan.planning_graph
            .compiled_completion_entry
            .as_ref()
            .map(|entry| entry.entry_key),
        Some(GraphLoopEntryKey::ClosureTarget)
    );
    assert_eq!(plan.resolved_assets[0].logical_id, "mode:default_codex");

    let serialized = serde_json::to_value(&plan).unwrap();
    let decoded_again = CompiledRunPlan::from_json_value(serialized).unwrap();
    assert_eq!(decoded_again, plan);

    let currentness = CompiledPlanCurrentness {
        state: CompiledPlanCurrentnessState::Current,
        expected_fingerprint: plan.compile_input_fingerprint.clone(),
        persisted_plan_id: Some(plan.compiled_plan_id.clone()),
        persisted_fingerprint: Some(plan.compile_input_fingerprint.clone()),
    };
    currentness.validate().unwrap();
}

#[test]
fn mode_and_graph_contracts_accept_runner_neutral_thinking_bindings() {
    let mode: ModeDefinition = parse_contract(
        r#"{
  "schema_version": "1.0",
  "kind": "mode",
  "mode_id": "thinking_mode",
  "loop_ids_by_plane": {
    "execution": "execution.standard",
    "planning": "planning.standard"
  },
  "stage_thinking_bindings": {
    "checker": "high",
    "updater": null
  }
}"#,
    );
    assert_eq!(
        mode.stage_thinking_bindings
            .get(&millrace_ai::contracts::StageName::Checker),
        Some(&Some("high".to_owned()))
    );
    assert_eq!(
        mode.stage_thinking_bindings
            .get(&millrace_ai::contracts::StageName::Updater),
        Some(&None)
    );

    let mut graph_value = fixture_value(include_str!(
        "../src/assets/baseline/graphs/execution/standard.json"
    ));
    graph_value["nodes"][0]["thinking_level"] = json!("medium");
    let graph = GraphLoopDefinition::from_json_value(graph_value).unwrap();
    assert_eq!(graph.nodes[0].thinking_level.as_deref(), Some("medium"));
}

#[test]
fn stale_thinking_contract_shapes_are_rejected() {
    let mut blank_mode = fixture_value(include_str!(
        "../src/assets/baseline/modes/default_codex.json"
    ));
    blank_mode["stage_thinking_bindings"] = json!({"builder": " "});
    let error = ModeDefinition::from_json_value(blank_mode).unwrap_err();
    assert!(error.to_string().contains("stage binding"));
}

#[test]
fn required_fields_and_unknown_enums_are_rejected() {
    let mut missing_required = fixture_value(include_str!(
        "../src/assets/baseline/graphs/execution/standard.json"
    ));
    missing_required.as_object_mut().unwrap().remove("loop_id");
    assert!(matches!(
        GraphLoopDefinition::from_json_value(missing_required),
        Err(CompilerContractError::Json { .. })
    ));

    let mut unknown_plane = fixture_value(include_str!(
        "../src/assets/baseline/modes/default_codex.json"
    ));
    unknown_plane["loop_ids_by_plane"]["research"] = json!("research.standard");
    let error = ModeDefinition::from_json_value(unknown_plane).unwrap_err();
    assert!(error.to_string().contains("Plane"));

    let mut unknown_stage_key = fixture_value(include_str!(
        "../src/assets/baseline/modes/default_codex.json"
    ));
    unknown_stage_key["stage_runner_bindings"]["reviewer"] = json!("codex_cli");
    let error = ModeDefinition::from_json_value(unknown_stage_key).unwrap_err();
    assert!(error.to_string().contains("StageName"));

    let mut unknown_entry_key = fixture_value(include_str!(
        "../src/assets/baseline/graphs/execution/standard.json"
    ));
    unknown_entry_key["entry_nodes"][0]["entry_key"] = json!("bug");
    let error = GraphLoopDefinition::from_json_value(unknown_entry_key).unwrap_err();
    assert!(error.to_string().contains("GraphLoopEntryKey"));

    let currentness = json!({
        "state": "fresh",
        "expected_fingerprint": {
            "mode_id": "default_codex",
            "config_fingerprint": "cfg-001",
            "assets_fingerprint": "assets-001"
        },
        "persisted_plan_id": null,
        "persisted_fingerprint": null
    });
    let error = serde_json::from_value::<CompiledPlanCurrentness>(currentness).unwrap_err();
    assert!(error.to_string().contains("CompiledPlanCurrentnessState"));
}

#[test]
fn invalid_references_are_rejected_without_guesswork() {
    let mut bad_entry = fixture_value(include_str!(
        "../src/assets/baseline/graphs/execution/standard.json"
    ));
    bad_entry["entry_nodes"][0]["node_id"] = json!("missing");
    let error = GraphLoopDefinition::from_json_value(bad_entry).unwrap_err();
    assert!(error.to_string().contains("references unknown node_id"));

    let graph: GraphLoopDefinition = parse_contract(include_str!(
        "../src/assets/baseline/graphs/execution/standard.json"
    ));
    let builder_kind: RegisteredStageKindDefinition = parse_contract(include_str!(
        "../src/assets/baseline/registry/stage_kinds/execution/builder.json"
    ));
    let stage_kinds = HashMap::from([(builder_kind.stage_kind_id.clone(), builder_kind)]);
    let error = validate_graph_stage_kind_references(&graph, &stage_kinds).unwrap_err();
    assert!(error.to_string().contains("unknown stage_kind_id"));

    let mut bad_trigger = fixture_value(include_str!(
        "../src/assets/baseline/modes/learning_codex.json"
    ));
    bad_trigger["learning_trigger_rules"][0]["on_terminal_results"] = json!(["NOT_A_RESULT"]);
    let error = ModeDefinition::from_json_value(bad_trigger).unwrap_err();
    assert!(error.to_string().contains("unknown terminal result"));
}
