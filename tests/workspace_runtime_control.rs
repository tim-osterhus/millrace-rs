mod support;

use std::{fs, process};

use serde_json::Value;
use tempfile::TempDir;

use millrace_ai::contracts::{
    ActiveRunRequestKind, ActiveRunState, MailboxCommand, MailboxCommandEnvelope, Plane,
    RuntimeJsonContract, SpecDocument, SpecSourceType, StageName, TaskDocument, Timestamp,
    WorkItemKind,
};
use millrace_ai::workspace::{
    RuntimeControl, RuntimeControlMode, RuntimeOwnershipLockOptions,
    acquire_runtime_ownership_lock_with_options, initialize_workspace, load_snapshot,
};

const NOW: &str = "2026-04-15T00:00:00Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("created_at", value).unwrap()
}

fn task_document(task_id: &str) -> TaskDocument {
    TaskDocument {
        task_id: task_id.to_owned(),
        title: format!("Task {task_id}"),
        summary: "runtime control test".to_owned(),
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        spec_id: Some("spec-root-001".to_owned()),
        parent_task_id: None,
        incident_id: None,
        target_paths: vec!["src/workspace/runtime_control.rs".to_owned()],
        acceptance: vec!["runtime control behavior is deterministic".to_owned()],
        required_checks: vec!["cargo test --test workspace_runtime_control".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/runtime/control.py".to_owned()],
        risk: vec!["mailbox contract drift".to_owned()],
        depends_on: Vec::new(),
        blocks: Vec::new(),
        tags: vec!["runtime-control".to_owned()],
        status_hint: None,
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn spec_document(spec_id: &str) -> SpecDocument {
    SpecDocument {
        spec_id: spec_id.to_owned(),
        title: format!("Spec {spec_id}"),
        summary: "runtime control planning intake test".to_owned(),
        source_type: SpecSourceType::Idea,
        source_id: Some("idea-001".to_owned()),
        parent_spec_id: None,
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some(spec_id.to_owned()),
        goals: vec!["route planning intake through mailbox".to_owned()],
        non_goals: Vec::new(),
        scope: vec!["runtime control".to_owned()],
        constraints: vec!["preserve mailbox contract shape".to_owned()],
        assumptions: Vec::new(),
        risks: vec!["mailbox contract drift".to_owned()],
        target_paths: vec!["src/workspace/runtime_control.rs".to_owned()],
        entrypoints: Vec::new(),
        required_skills: Vec::new(),
        decomposition_hints: Vec::new(),
        acceptance: vec!["mailbox payload is deterministic".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/runtime/control.py".to_owned()],
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn active_lock_options(session_id: &str) -> RuntimeOwnershipLockOptions {
    RuntimeOwnershipLockOptions::new(process::id(), "test-host", session_id, NOW).unwrap()
}

fn mailbox_json_paths(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    paths
}

fn read_mailbox(path: &std::path::Path) -> MailboxCommandEnvelope {
    MailboxCommandEnvelope::from_json_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn project_active_planning_run(paths: &millrace_ai::WorkspacePaths) {
    let mut snapshot = load_snapshot(paths).unwrap();
    let active_run = ActiveRunState {
        plane: Plane::Planning,
        stage: StageName::Manager,
        node_id: "manager".to_owned(),
        stage_kind_id: "manager".to_owned(),
        run_id: "run-planning-retry".to_owned(),
        request_kind: ActiveRunRequestKind::ActiveWorkItem,
        work_item_kind: Some(WorkItemKind::Spec),
        work_item_id: Some("spec-active".to_owned()),
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        active_since: timestamp(NOW),
        running_status_marker: None,
    };
    snapshot
        .active_runs_by_plane
        .insert(Plane::Planning, active_run);
    snapshot.active_plane = Some(Plane::Planning);
    snapshot.active_stage = Some(StageName::Manager);
    snapshot.active_node_id = Some("manager".to_owned());
    snapshot.active_stage_kind_id = Some("manager".to_owned());
    snapshot.active_run_id = Some("run-planning-retry".to_owned());
    snapshot.active_work_item_kind = Some(WorkItemKind::Spec);
    snapshot.active_work_item_id = Some("spec-active".to_owned());
    snapshot.active_since = Some(timestamp(NOW));
    millrace_ai::workspace::save_snapshot(paths, &snapshot).unwrap();
}

#[test]
fn direct_offline_pause_and_task_intake_mutate_workspace_state_without_mailbox() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let control = RuntimeControl::from_paths(paths.clone()).unwrap();

    let pause = control.pause_runtime().unwrap();
    assert_eq!(pause.mode, RuntimeControlMode::Direct);
    assert_eq!(pause.action, MailboxCommand::Pause);
    assert!(pause.applied);
    assert!(pause.mailbox_path.is_none());

    let paused = load_snapshot(&paths).unwrap();
    assert!(paused.paused);
    assert!(
        paused
            .pause_sources
            .contains(&millrace_ai::contracts::PauseSource::Operator)
    );
    assert!(mailbox_json_paths(&paths.mailbox_incoming_dir).is_empty());

    let task = task_document("task-runtime-control");
    let add_task = control.add_task(&task).unwrap();
    assert_eq!(add_task.mode, RuntimeControlMode::Direct);
    assert_eq!(add_task.action, MailboxCommand::AddTask);
    assert_eq!(
        add_task.artifact_path,
        Some(paths.tasks_queue_dir.join("task-runtime-control.md"))
    );
    assert!(
        paths
            .tasks_queue_dir
            .join("task-runtime-control.md")
            .is_file()
    );

    let snapshot = load_snapshot(&paths).unwrap();
    assert_eq!(snapshot.queue_depth_execution, 1);
    assert_eq!(
        snapshot.queue_depths_by_plane.get(&Plane::Execution),
        Some(&1)
    );
    assert!(mailbox_json_paths(&paths.mailbox_incoming_dir).is_empty());
}

#[test]
fn direct_resume_preserves_governance_pause_when_blocker_is_active() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let mut snapshot = load_snapshot(&paths).unwrap();
    snapshot.paused = true;
    snapshot.pause_sources = vec![
        millrace_ai::contracts::PauseSource::Operator,
        millrace_ai::contracts::PauseSource::UsageGovernance,
    ];
    millrace_ai::workspace::save_snapshot(&paths, &snapshot).unwrap();
    fs::write(
        &paths.usage_governance_state_file,
        serde_json::json!({
            "version": "1.0",
            "enabled": true,
            "auto_resume": true,
            "auto_resume_possible": true,
            "evaluation_boundary": "between_stages",
            "calendar_timezone": "UTC",
            "daemon_session_id": "control-test",
            "last_evaluated_at": NOW,
            "active_blockers": [{
                "source": "runtime_token",
                "rule_id": "test-rolling",
                "window": "rolling_5h",
                "observed": 125.0,
                "threshold": 100.0,
                "metric": "total_tokens",
                "auto_resume_possible": true,
                "next_auto_resume_at": "2026-04-15T05:00:00Z",
                "detail": ""
            }],
            "paused_by_governance": true,
            "next_auto_resume_at": "2026-04-15T05:00:00Z",
            "subscription_quota_status": {
                "enabled": false,
                "provider": "codex_chatgpt_oauth",
                "state": "disabled",
                "degraded_policy": null,
                "detail": null,
                "last_refreshed_at": null,
                "windows": {}
            }
        })
        .to_string()
            + "\n",
    )
    .unwrap();

    let control = RuntimeControl::from_paths(paths.clone()).unwrap();
    let resume = control.resume_runtime().unwrap();

    assert_eq!(resume.mode, RuntimeControlMode::Direct);
    assert_eq!(resume.action, MailboxCommand::Resume);
    assert!(!resume.applied);
    assert_eq!(resume.detail, "runtime resume blocked by usage governance");
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.paused);
    assert_eq!(
        snapshot.pause_sources,
        vec![millrace_ai::contracts::PauseSource::UsageGovernance]
    );
}

#[test]
fn active_daemon_lock_routes_control_and_queue_intake_to_mailbox_envelopes() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    acquire_runtime_ownership_lock_with_options(&paths, active_lock_options("daemon-session"))
        .unwrap();
    let control = RuntimeControl::from_paths(paths.clone()).unwrap();

    let pause = control.pause_runtime_with_issuer("operator").unwrap();
    assert_eq!(pause.mode, RuntimeControlMode::Mailbox);
    assert!(!pause.applied);
    let pause_path = pause.mailbox_path.as_ref().unwrap();
    assert_eq!(
        pause_path.file_name().unwrap().to_string_lossy(),
        format!("{}.json", pause.command_id.as_ref().unwrap())
    );
    let pause_envelope = read_mailbox(pause_path);
    assert_eq!(pause_envelope.kind, "mailbox_command");
    assert_eq!(pause_envelope.command, MailboxCommand::Pause);
    assert_eq!(pause_envelope.issuer, "operator");
    assert!(pause_envelope.payload.is_empty());

    let task = task_document("task-mailbox-routed");
    let add_task = control.add_task(&task).unwrap();
    assert_eq!(add_task.mode, RuntimeControlMode::Mailbox);
    let add_task_envelope = read_mailbox(add_task.mailbox_path.as_ref().unwrap());
    assert_eq!(add_task_envelope.command, MailboxCommand::AddTask);
    assert_eq!(
        add_task_envelope.payload["document"]["task_id"],
        Value::String("task-mailbox-routed".to_owned())
    );
    assert!(
        !paths
            .tasks_queue_dir
            .join("task-mailbox-routed.md")
            .exists()
    );
    assert!(
        paths
            .mailbox_processed_dir
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );
    assert!(
        paths
            .mailbox_failed_dir
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );

    let snapshot = load_snapshot(&paths).unwrap();
    assert!(!snapshot.paused);
    assert_eq!(snapshot.queue_depth_execution, 0);
}

#[test]
fn active_daemon_lock_routes_remaining_control_commands_to_mailbox_envelopes() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    project_active_planning_run(&paths);
    acquire_runtime_ownership_lock_with_options(&paths, active_lock_options("daemon-session"))
        .unwrap();
    let control = RuntimeControl::from_paths(paths.clone()).unwrap();

    assert_eq!(
        control.resume_runtime().unwrap().mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control.stop_runtime().unwrap().mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control.retry_active("try active").unwrap().mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control.retry_active_planning("try planning").unwrap().mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control.clear_stale_state("cleanup").unwrap().mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control.reload_config().unwrap().mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control
            .add_spec(&spec_document("spec-mailbox-routed"))
            .unwrap()
            .mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control
            .add_idea_markdown("idea-mailbox-routed.md", "# Idea\n")
            .unwrap()
            .mode,
        RuntimeControlMode::Mailbox
    );

    let envelopes: Vec<_> = mailbox_json_paths(&paths.mailbox_incoming_dir)
        .iter()
        .map(|path| read_mailbox(path))
        .collect();
    let commands: Vec<_> = envelopes.iter().map(|envelope| envelope.command).collect();
    assert_eq!(
        commands,
        vec![
            MailboxCommand::AddIdea,
            MailboxCommand::AddSpec,
            MailboxCommand::ClearStaleState,
            MailboxCommand::ReloadConfig,
            MailboxCommand::Resume,
            MailboxCommand::RetryActive,
            MailboxCommand::RetryActive,
            MailboxCommand::Stop,
        ]
    );
    let planning_retry = envelopes
        .iter()
        .find(|envelope| envelope.payload.get("scope").is_some())
        .unwrap();
    assert_eq!(planning_retry.payload["scope"], "planning");
    assert_eq!(planning_retry.payload["reason"], "try planning");
    assert_eq!(
        envelopes
            .iter()
            .find(|envelope| envelope.command == MailboxCommand::AddSpec)
            .unwrap()
            .payload["document"]["spec_id"],
        "spec-mailbox-routed"
    );
    assert_eq!(
        envelopes
            .iter()
            .find(|envelope| envelope.command == MailboxCommand::AddIdea)
            .unwrap()
            .payload["source_name"],
        "idea-mailbox-routed.md"
    );
    assert!(
        !paths
            .specs_queue_dir
            .join("spec-mailbox-routed.md")
            .exists()
    );
    assert!(
        !paths
            .root
            .join("ideas/inbox/idea-mailbox-routed.md")
            .exists()
    );
}

#[test]
fn invalid_lock_is_not_treated_as_daemon_ownership_and_clear_stale_repairs_it_directly() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    fs::write(&paths.runtime_lock_file, "{not-valid-json").unwrap();
    let control = RuntimeControl::from_paths(paths.clone()).unwrap();

    let result = control.clear_stale_state("operator cleanup").unwrap();

    assert_eq!(result.mode, RuntimeControlMode::Direct);
    assert!(result.applied);
    assert!(
        result
            .detail
            .contains("runtime_ownership_lock=cleared_invalid")
    );
    assert!(!paths.runtime_lock_file.exists());
    assert!(mailbox_json_paths(&paths.mailbox_incoming_dir).is_empty());
}

#[test]
fn uninitialized_workspace_is_refused_without_creating_runtime_layout() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");

    let error = RuntimeControl::new(&root).unwrap_err();

    assert!(error.to_string().contains("workspace is not initialized"));
    assert!(!root.join("millrace-agents").exists());
}

#[test]
fn invalid_or_duplicate_inputs_do_not_write_mailbox_or_replace_existing_idea() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let control = RuntimeControl::from_paths(paths.clone()).unwrap();

    let invalid = control
        .add_idea_markdown("../bad.md", "# Bad idea\n")
        .unwrap_err();
    assert!(invalid.to_string().contains("source_name"));
    assert!(!paths.root.join("ideas").exists());
    assert!(mailbox_json_paths(&paths.mailbox_incoming_dir).is_empty());

    let first = control
        .add_idea_markdown("idea-runtime-control.md", "# First\n")
        .unwrap();
    assert_eq!(first.mode, RuntimeControlMode::Direct);
    let idea_path = paths.root.join("ideas/inbox/idea-runtime-control.md");
    assert_eq!(fs::read_to_string(&idea_path).unwrap(), "# First\n");

    let duplicate = control
        .add_idea_markdown("idea-runtime-control.md", "# Second\n")
        .unwrap_err();
    assert!(duplicate.to_string().contains("already exists"));
    assert_eq!(fs::read_to_string(&idea_path).unwrap(), "# First\n");
    assert!(mailbox_json_paths(&paths.mailbox_incoming_dir).is_empty());
}

#[test]
fn retry_active_requeues_single_active_work_item_and_clears_runtime_projection() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let control = RuntimeControl::from_paths(paths.clone()).unwrap();
    let task = task_document("task-retry-active");
    control.add_task(&task).unwrap();
    let claim = millrace_ai::workspace::claim_next_execution_task(&paths, None)
        .unwrap()
        .unwrap();
    assert_eq!(claim.work_item_kind, WorkItemKind::Task);

    let mut snapshot = load_snapshot(&paths).unwrap();
    snapshot.active_plane = Some(Plane::Execution);
    snapshot.active_stage = Some(millrace_ai::contracts::StageName::Builder);
    snapshot.active_node_id = Some("builder".to_owned());
    snapshot.active_stage_kind_id = Some("builder".to_owned());
    snapshot.active_run_id = Some("run-001".to_owned());
    snapshot.active_work_item_kind = Some(WorkItemKind::Task);
    snapshot.active_work_item_id = Some("task-retry-active".to_owned());
    snapshot.active_since = Some(timestamp(NOW));
    millrace_ai::workspace::save_snapshot(&paths, &snapshot).unwrap();

    let result = control.retry_active("try again").unwrap();

    assert_eq!(result.mode, RuntimeControlMode::Direct);
    assert!(result.applied);
    assert!(paths.tasks_queue_dir.join("task-retry-active.md").is_file());
    assert!(!paths.tasks_active_dir.join("task-retry-active.md").exists());
    let snapshot = load_snapshot(&paths).unwrap();
    assert!(snapshot.active_stage.is_none());
    assert_eq!(snapshot.execution_status_marker, "### IDLE");
}
