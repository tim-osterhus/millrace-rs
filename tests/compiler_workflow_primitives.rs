use std::fs;

use millrace_ai::{
    compiler::{
        CompileWorkspaceOptions, CompiledPlanCurrentnessState,
        compile_and_persist_workspace_plan_for_paths, compile_compiled_run_plan,
        export_compiled_stage_graph_at, inspect_workspace_plan_currentness_for_paths,
        resolve_compile_assets,
    },
    contracts::{Plane, Timestamp},
    workspace::{WorkspacePaths, initialize_workspace},
};
use serde_json::Value;
use tempfile::TempDir;

fn fixed_compiled_at() -> Timestamp {
    Timestamp::parse("compiled_at", "2026-05-21T08:30:00Z").unwrap()
}

fn initialized_workspace() -> (TempDir, WorkspacePaths) {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    (temp_dir, paths)
}

fn read_json(path: impl AsRef<std::path::Path>) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn write_json(path: impl AsRef<std::path::Path>, value: &Value) {
    fs::write(path, serde_json::to_string_pretty(value).unwrap() + "\n").unwrap();
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

#[test]
fn compiled_plan_and_graph_export_expose_workflow_primitive_authority() {
    let (_temp_dir, paths) = initialized_workspace();

    let plan =
        compile_compiled_run_plan(&paths, Some("blueprint_codex"), fixed_compiled_at()).unwrap();

    assert_eq!(
        plan.workspace_schema_epoch
            .as_ref()
            .map(|epoch| epoch.epoch_id.as_str()),
        Some("v0.20")
    );
    assert!(plan.pending_compiled_plan.is_none());
    assert!(
        plan.workflow_primitive_fingerprints
            .contains_key("terminal_actions")
    );
    assert!(
        plan.workflow_primitive_fingerprints
            .contains_key("runtime_effect_rules")
    );
    let lane_policy = plan.lane_policy.as_ref().unwrap();
    assert_eq!(lane_policy.policy_id, "compiled.default");
    assert!(lane_policy.lanes.iter().any(|lane| {
        lane.lane_id == "planning.main"
            && lane
                .allowed_family_ids
                .contains(&"blueprint_draft".to_owned())
            && lane.conflict_policy_id.as_deref() == Some("planning.single_writer")
    }));

    let manager = node(&plan.planning_graph, "manager_blueprint");
    assert_eq!(manager.lane_id.as_deref(), Some("planning.main"));
    assert_eq!(
        manager.request_context_profile_id.as_deref(),
        Some("manager_blueprint.default")
    );
    assert_eq!(
        manager.runtime_effect_rule_selections,
        ["manager_blueprint_manifest_to_blueprint_drafts".to_owned()]
    );
    assert_eq!(
        manager
            .terminal_action_mappings
            .get("MANAGER_BLUEPRINT_COMPLETE")
            .map(String::as_str),
        Some("complete_spec_source_after_blueprint_effect")
    );

    let evaluator = node(&plan.planning_graph, "evaluator_blueprint");
    assert_eq!(
        evaluator.request_context_profile_id.as_deref(),
        Some("evaluator_blueprint.default")
    );
    assert!(
        evaluator
            .runtime_effect_rule_selections
            .contains(&"evaluator_blueprint_approved_to_task".to_owned())
    );
    assert_eq!(
        evaluator
            .terminal_action_mappings
            .get("BLUEPRINT_APPROVED")
            .map(String::as_str),
        Some("approve_blueprint_draft_after_effect")
    );

    let export =
        export_compiled_stage_graph_at(&plan, Plane::Planning, fixed_compiled_at()).unwrap();
    assert_eq!(
        export
            .workspace_schema_epoch
            .as_ref()
            .map(|epoch| epoch.epoch_id.as_str()),
        Some("v0.20")
    );
    assert!(export.lane_policy.is_some());
    assert!(
        export
            .workflow_primitive_fingerprints
            .contains_key("request_context_profiles")
    );
    assert!(
        export
            .nodes
            .iter()
            .any(|node| node.node_id == "manager_blueprint"
                && node.lane_id.as_deref() == Some("planning.main")
                && node.request_context_profile_id.as_deref() == Some("manager_blueprint.default"))
    );
}

#[test]
fn workflow_primitive_validation_rejects_bad_authority_references() {
    expect_resolution_error(
        |paths| {
            let path = paths
                .runtime_root
                .join("registry/runtime_effect_rules/planner_effect_rules.json");
            let mut rules = read_json(&path);
            let mut duplicate = rules["definitions"][0].clone();
            duplicate["rule_id"] = Value::String("planner_disposition_duplicate".to_owned());
            rules["definitions"].as_array_mut().unwrap().push(duplicate);
            write_json(path, &rules);
        },
        "duplicate runtime-effect binding",
    );

    expect_resolution_error(
        |paths| {
            let path = paths
                .runtime_root
                .join("registry/runtime_effect_rules/planner_effect_rules.json");
            let mut rules = read_json(&path);
            rules["definitions"][0]["handler_id"] = Value::String("unknown_handler".to_owned());
            write_json(path, &rules);
        },
        "unknown handler",
    );

    expect_resolution_error(
        |paths| {
            let handlers_path = paths
                .runtime_root
                .join("registry/runtime_effect_handlers/default_effect_handlers.json");
            let mut handlers = read_json(&handlers_path);
            let mut handler = handlers["definitions"][0].clone();
            handler["handler_id"] = Value::String("custom_planner_handler".to_owned());
            handlers["definitions"]
                .as_array_mut()
                .unwrap()
                .push(handler);
            write_json(handlers_path, &handlers);

            let rules_path = paths
                .runtime_root
                .join("registry/runtime_effect_rules/planner_effect_rules.json");
            let mut rules = read_json(&rules_path);
            rules["definitions"][0]["handler_id"] =
                Value::String("custom_planner_handler".to_owned());
            write_json(rules_path, &rules);
        },
        "no packaged Rust implementation",
    );

    expect_resolution_error(
        |paths| {
            let path = paths
                .runtime_root
                .join("registry/work_item_families/task.json");
            let mut family = read_json(&path);
            family["document_adapter_id"] = Value::String("missing_adapter".to_owned());
            write_json(path, &family);
        },
        "document adapter",
    );

    expect_resolution_error(
        |paths| {
            let path = paths
                .runtime_root
                .join("registry/workspace_schema_epochs/current.json");
            let mut epoch = read_json(&path);
            epoch["epoch_id"] = Value::String("v0.19".to_owned());
            write_json(path, &epoch);
        },
        "stale workspace schema epoch",
    );

    expect_resolution_error(
        |paths| {
            let path = paths.runtime_root.join("graphs/planning/standard.json");
            let mut graph = read_json(&path);
            graph.as_object_mut().unwrap().remove("completion_behavior");
            write_json(path, &graph);
        },
        "has no completion behavior",
    );

    expect_resolution_error_for_mode(
        "blueprint_codex",
        |paths| {
            let path = paths.runtime_root.join("graphs/planning/blueprint.json");
            let mut graph = read_json(&path);
            graph["entry_nodes"]
                .as_array_mut()
                .unwrap()
                .retain(|entry| entry["entry_key"] != "blueprint_draft");
            write_json(path, &graph);
        },
        "blueprint graph mode reference drift",
    );

    expect_resolution_error(
        |paths| {
            let path = paths
                .runtime_root
                .join("registry/terminal_actions/default_terminal_actions.json");
            let mut actions = read_json(&path);
            actions["definitions"][0]["lifecycle_mutation_plan_id"] =
                Value::String("missing_plan".to_owned());
            write_json(path, &actions);
        },
        "terminal action references unknown lifecycle mutation plan",
    );
}

#[test]
fn compiled_plan_validation_rejects_missing_lane_conflict_policy() {
    let (_temp_dir, paths) = initialized_workspace();
    let mut plan =
        compile_compiled_run_plan(&paths, Some("default_codex"), fixed_compiled_at()).unwrap();
    plan.lane_policy
        .as_mut()
        .unwrap()
        .lane_conflict_policies
        .clear();

    let error = plan.validate().unwrap_err().to_string();
    assert!(error.contains("unknown lane conflict policy"), "{error}");
}

#[test]
fn currentness_detects_workflow_primitive_asset_drift() {
    let (_temp_dir, paths) = initialized_workspace();
    compile_and_persist_workspace_plan_for_paths(
        &paths,
        CompileWorkspaceOptions {
            requested_mode_id: Some("default_codex".to_owned()),
            compiled_at: Some(fixed_compiled_at()),
            ..CompileWorkspaceOptions::default()
        },
    )
    .unwrap();
    let current =
        inspect_workspace_plan_currentness_for_paths(&paths, Some("default_codex")).unwrap();
    assert_eq!(current.state, CompiledPlanCurrentnessState::Current);

    let path = paths
        .runtime_root
        .join("registry/request_context_profiles/default_request_context_profiles.json");
    let mut profiles = read_json(&path);
    profiles["definitions"][0]["visibility_policy"] =
        Value::String("active_item_only_changed".to_owned());
    write_json(path, &profiles);

    let stale =
        inspect_workspace_plan_currentness_for_paths(&paths, Some("default_codex")).unwrap();
    assert_eq!(stale.state, CompiledPlanCurrentnessState::Stale);
}

fn expect_resolution_error(mutator: impl FnOnce(&WorkspacePaths), expected: &str) {
    expect_resolution_error_for_mode("default_codex", mutator, expected);
}

fn expect_resolution_error_for_mode(
    mode_id: &str,
    mutator: impl FnOnce(&WorkspacePaths),
    expected: &str,
) {
    let (_temp_dir, paths) = initialized_workspace();
    mutator(&paths);
    let error = resolve_compile_assets(&paths, Some(mode_id))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(expected),
        "expected error containing {expected:?}, got {error}"
    );
}
