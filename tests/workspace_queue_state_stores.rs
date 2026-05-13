mod support;

use std::path::Path;
use std::process;
use std::{collections::BTreeSet, fs};

use serde_json::Value;
use tempfile::TempDir;

use millrace_ai::contracts::{
    ClosureTargetState, IncidentDecision, IncidentDocument, LearningRequestAction,
    LearningRequestDocument, Plane, ProbeDocument, RecoveryCounterEntry, RecoveryCounters,
    SpecDocument, SpecSourceType, StageName, TaskDocument, Timestamp, WorkItemKind,
};
use millrace_ai::work_documents::{
    parse_incident_document, parse_learning_request_document, parse_task_document,
    render_incident_document, render_learning_request_document, render_probe_document,
    render_spec_document, render_task_document,
};
use millrace_ai::workspace::{
    LineageRepairError, QueueStore, QueueStoreError, RuntimeOwnershipLockOptions, StateStore,
    StateStoreError, acquire_runtime_ownership_lock_with_options,
    find_duplicate_task_lifecycle_ids, initialize_workspace, load_closure_target_state,
    repair_closure_lineage, save_closure_target_state,
};
use support::parity::read_json_fixture;

const NOW: &str = "2026-04-15T00:00:00Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("created_at", value).unwrap()
}

fn task_document(task_id: &str, created_at: &str) -> TaskDocument {
    TaskDocument {
        task_id: task_id.to_owned(),
        title: format!("Task {task_id}"),
        summary: "queue test".to_owned(),
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        spec_id: Some("spec-root-001".to_owned()),
        parent_task_id: None,
        incident_id: None,
        target_paths: vec!["src/workspace/queue_store.rs".to_owned()],
        acceptance: vec!["queue behavior is deterministic".to_owned()],
        required_checks: vec!["cargo test --test workspace_queue_state_stores".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/workspace/queue_store.py".to_owned()],
        risk: vec!["queue drift".to_owned()],
        depends_on: Vec::new(),
        blocks: Vec::new(),
        tags: vec!["queue-store".to_owned()],
        status_hint: None,
        created_at: timestamp(created_at),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn spec_document(spec_id: &str, created_at: &str) -> SpecDocument {
    SpecDocument {
        spec_id: spec_id.to_owned(),
        title: format!("Spec {spec_id}"),
        summary: "planning input".to_owned(),
        source_type: SpecSourceType::Manual,
        source_id: None,
        parent_spec_id: None,
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        goals: vec!["define implementation plan".to_owned()],
        non_goals: Vec::new(),
        scope: Vec::new(),
        constraints: vec!["stay deterministic".to_owned()],
        assumptions: Vec::new(),
        risks: Vec::new(),
        target_paths: vec!["src/workspace/state_store.rs".to_owned()],
        entrypoints: Vec::new(),
        required_skills: Vec::new(),
        decomposition_hints: Vec::new(),
        acceptance: vec!["planning queue works".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/workspace/queue_store.py".to_owned()],
        created_at: timestamp(created_at),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn probe_document(probe_id: &str, created_at: &str) -> ProbeDocument {
    ProbeDocument {
        probe_id: probe_id.to_owned(),
        title: format!("Probe {probe_id}"),
        summary: "ambiguous repo-facing request".to_owned(),
        request: "Research the codebase and route the smallest safe change.".to_owned(),
        target_paths: vec!["src/example/parser.rs".to_owned()],
        constraints: vec!["Do not implement during recon.".to_owned()],
        acceptance: vec!["Recon routes the probe with a durable packet.".to_owned()],
        risk_notes: vec!["Parser changes can regress adjacent behavior.".to_owned()],
        references: vec!["operator request".to_owned()],
        tags: vec!["probe".to_owned()],
        status_hint: None,
        created_at: timestamp(created_at),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn incident_document(incident_id: &str, opened_at: &str) -> IncidentDocument {
    IncidentDocument {
        incident_id: incident_id.to_owned(),
        title: format!("Incident {incident_id}"),
        summary: "execution recovery".to_owned(),
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        source_task_id: None,
        source_spec_id: Some("spec-root-001".to_owned()),
        source_stage: StageName::Consultant,
        source_plane: millrace_ai::contracts::Plane::Execution,
        failure_class: "malformed_output".to_owned(),
        severity: millrace_ai::contracts::IncidentSeverity::Medium,
        needs_planning: true,
        trigger_reason: "bad terminal marker".to_owned(),
        observed_symptoms: Vec::new(),
        failed_attempts: Vec::new(),
        consultant_decision: IncidentDecision::NeedsPlanning,
        evidence_paths: Vec::new(),
        related_run_ids: Vec::new(),
        related_stage_results: Vec::new(),
        references: Vec::new(),
        opened_at: timestamp(opened_at),
        opened_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn learning_request_document(
    learning_request_id: &str,
    created_at: &str,
) -> LearningRequestDocument {
    LearningRequestDocument {
        learning_request_id: learning_request_id.to_owned(),
        title: format!("Learning request {learning_request_id}"),
        summary: "skill update".to_owned(),
        requested_action: LearningRequestAction::Improve,
        target_skill_id: Some("checker-core".to_owned()),
        target_stage: None,
        source_refs: vec!["run:run-001".to_owned()],
        preferred_output_paths: Vec::new(),
        trigger_metadata: serde_json::json!({"source": "test"}),
        originating_run_ids: vec!["run-001".to_owned()],
        artifact_paths: Vec::new(),
        references: Vec::new(),
        created_at: timestamp(created_at),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn read_json_lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn workspace_queue_v0_18_4_guardrail_fixture_requires_blocked_retry_audit_surface() {
    let fixture = read_json_fixture("cli_parity/auto_port_v0_18_4_parity_evidence.json");
    assert_eq!(fixture["kind"], "auto_port_v0_18_4_parity_evidence");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.4");

    let mappings = fixture["changed_path_mappings"]
        .as_array()
        .expect("changed path mappings are present");
    for source in [
        "src/millrace_ai/workspace/queue_store.py",
        "src/millrace_ai/workspace/queue_transitions.py",
        "tests/workspace/test_queue_store.py",
    ] {
        assert!(
            mappings.iter().any(|mapping| {
                mapping["python_path"].as_str() == Some(source)
                    && mapping["surface"].as_str() == Some("workspace_queue_blocked_requeue")
            }),
            "missing v0.18.4 workspace queue blocked-requeue mapping for {source}"
        );
    }

    let queue_retry = &fixture["queue_retry_behavior"];
    assert_eq!(
        queue_retry["queue_log_path_template"],
        "millrace-agents/tasks/queue/<TASK_ID>.requeue.jsonl"
    );
    let audit_fields: BTreeSet<_> = queue_retry["audit_fields"]
        .as_array()
        .expect("queue retry audit fields are present")
        .iter()
        .map(|value| value.as_str().expect("audit field"))
        .collect();
    assert_eq!(
        audit_fields,
        BTreeSet::from([
            "at",
            "actor",
            "attempt_number",
            "auto",
            "destination_state",
            "failure_class",
            "kind",
            "reason",
            "source_state",
        ])
    );

    let required_guards: BTreeSet<_> = queue_retry["required_guards"]
        .as_array()
        .expect("queue retry guards are present")
        .iter()
        .map(|value| value.as_str().expect("queue retry guard"))
        .collect();
    for guard in [
        "safe work item id parsing",
        "live daemon lock refusal",
        "root spec guard",
        "retryability check",
        "retry budget check",
        "force override",
        "queue depth snapshot refresh",
    ] {
        assert!(
            required_guards.contains(guard),
            "missing v0.18.4 blocked retry queue guard {guard}"
        );
    }

    let blocked = &fixture["blocked_metadata_contract"];
    for field in [
        "task_id",
        "source_path",
        "destination_path",
        "source_state",
        "destination_state",
        "actor",
        "auto",
        "reason",
        "failure_class",
        "attempt_number",
        "diagnostics_path",
    ] {
        assert!(
            blocked["result_fields"]
                .as_array()
                .expect("blocked requeue result fields are present")
                .iter()
                .any(|value| value.as_str() == Some(field)),
            "missing v0.18.4 blocked requeue result field {field}"
        );
    }
}

#[test]
fn requeue_blocked_task_moves_task_to_queue_and_writes_retry_audit_log() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = QueueStore::from_paths(paths.clone());
    store
        .enqueue_task(&task_document("task-retry", NOW))
        .unwrap();
    store.claim_next_execution_task(None).unwrap().unwrap();
    store.mark_task_blocked("task-retry").unwrap();

    let destination = store
        .requeue_blocked_task(
            "task-retry",
            " retry after network_unavailable ",
            "operator",
            false,
            Some("network_unavailable"),
            Some(1),
        )
        .unwrap();

    assert_eq!(destination, paths.tasks_queue_dir.join("task-retry.md"));
    assert!(destination.is_file());
    assert!(!paths.tasks_blocked_dir.join("task-retry.md").exists());
    let audit = read_json_lines(&paths.tasks_queue_dir.join("task-retry.requeue.jsonl"));
    assert_eq!(audit.len(), 1);
    assert!(!audit[0]["at"].as_str().unwrap().is_empty());
    assert_eq!(audit[0]["actor"], "operator");
    assert_eq!(audit[0]["attempt_number"], 1);
    assert_eq!(audit[0]["auto"], false);
    assert_eq!(audit[0]["destination_state"], "queue");
    assert_eq!(audit[0]["failure_class"], "network_unavailable");
    assert_eq!(audit[0]["kind"], "task");
    assert_eq!(audit[0]["reason"], "retry after network_unavailable");
    assert_eq!(audit[0]["source_state"], "blocked");
}

fn closure_target_state(root_spec_id: &str, root_idea_id: &str) -> ClosureTargetState {
    ClosureTargetState {
        schema_version: "1.0".to_owned(),
        kind: "closure_target_state".to_owned(),
        root_spec_id: root_spec_id.to_owned(),
        root_idea_id: root_idea_id.to_owned(),
        root_intake_kind: None,
        root_intake_id: None,
        root_spec_path: format!("millrace-agents/arbiter/contracts/root-specs/{root_spec_id}.md"),
        root_idea_path: format!("millrace-agents/arbiter/contracts/ideas/{root_idea_id}.md"),
        rubric_path: format!("millrace-agents/arbiter/rubrics/{root_spec_id}.md"),
        latest_verdict_path: Some(format!(
            "millrace-agents/arbiter/verdicts/{root_spec_id}.json"
        )),
        latest_report_path: Some("millrace-agents/arbiter/reports/run-001.md".to_owned()),
        closure_open: true,
        closure_blocked_by_lineage_work: false,
        blocking_work_ids: Vec::new(),
        opened_at: timestamp(NOW),
        closed_at: None,
        last_arbiter_run_id: Some("run-001".to_owned()),
    }
}

#[test]
fn closure_target_loading_reports_path_aware_failures() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let invalid = load_closure_target_state(&paths, "../bad-root").unwrap_err();
    assert!(matches!(
        invalid,
        LineageRepairError::InvalidRootSpecId { .. }
    ));

    let missing = load_closure_target_state(&paths, "missing-target").unwrap_err();
    assert!(matches!(
        missing,
        LineageRepairError::MissingClosureTarget { .. }
    ));
    assert!(missing.to_string().contains("missing-target.json"));

    fs::write(
        paths.arbiter_targets_dir.join("malformed-target.json"),
        "{\n",
    )
    .unwrap();
    let malformed = load_closure_target_state(&paths, "malformed-target").unwrap_err();
    assert!(matches!(malformed, LineageRepairError::JsonSyntax { .. }));
    assert!(malformed.to_string().contains("malformed-target.json"));

    fs::write(
        paths.arbiter_targets_dir.join("invalid-target.json"),
        serde_json::json!({
            "schema_version": "1.0",
            "kind": "closure_target_state",
            "root_spec_id": "invalid-target",
            "root_idea_id": "idea-invalid",
            "root_spec_path": "millrace-agents/arbiter/contracts/root-specs/invalid-target.md",
            "root_idea_path": "millrace-agents/arbiter/contracts/ideas/idea-invalid.md",
            "rubric_path": "millrace-agents/arbiter/rubrics/invalid-target.md",
            "closure_open": true,
            "closure_blocked_by_lineage_work": false,
            "blocking_work_ids": ["task-blocker"],
            "opened_at": NOW
        })
        .to_string()
            + "\n",
    )
    .unwrap();
    let invalid_target = load_closure_target_state(&paths, "invalid-target").unwrap_err();
    assert!(matches!(
        invalid_target,
        LineageRepairError::ClosureTargetContract { .. }
    ));
    assert!(invalid_target.to_string().contains("invalid-target.json"));
}

#[test]
fn queue_store_lists_deferred_root_specs_without_mutating_invalid_queue_artifacts() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = QueueStore::from_paths(paths.clone());

    let mut same_root = spec_document("spec-root-open", "2026-04-15T00:00:01Z");
    same_root.root_spec_id = Some("spec-root-open".to_owned());
    store.enqueue_spec(&same_root).unwrap();

    let mut later_root = spec_document("spec-root-later", "2026-04-15T00:00:03Z");
    later_root.root_spec_id = Some("spec-root-later".to_owned());
    store.enqueue_spec(&later_root).unwrap();

    let mut earlier_root = spec_document("spec-root-earlier", "2026-04-15T00:00:02Z");
    earlier_root.root_spec_id = Some("spec-root-earlier".to_owned());
    store.enqueue_spec(&earlier_root).unwrap();

    let mut child_spec = spec_document("spec-child", "2026-04-15T00:00:00Z");
    child_spec.root_spec_id = Some("spec-root-earlier".to_owned());
    store.enqueue_spec(&child_spec).unwrap();

    fs::write(paths.specs_queue_dir.join("malformed.md"), "not a spec\n").unwrap();
    let before = fs::read_dir(&paths.specs_queue_dir).unwrap().count();

    let deferred = store.list_deferred_root_spec_ids("spec-root-open").unwrap();

    assert_eq!(
        deferred,
        vec!["spec-root-earlier".to_owned(), "spec-root-later".to_owned()]
    );
    assert_eq!(
        fs::read_dir(&paths.specs_queue_dir).unwrap().count(),
        before
    );
    assert!(paths.specs_queue_dir.join("malformed.md").is_file());
}

#[test]
fn lineage_repair_preview_writes_report_and_does_not_mutate_documents_or_snapshot() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = StateStore::from_paths(paths.clone());
    let target = closure_target_state("idea-idea-root", "idea-root-idea");
    save_closure_target_state(&paths, &target).unwrap();

    let mut queued_task = task_document("task-queued", "2026-04-15T00:00:01Z");
    queued_task.root_idea_id = Some(target.root_idea_id.clone());
    queued_task.root_spec_id = Some("old-root".to_owned());
    queued_task.spec_id = Some("old-root".to_owned());
    let queued_task_raw = render_task_document(&queued_task);
    fs::write(
        paths.tasks_queue_dir.join("task-queued.md"),
        &queued_task_raw,
    )
    .unwrap();

    let mut blocked_task = task_document("task-blocked", "2026-04-15T00:00:02Z");
    blocked_task.root_idea_id = Some("idea-other".to_owned());
    blocked_task.root_spec_id = Some("idea-root".to_owned());
    blocked_task.spec_id = Some("idea-root".to_owned());
    let blocked_task_raw = render_task_document(&blocked_task);
    fs::write(
        paths.tasks_blocked_dir.join("task-blocked.md"),
        &blocked_task_raw,
    )
    .unwrap();

    let mut active_task = task_document("task-active", "2026-04-15T00:00:03Z");
    active_task.root_idea_id = Some(target.root_idea_id.clone());
    active_task.root_spec_id = Some("old-root".to_owned());
    active_task.spec_id = Some("old-root".to_owned());
    let active_task_raw = render_task_document(&active_task);
    fs::write(
        paths.tasks_active_dir.join("task-active.md"),
        &active_task_raw,
    )
    .unwrap();

    let mut queued_incident = incident_document("inc-queued", "2026-04-15T00:00:04Z");
    queued_incident.root_idea_id = Some(target.root_idea_id.clone());
    queued_incident.root_spec_id = Some("old-root".to_owned());
    queued_incident.source_spec_id = Some("old-root".to_owned());
    let queued_incident_raw = render_incident_document(&queued_incident);
    fs::write(
        paths.incidents_incoming_dir.join("inc-queued.md"),
        &queued_incident_raw,
    )
    .unwrap();

    let mut queued_spec = spec_document("spec-queued", "2026-04-15T00:00:05Z");
    queued_spec.root_idea_id = Some(target.root_idea_id.clone());
    queued_spec.root_spec_id = Some("old-root".to_owned());
    let queued_spec_raw = render_spec_document(&queued_spec);
    fs::write(
        paths.specs_queue_dir.join("spec-queued.md"),
        &queued_spec_raw,
    )
    .unwrap();

    let mut snapshot = store.load_snapshot().unwrap();
    snapshot.queue_depth_execution = 77;
    snapshot.queue_depth_planning = 88;
    store.save_snapshot(&snapshot).unwrap();

    let outcome = repair_closure_lineage(&paths, &target.root_spec_id, false).unwrap();

    assert_eq!(outcome.repaired_count, 0);
    assert_eq!(outcome.applied_report_path, None);
    assert!(outcome.preview_report_path.is_file());
    assert_eq!(
        fs::read_to_string(paths.tasks_queue_dir.join("task-queued.md")).unwrap(),
        queued_task_raw
    );
    assert_eq!(
        fs::read_to_string(paths.tasks_blocked_dir.join("task-blocked.md")).unwrap(),
        blocked_task_raw
    );
    assert_eq!(
        fs::read_to_string(paths.tasks_active_dir.join("task-active.md")).unwrap(),
        active_task_raw
    );
    assert_eq!(
        fs::read_to_string(paths.incidents_incoming_dir.join("inc-queued.md")).unwrap(),
        queued_incident_raw
    );
    assert_eq!(
        fs::read_to_string(paths.specs_queue_dir.join("spec-queued.md")).unwrap(),
        queued_spec_raw
    );
    let loaded_snapshot = store.load_snapshot().unwrap();
    assert_eq!(loaded_snapshot.queue_depth_execution, 77);
    assert_eq!(loaded_snapshot.queue_depth_planning, 88);
    assert!(!paths.logs_dir.join("runtime_events.jsonl").exists());

    let report: Value =
        serde_json::from_str(&fs::read_to_string(&outcome.preview_report_path).unwrap()).unwrap();
    assert_eq!(report["kind"], "closure_lineage_repair_plan");
    assert_eq!(report["applied"], false);
    assert_eq!(report["changes"].as_array().unwrap().len(), 5);
    assert!(
        report["skipped_findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["work_item_id"] == "task-active")
    );
    assert!(
        report["skipped_findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["work_item_id"] == "spec-queued")
    );
}

#[test]
fn lineage_repair_apply_mutates_only_safe_documents_refreshes_snapshot_and_emits_event() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = StateStore::from_paths(paths.clone());
    let target = closure_target_state("idea-idea-root", "idea-root-idea");
    save_closure_target_state(&paths, &target).unwrap();

    let mut queued_task = task_document("task-queued", "2026-04-15T00:00:01Z");
    queued_task.root_idea_id = Some(target.root_idea_id.clone());
    queued_task.root_spec_id = Some("old-root".to_owned());
    queued_task.spec_id = Some("old-root".to_owned());
    fs::write(
        paths.tasks_queue_dir.join("task-queued.md"),
        render_task_document(&queued_task),
    )
    .unwrap();

    let mut blocked_task = task_document("task-blocked", "2026-04-15T00:00:02Z");
    blocked_task.root_idea_id = Some("idea-other".to_owned());
    blocked_task.root_spec_id = Some("idea-root".to_owned());
    blocked_task.spec_id = Some("idea-root".to_owned());
    fs::write(
        paths.tasks_blocked_dir.join("task-blocked.md"),
        render_task_document(&blocked_task),
    )
    .unwrap();

    let mut active_task = task_document("task-active", "2026-04-15T00:00:03Z");
    active_task.root_idea_id = Some(target.root_idea_id.clone());
    active_task.root_spec_id = Some("old-root".to_owned());
    active_task.spec_id = Some("old-root".to_owned());
    let active_task_raw = render_task_document(&active_task);
    fs::write(
        paths.tasks_active_dir.join("task-active.md"),
        &active_task_raw,
    )
    .unwrap();

    let mut queued_incident = incident_document("inc-queued", "2026-04-15T00:00:04Z");
    queued_incident.root_idea_id = Some(target.root_idea_id.clone());
    queued_incident.root_spec_id = Some("old-root".to_owned());
    queued_incident.source_spec_id = Some("old-root".to_owned());
    fs::write(
        paths.incidents_incoming_dir.join("inc-queued.md"),
        render_incident_document(&queued_incident),
    )
    .unwrap();

    let mut blocked_incident = incident_document("inc-blocked", "2026-04-15T00:00:05Z");
    blocked_incident.root_idea_id = Some(target.root_idea_id.clone());
    blocked_incident.root_spec_id = Some("old-root".to_owned());
    blocked_incident.source_spec_id = Some("old-root".to_owned());
    fs::write(
        paths.incidents_blocked_dir.join("inc-blocked.md"),
        render_incident_document(&blocked_incident),
    )
    .unwrap();

    let mut queued_spec = spec_document("spec-queued", "2026-04-15T00:00:06Z");
    queued_spec.root_idea_id = Some(target.root_idea_id.clone());
    queued_spec.root_spec_id = Some("old-root".to_owned());
    let queued_spec_raw = render_spec_document(&queued_spec);
    fs::write(
        paths.specs_queue_dir.join("spec-queued.md"),
        &queued_spec_raw,
    )
    .unwrap();

    let mut snapshot = store.load_snapshot().unwrap();
    snapshot.queue_depth_execution = 77;
    snapshot.queue_depth_planning = 88;
    store.save_snapshot(&snapshot).unwrap();

    let outcome = repair_closure_lineage(&paths, &target.root_spec_id, true).unwrap();

    assert_eq!(outcome.repaired_count, 4);
    assert!(outcome.preview_report_path.is_file());
    assert!(outcome.applied_report_path.as_ref().unwrap().is_file());

    let repaired_task = parse_task_document(
        &fs::read_to_string(paths.tasks_queue_dir.join("task-queued.md")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        repaired_task.root_spec_id.as_deref(),
        Some("idea-idea-root")
    );
    assert_eq!(repaired_task.spec_id.as_deref(), Some("idea-idea-root"));
    let repaired_blocked = parse_task_document(
        &fs::read_to_string(paths.tasks_blocked_dir.join("task-blocked.md")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        repaired_blocked.root_spec_id.as_deref(),
        Some("idea-idea-root")
    );
    assert_eq!(repaired_blocked.spec_id.as_deref(), Some("idea-idea-root"));
    let repaired_incident = parse_incident_document(
        &fs::read_to_string(paths.incidents_incoming_dir.join("inc-queued.md")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        repaired_incident.root_spec_id.as_deref(),
        Some("idea-idea-root")
    );
    assert_eq!(
        fs::read_to_string(paths.tasks_active_dir.join("task-active.md")).unwrap(),
        active_task_raw
    );
    assert_eq!(
        fs::read_to_string(paths.specs_queue_dir.join("spec-queued.md")).unwrap(),
        queued_spec_raw
    );

    let loaded_snapshot = store.load_snapshot().unwrap();
    assert_eq!(loaded_snapshot.queue_depth_execution, 1);
    assert_eq!(loaded_snapshot.queue_depth_planning, 2);
    assert_eq!(
        loaded_snapshot
            .queue_depths_by_plane
            .get(&Plane::Execution)
            .copied(),
        Some(1)
    );
    assert_eq!(
        loaded_snapshot
            .queue_depths_by_plane
            .get(&Plane::Planning)
            .copied(),
        Some(2)
    );

    let applied_report: Value = serde_json::from_str(
        &fs::read_to_string(outcome.applied_report_path.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(applied_report["applied"], true);

    let events = read_json_lines(outcome.event_log_path.as_ref().unwrap());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event_type"], "closure_lineage_repaired");
    assert_eq!(events[0]["data"]["root_spec_id"], "idea-idea-root");
    assert_eq!(events[0]["data"]["repair_count"], 4);
    assert_eq!(
        events[0]["data"]["repair_report_path"],
        "millrace-agents/arbiter/diagnostics/lineage-repairs/".to_owned()
            + outcome
                .applied_report_path
                .as_ref()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
    );
}

#[test]
fn lineage_repair_apply_refuses_active_runtime_ownership_before_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let target = closure_target_state("idea-root", "idea-root-idea");
    save_closure_target_state(&paths, &target).unwrap();

    let mut queued_task = task_document("task-queued", "2026-04-15T00:00:01Z");
    queued_task.root_idea_id = Some(target.root_idea_id.clone());
    queued_task.root_spec_id = Some("old-root".to_owned());
    queued_task.spec_id = Some("old-root".to_owned());
    let queued_task_raw = render_task_document(&queued_task);
    fs::write(
        paths.tasks_queue_dir.join("task-queued.md"),
        &queued_task_raw,
    )
    .unwrap();

    acquire_runtime_ownership_lock_with_options(
        &paths,
        RuntimeOwnershipLockOptions::new(process::id(), "host", "session-active", NOW).unwrap(),
    )
    .unwrap();

    let error = repair_closure_lineage(&paths, &target.root_spec_id, true).unwrap_err();
    assert!(matches!(
        error,
        LineageRepairError::ActiveRuntimeOwnershipLock { .. }
    ));
    assert_eq!(
        fs::read_to_string(paths.tasks_queue_dir.join("task-queued.md")).unwrap(),
        queued_task_raw
    );
    assert!(!paths.logs_dir.join("runtime_events.jsonl").exists());
}

#[test]
fn lineage_repair_apply_refuses_active_snapshot_stage_before_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = StateStore::from_paths(paths.clone());
    let target = closure_target_state("idea-root", "idea-root-idea");
    save_closure_target_state(&paths, &target).unwrap();

    let mut queued_task = task_document("task-queued", "2026-04-15T00:00:01Z");
    queued_task.root_idea_id = Some(target.root_idea_id.clone());
    queued_task.root_spec_id = Some("old-root".to_owned());
    queued_task.spec_id = Some("old-root".to_owned());
    let queued_task_raw = render_task_document(&queued_task);
    fs::write(
        paths.tasks_queue_dir.join("task-queued.md"),
        &queued_task_raw,
    )
    .unwrap();

    let mut snapshot = store.load_snapshot().unwrap();
    snapshot.active_plane = Some(Plane::Execution);
    snapshot.active_stage = Some(StageName::Builder);
    snapshot.active_run_id = Some("run-001".to_owned());
    snapshot.active_work_item_kind = Some(WorkItemKind::Task);
    snapshot.active_work_item_id = Some("task-queued".to_owned());
    snapshot.active_since = Some(timestamp(NOW));
    store.save_snapshot(&snapshot).unwrap();

    let error = repair_closure_lineage(&paths, &target.root_spec_id, true).unwrap_err();
    assert!(matches!(
        error,
        LineageRepairError::ActiveRuntimeStage { .. }
    ));
    assert_eq!(
        fs::read_to_string(paths.tasks_queue_dir.join("task-queued.md")).unwrap(),
        queued_task_raw
    );
    assert!(!paths.logs_dir.join("runtime_events.jsonl").exists());
}

#[test]
fn task_queue_claiming_is_deterministic_dependency_aware_and_canonical() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = QueueStore::from_paths(paths.clone());

    let mut dependent = task_document("task-dependent", NOW);
    dependent.depends_on = vec!["task-prereq".to_owned()];
    store.enqueue_task(&dependent).unwrap();
    store
        .enqueue_task(&task_document("task-prereq", "2026-04-15T00:00:01Z"))
        .unwrap();

    let raw = fs::read_to_string(paths.tasks_queue_dir.join("task-dependent.md")).unwrap();
    let parsed = parse_task_document(&raw).unwrap();
    assert_eq!(render_task_document(&parsed), raw);

    let first = store.claim_next_execution_task(None).unwrap().unwrap();
    assert_eq!(first.work_item_kind, WorkItemKind::Task);
    assert_eq!(first.work_item_id, "task-prereq");
    assert_eq!(first.path, paths.tasks_active_dir.join("task-prereq.md"));

    store.mark_task_done("task-prereq").unwrap();
    assert!(paths.tasks_done_dir.join("task-prereq.md").is_file());
    assert!(!paths.tasks_active_dir.join("task-prereq.md").exists());

    let second = store.claim_next_execution_task(None).unwrap().unwrap();
    assert_eq!(second.work_item_id, "task-dependent");
    store.mark_task_blocked("task-dependent").unwrap();
    assert!(paths.tasks_blocked_dir.join("task-dependent.md").is_file());

    let duplicate = store
        .enqueue_task(&task_document("task-prereq", NOW))
        .unwrap_err();
    assert!(duplicate.to_string().contains("already exists"));
}

#[test]
fn task_lifecycle_duplicate_scan_uses_parsed_ids_and_parse_error_filename_fallback() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    fs::write(
        paths.tasks_active_dir.join("task-duplicate.md"),
        "# Broken continuation\n\nTask-ID: task-duplicate\nTitle: Broken continuation\n",
    )
    .unwrap();
    fs::write(
        paths.tasks_done_dir.join("done-alias.md"),
        render_task_document(&task_document("task-duplicate", NOW)),
    )
    .unwrap();

    let duplicates = find_duplicate_task_lifecycle_ids(&paths).unwrap();

    assert_eq!(duplicates.len(), 1);
    assert_eq!(duplicates[0].task_id, "task-duplicate");
    assert_eq!(duplicates[0].states(), vec!["active", "done"]);
    assert_eq!(
        duplicates[0].state_paths[0].1,
        paths.tasks_active_dir.join("task-duplicate.md")
    );
    assert_eq!(
        duplicates[0].state_paths[1].1,
        paths.tasks_done_dir.join("done-alias.md")
    );
}

#[test]
fn mark_task_done_retires_same_root_blocked_duplicate_and_records_audit_evidence() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = QueueStore::from_paths(paths.clone());
    let source = task_document("task-duplicate", NOW);
    let mut continuation = source.clone();
    continuation.summary = "same-id continuation".to_owned();
    let blocked_path = paths.tasks_blocked_dir.join("task-duplicate.md");
    let active_path = paths.tasks_active_dir.join("task-duplicate.md");
    fs::write(&blocked_path, render_task_document(&source)).unwrap();
    fs::write(&active_path, render_task_document(&continuation)).unwrap();
    fs::write(
        paths.tasks_blocked_dir.join("task-unrelated.md"),
        render_task_document(&task_document("task-unrelated", NOW)),
    )
    .unwrap();

    let destination = store.mark_task_done("task-duplicate").unwrap();

    assert_eq!(destination, paths.tasks_done_dir.join("task-duplicate.md"));
    assert!(destination.is_file());
    assert!(!blocked_path.exists());
    assert!(paths.tasks_blocked_dir.join("task-unrelated.md").is_file());
    let superseded_dir = paths.tasks_blocked_dir.join("superseded");
    let superseded: Vec<_> = fs::read_dir(&superseded_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("md"))
        .collect();
    assert_eq!(superseded.len(), 1);
    let archived = fs::read_to_string(&superseded[0]).unwrap();
    assert!(archived.contains("Task-ID: task-duplicate"));
    assert!(!archived.contains("same-id continuation"));

    let retirements = read_json_lines(&superseded_dir.join("retirements.jsonl"));
    assert_eq!(retirements.len(), 1);
    assert_eq!(retirements[0]["task_id"], "task-duplicate");
    assert_eq!(retirements[0]["root_spec_id"], "spec-root-001");
    assert_eq!(retirements[0]["reason"], "same_id_done_continuation");
    assert_eq!(
        retirements[0]["source_path"],
        "millrace-agents/tasks/blocked/task-duplicate.md"
    );
    assert!(
        retirements[0]["archive_path"]
            .as_str()
            .unwrap()
            .starts_with("millrace-agents/tasks/blocked/superseded/task-duplicate.")
    );
}

#[test]
fn mark_task_done_keeps_different_root_or_unparseable_blocked_duplicate_in_place() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = QueueStore::from_paths(paths.clone());
    let source = task_document("task-different-root", NOW);
    let mut continuation = source.clone();
    continuation.root_spec_id = Some("spec-root-002".to_owned());
    continuation.spec_id = Some("spec-root-002".to_owned());
    fs::write(
        paths.tasks_blocked_dir.join("task-different-root.md"),
        render_task_document(&source),
    )
    .unwrap();
    fs::write(
        paths.tasks_active_dir.join("task-different-root.md"),
        render_task_document(&continuation),
    )
    .unwrap();

    store.mark_task_done("task-different-root").unwrap();

    assert!(
        paths
            .tasks_blocked_dir
            .join("task-different-root.md")
            .is_file()
    );
    assert!(!paths.tasks_blocked_dir.join("superseded").exists());

    let valid = task_document("task-unparseable-blocked", NOW);
    fs::write(
        paths.tasks_blocked_dir.join("task-unparseable-blocked.md"),
        "# Broken blocked predecessor\n\nTask-ID: task-unparseable-blocked\n",
    )
    .unwrap();
    fs::write(
        paths.tasks_active_dir.join("task-unparseable-blocked.md"),
        render_task_document(&valid),
    )
    .unwrap();

    store.mark_task_done("task-unparseable-blocked").unwrap();

    assert!(
        paths
            .tasks_blocked_dir
            .join("task-unparseable-blocked.md")
            .is_file()
    );
    assert!(!paths.tasks_blocked_dir.join("superseded").exists());
}

#[test]
fn planning_and_learning_lifecycles_move_across_the_expected_surfaces() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = QueueStore::from_paths(paths.clone());

    store
        .enqueue_spec(&spec_document("spec-001", "2026-04-15T00:05:00Z"))
        .unwrap();
    store
        .enqueue_spec(&spec_document("spec-002", "2026-04-15T00:08:00Z"))
        .unwrap();
    store
        .enqueue_incident(&incident_document("inc-001", "2026-04-15T00:06:00Z"))
        .unwrap();
    store
        .enqueue_incident(&incident_document("inc-002", "2026-04-15T00:07:00Z"))
        .unwrap();

    let first = store.claim_next_planning_item(None).unwrap().unwrap();
    assert_eq!(first.work_item_kind, WorkItemKind::Incident);
    assert_eq!(first.work_item_id, "inc-001");
    store.mark_incident_resolved("inc-001").unwrap();
    assert!(paths.incidents_resolved_dir.join("inc-001.md").is_file());

    let second = store.claim_next_planning_item(None).unwrap().unwrap();
    assert_eq!(second.work_item_id, "inc-002");
    store.mark_incident_blocked("inc-002").unwrap();
    assert!(paths.incidents_blocked_dir.join("inc-002.md").is_file());

    let third = store.claim_next_planning_item(None).unwrap().unwrap();
    assert_eq!(third.work_item_kind, WorkItemKind::Spec);
    assert_eq!(third.work_item_id, "spec-001");
    store.mark_spec_done("spec-001").unwrap();
    assert!(paths.specs_done_dir.join("spec-001.md").is_file());

    let fourth = store.claim_next_planning_item(None).unwrap().unwrap();
    assert_eq!(fourth.work_item_id, "spec-002");
    store.mark_spec_blocked("spec-002").unwrap();
    assert!(paths.specs_blocked_dir.join("spec-002.md").is_file());

    store
        .enqueue_learning_request(&learning_request_document("learn-001", NOW))
        .unwrap();
    store
        .enqueue_learning_request(&learning_request_document(
            "learn-002",
            "2026-04-15T00:00:01Z",
        ))
        .unwrap();
    let raw = fs::read_to_string(paths.learning_requests_queue_dir.join("learn-001.md")).unwrap();
    let parsed = parse_learning_request_document(&raw).unwrap();
    assert_eq!(render_learning_request_document(&parsed), raw);

    let learning_first = store.claim_next_learning_request().unwrap().unwrap();
    assert_eq!(learning_first.work_item_kind, WorkItemKind::LearningRequest);
    assert_eq!(learning_first.work_item_id, "learn-001");
    store.mark_learning_request_done("learn-001").unwrap();
    assert!(
        paths
            .learning_requests_done_dir
            .join("learn-001.md")
            .is_file()
    );

    let learning_second = store.claim_next_learning_request().unwrap().unwrap();
    assert_eq!(learning_second.work_item_id, "learn-002");
    store.mark_learning_request_blocked("learn-002").unwrap();
    assert!(
        paths
            .learning_requests_blocked_dir
            .join("learn-002.md")
            .is_file()
    );
}

#[test]
fn probe_queue_lifecycle_claims_oldest_probe_or_spec_after_incidents() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = QueueStore::from_paths(paths.clone());

    store
        .enqueue_spec(&spec_document("spec-001", "2026-04-15T00:05:00Z"))
        .unwrap();
    store
        .enqueue_probe(&probe_document("probe-001", "2026-04-15T00:01:00Z"))
        .unwrap();
    store
        .enqueue_incident(&incident_document("inc-001", "2026-04-15T00:02:00Z"))
        .unwrap();

    let incident = store.claim_next_planning_item(None).unwrap().unwrap();
    assert_eq!(incident.work_item_kind, WorkItemKind::Incident);
    assert_eq!(incident.work_item_id, "inc-001");
    store.mark_incident_resolved("inc-001").unwrap();

    let probe = store.claim_next_planning_item(None).unwrap().unwrap();
    assert_eq!(probe.work_item_kind, WorkItemKind::Probe);
    assert_eq!(probe.work_item_id, "probe-001");
    assert_eq!(probe.path, paths.probes_active_dir.join("probe-001.md"));

    let raw = fs::read_to_string(&probe.path).unwrap();
    assert_eq!(
        render_probe_document(&probe_document("probe-001", "2026-04-15T00:01:00Z")),
        raw
    );

    store
        .requeue_probe("probe-001", "operator requested another recon pass")
        .unwrap();
    assert!(paths.probes_queue_dir.join("probe-001.md").is_file());
    let requeue_log = read_json_lines(&paths.probes_queue_dir.join("probe-001.requeue.jsonl"));
    assert_eq!(requeue_log[0]["kind"], "probe");
    assert_eq!(
        requeue_log[0]["reason"],
        "operator requested another recon pass"
    );

    let probe = store.claim_next_planning_item(None).unwrap().unwrap();
    assert_eq!(probe.work_item_kind, WorkItemKind::Probe);
    store.mark_probe_done("probe-001").unwrap();
    assert!(paths.probes_done_dir.join("probe-001.md").is_file());

    let duplicate = store
        .enqueue_probe(&probe_document("probe-001", "2026-04-15T00:03:00Z"))
        .unwrap_err();
    assert!(duplicate.to_string().contains("already exists"));

    let spec = store.claim_next_planning_item(None).unwrap().unwrap();
    assert_eq!(spec.work_item_kind, WorkItemKind::Spec);
    assert_eq!(spec.work_item_id, "spec-001");
}

#[test]
fn closure_scoped_planning_claims_skip_root_intake_probes() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = QueueStore::from_paths(paths.clone());

    store
        .enqueue_probe(&probe_document("probe-root-intake", "2026-04-15T00:00:00Z"))
        .unwrap();
    let mut same_root_spec = spec_document("spec-same-root", "2026-04-15T00:01:00Z");
    same_root_spec.root_spec_id = Some("spec-root-001".to_owned());
    store.enqueue_spec(&same_root_spec).unwrap();

    let claim = store
        .claim_next_planning_item(Some("spec-root-001"))
        .unwrap()
        .unwrap();

    assert_eq!(claim.work_item_kind, WorkItemKind::Spec);
    assert_eq!(claim.work_item_id, "spec-same-root");
    assert!(
        paths
            .probes_queue_dir
            .join("probe-root-intake.md")
            .is_file()
    );
}

#[test]
fn requeue_records_reasons_and_invalid_queue_artifacts_are_quarantined() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = QueueStore::from_paths(paths.clone());

    store.enqueue_task(&task_document("task-001", NOW)).unwrap();
    let claim = store.claim_next_execution_task(None).unwrap().unwrap();
    assert_eq!(claim.work_item_id, "task-001");
    store
        .requeue_task("task-001", "retry after consultant guidance")
        .unwrap();
    assert!(paths.tasks_queue_dir.join("task-001.md").is_file());
    let requeue_log = read_json_lines(&paths.tasks_queue_dir.join("task-001.requeue.jsonl"));
    assert_eq!(requeue_log[0]["kind"], "task");
    assert_eq!(requeue_log[0]["reason"], "retry after consultant guidance");

    let mismatched = task_document("task-mismatch", NOW);
    fs::write(
        paths.tasks_queue_dir.join("task-alias.md"),
        render_task_document(&mismatched),
    )
    .unwrap();
    store
        .enqueue_task(&task_document("task-002", "2026-04-15T00:00:01Z"))
        .unwrap();

    let next = store.claim_next_execution_task(None).unwrap().unwrap();
    assert_eq!(next.work_item_id, "task-001");
    store.mark_task_done("task-001").unwrap();
    let next = store.claim_next_execution_task(None).unwrap().unwrap();
    assert_eq!(next.work_item_id, "task-002");
    assert!(!paths.tasks_queue_dir.join("task-alias.md").exists());
    assert!(
        paths
            .tasks_queue_dir
            .join("task-alias.md.invalid")
            .is_file()
    );
    let invalid_log = read_json_lines(&paths.tasks_queue_dir.join("invalid-artifacts.jsonl"));
    assert!(
        invalid_log
            .iter()
            .any(|entry| entry["source_name"] == "task-alias.md")
    );
}

#[test]
fn stale_active_detection_reports_execution_and_planning_contradictions() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = QueueStore::from_paths(paths.clone());

    store.enqueue_task(&task_document("task-001", NOW)).unwrap();
    store
        .enqueue_task(&task_document("task-002", "2026-04-15T00:00:01Z"))
        .unwrap();
    store.claim_next_execution_task(None).unwrap().unwrap();

    let stale_no_snapshot = store.detect_execution_stale_state(None).unwrap();
    assert!(stale_no_snapshot.is_stale);
    assert!(
        stale_no_snapshot
            .reasons
            .contains(&"active_without_snapshot".to_owned())
    );

    let stale_snapshot_queue = store
        .detect_execution_stale_state(Some("task-002"))
        .unwrap();
    assert!(
        stale_snapshot_queue
            .reasons
            .contains(&"snapshot_points_to_queued_item".to_owned())
    );
    assert!(
        stale_snapshot_queue
            .reasons
            .contains(&"snapshot_active_id_mismatch".to_owned())
    );

    fs::write(
        paths.tasks_active_dir.join("task-777.md"),
        render_task_document(&task_document("task-777", "2026-04-15T00:00:02Z")),
    )
    .unwrap();
    let stale_multiple = store
        .detect_execution_stale_state(Some("task-001"))
        .unwrap();
    assert!(
        stale_multiple
            .reasons
            .contains(&"multiple_active_items".to_owned())
    );

    let partial = store
        .detect_planning_stale_state(Some(WorkItemKind::Spec), None)
        .unwrap_err();
    assert!(partial.to_string().contains("must be set together"));

    store
        .enqueue_spec(&spec_document("spec-001", "2026-04-15T00:00:03Z"))
        .unwrap();
    let planning_stale = store
        .detect_planning_stale_state(Some(WorkItemKind::Spec), Some("spec-001"))
        .unwrap();
    assert!(
        planning_stale
            .reasons
            .contains(&"snapshot_points_to_queued_item".to_owned())
    );
    assert!(
        planning_stale
            .reasons
            .contains(&"snapshot_without_active_artifact".to_owned())
    );
}

#[test]
fn state_store_round_trips_runtime_json_status_markers_and_recovery_counters() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = StateStore::from_paths(paths.clone());

    let mut snapshot = store.load_snapshot().unwrap();
    snapshot.paused = true;
    snapshot.active_plane = Some(millrace_ai::contracts::Plane::Execution);
    snapshot.active_stage = Some(StageName::Checker);
    snapshot.active_run_id = Some("run-001".to_owned());
    snapshot.active_work_item_kind = Some(WorkItemKind::Task);
    snapshot.active_work_item_id = Some("task-001".to_owned());
    snapshot.active_since = Some(timestamp(NOW));
    snapshot.updated_at = timestamp(NOW);
    store.save_snapshot(&snapshot).unwrap();

    let loaded = store.load_snapshot().unwrap();
    assert!(loaded.paused);
    assert_eq!(loaded.active_stage, Some(StageName::Checker));
    assert_eq!(loaded.active_work_item_id.as_deref(), Some("task-001"));
    assert!(
        loaded
            .active_runs_by_plane
            .contains_key(&millrace_ai::contracts::Plane::Execution)
    );

    let counters = RecoveryCounters {
        schema_version: "1.0".to_owned(),
        kind: "recovery_counters".to_owned(),
        entries: vec![RecoveryCounterEntry {
            failure_class: "missing_terminal_result".to_owned(),
            work_item_kind: WorkItemKind::Task,
            work_item_id: "task-001".to_owned(),
            troubleshoot_attempt_count: 1,
            mechanic_attempt_count: 0,
            fix_cycle_count: 0,
            consultant_invocations: 0,
            last_updated_at: timestamp(NOW),
        }],
    };
    store.save_recovery_counters(&counters).unwrap();
    assert_eq!(
        store.load_recovery_counters().unwrap().entries[0].failure_class,
        "missing_terminal_result"
    );

    let first = store
        .increment_troubleshoot_attempt(
            "stale_active_ownership",
            WorkItemKind::Task,
            "task-002",
            timestamp(NOW),
        )
        .unwrap();
    let second = store
        .increment_troubleshoot_attempt(
            "stale_active_ownership",
            WorkItemKind::Task,
            "task-002",
            timestamp(NOW),
        )
        .unwrap();
    assert_eq!(first.troubleshoot_attempt_count, 1);
    assert_eq!(second.troubleshoot_attempt_count, 2);
    store
        .reset_forward_progress_counters(WorkItemKind::Task, "task-002")
        .unwrap();
    assert!(
        !store
            .load_recovery_counters()
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.work_item_id == "task-002")
    );

    store.set_execution_status("### CHECKER_PASS").unwrap();
    store
        .set_execution_status("### CUSTOM_EXECUTION_RUNNING")
        .unwrap();
    assert_eq!(
        store.load_execution_status().unwrap(),
        "### CUSTOM_EXECUTION_RUNNING"
    );
    assert_eq!(
        fs::read_to_string(&paths.execution_status_file).unwrap(),
        "### CUSTOM_EXECUTION_RUNNING\n"
    );
    assert!(
        store
            .set_planning_status("PLANNER_RUNNING")
            .unwrap_err()
            .to_string()
            .contains("must start with '### '")
    );
    assert!(
        store
            .set_learning_status("### ANALYST_COMPLETE\n### EXTRA")
            .unwrap_err()
            .to_string()
            .contains("single line")
    );
}

#[test]
fn malformed_runtime_artifacts_preserve_typed_state_errors() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let store = StateStore::from_paths(paths.clone());

    fs::write(&paths.runtime_snapshot_file, "[\"not-an-object\"]\n").unwrap();
    let error = store.load_snapshot().unwrap_err();
    assert!(matches!(error, StateStoreError::NonObjectPayload { .. }));

    fs::write(
        &paths.runtime_snapshot_file,
        "{\"kind\":\"runtime_snapshot\"}\n",
    )
    .unwrap();
    let error = store.load_snapshot().unwrap_err();
    assert!(matches!(error, StateStoreError::RuntimeJson { .. }));

    fs::write(&paths.recovery_counters_file, "{\"kind\":\"wrong\"}\n").unwrap();
    let error = store.load_recovery_counters().unwrap_err();
    assert!(matches!(error, StateStoreError::RuntimeJson { .. }));

    let bad_task = TaskDocument {
        target_paths: Vec::new(),
        ..task_document("task-bad", NOW)
    };
    let queue_error = QueueStore::from_paths(paths)
        .enqueue_task(&bad_task)
        .unwrap_err();
    assert!(matches!(queue_error, QueueStoreError::WorkDocument { .. }));
}
