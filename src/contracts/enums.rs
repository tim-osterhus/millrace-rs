use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::ContractError;

macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident => $value:literal,
            )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $(
                $(#[$variant_meta])*
                $variant,
            )+
        }

        impl $name {
            /// Every canonical value for this enum.
            pub const ALL: &'static [Self] = &[
                $(Self::$variant,)+
            ];

            /// Returns the canonical string value used in Millrace artifacts.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value,)+
                }
            }

            /// Parses a canonical string value.
            pub fn from_value(value: &str) -> Result<Self, ContractError> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(ContractError::UnknownEnumValue {
                        enum_name: stringify!($name),
                        value: value.to_owned(),
                    }),
                }
            }
        }

        impl TryFrom<&str> for $name {
            type Error = ContractError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::from_value(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::from_value(&value).map_err(serde::de::Error::custom)
            }
        }
    };
}

string_enum! {
    /// Runtime plane names.
    pub enum Plane {
        Execution => "execution",
        Planning => "planning",
        Learning => "learning",
    }
}

string_enum! {
    /// Execution-plane stage names.
    pub enum ExecutionStageName {
        Builder => "builder",
        Integrator => "integrator",
        Checker => "checker",
        Fixer => "fixer",
        Doublechecker => "doublechecker",
        Updater => "updater",
        Troubleshooter => "troubleshooter",
        Consultant => "consultant",
    }
}

string_enum! {
    /// Planning-plane stage names.
    pub enum PlanningStageName {
        Recon => "recon",
        Planner => "planner",
        Manager => "manager",
        Mechanic => "mechanic",
        Auditor => "auditor",
        Arbiter => "arbiter",
    }
}

string_enum! {
    /// Learning-plane stage names.
    pub enum LearningStageName {
        Analyst => "analyst",
        Professor => "professor",
        Curator => "curator",
        Librarian => "librarian",
    }
}

string_enum! {
    /// All stage names across runtime planes.
    pub enum StageName {
        Builder => "builder",
        Integrator => "integrator",
        Checker => "checker",
        Fixer => "fixer",
        Doublechecker => "doublechecker",
        Updater => "updater",
        Troubleshooter => "troubleshooter",
        Consultant => "consultant",
        Recon => "recon",
        Planner => "planner",
        Manager => "manager",
        Mechanic => "mechanic",
        Auditor => "auditor",
        Arbiter => "arbiter",
        Analyst => "analyst",
        Professor => "professor",
        Curator => "curator",
        Librarian => "librarian",
    }
}

impl StageName {
    /// Returns the plane that owns this stage.
    #[must_use]
    pub const fn plane(self) -> Plane {
        match self {
            Self::Builder
            | Self::Integrator
            | Self::Checker
            | Self::Fixer
            | Self::Doublechecker
            | Self::Updater
            | Self::Troubleshooter
            | Self::Consultant => Plane::Execution,
            Self::Recon
            | Self::Planner
            | Self::Manager
            | Self::Mechanic
            | Self::Auditor
            | Self::Arbiter => Plane::Planning,
            Self::Analyst | Self::Professor | Self::Curator | Self::Librarian => Plane::Learning,
        }
    }
}

impl From<ExecutionStageName> for StageName {
    fn from(value: ExecutionStageName) -> Self {
        match value {
            ExecutionStageName::Builder => Self::Builder,
            ExecutionStageName::Integrator => Self::Integrator,
            ExecutionStageName::Checker => Self::Checker,
            ExecutionStageName::Fixer => Self::Fixer,
            ExecutionStageName::Doublechecker => Self::Doublechecker,
            ExecutionStageName::Updater => Self::Updater,
            ExecutionStageName::Troubleshooter => Self::Troubleshooter,
            ExecutionStageName::Consultant => Self::Consultant,
        }
    }
}

impl From<PlanningStageName> for StageName {
    fn from(value: PlanningStageName) -> Self {
        match value {
            PlanningStageName::Recon => Self::Recon,
            PlanningStageName::Planner => Self::Planner,
            PlanningStageName::Manager => Self::Manager,
            PlanningStageName::Mechanic => Self::Mechanic,
            PlanningStageName::Auditor => Self::Auditor,
            PlanningStageName::Arbiter => Self::Arbiter,
        }
    }
}

impl From<LearningStageName> for StageName {
    fn from(value: LearningStageName) -> Self {
        match value {
            LearningStageName::Analyst => Self::Analyst,
            LearningStageName::Professor => Self::Professor,
            LearningStageName::Curator => Self::Curator,
            LearningStageName::Librarian => Self::Librarian,
        }
    }
}

string_enum! {
    /// Execution-plane terminal results.
    pub enum ExecutionTerminalResult {
        BuilderComplete => "BUILDER_COMPLETE",
        IntegrationComplete => "INTEGRATION_COMPLETE",
        CheckerPass => "CHECKER_PASS",
        FixNeeded => "FIX_NEEDED",
        FixerComplete => "FIXER_COMPLETE",
        DoublecheckPass => "DOUBLECHECK_PASS",
        UpdateComplete => "UPDATE_COMPLETE",
        TroubleshootComplete => "TROUBLESHOOT_COMPLETE",
        ConsultComplete => "CONSULT_COMPLETE",
        NeedsPlanning => "NEEDS_PLANNING",
        Blocked => "BLOCKED",
    }
}

string_enum! {
    /// Planning-plane terminal results.
    pub enum PlanningTerminalResult {
        ReconToExecution => "RECON_TO_EXECUTION",
        ReconToPlanning => "RECON_TO_PLANNING",
        ReconBlocked => "RECON_BLOCKED",
        ReconNoop => "RECON_NOOP",
        PlannerComplete => "PLANNER_COMPLETE",
        ManagerComplete => "MANAGER_COMPLETE",
        MechanicComplete => "MECHANIC_COMPLETE",
        AuditorComplete => "AUDITOR_COMPLETE",
        ArbiterComplete => "ARBITER_COMPLETE",
        RemediationNeeded => "REMEDIATION_NEEDED",
        Blocked => "BLOCKED",
    }
}

string_enum! {
    /// Learning-plane terminal results.
    pub enum LearningTerminalResult {
        AnalystComplete => "ANALYST_COMPLETE",
        AnalystNoop => "ANALYST_NOOP",
        ProfessorComplete => "PROFESSOR_COMPLETE",
        ProfessorNoop => "PROFESSOR_NOOP",
        CuratorComplete => "CURATOR_COMPLETE",
        CuratorNoop => "CURATOR_NOOP",
        LibrarianComplete => "LIBRARIAN_COMPLETE",
        LibrarianNoop => "LIBRARIAN_NOOP",
        Blocked => "BLOCKED",
    }
}

/// Plane-qualified terminal result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalResult {
    /// Execution-plane terminal result.
    Execution(ExecutionTerminalResult),
    /// Planning-plane terminal result.
    Planning(PlanningTerminalResult),
    /// Learning-plane terminal result.
    Learning(LearningTerminalResult),
}

impl TerminalResult {
    /// Returns the plane that owns this terminal result.
    #[must_use]
    pub const fn plane(self) -> Plane {
        match self {
            Self::Execution(_) => Plane::Execution,
            Self::Planning(_) => Plane::Planning,
            Self::Learning(_) => Plane::Learning,
        }
    }

    /// Returns the canonical terminal result token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Execution(result) => result.as_str(),
            Self::Planning(result) => result.as_str(),
            Self::Learning(result) => result.as_str(),
        }
    }

    /// Returns the canonical terminal marker for this result.
    #[must_use]
    pub fn marker(self) -> String {
        format!("### {}", self.as_str())
    }
}

impl From<ExecutionTerminalResult> for TerminalResult {
    fn from(value: ExecutionTerminalResult) -> Self {
        Self::Execution(value)
    }
}

impl From<PlanningTerminalResult> for TerminalResult {
    fn from(value: PlanningTerminalResult) -> Self {
        Self::Planning(value)
    }
}

impl From<LearningTerminalResult> for TerminalResult {
    fn from(value: LearningTerminalResult) -> Self {
        Self::Learning(value)
    }
}

impl fmt::Display for TerminalResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for TerminalResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TerminalResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if let Ok(result) = ExecutionTerminalResult::from_value(&value) {
            return Ok(Self::Execution(result));
        }
        if let Ok(result) = PlanningTerminalResult::from_value(&value) {
            return Ok(Self::Planning(result));
        }
        LearningTerminalResult::from_value(&value)
            .map(Self::Learning)
            .map_err(serde::de::Error::custom)
    }
}

string_enum! {
    /// Stage result classes.
    pub enum ResultClass {
        Success => "success",
        NoOp => "no_op",
        FollowupNeeded => "followup_needed",
        RecoverableFailure => "recoverable_failure",
        EscalatePlanning => "escalate_planning",
        Blocked => "blocked",
    }
}

string_enum! {
    /// Work item kinds accepted by runtime queues.
    pub enum WorkItemKind {
        Task => "task",
        Probe => "probe",
        Spec => "spec",
        Incident => "incident",
        LearningRequest => "learning_request",
    }
}

string_enum! {
    /// Source categories for spec work documents.
    pub enum SpecSourceType {
        Idea => "idea",
        Incident => "incident",
        Manual => "manual",
        DerivedSpec => "derived_spec",
        Probe => "probe",
    }
}

string_enum! {
    /// Supported learning request actions.
    pub enum LearningRequestAction {
        Create => "create",
        Improve => "improve",
        Promote => "promote",
        Export => "export",
        Install => "install",
    }
}

string_enum! {
    /// Task status hints used in human-facing task documents.
    pub enum TaskStatusHint {
        Queued => "queued",
        Active => "active",
        Blocked => "blocked",
        Done => "done",
    }
}

string_enum! {
    /// Probe status hints used in human-facing probe documents.
    pub enum ProbeStatusHint {
        Queued => "queued",
        Active => "active",
        Blocked => "blocked",
        Done => "done",
    }
}

string_enum! {
    /// Root intake sources tracked separately from root idea/spec lineage.
    pub enum RootIntakeKind {
        Idea => "idea",
        Probe => "probe",
        Manual => "manual",
        Incident => "incident",
        DerivedSpec => "derived_spec",
    }
}

string_enum! {
    /// Incident severities.
    pub enum IncidentSeverity {
        Low => "low",
        Medium => "medium",
        High => "high",
        Critical => "critical",
    }
}

string_enum! {
    /// Incident consultant decisions.
    pub enum IncidentDecision {
        NeedsPlanning => "needs_planning",
        Blocked => "blocked",
    }
}

string_enum! {
    /// Runtime invocation modes.
    pub enum RuntimeMode {
        Once => "once",
        Daemon => "daemon",
    }
}

string_enum! {
    /// Workspace watcher modes.
    pub enum WatcherMode {
        Watch => "watch",
        Poll => "poll",
        Off => "off",
    }
}

string_enum! {
    /// Config reload outcomes.
    pub enum ReloadOutcome {
        Applied => "applied",
        FailedRetainedPreviousPlan => "failed_retained_previous_plan",
    }
}

string_enum! {
    /// Runtime error codes persisted by the reference runtime.
    pub enum RuntimeErrorCode {
        PlanningWorkItemCompletionConflict => "planning_work_item_completion_conflict",
        ExecutionWorkItemCompletionConflict => "execution_work_item_completion_conflict",
        PlanningPostStageApplyFailed => "planning_post_stage_apply_failed",
        ExecutionPostStageApplyFailed => "execution_post_stage_apply_failed",
        ReconHandoffInvalid => "recon_handoff_invalid",
        StageWorkItemOwnershipInvalid => "stage_work_item_ownership_invalid",
    }
}

string_enum! {
    /// Mailbox commands accepted by the runtime.
    pub enum MailboxCommand {
        Stop => "stop",
        Pause => "pause",
        Resume => "resume",
        ReloadConfig => "reload_config",
        AddTask => "add_task",
        AddProbe => "add_probe",
        AddSpec => "add_spec",
        AddIdea => "add_idea",
        RetryActive => "retry_active",
        ClearStaleState => "clear_stale_state",
        CancelWorkItem => "cancel_work_item",
        ArchiveBlockedTask => "archive_blocked_task",
        SupersedeTask => "supersede_task",
        RetargetTaskDependency => "retarget_task_dependency",
        ResolveIncident => "resolve_incident",
        CancelIncident => "cancel_incident",
        ArchiveInvalidIncident => "archive_invalid_incident",
    }
}

string_enum! {
    /// Compiled loop edge kinds.
    pub enum LoopEdgeKind {
        Normal => "normal",
        Retry => "retry",
        Escalation => "escalation",
        Handoff => "handoff",
        Terminal => "terminal",
    }
}
