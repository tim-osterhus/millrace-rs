mod support;

use std::{collections::BTreeSet, fs};

use serde_json::{Map, Value};
use support::parity::{fixture_path, read_fixture, run_rust_millrace};
use tempfile::TempDir;

const CASES: [(&str, &str); 5] = [
    ("default_codex", "default_codex"),
    ("default_pi", "default_pi"),
    ("learning_codex", "learning_codex"),
    ("learning_pi", "learning_pi"),
    ("standard_plain", "default_codex"),
];

#[test]
fn rust_compiler_matches_python_normalized_plan_and_cli_fixtures() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("compiler_parity/python_compiler_parity.json")
            .expect("read compiler parity fixture"),
    )
    .expect("parse compiler parity fixture");

    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["kind"], "python_compiler_parity_fixture");

    for (requested_mode_id, expected_effective_mode_id) in CASES {
        let expected = fixture_case(&fixture, requested_mode_id);
        assert_eq!(expected["effective_mode_id"], expected_effective_mode_id);

        let validate_workspace = TempDir::new().expect("create validate workspace");
        let validate_root = validate_workspace.path().join("workspace");
        init_workspace(&validate_root);
        let validate = run_rust_millrace([
            "compile",
            "validate",
            "--workspace",
            validate_root.to_str().unwrap(),
            "--mode",
            requested_mode_id,
        ])
        .expect("run Rust compile validate");
        validate.assert_success();
        assert_eq!(validate.stderr, "");
        assert_eq!(
            normalize_cli_output(&validate.stdout),
            expected["normalized_validate_output"],
            "normalized compile validate output drifted for {requested_mode_id}",
        );

        let show_workspace = TempDir::new().expect("create show workspace");
        let show_root = show_workspace.path().join("workspace");
        init_workspace(&show_root);
        let show = run_rust_millrace([
            "compile",
            "show",
            "--workspace",
            show_root.to_str().unwrap(),
            "--mode",
            requested_mode_id,
        ])
        .expect("run Rust compile show");
        show.assert_success();
        assert_eq!(show.stderr, "");
        assert_eq!(
            normalize_cli_output(&show.stdout),
            expected["normalized_show_output"],
            "normalized compile show output drifted for {requested_mode_id}",
        );

        let persisted_plan_path = show_root
            .join("millrace-agents")
            .join("state")
            .join("compiled_plan.json");
        let persisted_plan: Value = serde_json::from_str(
            &fs::read_to_string(&persisted_plan_path).expect("read persisted compiled plan"),
        )
        .expect("parse persisted compiled plan");
        assert_fingerprint_shapes(&persisted_plan);
        assert_eq!(
            normalize_plan(persisted_plan),
            expected["normalized_plan"],
            "normalized compiled_plan.json drifted for {requested_mode_id}",
        );
    }
}

#[test]
fn rust_compile_graph_cli_exports_python_v0_18_1_recon_graph_shape() {
    let temp_dir = TempDir::new().expect("create compile graph workspace");
    let root = temp_dir.path().join("workspace");
    init_workspace(&root);

    let output = run_rust_millrace([
        "compile",
        "graph",
        "--workspace",
        root.to_str().unwrap(),
        "--mode",
        "learning_codex",
        "--format",
        "json",
    ])
    .expect("run Rust compile graph");
    output.assert_success();
    assert_eq!(output.stderr, "");

    let graphs: Vec<Value> = serde_json::from_str(&output.stdout).expect("parse graph JSON");
    assert_eq!(graphs.len(), 3);
    assert_eq!(graphs[0]["kind"], "compiled_stage_graph");
    assert_eq!(graphs[0]["plane"], "execution");
    assert_eq!(graphs[1]["plane"], "learning");
    assert_eq!(graphs[2]["plane"], "planning");
    assert_eq!(graphs[0]["nodes"][0]["node_id"], "builder");
    assert_eq!(graphs[1]["nodes"][0]["node_id"], "analyst");
    assert_eq!(graphs[2]["nodes"][0]["node_id"], "recon");
    assert!(
        graphs[2]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["entry_key"] == "probe" && entry["node_id"] == "recon" })
    );
    assert!(graphs[2]["edges"].as_array().unwrap().iter().any(|edge| {
        edge["source_node_id"] == "recon"
            && edge["outcome"] == "RECON_TO_EXECUTION"
            && edge["terminal_state_id"] == "recon_to_execution"
    }));
    assert!(graphs[0]["edges"].as_array().unwrap().iter().any(|edge| {
        edge["source_node_id"] == "builder"
            && edge["outcome"] == "BUILDER_COMPLETE"
            && edge["target_node_id"] == "checker"
    }));

    let integrated_output = run_rust_millrace([
        "compile",
        "graph",
        "--workspace",
        root.to_str().unwrap(),
        "--mode",
        "default_codex_integrated",
        "--plane",
        "execution",
        "--format",
        "json",
    ])
    .expect("run Rust integrated compile graph");
    integrated_output.assert_success();
    assert_eq!(integrated_output.stderr, "");
    let integrated_graphs: Vec<Value> =
        serde_json::from_str(&integrated_output.stdout).expect("parse integrated graph JSON");
    assert_eq!(integrated_graphs.len(), 1);
    let execution = &integrated_graphs[0];
    assert_eq!(execution["mode_id"], "default_codex_integrated");
    assert_eq!(execution["loop_id"], "execution.with_integrator");
    assert!(execution["nodes"].as_array().unwrap().iter().any(|node| {
        node["node_id"] == "integrator"
            && node["stage_kind_id"] == "integrator"
            && node["runner_name"] == "codex_cli"
    }));
    assert!(execution["edges"].as_array().unwrap().iter().any(|edge| {
        edge["source_node_id"] == "builder"
            && edge["outcome"] == "BUILDER_COMPLETE"
            && edge["target_node_id"] == "integrator"
    }));
    assert!(execution["edges"].as_array().unwrap().iter().any(|edge| {
        edge["source_node_id"] == "integrator"
            && edge["outcome"] == "INTEGRATION_COMPLETE"
            && edge["target_node_id"] == "checker"
    }));
}

#[test]
fn compiler_parity_fixture_documents_regeneration_surface() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("compiler_parity/python_compiler_parity.json")
            .expect("read compiler parity fixture"),
    )
    .expect("parse compiler parity fixture");
    assert_eq!(fixture["source"]["previous_version"], "0.18.0");
    assert_eq!(fixture["source"]["target_version"], "0.18.1");
    assert_eq!(fixture["source"]["version"], "0.18.1");
    assert_eq!(fixture["source"]["previous_tag"], "v0.18.0");
    assert_eq!(
        fixture["source"]["previous_commit"],
        "e4ccf099c8345a8b8708cdaa1ac510bdc7851387"
    );
    assert_eq!(fixture["source"]["target_tag"], "v0.18.1");
    assert_eq!(
        fixture["source"]["target_commit"],
        "0396c7852793b212d31345862b38a7d6f3f02854"
    );
    assert_eq!(fixture["source"]["diff_range"], "v0.18.0..v0.18.1");
    assert_ne!(
        fixture["source"]["target_version"], fixture["source"]["previous_version"],
        "compiler parity fixture is still pinned to the previous Python baseline as target",
    );
    for source_path in [
        "src/millrace_ai/config/models.py",
        "src/millrace_ai/contracts/modes.py",
        "src/millrace_ai/contracts/stage_metadata.py",
        "src/millrace_ai/architecture/loop_graphs.py",
        "src/millrace_ai/assets/entrypoints/planning/recon.md",
        "src/millrace_ai/assets/graphs/planning/standard.json",
        "src/millrace_ai/assets/registry/stage_kinds/planning/recon.json",
        "src/millrace_ai/assets/skills/stage/planning/recon-core/SKILL.md",
        "src/millrace_ai/cli/commands/compile.py",
        "src/millrace_ai/cli/formatting.py",
        "src/millrace_ai/compilation/graph_exports.py",
        "src/millrace_ai/compilation/learning_triggers.py",
        "src/millrace_ai/compilation/node_materialization.py",
        "src/millrace_ai/cli/compile_view.py",
        "src/millrace_ai/contracts/graph_exports.py",
        "tests/config/test_config.py",
        "tests/cli/test_graph_trace_cli.py",
        "tests/assets/test_loop_graphs.py",
        "tests/assets/test_modes.py",
        "tests/assets/test_stage_kinds.py",
        "tests/integration/test_compiler.py",
        "tests/integration/test_graph_exports.py",
    ] {
        assert!(
            fixture["source"]["contract_sources"]
                .as_array()
                .expect("fixture source paths must be an array")
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "compiler parity fixture does not name Python source {source_path}",
        );
    }

    assert!(fixture_path("compiler_parity/README.md").is_file());
    assert!(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/support/generate_python_compiler_parity_fixtures.py")
            .is_file()
    );
}

#[test]
fn compiler_parity_scout_pins_python_v0_18_1_recon_assets_and_graph_sources() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("compiler_parity/auto_port_v0_18_1_compiler_contract_scout.json")
            .expect("read v0.18.1 compiler scout fixture"),
    )
    .expect("parse v0.18.1 compiler scout fixture");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["kind"], "auto_port_v0_18_1_compiler_contract_scout");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.0");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.1");
    assert_eq!(
        fixture["python_reference"]["previous_commit"],
        "e4ccf099c8345a8b8708cdaa1ac510bdc7851387"
    );
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "0396c7852793b212d31345862b38a7d6f3f02854"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.0..v0.18.1"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.3.1"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_version_role"],
        "released_target_for_python_v0.18.1"
    );
    assert_eq!(
        fixture["rust_reference"]["previous_repo_crate_version"],
        "0.3.0"
    );
    assert_eq!(
        fixture["rust_reference"]["previous_repo_version_role"],
        "previous_baseline_for_python_v0.18.0"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.1");
    assert_eq!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.18.1 compiler scout must treat Rust 0.3.1 as the current target"
    );

    let sources: BTreeSet<_> = fixture["compiler_source_refs"]
        .as_array()
        .expect("compiler source refs are present")
        .iter()
        .map(|value| value.as_str().expect("compiler source ref"))
        .collect();
    for source_path in [
        "../millrace-py/src/millrace_ai/architecture/loop_graphs.py",
        "../millrace-py/src/millrace_ai/assets/entrypoints/planning/recon.md",
        "../millrace-py/src/millrace_ai/assets/graphs/planning/standard.json",
        "../millrace-py/src/millrace_ai/assets/registry/stage_kinds/planning/recon.json",
        "../millrace-py/src/millrace_ai/assets/skills/stage/planning/recon-core/SKILL.md",
        "../millrace-py/src/millrace_ai/compilation/node_materialization.py",
        "../millrace-py/tests/assets/test_entrypoints.py",
        "../millrace-py/tests/assets/test_loop_graphs.py",
        "../millrace-py/tests/assets/test_stage_kinds.py",
        "../millrace-py/tests/integration/test_compiler.py",
        "../millrace-py/tests/integration/test_single_compiled_plan.py",
    ] {
        assert!(
            sources.contains(source_path),
            "missing v0.18.1 compiler/recon source {source_path}"
        );
    }

    let targets: BTreeSet<_> = fixture["expected_rust_targets"]
        .as_array()
        .expect("expected Rust targets are present")
        .iter()
        .map(|value| value.as_str().expect("expected Rust target"))
        .collect();
    for target_path in [
        "millrace-agents/entrypoints/planning/recon.md",
        "millrace-agents/graphs/planning/standard.json",
        "millrace-agents/registry/stage_kinds/planning/recon.json",
        "millrace-agents/skills/stage/planning/recon-core/SKILL.md",
        "src/assets/baseline/entrypoints/planning/recon.md",
        "src/assets/baseline/graphs/planning/standard.json",
        "src/assets/baseline/registry/stage_kinds/planning/recon.json",
        "src/assets/baseline/skills/stage/planning/recon-core/SKILL.md",
        "src/compiler/contracts.rs",
        "src/compiler/materialization.rs",
        "src/compiler/graph_exports.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/compiler_parity.rs",
    ] {
        assert!(
            targets.contains(target_path),
            "missing v0.18.1 compiler/recon Rust target {target_path}"
        );
    }
}

#[test]
fn compiler_parity_scout_pins_python_v0_18_2_integrator_assets_and_graph_sources() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("compiler_parity/auto_port_v0_18_2_compiler_contract_scout.json")
            .expect("read v0.18.2 compiler scout fixture"),
    )
    .expect("parse v0.18.2 compiler scout fixture");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["kind"], "auto_port_v0_18_2_compiler_contract_scout");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.1");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.2");
    assert_eq!(
        fixture["python_reference"]["previous_commit"],
        "0396c7852793b212d31345862b38a7d6f3f02854"
    );
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "5444cb9485ea90b67b2ed6ba7e0723ae9fe7b79f"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.1..v0.18.2"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.3.1"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_version_role"],
        "previous_baseline_for_python_v0.18.1"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.2");
    assert_ne!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.18.2 compiler scout must not treat Rust 0.3.1 as the target"
    );

    let sources: BTreeSet<_> = fixture["compiler_source_refs"]
        .as_array()
        .expect("compiler source refs are present")
        .iter()
        .map(|value| value.as_str().expect("compiler source ref"))
        .collect();
    for source_path in [
        "../millrace-py/src/millrace_ai/contracts/enums.py",
        "../millrace-py/src/millrace_ai/contracts/stage_metadata.py",
        "../millrace-py/src/millrace_ai/assets/entrypoints/execution/checker.md",
        "../millrace-py/src/millrace_ai/assets/entrypoints/execution/integrator.md",
        "../millrace-py/src/millrace_ai/assets/graphs/execution/with_integrator.json",
        "../millrace-py/src/millrace_ai/assets/loop_graphs.py",
        "../millrace-py/src/millrace_ai/assets/loops/execution/with_integrator.json",
        "../millrace-py/src/millrace_ai/assets/modes/default_codex_integrated.json",
        "../millrace-py/src/millrace_ai/assets/modes/learning_codex_integrated.json",
        "../millrace-py/src/millrace_ai/assets/registry/stage_kinds/execution/integrator.json",
        "../millrace-py/src/millrace_ai/assets/skills/stage/execution/integrator-core/SKILL.md",
        "../millrace-py/tests/assets/test_entrypoints.py",
        "../millrace-py/tests/assets/test_loop_graphs.py",
        "../millrace-py/tests/assets/test_modes.py",
        "../millrace-py/tests/assets/test_packaging_runtime_assets.py",
        "../millrace-py/tests/assets/test_stage_kinds.py",
    ] {
        assert!(
            sources.contains(source_path),
            "missing v0.18.2 compiler/integrator source {source_path}"
        );
    }

    let targets: BTreeSet<_> = fixture["expected_rust_targets"]
        .as_array()
        .expect("expected Rust targets are present")
        .iter()
        .map(|value| value.as_str().expect("expected Rust target"))
        .collect();
    for target_path in [
        "millrace-agents/entrypoints/execution/integrator.md",
        "millrace-agents/graphs/execution/with_integrator.json",
        "millrace-agents/loops/execution/with_integrator.json",
        "millrace-agents/modes/default_codex_integrated.json",
        "millrace-agents/modes/learning_codex_integrated.json",
        "millrace-agents/registry/stage_kinds/execution/integrator.json",
        "millrace-agents/skills/stage/execution/integrator-core/SKILL.md",
        "src/assets/baseline/entrypoints/execution/integrator.md",
        "src/assets/baseline/graphs/execution/with_integrator.json",
        "src/assets/baseline/loops/execution/with_integrator.json",
        "src/assets/baseline/modes/default_codex_integrated.json",
        "src/assets/baseline/modes/learning_codex_integrated.json",
        "src/assets/baseline/registry/stage_kinds/execution/integrator.json",
        "src/assets/baseline/skills/stage/execution/integrator-core/SKILL.md",
        "src/contracts/enums.rs",
        "src/contracts/stage_metadata.rs",
        "src/compiler/assets.rs",
        "src/compiler/contracts.rs",
        "src/compiler/materialization.rs",
        "src/compiler/graph_exports.rs",
        "tests/contracts_stage_metadata.rs",
        "tests/compiler_assets.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/compiler_parity.rs",
        "tests/workspace_assets_baseline.rs",
    ] {
        assert!(
            targets.contains(target_path),
            "missing v0.18.2 compiler/integrator Rust target {target_path}"
        );
    }
}

#[test]
fn compiler_parity_scout_pins_python_v0_18_3_librarian_assets_graph_modes_and_skill_lint_sources() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("compiler_parity/auto_port_v0_18_3_compiler_contract_scout.json")
            .expect("read v0.18.3 compiler scout fixture"),
    )
    .expect("parse v0.18.3 compiler scout fixture");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["kind"], "auto_port_v0_18_3_compiler_contract_scout");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.2");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.3");
    assert_eq!(
        fixture["python_reference"]["previous_commit"],
        "5444cb9485ea90b67b2ed6ba7e0723ae9fe7b79f"
    );
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "6556e55c8463ce9256716bc425a49059b4c5981c"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.2..v0.18.3"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.3.2"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_version_role"],
        "previous_baseline_for_python_v0.18.2"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.3");
    assert_ne!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.18.3 compiler scout must not treat Rust 0.3.2 as the target"
    );

    let sources: BTreeSet<_> = fixture["compiler_source_refs"]
        .as_array()
        .expect("compiler source refs are present")
        .iter()
        .map(|value| value.as_str().expect("compiler source ref"))
        .collect();
    for source_path in [
        "../millrace-py/src/millrace_ai/contracts/enums.py",
        "../millrace-py/src/millrace_ai/contracts/stage_metadata.py",
        "../millrace-py/src/millrace_ai/assets/entrypoints/learning/librarian.md",
        "../millrace-py/src/millrace_ai/assets/graphs/learning/standard.json",
        "../millrace-py/src/millrace_ai/assets/loops/learning/default.json",
        "../millrace-py/src/millrace_ai/assets/modes/learning_codex.json",
        "../millrace-py/src/millrace_ai/assets/modes/learning_codex_integrated.json",
        "../millrace-py/src/millrace_ai/assets/modes/learning_pi.json",
        "../millrace-py/src/millrace_ai/assets/registry/stage_kinds/learning/librarian.json",
        "../millrace-py/src/millrace_ai/assets/skills/shared/marathon-qa-audit/SKILL.md",
        "../millrace-py/src/millrace_ai/assets/skills/stage/learning/librarian-core/SKILL.md",
        "../millrace-py/src/millrace_ai/compilation/node_materialization.py",
        "../millrace-py/tests/assets/test_packaging_runtime_assets.py",
        "../millrace-py/tests/assets/test_shipped_skill_lint.py",
        "../millrace-py/tests/integration/test_compiler.py",
    ] {
        assert!(
            sources.contains(source_path),
            "missing v0.18.3 compiler/Librarian source {source_path}"
        );
    }

    let targets: BTreeSet<_> = fixture["expected_rust_targets"]
        .as_array()
        .expect("expected Rust targets are present")
        .iter()
        .map(|value| value.as_str().expect("expected Rust target"))
        .collect();
    for target_path in [
        "millrace-agents/entrypoints/learning/librarian.md",
        "millrace-agents/graphs/learning/standard.json",
        "millrace-agents/loops/learning/default.json",
        "millrace-agents/modes/learning_codex.json",
        "millrace-agents/modes/learning_codex_auto_port.json",
        "millrace-agents/modes/learning_codex_integrated.json",
        "millrace-agents/modes/learning_pi.json",
        "millrace-agents/registry/stage_kinds/learning/librarian.json",
        "millrace-agents/skills/stage/learning/librarian-core/SKILL.md",
        "src/assets/baseline/entrypoints/learning/librarian.md",
        "src/assets/baseline/graphs/learning/standard.json",
        "src/assets/baseline/loops/learning/default.json",
        "src/assets/baseline/modes/learning_codex.json",
        "src/assets/baseline/modes/learning_codex_integrated.json",
        "src/assets/baseline/modes/learning_pi.json",
        "src/assets/baseline/registry/stage_kinds/learning/librarian.json",
        "src/assets/baseline/skills/stage/learning/librarian-core/SKILL.md",
        "src/compiler/assets.rs",
        "src/compiler/contracts.rs",
        "src/compiler/graph_exports.rs",
        "src/compiler/materialization.rs",
        "src/contracts/enums.rs",
        "src/contracts/stage_metadata.rs",
        "tests/compiler_assets.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/compiler_parity.rs",
        "tests/shipped_skill_lint.rs",
        "tests/workspace_assets_baseline.rs",
    ] {
        assert!(
            targets.contains(target_path),
            "missing v0.18.3 compiler/Librarian Rust target {target_path}"
        );
    }
}

fn init_workspace(root: &std::path::Path) {
    run_rust_millrace(["init", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace init")
        .assert_success();
}

fn fixture_case<'a>(fixture: &'a Value, requested_mode_id: &str) -> &'a Value {
    fixture["cases"]
        .as_array()
        .expect("fixture cases must be an array")
        .iter()
        .find(|case| case["requested_mode_id"] == requested_mode_id)
        .unwrap_or_else(|| panic!("missing compiler parity fixture case for {requested_mode_id}"))
}

fn normalize_plan(value: Value) -> Value {
    normalize_plan_value(value, None, None)
}

fn normalize_plan_value(value: Value, key: Option<&str>, mode_id: Option<String>) -> Value {
    match value {
        Value::Object(map) => {
            let object_mode_id = map
                .get("mode_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or(mode_id);
            let mut normalized = Map::new();
            for (child_key, child_value) in map {
                let mut child =
                    normalize_plan_value(child_value, Some(&child_key), object_mode_id.clone());
                if child_key == "compiled_plan_id" {
                    child = Value::String(format!(
                        "<compiled_plan_id:{}>",
                        object_mode_id.as_deref().unwrap_or("unknown")
                    ));
                }
                if child_key == "resolved_assets" {
                    if let Value::Array(mut assets) = child {
                        assets.sort_by_key(asset_sort_key);
                        child = Value::Array(assets);
                    }
                }
                normalized.insert(child_key, child);
            }
            Value::Object(normalized)
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|item| normalize_plan_value(item, key, mode_id.clone()))
                .collect(),
        ),
        Value::String(text) => match key {
            Some("compiled_at" | "emitted_at") => Value::String("<timestamp>".to_owned()),
            Some("config_fingerprint") => Value::String("<cfg-fingerprint>".to_owned()),
            Some("assets_fingerprint") => Value::String("<assets-fingerprint>".to_owned()),
            Some("content_sha256") if text != "missing" => {
                Value::String("<content-sha256>".to_owned())
            }
            Some("compile_time_path") => Value::String(normalize_runtime_path(&text)),
            _ => Value::String(text),
        },
        other => other,
    }
}

fn asset_sort_key(value: &Value) -> String {
    format!(
        "{}\0{}\0{}",
        value["asset_family"].as_str().unwrap_or_default(),
        value["logical_id"].as_str().unwrap_or_default(),
        value["compile_time_path"].as_str().unwrap_or_default(),
    )
}

fn normalize_cli_output(stdout: &str) -> Value {
    let mut diagnostics = Map::new();
    let mut show = Map::new();
    let mut entries = Vec::new();
    let mut completion_behavior = Map::new();
    let mut stages: Vec<Value> = Vec::new();
    let mut current_stage: Option<Map<String, Value>> = None;
    let mut in_show = false;

    for line in stdout.lines() {
        if line.starts_with("loop_id: ")
            || line.starts_with("node_order: ")
            || line.starts_with("learning_triggers: ")
            || line.starts_with("learning_trigger")
            || line.starts_with("concurrency_policy")
        {
            continue;
        }

        if line.starts_with("entry: ") {
            entries.push(Value::String(line.to_owned()));
            continue;
        }

        if let Some(completion) = line.strip_prefix("completion: ") {
            show.insert(
                "completion".to_owned(),
                Value::String(completion.to_owned()),
            );
            continue;
        }

        let Some((key, raw_value)) = line.split_once(": ") else {
            continue;
        };
        let value = normalize_cli_value(key, raw_value);

        if key == "compiled_plan_currentness" {
            in_show = true;
            show.insert(key.to_owned(), value);
            continue;
        }

        if key.starts_with("completion_behavior.") {
            completion_behavior.insert(key.to_owned(), value);
            continue;
        }

        if key == "stage" {
            if let Some(stage) = current_stage.take() {
                stages.push(Value::Object(stage));
            }
            let mut stage = Map::new();
            stage.insert("stage".to_owned(), Value::String(raw_value.to_owned()));
            current_stage = Some(stage);
            continue;
        }

        if is_stage_field(key) {
            if let Some(stage) = current_stage.as_mut() {
                stage.insert(key.to_owned(), value);
            }
            continue;
        }

        if is_diagnostic_field(key) && (!in_show || !key.starts_with("compile_input.")) {
            diagnostics.insert(key.to_owned(), value);
            continue;
        }

        if is_show_field(key) {
            show.insert(key.to_owned(), value);
            continue;
        }
    }

    if let Some(stage) = current_stage {
        stages.push(Value::Object(stage));
    }
    entries.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    stages.sort_by(|left, right| left["stage"].as_str().cmp(&right["stage"].as_str()));

    let mut result = Map::new();
    result.insert("diagnostics".to_owned(), Value::Object(diagnostics));
    if !show.is_empty() {
        show.insert("entries".to_owned(), Value::Array(entries));
        if !completion_behavior.is_empty() {
            show.insert(
                "completion_behavior".to_owned(),
                Value::Object(completion_behavior),
            );
        }
        if !stages.is_empty() {
            show.insert("stages".to_owned(), Value::Array(stages));
        }
        result.insert("show".to_owned(), Value::Object(show));
    }
    Value::Object(result)
}

fn normalize_cli_value(key: &str, value: &str) -> Value {
    let normalized = match key {
        "compiled_plan_id" => normalize_compiled_plan_id(value),
        "compile_input.config_fingerprint" | "persisted_compile_input.config_fingerprint" => {
            "<cfg-fingerprint>".to_owned()
        }
        "compile_input.assets_fingerprint" | "persisted_compile_input.assets_fingerprint" => {
            "<assets-fingerprint>".to_owned()
        }
        "baseline_manifest_id" => "<baseline_manifest_id>".to_owned(),
        "baseline_seed_package_version" => "<package_version>".to_owned(),
        "entrypoint_path" => normalize_runtime_path(value),
        "required_skills" | "attached_skills" => value
            .split(", ")
            .map(normalize_runtime_path)
            .collect::<Vec<_>>()
            .join(", "),
        _ => value.to_owned(),
    };
    Value::String(normalized)
}

fn normalize_compiled_plan_id(value: &str) -> String {
    let Some(remainder) = value.strip_prefix("plan-") else {
        return "<compiled_plan_id:unknown>".to_owned();
    };
    let Some((mode_id, digest)) = remainder.rsplit_once('-') else {
        return "<compiled_plan_id:unknown>".to_owned();
    };
    if digest.len() == 12
        && digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        format!("<compiled_plan_id:{mode_id}>")
    } else {
        "<compiled_plan_id:unknown>".to_owned()
    }
}

fn normalize_runtime_path(value: &str) -> String {
    let normalized = value.replace('\\', "/");
    normalized
        .strip_prefix("millrace-agents/")
        .unwrap_or(&normalized)
        .to_owned()
}

fn is_diagnostic_field(key: &str) -> bool {
    matches!(
        key,
        "ok" | "mode_id"
            | "used_last_known_good"
            | "compile_input.mode_id"
            | "compile_input.config_fingerprint"
            | "compile_input.assets_fingerprint"
    )
}

fn is_show_field(key: &str) -> bool {
    matches!(
        key,
        "execution_loop_id"
            | "planning_loop_id"
            | "learning_loop_id"
            | "compiled_plan_id"
            | "baseline_manifest_id"
            | "baseline_seed_package_version"
            | "compile_input.mode_id"
            | "compile_input.config_fingerprint"
            | "compile_input.assets_fingerprint"
            | "persisted_compile_input.mode_id"
            | "persisted_compile_input.config_fingerprint"
            | "persisted_compile_input.assets_fingerprint"
    )
}

fn is_stage_field(key: &str) -> bool {
    matches!(
        key,
        "stage_kind_id"
            | "running_status_marker"
            | "entrypoint_path"
            | "entrypoint_contract_id"
            | "required_skills"
            | "attached_skills"
            | "runner_name"
            | "model_name"
            | "thinking_level"
            | "model_reasoning_effort"
            | "timeout_seconds"
    )
}

fn assert_fingerprint_shapes(plan: &Value) {
    let fingerprint = &plan["compile_input_fingerprint"];
    let mode_id = fingerprint["mode_id"]
        .as_str()
        .expect("compile input mode id must be a string");
    assert_eq!(mode_id, plan["mode_id"].as_str().unwrap());
    assert!(
        fingerprint["config_fingerprint"]
            .as_str()
            .is_some_and(|value| value.starts_with("cfg-") && value.len() == 16),
        "config fingerprint shape drifted: {fingerprint:?}",
    );
    assert!(
        fingerprint["assets_fingerprint"]
            .as_str()
            .is_some_and(|value| value.starts_with("assets-") && value.len() == 19),
        "assets fingerprint shape drifted: {fingerprint:?}",
    );
    assert!(
        plan["resolved_assets"]
            .as_array()
            .expect("resolved_assets must be an array")
            .iter()
            .all(|asset| asset["content_sha256"]
                .as_str()
                .is_some_and(|value| value == "missing"
                    || (value.len() == 64
                        && value.chars().all(|character| character.is_ascii_hexdigit())))),
        "resolved asset content fingerprints drifted",
    );
}
