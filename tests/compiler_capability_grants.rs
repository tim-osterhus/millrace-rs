use std::fs;

use millrace_ai::{
    compiler::{
        CompileWorkspaceOptions, CompiledPlanCurrentnessState,
        compile_and_persist_workspace_plan_for_paths, compile_compiled_run_plan,
        inspect_workspace_plan_currentness_for_paths,
    },
    contracts::{CapabilityDecisionState, CapabilityEnforcementMode, Plane, Timestamp},
    workspace::{WorkspacePaths, initialize_workspace},
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn fixed_compiled_at() -> Timestamp {
    Timestamp::parse("compiled_at", "2026-05-18T06:00:00Z").unwrap()
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

fn edit_builder_node(paths: &WorkspacePaths, edit: impl FnOnce(&mut Value)) {
    let graph_path = paths.graphs_dir.join("execution/standard.json");
    let mut graph: Value = serde_json::from_str(&fs::read_to_string(&graph_path).unwrap()).unwrap();
    let builder = graph["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node["node_id"] == "builder")
        .unwrap();
    edit(builder);
    fs::write(
        &graph_path,
        serde_json::to_string_pretty(&graph).unwrap() + "\n",
    )
    .unwrap();
}

fn compile_options(mode_id: &str) -> CompileWorkspaceOptions {
    CompileWorkspaceOptions {
        requested_mode_id: Some(mode_id.to_owned()),
        compiled_at: Some(fixed_compiled_at()),
        ..CompileWorkspaceOptions::default()
    }
}

#[test]
fn default_materialization_seals_framework_grants_and_summaries() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let first =
        compile_compiled_run_plan(&paths, Some("default_codex"), fixed_compiled_at()).unwrap();
    let second =
        compile_compiled_run_plan(&paths, Some("default_codex"), fixed_compiled_at()).unwrap();
    let builder = node(&first.execution_graph, "builder");

    assert_eq!(
        builder
            .execution_capability_grants
            .iter()
            .map(|grant| grant.capability_id.as_str())
            .collect::<Vec<_>>(),
        vec!["runner.invoke", "workspace.read", "artifact.write"]
    );
    let runner = builder
        .execution_capability_grants
        .iter()
        .find(|grant| grant.capability_id == "runner.invoke")
        .unwrap();
    assert_eq!(runner.decision_state, CapabilityDecisionState::Granted);
    assert_eq!(
        runner.enforcement_mode,
        CapabilityEnforcementMode::RuntimeEnforced
    );
    let workspace = builder
        .execution_capability_grants
        .iter()
        .find(|grant| grant.capability_id == "workspace.read")
        .unwrap();
    assert_eq!(workspace.decision_state, CapabilityDecisionState::Granted);
    assert_eq!(
        workspace.enforcement_mode,
        CapabilityEnforcementMode::AdvisoryOnly
    );
    let artifact = builder
        .execution_capability_grants
        .iter()
        .find(|grant| grant.capability_id == "artifact.write")
        .unwrap();
    assert_eq!(artifact.decision_state, CapabilityDecisionState::Granted);
    assert_eq!(
        artifact.enforcement_mode,
        CapabilityEnforcementMode::RuntimeEnforced
    );
    assert!(
        builder
            .execution_capability_grants
            .iter()
            .all(|grant| grant.fingerprint.starts_with("grant-"))
    );
    assert!(
        builder
            .execution_capability_warnings
            .iter()
            .any(|warning| warning.capability_id == "workspace.read"
                && warning.severity == "required_advisory")
    );
    assert!(
        builder
            .execution_capability_policy_fingerprint
            .starts_with("cap-pol-")
    );
    assert_eq!(
        builder.execution_capability_policy_fingerprint,
        node(&second.execution_graph, "builder").execution_capability_policy_fingerprint
    );
    assert_eq!(
        first
            .execution_graph
            .execution_capability_summary
            .total_grants as usize,
        first.execution_graph.nodes.len() * 3
    );
    assert_eq!(
        first
            .execution_graph
            .execution_capability_summary
            .by_enforcement["runtime_enforced"] as usize,
        first.execution_graph.nodes.len() * 2
    );
    assert_eq!(
        first
            .execution_graph
            .execution_capability_summary
            .by_enforcement["advisory_only"] as usize,
        first.execution_graph.nodes.len()
    );
    assert_eq!(
        first.execution_capability_summaries_by_plane[&Plane::Execution],
        first.execution_graph.execution_capability_summary
    );
    assert_eq!(
        first.execution_capability_summary.total_grants,
        first
            .execution_capability_summaries_by_plane
            .values()
            .map(|summary| summary.total_grants)
            .sum::<u64>()
    );
}

#[test]
fn declared_requests_dedupe_and_runtime_config_policy_overrides_mode_policy() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    fs::write(
        paths.modes_dir.join("custom_capabilities.json"),
        r#"{
  "schema_version": "1.0",
  "kind": "mode",
  "mode_id": "custom_capabilities",
  "loop_ids_by_plane": {
    "execution": "execution.standard",
    "planning": "planning.standard"
  },
  "execution_capability_requests": [
    {
      "request_id": "mode-evidence-emit",
      "capability_id": "evidence.emit",
      "access": "write",
      "scope": {"kind": "artifact_kind", "value": "capability_evidence"},
      "reason": "mode requests capability evidence emission"
    }
  ],
  "execution_capability_policies": [
    {
      "capability_id": "network.access",
      "decision": "allow",
      "reason": "mode would allow package index network"
    },
    {
      "capability_id": "package.install",
      "decision": "allow",
      "reason": "mode would allow package installs"
    },
    {
      "capability_id": "evidence.emit",
      "decision": "allow",
      "reason": "mode allows capability evidence emission"
    }
  ]
}
"#,
    )
    .unwrap();
    edit_builder_node(&paths, |builder| {
        builder["execution_capability_requests"] = json!([
            {
                "request_id": "builder-package-install-1",
                "capability_id": "package_install",
                "access": "execute",
                "scope": {"kind": "package_manager", "value": "cargo"},
                "reason": "install build dependency"
            },
            {
                "request_id": "builder-package-install-duplicate",
                "capability_id": "package.install",
                "access": "execute",
                "scope": {"kind": "package_manager", "value": "cargo"},
                "reason": "duplicate package install request"
            },
            {
                "request_id": "builder-network-pypi",
                "capability_id": "network.access",
                "access": "execute",
                "scope": {"kind": "network_class", "value": "package_index"},
                "reason": "reach package index"
            },
            {
                "request_id": "builder-artifact-read",
                "capability_id": "artifact.read",
                "access": "read",
                "scope": {"kind": "artifact_kind", "value": "prior_stage_result"},
                "reason": "read prior stage artifact"
            }
        ]);
        builder["execution_capability_policies"] = json!([
            {
                "capability_id": "artifact.read",
                "decision": "allow",
                "reason": "graph node allows reading prior stage artifacts"
            }
        ]);
    });
    fs::write(
        &paths.runtime_config_file,
        r#"[execution_capabilities.defaults]
network_access = "deny"
package_install = "approval_required"
"#,
    )
    .unwrap();

    let plan = compile_compiled_run_plan(&paths, Some("custom_capabilities"), fixed_compiled_at())
        .unwrap();
    let builder = node(&plan.execution_graph, "builder");

    assert_eq!(builder.execution_capability_grants.len(), 7);
    assert_eq!(
        builder
            .execution_capability_grants
            .iter()
            .filter(|grant| grant.capability_id == "package.install")
            .count(),
        1
    );
    let package_install = builder
        .execution_capability_grants
        .iter()
        .find(|grant| grant.capability_id == "package.install")
        .unwrap();
    assert_eq!(
        package_install.decision_state,
        CapabilityDecisionState::ApprovalRequired
    );
    assert_eq!(package_install.resolved_by, "runtime_config");
    assert!(package_install.approval_policy_ref.is_some());

    let network = builder
        .execution_capability_grants
        .iter()
        .find(|grant| grant.capability_id == "network.access")
        .unwrap();
    assert_eq!(network.decision_state, CapabilityDecisionState::Denied);
    assert_eq!(network.resolved_by, "runtime_config");
    let artifact_read = builder
        .execution_capability_grants
        .iter()
        .find(|grant| grant.capability_id == "artifact.read")
        .unwrap();
    assert_eq!(
        artifact_read.decision_state,
        CapabilityDecisionState::Granted
    );
    assert_eq!(artifact_read.resolved_by, "graph_node");
    let evidence_emit = builder
        .execution_capability_grants
        .iter()
        .find(|grant| grant.capability_id == "evidence.emit")
        .unwrap();
    assert_eq!(
        evidence_emit.decision_state,
        CapabilityDecisionState::Granted
    );
    assert_eq!(evidence_emit.resolved_by, "mode");
    assert!(
        builder
            .execution_capability_warnings
            .iter()
            .any(|warning| warning.severity == "approval_required"
                && warning.capability_id == "package.install")
    );
    assert!(
        builder.execution_capability_warnings.iter().any(
            |warning| warning.severity == "denied" && warning.capability_id == "network.access"
        )
    );
}

#[test]
fn disabled_execution_capabilities_compile_zero_grants_and_warnings() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        "[execution_capabilities]\nenabled = false\n",
    )
    .unwrap();

    let plan =
        compile_compiled_run_plan(&paths, Some("default_codex"), fixed_compiled_at()).unwrap();
    let builder = node(&plan.execution_graph, "builder");

    assert!(builder.execution_capability_grants.is_empty());
    assert!(builder.execution_capability_warnings.is_empty());
    assert_eq!(plan.execution_capability_summary.total_grants, 0);
    assert!(plan.execution_capability_summary.by_decision.is_empty());
    assert!(plan.execution_capability_summary.by_enforcement.is_empty());
    assert!(plan.execution_capability_summaries_by_plane.values().all(
        |summary| summary.total_grants == 0
            && summary.by_decision.is_empty()
            && summary.by_enforcement.is_empty()
    ));
}

#[test]
fn strict_required_advisory_policy_fails_default_plan_with_compile_diagnostics() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        "[execution_capabilities]\nfail_required_advisory = true\n",
    )
    .unwrap();

    let outcome =
        compile_and_persist_workspace_plan_for_paths(&paths, compile_options("default_codex"))
            .unwrap();

    assert!(!outcome.diagnostics.ok);
    assert!(outcome.active_plan.is_none());
    assert!(
        outcome.diagnostics.errors[0].contains("fail_required_advisory"),
        "{:?}",
        outcome.diagnostics.errors
    );
    assert!(outcome.diagnostics.errors[0].contains("workspace.read"));
}

#[test]
fn currentness_treats_execution_capability_policy_changes_as_recompile_boundaries() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let compiled =
        compile_and_persist_workspace_plan_for_paths(&paths, compile_options("default_codex"))
            .unwrap();
    assert!(compiled.diagnostics.ok);
    let current =
        inspect_workspace_plan_currentness_for_paths(&paths, Some("default_codex")).unwrap();
    assert_eq!(current.state, CompiledPlanCurrentnessState::Current);

    fs::write(
        &paths.runtime_config_file,
        r#"[execution_capabilities.defaults]
shell_run = "deny"
"#,
    )
    .unwrap();

    let stale =
        inspect_workspace_plan_currentness_for_paths(&paths, Some("default_codex")).unwrap();
    assert_eq!(stale.state, CompiledPlanCurrentnessState::Stale);
    assert_eq!(stale.persisted_plan_id, compiled.compiled_plan_id);
}
