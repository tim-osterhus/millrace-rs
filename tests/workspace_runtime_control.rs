mod support;

use std::{collections::BTreeSet, fs, process};

use serde_json::Value;
use tempfile::TempDir;

use millrace_ai::contracts::{
    ActiveRunRequestKind, ActiveRunState, MailboxCommand, MailboxCommandEnvelope,
    MailboxSupersedeCascade, Plane, ProbeDocument, RuntimeJsonContract, SpecDocument,
    SpecSourceType, StageName, TaskDocument, Timestamp, WorkItemKind,
};
use millrace_ai::work_documents::parse_task_document;
use millrace_ai::workspace::{
    QueueStore, RuntimeControl, RuntimeControlMode, RuntimeOwnershipLockOptions,
    acquire_runtime_ownership_lock_with_options, initialize_workspace, load_snapshot,
};
use support::parity::read_json_fixture;

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
        root_intake_kind: None,
        root_intake_id: None,
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
        root_intake_kind: None,
        root_intake_id: None,
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

fn probe_document(probe_id: &str) -> ProbeDocument {
    ProbeDocument {
        probe_id: probe_id.to_owned(),
        title: format!("Probe {probe_id}"),
        summary: "runtime control probe intake test".to_owned(),
        request: "Research the codebase and route the smallest safe change.".to_owned(),
        target_paths: vec!["src/workspace/runtime_control.rs".to_owned()],
        constraints: vec!["Do not implement during recon.".to_owned()],
        acceptance: vec!["probe intake updates planning depth".to_owned()],
        risk_notes: vec!["mailbox contract drift".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/runtime/control.py".to_owned()],
        tags: vec!["runtime-control".to_owned()],
        status_hint: None,
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
fn workspace_runtime_control_v0_18_6_guardrail_fixture_requires_intervention_mailbox_payloads() {
    let fixture = read_json_fixture("runtime_json/auto_port_v0_18_6_runtime_contract_scout.json");
    assert_eq!(fixture["kind"], "auto_port_v0_18_6_runtime_contract_scout");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.6");
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.5");

    let mailbox = &fixture["mailbox_intervention_contract"];
    let commands: BTreeSet<_> = mailbox["command_values"]
        .as_array()
        .expect("mailbox command values are present")
        .iter()
        .map(|value| value.as_str().expect("mailbox command"))
        .collect();
    assert_eq!(
        commands,
        BTreeSet::from([
            "archive_blocked_task",
            "archive_invalid_incident",
            "cancel_incident",
            "cancel_work_item",
            "resolve_incident",
            "retarget_task_dependency",
            "supersede_task",
        ])
    );
    assert_eq!(
        mailbox["payload_fields"]["cancel_work_item"],
        serde_json::json!(["work_item_id", "work_item_kind", "reason", "force"])
    );
    assert_eq!(
        mailbox["payload_fields"]["archive_invalid_incident"],
        serde_json::json!(["filename", "reason"])
    );
    assert_eq!(
        mailbox["payload_fields"]["retarget_task_dependency"],
        serde_json::json!([
            "task_id",
            "old_dependency_id",
            "new_dependency_id",
            "reason"
        ])
    );
    for failure in [
        "empty_reason",
        "unsafe_work_item_id",
        "invalid_work_item_kind",
        "invalid_cascade",
        "unsafe_invalid_incident_filename",
    ] {
        assert!(
            mailbox["validation_failures"]
                .as_array()
                .expect("validation failures are present")
                .iter()
                .any(|value| value.as_str() == Some(failure)),
            "missing v0.18.6 intervention payload validation failure {failure}"
        );
    }

    let runtime_control = &fixture["runtime_control_contract"];
    let methods: BTreeSet<_> = runtime_control["direct_methods"]
        .as_array()
        .expect("runtime control methods are present")
        .iter()
        .map(|value| value.as_str().expect("runtime control method"))
        .collect();
    assert_eq!(methods, commands);
    assert_eq!(
        runtime_control["routing_modes"],
        serde_json::json!(["direct", "mailbox"])
    );
    assert_eq!(
        runtime_control["direct_active_mutation_guard"],
        "active runtime stage prevents operator intervention"
    );
    assert_eq!(
        runtime_control["daemon_mailbox_guard"],
        "daemon ownership routes intervention commands through mailbox envelopes"
    );
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

    let probe = probe_document("probe-runtime-control");
    let add_probe = control.add_probe(&probe).unwrap();
    assert_eq!(add_probe.mode, RuntimeControlMode::Direct);
    assert_eq!(add_probe.action, MailboxCommand::AddProbe);
    assert_eq!(
        add_probe.artifact_path,
        Some(paths.probes_queue_dir.join("probe-runtime-control.md"))
    );

    let snapshot = load_snapshot(&paths).unwrap();
    assert_eq!(snapshot.queue_depth_planning, 1);
    assert_eq!(
        snapshot.queue_depths_by_plane.get(&Plane::Planning),
        Some(&1)
    );
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

    let probe = probe_document("probe-mailbox-routed");
    let add_probe = control.add_probe(&probe).unwrap();
    assert_eq!(add_probe.mode, RuntimeControlMode::Mailbox);
    let add_probe_envelope = read_mailbox(add_probe.mailbox_path.as_ref().unwrap());
    assert_eq!(add_probe_envelope.command, MailboxCommand::AddProbe);
    assert_eq!(
        add_probe_envelope.payload["document"]["probe_id"],
        Value::String("probe-mailbox-routed".to_owned())
    );
    assert!(
        !paths
            .probes_queue_dir
            .join("probe-mailbox-routed.md")
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
            .add_probe(&probe_document("probe-mailbox-routed"))
            .unwrap()
            .mode,
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
            MailboxCommand::AddProbe,
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
            .find(|envelope| envelope.command == MailboxCommand::AddProbe)
            .unwrap()
            .payload["document"]["probe_id"],
        "probe-mailbox-routed"
    );
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
            .probes_queue_dir
            .join("probe-mailbox-routed.md")
            .exists()
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
fn active_daemon_lock_routes_operator_intervention_commands_to_mailbox_envelopes() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    acquire_runtime_ownership_lock_with_options(
        &paths,
        active_lock_options("intervention-daemon-session"),
    )
    .unwrap();
    let control = RuntimeControl::from_paths(paths.clone()).unwrap();

    assert_eq!(
        control
            .cancel_work_item_with_options(
                "task-cancel-mailbox",
                Some(WorkItemKind::Task),
                "operator cancelled bad intake",
                false,
                "operator",
            )
            .unwrap()
            .mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control
            .archive_blocked_task("task-blocked-mailbox", "operator archived obsolete task")
            .unwrap()
            .mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control
            .supersede_task_with_cascade(
                "task-old-mailbox",
                "task-new-mailbox",
                "operator corrected task scope",
                MailboxSupersedeCascade::Cancel,
                "operator",
            )
            .unwrap()
            .mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control
            .retarget_task_dependency(
                "task-dependent-mailbox",
                "task-old-mailbox",
                "task-new-mailbox",
                "operator retargeted dependency",
            )
            .unwrap()
            .mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control
            .resolve_incident("incident-resolve-mailbox", "operator resolved incident")
            .unwrap()
            .mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control
            .cancel_incident("incident-cancel-mailbox", "operator cancelled incident")
            .unwrap()
            .mode,
        RuntimeControlMode::Mailbox
    );
    assert_eq!(
        control
            .archive_invalid_incident(
                "incident-invalid.md",
                "operator archived invalid incident artifact",
            )
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
            MailboxCommand::ArchiveBlockedTask,
            MailboxCommand::ArchiveInvalidIncident,
            MailboxCommand::CancelIncident,
            MailboxCommand::CancelWorkItem,
            MailboxCommand::ResolveIncident,
            MailboxCommand::RetargetTaskDependency,
            MailboxCommand::SupersedeTask,
        ]
    );

    let cancel = envelopes
        .iter()
        .find(|envelope| envelope.command == MailboxCommand::CancelWorkItem)
        .unwrap();
    assert_eq!(cancel.payload["work_item_id"], "task-cancel-mailbox");
    assert_eq!(cancel.payload["work_item_kind"], "task");
    assert_eq!(cancel.payload["reason"], "operator cancelled bad intake");
    assert_eq!(cancel.payload["force"], false);

    let supersede = envelopes
        .iter()
        .find(|envelope| envelope.command == MailboxCommand::SupersedeTask)
        .unwrap();
    assert_eq!(supersede.payload["old_task_id"], "task-old-mailbox");
    assert_eq!(supersede.payload["replacement_task_id"], "task-new-mailbox");
    assert_eq!(supersede.payload["cascade"], "cancel");

    let retarget = envelopes
        .iter()
        .find(|envelope| envelope.command == MailboxCommand::RetargetTaskDependency)
        .unwrap();
    assert_eq!(retarget.payload["task_id"], "task-dependent-mailbox");
    assert_eq!(retarget.payload["old_dependency_id"], "task-old-mailbox");
    assert_eq!(retarget.payload["new_dependency_id"], "task-new-mailbox");

    let archive_invalid = envelopes
        .iter()
        .find(|envelope| envelope.command == MailboxCommand::ArchiveInvalidIncident)
        .unwrap();
    assert_eq!(archive_invalid.payload["filename"], "incident-invalid.md");
}

#[test]
fn direct_operator_intervention_commands_refuse_active_runtime_stage() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let mut snapshot = load_snapshot(&paths).unwrap();
    snapshot.active_plane = Some(Plane::Execution);
    snapshot.active_stage = Some(StageName::Builder);
    snapshot.active_node_id = Some("builder".to_owned());
    snapshot.active_stage_kind_id = Some("builder".to_owned());
    snapshot.active_run_id = Some("run-active".to_owned());
    snapshot.active_work_item_kind = Some(WorkItemKind::Task);
    snapshot.active_work_item_id = Some("task-active".to_owned());
    snapshot.active_since = Some(timestamp(NOW));
    millrace_ai::workspace::save_snapshot(&paths, &snapshot).unwrap();
    let control = RuntimeControl::from_paths(paths.clone()).unwrap();

    let result = control
        .cancel_work_item("task-active", "operator cancelled active task")
        .unwrap();

    assert_eq!(result.mode, RuntimeControlMode::Direct);
    assert_eq!(result.action, MailboxCommand::CancelWorkItem);
    assert!(!result.applied);
    assert!(
        result
            .detail
            .contains("active runtime stage prevents operator intervention")
    );
    assert!(mailbox_json_paths(&paths.mailbox_incoming_dir).is_empty());
}

#[test]
fn direct_operator_intervention_supersede_retargets_and_refreshes_snapshot() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-old")).unwrap();
    queue.enqueue_task(&task_document("task-new")).unwrap();
    let mut dependent = task_document("task-dependent");
    dependent.depends_on = vec!["task-old".to_owned()];
    queue.enqueue_task(&dependent).unwrap();
    let mut snapshot = load_snapshot(&paths).unwrap();
    snapshot.queue_depth_execution = 999;
    snapshot.queue_depths_by_plane.insert(Plane::Execution, 999);
    millrace_ai::workspace::save_snapshot(&paths, &snapshot).unwrap();
    let control = RuntimeControl::from_paths(paths.clone()).unwrap();

    let result = control
        .supersede_task_with_cascade(
            "task-old",
            "task-new",
            "replacement task has corrected scope",
            MailboxSupersedeCascade::Retarget,
            "operator",
        )
        .unwrap();

    assert_eq!(result.mode, RuntimeControlMode::Direct);
    assert_eq!(result.action, MailboxCommand::SupersedeTask);
    assert!(result.applied);
    assert!(result.detail.contains("task_superseded: task task-old"));
    assert!(result.detail.contains("replacement=task-new"));
    assert!(result.detail.contains("affected_dependents=task-dependent"));
    assert_eq!(
        result.artifact_path.as_ref().unwrap().parent().unwrap(),
        paths.tasks_queue_dir.join("superseded")
    );
    assert!(!paths.tasks_queue_dir.join("task-old.md").exists());

    let dependent = parse_task_document(
        &fs::read_to_string(paths.tasks_queue_dir.join("task-dependent.md")).unwrap(),
    )
    .unwrap();
    assert_eq!(dependent.depends_on, vec!["task-new".to_owned()]);
    let snapshot = load_snapshot(&paths).unwrap();
    assert_eq!(snapshot.queue_depth_execution, 2);
    assert_eq!(
        snapshot.queue_depths_by_plane.get(&Plane::Execution),
        Some(&2)
    );
    assert!(
        fs::read_to_string(paths.logs_dir.join("runtime_events.jsonl"))
            .unwrap()
            .contains("\"event_type\":\"task_superseded\"")
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
