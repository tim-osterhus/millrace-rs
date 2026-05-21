use std::{
    fs,
    path::{Path, PathBuf},
    process,
};

use serde::Serialize;
use serde_json::json;
use tempfile::TempDir;

use millrace_ai::contracts::{
    BlueprintDraftDocument, BlueprintDraftStatus, BlueprintManifestDocument,
    BlueprintPacketDocument as StateBlueprintPacketDocument,
    BlueprintPromotionRecord as StateBlueprintPromotionRecord, BlueprintSourceWorkItemKind,
    ClosureTargetState as StateClosureTargetState, SpecDocument, SpecSourceType,
    TaskDocument as StateTaskDocument, Timestamp, WorkItemKind,
};
use millrace_ai::workspace::{
    QueueStore, RuntimeOwnershipLockOptions, apply_source_lifecycle_intent,
    block_active_blueprint_draft, enqueue_blueprint_draft, initialize_workspace,
    load_closure_target_state, persist_blueprint_packet, persist_blueprint_promotion,
    read_blueprint_draft, read_blueprint_manifest, save_closure_target_state,
    write_blueprint_manifest,
};
use millrace_ai::{
    FakeRunner, FakeRunnerConfig, FakeRunnerResult, RouterAction, RunnerRawResult, RunnerResult,
    RuntimeStartupOptions, RuntimeTickOptions, RuntimeTickOutcomeKind, SourceLifecycleAction,
    StageRunRequest, StageRunnerAdapter, run_serial_runtime_tick,
    run_serial_runtime_tick_with_runner, startup_runtime_once_for_paths,
};

const NOW: &str = "2026-05-19T12:00:00Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("created_at", value).unwrap()
}

fn startup_options(session_id: &str) -> RuntimeStartupOptions {
    RuntimeStartupOptions {
        requested_mode_id: Some("blueprint_codex".to_owned()),
        lock_options: Some(
            RuntimeOwnershipLockOptions::new(process::id(), "test-host", session_id, NOW).unwrap(),
        ),
        now: Some(timestamp(NOW)),
        ..RuntimeStartupOptions::default()
    }
}

fn tick_options(run_id: &str, request_id: &str) -> RuntimeTickOptions {
    RuntimeTickOptions {
        now: Some(timestamp("2026-05-19T12:10:00Z")),
        run_id: Some(run_id.to_owned()),
        request_id: Some(request_id.to_owned()),
    }
}

fn write_json(path: &Path, value: &impl Serialize) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_string_pretty(value).unwrap() + "\n").unwrap();
}

fn spec_document(spec_id: &str) -> SpecDocument {
    SpecDocument {
        spec_id: spec_id.to_owned(),
        title: "Blueprint source spec".to_owned(),
        summary: "Seed Blueprint Manager output.".to_owned(),
        source_type: SpecSourceType::Idea,
        source_id: Some("idea-blueprint".to_owned()),
        parent_spec_id: None,
        root_idea_id: Some("idea-blueprint".to_owned()),
        root_spec_id: Some(spec_id.to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        goals: vec!["Blueprint runtime effects are applied.".to_owned()],
        non_goals: Vec::new(),
        scope: vec!["runtime".to_owned()],
        constraints: vec!["runtime owns lifecycle movement".to_owned()],
        assumptions: Vec::new(),
        risks: vec!["duplicate manifests".to_owned()],
        target_paths: vec!["src/runtime/blueprint_effects.rs".to_owned()],
        entrypoints: Vec::new(),
        required_skills: Vec::new(),
        decomposition_hints: Vec::new(),
        acceptance: vec!["drafts are queued".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/runtime/blueprint_effects.py".to_owned()],
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn manifest(manifest_id: &str, draft_ids: Vec<String>) -> BlueprintManifestDocument {
    BlueprintManifestDocument {
        schema_version: "1.0".to_owned(),
        kind: "blueprint_manifest".to_owned(),
        manifest_id: manifest_id.to_owned(),
        root_spec_id: "spec-blueprint".to_owned(),
        root_idea_id: "idea-blueprint".to_owned(),
        source_work_item_kind: BlueprintSourceWorkItemKind::Spec,
        source_work_item_id: "spec-blueprint".to_owned(),
        source_spec_id: "spec-blueprint".to_owned(),
        draft_count: draft_ids.len() as u64,
        draft_ids,
        strict_sequence: true,
        spec_summary: "Blueprint state helpers".to_owned(),
        decomposition_strategy: "strict sequence".to_owned(),
        global_acceptance_intent: vec!["drafts claim in order".to_owned()],
        integration_boundary_notes: Vec::new(),
        risk_notes: Vec::new(),
        references: vec!["lab/specs/pending/blueprint.md".to_owned()],
        created_at: timestamp(NOW),
        created_by: "manager_blueprint".to_owned(),
    }
}

fn draft(draft_id: &str, index: u64, depends_on: Vec<String>) -> BlueprintDraftDocument {
    BlueprintDraftDocument {
        schema_version: "1.0".to_owned(),
        kind: "blueprint_draft".to_owned(),
        draft_id: draft_id.to_owned(),
        manifest_id: "manifest-blueprint".to_owned(),
        root_spec_id: "spec-blueprint".to_owned(),
        root_idea_id: "idea-blueprint".to_owned(),
        source_spec_id: "spec-blueprint".to_owned(),
        draft_index: index,
        depends_on_draft_ids: depends_on,
        title: format!("Draft {draft_id}"),
        summary: "Implement a Blueprint slice.".to_owned(),
        scope: vec!["runtime".to_owned()],
        non_goals: Vec::new(),
        target_paths: vec!["src/runtime/blueprint_effects.rs".to_owned()],
        acceptance_intent: vec!["runtime effect passes".to_owned()],
        verification_intent: vec!["cargo test --test blueprint_effects".to_owned()],
        dependency_notes: Vec::new(),
        integration_boundary_notes: Vec::new(),
        context_excerpt: "Blueprint test context".to_owned(),
        current_revision: 0,
        latest_blueprint_id: None,
        latest_critique_id: None,
        status: BlueprintDraftStatus::Queued,
        references: vec!["lab/specs/pending/blueprint.md".to_owned()],
        created_at: timestamp(NOW),
        created_by: "manager_blueprint".to_owned(),
        updated_at: None,
    }
}

fn blueprint_closure_target_state() -> StateClosureTargetState {
    StateClosureTargetState {
        schema_version: "1.0".to_owned(),
        kind: "closure_target_state".to_owned(),
        root_spec_id: "spec-blueprint".to_owned(),
        root_idea_id: "idea-blueprint".to_owned(),
        root_intake_kind: None,
        root_intake_id: None,
        root_spec_path: "millrace-agents/arbiter/contracts/root-specs/spec-blueprint.md".to_owned(),
        root_idea_path: "millrace-agents/arbiter/contracts/ideas/idea-blueprint.md".to_owned(),
        rubric_path: "millrace-agents/arbiter/rubrics/spec-blueprint.md".to_owned(),
        latest_verdict_path: None,
        latest_report_path: None,
        closure_open: true,
        closure_blocked_by_lineage_work: false,
        blocking_work_ids: Vec::new(),
        blocking_work_refs: Vec::new(),
        opened_at: timestamp(NOW),
        closed_at: None,
        last_arbiter_run_id: None,
    }
}

fn closure_packet(draft_id: &str) -> StateBlueprintPacketDocument {
    StateBlueprintPacketDocument {
        schema_version: "1.0".to_owned(),
        kind: "blueprint_packet".to_owned(),
        blueprint_id: format!("blueprint-{draft_id}-r1"),
        draft_id: draft_id.to_owned(),
        manifest_id: "manifest-blueprint".to_owned(),
        root_spec_id: "spec-blueprint".to_owned(),
        root_idea_id: "idea-blueprint".to_owned(),
        revision: 1,
        title: format!("Blueprint packet {draft_id}"),
        implementation_scope: vec!["Exercise Blueprint closure blockers.".to_owned()],
        intended_files: vec!["src/runtime/blueprint_effects.rs".to_owned()],
        design_decisions: vec!["Runtime artifacts remain durable.".to_owned()],
        non_goals: Vec::new(),
        dependency_assumptions: Vec::new(),
        verification_plan: vec!["cargo test --test blueprint_effects".to_owned()],
        task_acceptance: vec!["Closure readiness is conservative.".to_owned()],
        required_checks: vec!["cargo test --test blueprint_effects".to_owned()],
        risk_notes: vec!["same-lineage closure".to_owned()],
        open_questions: Vec::new(),
        references: vec!["lab/specs/pending/blueprint.md".to_owned()],
        created_at: timestamp(NOW),
        created_by: "contractor_blueprint".to_owned(),
    }
}

fn closure_promotion(
    promotion_id: &str,
    packet: &StateBlueprintPacketDocument,
    generated_task_id: &str,
) -> StateBlueprintPromotionRecord {
    let evaluation_id = format!("evaluation-{}", packet.blueprint_id);
    StateBlueprintPromotionRecord {
        schema_version: "1.0".to_owned(),
        kind: "blueprint_promotion".to_owned(),
        promotion_id: promotion_id.to_owned(),
        blueprint_id: packet.blueprint_id.clone(),
        evaluation_id: evaluation_id.clone(),
        draft_id: packet.draft_id.clone(),
        manifest_id: packet.manifest_id.clone(),
        root_spec_id: packet.root_spec_id.clone(),
        root_idea_id: packet.root_idea_id.clone(),
        generated_task_id: generated_task_id.to_owned(),
        generated_task_path: format!("millrace-agents/tasks/queue/{generated_task_id}.md"),
        approved_blueprint_path: format!(
            "millrace-agents/blueprints/packets/approved/{}.json",
            packet.blueprint_id
        ),
        evaluation_path: format!("millrace-agents/blueprints/evaluations/{evaluation_id}.json"),
        promoted_at: timestamp(NOW),
        promoted_by: "runtime".to_owned(),
    }
}

fn blueprint_generated_task(task_id: &str) -> StateTaskDocument {
    StateTaskDocument {
        task_id: task_id.to_owned(),
        title: format!("Generated {task_id}"),
        summary: "Generated Blueprint execution work.".to_owned(),
        root_idea_id: Some("idea-blueprint".to_owned()),
        root_spec_id: Some("spec-blueprint".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        spec_id: Some("spec-blueprint".to_owned()),
        parent_task_id: None,
        incident_id: None,
        target_paths: vec!["src/runtime/blueprint_effects.rs".to_owned()],
        acceptance: vec!["Generated work drains before Arbiter.".to_owned()],
        required_checks: vec!["cargo test --test blueprint_effects".to_owned()],
        references: vec!["lab/specs/pending/blueprint.md".to_owned()],
        risk: vec!["closure readiness".to_owned()],
        depends_on: Vec::new(),
        blocks: Vec::new(),
        tags: vec!["blueprint".to_owned()],
        status_hint: None,
        created_at: timestamp(NOW),
        created_by: "evaluator_blueprint".to_owned(),
        updated_at: None,
    }
}

struct ArbiterArtifactRunner {
    terminal_marker: &'static str,
    verdict_json: &'static str,
    report_text: &'static str,
}

impl StageRunnerAdapter for ArbiterArtifactRunner {
    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        let verdict_path = request
            .preferred_verdict_path
            .as_ref()
            .map(PathBuf::from)
            .expect("arbiter request should include preferred verdict path");
        if let Some(parent) = verdict_path.parent() {
            fs::create_dir_all(parent).map_err(|error| millrace_ai::RunnerError::Io {
                path: parent.display().to_string(),
                message: error.to_string(),
            })?;
        }
        fs::write(&verdict_path, self.verdict_json).map_err(|error| {
            millrace_ai::RunnerError::Io {
                path: verdict_path.display().to_string(),
                message: error.to_string(),
            }
        })?;

        let report_path = request
            .preferred_report_path
            .as_ref()
            .map(PathBuf::from)
            .expect("arbiter request should include preferred report path");
        if let Some(parent) = report_path.parent() {
            fs::create_dir_all(parent).map_err(|error| millrace_ai::RunnerError::Io {
                path: parent.display().to_string(),
                message: error.to_string(),
            })?;
        }
        fs::write(&report_path, self.report_text).map_err(|error| {
            millrace_ai::RunnerError::Io {
                path: report_path.display().to_string(),
                message: error.to_string(),
            }
        })?;

        FakeRunner::with_default(FakeRunnerResult::terminal_marker(self.terminal_marker))
            .unwrap()
            .run(request)
    }
}

#[test]
fn blueprint_manifest_identity_uses_manifest_id_and_legacy_root_keyed_reads() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let first = manifest("manifest-blueprint", vec!["draft-001".to_owned()]);
    let second = manifest("manifest-remediation", vec!["draft-002".to_owned()]);

    let first_path = write_blueprint_manifest(&paths, &first).unwrap();
    let second_path = write_blueprint_manifest(&paths, &second).unwrap();
    assert!(first_path.ends_with("blueprints/manifests/manifest-blueprint.json"));
    assert!(second_path.ends_with("blueprints/manifests/manifest-remediation.json"));
    assert_eq!(
        read_blueprint_manifest(&paths, "manifest-blueprint")
            .unwrap()
            .manifest_id,
        "manifest-blueprint"
    );

    fs::rename(
        &first_path,
        paths
            .runtime_root
            .join("blueprints/manifests/spec-blueprint.json"),
    )
    .unwrap();
    assert_eq!(
        read_blueprint_manifest(&paths, "manifest-blueprint")
            .unwrap()
            .manifest_id,
        "manifest-blueprint"
    );
}

#[test]
fn blueprint_draft_lifecycle_updates_status_through_runtime_intents() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    enqueue_blueprint_draft(&paths, &draft("draft-001", 1, Vec::new())).unwrap();
    let claim = millrace_ai::claim_next_blueprint_draft(&paths, None)
        .unwrap()
        .unwrap();

    let destination = apply_source_lifecycle_intent(
        &paths,
        &millrace_ai::SourceLifecycleIntent {
            lifecycle_plan_id: "approve_blueprint_draft_after_effect".to_owned(),
            action: SourceLifecycleAction::Complete,
            work_item_family_id: Some("blueprint_draft".to_owned()),
            work_item_kind: Some(WorkItemKind::BlueprintDraft),
            work_item_id: claim.work_item_id,
            reason: Some("test approval".to_owned()),
        },
    )
    .unwrap();
    assert!(destination.ends_with("blueprints/drafts/approved/draft-001.json"));
    assert_eq!(
        read_blueprint_draft(&destination).unwrap().status,
        BlueprintDraftStatus::Approved
    );
}

#[test]
fn manager_blueprint_effect_queues_drafts_and_completes_source_after_artifacts_exist() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document("spec-blueprint"))
        .unwrap();
    let planner_run = paths.runs_dir.join("run-blueprint-planner");
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
    let manager_run = planner_run.clone();
    write_json(
        &manager_run.join("blueprint_manifest.json"),
        &manifest(
            "manifest-blueprint",
            vec!["draft-001".to_owned(), "draft-002".to_owned()],
        ),
    );
    write_json(
        &manager_run.join("blueprint_drafts.json"),
        &vec![
            draft("draft-001", 1, Vec::new()),
            draft("draft-002", 2, vec!["draft-001".to_owned()]),
        ],
    );

    let config = FakeRunnerConfig::new(FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"))
        .unwrap()
        .with_node_result(
            "manager_blueprint",
            FakeRunnerResult::terminal_marker("### MANAGER_BLUEPRINT_COMPLETE"),
        );
    let runner = FakeRunner::new(config);
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("blueprint-manager")).unwrap();
    run_tick(
        &mut session,
        tick_options("run-blueprint-planner", "request-blueprint-planner"),
        &runner,
    );
    let outcome = run_tick(
        &mut session,
        tick_options("run-blueprint-manager", "request-blueprint-manager"),
        &runner,
    );

    assert!(paths.specs_done_dir.join("spec-blueprint.md").is_file());
    assert!(
        paths
            .runtime_root
            .join("blueprints/manifests/manifest-blueprint.json")
            .is_file()
    );
    assert!(
        paths
            .runtime_root
            .join("blueprints/drafts/queue/draft-001.json")
            .is_file()
    );
    assert_eq!(
        outcome.stage_result.unwrap().metadata["runtime_effect_handler_id"],
        "manager_blueprint_manifest_to_blueprint_drafts"
    );
}

#[test]
fn manager_blueprint_missing_manifest_routes_pre_mutation_failure_to_mechanic() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document("spec-blueprint"))
        .unwrap();
    let planner_run = paths.runs_dir.join("run-manager-missing-planner");
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

    let config = FakeRunnerConfig::new(FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"))
        .unwrap()
        .with_node_result(
            "manager_blueprint",
            FakeRunnerResult::terminal_marker("### MANAGER_BLUEPRINT_COMPLETE"),
        );
    let runner = FakeRunner::new(config);
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("blueprint-manager-missing"))
            .unwrap();
    run_tick(
        &mut session,
        tick_options(
            "run-manager-missing-planner",
            "request-manager-missing-planner",
        ),
        &runner,
    );
    let outcome = run_tick(
        &mut session,
        tick_options("run-manager-missing", "request-manager-missing"),
        &runner,
    );

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::RunStage);
    assert_eq!(decision.next_node_id.as_deref(), Some("mechanic_blueprint"));
    assert_eq!(
        decision.next_stage_kind_id.as_deref(),
        Some("mechanic_blueprint")
    );
    assert_eq!(
        decision.failure_class.as_deref(),
        Some("blueprint_manifest_missing")
    );
    assert!(paths.specs_active_dir.join("spec-blueprint.md").is_file());
    assert!(!paths.specs_done_dir.join("spec-blueprint.md").exists());
    assert!(!paths.specs_blocked_dir.join("spec-blueprint.md").exists());

    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(
        stage_result.metadata["runtime_effect_handler_id"],
        "manager_blueprint_manifest_to_blueprint_drafts"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_failure_class"],
        "blueprint_manifest_missing"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_failure_policy_id"],
        "manager_blueprint_pre_mutation_artifact_repair"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_mutation_phase"],
        "pre_mutation"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_source_lifecycle_action"],
        "block"
    );
    assert!(
        Path::new(
            stage_result.metadata["runtime_effect_result_path"]
                .as_str()
                .unwrap()
        )
        .is_file()
    );
}

#[test]
fn manager_blueprint_duplicate_manifest_blocks_source_conservatively() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document("spec-blueprint"))
        .unwrap();
    write_blueprint_manifest(
        &paths,
        &manifest("manifest-blueprint", vec!["draft-existing".to_owned()]),
    )
    .unwrap();
    let planner_run = paths.runs_dir.join("run-manager-duplicate-planner");
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
    let manager_run = planner_run.clone();
    write_json(
        &manager_run.join("blueprint_manifest.json"),
        &manifest("manifest-blueprint", vec!["draft-001".to_owned()]),
    );
    write_json(
        &manager_run.join("blueprint_drafts.json"),
        &vec![draft("draft-001", 1, Vec::new())],
    );

    let config = FakeRunnerConfig::new(FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"))
        .unwrap()
        .with_node_result(
            "manager_blueprint",
            FakeRunnerResult::terminal_marker("### MANAGER_BLUEPRINT_COMPLETE"),
        );
    let runner = FakeRunner::new(config);
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("blueprint-manager-duplicate"))
            .unwrap();
    run_tick(
        &mut session,
        tick_options(
            "run-manager-duplicate-planner",
            "request-manager-duplicate-planner",
        ),
        &runner,
    );
    let outcome = run_tick(
        &mut session,
        tick_options("run-manager-duplicate", "request-manager-duplicate"),
        &runner,
    );

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Blocked);
    assert_eq!(
        decision.failure_class.as_deref(),
        Some("blueprint_manifest_duplicate")
    );
    assert!(paths.specs_blocked_dir.join("spec-blueprint.md").is_file());

    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(
        stage_result.metadata["runtime_effect_failure_policy_id"],
        "manager_blueprint_pre_mutation_conservative_block"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_mutation_phase"],
        "pre_mutation"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_created_paths"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert!(
        stage_result.metadata["runtime_effect_failure_message"]
            .as_str()
            .unwrap()
            .contains("blueprint_manifest_duplicate")
    );
    assert!(
        Path::new(
            stage_result.metadata["runtime_effect_result_path"]
                .as_str()
                .unwrap()
        )
        .is_file()
    );
}

#[test]
fn manager_blueprint_partial_mutation_blocks_source_with_created_path_diagnostics() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_spec(&spec_document("spec-blueprint"))
        .unwrap();
    let planner_run = paths.runs_dir.join("run-manager-partial-planner");
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
    let manager_run = planner_run.clone();
    write_json(
        &manager_run.join("blueprint_manifest.json"),
        &manifest("manifest-blueprint", vec!["draft-001".to_owned()]),
    );
    write_json(
        &manager_run.join("blueprint_drafts.json"),
        &vec![draft("draft-001", 1, Vec::new())],
    );
    let draft_temp_path = paths
        .runtime_root
        .join("blueprints/drafts/queue/.draft-001.json.tmp");
    fs::create_dir_all(&draft_temp_path).unwrap();

    let config = FakeRunnerConfig::new(FakeRunnerResult::terminal_marker("### PLANNER_COMPLETE"))
        .unwrap()
        .with_node_result(
            "manager_blueprint",
            FakeRunnerResult::terminal_marker("### MANAGER_BLUEPRINT_COMPLETE"),
        );
    let runner = FakeRunner::new(config);
    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("blueprint-manager-partial"))
            .unwrap();
    run_tick(
        &mut session,
        tick_options(
            "run-manager-partial-planner",
            "request-manager-partial-planner",
        ),
        &runner,
    );
    let outcome = run_tick(
        &mut session,
        tick_options("run-manager-partial", "request-manager-partial"),
        &runner,
    );

    let decision = outcome.router_decision.as_ref().unwrap();
    assert_eq!(decision.action, RouterAction::Blocked);
    assert_eq!(
        decision.failure_class.as_deref(),
        Some("blueprint_partial_mutation")
    );
    assert!(paths.specs_blocked_dir.join("spec-blueprint.md").is_file());
    assert!(
        paths
            .runtime_root
            .join("blueprints/manifests/manifest-blueprint.json")
            .is_file()
    );

    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(
        stage_result.metadata["runtime_effect_failure_policy_id"],
        "manager_blueprint_partial_mutation_conservative_block"
    );
    assert_eq!(
        stage_result.metadata["runtime_effect_mutation_phase"],
        "partial_mutation"
    );
    let created_paths = stage_result.metadata["runtime_effect_created_paths"]
        .as_array()
        .unwrap();
    assert!(created_paths.iter().any(|path| {
        path.as_str()
            .is_some_and(|path| path.ends_with("blueprints/manifests/manifest-blueprint.json"))
    }));
    assert!(
        Path::new(
            stage_result.metadata["runtime_effect_result_path"]
                .as_str()
                .unwrap()
        )
        .is_file()
    );
}

#[test]
fn closure_readiness_suppresses_arbiter_on_blueprint_artifacts_and_generated_work() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    save_closure_target_state(&paths, &blueprint_closure_target_state()).unwrap();

    enqueue_blueprint_draft(&paths, &draft("draft-blocked", 1, Vec::new())).unwrap();
    millrace_ai::claim_next_blueprint_draft(&paths, Some("spec-blueprint"))
        .unwrap()
        .unwrap();
    block_active_blueprint_draft(&paths, "draft-blocked").unwrap();

    let candidate_packet = closure_packet("draft-candidate");
    persist_blueprint_packet(&paths, &candidate_packet, "candidates").unwrap();
    let approved_packet = closure_packet("draft-approved");
    persist_blueprint_packet(&paths, &approved_packet, "approved").unwrap();
    let missing_task_packet = closure_packet("draft-missing-generated");
    persist_blueprint_promotion(
        &paths,
        &closure_promotion(
            "promotion-missing-generated",
            &missing_task_packet,
            "task-missing-generated",
        ),
    )
    .unwrap();

    let generated_packet = closure_packet("draft-generated-open");
    persist_blueprint_promotion(
        &paths,
        &closure_promotion(
            "promotion-generated-open",
            &generated_packet,
            "task-generated-open",
        ),
    )
    .unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&blueprint_generated_task("task-generated-open"))
        .unwrap();
    queue
        .claim_next_execution_task(Some("spec-blueprint"))
        .unwrap()
        .unwrap();
    queue.mark_task_blocked("task-generated-open").unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("blueprint-closure-blocked"))
            .unwrap();
    let outcome = run_serial_runtime_tick(
        &mut session,
        tick_options(
            "run-closure-blueprint-blocked",
            "request-closure-blueprint-blocked",
        ),
    )
    .unwrap();

    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::NoWork);
    assert!(outcome.stage_request.is_none());
    let target = load_closure_target_state(&paths, "spec-blueprint").unwrap();
    assert!(target.closure_blocked_by_lineage_work);
    for expected in [
        "draft-blocked",
        "blueprint-draft-candidate-r1",
        "blueprint-draft-approved-r1",
        "promotion-missing-generated",
        "task-generated-open",
    ] {
        assert!(
            target.blocking_work_ids.contains(&expected.to_owned()),
            "missing blocking id {expected}: {:?}",
            target.blocking_work_ids
        );
    }

    let ref_summary = target
        .blocking_work_refs
        .iter()
        .map(|reference| {
            (
                reference.blocker_type.as_str(),
                reference.work_item_id.as_deref(),
                reference.artifact_path.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    assert!(ref_summary.iter().any(|entry| {
        entry.0 == "blueprint_draft"
            && entry.1 == Some("draft-blocked")
            && entry
                .2
                .is_some_and(|path| path.ends_with("blueprints/drafts/blocked/draft-blocked.json"))
    }));
    assert!(ref_summary.iter().any(|entry| {
        entry.0 == "blueprint_candidate"
            && entry.1 == Some("blueprint-draft-candidate-r1")
            && entry.2.is_some_and(|path| {
                path.ends_with("blueprints/packets/candidates/blueprint-draft-candidate-r1.json")
            })
    }));
    assert!(ref_summary.iter().any(|entry| {
        entry.0 == "blueprint_approved"
            && entry.1 == Some("blueprint-draft-approved-r1")
            && entry.2.is_some_and(|path| {
                path.ends_with("blueprints/packets/approved/blueprint-draft-approved-r1.json")
            })
    }));
    assert!(ref_summary.iter().any(|entry| {
        entry.0 == "blueprint_promotion"
            && entry.1 == Some("promotion-missing-generated")
            && entry.2.is_some_and(|path| {
                path.ends_with("blueprints/promotions/promotion-missing-generated.json")
            })
    }));
}

#[test]
fn closure_readiness_runs_arbiter_after_blueprint_artifacts_and_generated_work_drain() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    save_closure_target_state(&paths, &blueprint_closure_target_state()).unwrap();

    let approved_packet = closure_packet("draft-drained");
    persist_blueprint_packet(&paths, &approved_packet, "approved").unwrap();
    persist_blueprint_promotion(
        &paths,
        &closure_promotion("promotion-drained", &approved_packet, "task-drained"),
    )
    .unwrap();
    let queue = QueueStore::from_paths(paths.clone());
    queue
        .enqueue_task(&blueprint_generated_task("task-drained"))
        .unwrap();
    queue
        .claim_next_execution_task(Some("spec-blueprint"))
        .unwrap()
        .unwrap();
    queue.mark_task_done("task-drained").unwrap();

    let mut session =
        startup_runtime_once_for_paths(&paths, startup_options("blueprint-closure-drained"))
            .unwrap();
    let runner = ArbiterArtifactRunner {
        terminal_marker: "### ARBITER_COMPLETE",
        verdict_json: "{\"status\":\"pass\"}\n",
        report_text: "# Arbiter Report\n\nBlueprint lineage drained.\n",
    };
    let outcome = run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options(
            "run-closure-blueprint-drained",
            "request-closure-blueprint-drained",
        ),
        &runner,
    )
    .unwrap();

    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::StageDispatched);
    assert_eq!(
        outcome.stage_result.as_ref().unwrap().stage_kind_id,
        "arbiter"
    );
    let target = load_closure_target_state(&paths, "spec-blueprint").unwrap();
    assert!(!target.closure_blocked_by_lineage_work);
    assert!(target.blocking_work_ids.is_empty());
    assert!(target.blocking_work_refs.is_empty());
}

fn run_tick(
    session: &mut millrace_ai::RuntimeStartupSession,
    options: RuntimeTickOptions,
    runner: &impl millrace_ai::StageRunnerAdapter,
) -> millrace_ai::RuntimeTickDispatchOutcome {
    millrace_ai::run_serial_runtime_tick_with_runner(session, options, runner).unwrap()
}
