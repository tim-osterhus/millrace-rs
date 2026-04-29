use std::{collections::HashMap, fs};

use millrace_ai::{
    compiler::{
        CompilerMaterializationError, MISSING_ASSET_TOKEN, build_compiled_plan_id,
        compile_compiled_run_plan, materialize_graph_plane_plan, resolve_compile_assets,
    },
    contracts::{Plane, ResultClass, Timestamp},
    workspace::initialize_workspace,
};
use tempfile::TempDir;

fn fixed_compiled_at() -> Timestamp {
    Timestamp::parse("compiled_at", "2026-04-28T15:30:00Z").unwrap()
}

fn node<'a>(
    graph: &'a millrace_ai::compiler::FrozenGraphPlanePlan,
    node_id: &str,
) -> &'a millrace_ai::compiler::MaterializedGraphNodePlan {
    graph
        .nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .unwrap_or_else(|| panic!("missing node {node_id}"))
}

fn threshold<'a>(
    graph: &'a millrace_ai::compiler::FrozenGraphPlanePlan,
    policy_id: &str,
) -> &'a millrace_ai::compiler::CompiledGraphThresholdPolicyPlan {
    graph
        .compiled_threshold_policies
        .iter()
        .find(|policy| policy.policy_id == policy_id)
        .unwrap_or_else(|| panic!("missing threshold policy {policy_id}"))
}

#[test]
fn default_codex_materializes_execution_and_planning_graphs() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let plan = compile_compiled_run_plan(&paths, None, fixed_compiled_at()).unwrap();

    assert_eq!(plan.mode_id, "default_codex");
    assert!(plan.compiled_plan_id.starts_with("plan-default_codex-"));
    assert_eq!(
        plan.compiled_plan_id,
        build_compiled_plan_id(
            &plan.mode_id,
            &plan.loop_ids_by_plane,
            &plan.graphs_by_plane,
            &plan.concurrency_policy,
            &plan.learning_trigger_rules,
        )
    );
    assert_eq!(plan.execution_graph.loop_id, "execution.standard");
    assert_eq!(plan.planning_graph.loop_id, "planning.standard");
    assert!(plan.learning_graph.is_none());
    assert_eq!(plan.execution_graph.nodes.len(), 7);
    assert_eq!(plan.planning_graph.nodes.len(), 5);
    assert_eq!(
        plan.graphs_by_plane.get(&Plane::Execution),
        Some(&plan.execution_graph)
    );
    assert_eq!(
        plan.graphs_by_plane.get(&Plane::Planning),
        Some(&plan.planning_graph)
    );

    let builder = node(&plan.execution_graph, "builder");
    assert_eq!(builder.stage_kind_id, "builder");
    assert_eq!(builder.entrypoint_path, "entrypoints/execution/builder.md");
    assert_eq!(
        builder.entrypoint_contract_id.as_deref(),
        Some("builder.contract.v1")
    );
    assert_eq!(builder.running_status_marker, "BUILDER_RUNNING");
    assert_eq!(
        builder.allowed_result_classes_by_outcome["BUILDER_COMPLETE"],
        vec![ResultClass::Success]
    );
    assert_eq!(
        builder.allowed_result_classes_by_outcome["BLOCKED"],
        vec![ResultClass::Blocked, ResultClass::RecoverableFailure]
    );
    assert_eq!(
        builder.declared_output_artifacts,
        vec!["stage_result".to_owned(), "report".to_owned()]
    );
    assert_eq!(
        builder.required_skill_paths,
        vec!["skills/stage/execution/builder-core/SKILL.md".to_owned()]
    );
    assert!(builder.attached_skill_additions.is_empty());
    assert_eq!(builder.runner_name.as_deref(), Some("codex_cli"));
    assert_eq!(builder.model_name, None);
    assert_eq!(builder.model_reasoning_effort, None);
    assert_eq!(builder.timeout_seconds, 3600);

    assert!(
        plan.execution_graph
            .compiled_transitions
            .iter()
            .any(|transition| {
                transition.edge_id == "checker-fix-needed-to-fixer"
                    && transition.source_node_id == "checker"
                    && transition.outcome == "FIX_NEEDED"
                    && transition.target_node_id.as_deref() == Some("fixer")
            })
    );
    assert_eq!(
        threshold(&plan.execution_graph, "execution.fix-needed.exhaustion").threshold,
        2
    );
    assert_eq!(
        threshold(&plan.execution_graph, "execution.blocked.recovery").threshold,
        2
    );

    let completion = plan
        .planning_graph
        .compiled_completion_entry
        .as_ref()
        .unwrap();
    assert_eq!(completion.node_id, "arbiter");
    assert_eq!(completion.stage_kind_id, "arbiter");
    assert_eq!(completion.trigger, "backlog_drained");
    assert_eq!(completion.readiness_rule, "no_open_lineage_work");
    assert_eq!(completion.request_kind, "closure_target");
    assert_eq!(completion.target_selector, "active_closure_target");
    assert_eq!(completion.rubric_policy, "reuse_or_create");
    assert_eq!(completion.blocked_work_policy, "suppress");
    assert!(completion.skip_if_already_closed);
    assert_eq!(completion.on_pass_terminal_state_id, "arbiter_complete");
    assert_eq!(completion.on_gap_terminal_state_id, "remediation_needed");
    assert!(completion.create_incident_on_gap);
    assert!(plan.source_refs.contains(&"mode:default_codex".to_owned()));
    assert!(
        plan.source_refs
            .contains(&"graph_completion_behavior:planning.standard".to_owned())
    );
}

#[test]
fn pi_mode_and_standard_plain_alias_materialize_normalized_authority() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let pi = compile_compiled_run_plan(&paths, Some("default_pi"), fixed_compiled_at()).unwrap();
    assert_eq!(pi.mode_id, "default_pi");
    assert!(
        pi.execution_graph
            .nodes
            .iter()
            .chain(pi.planning_graph.nodes.iter())
            .all(|node| node.runner_name.as_deref() == Some("pi_rpc"))
    );

    let alias =
        compile_compiled_run_plan(&paths, Some("standard_plain"), fixed_compiled_at()).unwrap();
    let canonical =
        compile_compiled_run_plan(&paths, Some("default_codex"), fixed_compiled_at()).unwrap();
    assert_eq!(alias.mode_id, "default_codex");
    assert_eq!(alias, canonical);
}

#[test]
fn learning_modes_materialize_learning_graph_triggers_and_concurrency_policy() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let plan =
        compile_compiled_run_plan(&paths, Some("learning_codex"), fixed_compiled_at()).unwrap();

    let learning_graph = plan.learning_graph.as_ref().unwrap();
    assert_eq!(plan.learning_loop_id.as_deref(), Some("learning.standard"));
    assert_eq!(learning_graph.loop_id, "learning.standard");
    assert_eq!(
        learning_graph
            .nodes
            .iter()
            .map(|node| node.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["analyst", "professor", "curator"]
    );
    assert!(
        learning_graph
            .nodes
            .iter()
            .all(|node| node.runner_name.as_deref() == Some("codex_cli"))
    );
    assert_eq!(plan.learning_trigger_rules.len(), 3);
    assert_eq!(
        plan.learning_trigger_rules[0].rule_id,
        "execution.doublechecker.success-to-curator"
    );
    let concurrency = plan.concurrency_policy.as_ref().unwrap();
    assert_eq!(concurrency.mutually_exclusive_planes.len(), 1);
    assert_eq!(concurrency.may_run_concurrently.len(), 2);
    assert_eq!(
        plan.source_refs,
        vec![
            "mode:learning_codex".to_owned(),
            "graph_loop:execution.standard".to_owned(),
            "graph_loop:learning.standard".to_owned(),
            "graph_loop:planning.standard".to_owned(),
            "graph_completion_behavior:planning.standard".to_owned(),
        ]
    );
}

#[test]
fn configured_overrides_thresholds_and_attached_skills_affect_materialized_plan() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    fs::write(
        paths.runtime_root.join("modes/custom_overrides.json"),
        r#"{
  "schema_version": "1.0",
  "kind": "mode",
  "mode_id": "custom_overrides",
  "loop_ids_by_plane": {
    "execution": "execution.standard",
    "planning": "planning.standard"
  },
  "stage_entrypoint_overrides": {
    "builder": "entrypoints/execution/checker.md"
  },
  "stage_skill_additions": {
    "builder": [
      "skills/shared/marathon-qa-audit/SKILL.md",
      "skills/shared/marathon-qa-audit/SKILL.md"
    ]
  },
  "stage_model_bindings": {},
  "stage_runner_bindings": {}
}
"#,
    )
    .unwrap();
    fs::write(
        &paths.runtime_config_file,
        r#"[runners.codex]
model_reasoning_effort = "medium"

[recovery]
max_fix_cycles = 5
max_troubleshoot_attempts_before_consult = 7
max_mechanic_attempts = 3

[stages.builder]
runner = "stage-runner"
model = "stage-model"
model_reasoning_effort = "xhigh"
timeout_seconds = 123
"#,
    )
    .unwrap();

    let plan =
        compile_compiled_run_plan(&paths, Some("custom_overrides"), fixed_compiled_at()).unwrap();
    let builder = node(&plan.execution_graph, "builder");
    assert_eq!(builder.entrypoint_path, "entrypoints/execution/checker.md");
    assert_eq!(
        builder.attached_skill_additions,
        vec!["skills/shared/marathon-qa-audit/SKILL.md".to_owned()]
    );
    assert_eq!(builder.runner_name.as_deref(), Some("stage-runner"));
    assert_eq!(builder.model_name.as_deref(), Some("stage-model"));
    assert_eq!(builder.model_reasoning_effort.as_deref(), Some("xhigh"));
    assert_eq!(builder.timeout_seconds, 123);

    let checker = node(&plan.execution_graph, "checker");
    assert_eq!(checker.runner_name, None);
    assert_eq!(checker.model_reasoning_effort.as_deref(), Some("medium"));
    assert_eq!(
        threshold(&plan.execution_graph, "execution.fix-needed.exhaustion").threshold,
        5
    );
    assert_eq!(
        threshold(&plan.execution_graph, "execution.blocked.recovery").threshold,
        7
    );
    assert_eq!(
        threshold(&plan.planning_graph, "planning.blocked.recovery").threshold,
        3
    );

    let attached_skill = plan
        .resolved_assets
        .iter()
        .find(|asset| {
            asset.logical_id == "skill:skills/shared/marathon-qa-audit/SKILL.md"
                && asset.compile_time_path == "skills/shared/marathon-qa-audit/SKILL.md"
        })
        .unwrap();
    assert_ne!(attached_skill.content_sha256, MISSING_ASSET_TOKEN);
}

#[test]
fn graph_materialization_rejects_unknown_stage_kind_references() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let resolved = resolve_compile_assets(&paths, Some("default_codex")).unwrap();
    let execution_graph = resolved
        .graph_loops
        .iter()
        .find(|graph| graph.plane == Plane::Execution)
        .unwrap();
    let stage_kinds: HashMap<_, _> = resolved
        .stage_kinds
        .iter()
        .filter(|stage_kind| stage_kind.stage_kind_id != "builder")
        .map(|stage_kind| {
            (
                stage_kind.stage_kind_id.clone(),
                stage_kind.definition.clone(),
            )
        })
        .collect();

    let error = materialize_graph_plane_plan(
        &execution_graph.graph_loop,
        &resolved.mode,
        &resolved.config,
        &stage_kinds,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CompilerMaterializationError::UnknownStageKind {
            ref node_id,
            ref stage_kind_id,
        } if node_id == "builder" && stage_kind_id == "builder"
    ));
}

#[test]
fn graph_materialization_rejects_stage_kind_outcome_mismatches() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let resolved = resolve_compile_assets(&paths, Some("default_codex")).unwrap();
    let mut execution_graph = resolved
        .graph_loops
        .iter()
        .find(|graph| graph.plane == Plane::Execution)
        .unwrap()
        .graph_loop
        .clone();
    execution_graph.edges[0].on_outcomes = vec!["CHECKER_PASS".to_owned()];
    let stage_kinds: HashMap<_, _> = resolved
        .stage_kinds
        .iter()
        .map(|stage_kind| {
            (
                stage_kind.stage_kind_id.clone(),
                stage_kind.definition.clone(),
            )
        })
        .collect();

    let error = materialize_graph_plane_plan(
        &execution_graph,
        &resolved.mode,
        &resolved.config,
        &stage_kinds,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CompilerMaterializationError::InvalidStageKindReference {
            ref graph_loop_id,
            ref message,
        } if graph_loop_id == "execution.standard"
            && message.contains("illegal outcome CHECKER_PASS")
            && message.contains("stage kind builder")
    ));
}
