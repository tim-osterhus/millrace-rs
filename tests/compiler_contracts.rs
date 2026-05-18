use std::{
    any::type_name,
    collections::{BTreeSet, HashMap},
};

use serde_json::{Value, json};

use millrace_ai::{
    compiler::{
        CompileInputFingerprint, CompileOutcome, CompiledGraphCompletionEntryPlan,
        CompiledGraphEntryPlan, CompiledGraphResumePolicyPlan, CompiledGraphThresholdPolicyPlan,
        CompiledGraphTransitionPlan, CompiledPlanCurrentness, CompiledPlanCurrentnessState,
        CompiledRunPlan, CompilerContract, CompilerContractError, CompilerGraphExportError,
        FrozenGraphPlanePlan, GraphLoopCompletionBehaviorDefinition, GraphLoopCounterName,
        GraphLoopDefinition, GraphLoopDynamicPoliciesDefinition, GraphLoopEntryDefinition,
        GraphLoopEntryKey, GraphLoopNodeDefinition, GraphLoopTerminalClass,
        GraphLoopTerminalStateDefinition, LearningTriggerRuleDefinition, MaterializedGraphNodePlan,
        ModeDefinition, PlaneConcurrencyPolicyDefinition, RecoveryRole,
        RegisteredStageKindDefinition, ResolvedAssetRef, StageIdempotencePolicy,
        export_compiled_stage_graph, export_compiled_stage_graph_at, export_compiled_stage_graphs,
        export_compiled_stage_graphs_at, validate_graph_stage_kind_references,
    },
    contracts::{
        CompileDiagnostics, CompiledStageGraphExport, GraphExportContractError, GraphExportEdge,
        GraphExportEntry, GraphExportNode, GraphExportTerminalState, LearningRequestAction,
        LearningStageName, LoopEdgeKind, Plane, ResultClass, Timestamp,
    },
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
        type_name::<CompilerGraphExportError>(),
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
    assert_eq!(GraphLoopEntryKey::Probe.as_str(), "probe");
    assert_eq!(
        GraphLoopCounterName::FixCycleCount.as_str(),
        "fix_cycle_count"
    );
    assert_eq!(GraphLoopTerminalClass::NoOp.as_str(), "no_op");
    assert_eq!(CompiledPlanCurrentnessState::Unknown.as_str(), "unknown");
    let _graph_exporter: fn(
        &CompiledRunPlan,
    )
        -> Result<Vec<CompiledStageGraphExport>, CompilerGraphExportError> =
        export_compiled_stage_graphs;
    let _graph_exporter_at: fn(
        &CompiledRunPlan,
        Timestamp,
    )
        -> Result<Vec<CompiledStageGraphExport>, CompilerGraphExportError> =
        export_compiled_stage_graphs_at;
    let _single_graph_exporter: fn(
        &CompiledRunPlan,
        Plane,
    )
        -> Result<CompiledStageGraphExport, CompilerGraphExportError> = export_compiled_stage_graph;
    let _single_graph_exporter_at: fn(
        &CompiledRunPlan,
        Plane,
        Timestamp,
    ) -> Result<
        CompiledStageGraphExport,
        CompilerGraphExportError,
    > = export_compiled_stage_graph_at;
}

#[test]
fn compiler_contracts_v0_19_0_guardrail_fixture_requires_capability_request_policy_fields() {
    let fixture: Value = fixture_value(include_str!(
        "fixtures/runtime_json/auto_port_v0_19_0_runtime_contract_scout.json"
    ));
    assert_eq!(fixture["kind"], "auto_port_v0_19_0_runtime_contract_scout");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.19.0");

    let sources: BTreeSet<_> = fixture["contract_sources"]
        .as_array()
        .expect("contract source references are present")
        .iter()
        .map(|value| value.as_str().expect("contract source"))
        .collect();
    for source in [
        "../millrace-py/src/millrace_ai/architecture/loop_graphs.py",
        "../millrace-py/src/millrace_ai/architecture/materialization.py",
        "../millrace-py/src/millrace_ai/architecture/stage_kinds.py",
        "../millrace-py/src/millrace_ai/contracts/modes.py",
        "../millrace-py/src/millrace_ai/compilation/capabilities.py",
    ] {
        assert!(
            sources.contains(source),
            "missing v0.19.0 compiler capability source {source}"
        );
    }

    let targets: BTreeSet<_> = fixture["expected_rust_contract_targets"]
        .as_array()
        .expect("expected Rust contract targets are present")
        .iter()
        .map(|value| value.as_str().expect("expected Rust target"))
        .collect();
    for target in [
        "src/compiler/contracts.rs",
        "src/compiler/materialization.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
    ] {
        assert!(
            targets.contains(target),
            "missing v0.19.0 compiler capability target {target}"
        );
    }

    let compiler = &fixture["compiler_grant_contract"];
    assert_eq!(
        compiler["default_framework_grants"],
        json!(["runner.invoke", "workspace.read", "artifact.write"])
    );
    assert_eq!(
        compiler["request_sources"],
        json!([
            "stage_kind_default",
            "stage_kind",
            "graph_node",
            "mode",
            "runtime_config"
        ])
    );
    assert_eq!(
        compiler["resolution_precedence"],
        json!(["mode", "graph_node", "runtime_config"])
    );
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
    assert_eq!(learning_mode.learning_trigger_rules.len(), 4);
    assert_eq!(
        learning_mode.learning_trigger_rules[0].requested_action,
        LearningRequestAction::Improve
    );
    assert_eq!(
        learning_mode.learning_trigger_rules[0].rule_id,
        "execution.doublechecker.success-to-analyst"
    );
    assert_eq!(
        learning_mode.learning_trigger_rules[0].target_stage,
        LearningStageName::Analyst
    );
    assert!(
        learning_mode.learning_trigger_rules[0]
            .target_skill_id
            .is_none()
    );
    assert!(
        learning_mode.learning_trigger_rules[0]
            .preferred_output_paths
            .is_empty()
    );
    assert_eq!(
        learning_mode
            .stage_runner_bindings
            .get(&millrace_ai::contracts::StageName::Librarian)
            .map(String::as_str),
        Some("codex_cli")
    );
    let librarian_trigger = learning_mode
        .learning_trigger_rules
        .iter()
        .find(|rule| rule.rule_id == "planning.planner.complete-to-librarian")
        .expect("learning_codex includes Planner-to-Librarian trigger");
    assert_eq!(
        librarian_trigger.source_stage,
        millrace_ai::contracts::StageName::Planner
    );
    assert_eq!(librarian_trigger.target_stage, LearningStageName::Librarian);
    assert_eq!(
        librarian_trigger.requested_action,
        LearningRequestAction::Install
    );
    assert_eq!(
        librarian_trigger.on_terminal_results,
        ["PLANNER_COMPLETE".to_owned()]
    );

    let integrated_mode: ModeDefinition = parse_contract(include_str!(
        "../src/assets/baseline/modes/default_codex_integrated.json"
    ));
    assert_eq!(integrated_mode.mode_id, "default_codex_integrated");
    assert_eq!(
        integrated_mode
            .loop_ids_by_plane
            .get(&Plane::Execution)
            .map(String::as_str),
        Some("execution.with_integrator")
    );
    assert_eq!(
        integrated_mode
            .loop_ids_by_plane
            .get(&Plane::Planning)
            .map(String::as_str),
        Some("planning.standard")
    );
    assert_eq!(
        integrated_mode
            .stage_runner_bindings
            .get(&millrace_ai::contracts::StageName::Integrator)
            .map(String::as_str),
        Some("codex_cli")
    );
    assert!(integrated_mode.concurrency_policy.is_none());
    assert!(integrated_mode.learning_trigger_rules.is_empty());
    assert!(
        !integrated_mode
            .stage_runner_bindings
            .contains_key(&millrace_ai::contracts::StageName::Librarian)
    );

    let learning_integrated_mode: ModeDefinition = parse_contract(include_str!(
        "../src/assets/baseline/modes/learning_codex_integrated.json"
    ));
    assert_eq!(
        learning_integrated_mode.mode_id,
        "learning_codex_integrated"
    );
    assert_eq!(
        learning_integrated_mode
            .loop_ids_by_plane
            .get(&Plane::Execution)
            .map(String::as_str),
        Some("execution.with_integrator")
    );
    assert_eq!(
        learning_integrated_mode
            .loop_ids_by_plane
            .get(&Plane::Learning)
            .map(String::as_str),
        Some("learning.standard")
    );
    assert!(learning_integrated_mode.concurrency_policy.is_some());
    assert_eq!(learning_integrated_mode.learning_trigger_rules.len(), 4);
    assert_eq!(
        learning_integrated_mode
            .stage_runner_bindings
            .get(&millrace_ai::contracts::StageName::Integrator)
            .map(String::as_str),
        Some("codex_cli")
    );
    assert_eq!(
        learning_integrated_mode
            .stage_runner_bindings
            .get(&millrace_ai::contracts::StageName::Librarian)
            .map(String::as_str),
        Some("codex_cli")
    );

    let execution_graph: GraphLoopDefinition = parse_contract(include_str!(
        "../src/assets/baseline/graphs/execution/standard.json"
    ));
    assert_eq!(execution_graph.loop_id, "execution.standard");
    assert_eq!(execution_graph.nodes[0].stage_kind_id, "builder");
    assert_eq!(execution_graph.edges[0].kind, LoopEdgeKind::Normal);

    let integrated_execution_graph: GraphLoopDefinition = parse_contract(include_str!(
        "../src/assets/baseline/graphs/execution/with_integrator.json"
    ));
    assert_eq!(
        integrated_execution_graph.loop_id,
        "execution.with_integrator"
    );
    assert!(
        integrated_execution_graph
            .nodes
            .iter()
            .any(|node| node.node_id == "integrator" && node.stage_kind_id == "integrator")
    );
    assert!(integrated_execution_graph.edges.iter().any(|edge| {
        edge.edge_id == "builder-complete-to-integrator"
            && edge.from_node_id == "builder"
            && edge.to_node_id.as_deref() == Some("integrator")
            && edge.on_outcomes == ["BUILDER_COMPLETE"]
    }));
    assert!(integrated_execution_graph.edges.iter().any(|edge| {
        edge.edge_id == "integrator-complete-to-checker"
            && edge.from_node_id == "integrator"
            && edge.to_node_id.as_deref() == Some("checker")
            && edge.on_outcomes == ["INTEGRATION_COMPLETE"]
    }));

    let planning_graph: GraphLoopDefinition = parse_contract(include_str!(
        "../src/assets/baseline/graphs/planning/standard.json"
    ));
    assert_eq!(planning_graph.nodes[0].stage_kind_id, "recon");
    assert_eq!(
        planning_graph.entry_nodes[0].entry_key,
        GraphLoopEntryKey::Probe
    );
    assert_eq!(
        planning_graph
            .completion_behavior
            .as_ref()
            .map(|completion| completion.target_node_id.as_str()),
        Some("arbiter")
    );
    assert!(
        planning_graph
            .terminal_states
            .iter()
            .any(|state| state.terminal_state_id == "recon_to_execution"
                && state.terminal_class == GraphLoopTerminalClass::Success
                && state.writes_status == "RECON_TO_EXECUTION")
    );

    let learning_graph: GraphLoopDefinition = parse_contract(include_str!(
        "../src/assets/baseline/graphs/learning/standard.json"
    ));
    assert!(
        learning_graph
            .nodes
            .iter()
            .any(|node| node.node_id == "librarian" && node.stage_kind_id == "librarian")
    );
    assert!(
        learning_graph
            .terminal_states
            .iter()
            .any(|state| state.terminal_class == GraphLoopTerminalClass::NoOp
                && state.writes_status == "ANALYST_NOOP")
    );
    assert!(
        learning_graph
            .terminal_states
            .iter()
            .any(|state| state.terminal_state_id == "librarian_noop"
                && state.terminal_class == GraphLoopTerminalClass::NoOp
                && state.writes_status == "LIBRARIAN_NOOP")
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

    let integrator_kind: RegisteredStageKindDefinition = parse_contract(include_str!(
        "../src/assets/baseline/registry/stage_kinds/execution/integrator.json"
    ));
    assert_eq!(integrator_kind.stage_kind_id, "integrator");
    assert_eq!(integrator_kind.plane, Plane::Execution);
    assert_eq!(integrator_kind.running_status_marker, "INTEGRATOR_RUNNING");
    assert_eq!(
        integrator_kind.legal_outcomes,
        ["INTEGRATION_COMPLETE", "BLOCKED"]
    );
    assert_eq!(
        integrator_kind.required_skill_paths,
        ["skills/stage/execution/integrator-core/SKILL.md".to_owned()]
    );
    assert_eq!(
        integrator_kind.allowed_result_classes_by_outcome["INTEGRATION_COMPLETE"],
        [ResultClass::Success]
    );
    assert_eq!(
        integrator_kind.allowed_result_classes_by_outcome["BLOCKED"],
        [ResultClass::Blocked, ResultClass::RecoverableFailure]
    );
    assert_eq!(
        integrator_kind.declared_output_artifacts,
        ["stage_result".to_owned(), "integration_report".to_owned()]
    );

    let recon_kind: RegisteredStageKindDefinition = parse_contract(include_str!(
        "../src/assets/baseline/registry/stage_kinds/planning/recon.json"
    ));
    assert_eq!(recon_kind.stage_kind_id, "recon");
    assert_eq!(recon_kind.plane, Plane::Planning);
    assert_eq!(
        recon_kind.required_skill_paths,
        ["skills/stage/planning/recon-core/SKILL.md".to_owned()]
    );
    assert_eq!(
        recon_kind.allowed_result_classes_by_outcome["RECON_NOOP"],
        [ResultClass::NoOp]
    );

    let librarian_kind: RegisteredStageKindDefinition = parse_contract(include_str!(
        "../src/assets/baseline/registry/stage_kinds/learning/librarian.json"
    ));
    assert_eq!(librarian_kind.stage_kind_id, "librarian");
    assert_eq!(librarian_kind.plane, Plane::Learning);
    assert_eq!(librarian_kind.running_status_marker, "LIBRARIAN_RUNNING");
    assert_eq!(
        librarian_kind.required_skill_paths,
        ["skills/stage/learning/librarian-core/SKILL.md".to_owned()]
    );
    assert_eq!(
        librarian_kind.allowed_result_classes_by_outcome["LIBRARIAN_NOOP"],
        [ResultClass::NoOp]
    );
    assert_eq!(
        librarian_kind.declared_output_artifacts,
        ["stage_result".to_owned(), "skill_install_report".to_owned()]
    );
}

#[test]
fn learning_trigger_destination_metadata_normalizes_and_serializes() {
    let mode = ModeDefinition::from_json_value(json!({
        "schema_version": "1.0",
        "kind": "mode",
        "mode_id": "targeted_learning",
        "loop_ids_by_plane": {
            "execution": "execution.standard",
            "planning": "planning.standard",
            "learning": "learning.standard"
        },
        "learning_trigger_rules": [
            {
                "rule_id": "execution.doublechecker.precise-to-curator",
                "source_plane": "execution",
                "source_stage": "doublechecker",
                "on_terminal_results": ["DOUBLECHECK_PASS"],
                "target_stage": "curator",
                "requested_action": "improve",
                "target_skill_id": "doublechecker-core",
                "preferred_output_paths": [
                    " skills/stage/execution/doublechecker-core/SKILL.md ",
                    "skills/stage/execution/doublechecker-core/SKILL.md",
                    "millrace-agents/runs/latest/curator_decision.md"
                ]
            }
        ]
    }))
    .unwrap();

    let rule = &mode.learning_trigger_rules[0];
    assert_eq!(rule.target_skill_id.as_deref(), Some("doublechecker-core"));
    assert_eq!(
        rule.preferred_output_paths,
        [
            "skills/stage/execution/doublechecker-core/SKILL.md".to_owned(),
            "millrace-agents/runs/latest/curator_decision.md".to_owned(),
        ]
    );

    let serialized = serde_json::to_value(rule).unwrap();
    assert_eq!(serialized["target_skill_id"], json!("doublechecker-core"));
    assert_eq!(
        serialized["preferred_output_paths"],
        json!([
            "skills/stage/execution/doublechecker-core/SKILL.md",
            "millrace-agents/runs/latest/curator_decision.md"
        ])
    );

    let single_path_mode = ModeDefinition::from_json_value(json!({
        "schema_version": "1.0",
        "kind": "mode",
        "mode_id": "single_destination",
        "loop_ids_by_plane": {
            "execution": "execution.standard",
            "planning": "planning.standard",
            "learning": "learning.standard"
        },
        "learning_trigger_rules": [
            {
                "rule_id": "execution.doublechecker.path-to-curator",
                "source_plane": "execution",
                "source_stage": "doublechecker",
                "on_terminal_results": ["DOUBLECHECK_PASS"],
                "target_stage": "curator",
                "preferred_output_paths": "skills/stage/execution/doublechecker-core/SKILL.md"
            }
        ]
    }))
    .unwrap();
    assert_eq!(
        single_path_mode.learning_trigger_rules[0].preferred_output_paths,
        ["skills/stage/execution/doublechecker-core/SKILL.md".to_owned()]
    );

    let librarian_mode = ModeDefinition::from_json_value(json!({
        "schema_version": "1.0",
        "kind": "mode",
        "mode_id": "targeted_librarian_learning",
        "loop_ids_by_plane": {
            "execution": "execution.standard",
            "planning": "planning.standard",
            "learning": "learning.standard"
        },
        "learning_trigger_rules": [
            {
                "rule_id": "execution.checker.pass-to-librarian",
                "source_plane": "execution",
                "source_stage": "checker",
                "on_terminal_results": ["CHECKER_PASS"],
                "target_stage": "librarian",
                "requested_action": "install",
                "target_skill_id": "checker-core"
            }
        ]
    }))
    .unwrap();
    assert_eq!(
        librarian_mode.learning_trigger_rules[0].target_stage,
        LearningStageName::Librarian
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
fn compiled_stage_graph_export_contract_matches_python_public_shape() {
    let export = CompiledStageGraphExport {
        schema_version: "1.0".to_owned(),
        kind: "compiled_stage_graph".to_owned(),
        compiled_plan_id: "plan-default_codex-test".to_owned(),
        mode_id: "default_codex".to_owned(),
        loop_id: "execution.standard".to_owned(),
        plane: Plane::Execution,
        nodes: vec![GraphExportNode {
            node_id: "builder".to_owned(),
            plane: Plane::Execution,
            stage_kind_id: "builder".to_owned(),
            entrypoint_path: "entrypoints/execution/builder.md".to_owned(),
            entrypoint_contract_id: Some("builder.contract.v1".to_owned()),
            running_status_marker: "BUILDER_RUNNING".to_owned(),
            required_skill_paths: vec!["skills/stage/execution/builder-core/SKILL.md".to_owned()],
            attached_skill_additions: Vec::new(),
            runner_name: Some("codex_cli".to_owned()),
            model_name: None,
            thinking_level: Some("medium".to_owned()),
            model_reasoning_effort: Some("medium".to_owned()),
            timeout_seconds: 3600,
            allowed_result_classes_by_outcome: HashMap::from([(
                "BUILDER_COMPLETE".to_owned(),
                vec![ResultClass::Success],
            )]),
            declared_output_artifacts: vec!["stage_result".to_owned(), "report".to_owned()],
            execution_capability_grants: Vec::new(),
            execution_capability_warnings: Vec::new(),
            execution_capability_policy_fingerprint: String::new(),
        }],
        edges: vec![GraphExportEdge {
            edge_id: "builder-complete-to-checker".to_owned(),
            source_node_id: "builder".to_owned(),
            outcome: "BUILDER_COMPLETE".to_owned(),
            target_node_id: Some("checker".to_owned()),
            terminal_state_id: None,
            kind: "normal".to_owned(),
            priority: 0,
            max_attempts: None,
        }],
        entries: vec![GraphExportEntry {
            entry_key: "task".to_owned(),
            node_id: "builder".to_owned(),
            stage_kind_id: "builder".to_owned(),
            plane: Plane::Execution,
        }],
        terminal_states: vec![GraphExportTerminalState {
            terminal_state_id: "update_complete".to_owned(),
            terminal_class: "success".to_owned(),
            writes_status: "UPDATER_COMPLETE".to_owned(),
            emits_artifacts: Vec::new(),
            ends_plane_run: true,
        }],
        source_refs: vec![
            "mode:default_codex".to_owned(),
            "graph_loop:execution.standard".to_owned(),
        ],
        exported_at: Timestamp::parse("exported_at", "2026-05-05T12:00:00Z").unwrap(),
    };

    export.validate().unwrap();
    let value = serde_json::to_value(&export).unwrap();
    assert_eq!(value["schema_version"], json!("1.0"));
    assert_eq!(value["kind"], json!("compiled_stage_graph"));
    assert_eq!(
        object_keys(&value),
        vec![
            "compiled_plan_id",
            "edges",
            "entries",
            "exported_at",
            "kind",
            "loop_id",
            "mode_id",
            "nodes",
            "plane",
            "schema_version",
            "source_refs",
            "terminal_states",
        ]
    );
    assert_eq!(
        object_keys(&value["nodes"][0]),
        vec![
            "allowed_result_classes_by_outcome",
            "attached_skill_additions",
            "declared_output_artifacts",
            "entrypoint_contract_id",
            "entrypoint_path",
            "execution_capability_grants",
            "execution_capability_policy_fingerprint",
            "execution_capability_warnings",
            "model_name",
            "model_reasoning_effort",
            "node_id",
            "plane",
            "required_skill_paths",
            "runner_name",
            "running_status_marker",
            "stage_kind_id",
            "thinking_level",
            "timeout_seconds",
        ]
    );
    assert_eq!(
        object_keys(&value["edges"][0]),
        vec![
            "edge_id",
            "kind",
            "max_attempts",
            "outcome",
            "priority",
            "source_node_id",
            "target_node_id",
            "terminal_state_id",
        ]
    );

    let decoded = CompiledStageGraphExport::from_json_value(value).unwrap();
    assert_eq!(decoded, export);
}

#[test]
fn compiled_stage_graph_export_rejects_literal_and_plane_drift() {
    let mut bad_kind = json!({
        "schema_version": "1.0",
        "kind": "compiled_graph",
        "compiled_plan_id": "plan-default_codex-test",
        "mode_id": "default_codex",
        "loop_id": "execution.standard",
        "plane": "execution",
        "nodes": [
            {
                "node_id": "builder",
                "plane": "execution",
                "stage_kind_id": "builder",
                "entrypoint_path": "entrypoints/execution/builder.md",
                "entrypoint_contract_id": "builder.contract.v1",
                "running_status_marker": "BUILDER_RUNNING",
                "required_skill_paths": ["skills/stage/execution/builder-core/SKILL.md"],
                "attached_skill_additions": [],
                "runner_name": "codex_cli",
                "model_name": null,
                "thinking_level": null,
                "model_reasoning_effort": null,
                "timeout_seconds": 3600,
                "allowed_result_classes_by_outcome": {"BUILDER_COMPLETE": ["success"]},
                "declared_output_artifacts": ["stage_result"]
            }
        ],
        "edges": [
            {
                "edge_id": "builder-complete-to-checker",
                "source_node_id": "builder",
                "outcome": "BUILDER_COMPLETE",
                "target_node_id": "checker",
                "terminal_state_id": null,
                "kind": "normal",
                "priority": 0,
                "max_attempts": null
            }
        ],
        "entries": [
            {
                "entry_key": "task",
                "node_id": "builder",
                "stage_kind_id": "builder",
                "plane": "execution"
            }
        ],
        "terminal_states": [
            {
                "terminal_state_id": "update_complete",
                "terminal_class": "success",
                "writes_status": "UPDATER_COMPLETE",
                "emits_artifacts": [],
                "ends_plane_run": true
            }
        ],
        "source_refs": ["mode:default_codex"],
        "exported_at": "2026-05-05T12:00:00Z"
    });
    let error = CompiledStageGraphExport::from_json_value(bad_kind.clone()).unwrap_err();
    assert!(matches!(
        error,
        GraphExportContractError::InvalidLiteral {
            field_name: "kind",
            expected: "compiled_stage_graph",
            ..
        }
    ));

    bad_kind["kind"] = json!("compiled_stage_graph");
    bad_kind["nodes"][0]["plane"] = json!("planning");
    let error = CompiledStageGraphExport::from_json_value(bad_kind).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("nodes must belong to graph plane")
    );
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

fn object_keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<_> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
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

#[test]
fn recon_handoff_outcomes_cannot_route_directly_to_stage_nodes() {
    let mut graph = fixture_value(include_str!(
        "../src/assets/baseline/graphs/planning/standard.json"
    ));
    let edge = graph["edges"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|edge| edge["edge_id"] == "recon-to-execution-to-terminal-recon-to-execution")
        .unwrap();
    edge["to_node_id"] = json!("planner");
    edge["terminal_state_id"] = Value::Null;
    edge["kind"] = json!("normal");

    let error = GraphLoopDefinition::from_json_value(graph).unwrap_err();
    assert!(error.to_string().contains("Recon handoff outcome"));
    assert!(error.to_string().contains("runtime-owned terminal states"));
}
