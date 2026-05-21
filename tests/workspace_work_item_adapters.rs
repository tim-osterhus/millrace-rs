use std::{fs, path::Path};

use millrace_ai::{
    compiler::compile_compiled_run_plan,
    contracts::{
        IncidentDecision, IncidentDocument, IncidentSeverity, LearningRequestAction,
        LearningRequestDocument, Plane, ProbeDocument, SpecDocument, SpecSourceType, StageName,
        TaskDocument, Timestamp, WorkItemKind,
    },
    work_documents::{
        render_incident_document, render_learning_request_document, render_probe_document,
        render_spec_document, render_task_document,
    },
    workspace::{
        QueueLifecycleInterpreter, QueueStore, SourceLifecycleAction, SourceLifecycleIntent,
        adapter_for_family_id, adapter_for_kind, builtin_work_item_adapters,
        enqueue_rendered_with_adapter, initialize_workspace, parse_with_adapter,
        validate_filename_with_adapter,
    },
};
use tempfile::TempDir;

const NOW: &str = "2026-05-19T00:00:00Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("created_at", value).unwrap()
}

fn task_document(task_id: &str) -> TaskDocument {
    TaskDocument {
        task_id: task_id.to_owned(),
        title: format!("Task {task_id}"),
        summary: "adapter test".to_owned(),
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        spec_id: Some("spec-root-001".to_owned()),
        parent_task_id: None,
        incident_id: None,
        target_paths: vec!["src/workspace/work_item_adapters.rs".to_owned()],
        acceptance: vec!["adapter behavior is deterministic".to_owned()],
        required_checks: vec!["cargo test --test workspace_work_item_adapters".to_owned()],
        references: vec![
            "../millrace-py/src/millrace_ai/workspace/work_item_adapters.py".to_owned(),
        ],
        risk: vec!["queue adapter drift".to_owned()],
        depends_on: Vec::new(),
        blocks: Vec::new(),
        tags: vec!["adapter".to_owned()],
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
        summary: "adapter spec".to_owned(),
        source_type: SpecSourceType::Manual,
        source_id: None,
        parent_spec_id: None,
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        goals: vec!["test adapters".to_owned()],
        non_goals: Vec::new(),
        scope: Vec::new(),
        constraints: vec!["deterministic".to_owned()],
        assumptions: Vec::new(),
        risks: Vec::new(),
        target_paths: Vec::new(),
        entrypoints: Vec::new(),
        required_skills: Vec::new(),
        decomposition_hints: Vec::new(),
        acceptance: vec!["adapter parses spec".to_owned()],
        references: vec!["tests/workspace_work_item_adapters.rs".to_owned()],
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn probe_document(probe_id: &str) -> ProbeDocument {
    ProbeDocument {
        probe_id: probe_id.to_owned(),
        title: format!("Probe {probe_id}"),
        summary: "adapter probe".to_owned(),
        request: "Research adapter behavior.".to_owned(),
        target_paths: Vec::new(),
        constraints: Vec::new(),
        acceptance: Vec::new(),
        risk_notes: Vec::new(),
        references: Vec::new(),
        tags: Vec::new(),
        status_hint: None,
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn incident_document(incident_id: &str) -> IncidentDocument {
    IncidentDocument {
        incident_id: incident_id.to_owned(),
        title: format!("Incident {incident_id}"),
        summary: "adapter incident".to_owned(),
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        source_task_id: None,
        source_spec_id: Some("spec-root-001".to_owned()),
        source_stage: StageName::Consultant,
        source_plane: Plane::Execution,
        failure_class: "adapter_failure".to_owned(),
        severity: IncidentSeverity::Medium,
        needs_planning: true,
        trigger_reason: "adapter test".to_owned(),
        observed_symptoms: Vec::new(),
        failed_attempts: Vec::new(),
        consultant_decision: IncidentDecision::NeedsPlanning,
        evidence_paths: Vec::new(),
        related_run_ids: Vec::new(),
        related_stage_results: Vec::new(),
        references: Vec::new(),
        opened_at: timestamp(NOW),
        opened_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn learning_request_document(learning_request_id: &str) -> LearningRequestDocument {
    LearningRequestDocument {
        learning_request_id: learning_request_id.to_owned(),
        title: format!("Learning {learning_request_id}"),
        summary: "adapter learning".to_owned(),
        requested_action: LearningRequestAction::Improve,
        target_skill_id: Some("checker-core".to_owned()),
        target_stage: None,
        source_refs: Vec::new(),
        preferred_output_paths: Vec::new(),
        trigger_metadata: serde_json::json!({}),
        originating_run_ids: Vec::new(),
        artifact_paths: Vec::new(),
        references: Vec::new(),
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

#[test]
fn builtin_adapter_registry_covers_builtin_markdown_families() {
    let families: Vec<_> = builtin_work_item_adapters()
        .iter()
        .map(|adapter| adapter.family_id)
        .collect();

    assert_eq!(
        families,
        ["task", "probe", "spec", "incident", "learning_request"]
    );
    assert_eq!(
        adapter_for_kind(WorkItemKind::Task).unwrap().id_field,
        "task_id"
    );
    assert_eq!(
        adapter_for_family_id("incident").unwrap().timestamp_field,
        "opened_at"
    );
}

#[test]
fn adapter_parse_matches_typed_work_document_parsing() {
    let cases = [
        (
            WorkItemKind::Task,
            render_task_document(&task_document("task-001")),
            "task-001",
        ),
        (
            WorkItemKind::Spec,
            render_spec_document(&spec_document("spec-001")),
            "spec-001",
        ),
        (
            WorkItemKind::Probe,
            render_probe_document(&probe_document("probe-001")),
            "probe-001",
        ),
        (
            WorkItemKind::Incident,
            render_incident_document(&incident_document("incident-001")),
            "incident-001",
        ),
        (
            WorkItemKind::LearningRequest,
            render_learning_request_document(&learning_request_document("learn-001")),
            "learn-001",
        ),
    ];

    for (kind, raw, expected_id) in cases {
        let parsed = parse_with_adapter(
            adapter_for_kind(kind).unwrap(),
            &raw,
            Path::new("work-item.md"),
        )
        .unwrap();
        assert_eq!(parsed.work_item_id, expected_id);
        assert_eq!(parsed.created_at, timestamp(NOW));
    }
}

#[test]
fn generic_enqueue_adapter_and_claim_policy_metadata_match_queue_store() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let adapter = adapter_for_kind(WorkItemKind::Task).unwrap();

    let path = enqueue_rendered_with_adapter(
        &paths,
        adapter,
        "task-generic",
        &render_task_document(&task_document("task-generic")),
    )
    .unwrap();
    assert_eq!(path, paths.tasks_queue_dir.join("task-generic.md"));

    let claim = QueueStore::from_paths(paths.clone())
        .claim_next_execution_task(None)
        .unwrap()
        .unwrap();
    assert_eq!(claim.work_item_kind, WorkItemKind::Task);
    assert_eq!(claim.family_id, "task");
    assert_eq!(claim.plane, Plane::Execution);
    assert_eq!(claim.source_state.as_deref(), Some("queue"));
    assert_eq!(claim.claim_policy_id.as_deref(), Some("execution.default"));
}

#[test]
fn adapter_rejects_filename_id_mismatch() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let path = paths.tasks_queue_dir.join("task-alias.md");
    fs::write(&path, render_task_document(&task_document("task-real"))).unwrap();

    let error =
        validate_filename_with_adapter(adapter_for_kind(WorkItemKind::Task).unwrap(), &path)
            .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("filename stem does not match task_id")
    );
}

#[test]
fn lifecycle_interpreter_moves_builtin_active_items_from_runtime_intents() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue.enqueue_task(&task_document("task-001")).unwrap();
    queue.claim_next_execution_task(None).unwrap().unwrap();

    let done = QueueLifecycleInterpreter::new(paths.clone(), Vec::new())
        .apply(&SourceLifecycleIntent::for_builtin(
            "complete_work_item",
            SourceLifecycleAction::Complete,
            WorkItemKind::Task,
            "task-001",
        ))
        .unwrap();

    assert_eq!(done, paths.tasks_done_dir.join("task-001.md"));
    assert!(done.is_file());

    queue.enqueue_spec(&spec_document("spec-001")).unwrap();
    queue.claim_next_planning_item(None).unwrap().unwrap();
    let blocked = QueueLifecycleInterpreter::new(paths.clone(), Vec::new())
        .apply(&SourceLifecycleIntent::for_builtin(
            "block_work_item",
            SourceLifecycleAction::Block,
            WorkItemKind::Spec,
            "spec-001",
        ))
        .unwrap();

    assert_eq!(blocked, paths.specs_blocked_dir.join("spec-001.md"));
    assert!(blocked.is_file());

    queue.enqueue_probe(&probe_document("probe-001")).unwrap();
    queue.claim_next_planning_item(None).unwrap().unwrap();
    let probe_done = QueueLifecycleInterpreter::new(paths.clone(), Vec::new())
        .apply(&SourceLifecycleIntent::for_builtin(
            "complete_work_item",
            SourceLifecycleAction::Complete,
            WorkItemKind::Probe,
            "probe-001",
        ))
        .unwrap();
    assert_eq!(probe_done, paths.probes_done_dir.join("probe-001.md"));

    queue
        .enqueue_incident(&incident_document("incident-001"))
        .unwrap();
    queue.claim_next_planning_item(None).unwrap().unwrap();
    let incident_done = QueueLifecycleInterpreter::new(paths.clone(), Vec::new())
        .apply(&SourceLifecycleIntent::for_builtin(
            "complete_work_item",
            SourceLifecycleAction::Complete,
            WorkItemKind::Incident,
            "incident-001",
        ))
        .unwrap();
    assert_eq!(
        incident_done,
        paths.incidents_resolved_dir.join("incident-001.md")
    );

    queue
        .enqueue_learning_request(&learning_request_document("learn-001"))
        .unwrap();
    queue.claim_next_learning_request().unwrap().unwrap();
    let learning_blocked = QueueLifecycleInterpreter::new(paths.clone(), Vec::new())
        .apply(&SourceLifecycleIntent::for_builtin(
            "block_work_item",
            SourceLifecycleAction::Block,
            WorkItemKind::LearningRequest,
            "learn-001",
        ))
        .unwrap();
    assert_eq!(
        learning_blocked,
        paths.learning_requests_blocked_dir.join("learn-001.md")
    );
}

#[test]
fn lifecycle_interpreter_moves_compiled_blueprint_draft_family() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let plan = compile_compiled_run_plan(&paths, Some("blueprint_codex"), timestamp(NOW)).unwrap();
    let active_dir = paths.runtime_root.join("blueprints/drafts/active");
    fs::create_dir_all(&active_dir).unwrap();
    fs::write(
        active_dir.join("draft-001.json"),
        r#"{
  "draft_id": "draft-001",
  "manifest_id": "manifest-001",
  "root_spec_id": "spec-root-001",
  "root_idea_id": "idea-root-001",
  "source_spec_id": "spec-source-001",
  "draft_index": 1,
  "title": "Draft fixture",
  "summary": "Exercise generic Blueprint draft lifecycle movement.",
  "target_paths": ["src/lib.rs"],
  "acceptance_intent": ["Lifecycle interpreter moves the draft artifact."],
  "context_excerpt": "Compiled Blueprint draft family fixture.",
  "current_revision": 1,
  "created_at": "2026-05-21T00:00:00Z"
}
"#,
    )
    .unwrap();

    let destination = QueueLifecycleInterpreter::new(
        paths.clone(),
        plan.workflow_primitives.work_item_families.clone(),
    )
    .apply(&SourceLifecycleIntent {
        lifecycle_plan_id: "approve_blueprint_draft_after_effect".to_owned(),
        action: SourceLifecycleAction::Complete,
        work_item_family_id: Some("blueprint_draft".to_owned()),
        work_item_kind: Some(WorkItemKind::BlueprintDraft),
        work_item_id: "draft-001".to_owned(),
        reason: None,
    })
    .unwrap();

    assert_eq!(
        destination,
        paths
            .runtime_root
            .join("blueprints/drafts/approved/draft-001.json")
    );
    assert!(destination.is_file());
}
