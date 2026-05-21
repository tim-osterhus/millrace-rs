include!("blueprint_effects.rs");

use millrace_ai::contracts::{
    BlueprintCritiqueDocument, BlueprintEvaluationDecision, BlueprintEvaluationDocument,
    BlueprintPacketDocument, TaskDocument,
};
use millrace_ai::work_documents::render_task_document;

fn packet(draft_id: &str) -> BlueprintPacketDocument {
    BlueprintPacketDocument {
        schema_version: "1.0".to_owned(),
        kind: "blueprint_packet".to_owned(),
        blueprint_id: format!("blueprint-{draft_id}-r1"),
        draft_id: draft_id.to_owned(),
        manifest_id: "manifest-blueprint".to_owned(),
        root_spec_id: "spec-blueprint".to_owned(),
        root_idea_id: "idea-blueprint".to_owned(),
        revision: 1,
        title: format!("Implement {draft_id}"),
        implementation_scope: vec!["Add Blueprint runtime effects.".to_owned()],
        intended_files: vec!["src/runtime/blueprint_effects.rs".to_owned()],
        design_decisions: vec!["Runtime effects own mutation.".to_owned()],
        non_goals: Vec::new(),
        dependency_assumptions: Vec::new(),
        verification_plan: vec!["cargo test --test blueprint_planning_loop".to_owned()],
        task_acceptance: vec!["Blueprint task is generated.".to_owned()],
        required_checks: vec!["cargo test --test blueprint_planning_loop".to_owned()],
        risk_notes: vec!["partial mutation".to_owned()],
        open_questions: Vec::new(),
        references: vec!["lab/specs/pending/blueprint.md".to_owned()],
        created_at: timestamp(NOW),
        created_by: "contractor_blueprint".to_owned(),
    }
}

fn evaluation(packet: &BlueprintPacketDocument) -> BlueprintEvaluationDocument {
    BlueprintEvaluationDocument {
        schema_version: "1.0".to_owned(),
        kind: "blueprint_evaluation".to_owned(),
        evaluation_id: format!("evaluation-{}", packet.blueprint_id),
        blueprint_id: packet.blueprint_id.clone(),
        draft_id: packet.draft_id.clone(),
        manifest_id: packet.manifest_id.clone(),
        root_spec_id: packet.root_spec_id.clone(),
        root_idea_id: packet.root_idea_id.clone(),
        decision: BlueprintEvaluationDecision::Approved,
        rubric_findings: vec!["Blueprint is coherent.".to_owned()],
        lineage_consistency_findings: vec!["Lineage matches draft.".to_owned()],
        dependency_findings: Vec::new(),
        verification_findings: vec!["Check is concrete.".to_owned()],
        overlap_findings: Vec::new(),
        required_task_fields: vec!["task_id".to_owned(), "target_paths".to_owned()],
        critique_id: None,
        references: vec!["lab/specs/pending/blueprint.md".to_owned()],
        created_at: timestamp(NOW),
        created_by: "evaluator_blueprint".to_owned(),
    }
}

fn rejection_evaluation(packet: &BlueprintPacketDocument) -> BlueprintEvaluationDocument {
    BlueprintEvaluationDocument {
        schema_version: "1.0".to_owned(),
        kind: "blueprint_evaluation".to_owned(),
        evaluation_id: format!("evaluation-{}", packet.blueprint_id),
        blueprint_id: packet.blueprint_id.clone(),
        draft_id: packet.draft_id.clone(),
        manifest_id: packet.manifest_id.clone(),
        root_spec_id: packet.root_spec_id.clone(),
        root_idea_id: packet.root_idea_id.clone(),
        decision: BlueprintEvaluationDecision::Rejected,
        rubric_findings: vec!["Blueprint needs revision.".to_owned()],
        lineage_consistency_findings: vec!["Lineage matches draft.".to_owned()],
        dependency_findings: vec!["Dependency sequencing needs detail.".to_owned()],
        verification_findings: vec!["Verification plan is incomplete.".to_owned()],
        overlap_findings: Vec::new(),
        required_task_fields: Vec::new(),
        critique_id: Some(format!("critique-{}", packet.blueprint_id)),
        references: vec!["lab/specs/pending/blueprint.md".to_owned()],
        created_at: timestamp(NOW),
        created_by: "evaluator_blueprint".to_owned(),
    }
}

fn critique(packet: &BlueprintPacketDocument) -> BlueprintCritiqueDocument {
    let evaluation_id = format!("evaluation-{}", packet.blueprint_id);
    BlueprintCritiqueDocument {
        schema_version: "1.0".to_owned(),
        kind: "blueprint_critique".to_owned(),
        critique_id: format!("critique-{}", packet.blueprint_id),
        evaluation_id,
        blueprint_id: packet.blueprint_id.clone(),
        draft_id: packet.draft_id.clone(),
        manifest_id: packet.manifest_id.clone(),
        root_spec_id: packet.root_spec_id.clone(),
        root_idea_id: packet.root_idea_id.clone(),
        revision: packet.revision,
        required_changes: vec!["Add concrete runtime-effect evidence.".to_owned()],
        scope_issues: Vec::new(),
        dependency_issues: Vec::new(),
        verification_issues: vec!["Add rejection-path coverage.".to_owned()],
        acceptance_issues: Vec::new(),
        risk_issues: Vec::new(),
        blocking_reason: "Blueprint is not ready for execution promotion.".to_owned(),
        resolved_by_blueprint_id: None,
        resolved_at: None,
        references: vec!["lab/specs/pending/blueprint.md".to_owned()],
        created_at: timestamp(NOW),
        created_by: "evaluator_blueprint".to_owned(),
    }
}

fn generated_task(packet: &BlueprintPacketDocument) -> TaskDocument {
    TaskDocument {
        task_id: format!("task-{}", packet.draft_id),
        title: format!("Execute {}", packet.draft_id),
        summary: "Generated task from approved Blueprint.".to_owned(),
        root_idea_id: Some(packet.root_idea_id.clone()),
        root_spec_id: Some(packet.root_spec_id.clone()),
        root_intake_kind: None,
        root_intake_id: None,
        spec_id: Some("spec-blueprint".to_owned()),
        parent_task_id: None,
        incident_id: None,
        target_paths: packet.intended_files.clone(),
        acceptance: packet.task_acceptance.clone(),
        required_checks: packet.required_checks.clone(),
        references: vec!["lab/specs/pending/blueprint.md".to_owned()],
        risk: packet.risk_notes.clone(),
        depends_on: Vec::new(),
        blocks: Vec::new(),
        tags: Vec::new(),
        status_hint: None,
        created_at: timestamp(NOW),
        created_by: "evaluator_blueprint".to_owned(),
        updated_at: None,
    }
}

#[test]
fn blueprint_mode_promotes_approved_draft_to_execution_before_arbiter() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document("spec-blueprint"))
        .unwrap();

    write_json(
        &paths
            .runs_dir
            .join("run-loop-planner")
            .join("planner_disposition.json"),
        &json!({
            "schema_version": "1.0",
            "kind": "planner_disposition",
            "source_work_item_family_id": "spec",
            "source_work_item_id": "spec-blueprint",
            "disposition": "active_source_ready_for_manager",
            "emitted_spec_ids": [],
            "refined_active_source": false,
            "recommended_next_action": "active_source_ready_for_manager",
            "created_at": NOW,
            "created_by": "planner"
        }),
    );
    write_json(
        &paths
            .runs_dir
            .join("run-loop-planner")
            .join("blueprint_manifest.json"),
        &manifest("manifest-blueprint", vec!["draft-001".to_owned()]),
    );
    write_json(
        &paths
            .runs_dir
            .join("run-loop-planner")
            .join("blueprint_drafts.json"),
        &vec![draft("draft-001", 1, Vec::new())],
    );
    let packet = packet("draft-001");
    let contractor_run = paths.runs_dir.join("run-loop-contractor");
    write_json(&contractor_run.join("blueprint_packet.json"), &packet);
    fs::write(
        contractor_run.join("blueprint.md"),
        "# Blueprint\n\nRuntime-owned packet.\n",
    )
    .unwrap();
    let evaluator_run = contractor_run.clone();
    write_json(
        &evaluator_run.join("blueprint_evaluation.json"),
        &evaluation(&packet),
    );
    fs::write(
        evaluator_run.join("generated_task.md"),
        render_task_document(&generated_task(&packet)),
    )
    .unwrap();

    let config = FakeRunnerConfig::new(FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"))
        .unwrap()
        .with_node_result(
            "manager_blueprint",
            FakeRunnerResult::terminal_marker("### MANAGER_BLUEPRINT_COMPLETE"),
        )
        .with_node_result(
            "contractor_blueprint",
            FakeRunnerResult::terminal_marker("### BLUEPRINT_CANDIDATE_READY"),
        )
        .with_node_result(
            "evaluator_blueprint",
            FakeRunnerResult::terminal_marker("### BLUEPRINT_APPROVED"),
        )
        .with_node_result(
            "builder",
            FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE"),
        );
    let runner = FakeRunner::new(config);
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("blueprint-loop")).unwrap();

    let planner = run_tick(
        &mut session,
        tick_options("run-loop-planner", "request-loop-planner"),
        &runner,
    );
    let manager = run_tick(
        &mut session,
        tick_options("run-loop-manager", "request-loop-manager"),
        &runner,
    );
    let contractor = run_tick(
        &mut session,
        tick_options("run-loop-contractor", "request-loop-contractor"),
        &runner,
    );
    let evaluator = run_tick(
        &mut session,
        tick_options("run-loop-evaluator", "request-loop-evaluator"),
        &runner,
    );
    let builder = run_tick(
        &mut session,
        tick_options("run-loop-builder", "request-loop-builder"),
        &runner,
    );
    let outcomes = [planner, manager, contractor, evaluator, builder];

    let stage_order = outcomes
        .iter()
        .map(|outcome| {
            outcome
                .stage_result
                .as_ref()
                .unwrap()
                .stage_kind_id
                .as_str()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        stage_order,
        vec![
            "planner",
            "manager_blueprint",
            "contractor_blueprint",
            "evaluator_blueprint",
            "builder"
        ]
    );
    assert!(
        paths
            .runtime_root
            .join("blueprints/drafts/approved/draft-001.json")
            .is_file()
    );
    assert!(paths.tasks_active_dir.join("task-draft-001.md").is_file());
    let approved_packet_path = paths
        .runtime_root
        .join("blueprints/packets/approved/blueprint-draft-001-r1.json");
    let evaluation_path = paths
        .runtime_root
        .join("blueprints/evaluations/evaluation-blueprint-draft-001-r1.json");
    let promotion_path = paths
        .runtime_root
        .join("blueprints/promotions/promotion-evaluation-blueprint-draft-001-r1.json");
    assert!(approved_packet_path.is_file());
    assert!(evaluation_path.is_file());
    assert!(promotion_path.is_file());

    let approved_packet: BlueprintPacketDocument =
        serde_json::from_str(&fs::read_to_string(&approved_packet_path).unwrap()).unwrap();
    assert_eq!(approved_packet.blueprint_id, "blueprint-draft-001-r1");
    let persisted_evaluation: BlueprintEvaluationDocument =
        serde_json::from_str(&fs::read_to_string(&evaluation_path).unwrap()).unwrap();
    assert_eq!(
        persisted_evaluation.decision,
        BlueprintEvaluationDecision::Approved
    );
    let promotion: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&promotion_path).unwrap()).unwrap();
    assert_eq!(promotion["generated_task_id"], "task-draft-001");
    assert_eq!(
        promotion["approved_blueprint_path"],
        "millrace-agents/blueprints/packets/approved/blueprint-draft-001-r1.json"
    );
    assert_eq!(
        promotion["evaluation_path"],
        "millrace-agents/blueprints/evaluations/evaluation-blueprint-draft-001-r1.json"
    );
    let evaluator_result = outcomes[3].stage_result.as_ref().unwrap();
    assert_eq!(
        evaluator_result.metadata["runtime_effect_source_lifecycle_plan_id"],
        "approve_blueprint_draft_after_effect"
    );
    assert_eq!(
        evaluator_result.metadata["runtime_effect_source_lifecycle_action"],
        "complete"
    );
}

#[test]
fn blueprint_mode_rejection_persists_critique_and_routes_back_to_contractor() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document("spec-blueprint"))
        .unwrap();

    let planner_run = paths.runs_dir.join("run-loop-reject-planner");
    write_json(
        &planner_run.join("planner_disposition.json"),
        &json!({
            "schema_version": "1.0",
            "kind": "planner_disposition",
            "source_work_item_family_id": "spec",
            "source_work_item_id": "spec-blueprint",
            "disposition": "active_source_ready_for_manager",
            "emitted_spec_ids": [],
            "refined_active_source": false,
            "recommended_next_action": "active_source_ready_for_manager",
            "created_at": NOW,
            "created_by": "planner"
        }),
    );
    write_json(
        &planner_run.join("blueprint_manifest.json"),
        &manifest("manifest-blueprint", vec!["draft-001".to_owned()]),
    );
    write_json(
        &planner_run.join("blueprint_drafts.json"),
        &vec![draft("draft-001", 1, Vec::new())],
    );
    let packet = packet("draft-001");
    let contractor_run = paths.runs_dir.join("run-loop-reject-contractor");
    write_json(&contractor_run.join("blueprint_packet.json"), &packet);
    fs::write(
        contractor_run.join("blueprint.md"),
        "# Blueprint\n\nRejected packet body.\n",
    )
    .unwrap();
    let rejection = rejection_evaluation(&packet);
    write_json(
        &contractor_run.join("blueprint_evaluation.json"),
        &rejection,
    );
    let critique = critique(&packet);
    write_json(&contractor_run.join("blueprint_critique.json"), &critique);

    let config = FakeRunnerConfig::new(FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"))
        .unwrap()
        .with_node_result(
            "manager_blueprint",
            FakeRunnerResult::terminal_marker("### MANAGER_BLUEPRINT_COMPLETE"),
        )
        .with_node_result(
            "contractor_blueprint",
            FakeRunnerResult::terminal_marker("### BLUEPRINT_CANDIDATE_READY"),
        )
        .with_node_result(
            "evaluator_blueprint",
            FakeRunnerResult::terminal_marker("### BLUEPRINT_REJECTED"),
        );
    let runner = FakeRunner::new(config);
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("blueprint-reject-loop")).unwrap();

    let planner = run_tick(
        &mut session,
        tick_options("run-loop-reject-planner", "request-loop-reject-planner"),
        &runner,
    );
    let manager = run_tick(
        &mut session,
        tick_options("run-loop-reject-manager", "request-loop-reject-manager"),
        &runner,
    );
    let contractor = run_tick(
        &mut session,
        tick_options(
            "run-loop-reject-contractor",
            "request-loop-reject-contractor",
        ),
        &runner,
    );
    let evaluator = run_tick(
        &mut session,
        tick_options("run-loop-reject-evaluator", "request-loop-reject-evaluator"),
        &runner,
    );
    let mut outcomes = vec![planner, manager, contractor, evaluator];
    if outcomes.last().unwrap().stage_result.is_none() {
        outcomes.push(run_tick(
            &mut session,
            tick_options(
                "run-loop-reject-evaluator-dispatch",
                "request-loop-reject-evaluator-dispatch",
            ),
            &runner,
        ));
    }

    let stage_order = outcomes
        .iter()
        .filter_map(|outcome| {
            outcome
                .stage_result
                .as_ref()
                .map(|stage_result| stage_result.stage_kind_id.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        stage_order,
        vec![
            "planner",
            "manager_blueprint",
            "contractor_blueprint",
            "evaluator_blueprint"
        ],
        "unexpected rejection loop outcomes: {:?}",
        outcomes
            .iter()
            .map(|outcome| (
                &outcome.kind,
                outcome.stage_result.as_ref().map(|stage_result| (
                    stage_result.stage_kind_id.as_str(),
                    stage_result.terminal_result.as_str()
                )),
                outcome.router_decision.as_ref()
            ))
            .collect::<Vec<_>>()
    );
    let evaluator_index = outcomes
        .iter()
        .position(|outcome| {
            outcome
                .stage_result
                .as_ref()
                .is_some_and(|stage_result| stage_result.stage_kind_id == "evaluator_blueprint")
        })
        .unwrap();

    let evaluator_decision = outcomes[evaluator_index].router_decision.as_ref().unwrap();
    assert_eq!(evaluator_decision.action, RouterAction::RunStage);
    assert_eq!(
        evaluator_decision.next_node_id.as_deref(),
        Some("contractor_blueprint")
    );
    assert_eq!(
        evaluator_decision.next_stage_kind_id.as_deref(),
        Some("contractor_blueprint")
    );

    let evaluation_path = paths
        .runtime_root
        .join("blueprints/evaluations/evaluation-blueprint-draft-001-r1.json");
    let rejected_packet_path = paths
        .runtime_root
        .join("blueprints/packets/rejected/blueprint-draft-001-r1.json");
    let rejected_markdown_path = paths
        .runtime_root
        .join("blueprints/packets/rejected/blueprint-draft-001-r1.md");
    let critique_path = paths
        .runtime_root
        .join("blueprints/critiques/open/critique-blueprint-draft-001-r1.json");
    assert!(evaluation_path.is_file());
    assert!(rejected_packet_path.is_file());
    assert!(rejected_markdown_path.is_file());
    assert!(critique_path.is_file());
    assert!(
        !paths
            .runtime_root
            .join("blueprints/packets/candidates/blueprint-draft-001-r1.json")
            .exists()
    );

    let persisted_evaluation: BlueprintEvaluationDocument =
        serde_json::from_str(&fs::read_to_string(&evaluation_path).unwrap()).unwrap();
    assert_eq!(
        persisted_evaluation.decision,
        BlueprintEvaluationDecision::Rejected
    );
    assert_eq!(
        persisted_evaluation.critique_id.as_deref(),
        Some("critique-blueprint-draft-001-r1")
    );
    let persisted_critique: BlueprintCritiqueDocument =
        serde_json::from_str(&fs::read_to_string(&critique_path).unwrap()).unwrap();
    assert_eq!(persisted_critique.resolved_by_blueprint_id, None);
    assert_eq!(persisted_critique.resolved_at, None);

    let active_draft = read_blueprint_draft(
        &paths
            .runtime_root
            .join("blueprints/drafts/active/draft-001.json"),
    )
    .unwrap();
    assert_eq!(active_draft.current_revision, 1);
    assert_eq!(
        active_draft.latest_blueprint_id.as_deref(),
        Some("blueprint-draft-001-r1")
    );
    assert_eq!(
        active_draft.latest_critique_id.as_deref(),
        Some("critique-blueprint-draft-001-r1")
    );
    let evaluator_result = outcomes[evaluator_index].stage_result.as_ref().unwrap();
    assert_eq!(
        evaluator_result.metadata["runtime_effect_handler_id"],
        "evaluator_blueprint_rejected_to_draft_revision"
    );
    assert_eq!(
        evaluator_result.metadata["runtime_effect_decision"],
        "continue_route"
    );
    assert_eq!(
        evaluator_result.metadata["runtime_effect_source_lifecycle_action"],
        serde_json::Value::Null
    );
}
