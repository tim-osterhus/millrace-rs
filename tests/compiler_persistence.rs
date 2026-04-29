use std::fs;

use millrace_ai::{
    compiler::{
        CompileWorkspaceOptions, CompiledPlanCurrentnessState, compile_and_persist_workspace_plan,
        compile_and_persist_workspace_plan_for_paths,
        compile_and_persist_workspace_plan_with_options, inspect_workspace_plan_currentness,
        inspect_workspace_plan_currentness_for_paths, load_persisted_compile_diagnostics,
        load_persisted_compiled_plan,
    },
    contracts::Timestamp,
    workspace::{WorkspacePaths, atomic_write_text, initialize_workspace},
};
use tempfile::TempDir;

fn fixed_compiled_at() -> Timestamp {
    Timestamp::parse("compiled_at", "2026-04-28T16:00:00Z").unwrap()
}

fn options_for(mode_id: &str) -> CompileWorkspaceOptions {
    CompileWorkspaceOptions {
        requested_mode_id: Some(mode_id.to_owned()),
        compiled_at: Some(fixed_compiled_at()),
        ..CompileWorkspaceOptions::default()
    }
}

fn initialized_workspace() -> (TempDir, WorkspacePaths) {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    (temp_dir, paths)
}

fn compile_default(paths: &WorkspacePaths) -> millrace_ai::compiler::CompileOutcome {
    compile_and_persist_workspace_plan_for_paths(paths, options_for("default_codex")).unwrap()
}

#[test]
fn compile_facade_persists_plan_and_diagnostics_without_legacy_artifact() {
    let temp_dir = TempDir::new().unwrap();
    let workspace_root = temp_dir.path().join("workspace");

    let outcome = compile_and_persist_workspace_plan_with_options(
        &workspace_root,
        options_for("standard_plain"),
    )
    .unwrap();
    let paths = WorkspacePaths::new(&workspace_root);

    assert!(outcome.diagnostics.ok);
    assert!(!outcome.used_last_known_good);
    assert_eq!(outcome.diagnostics.mode_id, "default_codex");
    let active_plan = outcome.active_plan.as_ref().unwrap();
    assert_eq!(
        outcome.compiled_plan_id.as_deref(),
        Some(active_plan.compiled_plan_id.as_str())
    );
    assert_eq!(
        outcome.compile_input_fingerprint.as_ref(),
        Some(&active_plan.compile_input_fingerprint)
    );
    assert_eq!(outcome.resolved_assets, active_plan.resolved_assets);

    assert!(paths.compiled_plan_file.is_file());
    assert!(paths.compile_diagnostics_file.is_file());
    assert!(!paths.state_dir.join("compiled_graph_plan.json").exists());

    let persisted_plan = load_persisted_compiled_plan(&paths).unwrap().unwrap();
    let persisted_diagnostics = load_persisted_compile_diagnostics(&paths).unwrap().unwrap();
    assert_eq!(persisted_plan, *active_plan);
    assert!(persisted_diagnostics.ok);
    assert_eq!(persisted_diagnostics.mode_id, "default_codex");
    assert!(persisted_diagnostics.errors.is_empty());
    assert_eq!(persisted_diagnostics.emitted_at, fixed_compiled_at());
}

#[test]
fn failed_validation_persists_diagnostics_and_preserves_last_known_good() {
    let (_temp_dir, paths) = initialized_workspace();
    let initial = compile_default(&paths);
    let initial_plan = initial.active_plan.unwrap();
    let original_plan_text = fs::read_to_string(&paths.compiled_plan_file).unwrap();

    let mut mode: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(paths.modes_dir.join("default_codex.json")).unwrap(),
    )
    .unwrap();
    mode["loop_ids_by_plane"]["planning"] =
        serde_json::Value::String("planning.unknown".to_owned());
    fs::write(
        paths.modes_dir.join("default_codex.json"),
        serde_json::to_string_pretty(&mode).unwrap() + "\n",
    )
    .unwrap();

    let failed = compile_default(&paths);

    assert!(!failed.diagnostics.ok);
    assert_eq!(failed.diagnostics.mode_id, "default_codex");
    assert!(
        failed.diagnostics.errors[0].contains("planning.unknown"),
        "{:?}",
        failed.diagnostics.errors
    );
    assert!(failed.used_last_known_good);
    assert_eq!(
        failed
            .active_plan
            .as_ref()
            .map(|plan| &plan.compiled_plan_id),
        Some(&initial_plan.compiled_plan_id)
    );
    assert_eq!(
        fs::read_to_string(&paths.compiled_plan_file).unwrap(),
        original_plan_text
    );

    let diagnostics = load_persisted_compile_diagnostics(&paths).unwrap().unwrap();
    assert!(!diagnostics.ok);
    assert_eq!(diagnostics.mode_id, "default_codex");
    assert!(!diagnostics.errors.is_empty());
}

#[test]
fn currentness_reports_missing_current_stale_and_unknown() {
    let (_temp_dir, paths) = initialized_workspace();

    let missing =
        inspect_workspace_plan_currentness_for_paths(&paths, Some("default_codex")).unwrap();
    assert_eq!(missing.state, CompiledPlanCurrentnessState::Missing);
    assert_eq!(missing.persisted_plan_id, None);
    assert_eq!(missing.expected_fingerprint.mode_id, "default_codex");

    let compiled = compile_default(&paths);
    let current =
        inspect_workspace_plan_currentness_for_paths(&paths, Some("default_codex")).unwrap();
    assert_eq!(current.state, CompiledPlanCurrentnessState::Current);
    assert_eq!(current.persisted_plan_id, compiled.compiled_plan_id);

    fs::write(
        paths.entrypoints_dir.join("execution/builder.md"),
        "changed builder entrypoint\n",
    )
    .unwrap();
    let stale =
        inspect_workspace_plan_currentness_for_paths(&paths, Some("default_codex")).unwrap();
    assert_eq!(stale.state, CompiledPlanCurrentnessState::Stale);
    assert_eq!(stale.persisted_plan_id, compiled.compiled_plan_id);

    fs::write(&paths.compiled_plan_file, "{not valid json\n").unwrap();
    let unknown =
        inspect_workspace_plan_currentness_for_paths(&paths, Some("default_codex")).unwrap();
    assert_eq!(unknown.state, CompiledPlanCurrentnessState::Unknown);
    assert_eq!(unknown.persisted_plan_id, None);
}

#[test]
fn currentness_detects_referenced_mode_graph_stage_kind_entrypoint_and_config_drift() {
    assert_stale_after(|paths| {
        fs::write(paths.modes_dir.join("default_codex.json"), "{}\n").unwrap();
    });
    assert_stale_after(|paths| {
        fs::write(paths.graphs_dir.join("execution/standard.json"), "{}\n").unwrap();
    });
    assert_stale_after(|paths| {
        fs::write(
            paths.stage_kind_registry_dir.join("execution/builder.json"),
            "{}\n",
        )
        .unwrap();
    });
    assert_stale_after(|paths| {
        fs::write(
            paths.entrypoints_dir.join("execution/builder.md"),
            "drift\n",
        )
        .unwrap();
    });
    assert_stale_after(|paths| {
        fs::write(
            &paths.runtime_config_file,
            [
                "[runtime]",
                "default_mode = \"default_codex\"",
                "",
                "[recovery]",
                "max_fix_cycles = 5",
                "",
            ]
            .join("\n"),
        )
        .unwrap();
    });
}

#[test]
fn currentness_tracks_attached_skill_drift_but_ignores_unreferenced_assets() {
    let (_temp_dir, paths) = initialized_workspace();
    fs::write(
        paths.modes_dir.join("custom_attached.json"),
        r#"{
  "schema_version": "1.0",
  "kind": "mode",
  "mode_id": "custom_attached",
  "loop_ids_by_plane": {
    "execution": "execution.standard",
    "planning": "planning.standard"
  },
  "stage_skill_additions": {
    "builder": ["skills/execution/builder.md"]
  }
}
"#,
    )
    .unwrap();
    fs::create_dir_all(paths.skills_dir.join("execution")).unwrap();
    fs::write(
        paths.skills_dir.join("execution/builder.md"),
        "builder attached skill\n",
    )
    .unwrap();
    compile_and_persist_workspace_plan_for_paths(&paths, options_for("custom_attached")).unwrap();

    fs::write(paths.skills_dir.join("unused.md"), "unused skill\n").unwrap();
    fs::write(
        paths.entrypoints_dir.join("execution/unused.md"),
        "unused entrypoint\n",
    )
    .unwrap();
    let current =
        inspect_workspace_plan_currentness_for_paths(&paths, Some("custom_attached")).unwrap();
    assert_eq!(current.state, CompiledPlanCurrentnessState::Current);

    fs::write(
        paths.skills_dir.join("execution/builder.md"),
        "attached skill drift\n",
    )
    .unwrap();
    let stale =
        inspect_workspace_plan_currentness_for_paths(&paths, Some("custom_attached")).unwrap();
    assert_eq!(stale.state, CompiledPlanCurrentnessState::Stale);
}

#[test]
fn stale_last_known_good_refusal_returns_no_active_plan() {
    let (_temp_dir, paths) = initialized_workspace();
    let initial = compile_default(&paths);
    let initial_plan_text = fs::read_to_string(&paths.compiled_plan_file).unwrap();
    assert!(initial.active_plan.is_some());

    let mut mode: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(paths.modes_dir.join("default_codex.json")).unwrap(),
    )
    .unwrap();
    mode["loop_ids_by_plane"]["planning"] =
        serde_json::Value::String("planning.unknown".to_owned());
    fs::write(
        paths.modes_dir.join("default_codex.json"),
        serde_json::to_string_pretty(&mode).unwrap() + "\n",
    )
    .unwrap();

    let outcome = compile_and_persist_workspace_plan_for_paths(
        &paths,
        CompileWorkspaceOptions {
            refuse_stale_last_known_good: true,
            ..options_for("default_codex")
        },
    )
    .unwrap();

    assert!(!outcome.diagnostics.ok);
    assert!(outcome.active_plan.is_none());
    assert!(!outcome.used_last_known_good);
    assert_eq!(
        fs::read_to_string(&paths.compiled_plan_file).unwrap(),
        initial_plan_text
    );
}

#[test]
fn simple_facade_accepts_workspace_root_and_requested_mode() {
    let temp_dir = TempDir::new().unwrap();
    let workspace_root = temp_dir.path().join("workspace");

    let outcome = compile_and_persist_workspace_plan(&workspace_root, Some("default_pi")).unwrap();

    assert!(outcome.diagnostics.ok);
    assert_eq!(outcome.diagnostics.mode_id, "default_pi");
    let current = inspect_workspace_plan_currentness(&workspace_root, Some("default_pi")).unwrap();
    assert_eq!(current.state, CompiledPlanCurrentnessState::Current);
}

#[cfg(unix)]
#[test]
fn atomic_write_preserves_destination_when_same_directory_temp_write_fails() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().unwrap();
    let state_dir = temp_dir.path().join("state");
    fs::create_dir_all(&state_dir).unwrap();
    let path = state_dir.join("compiled_plan.json");
    fs::write(&path, "original\n").unwrap();

    let original_permissions = fs::metadata(&state_dir).unwrap().permissions();
    fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o555)).unwrap();
    let result = atomic_write_text(&path, "replacement\n");
    fs::set_permissions(&state_dir, original_permissions).unwrap();

    assert!(result.is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
    let leftovers: Vec<_> = fs::read_dir(&state_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty());
}

fn assert_stale_after(drift: impl FnOnce(&WorkspacePaths)) {
    let (_temp_dir, paths) = initialized_workspace();
    let compiled = compile_default(&paths);
    let before =
        inspect_workspace_plan_currentness_for_paths(&paths, Some("default_codex")).unwrap();
    assert_eq!(before.state, CompiledPlanCurrentnessState::Current);

    drift(&paths);

    let after =
        inspect_workspace_plan_currentness_for_paths(&paths, Some("default_codex")).unwrap();
    assert_eq!(after.state, CompiledPlanCurrentnessState::Stale);
    assert_eq!(after.persisted_plan_id, compiled.compiled_plan_id);
}
