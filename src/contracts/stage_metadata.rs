use super::{
    ContractError, ExecutionTerminalResult, IdentifierErrorReason, LearningTerminalResult, Plane,
    PlanningTerminalResult, ResultClass, StageName, TerminalResult,
};

/// Human-readable description of the safe identifier pattern used by Python.
pub const SAFE_ID_PATTERN_DESCRIPTION: &str = "^[A-Za-z0-9][A-Za-z0-9._-]*$";

const SUCCESS_CLASSES: &[ResultClass] = &[ResultClass::Success];
const FOLLOWUP_CLASSES: &[ResultClass] = &[ResultClass::FollowupNeeded];
const ESCALATE_PLANNING_CLASSES: &[ResultClass] = &[ResultClass::EscalatePlanning];
const BLOCKED_CLASSES: &[ResultClass] = &[ResultClass::Blocked, ResultClass::RecoverableFailure];

const E_BUILDER_COMPLETE: TerminalResult =
    TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete);
const E_CHECKER_PASS: TerminalResult =
    TerminalResult::Execution(ExecutionTerminalResult::CheckerPass);
const E_FIX_NEEDED: TerminalResult = TerminalResult::Execution(ExecutionTerminalResult::FixNeeded);
const E_FIXER_COMPLETE: TerminalResult =
    TerminalResult::Execution(ExecutionTerminalResult::FixerComplete);
const E_DOUBLECHECK_PASS: TerminalResult =
    TerminalResult::Execution(ExecutionTerminalResult::DoublecheckPass);
const E_UPDATE_COMPLETE: TerminalResult =
    TerminalResult::Execution(ExecutionTerminalResult::UpdateComplete);
const E_TROUBLESHOOT_COMPLETE: TerminalResult =
    TerminalResult::Execution(ExecutionTerminalResult::TroubleshootComplete);
const E_CONSULT_COMPLETE: TerminalResult =
    TerminalResult::Execution(ExecutionTerminalResult::ConsultComplete);
const E_NEEDS_PLANNING: TerminalResult =
    TerminalResult::Execution(ExecutionTerminalResult::NeedsPlanning);
const E_BLOCKED: TerminalResult = TerminalResult::Execution(ExecutionTerminalResult::Blocked);

const P_PLANNER_COMPLETE: TerminalResult =
    TerminalResult::Planning(PlanningTerminalResult::PlannerComplete);
const P_MANAGER_COMPLETE: TerminalResult =
    TerminalResult::Planning(PlanningTerminalResult::ManagerComplete);
const P_MECHANIC_COMPLETE: TerminalResult =
    TerminalResult::Planning(PlanningTerminalResult::MechanicComplete);
const P_AUDITOR_COMPLETE: TerminalResult =
    TerminalResult::Planning(PlanningTerminalResult::AuditorComplete);
const P_ARBITER_COMPLETE: TerminalResult =
    TerminalResult::Planning(PlanningTerminalResult::ArbiterComplete);
const P_REMEDIATION_NEEDED: TerminalResult =
    TerminalResult::Planning(PlanningTerminalResult::RemediationNeeded);
const P_BLOCKED: TerminalResult = TerminalResult::Planning(PlanningTerminalResult::Blocked);

const L_ANALYST_COMPLETE: TerminalResult =
    TerminalResult::Learning(LearningTerminalResult::AnalystComplete);
const L_PROFESSOR_COMPLETE: TerminalResult =
    TerminalResult::Learning(LearningTerminalResult::ProfessorComplete);
const L_CURATOR_COMPLETE: TerminalResult =
    TerminalResult::Learning(LearningTerminalResult::CuratorComplete);
const L_BLOCKED: TerminalResult = TerminalResult::Learning(LearningTerminalResult::Blocked);

/// Legal result classes for one terminal outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutcomeResultClasses {
    /// Terminal result controlled by the stage metadata.
    pub terminal_result: TerminalResult,
    /// Result classes permitted for this terminal result.
    pub result_classes: &'static [ResultClass],
}

/// Ownership and terminal-result metadata for a stage kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageMetadata {
    /// Canonical stage name.
    pub stage: StageName,
    /// Plane that owns the stage.
    pub plane: Plane,
    /// Legal terminal results for this stage.
    pub legal_terminal_results: &'static [TerminalResult],
    /// Legal result classes keyed by terminal outcome.
    pub allowed_result_classes_by_outcome: &'static [OutcomeResultClasses],
}

impl StageMetadata {
    /// Returns the running status marker for this stage.
    #[must_use]
    pub fn running_status_marker(self) -> &'static str {
        running_status_marker(self.stage)
    }

    /// Returns the legal terminal markers for this stage.
    #[must_use]
    pub fn legal_terminal_markers(self) -> Vec<String> {
        legal_terminal_markers(self.stage)
    }

    /// Returns true when the terminal result is legal for this stage.
    #[must_use]
    pub fn allows_terminal_result(self, terminal_result: TerminalResult) -> bool {
        self.legal_terminal_results.contains(&terminal_result)
    }

    /// Returns true when the terminal result and result class are legal together.
    #[must_use]
    pub fn allows_result_class(
        self,
        terminal_result: TerminalResult,
        result_class: ResultClass,
    ) -> bool {
        self.allowed_result_classes_by_outcome
            .iter()
            .find(|entry| entry.terminal_result == terminal_result)
            .is_some_and(|entry| entry.result_classes.contains(&result_class))
    }
}

const BUILDER_LEGAL: &[TerminalResult] = &[E_BUILDER_COMPLETE, E_BLOCKED];
const BUILDER_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: E_BUILDER_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: E_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const CHECKER_LEGAL: &[TerminalResult] = &[E_CHECKER_PASS, E_FIX_NEEDED, E_BLOCKED];
const CHECKER_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: E_CHECKER_PASS,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: E_FIX_NEEDED,
        result_classes: FOLLOWUP_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: E_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const FIXER_LEGAL: &[TerminalResult] = &[E_FIXER_COMPLETE, E_BLOCKED];
const FIXER_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: E_FIXER_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: E_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const DOUBLECHECKER_LEGAL: &[TerminalResult] = &[E_DOUBLECHECK_PASS, E_FIX_NEEDED, E_BLOCKED];
const DOUBLECHECKER_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: E_DOUBLECHECK_PASS,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: E_FIX_NEEDED,
        result_classes: FOLLOWUP_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: E_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const UPDATER_LEGAL: &[TerminalResult] = &[E_UPDATE_COMPLETE, E_BLOCKED];
const UPDATER_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: E_UPDATE_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: E_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const TROUBLESHOOTER_LEGAL: &[TerminalResult] = &[E_TROUBLESHOOT_COMPLETE, E_BLOCKED];
const TROUBLESHOOTER_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: E_TROUBLESHOOT_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: E_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const CONSULTANT_LEGAL: &[TerminalResult] = &[E_CONSULT_COMPLETE, E_NEEDS_PLANNING, E_BLOCKED];
const CONSULTANT_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: E_CONSULT_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: E_NEEDS_PLANNING,
        result_classes: ESCALATE_PLANNING_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: E_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const PLANNER_LEGAL: &[TerminalResult] = &[P_PLANNER_COMPLETE, P_BLOCKED];
const PLANNER_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: P_PLANNER_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: P_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const MANAGER_LEGAL: &[TerminalResult] = &[P_MANAGER_COMPLETE, P_BLOCKED];
const MANAGER_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: P_MANAGER_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: P_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const MECHANIC_LEGAL: &[TerminalResult] = &[P_MECHANIC_COMPLETE, P_BLOCKED];
const MECHANIC_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: P_MECHANIC_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: P_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const AUDITOR_LEGAL: &[TerminalResult] = &[P_AUDITOR_COMPLETE, P_BLOCKED];
const AUDITOR_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: P_AUDITOR_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: P_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const ARBITER_LEGAL: &[TerminalResult] = &[P_ARBITER_COMPLETE, P_REMEDIATION_NEEDED, P_BLOCKED];
const ARBITER_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: P_ARBITER_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: P_REMEDIATION_NEEDED,
        result_classes: FOLLOWUP_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: P_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const ANALYST_LEGAL: &[TerminalResult] = &[L_ANALYST_COMPLETE, L_BLOCKED];
const ANALYST_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: L_ANALYST_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: L_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const PROFESSOR_LEGAL: &[TerminalResult] = &[L_PROFESSOR_COMPLETE, L_BLOCKED];
const PROFESSOR_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: L_PROFESSOR_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: L_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const CURATOR_LEGAL: &[TerminalResult] = &[L_CURATOR_COMPLETE, L_BLOCKED];
const CURATOR_ALLOWED: &[OutcomeResultClasses] = &[
    OutcomeResultClasses {
        terminal_result: L_CURATOR_COMPLETE,
        result_classes: SUCCESS_CLASSES,
    },
    OutcomeResultClasses {
        terminal_result: L_BLOCKED,
        result_classes: BLOCKED_CLASSES,
    },
];

const BUILDER_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Builder,
    plane: Plane::Execution,
    legal_terminal_results: BUILDER_LEGAL,
    allowed_result_classes_by_outcome: BUILDER_ALLOWED,
};
const CHECKER_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Checker,
    plane: Plane::Execution,
    legal_terminal_results: CHECKER_LEGAL,
    allowed_result_classes_by_outcome: CHECKER_ALLOWED,
};
const FIXER_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Fixer,
    plane: Plane::Execution,
    legal_terminal_results: FIXER_LEGAL,
    allowed_result_classes_by_outcome: FIXER_ALLOWED,
};
const DOUBLECHECKER_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Doublechecker,
    plane: Plane::Execution,
    legal_terminal_results: DOUBLECHECKER_LEGAL,
    allowed_result_classes_by_outcome: DOUBLECHECKER_ALLOWED,
};
const UPDATER_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Updater,
    plane: Plane::Execution,
    legal_terminal_results: UPDATER_LEGAL,
    allowed_result_classes_by_outcome: UPDATER_ALLOWED,
};
const TROUBLESHOOTER_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Troubleshooter,
    plane: Plane::Execution,
    legal_terminal_results: TROUBLESHOOTER_LEGAL,
    allowed_result_classes_by_outcome: TROUBLESHOOTER_ALLOWED,
};
const CONSULTANT_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Consultant,
    plane: Plane::Execution,
    legal_terminal_results: CONSULTANT_LEGAL,
    allowed_result_classes_by_outcome: CONSULTANT_ALLOWED,
};
const PLANNER_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Planner,
    plane: Plane::Planning,
    legal_terminal_results: PLANNER_LEGAL,
    allowed_result_classes_by_outcome: PLANNER_ALLOWED,
};
const MANAGER_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Manager,
    plane: Plane::Planning,
    legal_terminal_results: MANAGER_LEGAL,
    allowed_result_classes_by_outcome: MANAGER_ALLOWED,
};
const MECHANIC_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Mechanic,
    plane: Plane::Planning,
    legal_terminal_results: MECHANIC_LEGAL,
    allowed_result_classes_by_outcome: MECHANIC_ALLOWED,
};
const AUDITOR_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Auditor,
    plane: Plane::Planning,
    legal_terminal_results: AUDITOR_LEGAL,
    allowed_result_classes_by_outcome: AUDITOR_ALLOWED,
};
const ARBITER_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Arbiter,
    plane: Plane::Planning,
    legal_terminal_results: ARBITER_LEGAL,
    allowed_result_classes_by_outcome: ARBITER_ALLOWED,
};
const ANALYST_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Analyst,
    plane: Plane::Learning,
    legal_terminal_results: ANALYST_LEGAL,
    allowed_result_classes_by_outcome: ANALYST_ALLOWED,
};
const PROFESSOR_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Professor,
    plane: Plane::Learning,
    legal_terminal_results: PROFESSOR_LEGAL,
    allowed_result_classes_by_outcome: PROFESSOR_ALLOWED,
};
const CURATOR_METADATA: StageMetadata = StageMetadata {
    stage: StageName::Curator,
    plane: Plane::Learning,
    legal_terminal_results: CURATOR_LEGAL,
    allowed_result_classes_by_outcome: CURATOR_ALLOWED,
};

/// Stage metadata entries keyed by each stage value.
pub const STAGE_METADATA_BY_VALUE: &[StageMetadata] = &[
    BUILDER_METADATA,
    CHECKER_METADATA,
    FIXER_METADATA,
    DOUBLECHECKER_METADATA,
    UPDATER_METADATA,
    TROUBLESHOOTER_METADATA,
    CONSULTANT_METADATA,
    PLANNER_METADATA,
    MANAGER_METADATA,
    MECHANIC_METADATA,
    AUDITOR_METADATA,
    ARBITER_METADATA,
    ANALYST_METADATA,
    PROFESSOR_METADATA,
    CURATOR_METADATA,
];

/// Canonical stage names in metadata order.
pub const STAGE_NAME_BY_VALUE: &[StageName] = &[
    StageName::Builder,
    StageName::Checker,
    StageName::Fixer,
    StageName::Doublechecker,
    StageName::Updater,
    StageName::Troubleshooter,
    StageName::Consultant,
    StageName::Planner,
    StageName::Manager,
    StageName::Mechanic,
    StageName::Auditor,
    StageName::Arbiter,
    StageName::Analyst,
    StageName::Professor,
    StageName::Curator,
];

/// Stage-to-plane relationships in metadata order.
pub const STAGE_TO_PLANE: &[(StageName, Plane)] = &[
    (StageName::Builder, Plane::Execution),
    (StageName::Checker, Plane::Execution),
    (StageName::Fixer, Plane::Execution),
    (StageName::Doublechecker, Plane::Execution),
    (StageName::Updater, Plane::Execution),
    (StageName::Troubleshooter, Plane::Execution),
    (StageName::Consultant, Plane::Execution),
    (StageName::Planner, Plane::Planning),
    (StageName::Manager, Plane::Planning),
    (StageName::Mechanic, Plane::Planning),
    (StageName::Auditor, Plane::Planning),
    (StageName::Arbiter, Plane::Planning),
    (StageName::Analyst, Plane::Learning),
    (StageName::Professor, Plane::Learning),
    (StageName::Curator, Plane::Learning),
];

/// Legal terminal results for every stage in metadata order.
pub const STAGE_LEGAL_TERMINAL_RESULTS: &[(StageName, &'static [TerminalResult])] = &[
    (StageName::Builder, BUILDER_LEGAL),
    (StageName::Checker, CHECKER_LEGAL),
    (StageName::Fixer, FIXER_LEGAL),
    (StageName::Doublechecker, DOUBLECHECKER_LEGAL),
    (StageName::Updater, UPDATER_LEGAL),
    (StageName::Troubleshooter, TROUBLESHOOTER_LEGAL),
    (StageName::Consultant, CONSULTANT_LEGAL),
    (StageName::Planner, PLANNER_LEGAL),
    (StageName::Manager, MANAGER_LEGAL),
    (StageName::Mechanic, MECHANIC_LEGAL),
    (StageName::Auditor, AUDITOR_LEGAL),
    (StageName::Arbiter, ARBITER_LEGAL),
    (StageName::Analyst, ANALYST_LEGAL),
    (StageName::Professor, PROFESSOR_LEGAL),
    (StageName::Curator, CURATOR_LEGAL),
];

/// Returns stage metadata for a known stage.
#[must_use]
pub fn stage_metadata(stage: StageName) -> &'static StageMetadata {
    match stage {
        StageName::Builder => &BUILDER_METADATA,
        StageName::Checker => &CHECKER_METADATA,
        StageName::Fixer => &FIXER_METADATA,
        StageName::Doublechecker => &DOUBLECHECKER_METADATA,
        StageName::Updater => &UPDATER_METADATA,
        StageName::Troubleshooter => &TROUBLESHOOTER_METADATA,
        StageName::Consultant => &CONSULTANT_METADATA,
        StageName::Planner => &PLANNER_METADATA,
        StageName::Manager => &MANAGER_METADATA,
        StageName::Mechanic => &MECHANIC_METADATA,
        StageName::Auditor => &AUDITOR_METADATA,
        StageName::Arbiter => &ARBITER_METADATA,
        StageName::Analyst => &ANALYST_METADATA,
        StageName::Professor => &PROFESSOR_METADATA,
        StageName::Curator => &CURATOR_METADATA,
    }
}

/// Looks up stage metadata from a raw stage value.
pub fn stage_metadata_for_value(
    stage_value: &str,
) -> Result<&'static StageMetadata, ContractError> {
    Ok(stage_metadata(stage_name_for_value(stage_value)?))
}

/// Returns the plane that owns a stage.
#[must_use]
pub const fn stage_plane(stage: StageName) -> Plane {
    stage.plane()
}

/// Returns the legal terminal results for a stage.
#[must_use]
pub fn legal_terminal_results(stage: StageName) -> &'static [TerminalResult] {
    stage_metadata(stage).legal_terminal_results
}

/// Returns the legal terminal markers for a stage.
#[must_use]
pub fn legal_terminal_markers(stage: StageName) -> Vec<String> {
    legal_terminal_results(stage)
        .iter()
        .map(|result| result.marker())
        .collect()
}

/// Returns the running status marker for a stage.
#[must_use]
pub const fn running_status_marker(stage: StageName) -> &'static str {
    match stage {
        StageName::Builder => "BUILDER_RUNNING",
        StageName::Checker => "CHECKER_RUNNING",
        StageName::Fixer => "FIXER_RUNNING",
        StageName::Doublechecker => "DOUBLECHECKER_RUNNING",
        StageName::Updater => "UPDATER_RUNNING",
        StageName::Troubleshooter => "TROUBLESHOOTER_RUNNING",
        StageName::Consultant => "CONSULTANT_RUNNING",
        StageName::Planner => "PLANNER_RUNNING",
        StageName::Manager => "MANAGER_RUNNING",
        StageName::Mechanic => "MECHANIC_RUNNING",
        StageName::Auditor => "AUDITOR_RUNNING",
        StageName::Arbiter => "ARBITER_RUNNING",
        StageName::Analyst => "ANALYST_RUNNING",
        StageName::Professor => "PROFESSOR_RUNNING",
        StageName::Curator => "CURATOR_RUNNING",
    }
}

/// Returns allowed result classes grouped by terminal outcome for a stage.
#[must_use]
pub fn allowed_result_classes_by_outcome(stage: StageName) -> &'static [OutcomeResultClasses] {
    stage_metadata(stage).allowed_result_classes_by_outcome
}

/// Parses a raw stage value.
pub fn stage_name_for_value(stage_value: &str) -> Result<StageName, ContractError> {
    match stage_value {
        "builder" => Ok(StageName::Builder),
        "checker" => Ok(StageName::Checker),
        "fixer" => Ok(StageName::Fixer),
        "doublechecker" => Ok(StageName::Doublechecker),
        "updater" => Ok(StageName::Updater),
        "troubleshooter" => Ok(StageName::Troubleshooter),
        "consultant" => Ok(StageName::Consultant),
        "planner" => Ok(StageName::Planner),
        "manager" => Ok(StageName::Manager),
        "mechanic" => Ok(StageName::Mechanic),
        "auditor" => Ok(StageName::Auditor),
        "arbiter" => Ok(StageName::Arbiter),
        "analyst" => Ok(StageName::Analyst),
        "professor" => Ok(StageName::Professor),
        "curator" => Ok(StageName::Curator),
        _ => Err(ContractError::UnknownStageValue {
            value: stage_value.to_owned(),
        }),
    }
}

/// Parses a raw stage value and validates that it belongs to the requested plane.
pub fn stage_name_for_plane(plane: Plane, stage_value: &str) -> Result<StageName, ContractError> {
    let stage = stage_name_for_value(stage_value)?;
    if stage.plane() == plane {
        Ok(stage)
    } else {
        Err(ContractError::StagePlaneMismatch { plane, stage })
    }
}

/// Returns all known stage values.
#[must_use]
pub fn known_stage_values() -> Vec<&'static str> {
    STAGE_METADATA_BY_VALUE
        .iter()
        .map(|metadata| metadata.stage.as_str())
        .collect()
}

/// Returns all known stage values for a plane.
#[must_use]
pub fn known_stage_values_for_plane(plane: Plane) -> Vec<&'static str> {
    STAGE_METADATA_BY_VALUE
        .iter()
        .filter(|metadata| metadata.plane == plane)
        .map(|metadata| metadata.stage.as_str())
        .collect()
}

/// Parses a terminal result token for a specific plane.
pub fn terminal_result_for_plane(
    plane: Plane,
    token: &str,
) -> Result<TerminalResult, ContractError> {
    match plane {
        Plane::Execution => ExecutionTerminalResult::from_value(token)
            .map(TerminalResult::Execution)
            .map_err(|_| ContractError::UnknownTerminalResult {
                plane,
                value: token.to_owned(),
            }),
        Plane::Planning => PlanningTerminalResult::from_value(token)
            .map(TerminalResult::Planning)
            .map_err(|_| ContractError::UnknownTerminalResult {
                plane,
                value: token.to_owned(),
            }),
        Plane::Learning => LearningTerminalResult::from_value(token)
            .map(TerminalResult::Learning)
            .map_err(|_| ContractError::UnknownTerminalResult {
                plane,
                value: token.to_owned(),
            }),
    }
}

/// Returns the plane-specific blocked terminal result.
#[must_use]
pub const fn blocked_terminal_for_plane(plane: Plane) -> TerminalResult {
    match plane {
        Plane::Execution => E_BLOCKED,
        Plane::Planning => P_BLOCKED,
        Plane::Learning => L_BLOCKED,
    }
}

/// Parses a `### OUTCOME` terminal marker for a specific plane.
pub fn parse_terminal_marker_for_plane(
    plane: Plane,
    marker: &str,
) -> Result<TerminalResult, ContractError> {
    let Some(token) = marker.strip_prefix("### ") else {
        return Err(ContractError::InvalidTerminalMarker {
            marker: marker.to_owned(),
        });
    };

    if token.is_empty() || token.trim() != token || token.contains(' ') {
        return Err(ContractError::InvalidTerminalMarker {
            marker: marker.to_owned(),
        });
    }

    terminal_result_for_plane(plane, token)
}

/// Parses a terminal marker and validates that it is legal for the stage.
pub fn validate_terminal_marker_for_stage(
    stage: StageName,
    marker: &str,
) -> Result<TerminalResult, ContractError> {
    let terminal_result = parse_terminal_marker_for_plane(stage.plane(), marker)?;
    if legal_terminal_results(stage).contains(&terminal_result) {
        Ok(terminal_result)
    } else {
        Err(ContractError::TerminalResultNotAllowed {
            stage,
            terminal_result,
        })
    }
}

/// Validates a stage terminal result/result-class pair.
pub fn validate_stage_result_class(
    stage: StageName,
    terminal_result: TerminalResult,
    result_class: ResultClass,
) -> Result<(), ContractError> {
    if terminal_result.plane() != stage.plane()
        || !legal_terminal_results(stage).contains(&terminal_result)
    {
        return Err(ContractError::TerminalResultNotAllowed {
            stage,
            terminal_result,
        });
    }

    let allowed_classes = allowed_result_classes_by_outcome(stage)
        .iter()
        .find(|entry| entry.terminal_result == terminal_result)
        .map(|entry| entry.result_classes)
        .unwrap_or(&[]);

    if allowed_classes.contains(&result_class) {
        Ok(())
    } else {
        Err(ContractError::ResultClassNotAllowed {
            stage,
            terminal_result,
            result_class,
        })
    }
}

/// Validates the safe identifier contract used by runtime artifact ids.
pub fn validate_safe_identifier<'a>(
    value: &'a str,
    field_name: &str,
) -> Result<&'a str, ContractError> {
    if value.trim() != value {
        return Err(ContractError::UnsafeIdentifier {
            field_name: field_name.to_owned(),
            value: value.to_owned(),
            reason: IdentifierErrorReason::SurroundingWhitespace,
        });
    }

    if value.is_empty() {
        return Err(ContractError::UnsafeIdentifier {
            field_name: field_name.to_owned(),
            value: value.to_owned(),
            reason: IdentifierErrorReason::Empty,
        });
    }

    let mut chars = value.chars();
    let first = chars.next().expect("value is not empty");
    if !first.is_ascii_alphanumeric()
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(ContractError::UnsafeIdentifier {
            field_name: field_name.to_owned(),
            value: value.to_owned(),
            reason: IdentifierErrorReason::InvalidCharacters,
        });
    }

    Ok(value)
}
