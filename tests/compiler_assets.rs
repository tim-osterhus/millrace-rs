use std::fs;

use millrace_ai::{
    compiler::{
        CompilerAssetError, DEFAULT_MODE_ID, MISSING_ASSET_TOKEN,
        compile_input_fingerprint_for_workspace, resolve_compile_assets,
    },
    workspace::initialize_workspace,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn sha256_hex(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    let mut rendered = String::with_capacity(digest.len() * 2);
    for byte in digest {
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}

fn asset_hash(resolution: &millrace_ai::compiler::ResolvedCompileAssetSet, path: &str) -> String {
    resolution
        .resolved_assets
        .iter()
        .find(|asset| asset.compile_time_path == path)
        .unwrap_or_else(|| panic!("missing resolved asset {path}"))
        .content_sha256
        .clone()
}

#[test]
fn initialized_workspace_assets_resolve_deterministically_from_authoritative_roots() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let first = resolve_compile_assets(&paths, None).unwrap();
    let second = resolve_compile_assets(&paths, None).unwrap();

    assert_eq!(first.mode_id, DEFAULT_MODE_ID);
    assert_eq!(first.resolved_assets, second.resolved_assets);
    assert_eq!(
        first.compile_input_fingerprint,
        second.compile_input_fingerprint
    );
    assert_eq!(
        first
            .resolved_assets
            .iter()
            .map(|asset| asset.compile_time_path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "modes/default_codex.json",
            "graphs/execution/standard.json",
            "graphs/planning/standard.json",
            "registry/stage_kinds/execution/builder.json",
            "registry/stage_kinds/execution/checker.json",
            "registry/stage_kinds/execution/fixer.json",
            "registry/stage_kinds/execution/doublechecker.json",
            "registry/stage_kinds/execution/updater.json",
            "registry/stage_kinds/execution/troubleshooter.json",
            "registry/stage_kinds/execution/consultant.json",
            "registry/stage_kinds/planning/recon.json",
            "registry/stage_kinds/planning/planner.json",
            "registry/stage_kinds/planning/manager.json",
            "registry/stage_kinds/planning/mechanic.json",
            "registry/stage_kinds/planning/auditor.json",
            "registry/stage_kinds/planning/arbiter.json",
            "registry/artifact_contracts/default_artifact_contracts.json",
            "registry/document_adapters/builtin_markdown_v1.json",
            "registry/document_adapters/blueprint_draft_markdown_v1.json",
            "registry/lifecycle_mutation_plans/default_lifecycle_mutations.json",
            "registry/queue_claim_policies/default_queue_claim_policies.json",
            "registry/request_context_profiles/default_request_context_profiles.json",
            "registry/runtime_effect_handlers/default_effect_handlers.json",
            "registry/runtime_effect_rules/planner_effect_rules.json",
            "registry/runtime_effect_rules/blueprint_effect_rules.json",
            "registry/runtime_failure_policies/default_runtime_failure_policies.json",
            "registry/terminal_actions/default_terminal_actions.json",
            "registry/recovery_policies/default_recovery_policies.json",
            "registry/workspace_schema_epochs/current.json",
            "registry/work_item_families/blueprint_draft.json",
            "registry/work_item_families/incident.json",
            "registry/work_item_families/learning_request.json",
            "registry/work_item_families/probe.json",
            "registry/work_item_families/spec.json",
            "registry/work_item_families/task.json",
            "entrypoints/execution/builder.md",
            "entrypoints/execution/checker.md",
            "entrypoints/execution/fixer.md",
            "entrypoints/execution/doublechecker.md",
            "entrypoints/execution/updater.md",
            "entrypoints/execution/troubleshooter.md",
            "entrypoints/execution/consultant.md",
            "entrypoints/planning/recon.md",
            "entrypoints/planning/planner.md",
            "entrypoints/planning/manager.md",
            "entrypoints/planning/mechanic.md",
            "entrypoints/planning/auditor.md",
            "entrypoints/planning/arbiter.md",
            "skills/stage/execution/builder-core/SKILL.md",
            "skills/stage/execution/checker-core/SKILL.md",
            "skills/stage/execution/fixer-core/SKILL.md",
            "skills/stage/execution/doublechecker-core/SKILL.md",
            "skills/stage/execution/updater-core/SKILL.md",
            "skills/stage/execution/troubleshooter-core/SKILL.md",
            "skills/stage/execution/consultant-core/SKILL.md",
            "skills/stage/planning/recon-core/SKILL.md",
            "skills/stage/planning/planner-core/SKILL.md",
            "skills/stage/planning/manager-core/SKILL.md",
            "skills/stage/planning/mechanic-core/SKILL.md",
            "skills/stage/planning/auditor-core/SKILL.md",
            "skills/stage/planning/arbiter-core/SKILL.md",
        ]
    );
    assert!(first.resolved_assets.iter().all(|asset| {
        !asset.compile_time_path.starts_with('/')
            && !asset.compile_time_path.contains('\\')
            && !asset.compile_time_path.starts_with("loops/")
            && asset.content_sha256 != MISSING_ASSET_TOKEN
    }));
    assert_eq!(
        asset_hash(&first, "entrypoints/execution/builder.md"),
        sha256_hex(&fs::read(paths.runtime_root.join("entrypoints/execution/builder.md")).unwrap())
    );
}

#[test]
fn opt_in_integrated_execution_assets_resolve_without_changing_default_mode() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let default = resolve_compile_assets(&paths, Some("default_codex")).unwrap();
    assert!(
        default
            .resolved_assets
            .iter()
            .all(|asset| asset.compile_time_path != "graphs/execution/with_integrator.json")
    );
    assert!(default.mode.learning_trigger_rules.is_empty());
    assert!(
        default
            .resolved_assets
            .iter()
            .all(|asset| !asset.compile_time_path.contains("librarian"))
    );

    let integrated = resolve_compile_assets(&paths, Some("default_codex_integrated")).unwrap();
    assert_eq!(integrated.mode_id, "default_codex_integrated");
    assert_eq!(
        integrated
            .graph_loops
            .iter()
            .find(|graph| graph.plane == millrace_ai::contracts::Plane::Execution)
            .unwrap()
            .graph_loop
            .loop_id,
        "execution.with_integrator"
    );

    let resolved_paths: Vec<_> = integrated
        .resolved_assets
        .iter()
        .map(|asset| asset.compile_time_path.as_str())
        .collect();
    for expected_path in [
        "modes/default_codex_integrated.json",
        "graphs/execution/with_integrator.json",
        "registry/stage_kinds/execution/integrator.json",
        "entrypoints/execution/integrator.md",
        "skills/stage/execution/integrator-core/SKILL.md",
    ] {
        assert!(
            resolved_paths.contains(&expected_path),
            "integrated resolution missed {expected_path}"
        );
    }
    assert!(
        integrated
            .resolved_assets
            .iter()
            .all(|asset| asset.content_sha256 != MISSING_ASSET_TOKEN)
    );

    let learning_integrated =
        resolve_compile_assets(&paths, Some("learning_codex_integrated")).unwrap();
    assert_eq!(learning_integrated.mode_id, "learning_codex_integrated");
    assert!(
        learning_integrated
            .graph_loops
            .iter()
            .any(|graph| graph.graph_loop.loop_id == "learning.standard")
    );
    assert!(
        learning_integrated
            .mode
            .concurrency_policy
            .as_ref()
            .is_some_and(|policy| policy.may_run_concurrently.len() == 2)
    );
    assert_eq!(learning_integrated.mode.learning_trigger_rules.len(), 4);
    assert!(
        learning_integrated
            .mode
            .stage_runner_bindings
            .values()
            .all(|runner| runner == "codex_cli")
    );
    assert!(
        learning_integrated
            .mode
            .stage_runner_bindings
            .contains_key(&millrace_ai::contracts::StageName::Integrator)
    );
    assert_eq!(
        learning_integrated
            .mode
            .stage_runner_bindings
            .get(&millrace_ai::contracts::StageName::Librarian)
            .map(String::as_str),
        Some("codex_cli")
    );
    let learning_integrated_paths: Vec<_> = learning_integrated
        .resolved_assets
        .iter()
        .map(|asset| asset.compile_time_path.as_str())
        .collect();
    for expected_path in [
        "registry/stage_kinds/learning/librarian.json",
        "entrypoints/learning/librarian.md",
        "skills/stage/learning/librarian-core/SKILL.md",
    ] {
        assert!(
            learning_integrated_paths.contains(&expected_path),
            "learning integrated resolution missed {expected_path}"
        );
    }
}

#[test]
fn blueprint_modes_resolve_blueprint_graph_stage_kinds_entrypoints_and_skills() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let blueprint = resolve_compile_assets(&paths, Some("blueprint_codex")).unwrap();
    assert_eq!(blueprint.mode_id, "blueprint_codex");
    assert!(
        blueprint
            .graph_loops
            .iter()
            .any(|graph| graph.graph_loop.loop_id == "planning.blueprint")
    );
    let resolved_paths: Vec<_> = blueprint
        .resolved_assets
        .iter()
        .map(|asset| asset.compile_time_path.as_str())
        .collect();
    for expected_path in [
        "modes/blueprint_codex.json",
        "graphs/planning/blueprint.json",
        "registry/stage_kinds/planning/manager_blueprint.json",
        "registry/stage_kinds/planning/contractor_blueprint.json",
        "registry/stage_kinds/planning/evaluator_blueprint.json",
        "registry/stage_kinds/planning/mechanic_blueprint.json",
        "entrypoints/planning/manager_blueprint.md",
        "entrypoints/planning/contractor_blueprint.md",
        "entrypoints/planning/evaluator_blueprint.md",
        "entrypoints/planning/mechanic_blueprint.md",
        "skills/stage/planning/manager-blueprint-core/SKILL.md",
        "skills/stage/planning/contractor-blueprint-core/SKILL.md",
        "skills/stage/planning/evaluator-blueprint-core/SKILL.md",
        "skills/stage/planning/mechanic-blueprint-core/SKILL.md",
    ] {
        assert!(
            resolved_paths.contains(&expected_path),
            "blueprint resolution missed {expected_path}"
        );
    }

    let learning = resolve_compile_assets(&paths, Some("blueprint_learning_codex")).unwrap();
    assert_eq!(learning.mode_id, "blueprint_learning_codex");
    assert!(
        learning
            .graph_loops
            .iter()
            .any(|graph| graph.graph_loop.loop_id == "learning.standard")
    );
    assert_eq!(learning.mode.learning_trigger_rules.len(), 4);
}

#[test]
fn mode_resolution_uses_requested_mode_config_default_and_aliases() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        [
            "[runtime]",
            "default_mode = \"learning_codex\"",
            "",
            "[runners.codex]",
            "permission_default = \"maximum\"",
            "",
        ]
        .join("\n"),
    )
    .unwrap();

    let configured = resolve_compile_assets(&paths, None).unwrap();
    assert_eq!(configured.mode_id, "learning_codex");
    assert!(
        configured
            .graph_loops
            .iter()
            .any(|graph| graph.graph_loop.loop_id == "learning.standard")
    );

    let alias = resolve_compile_assets(&paths, Some("standard_plain")).unwrap();
    let canonical = resolve_compile_assets(&paths, Some("default_codex")).unwrap();
    assert_eq!(alias.mode_id, "default_codex");
    assert_eq!(
        alias.compile_input_fingerprint,
        canonical.compile_input_fingerprint
    );
}

#[test]
fn fingerprints_detect_referenced_drift_and_ignore_unreferenced_files() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let before = compile_input_fingerprint_for_workspace(&paths, Some("default_codex")).unwrap();

    fs::write(
        paths.runtime_root.join("loops/execution/default.json"),
        "{}\n",
    )
    .unwrap();
    fs::write(
        paths
            .runtime_root
            .join("entrypoints/execution/unreferenced.md"),
        "unreferenced\n",
    )
    .unwrap();
    let after_unreferenced =
        compile_input_fingerprint_for_workspace(&paths, Some("default_codex")).unwrap();
    assert_eq!(after_unreferenced, before);

    fs::write(
        paths.runtime_root.join("entrypoints/execution/builder.md"),
        "changed builder entrypoint\n",
    )
    .unwrap();
    let after_referenced =
        compile_input_fingerprint_for_workspace(&paths, Some("default_codex")).unwrap();
    assert_ne!(
        after_referenced.assets_fingerprint,
        before.assets_fingerprint
    );
    assert_eq!(
        after_referenced.config_fingerprint,
        before.config_fingerprint
    );
    assert_eq!(after_referenced.mode_id, before.mode_id);
}

#[test]
fn compile_fingerprints_ignore_adapter_only_runner_config() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let before = compile_input_fingerprint_for_workspace(&paths, Some("default_codex")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        [
            "[runtime]",
            "default_mode = \"default_codex\"",
            "",
            "[runners.codex]",
            "command = \"custom-codex\"",
            "args = [\"exec\", \"--trace\"]",
            "profile = \"ops\"",
            "permission_default = \"elevated\"",
            "permission_by_stage = { builder = \"basic\" }",
            "permission_by_model = { \"gpt-5\" = \"maximum\" }",
            "skip_git_repo_check = false",
            "extra_config = [\"sandbox_workspace_write.network_access=true\"]",
            "",
            "[runners.codex.env]",
            "CODEX_HOME = \"/tmp/codex\"",
            "",
            "[runners.pi]",
            "command = \"custom-pi\"",
            "args = [\"--debug\"]",
            "provider = \"openai\"",
            "thinking = \"medium\"",
            "disable_context_files = false",
            "disable_skills = false",
            "event_log_policy = \"full\"",
            "",
            "[runners.pi.env]",
            "PI_HOME = \"/tmp/pi\"",
            "",
        ]
        .join("\n"),
    )
    .unwrap();

    let after = compile_input_fingerprint_for_workspace(&paths, Some("default_codex")).unwrap();
    assert_eq!(after, before);
}

#[test]
fn unreferenced_invalid_assets_do_not_enter_resolution_or_fingerprints() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let before = resolve_compile_assets(&paths, Some("default_codex")).unwrap();

    fs::write(
        paths
            .runtime_root
            .join("graphs/execution/unreferenced.json"),
        "{not valid json",
    )
    .unwrap();
    fs::write(
        paths.runtime_root.join("modes/unreferenced.json"),
        "{not valid json",
    )
    .unwrap();

    let after = resolve_compile_assets(&paths, Some("default_codex")).unwrap();
    assert_eq!(after.resolved_assets, before.resolved_assets);
    assert_eq!(
        after.compile_input_fingerprint,
        before.compile_input_fingerprint
    );
}

#[test]
fn missing_required_assets_and_invalid_references_report_path_context() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    fs::remove_file(
        paths
            .runtime_root
            .join("skills/stage/execution/builder-core/SKILL.md"),
    )
    .unwrap();
    let error = resolve_compile_assets(&paths, Some("default_codex")).unwrap_err();
    assert!(matches!(
        error,
        CompilerAssetError::MissingReferencedAsset {
            asset_family: "skill",
            ref logical_id,
            ref path,
        } if logical_id == "skill:skills/stage/execution/builder-core/SKILL.md"
            && path.ends_with("skills/stage/execution/builder-core/SKILL.md")
    ));

    let paths = initialize_workspace(temp_dir.path().join("workspace-invalid-graph")).unwrap();
    fs::write(
        paths.runtime_root.join("graphs/execution/standard.json"),
        "{not valid json",
    )
    .unwrap();
    let error = resolve_compile_assets(&paths, Some("default_codex")).unwrap_err();
    assert!(matches!(
        error,
        CompilerAssetError::Contract {
            artifact: "graph_loop",
            ref path,
            ..
        } if path.ends_with("graphs/execution/standard.json")
    ));

    let paths = initialize_workspace(temp_dir.path().join("workspace-missing-entrypoint")).unwrap();
    fs::remove_file(paths.runtime_root.join("entrypoints/execution/builder.md")).unwrap();
    let error = resolve_compile_assets(&paths, Some("default_codex")).unwrap_err();
    assert!(matches!(
        error,
        CompilerAssetError::MissingReferencedAsset {
            asset_family: "entrypoint",
            ref logical_id,
            ref path,
        } if logical_id == "entrypoint:entrypoints/execution/builder.md"
            && path.ends_with("entrypoints/execution/builder.md")
    ));
}

#[test]
fn compile_config_rejects_unsupported_stage_override_keys() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    fs::write(
        &paths.runtime_config_file,
        "[stages.builder]\npermission_default = \"basic\"\n",
    )
    .unwrap();

    let error = resolve_compile_assets(&paths, Some("default_codex")).unwrap_err();
    assert!(matches!(
        error,
        CompilerAssetError::InvalidConfig {
            ref field,
            ref message,
            ..
        } if field == "stages.builder.permission_default"
            && message.contains("unsupported stage override key")
    ));
}
