mod support;

use std::{fs, process};

use serde_json::Value;
use tempfile::TempDir;

use millrace_ai::workspace::{
    RuntimeOwnershipLockError, RuntimeOwnershipLockOptions, RuntimeOwnershipLockState,
    RuntimeOwnershipRecord, acquire_runtime_ownership_lock_with_options,
    clear_stale_runtime_ownership_lock_with_pid_checker, initialize_workspace,
    inspect_runtime_ownership_lock, inspect_runtime_ownership_lock_with_pid_checker,
    release_runtime_ownership_lock,
};

const ACQUIRED_AT: &str = "2026-04-15T00:00:00Z";

fn options(pid: u32, session_id: &str) -> RuntimeOwnershipLockOptions {
    RuntimeOwnershipLockOptions::new(pid, "test-host", session_id, ACQUIRED_AT).unwrap()
}

fn record(workspace_root: &str, pid: u32, session_id: &str) -> RuntimeOwnershipRecord {
    RuntimeOwnershipRecord::new(workspace_root, pid, "test-host", session_id, ACQUIRED_AT).unwrap()
}

fn write_record(path: &std::path::Path, record: &RuntimeOwnershipRecord) {
    let mut payload = serde_json::to_string_pretty(record).unwrap();
    payload.push('\n');
    fs::write(path, payload).unwrap();
}

#[test]
fn inspect_classifies_absent_active_stale_invalid_and_wrong_workspace_locks() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let absent = inspect_runtime_ownership_lock(&paths);
    assert_eq!(absent.state, RuntimeOwnershipLockState::Absent);
    assert!(absent.record.is_none());

    let lock_record = record(&paths.root.to_string_lossy(), 42, "active-session");
    write_record(&paths.runtime_lock_file, &lock_record);
    let active = inspect_runtime_ownership_lock_with_pid_checker(&paths, |pid| pid == 42);
    assert_eq!(active.state, RuntimeOwnershipLockState::Active);
    assert_eq!(active.record, Some(lock_record.clone()));
    assert!(active.detail.contains("pid=42"));

    let stale = inspect_runtime_ownership_lock_with_pid_checker(&paths, |_pid| false);
    assert_eq!(stale.state, RuntimeOwnershipLockState::Stale);
    assert_eq!(stale.record, Some(lock_record));

    fs::write(&paths.runtime_lock_file, "{not-valid-json").unwrap();
    let invalid = inspect_runtime_ownership_lock(&paths);
    assert_eq!(invalid.state, RuntimeOwnershipLockState::Invalid);
    assert!(invalid.record.is_none());
    assert!(
        invalid
            .detail
            .contains("invalid runtime ownership lock payload")
    );

    let wrong_workspace = record("/tmp/other-workspace", 42, "wrong-workspace-session");
    write_record(&paths.runtime_lock_file, &wrong_workspace);
    let wrong = inspect_runtime_ownership_lock_with_pid_checker(&paths, |pid| pid == 42);
    assert_eq!(wrong.state, RuntimeOwnershipLockState::Invalid);
    assert_eq!(wrong.record, Some(wrong_workspace));
    assert!(wrong.detail.contains("different workspace root"));
}

#[test]
fn acquire_writes_deterministic_payload_and_fails_exclusively_when_held() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let acquired =
        acquire_runtime_ownership_lock_with_options(&paths, options(process::id(), "owner-one"))
            .unwrap();

    assert_eq!(acquired.workspace_root, paths.root.to_string_lossy());
    assert_eq!(acquired.owner_pid, process::id());
    assert_eq!(acquired.owner_hostname, "test-host");
    assert_eq!(acquired.owner_session_id, "owner-one");
    assert_eq!(acquired.acquired_at, ACQUIRED_AT);

    let raw = fs::read_to_string(&paths.runtime_lock_file).unwrap();
    assert!(raw.ends_with('\n'));
    let payload: Value = serde_json::from_str(&raw).unwrap();
    let workspace_root = paths.root.to_string_lossy().into_owned();
    assert_eq!(payload["workspace_root"], workspace_root);
    assert_eq!(payload["owner_pid"], process::id());
    assert_eq!(payload["owner_hostname"], "test-host");
    assert_eq!(payload["owner_session_id"], "owner-one");
    assert_eq!(payload["acquired_at"], ACQUIRED_AT);

    let duplicate =
        acquire_runtime_ownership_lock_with_options(&paths, options(process::id(), "owner-two"))
            .unwrap_err();
    match duplicate {
        RuntimeOwnershipLockError::AlreadyHeld { status } => {
            assert_eq!(status.state, RuntimeOwnershipLockState::Active);
            assert_eq!(status.record.unwrap().owner_session_id, "owner-one");
        }
        other => panic!("expected AlreadyHeld, got {other:?}"),
    }
}

#[test]
fn release_supports_matching_session_and_forced_release() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    acquire_runtime_ownership_lock_with_options(&paths, options(process::id(), "owned-session"))
        .unwrap();
    assert!(
        !release_runtime_ownership_lock(&paths, Some("wrong-session"), false).unwrap(),
        "wrong owner session must not release the lock"
    );
    assert!(paths.runtime_lock_file.is_file());

    assert!(release_runtime_ownership_lock(&paths, Some("owned-session"), false).unwrap());
    assert!(!paths.runtime_lock_file.exists());

    acquire_runtime_ownership_lock_with_options(&paths, options(process::id(), "forced-session"))
        .unwrap();
    assert!(release_runtime_ownership_lock(&paths, Some("wrong-session"), true).unwrap());
    assert!(!paths.runtime_lock_file.exists());
}

#[test]
fn release_does_not_delete_wrong_workspace_lock_without_force() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let wrong_workspace = record("/tmp/other-workspace", process::id(), "external-session");
    write_record(&paths.runtime_lock_file, &wrong_workspace);

    assert!(!release_runtime_ownership_lock(&paths, Some("external-session"), false).unwrap());
    assert!(paths.runtime_lock_file.is_file());

    assert!(release_runtime_ownership_lock(&paths, Some("external-session"), true).unwrap());
    assert!(!paths.runtime_lock_file.exists());
}

#[test]
fn clear_removes_only_stale_or_invalid_lock_files() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let unrelated_file = paths.runtime_root.join("operator-note.txt");
    fs::write(&unrelated_file, "preserve me\n").unwrap();

    let missing =
        clear_stale_runtime_ownership_lock_with_pid_checker(&paths, |_pid| false).unwrap();
    assert!(!missing.cleared);
    assert_eq!(missing.reason, "missing");

    let stale_record = record(&paths.root.to_string_lossy(), 999_999, "stale-session");
    write_record(&paths.runtime_lock_file, &stale_record);
    let stale = clear_stale_runtime_ownership_lock_with_pid_checker(&paths, |_pid| false).unwrap();
    assert!(stale.cleared);
    assert_eq!(stale.reason, "cleared_stale");
    assert_eq!(stale.status.state, RuntimeOwnershipLockState::Stale);
    assert!(!paths.runtime_lock_file.exists());
    assert!(unrelated_file.is_file());

    fs::write(&paths.runtime_lock_file, "{not-valid-json").unwrap();
    let invalid =
        clear_stale_runtime_ownership_lock_with_pid_checker(&paths, |_pid| false).unwrap();
    assert!(invalid.cleared);
    assert_eq!(invalid.reason, "cleared_invalid");
    assert_eq!(invalid.status.state, RuntimeOwnershipLockState::Invalid);
    assert!(!paths.runtime_lock_file.exists());
    assert!(unrelated_file.is_file());

    let wrong_workspace = record("/tmp/other-workspace", 42, "wrong-workspace-session");
    write_record(&paths.runtime_lock_file, &wrong_workspace);
    let wrong =
        clear_stale_runtime_ownership_lock_with_pid_checker(&paths, |pid| pid == 42).unwrap();
    assert!(wrong.cleared);
    assert_eq!(wrong.reason, "cleared_invalid");
    assert_eq!(wrong.status.state, RuntimeOwnershipLockState::Invalid);
    assert!(!paths.runtime_lock_file.exists());

    let active_record = record(&paths.root.to_string_lossy(), 42, "active-session");
    write_record(&paths.runtime_lock_file, &active_record);
    let active =
        clear_stale_runtime_ownership_lock_with_pid_checker(&paths, |pid| pid == 42).unwrap();
    assert!(!active.cleared);
    assert_eq!(active.reason, "active_owner");
    assert_eq!(active.status.state, RuntimeOwnershipLockState::Active);
    assert!(paths.runtime_lock_file.is_file());
    assert!(unrelated_file.is_file());
}
