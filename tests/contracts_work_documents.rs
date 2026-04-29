mod support;

use serde_json::json;

use millrace_ai::contracts::{
    IncidentDecision, IncidentDocument, IncidentSeverity, LearningRequestAction,
    LearningRequestDocument, LearningStageName, Plane, SpecDocument, SpecSourceType, StageName,
    TaskDocument, TaskStatusHint, Timestamp, WorkDocument, WorkItemKind,
};
use millrace_ai::work_documents::{
    parse_incident_document, parse_spec_json_import, parse_task_document, parse_task_json_import,
    parse_work_document_with_source, render_incident_document, render_learning_request_document,
    render_spec_document, render_task_document, render_work_document,
};

use support::parity::{fixture_path, read_fixture};

const NOW: &str = "2026-04-15T00:00:00Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("created_at", value).unwrap()
}

fn task_document() -> TaskDocument {
    TaskDocument {
        task_id: "task-roundtrip".to_owned(),
        title: "Implement queue contracts".to_owned(),
        summary: "queue test".to_owned(),
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        spec_id: Some("spec-root-001".to_owned()),
        parent_task_id: None,
        incident_id: None,
        target_paths: vec!["src/work_documents.rs".to_owned()],
        acceptance: vec!["queue behavior is deterministic".to_owned()],
        required_checks: vec!["cargo test --test contracts_work_documents".to_owned()],
        references: vec!["docs/rust-port-roadmap.md".to_owned()],
        risk: vec!["queue drift".to_owned()],
        depends_on: vec!["task-prereq".to_owned()],
        blocks: vec!["task-next".to_owned()],
        tags: vec!["slice-1".to_owned()],
        status_hint: Some(TaskStatusHint::Queued),
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn spec_document() -> SpecDocument {
    SpecDocument {
        spec_id: "spec-roundtrip".to_owned(),
        title: "Contracts spec".to_owned(),
        summary: "Define canonical runtime contracts".to_owned(),
        source_type: SpecSourceType::Manual,
        source_id: Some("idea-001".to_owned()),
        parent_spec_id: None,
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        goals: vec!["define typed models".to_owned()],
        non_goals: vec!["implement scheduling".to_owned()],
        scope: vec!["contract parsing".to_owned()],
        constraints: vec!["stay deterministic".to_owned()],
        assumptions: vec!["Python reference is available".to_owned()],
        risks: vec!["schema drift".to_owned()],
        target_paths: vec!["src/contracts/".to_owned()],
        entrypoints: vec!["src/lib.rs".to_owned()],
        required_skills: vec!["builder-core".to_owned()],
        decomposition_hints: vec!["keep parser separate".to_owned()],
        acceptance: vec!["tests pass".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/workspace/work_documents.py".to_owned()],
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn incident_document() -> IncidentDocument {
    IncidentDocument {
        incident_id: "inc-roundtrip".to_owned(),
        title: "Parity gap".to_owned(),
        summary: "Closure needs remediation".to_owned(),
        root_idea_id: Some("idea-001".to_owned()),
        root_spec_id: Some("spec-root-001".to_owned()),
        source_task_id: Some("task-roundtrip".to_owned()),
        source_spec_id: Some("spec-roundtrip".to_owned()),
        source_stage: StageName::Auditor,
        source_plane: Plane::Planning,
        failure_class: "arbiter_parity_gap".to_owned(),
        severity: IncidentSeverity::Medium,
        needs_planning: true,
        trigger_reason: "parity gap found".to_owned(),
        observed_symptoms: vec!["rendered markdown lost lineage".to_owned()],
        failed_attempts: vec!["builder pass".to_owned()],
        consultant_decision: IncidentDecision::NeedsPlanning,
        evidence_paths: vec!["millrace-agents/runs/run-001/report.md".to_owned()],
        related_run_ids: vec!["run-001".to_owned()],
        related_stage_results: vec!["request-001.json".to_owned()],
        references: vec!["docs/rust-port-roadmap.md".to_owned()],
        opened_at: timestamp(NOW),
        opened_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn learning_request_document() -> LearningRequestDocument {
    LearningRequestDocument {
        learning_request_id: "learn-roundtrip".to_owned(),
        title: "Improve checker skill".to_owned(),
        summary: "Use observed run evidence".to_owned(),
        requested_action: LearningRequestAction::Improve,
        target_skill_id: Some("checker-core".to_owned()),
        target_stage: Some(LearningStageName::Curator),
        source_refs: vec!["run:run-001".to_owned()],
        preferred_output_paths: vec![
            "millrace-agents/skills/stage/execution/checker-core/SKILL.md".to_owned(),
        ],
        trigger_metadata: json!({
            "source_stage": "doublechecker",
            "terminal_result": "DOUBLECHECK_PASS"
        }),
        originating_run_ids: vec!["run-001".to_owned()],
        artifact_paths: vec![
            "millrace-agents/runs/run-001/stage_results/request-001.json".to_owned(),
        ],
        references: vec!["docs/rust-port-roadmap.md".to_owned()],
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

#[test]
fn work_documents_round_trip_for_task_spec_incident_and_learning_request() {
    let documents = [
        WorkDocument::Task(task_document()),
        WorkDocument::Spec(spec_document()),
        WorkDocument::Incident(incident_document()),
        WorkDocument::LearningRequest(learning_request_document()),
    ];

    for document in documents {
        let raw = render_work_document(&document);

        assert!(raw.starts_with(&format!("# {}\n", document.title())));
        assert!(!raw.contains("---"));
        assert!(!raw.contains("Schema-Version:"));
        assert!(!raw.contains("Kind:"));

        let parsed = parse_work_document_with_source(&raw, document.kind().as_str()).unwrap();
        assert_eq!(parsed, document);
    }
}

#[test]
fn python_rendered_work_document_fixtures_round_trip_exactly() {
    let fixtures = [
        ("work_documents/task.md", WorkItemKind::Task, "Fixture task"),
        ("work_documents/spec.md", WorkItemKind::Spec, "Fixture spec"),
        (
            "work_documents/incident.md",
            WorkItemKind::Incident,
            "Fixture incident",
        ),
        (
            "work_documents/learning_request.md",
            WorkItemKind::LearningRequest,
            "Fixture learning request",
        ),
    ];

    for (relative_path, expected_kind, expected_title) in fixtures {
        let path = fixture_path(relative_path);
        assert!(path.ends_with(relative_path));
        assert!(path.starts_with(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")));

        let raw = read_fixture(relative_path).expect("read Python work-document fixture");
        let parsed = parse_work_document_with_source(&raw, relative_path).unwrap();

        assert_eq!(parsed.kind(), expected_kind);
        assert_eq!(parsed.title(), expected_title);
        assert_eq!(render_work_document(&parsed), raw);
    }
}

#[test]
fn renderers_preserve_root_lineage_and_omit_empty_relationship_blocks() {
    let mut task = task_document();
    task.depends_on.clear();
    task.blocks.clear();

    let rendered = (
        render_task_document(&task),
        render_spec_document(&spec_document()),
        render_incident_document(&incident_document()),
    );

    assert!(rendered.0.contains("Root-Idea-ID: idea-001"));
    assert!(rendered.0.contains("Root-Spec-ID: spec-root-001"));
    assert!(rendered.1.contains("Root-Idea-ID: idea-001"));
    assert!(rendered.1.contains("Root-Spec-ID: spec-root-001"));
    assert!(rendered.2.contains("Root-Idea-ID: idea-001"));
    assert!(rendered.2.contains("Root-Spec-ID: spec-root-001"));
    assert!(!rendered.0.contains("Depends-On:"));
    assert!(!rendered.0.contains("Blocks:"));
}

#[test]
fn parse_task_document_treats_blank_optional_scalars_as_omitted() {
    let raw = format!(
        "# Queue task\n\n\
         Task-ID: queue-task\n\
         Title: Queue task\n\
         Summary: queue test\n\
         Root-Idea-ID:\n\n\
         Root-Spec-ID:\n\n\
         Spec-ID: spec-001\n\
         Parent-Task-ID:\n\n\
         Incident-ID:\n\n\
         Status-Hint: queued\n\
         Created-At: {NOW}\n\
         Created-By: manager\n\n\
         Target-Paths:\n\
         - millrace/queue_store.py\n\n\
         Acceptance:\n\
         - queue claim succeeds\n\n\
         Required-Checks:\n\
         - cargo test\n\n\
         References:\n\
         - millrace-issue-1.md\n\n\
         Risk:\n\
         - queue intake regression\n"
    );

    let document = parse_task_document(&raw).unwrap();

    assert_eq!(document.task_id, "queue-task");
    assert_eq!(document.root_idea_id, None);
    assert_eq!(document.root_spec_id, None);
    assert_eq!(document.spec_id.as_deref(), Some("spec-001"));
    assert_eq!(document.parent_task_id, None);
    assert_eq!(document.incident_id, None);
}

#[test]
fn task_relationship_placeholders_normalize_to_empty_lists() {
    let raw = format!(
        "# Queue task with placeholder dependency\n\n\
         Task-ID: queue-task-placeholder\n\
         Title: Queue task with placeholder dependency\n\
         Summary: queue test\n\
         Root-Idea-ID: idea-001\n\
         Root-Spec-ID: spec-root-001\n\
         Spec-ID: spec-root-001\n\
         Status-Hint: queued\n\
         Created-At: {NOW}\n\
         Created-By: manager\n\n\
         Depends-On:\n\
         - none\n\n\
         Blocks:\n\
         - NONE\n\n\
         Target-Paths:\n\
         - millrace/queue_store.py\n\n\
         Acceptance:\n\
         - queue claim succeeds\n\n\
         Required-Checks:\n\
         - cargo test\n\n\
         References:\n\
         - millrace-issue-1.md\n\n\
         Risk:\n\
         - none\n"
    );

    let document = parse_task_document(&raw).unwrap();

    assert!(document.depends_on.is_empty());
    assert!(document.blocks.is_empty());
    assert_eq!(document.risk, ["none"]);
}

#[test]
fn parser_rejects_json_frontmatter_title_mismatch_and_missing_required_lists() {
    let frontmatter_error =
        parse_work_document_with_source("---\n{\"kind\":\"task\"}\n---\n", "frontmatter.md")
            .unwrap_err();
    assert!(frontmatter_error.to_string().contains("JSON frontmatter"));

    let mut task = task_document();
    task.title = "Rendered title".to_owned();
    let raw = render_task_document(&task).replace("# Rendered title", "# Different title");
    let mismatch = parse_task_document(&raw).unwrap_err();
    assert!(mismatch.to_string().contains("H1 title"));

    let missing_list = format!(
        "# Missing list\n\
         Task-ID: task-missing-list\n\
         Title: Missing list\n\
         Created-At: {NOW}\n\
         Created-By: tests\n"
    );
    let error = parse_task_document(&missing_list).unwrap_err();
    assert!(error.to_string().contains("target_paths"));
}

#[test]
fn json_imports_validate_task_spec_metadata_and_render_canonical_markdown() {
    let task_json = serde_json::to_string(&json!({
        "schema_version": "1.0",
        "kind": "task",
        "task_id": "task-json-import",
        "title": "Task JSON import",
        "summary": "json intake",
        "root_idea_id": "idea-json-import",
        "root_spec_id": "spec-json-import",
        "spec_id": "spec-json-import",
        "target_paths": ["src/work_documents.rs"],
        "acceptance": ["json import works"],
        "required_checks": ["cargo test --test contracts_work_documents"],
        "references": ["tests/contracts_work_documents.rs"],
        "risk": ["schema drift"],
        "created_at": NOW,
        "created_by": "tests"
    }))
    .unwrap();
    let task = parse_task_json_import(&task_json).unwrap();
    assert_eq!(task.task_id, "task-json-import");
    assert!(render_task_document(&task).contains("Root-Idea-ID: idea-json-import\n"));

    let spec_json = serde_json::to_string(&json!({
        "schema_version": "1.0",
        "kind": "spec",
        "spec_id": "spec-json-import",
        "title": "Spec JSON import",
        "summary": "json intake",
        "source_type": "manual",
        "root_idea_id": "idea-json-import",
        "root_spec_id": "spec-json-import",
        "goals": ["parse spec JSON"],
        "constraints": ["stay deterministic"],
        "acceptance": ["json import works"],
        "references": ["tests/contracts_work_documents.rs"],
        "created_at": NOW,
        "created_by": "tests"
    }))
    .unwrap();
    let spec = parse_spec_json_import(&spec_json).unwrap();
    assert_eq!(spec.spec_id, "spec-json-import");
    assert!(render_spec_document(&spec).contains("Source-Type: manual\n"));

    let wrong_kind = parse_task_json_import(&spec_json).unwrap_err();
    assert!(wrong_kind.to_string().contains("kind"));
}

#[test]
fn parser_rejects_stage_plane_mismatch_and_bad_learning_metadata() {
    let incident = format!(
        "# Stage mismatch\n\n\
         Incident-ID: inc-mismatch\n\
         Title: Stage mismatch\n\
         Summary: bad routing\n\
         Source-Stage: builder\n\
         Source-Plane: planning\n\
         Failure-Class: illegal_state\n\
         Trigger-Reason: bad routing\n\
         Consultant-Decision: blocked\n\
         Opened-At: {NOW}\n\
         Opened-By: tests\n"
    );
    let incident_error = parse_incident_document(&incident).unwrap_err();
    assert!(incident_error.to_string().contains("source_stage"));

    let mut learning = learning_request_document();
    learning.trigger_metadata = json!([]);
    let raw = render_learning_request_document(&learning);
    let error = parse_work_document_with_source(&raw, "learning.md").unwrap_err();
    assert!(error.to_string().contains("trigger_metadata"));
}
