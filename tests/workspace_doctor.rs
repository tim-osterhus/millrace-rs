mod support;

use std::{fs, process};

use tempfile::TempDir;

use millrace_ai::contracts::{TaskDocument, Timestamp};
use millrace_ai::work_documents::render_task_document;
use millrace_ai::workspace::{
    RuntimeOwnershipLockOptions, acquire_runtime_ownership_lock_with_options, initialize_workspace,
    run_workspace_doctor_for_paths,
};

const NOW: &str = "2026-04-15T00:00:00Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("created_at", value).unwrap()
}

fn task_document(task_id: &str) -> TaskDocument {
    TaskDocument {
        task_id: task_id.to_owned(),
        title: format!("Task {task_id}"),
        summary: "doctor queue artifact".to_owned(),
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        spec_id: Some("spec-root-001".to_owned()),
        parent_task_id: None,
        incident_id: None,
        target_paths: vec!["src/workspace/doctor.rs".to_owned()],
        acceptance: vec!["doctor flags queue artifacts".to_owned()],
        required_checks: vec!["cargo test --test workspace_doctor".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/doctor.py".to_owned()],
        risk: vec!["queue drift".to_owned()],
        depends_on: Vec::new(),
        blocks: Vec::new(),
        tags: vec!["doctor".to_owned()],
        status_hint: None,
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn has_error(report: &millrace_ai::DoctorReport, code: &str) -> bool {
    report.errors.iter().any(|issue| issue.code == code)
}

fn has_warning(report: &millrace_ai::DoctorReport, code: &str) -> bool {
    report.warnings.iter().any(|issue| issue.code == code)
}

fn lock_options(pid: u32, session_id: &str) -> RuntimeOwnershipLockOptions {
    RuntimeOwnershipLockOptions::new(pid, "test-host", session_id, NOW).unwrap()
}

#[test]
fn doctor_passes_for_healthy_initialized_workspace() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let report = run_workspace_doctor_for_paths(&paths);

    assert!(report.ok, "unexpected doctor errors: {:#?}", report.errors);
    assert!(report.errors.is_empty());
}

#[test]
fn doctor_flags_missing_directories_and_files() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    fs::remove_dir(&paths.logs_dir).unwrap();
    fs::remove_file(&paths.outline_file).unwrap();

    let report = run_workspace_doctor_for_paths(&paths);

    assert!(!report.ok);
    assert!(has_error(&report, "missing_directory"));
    assert!(has_error(&report, "missing_file"));
}

#[test]
fn doctor_flags_invalid_status_json_and_baseline_state() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    fs::write(&paths.execution_status_file, "RUNNING\n").unwrap();
    fs::write(&paths.runtime_snapshot_file, "{not-valid-json").unwrap();
    fs::write(
        &paths.baseline_manifest_file,
        serde_json::json!({
            "schema_version": "2.0",
            "manifest_id": "bad",
            "seed_package_version": "0.0.0",
            "entries": []
        })
        .to_string()
            + "\n",
    )
    .unwrap();

    let report = run_workspace_doctor_for_paths(&paths);

    assert!(!report.ok);
    assert!(has_error(&report, "execution_status_invalid"));
    assert!(has_error(&report, "snapshot_invalid"));
    assert!(has_error(&report, "baseline_manifest_invalid"));
}

#[test]
fn doctor_flags_missing_manifest_tracked_managed_assets() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    fs::remove_file(paths.runtime_root.join("entrypoints/execution/builder.md")).unwrap();

    let report = run_workspace_doctor_for_paths(&paths);

    assert!(!report.ok);
    assert!(has_error(&report, "baseline_manifest_managed_file_missing"));
}

#[test]
fn doctor_flags_unparseable_queue_artifacts_and_filename_id_mismatches() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    fs::write(
        paths.tasks_queue_dir.join("bad.md"),
        "# Bad task\nnot a valid task document\n",
    )
    .unwrap();
    fs::write(
        paths.tasks_queue_dir.join("task-alias.md"),
        render_task_document(&task_document("task-mismatch")),
    )
    .unwrap();

    let report = run_workspace_doctor_for_paths(&paths);

    assert!(!report.ok);
    let queue_errors: Vec<_> = report
        .errors
        .iter()
        .filter(|issue| issue.code == "queue_artifact_invalid")
        .collect();
    assert_eq!(queue_errors.len(), 2);
    assert!(queue_errors.iter().any(|issue| {
        issue
            .message
            .contains("filename stem does not match task_id")
    }));
}

#[test]
fn doctor_flags_duplicate_task_lifecycle_state_with_workspace_relative_paths() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let document = task_document("task-duplicate");
    fs::write(
        paths.tasks_done_dir.join("task-duplicate.md"),
        render_task_document(&document),
    )
    .unwrap();
    let mut blocked = document.clone();
    blocked.summary = "stale blocked predecessor".to_owned();
    fs::write(
        paths.tasks_blocked_dir.join("task-duplicate.md"),
        render_task_document(&blocked),
    )
    .unwrap();

    let report = run_workspace_doctor_for_paths(&paths);

    assert!(!report.ok);
    let duplicate = report
        .errors
        .iter()
        .find(|issue| issue.code == "duplicate_task_lifecycle_state")
        .expect("duplicate task lifecycle diagnostic");
    assert_eq!(
        duplicate.message,
        "task task-duplicate appears in multiple lifecycle states: done:millrace-agents/tasks/done/task-duplicate.md, blocked:millrace-agents/tasks/blocked/task-duplicate.md"
    );
    assert_eq!(
        duplicate.path.as_ref(),
        Some(&paths.tasks_done_dir.join("task-duplicate.md"))
    );
}

#[test]
fn doctor_reports_runtime_ownership_lock_health() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    acquire_runtime_ownership_lock_with_options(
        &paths,
        lock_options(process::id(), "active-session"),
    )
    .unwrap();
    let active = run_workspace_doctor_for_paths(&paths);
    assert!(active.ok);
    assert!(has_warning(&active, "runtime_ownership_lock_active"));

    fs::remove_file(&paths.runtime_lock_file).unwrap();
    acquire_runtime_ownership_lock_with_options(&paths, lock_options(999_999_999, "stale-session"))
        .unwrap();
    let stale = run_workspace_doctor_for_paths(&paths);
    assert!(!stale.ok);
    assert!(has_error(&stale, "runtime_ownership_lock_stale"));

    fs::write(&paths.runtime_lock_file, "{not-valid-json").unwrap();
    let invalid = run_workspace_doctor_for_paths(&paths);
    assert!(!invalid.ok);
    assert!(has_error(&invalid, "runtime_ownership_lock_invalid"));
}
