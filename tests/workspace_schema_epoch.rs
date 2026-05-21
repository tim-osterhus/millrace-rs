use std::{fs, process};

use millrace_ai::{
    RuntimeStartupOptions,
    contracts::Timestamp,
    startup_runtime_once_for_paths,
    workspace::{
        CURRENT_WORKSPACE_SCHEMA_EPOCH, RuntimeOwnershipLockOptions, RuntimeOwnershipLockState,
        SchemaArchiveResetOptions, acquire_runtime_ownership_lock_with_options,
        archive_reset_workspace_schema_with_options, ensure_workspace_schema_epoch_current,
        initialize_workspace, inspect_runtime_ownership_lock, load_workspace_schema_epoch_marker,
        release_runtime_ownership_lock, run_workspace_doctor_for_paths,
        workspace_schema_epoch_marker_path, write_workspace_schema_epoch_marker,
    },
};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-05-19T00:00:00Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("written_at", value).unwrap()
}

fn lock_options(session_id: &str) -> RuntimeOwnershipLockOptions {
    RuntimeOwnershipLockOptions::new(process::id(), "test-host", session_id, NOW).unwrap()
}

fn startup_options(session_id: &str) -> RuntimeStartupOptions {
    RuntimeStartupOptions {
        lock_options: Some(lock_options(session_id)),
        now: Some(timestamp(NOW)),
        ..RuntimeStartupOptions::default()
    }
}

#[test]
fn initialize_writes_current_workspace_schema_epoch_marker() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();

    let marker = load_workspace_schema_epoch_marker(&paths).unwrap();

    assert_eq!(marker.epoch_id, CURRENT_WORKSPACE_SCHEMA_EPOCH);
    assert!(workspace_schema_epoch_marker_path(&paths).is_file());
    ensure_workspace_schema_epoch_current(&paths).unwrap();
}

#[test]
fn archive_reset_refuses_active_daemon_owner() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let record = acquire_runtime_ownership_lock_with_options(&paths, lock_options("daemon"))
        .expect("acquire daemon lock");

    let error = archive_reset_workspace_schema_with_options(
        &paths,
        SchemaArchiveResetOptions::new("upgrade test").with_now(timestamp(NOW)),
    )
    .unwrap_err();

    assert!(error.to_string().contains("active daemon owner"));
    assert!(!paths.runtime_root.join("archives").exists());
    release_runtime_ownership_lock(&paths, Some(&record.owner_session_id), false).unwrap();
}

#[test]
fn archive_reset_moves_mutable_state_without_parsing_stale_json_and_reinitializes_clean_state() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    fs::write(&paths.runtime_snapshot_file, "{not-json").unwrap();
    fs::write(paths.tasks_active_dir.join("task-001.md"), "old task").unwrap();

    let result = archive_reset_workspace_schema_with_options(
        &paths,
        SchemaArchiveResetOptions::new("upgrade to v0.20").with_now(timestamp(NOW)),
    )
    .unwrap();

    assert_eq!(result.epoch_id, CURRENT_WORKSPACE_SCHEMA_EPOCH);
    assert!(
        result
            .archive_dir
            .starts_with(paths.runtime_root.join("archives"))
    );
    assert_eq!(
        fs::read_to_string(result.archive_dir.join("state/runtime_snapshot.json")).unwrap(),
        "{not-json"
    );
    assert!(
        result
            .archive_dir
            .join("tasks/active/task-001.md")
            .is_file()
    );
    assert!(
        result
            .moved_paths
            .contains(&"state/runtime_snapshot.json".to_owned())
    );
    assert!(
        result
            .moved_paths
            .contains(&"tasks/active/task-001.md".to_owned())
    );

    let snapshot: Value =
        serde_json::from_str(&fs::read_to_string(&paths.runtime_snapshot_file).unwrap()).unwrap();
    assert_eq!(snapshot["kind"], "runtime_snapshot");
    assert_eq!(
        load_workspace_schema_epoch_marker(&paths).unwrap().epoch_id,
        CURRENT_WORKSPACE_SCHEMA_EPOCH
    );

    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(&result.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["reason"], "upgrade to v0.20");
}

#[test]
fn startup_refuses_missing_or_stale_epoch_marker_before_loading_runtime_state() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    fs::write(&paths.runtime_snapshot_file, "{not-json").unwrap();
    fs::remove_file(workspace_schema_epoch_marker_path(&paths)).unwrap();

    let missing = startup_runtime_once_for_paths(&paths, startup_options("missing-marker"))
        .unwrap_err()
        .to_string();

    assert!(missing.contains("workspace schema epoch marker"));
    assert!(!missing.contains("runtime_snapshot"));
    assert_eq!(
        inspect_runtime_ownership_lock(&paths).state,
        RuntimeOwnershipLockState::Absent
    );

    write_workspace_schema_epoch_marker(&paths, Some("v0.19"), Some(timestamp(NOW))).unwrap();
    let stale = startup_runtime_once_for_paths(&paths, startup_options("stale-marker"))
        .unwrap_err()
        .to_string();
    assert!(stale.contains("incompatible"));
    assert!(!stale.contains("runtime_snapshot"));
}

#[test]
fn doctor_warns_for_schema_epoch_marker_drift() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    fs::remove_file(workspace_schema_epoch_marker_path(&paths)).unwrap();

    let report = run_workspace_doctor_for_paths(&paths);

    assert!(report.ok);
    assert!(
        report
            .warnings
            .iter()
            .any(|issue| issue.code == "workspace_schema_epoch_marker_invalid")
    );
}
