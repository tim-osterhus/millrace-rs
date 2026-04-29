mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use millrace_ai::contracts::{RecoveryCounters, RuntimeJsonContract, RuntimeSnapshot};
use millrace_ai::workspace::{
    WorkspacePaths, default_file_payloads, initialize_workspace, workspace_paths,
};
use tempfile::TempDir;

fn relative_to(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

fn relative_directories(paths: &WorkspacePaths) -> BTreeSet<String> {
    paths
        .directories()
        .into_iter()
        .map(|path| relative_to(path, &paths.root))
        .collect()
}

#[test]
fn workspace_paths_resolve_python_shaped_contract_surfaces() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");

    let paths = workspace_paths(&root);

    assert_eq!(paths.root, root);
    assert_eq!(paths.runtime_root, root.join("millrace-agents"));
    assert_eq!(paths.state_dir, root.join("millrace-agents/state"));
    assert_eq!(
        paths.mailbox_incoming_dir,
        root.join("millrace-agents/state/mailbox/incoming")
    );
    assert_eq!(paths.runs_dir, root.join("millrace-agents/runs"));
    assert_eq!(
        paths.tasks_queue_dir,
        root.join("millrace-agents/tasks/queue")
    );
    assert_eq!(
        paths.specs_active_dir,
        root.join("millrace-agents/specs/active")
    );
    assert_eq!(
        paths.incidents_resolved_dir,
        root.join("millrace-agents/incidents/resolved")
    );
    assert_eq!(
        paths.learning_requests_queue_dir,
        root.join("millrace-agents/learning/requests/queue")
    );
    assert_eq!(
        paths.learning_research_packets_dir,
        root.join("millrace-agents/learning/research-packets")
    );
    assert_eq!(
        paths.arbiter_root_spec_contracts_dir,
        root.join("millrace-agents/arbiter/contracts/root-specs")
    );
    assert_eq!(
        paths.execution_loops_dir,
        root.join("millrace-agents/loops/execution")
    );
    assert_eq!(
        paths.planning_graphs_dir,
        root.join("millrace-agents/graphs/planning")
    );
    assert_eq!(
        paths.execution_stage_kind_registry_dir,
        root.join("millrace-agents/registry/stage_kinds/execution")
    );
    assert_eq!(paths.modes_dir, root.join("millrace-agents/modes"));
    assert_eq!(paths.logs_dir, root.join("millrace-agents/logs"));
    assert_eq!(
        paths.entrypoints_dir,
        root.join("millrace-agents/entrypoints")
    );
    assert_eq!(paths.skills_dir, root.join("millrace-agents/skills"));
    assert_eq!(
        paths.execution_status_file,
        root.join("millrace-agents/state/execution_status.md")
    );
    assert_eq!(
        paths.runtime_snapshot_file,
        root.join("millrace-agents/state/runtime_snapshot.json")
    );
    assert_eq!(
        paths.recovery_counters_file,
        root.join("millrace-agents/state/recovery_counters.json")
    );
    assert_eq!(
        paths.baseline_manifest_file,
        root.join("millrace-agents/state/baseline_manifest.json")
    );
    assert_eq!(
        paths.runtime_config_file,
        root.join("millrace-agents/millrace.toml")
    );
    assert_eq!(
        paths.runtime_lock_file,
        root.join("millrace-agents/state/runtime_daemon.lock.json")
    );
}

#[test]
fn directories_cover_the_canonical_workspace_tree_without_legacy_roles() {
    let temp_dir = TempDir::new().unwrap();
    let paths = workspace_paths(temp_dir.path().join("workspace"));

    let expected = BTreeSet::from([
        "millrace-agents",
        "millrace-agents/arbiter",
        "millrace-agents/arbiter/contracts",
        "millrace-agents/arbiter/contracts/ideas",
        "millrace-agents/arbiter/contracts/root-specs",
        "millrace-agents/arbiter/reports",
        "millrace-agents/arbiter/rubrics",
        "millrace-agents/arbiter/targets",
        "millrace-agents/arbiter/verdicts",
        "millrace-agents/entrypoints",
        "millrace-agents/graphs",
        "millrace-agents/graphs/execution",
        "millrace-agents/graphs/learning",
        "millrace-agents/graphs/planning",
        "millrace-agents/incidents",
        "millrace-agents/incidents/active",
        "millrace-agents/incidents/blocked",
        "millrace-agents/incidents/incoming",
        "millrace-agents/incidents/resolved",
        "millrace-agents/learning",
        "millrace-agents/learning/requests",
        "millrace-agents/learning/requests/active",
        "millrace-agents/learning/requests/blocked",
        "millrace-agents/learning/requests/done",
        "millrace-agents/learning/requests/queue",
        "millrace-agents/learning/research-packets",
        "millrace-agents/learning/skill-candidates",
        "millrace-agents/learning/update-candidates",
        "millrace-agents/logs",
        "millrace-agents/loops",
        "millrace-agents/loops/execution",
        "millrace-agents/loops/learning",
        "millrace-agents/loops/planning",
        "millrace-agents/modes",
        "millrace-agents/registry",
        "millrace-agents/registry/stage_kinds",
        "millrace-agents/registry/stage_kinds/execution",
        "millrace-agents/registry/stage_kinds/learning",
        "millrace-agents/registry/stage_kinds/planning",
        "millrace-agents/runs",
        "millrace-agents/skills",
        "millrace-agents/specs",
        "millrace-agents/specs/active",
        "millrace-agents/specs/blocked",
        "millrace-agents/specs/done",
        "millrace-agents/specs/queue",
        "millrace-agents/state",
        "millrace-agents/state/mailbox",
        "millrace-agents/state/mailbox/failed",
        "millrace-agents/state/mailbox/incoming",
        "millrace-agents/state/mailbox/processed",
        "millrace-agents/tasks",
        "millrace-agents/tasks/active",
        "millrace-agents/tasks/blocked",
        "millrace-agents/tasks/done",
        "millrace-agents/tasks/queue",
    ])
    .into_iter()
    .map(str::to_owned)
    .collect();

    assert_eq!(relative_directories(&paths), expected);
    assert!(!paths.runtime_root.join("roles").exists());
}

#[test]
fn initialize_workspace_creates_defaults_and_preserves_operator_edits() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");

    let paths = initialize_workspace(&root).unwrap();

    for directory in paths.directories() {
        assert!(
            directory.is_dir(),
            "missing directory: {}",
            directory.display()
        );
    }

    assert_eq!(
        fs::read_to_string(&paths.execution_status_file).unwrap(),
        "### IDLE\n"
    );
    assert_eq!(
        fs::read_to_string(&paths.planning_status_file).unwrap(),
        "### IDLE\n"
    );
    assert_eq!(
        fs::read_to_string(&paths.learning_status_file).unwrap(),
        "### IDLE\n"
    );
    assert_eq!(fs::read_to_string(&paths.outline_file).unwrap(), "");
    assert_eq!(fs::read_to_string(&paths.historylog_file).unwrap(), "");
    assert_eq!(fs::read_to_string(&paths.learning_events_file).unwrap(), "");

    let config = fs::read_to_string(&paths.runtime_config_file).unwrap();
    assert!(config.contains("[runtime]\ndefault_mode = \"default_codex\""));
    assert!(config.contains("[runners.codex]\npermission_default = \"maximum\""));

    let snapshot =
        RuntimeSnapshot::from_json_str(&fs::read_to_string(&paths.runtime_snapshot_file).unwrap())
            .unwrap();
    assert_eq!(snapshot.runtime_mode.as_str(), "daemon");
    assert!(!snapshot.process_running);
    assert_eq!(snapshot.active_mode_id, "default_codex");
    assert_eq!(snapshot.compiled_plan_id, "bootstrap");
    assert_eq!(
        snapshot.compiled_plan_path,
        "millrace-agents/state/compiled_plan.json"
    );
    assert_eq!(snapshot.execution_status_marker, "### IDLE");
    assert_eq!(snapshot.queue_depth_execution, 0);
    assert!(snapshot.active_stage.is_none());

    let counters = RecoveryCounters::from_json_str(
        &fs::read_to_string(&paths.recovery_counters_file).unwrap(),
    )
    .unwrap();
    assert!(counters.entries.is_empty());

    let custom_config = "[runtime]\ndefault_mode = \"custom\"\n";
    let edited_files = [
        (&paths.execution_status_file, "### CHECKER_PASS\n"),
        (&paths.planning_status_file, "### PLANNER_COMPLETE\n"),
        (&paths.learning_status_file, "### ANALYST_COMPLETE\n"),
        (&paths.outline_file, "# Existing Outline\n"),
        (&paths.historylog_file, "existing history\n"),
        (&paths.runtime_snapshot_file, "{\"custom\": true}\n"),
        (
            &paths.recovery_counters_file,
            "{\"entries\": [\"custom\"]}\n",
        ),
        (&paths.learning_events_file, "{\"event\": true}\n"),
        (&paths.runtime_config_file, custom_config),
    ];
    for (path, payload) in edited_files {
        fs::write(path, payload).unwrap();
    }

    initialize_workspace(&root).unwrap();
    initialize_workspace(&root).unwrap();

    for (path, payload) in edited_files {
        assert_eq!(fs::read_to_string(path).unwrap(), payload);
    }
}

#[test]
fn initialize_workspace_does_not_create_legacy_root_runtime_directories() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let legacy_dirs = [
        "state",
        "runs",
        "tasks",
        "specs",
        "incidents",
        "loops",
        "graphs",
        "registry",
        "logs",
        "entrypoints",
        "skills",
        "roles",
    ];
    let repo_before: Vec<_> = legacy_dirs
        .iter()
        .map(|name| (name, repo_root.join(name).exists()))
        .collect();

    initialize_workspace(&root).unwrap();

    for name in legacy_dirs {
        assert!(
            !root.join(name).exists(),
            "unexpected root-level runtime artifact: {name}"
        );
    }
    assert!(!root.join("millrace-agents/roles").exists());

    for (name, existed_before) in repo_before {
        assert_eq!(
            repo_root.join(name).exists(),
            existed_before,
            "repository root artifact changed after init: {name}"
        );
    }
}

#[test]
fn default_file_payloads_cover_the_initialization_defaults() {
    let temp_dir = TempDir::new().unwrap();
    let paths = workspace_paths(temp_dir.path().join("workspace"));
    let defaults = default_file_payloads(&paths).unwrap();

    assert!(defaults.contains_key(&paths.execution_status_file));
    assert!(defaults.contains_key(&paths.planning_status_file));
    assert!(defaults.contains_key(&paths.learning_status_file));
    assert!(defaults.contains_key(&paths.runtime_snapshot_file));
    assert!(defaults.contains_key(&paths.recovery_counters_file));
    assert!(defaults.contains_key(&paths.learning_events_file));
    assert!(defaults.contains_key(&paths.runtime_config_file));
    assert!(defaults.contains_key(&paths.outline_file));
    assert!(defaults.contains_key(&paths.historylog_file));
    assert!(!defaults.contains_key(&paths.baseline_manifest_file));
}
