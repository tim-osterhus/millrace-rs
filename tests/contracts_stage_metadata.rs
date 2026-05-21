use millrace_ai::contracts::{
    ContractError, ExecutionStageName, ExecutionTerminalResult, IdentifierErrorReason,
    IncidentDecision, IncidentSeverity, LearningRequestAction, LearningStageName,
    LearningTerminalResult, LoopEdgeKind, MailboxCommand, Plane, PlanningStageName,
    PlanningTerminalResult, ProbeStatusHint, ReloadOutcome, ResultClass, RootIntakeKind,
    RuntimeErrorCode, RuntimeMode, StageName, TaskStatusHint, TerminalResult, WatcherMode,
    WorkItemKind, allowed_result_classes_by_outcome, allowed_work_item_kinds,
    blocked_terminal_for_plane, known_stage_values, known_stage_values_for_plane,
    legal_terminal_markers, legal_terminal_results, parse_terminal_marker_for_plane,
    running_status_marker, stage_allows_work_item_kind, stage_metadata, stage_metadata_for_value,
    stage_name_for_plane, stage_name_for_value, stage_plane, terminal_result_for_plane,
    validate_safe_identifier, validate_stage_result_class, validate_terminal_marker_for_stage,
};

fn values<T: Copy>(items: &'static [T], as_str: impl Fn(T) -> &'static str) -> Vec<&'static str> {
    items.iter().copied().map(as_str).collect()
}

#[test]
fn enum_values_match_python_reference_contracts() {
    assert_eq!(
        values(Plane::ALL, Plane::as_str),
        ["execution", "planning", "learning"]
    );
    assert_eq!(
        values(ExecutionStageName::ALL, ExecutionStageName::as_str),
        [
            "builder",
            "integrator",
            "checker",
            "fixer",
            "doublechecker",
            "updater",
            "troubleshooter",
            "consultant",
        ]
    );
    assert_eq!(
        values(PlanningStageName::ALL, PlanningStageName::as_str),
        [
            "recon",
            "planner",
            "manager",
            "manager_blueprint",
            "contractor_blueprint",
            "evaluator_blueprint",
            "mechanic",
            "mechanic_blueprint",
            "auditor",
            "arbiter",
        ]
    );
    assert_eq!(
        values(LearningStageName::ALL, LearningStageName::as_str),
        ["analyst", "professor", "curator", "librarian"]
    );
    assert_eq!(
        values(
            ExecutionTerminalResult::ALL,
            ExecutionTerminalResult::as_str
        ),
        [
            "BUILDER_COMPLETE",
            "INTEGRATION_COMPLETE",
            "CHECKER_PASS",
            "FIX_NEEDED",
            "FIXER_COMPLETE",
            "DOUBLECHECK_PASS",
            "UPDATE_COMPLETE",
            "TROUBLESHOOT_COMPLETE",
            "CONSULT_COMPLETE",
            "NEEDS_PLANNING",
            "BLOCKED",
        ]
    );
    assert_eq!(
        values(PlanningTerminalResult::ALL, PlanningTerminalResult::as_str),
        [
            "RECON_TO_EXECUTION",
            "RECON_TO_PLANNING",
            "RECON_BLOCKED",
            "RECON_NOOP",
            "PLANNER_COMPLETE",
            "MANAGER_COMPLETE",
            "MANAGER_BLUEPRINT_COMPLETE",
            "BLUEPRINT_CANDIDATE_READY",
            "BLUEPRINT_APPROVED",
            "BLUEPRINT_REJECTED",
            "MECHANIC_COMPLETE",
            "MECHANIC_BLUEPRINT_COMPLETE",
            "AUDITOR_COMPLETE",
            "ARBITER_COMPLETE",
            "REMEDIATION_NEEDED",
            "BLOCKED",
        ]
    );
    assert_eq!(
        values(LearningTerminalResult::ALL, LearningTerminalResult::as_str),
        [
            "ANALYST_COMPLETE",
            "ANALYST_NOOP",
            "PROFESSOR_COMPLETE",
            "PROFESSOR_NOOP",
            "CURATOR_COMPLETE",
            "CURATOR_NOOP",
            "LIBRARIAN_COMPLETE",
            "LIBRARIAN_NOOP",
            "BLOCKED",
        ]
    );
    assert_eq!(
        values(ResultClass::ALL, ResultClass::as_str),
        [
            "success",
            "no_op",
            "followup_needed",
            "recoverable_failure",
            "escalate_planning",
            "blocked",
        ]
    );
    assert_eq!(
        values(WorkItemKind::ALL, WorkItemKind::as_str),
        [
            "task",
            "probe",
            "spec",
            "incident",
            "learning_request",
            "blueprint_draft",
        ]
    );
    assert_eq!(
        values(LearningRequestAction::ALL, LearningRequestAction::as_str),
        ["create", "improve", "promote", "export", "install"]
    );
    assert_eq!(
        values(TaskStatusHint::ALL, TaskStatusHint::as_str),
        ["queued", "active", "blocked", "done"]
    );
    assert_eq!(
        values(ProbeStatusHint::ALL, ProbeStatusHint::as_str),
        ["queued", "active", "blocked", "done"]
    );
    assert_eq!(
        values(RootIntakeKind::ALL, RootIntakeKind::as_str),
        ["idea", "probe", "manual", "incident", "derived_spec"]
    );
    assert_eq!(
        values(IncidentSeverity::ALL, IncidentSeverity::as_str),
        ["low", "medium", "high", "critical"]
    );
    assert_eq!(
        values(IncidentDecision::ALL, IncidentDecision::as_str),
        ["needs_planning", "blocked"]
    );
    assert_eq!(
        values(RuntimeMode::ALL, RuntimeMode::as_str),
        ["once", "daemon"]
    );
    assert_eq!(
        values(WatcherMode::ALL, WatcherMode::as_str),
        ["watch", "poll", "off"]
    );
    assert_eq!(
        values(ReloadOutcome::ALL, ReloadOutcome::as_str),
        ["applied", "failed_retained_previous_plan"]
    );
    assert_eq!(
        values(RuntimeErrorCode::ALL, RuntimeErrorCode::as_str),
        [
            "planning_work_item_completion_conflict",
            "execution_work_item_completion_conflict",
            "planning_post_stage_apply_failed",
            "execution_post_stage_apply_failed",
            "recon_handoff_invalid",
            "stage_work_item_ownership_invalid",
        ]
    );
    assert_eq!(
        values(MailboxCommand::ALL, MailboxCommand::as_str),
        [
            "stop",
            "pause",
            "resume",
            "reload_config",
            "add_task",
            "add_probe",
            "add_spec",
            "add_idea",
            "retry_active",
            "clear_stale_state",
            "cancel_work_item",
            "archive_blocked_task",
            "supersede_task",
            "retarget_task_dependency",
            "resolve_incident",
            "cancel_incident",
            "archive_invalid_incident",
            "approve_execution_capability",
            "deny_execution_capability",
        ]
    );
    assert_eq!(
        values(LoopEdgeKind::ALL, LoopEdgeKind::as_str),
        ["normal", "retry", "escalation", "handoff", "terminal"]
    );
}

#[test]
fn stage_work_item_ownership_matrix_matches_runtime_contracts() {
    for stage in [
        StageName::Builder,
        StageName::Integrator,
        StageName::Checker,
        StageName::Fixer,
        StageName::Doublechecker,
        StageName::Updater,
        StageName::Troubleshooter,
        StageName::Consultant,
    ] {
        assert_eq!(allowed_work_item_kinds(stage), [WorkItemKind::Task]);
        assert!(stage_allows_work_item_kind(stage, WorkItemKind::Task));
        assert!(!stage_allows_work_item_kind(stage, WorkItemKind::Spec));
    }

    assert_eq!(
        allowed_work_item_kinds(StageName::Recon),
        [WorkItemKind::Probe]
    );
    assert_eq!(
        allowed_work_item_kinds(StageName::Planner),
        [WorkItemKind::Spec, WorkItemKind::Incident]
    );
    assert_eq!(
        allowed_work_item_kinds(StageName::Manager),
        [WorkItemKind::Spec, WorkItemKind::Incident]
    );
    assert_eq!(
        allowed_work_item_kinds(StageName::Mechanic),
        [WorkItemKind::Spec, WorkItemKind::Incident]
    );
    assert_eq!(
        allowed_work_item_kinds(StageName::Auditor),
        [WorkItemKind::Incident]
    );
    assert!(allowed_work_item_kinds(StageName::Arbiter).is_empty());
    assert_eq!(
        allowed_work_item_kinds(StageName::Analyst),
        [WorkItemKind::LearningRequest]
    );
    assert_eq!(
        allowed_work_item_kinds(StageName::Professor),
        [WorkItemKind::LearningRequest]
    );
    assert_eq!(
        allowed_work_item_kinds(StageName::Curator),
        [WorkItemKind::LearningRequest]
    );
    assert_eq!(
        allowed_work_item_kinds(StageName::Librarian),
        [WorkItemKind::LearningRequest]
    );
    assert!(stage_allows_work_item_kind(
        StageName::Librarian,
        WorkItemKind::LearningRequest
    ));
    assert!(!stage_allows_work_item_kind(
        StageName::Librarian,
        WorkItemKind::Task
    ));
}

#[test]
fn python_v0_17_4_learning_noop_enum_values_parse_as_first_class_contracts() {
    assert_eq!(
        LearningTerminalResult::from_value("ANALYST_NOOP").unwrap(),
        LearningTerminalResult::AnalystNoop
    );
    assert_eq!(
        LearningTerminalResult::from_value("PROFESSOR_NOOP").unwrap(),
        LearningTerminalResult::ProfessorNoop
    );
    assert_eq!(
        LearningTerminalResult::from_value("CURATOR_NOOP").unwrap(),
        LearningTerminalResult::CuratorNoop
    );
    assert_eq!(
        LearningTerminalResult::from_value("LIBRARIAN_NOOP").unwrap(),
        LearningTerminalResult::LibrarianNoop
    );
    assert_eq!(ResultClass::from_value("no_op").unwrap(), ResultClass::NoOp);
}

#[test]
fn every_stage_has_metadata_and_plane() {
    assert_eq!(
        known_stage_values(),
        values(StageName::ALL, StageName::as_str)
    );

    for stage in StageName::ALL.iter().copied() {
        let metadata = stage_metadata(stage);

        assert_eq!(stage_metadata_for_value(stage.as_str()).unwrap(), metadata);
        assert_eq!(stage_name_for_value(stage.as_str()).unwrap(), stage);
        assert_eq!(stage_plane(stage), metadata.plane);
        assert_eq!(
            stage_name_for_plane(metadata.plane, stage.as_str()).unwrap(),
            stage
        );
    }

    assert_eq!(
        known_stage_values_for_plane(Plane::Execution),
        [
            "builder",
            "integrator",
            "checker",
            "fixer",
            "doublechecker",
            "updater",
            "troubleshooter",
            "consultant",
        ]
    );
    assert_eq!(
        known_stage_values_for_plane(Plane::Planning),
        [
            "recon",
            "planner",
            "manager",
            "manager_blueprint",
            "contractor_blueprint",
            "evaluator_blueprint",
            "mechanic",
            "mechanic_blueprint",
            "auditor",
            "arbiter",
        ]
    );
    assert_eq!(
        known_stage_values_for_plane(Plane::Learning),
        ["analyst", "professor", "curator", "librarian"]
    );
}

#[test]
fn legal_markers_and_running_markers_derive_from_metadata() {
    assert_eq!(running_status_marker(StageName::Builder), "BUILDER_RUNNING");
    assert_eq!(
        running_status_marker(StageName::Integrator),
        "INTEGRATOR_RUNNING"
    );
    assert_eq!(running_status_marker(StageName::Curator), "CURATOR_RUNNING");
    assert_eq!(
        running_status_marker(StageName::Librarian),
        "LIBRARIAN_RUNNING"
    );
    assert_eq!(
        legal_terminal_markers(StageName::Builder),
        vec!["### BUILDER_COMPLETE".to_owned(), "### BLOCKED".to_owned()]
    );
    assert_eq!(
        legal_terminal_markers(StageName::Integrator),
        vec![
            "### INTEGRATION_COMPLETE".to_owned(),
            "### BLOCKED".to_owned()
        ]
    );
    assert_eq!(
        legal_terminal_markers(StageName::Consultant),
        vec![
            "### CONSULT_COMPLETE".to_owned(),
            "### NEEDS_PLANNING".to_owned(),
            "### BLOCKED".to_owned()
        ]
    );
    assert_eq!(
        legal_terminal_markers(StageName::Arbiter),
        vec![
            "### ARBITER_COMPLETE".to_owned(),
            "### REMEDIATION_NEEDED".to_owned(),
            "### BLOCKED".to_owned()
        ]
    );
    assert_eq!(
        legal_terminal_markers(StageName::Recon),
        vec![
            "### RECON_TO_EXECUTION".to_owned(),
            "### RECON_TO_PLANNING".to_owned(),
            "### RECON_NOOP".to_owned(),
            "### RECON_BLOCKED".to_owned(),
            "### BLOCKED".to_owned()
        ]
    );
    assert_eq!(
        legal_terminal_markers(StageName::Librarian),
        vec![
            "### LIBRARIAN_COMPLETE".to_owned(),
            "### LIBRARIAN_NOOP".to_owned(),
            "### BLOCKED".to_owned()
        ]
    );

    for stage in StageName::ALL.iter().copied() {
        let metadata = stage_metadata(stage);
        let expected: Vec<String> = legal_terminal_results(stage)
            .iter()
            .map(|result| format!("### {}", result.as_str()))
            .collect();

        assert_eq!(metadata.legal_terminal_markers(), expected);
        assert_eq!(legal_terminal_markers(stage), expected);
    }
}

#[test]
fn terminal_result_lookup_is_plane_specific() {
    assert_eq!(
        terminal_result_for_plane(Plane::Execution, "BLOCKED").unwrap(),
        TerminalResult::Execution(ExecutionTerminalResult::Blocked)
    );
    assert_eq!(
        terminal_result_for_plane(Plane::Planning, "BLOCKED").unwrap(),
        TerminalResult::Planning(PlanningTerminalResult::Blocked)
    );
    assert_eq!(
        terminal_result_for_plane(Plane::Planning, "RECON_TO_EXECUTION").unwrap(),
        TerminalResult::Planning(PlanningTerminalResult::ReconToExecution)
    );
    assert_eq!(
        terminal_result_for_plane(Plane::Learning, "CURATOR_COMPLETE").unwrap(),
        TerminalResult::Learning(LearningTerminalResult::CuratorComplete)
    );
    assert_eq!(
        terminal_result_for_plane(Plane::Learning, "ANALYST_NOOP").unwrap(),
        TerminalResult::Learning(LearningTerminalResult::AnalystNoop)
    );
    assert_eq!(
        terminal_result_for_plane(Plane::Learning, "LIBRARIAN_COMPLETE").unwrap(),
        TerminalResult::Learning(LearningTerminalResult::LibrarianComplete)
    );
    assert_eq!(
        blocked_terminal_for_plane(Plane::Learning),
        TerminalResult::Learning(LearningTerminalResult::Blocked)
    );
}

#[test]
fn invalid_stage_terminal_marker_and_result_class_fail_with_typed_errors() {
    assert!(matches!(
        stage_name_for_value("fake_stage"),
        Err(ContractError::UnknownStageValue { .. })
    ));
    assert!(matches!(
        stage_name_for_plane(Plane::Planning, "builder"),
        Err(ContractError::StagePlaneMismatch { .. })
    ));
    assert!(matches!(
        parse_terminal_marker_for_plane(Plane::Execution, "BUILDER_COMPLETE"),
        Err(ContractError::InvalidTerminalMarker { .. })
    ));
    assert!(matches!(
        validate_terminal_marker_for_stage(StageName::Builder, "### CHECKER_PASS"),
        Err(ContractError::TerminalResultNotAllowed { .. })
    ));
    assert!(matches!(
        validate_stage_result_class(
            StageName::Builder,
            TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete),
            ResultClass::Blocked,
        ),
        Err(ContractError::ResultClassNotAllowed { .. })
    ));
    assert!(matches!(
        validate_stage_result_class(
            StageName::Builder,
            TerminalResult::Execution(ExecutionTerminalResult::CheckerPass),
            ResultClass::Success,
        ),
        Err(ContractError::TerminalResultNotAllowed { .. })
    ));
    assert!(matches!(
        validate_stage_result_class(
            StageName::Builder,
            TerminalResult::Planning(PlanningTerminalResult::Blocked),
            ResultClass::Blocked,
        ),
        Err(ContractError::TerminalResultNotAllowed { .. })
    ));
    assert!(matches!(
        validate_stage_result_class(
            StageName::Analyst,
            TerminalResult::Learning(LearningTerminalResult::AnalystNoop),
            ResultClass::Success,
        ),
        Err(ContractError::ResultClassNotAllowed { .. })
    ));
    assert!(matches!(
        validate_stage_result_class(
            StageName::Analyst,
            TerminalResult::Learning(LearningTerminalResult::AnalystComplete),
            ResultClass::NoOp,
        ),
        Err(ContractError::ResultClassNotAllowed { .. })
    ));
    assert!(matches!(
        validate_stage_result_class(
            StageName::Analyst,
            TerminalResult::Learning(LearningTerminalResult::ProfessorNoop),
            ResultClass::NoOp,
        ),
        Err(ContractError::TerminalResultNotAllowed { .. })
    ));
}

#[test]
fn legal_stage_result_class_combinations_validate() {
    validate_stage_result_class(
        StageName::Builder,
        TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete),
        ResultClass::Success,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Builder,
        TerminalResult::Execution(ExecutionTerminalResult::Blocked),
        ResultClass::RecoverableFailure,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Integrator,
        TerminalResult::Execution(ExecutionTerminalResult::IntegrationComplete),
        ResultClass::Success,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Integrator,
        TerminalResult::Execution(ExecutionTerminalResult::Blocked),
        ResultClass::RecoverableFailure,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Consultant,
        TerminalResult::Execution(ExecutionTerminalResult::NeedsPlanning),
        ResultClass::EscalatePlanning,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Arbiter,
        TerminalResult::Planning(PlanningTerminalResult::RemediationNeeded),
        ResultClass::FollowupNeeded,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Recon,
        TerminalResult::Planning(PlanningTerminalResult::ReconToExecution),
        ResultClass::Success,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Recon,
        TerminalResult::Planning(PlanningTerminalResult::ReconToPlanning),
        ResultClass::Success,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Recon,
        TerminalResult::Planning(PlanningTerminalResult::ReconNoop),
        ResultClass::NoOp,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Recon,
        TerminalResult::Planning(PlanningTerminalResult::ReconBlocked),
        ResultClass::Blocked,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Librarian,
        TerminalResult::Learning(LearningTerminalResult::LibrarianComplete),
        ResultClass::Success,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Librarian,
        TerminalResult::Learning(LearningTerminalResult::LibrarianNoop),
        ResultClass::NoOp,
    )
    .unwrap();
    validate_stage_result_class(
        StageName::Librarian,
        TerminalResult::Learning(LearningTerminalResult::Blocked),
        ResultClass::RecoverableFailure,
    )
    .unwrap();

    let builder_allowed = allowed_result_classes_by_outcome(StageName::Builder);
    assert_eq!(builder_allowed.len(), 2);
    assert_eq!(
        builder_allowed[0].result_classes,
        &[ResultClass::Success][..]
    );
}

#[test]
fn python_v0_17_4_learning_stage_metadata_allows_only_stage_specific_noop_classes() {
    let analyst_noop = TerminalResult::Learning(LearningTerminalResult::AnalystNoop);
    let professor_noop = TerminalResult::Learning(LearningTerminalResult::ProfessorNoop);
    let curator_noop = TerminalResult::Learning(LearningTerminalResult::CuratorNoop);
    let librarian_noop = TerminalResult::Learning(LearningTerminalResult::LibrarianNoop);

    assert_eq!(
        legal_terminal_markers(StageName::Analyst),
        vec![
            "### ANALYST_COMPLETE".to_owned(),
            "### ANALYST_NOOP".to_owned(),
            "### BLOCKED".to_owned()
        ]
    );
    assert_eq!(
        legal_terminal_markers(StageName::Professor),
        vec![
            "### PROFESSOR_COMPLETE".to_owned(),
            "### PROFESSOR_NOOP".to_owned(),
            "### BLOCKED".to_owned()
        ]
    );
    assert_eq!(
        legal_terminal_markers(StageName::Curator),
        vec![
            "### CURATOR_COMPLETE".to_owned(),
            "### CURATOR_NOOP".to_owned(),
            "### BLOCKED".to_owned()
        ]
    );
    assert_eq!(
        legal_terminal_markers(StageName::Librarian),
        vec![
            "### LIBRARIAN_COMPLETE".to_owned(),
            "### LIBRARIAN_NOOP".to_owned(),
            "### BLOCKED".to_owned()
        ]
    );

    validate_stage_result_class(StageName::Analyst, analyst_noop, ResultClass::NoOp).unwrap();
    validate_stage_result_class(StageName::Professor, professor_noop, ResultClass::NoOp).unwrap();
    validate_stage_result_class(StageName::Curator, curator_noop, ResultClass::NoOp).unwrap();
    validate_stage_result_class(StageName::Librarian, librarian_noop, ResultClass::NoOp).unwrap();

    assert_eq!(
        allowed_result_classes_by_outcome(StageName::Analyst)[1].result_classes,
        &[ResultClass::NoOp]
    );
    assert!(matches!(
        validate_stage_result_class(StageName::Analyst, professor_noop, ResultClass::NoOp),
        Err(ContractError::TerminalResultNotAllowed { .. })
    ));
    assert!(matches!(
        validate_stage_result_class(StageName::Curator, librarian_noop, ResultClass::NoOp),
        Err(ContractError::TerminalResultNotAllowed { .. })
    ));
    assert!(matches!(
        validate_stage_result_class(StageName::Curator, curator_noop, ResultClass::Blocked),
        Err(ContractError::ResultClassNotAllowed { .. })
    ));
}

#[test]
fn safe_identifier_validation_matches_python_reference_contract() {
    assert_eq!(
        validate_safe_identifier("task-001.alpha_beta", "task_id").unwrap(),
        "task-001.alpha_beta"
    );

    assert!(matches!(
        validate_safe_identifier(" task-001", "task_id"),
        Err(ContractError::UnsafeIdentifier {
            reason: IdentifierErrorReason::SurroundingWhitespace,
            ..
        })
    ));
    assert!(matches!(
        validate_safe_identifier("", "task_id"),
        Err(ContractError::UnsafeIdentifier {
            reason: IdentifierErrorReason::Empty,
            ..
        })
    ));
    assert!(matches!(
        validate_safe_identifier("-task-001", "task_id"),
        Err(ContractError::UnsafeIdentifier {
            reason: IdentifierErrorReason::InvalidCharacters,
            ..
        })
    ));
    assert!(matches!(
        validate_safe_identifier("task/001", "task_id"),
        Err(ContractError::UnsafeIdentifier {
            reason: IdentifierErrorReason::InvalidCharacters,
            ..
        })
    ));
}
