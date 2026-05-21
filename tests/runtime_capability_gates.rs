use std::{cell::RefCell, fs, path::Path, rc::Rc};

use millrace_ai::{
    FakeRunner, FakeRunnerConfig, FakeRunnerResult, RuntimeDaemonSupervisor, RuntimeStartupOptions,
    RuntimeTickOptions, RuntimeTickOutcomeKind, StageRunRequest, StageRunnerAdapter,
    contracts::{
        ApprovalPolicyRef, CapabilityDecisionState, CapabilityEnforcementMode,
        CapabilityEvidenceStatus, CapabilityScope, CapabilitySupportDecision,
        CapabilitySupportState, ExecutionCapabilityGrant, ExecutionTerminalResult, Plane,
        ResultClass, StageName, TaskDocument, TerminalResult, Timestamp, WorkItemKind,
    },
    runtime::{
        ExecutionCapabilityApprovalRequest, approve_execution_capability_request,
        ensure_execution_capability_approval, evaluate_stage_request_capabilities,
        list_execution_capability_approvals,
    },
    workspace::{QueueStore, WorkspacePaths, initialize_workspace, save_snapshot},
};
use serde_json::{Value, json};
use tempfile::TempDir;

const NOW: &str = "2026-05-18T07:30:00Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("timestamp", value).unwrap()
}

fn task_document(task_id: &str) -> TaskDocument {
    TaskDocument {
        task_id: task_id.to_owned(),
        title: format!("Task {task_id}"),
        summary: "capability gate test".to_owned(),
        root_idea_id: Some("idea-capability".to_owned()),
        root_spec_id: Some("spec-capability".to_owned()),
        root_intake_kind: None,
        root_intake_id: None,
        spec_id: Some("spec-capability".to_owned()),
        parent_task_id: None,
        incident_id: None,
        target_paths: vec!["src/runtime/capability_gates.rs".to_owned()],
        acceptance: vec!["capability gate blocks before dispatch".to_owned()],
        required_checks: vec!["cargo test --test runtime_capability_gates".to_owned()],
        references: vec!["../millrace-py/src/millrace_ai/runtime/capability_gates.py".to_owned()],
        risk: vec!["gates must run before stage-start side effects".to_owned()],
        depends_on: Vec::new(),
        blocks: Vec::new(),
        tags: vec!["runtime".to_owned(), "capability-gates".to_owned()],
        status_hint: None,
        created_at: timestamp(NOW),
        created_by: "tests".to_owned(),
        updated_at: None,
    }
}

fn edit_builder_node(paths: &WorkspacePaths, edit: impl FnOnce(&mut Value)) {
    let graph_path = paths.graphs_dir.join("execution/standard.json");
    let mut graph: Value = serde_json::from_str(&fs::read_to_string(&graph_path).unwrap()).unwrap();
    let builder = graph["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node["node_id"] == "builder")
        .unwrap();
    edit(builder);
    fs::write(
        &graph_path,
        serde_json::to_string_pretty(&graph).unwrap() + "\n",
    )
    .unwrap();
}

fn startup_options(mode: &str) -> RuntimeStartupOptions {
    RuntimeStartupOptions {
        requested_mode_id: Some(mode.to_owned()),
        now: Some(timestamp(NOW)),
        ..RuntimeStartupOptions::default()
    }
}

fn daemon_options(mode: &str) -> RuntimeStartupOptions {
    RuntimeStartupOptions {
        runtime_mode: millrace_ai::contracts::RuntimeMode::Daemon,
        ..startup_options(mode)
    }
}

fn tick_options(run_id: &str, request_id: &str) -> RuntimeTickOptions {
    RuntimeTickOptions {
        now: Some(timestamp(NOW)),
        run_id: Some(run_id.to_owned()),
        request_id: Some(request_id.to_owned()),
    }
}

fn add_denied_network_request(paths: &WorkspacePaths) {
    edit_builder_node(paths, |builder| {
        builder["execution_capability_requests"] = json!([
            {
                "request_id": "builder-denied-network",
                "capability_id": "network.access",
                "access": "execute",
                "scope": {"kind": "network_class", "value": "package_index"},
                "reason": "network access is denied by default"
            }
        ]);
    });
}

fn add_missing_evidence_runtime_control_request(paths: &WorkspacePaths) {
    edit_builder_node(paths, |builder| {
        builder["execution_capability_requests"] = json!([
            {
                "request_id": "builder-runtime-control",
                "capability_id": "runtime.control",
                "access": "execute",
                "scope": {"kind": "runtime_action", "value": "pause"},
                "requires_enforcement": true,
                "reason": "requires explicit capability evidence"
            }
        ]);
        builder["execution_capability_policies"] = json!([
            {
                "capability_id": "runtime.control",
                "decision": "allow",
                "reason": "allow runtime control in missing-evidence test"
            }
        ]);
    });
}

#[derive(Clone)]
struct GateTestRunner {
    run_count: Rc<RefCell<u64>>,
}

impl GateTestRunner {
    fn new() -> Self {
        Self {
            run_count: Rc::new(RefCell::new(0)),
        }
    }

    fn run_count(&self) -> u64 {
        *self.run_count.borrow()
    }
}

impl StageRunnerAdapter for GateTestRunner {
    fn run(
        &self,
        _request: &StageRunRequest,
    ) -> millrace_ai::RunnerResult<millrace_ai::RunnerRawResult> {
        *self.run_count.borrow_mut() += 1;
        panic!("capability gate test runner must not be invoked for blocked grants")
    }

    fn evaluate_capability_grant(
        &self,
        grant: &ExecutionCapabilityGrant,
        request: &StageRunRequest,
    ) -> CapabilitySupportDecision {
        if grant.decision_state != CapabilityDecisionState::Granted {
            return CapabilitySupportDecision {
                runner_id: request
                    .runner_name
                    .clone()
                    .unwrap_or_else(|| "gate-test".to_owned()),
                invocation_context_ref: request.stage.as_str().to_owned(),
                grant_id: grant.grant_id.clone(),
                support_state: CapabilitySupportState::Unsupported,
                enforcement_mode: CapabilityEnforcementMode::NotApplicable,
                limitations: Vec::new(),
                evidence_available: Vec::new(),
                reason: "grant is not granted".to_owned(),
            };
        }

        let framework_owned = matches!(
            grant.capability_id.as_str(),
            "runner.invoke" | "artifact.read" | "artifact.write" | "evidence.emit"
        );
        CapabilitySupportDecision {
            runner_id: request
                .runner_name
                .clone()
                .unwrap_or_else(|| "gate-test".to_owned()),
            invocation_context_ref: request.stage.as_str().to_owned(),
            grant_id: grant.grant_id.clone(),
            support_state: CapabilitySupportState::Supported,
            enforcement_mode: if framework_owned || grant.capability_id == "runtime.control" {
                grant.enforcement_mode
            } else {
                CapabilityEnforcementMode::AdvisoryOnly
            },
            limitations: Vec::new(),
            evidence_available: if grant.capability_id == "runtime.control" {
                Vec::new()
            } else {
                vec![
                    "runner_invocation".to_owned(),
                    "runner_completion".to_owned(),
                ]
            },
            reason: "test runner reports support but omits capability_evidence".to_owned(),
        }
    }
}

fn approval_grant() -> ExecutionCapabilityGrant {
    ExecutionCapabilityGrant {
        grant_id: "grant-package-install".to_owned(),
        request_id: "request-package-install".to_owned(),
        capability_id: "package.install".to_owned(),
        access: "execute".to_owned(),
        scope: CapabilityScope {
            kind: "package_manager".to_owned(),
            value: "cargo".to_owned(),
            metadata: Default::default(),
        },
        required: true,
        decision_state: CapabilityDecisionState::ApprovalRequired,
        enforcement_mode: CapabilityEnforcementMode::NotApplicable,
        approval_policy_ref: Some(ApprovalPolicyRef {
            policy_id: "operator.package_install".to_owned(),
            gate_scope: "stage".to_owned(),
            expiration_seconds: None,
            required_decision: "approved".to_owned(),
        }),
        evidence_requirements: Vec::new(),
        evidence_status: CapabilityEvidenceStatus::NotRequired,
        decision_reason: "package installs require operator approval".to_owned(),
        resolved_by: "test".to_owned(),
        fingerprint: String::new(),
    }
}

fn request_with_grant(root: &Path, grant: ExecutionCapabilityGrant) -> StageRunRequest {
    let run_dir = root.join("millrace-agents/runs/run-approval");
    fs::create_dir_all(&run_dir).unwrap();
    let mut request = StageRunRequest {
        request_id: "request-approval".to_owned(),
        run_id: "run-approval".to_owned(),
        plane: Plane::Execution,
        stage: StageName::Builder,
        request_kind: millrace_ai::RequestKind::ActiveWorkItem,
        mode_id: "default_codex".to_owned(),
        compiled_plan_id: "plan-approval".to_owned(),
        launch_plan_id: None,
        lane_id: None,
        node_id: "builder".to_owned(),
        stage_kind_id: "builder".to_owned(),
        running_status_marker: "BUILDER_RUNNING".to_owned(),
        legal_terminal_markers: Vec::new(),
        allowed_result_classes_by_outcome: Default::default(),
        entrypoint_path: root
            .join("millrace-agents/entrypoints/execution/builder.md")
            .display()
            .to_string(),
        entrypoint_contract_id: Some("builder.contract.v1".to_owned()),
        required_skill_paths: Vec::new(),
        attached_skill_paths: Vec::new(),
        active_work_item_family_id: None,
        active_work_item_kind: Some(WorkItemKind::Task),
        active_work_item_id: Some("task-approval".to_owned()),
        active_work_item_path: Some(
            root.join("millrace-agents/tasks/active/task-approval.md")
                .display()
                .to_string(),
        ),
        closure_target_path: None,
        closure_target_root_spec_id: None,
        closure_target_root_idea_id: None,
        canonical_root_spec_path: None,
        canonical_seed_idea_path: None,
        preferred_rubric_path: None,
        preferred_verdict_path: None,
        preferred_report_path: None,
        run_dir: run_dir.display().to_string(),
        summary_status_path: root
            .join("millrace-agents/state/execution_status.md")
            .display()
            .to_string(),
        runtime_snapshot_path: root
            .join("millrace-agents/state/runtime_snapshot.json")
            .display()
            .to_string(),
        recovery_counters_path: root
            .join("millrace-agents/state/recovery_counters.json")
            .display()
            .to_string(),
        preferred_troubleshoot_report_path: Some(
            run_dir.join("troubleshoot_report.md").display().to_string(),
        ),
        runtime_error_code: None,
        runtime_error_report_path: None,
        runtime_error_catalog_path: None,
        skill_revision_evidence_path: None,
        runner_name: Some("gate-test".to_owned()),
        model_name: None,
        thinking_level: None,
        model_reasoning_effort: None,
        timeout_seconds: 60,
        execution_capability_grants: vec![grant],
        capability_support_decisions: Vec::new(),
        request_context_profile_id: None,
        context_bundle_path: None,
        context_artifact_refs: Vec::new(),
        context_render_plan_id: None,
        rendered_prompt_context_path: None,
    };
    request.validate().unwrap();
    request
}

#[test]
fn approval_required_gate_reuses_pending_and_allows_after_approval() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    let now = timestamp(NOW);
    let request = request_with_grant(&paths.root, approval_grant());

    let blocked = evaluate_stage_request_capabilities(
        &paths,
        &request,
        &|_, _| panic!("approval-required grants should not ask runner support"),
        &now,
    )
    .unwrap();

    assert!(!blocked.allowed);
    assert_eq!(
        blocked.failure_class.as_deref(),
        Some("capability_approval_required")
    );
    let listing = list_execution_capability_approvals(&paths).unwrap();
    assert_eq!(listing.pending.len(), 1);
    let approval_id = listing.pending[0].approval_id.clone();
    assert_eq!(listing.pending[0].requested_by, "runtime");
    assert_eq!(
        listing.pending[0].reason,
        "package installs require operator approval"
    );
    assert_eq!(listing.pending[0].grant.grant_id, "grant-package-install");

    let blocked_again = evaluate_stage_request_capabilities(
        &paths,
        &request,
        &|_, _| panic!("approval-required grants should not ask runner support"),
        &now,
    )
    .unwrap();
    assert_eq!(blocked_again.approval_ids, vec![approval_id.clone()]);
    assert_eq!(
        list_execution_capability_approvals(&paths)
            .unwrap()
            .pending
            .len(),
        1
    );

    let resolved = approve_execution_capability_request(
        &paths,
        &approval_id,
        "operator",
        "approved for retry",
        &now,
    )
    .unwrap();
    assert_eq!(resolved.status.as_str(), "approved");
    assert_eq!(resolved.decided_by.as_deref(), Some("operator"));
    assert_eq!(
        resolved.decision_reason.as_deref(),
        Some("approved for retry")
    );
    let listing = list_execution_capability_approvals(&paths).unwrap();
    assert!(listing.pending.is_empty());
    assert_eq!(listing.resolved.len(), 1);

    let allowed = evaluate_stage_request_capabilities(
        &paths,
        &request,
        &|_, _| panic!("approved approval-required grants should not ask runner support"),
        &now,
    )
    .unwrap();
    assert!(allowed.allowed);
    assert_eq!(allowed.approval_ids, vec![approval_id]);
    assert!(
        list_execution_capability_approvals(&paths)
            .unwrap()
            .pending
            .is_empty()
    );
}

#[test]
fn serial_dispatch_blocks_denied_grant_before_runner_invocation() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    add_denied_network_request(&paths);
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-denied"))
        .unwrap();

    let mut session =
        millrace_ai::startup_runtime_once_for_paths(&paths, startup_options("default_codex"))
            .unwrap();
    let runner = GateTestRunner::new();
    let outcome = millrace_ai::run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-denied", "request-denied"),
        &runner,
    )
    .unwrap();

    assert_eq!(runner.run_count(), 0);
    assert_eq!(outcome.kind, RuntimeTickOutcomeKind::StageDispatched);
    assert_eq!(
        outcome
            .runner_raw_result
            .as_ref()
            .unwrap()
            .failure_capability_class
            .as_deref(),
        Some("capability_grant_denied")
    );
    let stage_result = outcome.stage_result.as_ref().unwrap();
    assert_eq!(
        stage_result.terminal_result,
        TerminalResult::Execution(ExecutionTerminalResult::Blocked)
    );
    assert_eq!(stage_result.result_class, ResultClass::RecoverableFailure);
    assert_eq!(
        stage_result.metadata["failure_class"],
        "capability_grant_denied"
    );
    assert_eq!(
        stage_result.metadata["blocked_origin"],
        "runtime_capability_gate"
    );
    assert!(
        paths
            .runs_dir
            .join("run-denied/capability_gate.request-denied.json")
            .is_file()
    );
    let events = fs::read_to_string(paths.logs_dir.join("runtime_events.jsonl")).unwrap();
    assert!(events.contains("\"event_type\":\"capability_gate_evaluated\""));
    assert!(!events.contains("\"event_type\":\"stage_started\""));

    session.finish().unwrap();
}

#[test]
fn daemon_dispatch_blocks_missing_evidence_before_runner_invocation() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    add_missing_evidence_runtime_control_request(&paths);
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-missing-evidence"))
        .unwrap();

    let mut session =
        millrace_ai::startup_runtime_daemon_for_paths(&paths, daemon_options("default_codex"))
            .unwrap();
    assert!(
        session.snapshot.active_runs_by_plane.is_empty(),
        "unexpected active runs at daemon startup: {:?}",
        session.snapshot.active_runs_by_plane
    );
    let runtime_control_grant = session
        .compiled_plan
        .execution_graph
        .nodes
        .iter()
        .find(|node| node.node_id == "builder")
        .unwrap()
        .execution_capability_grants
        .iter()
        .find(|grant| grant.capability_id == "runtime.control")
        .unwrap();
    assert!(
        runtime_control_grant
            .evidence_requirements
            .iter()
            .any(|requirement| requirement == "capability_evidence")
    );
    assert!(runtime_control_grant.required);
    assert!(
        session
            .compiled_plan
            .execution_graph
            .compiled_entries
            .iter()
            .any(|entry| entry.entry_key.as_str() == "task" && entry.node_id == "builder"),
        "task entry was not builder: {:?}",
        session.compiled_plan.execution_graph.compiled_entries
    );
    let runner = GateTestRunner::new();
    let mut supervisor = RuntimeDaemonSupervisor::new(runner.clone());
    let dispatched = supervisor
        .dispatch_ready_work(
            &mut session,
            tick_options("run-missing-evidence", "request-missing-evidence"),
        )
        .unwrap();

    assert_eq!(dispatched, 1);
    assert_eq!(runner.run_count(), 0);
    assert!(
        paths
            .runs_dir
            .join("run-missing-evidence/capability_gate.request-missing-evidence.json")
            .is_file()
    );
    let events = fs::read_to_string(paths.logs_dir.join("runtime_events.jsonl")).unwrap();
    assert!(events.contains("\"event_type\":\"capability_gate_evaluated\""));
    assert!(!events.contains("\"event_type\":\"stage_started\""));

    session.snapshot.paused = true;
    save_snapshot(&paths, &session.snapshot).unwrap();
    let cycle = supervisor
        .run_cycle(&mut session, tick_options("run-unused", "request-unused"))
        .unwrap();
    assert_eq!(cycle.kind, RuntimeTickOutcomeKind::Paused);
    assert_eq!(cycle.completions.len(), 1);
    let stage_result = cycle.completions[0].stage_result.as_ref().unwrap();
    assert_eq!(
        stage_result.metadata["failure_class"],
        "capability_evidence_missing"
    );
    assert_eq!(
        stage_result.metadata["blocked_origin"],
        "runtime_capability_gate"
    );
    assert_eq!(stage_result.metadata["failure_scope"], "runtime_policy");

    session.close().unwrap();
}

#[test]
fn approved_capability_gate_allows_serial_dispatch_without_duplicate_pending_approval() {
    let temp = TempDir::new().unwrap();
    let paths = initialize_workspace(temp.path().join("workspace")).unwrap();
    edit_builder_node(&paths, |builder| {
        builder["execution_capability_requests"] = json!([
            {
                "request_id": "builder-package-install",
                "capability_id": "package.install",
                "access": "execute",
                "scope": {"kind": "package_manager", "value": "cargo"},
                "reason": "install build dependency"
            }
        ]);
    });
    fs::write(
        &paths.runtime_config_file,
        "[execution_capabilities.defaults]\npackage_install = \"approval_required\"\n",
    )
    .unwrap();
    QueueStore::from_paths(paths.clone())
        .enqueue_task(&task_document("task-approved"))
        .unwrap();

    let mut session =
        millrace_ai::startup_runtime_once_for_paths(&paths, startup_options("default_codex"))
            .unwrap();
    let grant = session
        .compiled_plan
        .execution_graph
        .nodes
        .iter()
        .find(|node| node.node_id == "builder")
        .unwrap()
        .execution_capability_grants
        .iter()
        .find(|grant| grant.capability_id == "package.install")
        .unwrap()
        .clone();
    let approval = ensure_execution_capability_approval(
        &paths,
        ExecutionCapabilityApprovalRequest {
            request_id: "request-approved",
            run_id: "run-approved",
            plane: Plane::Execution,
            node_id: "builder",
            stage_kind_id: "builder",
            work_item_kind: Some(WorkItemKind::Task),
            work_item_id: Some("task-approved"),
            grant: &grant,
            now: &timestamp(NOW),
            requested_by: "runtime",
        },
    )
    .unwrap();
    approve_execution_capability_request(
        &paths,
        &approval.approval_id,
        "operator",
        "approved retry",
        &timestamp(NOW),
    )
    .unwrap();
    assert_eq!(
        list_execution_capability_approvals(&paths)
            .unwrap()
            .pending
            .len(),
        0
    );

    let allowed_runner = FakeRunner::new(
        FakeRunnerConfig::new(FakeRunnerResult::terminal_marker("### BUILDER_COMPLETE")).unwrap(),
    );
    let allowed = millrace_ai::run_serial_runtime_tick_with_runner(
        &mut session,
        tick_options("run-approved", "request-approved"),
        &allowed_runner,
    )
    .unwrap();
    assert_eq!(allowed.kind, RuntimeTickOutcomeKind::StageDispatched);
    assert_eq!(
        list_execution_capability_approvals(&paths)
            .unwrap()
            .pending
            .len(),
        0
    );
    assert_eq!(
        list_execution_capability_approvals(&paths)
            .unwrap()
            .resolved
            .len(),
        1
    );
    assert_eq!(
        allowed.stage_result.unwrap().terminal_result,
        TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete)
    );

    session.finish().unwrap();
}
