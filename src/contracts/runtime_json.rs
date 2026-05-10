//! Serde-backed JSON contracts for persisted runtime artifacts.

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::{Map, Value};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{
    ContractError, MailboxCommand, Plane, ProbeDocument, ReloadOutcome, ResultClass,
    RuntimeErrorCode, RuntimeMode, SpecDocument, StageName, TaskDocument, TerminalResult,
    Timestamp, WatcherMode, WorkItemKind, parse_terminal_marker_for_plane, stage_plane,
    terminal_result_for_plane, validate_safe_identifier,
};

const SCHEMA_VERSION: &str = "1.0";

macro_rules! runtime_string_enum {
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
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

            /// Returns the canonical serialized value.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value,)+
                }
            }

            /// Parses a canonical serialized value.
            pub fn from_value(value: &str) -> Result<Self, RuntimeJsonError> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(RuntimeJsonError::UnknownValue {
                        field_name: stringify!($name),
                        value: value.to_owned(),
                    }),
                }
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

runtime_string_enum! {
    /// Active-run request categories persisted in runtime snapshots.
    pub enum ActiveRunRequestKind {
        ActiveWorkItem => "active_work_item",
        ClosureTarget => "closure_target",
        LearningRequest => "learning_request",
    }
}

runtime_string_enum! {
    /// Pause sources persisted in runtime snapshots.
    pub enum PauseSource {
        Operator => "operator",
        UsageGovernance => "usage_governance",
    }
}

runtime_string_enum! {
    /// Usage-governance evaluation boundaries.
    pub enum UsageGovernanceEvaluationBoundary {
        BetweenStages => "between_stages",
    }
}

runtime_string_enum! {
    /// Usage-governance blocker sources.
    pub enum UsageGovernanceBlockerSource {
        RuntimeToken => "runtime_token",
        SubscriptionQuota => "subscription_quota",
    }
}

runtime_string_enum! {
    /// Runtime token-accounting windows.
    pub enum UsageGovernanceRuntimeTokenWindow {
        Rolling5h => "rolling_5h",
        CalendarWeek => "calendar_week",
        DaemonSession => "daemon_session",
        PerRun => "per_run",
    }
}

runtime_string_enum! {
    /// Runtime token-accounting metrics.
    pub enum UsageGovernanceRuntimeTokenMetric {
        TotalTokens => "total_tokens",
    }
}

runtime_string_enum! {
    /// Subscription-quota windows reported by Codex quota telemetry.
    pub enum UsageGovernanceSubscriptionWindow {
        FiveHour => "five_hour",
        Weekly => "weekly",
    }
}

runtime_string_enum! {
    /// Subscription-quota provider identifiers.
    pub enum UsageGovernanceSubscriptionProvider {
        CodexChatGptOauth => "codex_chatgpt_oauth",
    }
}

runtime_string_enum! {
    /// Policy used when subscription quota telemetry is degraded.
    pub enum UsageGovernanceDegradedPolicy {
        FailOpen => "fail_open",
        FailClosed => "fail_closed",
    }
}

runtime_string_enum! {
    /// Subscription quota telemetry state.
    pub enum SubscriptionQuotaTelemetryState {
        Disabled => "disabled",
        Healthy => "healthy",
        Degraded => "degraded",
    }
}

/// Typed failures produced by runtime JSON contract validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeJsonError {
    /// A lower-level shared contract validation failed.
    Contract(ContractError),
    /// JSON syntax, required-field, type, or enum decoding failed.
    Json {
        /// Artifact type being decoded.
        artifact: &'static str,
        /// Serde error message.
        message: String,
    },
    /// A string value did not match a runtime-local enum.
    UnknownValue {
        /// Field or enum name.
        field_name: &'static str,
        /// Invalid value.
        value: String,
    },
    /// A literal schema or kind field had the wrong value.
    InvalidLiteral {
        /// Field name.
        field_name: &'static str,
        /// Expected literal value.
        expected: &'static str,
        /// Actual value.
        actual: String,
    },
    /// A scalar field failed validation.
    InvalidField {
        /// Field name.
        field_name: &'static str,
        /// Human-readable failure reason.
        message: String,
    },
    /// A document-level invariant failed.
    InvalidDocument {
        /// Human-readable failure reason.
        message: String,
    },
}

impl fmt::Display for RuntimeJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => write!(f, "{error}"),
            Self::Json { artifact, message } => {
                write!(f, "failed to decode {artifact}: {message}")
            }
            Self::UnknownValue { field_name, value } => {
                write!(f, "{field_name} has unknown value: {value}")
            }
            Self::InvalidLiteral {
                field_name,
                expected,
                actual,
            } => write!(
                f,
                "{field_name} must be literal `{expected}`, got `{actual}`"
            ),
            Self::InvalidField {
                field_name,
                message,
            } => write!(f, "{field_name} is invalid: {message}"),
            Self::InvalidDocument { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for RuntimeJsonError {}

impl From<ContractError> for RuntimeJsonError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}

/// Common helpers for JSON artifact contracts.
pub trait RuntimeJsonContract: Sized + Serialize + DeserializeOwned {
    /// Human-readable artifact name used in decode errors.
    const ARTIFACT: &'static str;

    /// Validates and normalizes this decoded artifact.
    fn validate_contract(&mut self) -> Result<(), RuntimeJsonError>;

    /// Deserializes and validates a JSON value.
    fn from_json_value(value: Value) -> Result<Self, RuntimeJsonError> {
        let mut decoded: Self = decode_json(Self::ARTIFACT, value)?;
        decoded.validate_contract()?;
        Ok(decoded)
    }

    /// Deserializes and validates a JSON string.
    fn from_json_str(raw: &str) -> Result<Self, RuntimeJsonError> {
        let value = serde_json::from_str(raw).map_err(|error| RuntimeJsonError::Json {
            artifact: Self::ARTIFACT,
            message: error.to_string(),
        })?;
        Self::from_json_value(value)
    }
}

/// One active plane entry inside a runtime snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveRunState {
    pub plane: Plane,
    pub stage: StageName,
    pub node_id: String,
    pub stage_kind_id: String,
    pub run_id: String,
    pub request_kind: ActiveRunRequestKind,
    pub work_item_kind: Option<WorkItemKind>,
    pub work_item_id: Option<String>,
    pub closure_target_root_spec_id: Option<String>,
    pub closure_target_root_idea_id: Option<String>,
    pub active_since: Timestamp,
    pub running_status_marker: Option<String>,
}

impl ActiveRunState {
    /// Deserializes and validates an active-run JSON value.
    pub fn from_json_value(value: Value) -> Result<Self, RuntimeJsonError> {
        let mut decoded: Self = decode_json("active_run_state", value)?;
        decoded.validate()?;
        Ok(decoded)
    }

    /// Validates active-run stage, request-kind, and identity invariants.
    pub fn validate(&mut self) -> Result<(), RuntimeJsonError> {
        if stage_plane(self.stage) != self.plane {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "active run stage must belong to active run plane".to_owned(),
            });
        }
        require_non_blank("node_id", &self.node_id)?;
        require_non_blank("stage_kind_id", &self.stage_kind_id)?;
        require_non_blank("run_id", &self.run_id)?;

        let has_work_kind = self.work_item_kind.is_some();
        let has_work_id = self.work_item_id.is_some();
        let has_closure_root = self.closure_target_root_spec_id.is_some();
        let has_closure_idea = self.closure_target_root_idea_id.is_some();

        if has_work_kind != has_work_id {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "active run work_item_kind and work_item_id must be set together"
                    .to_owned(),
            });
        }

        match self.request_kind {
            ActiveRunRequestKind::ActiveWorkItem => {
                if !has_work_kind || !has_work_id {
                    return Err(RuntimeJsonError::InvalidDocument {
                        message: "active_work_item active runs require work item identity"
                            .to_owned(),
                    });
                }
                if self.work_item_kind == Some(WorkItemKind::LearningRequest) {
                    return Err(RuntimeJsonError::InvalidDocument {
                        message:
                            "learning request active runs must use request_kind=learning_request"
                                .to_owned(),
                    });
                }
                if has_closure_root || has_closure_idea {
                    return Err(RuntimeJsonError::InvalidDocument {
                        message:
                            "active_work_item active runs cannot declare closure target fields"
                                .to_owned(),
                    });
                }
            }
            ActiveRunRequestKind::LearningRequest => {
                if self.plane != Plane::Learning {
                    return Err(RuntimeJsonError::InvalidDocument {
                        message: "learning_request active runs must use plane=learning".to_owned(),
                    });
                }
                if self.work_item_kind != Some(WorkItemKind::LearningRequest) || !has_work_id {
                    return Err(RuntimeJsonError::InvalidDocument {
                        message:
                            "learning_request active runs require learning_request work item identity"
                                .to_owned(),
                    });
                }
                if has_closure_root || has_closure_idea {
                    return Err(RuntimeJsonError::InvalidDocument {
                        message:
                            "learning_request active runs cannot declare closure target fields"
                                .to_owned(),
                    });
                }
            }
            ActiveRunRequestKind::ClosureTarget => {
                if self.plane != Plane::Planning {
                    return Err(RuntimeJsonError::InvalidDocument {
                        message: "closure_target active runs must use plane=planning".to_owned(),
                    });
                }
                if has_work_kind || has_work_id {
                    return Err(RuntimeJsonError::InvalidDocument {
                        message: "closure_target active runs cannot declare work item identity"
                            .to_owned(),
                    });
                }
                if !has_closure_root {
                    return Err(RuntimeJsonError::InvalidDocument {
                        message: "closure_target active runs require closure_target_root_spec_id"
                            .to_owned(),
                    });
                }
            }
        }

        Ok(())
    }
}

/// Runtime snapshot state persisted by `millrace status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSnapshot {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_runtime_snapshot_kind")]
    pub kind: String,

    pub runtime_mode: RuntimeMode,
    pub process_running: bool,
    pub paused: bool,
    #[serde(default)]
    pub pause_sources: Vec<PauseSource>,
    #[serde(default)]
    pub stop_requested: bool,
    pub active_mode_id: String,
    pub execution_loop_id: String,
    pub planning_loop_id: String,
    pub learning_loop_id: Option<String>,
    #[serde(default)]
    pub loop_ids_by_plane: HashMap<Plane, String>,
    pub compiled_plan_id: String,
    pub compiled_plan_path: String,

    pub active_plane: Option<Plane>,
    pub active_stage: Option<StageName>,
    pub active_node_id: Option<String>,
    pub active_stage_kind_id: Option<String>,
    pub active_run_id: Option<String>,
    pub active_work_item_kind: Option<WorkItemKind>,
    pub active_work_item_id: Option<String>,
    #[serde(default)]
    pub active_runs_by_plane: HashMap<Plane, ActiveRunState>,

    pub execution_status_marker: String,
    pub planning_status_marker: String,
    #[serde(default = "default_idle_marker")]
    pub learning_status_marker: String,
    #[serde(default)]
    pub status_markers_by_plane: HashMap<Plane, String>,

    #[serde(default)]
    pub queue_depth_execution: u64,
    #[serde(default)]
    pub queue_depth_planning: u64,
    #[serde(default)]
    pub queue_depth_learning: u64,
    #[serde(default)]
    pub queue_depths_by_plane: HashMap<Plane, u64>,

    pub last_terminal_result: Option<TerminalResult>,
    pub last_stage_result_path: Option<String>,

    pub current_failure_class: Option<String>,
    #[serde(default)]
    pub troubleshoot_attempt_count: u64,
    #[serde(default)]
    pub mechanic_attempt_count: u64,
    #[serde(default)]
    pub fix_cycle_count: u64,
    #[serde(default)]
    pub consultant_invocations: u64,

    pub config_version: String,
    pub watcher_mode: WatcherMode,
    pub last_reload_outcome: Option<ReloadOutcome>,
    pub last_reload_error: Option<String>,

    pub started_at: Option<Timestamp>,
    pub active_since: Option<Timestamp>,
    pub updated_at: Timestamp,
}

/// Stable read-only payload rendered by `millrace status --format json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadOnlyStatusPayload {
    pub workspace: String,
    pub runtime_mode: RuntimeMode,
    pub process_running: bool,
    pub runtime_ownership_lock: String,
    pub paused: bool,
    pub pause_sources: String,
    pub stop_requested: bool,
    pub active_mode_id: String,
    pub compiled_plan_id: String,
    pub compiled_plan_currentness: String,
    pub active_plane: Option<Plane>,
    pub active_stage: Option<StageName>,
    pub active_node_id: Option<String>,
    pub active_stage_kind_id: Option<String>,
    pub active_work_item_kind: Option<WorkItemKind>,
    pub active_work_item_id: Option<String>,
    pub active_run_count: u64,
    pub execution_queue_depth: u64,
    pub planning_queue_depth: u64,
    pub learning_queue_depth: u64,
    pub execution_status_marker: String,
    pub planning_status_marker: String,
    pub learning_status_marker: String,
    pub blocked_idle: bool,
    pub current_failure_class: Option<String>,
    pub latest_runtime_error_report_path: Option<String>,
    pub closure_target_root_spec_id: Value,
    pub closure_target_open: Value,
    pub closure_target_blocked_by_lineage_work: Value,
    pub planning_root_specs_deferred_by_closure_target: Value,
    pub closure_target_latest_verdict_path: Value,
    pub closure_target_latest_report_path: Value,
}

impl RuntimeJsonContract for RuntimeSnapshot {
    const ARTIFACT: &'static str = "runtime_snapshot";

    fn validate_contract(&mut self) -> Result<(), RuntimeJsonError> {
        validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
        validate_literal("kind", &self.kind, "runtime_snapshot")?;
        self.normalize_pause_sources();
        self.normalize_plane_indexes();
        self.normalize_active_runs()?;
        self.validate_active_state()
    }
}

impl RuntimeSnapshot {
    fn normalize_pause_sources(&mut self) {
        self.pause_sources.dedup();
        if !self.pause_sources.is_empty() {
            self.paused = true;
        } else if self.paused {
            self.pause_sources.push(PauseSource::Operator);
        }
    }

    fn normalize_plane_indexes(&mut self) {
        self.loop_ids_by_plane
            .entry(Plane::Execution)
            .or_insert_with(|| self.execution_loop_id.clone());
        self.loop_ids_by_plane
            .entry(Plane::Planning)
            .or_insert_with(|| self.planning_loop_id.clone());
        if let Some(loop_id) = &self.learning_loop_id {
            self.loop_ids_by_plane
                .entry(Plane::Learning)
                .or_insert_with(|| loop_id.clone());
        }

        self.status_markers_by_plane
            .entry(Plane::Execution)
            .or_insert_with(|| self.execution_status_marker.clone());
        self.status_markers_by_plane
            .entry(Plane::Planning)
            .or_insert_with(|| self.planning_status_marker.clone());
        self.status_markers_by_plane
            .entry(Plane::Learning)
            .or_insert_with(|| self.learning_status_marker.clone());

        self.queue_depths_by_plane
            .entry(Plane::Execution)
            .or_insert(self.queue_depth_execution);
        self.queue_depths_by_plane
            .entry(Plane::Planning)
            .or_insert(self.queue_depth_planning);
        self.queue_depths_by_plane
            .entry(Plane::Learning)
            .or_insert(self.queue_depth_learning);
    }

    fn normalize_active_runs(&mut self) -> Result<(), RuntimeJsonError> {
        if self.active_runs_by_plane.is_empty() {
            self.project_legacy_active_state_into_active_runs()?;
        }

        if !self.active_runs_by_plane.is_empty() {
            for (plane, active_run) in &mut self.active_runs_by_plane {
                active_run.validate()?;
                if *plane != active_run.plane {
                    return Err(RuntimeJsonError::InvalidDocument {
                        message: "active_runs_by_plane key must match active run plane".to_owned(),
                    });
                }
            }

            let (
                plane,
                stage,
                node_id,
                stage_kind_id,
                run_id,
                work_item_kind,
                work_item_id,
                active_since,
            ) = {
                let active_run = self.foreground_active_run()?;
                (
                    active_run.plane,
                    active_run.stage,
                    active_run.node_id.clone(),
                    active_run.stage_kind_id.clone(),
                    active_run.run_id.clone(),
                    active_run.work_item_kind,
                    active_run.work_item_id.clone(),
                    active_run.active_since.clone(),
                )
            };
            self.active_plane = Some(plane);
            self.active_stage = Some(stage);
            self.active_node_id = Some(node_id);
            self.active_stage_kind_id = Some(stage_kind_id);
            self.active_run_id = Some(run_id);
            self.active_work_item_kind = work_item_kind;
            self.active_work_item_id = work_item_id;
            self.active_since = Some(active_since);
        }

        Ok(())
    }

    fn project_legacy_active_state_into_active_runs(&mut self) -> Result<(), RuntimeJsonError> {
        let (
            Some(active_plane),
            Some(active_stage),
            Some(active_run_id),
            Some(active_since),
            Some(active_work_item_kind),
            Some(active_work_item_id),
        ) = (
            self.active_plane,
            self.active_stage,
            self.active_run_id.clone(),
            self.active_since.clone(),
            self.active_work_item_kind,
            self.active_work_item_id.clone(),
        )
        else {
            return Ok(());
        };

        let request_kind = if active_work_item_kind == WorkItemKind::LearningRequest {
            ActiveRunRequestKind::LearningRequest
        } else {
            ActiveRunRequestKind::ActiveWorkItem
        };
        let active_run = ActiveRunState {
            plane: active_plane,
            stage: active_stage,
            node_id: self
                .active_node_id
                .clone()
                .unwrap_or_else(|| active_stage.as_str().to_owned()),
            stage_kind_id: self
                .active_stage_kind_id
                .clone()
                .unwrap_or_else(|| active_stage.as_str().to_owned()),
            run_id: active_run_id,
            request_kind,
            work_item_kind: Some(active_work_item_kind),
            work_item_id: Some(active_work_item_id),
            closure_target_root_spec_id: None,
            closure_target_root_idea_id: None,
            active_since,
            running_status_marker: None,
        };
        self.active_runs_by_plane.insert(active_plane, active_run);
        Ok(())
    }

    fn foreground_active_run(&self) -> Result<&ActiveRunState, RuntimeJsonError> {
        for plane in [Plane::Planning, Plane::Execution, Plane::Learning] {
            if let Some(active_run) = self.active_runs_by_plane.get(&plane) {
                return Ok(active_run);
            }
        }
        Err(RuntimeJsonError::InvalidDocument {
            message: "active_runs_by_plane cannot be empty".to_owned(),
        })
    }

    fn validate_active_state(&mut self) -> Result<(), RuntimeJsonError> {
        if self.active_stage.is_none() && self.active_plane.is_some() {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "active_plane cannot be set when active_stage is missing".to_owned(),
            });
        }

        if let Some(active_stage) = self.active_stage {
            let Some(active_plane) = self.active_plane else {
                return Err(RuntimeJsonError::InvalidDocument {
                    message: "active_plane is required when active_stage is set".to_owned(),
                });
            };
            if stage_plane(active_stage) != active_plane {
                return Err(RuntimeJsonError::InvalidDocument {
                    message: "active_stage must belong to active_plane".to_owned(),
                });
            }
            if self.active_node_id.is_none() {
                self.active_node_id = Some(active_stage.as_str().to_owned());
            }
            if self.active_stage_kind_id.is_none() {
                self.active_stage_kind_id = Some(active_stage.as_str().to_owned());
            }
        } else {
            self.active_node_id = None;
            self.active_stage_kind_id = None;
        }

        if self.active_stage.is_some() {
            require_optional_non_blank("active_node_id", &self.active_node_id)?;
            require_optional_non_blank("active_stage_kind_id", &self.active_stage_kind_id)?;
        }

        let has_kind = self.active_work_item_kind.is_some();
        let has_id = self.active_work_item_id.is_some();
        if has_kind != has_id {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "active_work_item_kind and active_work_item_id must be set together"
                    .to_owned(),
            });
        }
        if has_kind && self.active_stage.is_none() {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "active work item requires active_stage".to_owned(),
            });
        }
        if has_kind && self.active_plane.is_none() {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "active work item requires active_plane".to_owned(),
            });
        }
        if has_kind && self.active_run_id.is_none() {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "active work item requires active_run_id".to_owned(),
            });
        }
        if self.active_since.is_some() && self.active_stage.is_none() {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "active_since requires active_stage".to_owned(),
            });
        }

        Ok(())
    }
}

/// One persisted recovery-counter entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryCounterEntry {
    pub failure_class: String,
    pub work_item_id: String,
    pub work_item_kind: WorkItemKind,
    #[serde(default)]
    pub troubleshoot_attempt_count: u64,
    #[serde(default)]
    pub mechanic_attempt_count: u64,
    #[serde(default)]
    pub fix_cycle_count: u64,
    #[serde(default)]
    pub consultant_invocations: u64,
    pub last_updated_at: Timestamp,
}

/// Recovery-counter state persisted by the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryCounters {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_recovery_counters_kind")]
    pub kind: String,
    #[serde(default)]
    pub entries: Vec<RecoveryCounterEntry>,
}

impl RuntimeJsonContract for RecoveryCounters {
    const ARTIFACT: &'static str = "recovery_counters";

    fn validate_contract(&mut self) -> Result<(), RuntimeJsonError> {
        validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
        validate_literal("kind", &self.kind, "recovery_counters")?;
        for entry in &self.entries {
            require_non_blank("failure_class", &entry.failure_class)?;
            require_non_blank("work_item_id", &entry.work_item_id)?;
        }
        Ok(())
    }
}

/// Runtime mailbox command envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailboxCommandEnvelope {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_mailbox_command_kind")]
    pub kind: String,
    pub command_id: String,
    pub command: MailboxCommand,
    pub issued_at: Timestamp,
    pub issuer: String,
    #[serde(default)]
    pub payload: Map<String, Value>,
}

impl RuntimeJsonContract for MailboxCommandEnvelope {
    const ARTIFACT: &'static str = "mailbox_command";

    fn validate_contract(&mut self) -> Result<(), RuntimeJsonError> {
        validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
        validate_literal("kind", &self.kind, "mailbox_command")?;
        require_non_blank("command_id", &self.command_id)?;
        require_non_blank("issuer", &self.issuer)
    }
}

/// Payload shape for `add_idea` mailbox commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailboxAddIdeaPayload {
    pub source_name: String,
    pub markdown: String,
}

impl MailboxAddIdeaPayload {
    /// Deserializes and validates an add-idea payload JSON value.
    pub fn from_json_value(value: Value) -> Result<Self, RuntimeJsonError> {
        let mut decoded: Self = decode_json("mailbox_add_idea_payload", value)?;
        decoded.validate()?;
        Ok(decoded)
    }

    /// Validates filename and markdown shape.
    pub fn validate(&mut self) -> Result<(), RuntimeJsonError> {
        if self.source_name.trim() != self.source_name {
            return Err(RuntimeJsonError::InvalidField {
                field_name: "source_name",
                message: "must not include surrounding whitespace".to_owned(),
            });
        }
        if self.source_name.is_empty() {
            return Err(RuntimeJsonError::InvalidField {
                field_name: "source_name",
                message: "is required".to_owned(),
            });
        }
        if !self.source_name.ends_with(".md") {
            return Err(RuntimeJsonError::InvalidField {
                field_name: "source_name",
                message: "must end with .md".to_owned(),
            });
        }
        if self.source_name.starts_with('/') || self.source_name.contains('/') {
            return Err(RuntimeJsonError::InvalidField {
                field_name: "source_name",
                message: "must be a single relative filename".to_owned(),
            });
        }
        let stem = self.source_name.trim_end_matches(".md");
        validate_safe_identifier(stem, "source_name")?;
        if self.markdown.trim().is_empty() {
            return Err(RuntimeJsonError::InvalidField {
                field_name: "markdown",
                message: "is required".to_owned(),
            });
        }
        Ok(())
    }
}

/// Payload shape for `add_task` mailbox commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailboxAddTaskPayload {
    pub document: TaskDocument,
}

impl MailboxAddTaskPayload {
    /// Deserializes and validates an add-task payload JSON value.
    pub fn from_json_value(value: Value) -> Result<Self, RuntimeJsonError> {
        let mut decoded: Self = decode_json("mailbox_add_task_payload", value)?;
        decoded.validate()?;
        Ok(decoded)
    }

    /// Validates the embedded task document shape.
    pub fn validate(&mut self) -> Result<(), RuntimeJsonError> {
        self.document
            .validate()
            .map_err(|source| RuntimeJsonError::InvalidDocument {
                message: source.to_string(),
            })
    }
}

/// Payload shape for `add_probe` mailbox commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailboxAddProbePayload {
    pub document: ProbeDocument,
}

impl MailboxAddProbePayload {
    /// Deserializes and validates an add-probe payload JSON value.
    pub fn from_json_value(value: Value) -> Result<Self, RuntimeJsonError> {
        let mut decoded: Self = decode_json("mailbox_add_probe_payload", value)?;
        decoded.validate()?;
        Ok(decoded)
    }

    /// Validates the embedded probe document shape.
    pub fn validate(&mut self) -> Result<(), RuntimeJsonError> {
        self.document
            .validate()
            .map_err(|source| RuntimeJsonError::InvalidDocument {
                message: source.to_string(),
            })
    }
}

/// Payload shape for `add_spec` mailbox commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailboxAddSpecPayload {
    pub document: SpecDocument,
}

impl MailboxAddSpecPayload {
    /// Deserializes and validates an add-spec payload JSON value.
    pub fn from_json_value(value: Value) -> Result<Self, RuntimeJsonError> {
        let mut decoded: Self = decode_json("mailbox_add_spec_payload", value)?;
        decoded.validate()?;
        Ok(decoded)
    }

    /// Validates the embedded spec document shape.
    pub fn validate(&mut self) -> Result<(), RuntimeJsonError> {
        self.document
            .validate()
            .map_err(|source| RuntimeJsonError::InvalidDocument {
                message: source.to_string(),
            })
    }
}

/// Compile diagnostics emitted by the mode compiler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileDiagnostics {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_compile_diagnostics_kind")]
    pub kind: String,
    pub ok: bool,
    pub mode_id: String,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub emitted_at: Timestamp,
}

impl RuntimeJsonContract for CompileDiagnostics {
    const ARTIFACT: &'static str = "compile_diagnostics";

    fn validate_contract(&mut self) -> Result<(), RuntimeJsonError> {
        validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
        validate_literal("kind", &self.kind, "compile_diagnostics")?;
        require_non_blank("mode_id", &self.mode_id)?;
        if !self.ok && self.errors.is_empty() {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "errors are required when ok is false".to_owned(),
            });
        }
        Ok(())
    }
}

/// Token accounting values captured from runner output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub thinking_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

impl RuntimeJsonContract for TokenUsage {
    const ARTIFACT: &'static str = "token_usage";

    fn validate_contract(&mut self) -> Result<(), RuntimeJsonError> {
        Ok(())
    }
}

/// One active usage-governance blocker persisted in governance state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageGovernanceBlocker {
    pub source: UsageGovernanceBlockerSource,
    pub rule_id: String,
    pub window: String,
    pub observed: f64,
    pub threshold: f64,
    #[serde(default)]
    pub metric: Option<UsageGovernanceRuntimeTokenMetric>,
    #[serde(default = "default_true")]
    pub auto_resume_possible: bool,
    #[serde(default)]
    pub next_auto_resume_at: Option<Timestamp>,
    #[serde(default)]
    pub detail: String,
}

impl UsageGovernanceBlocker {
    fn validate(&self) -> Result<(), RuntimeJsonError> {
        require_non_blank("rule_id", &self.rule_id)?;
        require_non_blank("window", &self.window)?;
        require_finite_non_negative("observed", self.observed)?;
        require_finite_non_negative("threshold", self.threshold)?;
        if self.source == UsageGovernanceBlockerSource::RuntimeToken && self.metric.is_none() {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "runtime_token blockers require metric".to_owned(),
            });
        }
        Ok(())
    }
}

/// One subscription-quota window reading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionQuotaWindowReading {
    pub window: UsageGovernanceSubscriptionWindow,
    pub percent_used: f64,
    #[serde(default)]
    pub resets_at: Option<Timestamp>,
    pub read_at: Timestamp,
}

impl SubscriptionQuotaWindowReading {
    fn validate(&self) -> Result<(), RuntimeJsonError> {
        require_percent("percent_used", self.percent_used)?;
        if let Some(resets_at) = &self.resets_at {
            parse_time("resets_at", resets_at)?;
        }
        parse_time("read_at", &self.read_at)?;
        Ok(())
    }
}

/// Subscription-quota telemetry status persisted in governance state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionQuotaStatus {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_subscription_quota_provider")]
    pub provider: UsageGovernanceSubscriptionProvider,
    #[serde(default = "default_subscription_quota_state")]
    pub state: SubscriptionQuotaTelemetryState,
    #[serde(default)]
    pub degraded_policy: Option<UsageGovernanceDegradedPolicy>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub last_refreshed_at: Option<Timestamp>,
    #[serde(default)]
    pub windows: BTreeMap<UsageGovernanceSubscriptionWindow, SubscriptionQuotaWindowReading>,
}

impl Default for SubscriptionQuotaStatus {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: UsageGovernanceSubscriptionProvider::CodexChatGptOauth,
            state: SubscriptionQuotaTelemetryState::Disabled,
            degraded_policy: None,
            detail: None,
            last_refreshed_at: None,
            windows: BTreeMap::new(),
        }
    }
}

impl SubscriptionQuotaStatus {
    fn validate(&self) -> Result<(), RuntimeJsonError> {
        if self.state == SubscriptionQuotaTelemetryState::Disabled && self.enabled {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "disabled subscription quota status cannot be enabled".to_owned(),
            });
        }
        if self.state == SubscriptionQuotaTelemetryState::Healthy && self.windows.is_empty() {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "healthy subscription quota status requires at least one window"
                    .to_owned(),
            });
        }
        if let Some(last_refreshed_at) = &self.last_refreshed_at {
            parse_time("last_refreshed_at", last_refreshed_at)?;
        }
        for (window, reading) in &self.windows {
            reading.validate()?;
            if window != &reading.window {
                return Err(RuntimeJsonError::InvalidDocument {
                    message: "subscription quota window key must match reading.window".to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// Usage-governance state persisted under `millrace-agents/state/`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageGovernanceState {
    #[serde(default = "default_schema_version")]
    pub version: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub auto_resume: bool,
    #[serde(default = "default_true")]
    pub auto_resume_possible: bool,
    #[serde(default = "default_usage_governance_evaluation_boundary")]
    pub evaluation_boundary: UsageGovernanceEvaluationBoundary,
    #[serde(default = "default_calendar_timezone")]
    pub calendar_timezone: String,
    #[serde(default)]
    pub daemon_session_id: Option<String>,
    pub last_evaluated_at: Timestamp,
    #[serde(default)]
    pub active_blockers: Vec<UsageGovernanceBlocker>,
    #[serde(default)]
    pub paused_by_governance: bool,
    #[serde(default)]
    pub next_auto_resume_at: Option<Timestamp>,
    #[serde(default)]
    pub subscription_quota_status: SubscriptionQuotaStatus,
}

impl RuntimeJsonContract for UsageGovernanceState {
    const ARTIFACT: &'static str = "usage_governance_state";

    fn validate_contract(&mut self) -> Result<(), RuntimeJsonError> {
        validate_literal("version", &self.version, SCHEMA_VERSION)?;
        require_non_blank("calendar_timezone", &self.calendar_timezone)?;
        parse_time("last_evaluated_at", &self.last_evaluated_at)?;
        if let Some(next_auto_resume_at) = &self.next_auto_resume_at {
            parse_time("next_auto_resume_at", next_auto_resume_at)?;
        }
        for blocker in &self.active_blockers {
            blocker.validate()?;
        }
        self.subscription_quota_status.validate()?;
        if !self.active_blockers.is_empty() && !self.paused_by_governance {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "active governance blockers require paused_by_governance=true".to_owned(),
            });
        }
        if !self.auto_resume_possible && self.next_auto_resume_at.is_some() {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "next_auto_resume_at requires auto_resume_possible=true".to_owned(),
            });
        }
        Ok(())
    }
}

/// One idempotent usage-governance token ledger entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageGovernanceLedgerEntry {
    pub dedupe_key: String,
    pub counted_at: Timestamp,
    pub stage_completed_at: Timestamp,
    pub plane: Plane,
    pub run_id: String,
    pub stage_id: String,
    pub work_item_kind: WorkItemKind,
    pub work_item_id: String,
    pub token_usage: TokenUsage,
    pub stage_result_path: String,
    #[serde(default)]
    pub daemon_session_id: Option<String>,
}

impl RuntimeJsonContract for UsageGovernanceLedgerEntry {
    const ARTIFACT: &'static str = "usage_governance_ledger_entry";

    fn validate_contract(&mut self) -> Result<(), RuntimeJsonError> {
        require_non_blank("dedupe_key", &self.dedupe_key)?;
        require_non_blank("run_id", &self.run_id)?;
        require_non_blank("stage_id", &self.stage_id)?;
        require_non_blank("work_item_id", &self.work_item_id)?;
        require_non_blank("stage_result_path", &self.stage_result_path)?;
        if self.dedupe_key != self.stage_result_path {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "dedupe_key must match stage_result_path".to_owned(),
            });
        }
        parse_time("counted_at", &self.counted_at)?;
        parse_time("stage_completed_at", &self.stage_completed_at)?;
        self.token_usage.validate_contract()
    }
}

/// Stage-result artifact persisted for one stage request.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StageResultEnvelope {
    pub schema_version: String,
    pub kind: String,
    pub run_id: String,
    pub plane: Plane,
    pub stage: StageName,
    pub node_id: String,
    pub stage_kind_id: String,
    pub work_item_kind: WorkItemKind,
    pub work_item_id: String,
    pub terminal_result: TerminalResult,
    pub result_class: ResultClass,
    pub summary_status_marker: String,
    pub success: bool,
    pub retryable: bool,
    pub exit_code: i32,
    pub duration_seconds: f64,
    pub prompt_artifact: Option<String>,
    pub report_artifact: Option<String>,
    pub artifact_paths: Vec<String>,
    pub detected_marker: Option<String>,
    pub stdout_path: Option<String>,
    pub stderr_path: Option<String>,
    pub runner_name: Option<String>,
    pub model_name: Option<String>,
    pub thinking_level: Option<String>,
    pub model_reasoning_effort: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub notes: Vec<String>,
    pub metadata: Map<String, Value>,
    pub started_at: Timestamp,
    pub completed_at: Timestamp,
}

impl<'de> Deserialize<'de> for StageResultEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        StageResultEnvelopeRaw::deserialize(deserializer)?
            .try_into_stage_result()
            .map_err(serde::de::Error::custom)
    }
}

impl StageResultEnvelope {
    /// Deserializes and validates a stage-result JSON value with typed errors.
    pub fn from_json_value(value: Value) -> Result<Self, RuntimeJsonError> {
        let raw: StageResultEnvelopeRaw = decode_json("stage_result", value)?;
        raw.try_into_stage_result()
    }

    /// Deserializes and validates a stage-result JSON string with typed errors.
    pub fn from_json_str(raw: &str) -> Result<Self, RuntimeJsonError> {
        let value = serde_json::from_str(raw).map_err(|error| RuntimeJsonError::Json {
            artifact: "stage_result",
            message: error.to_string(),
        })?;
        Self::from_json_value(value)
    }

    /// Validates stage, marker, timing, and result-class invariants.
    pub fn validate(&mut self) -> Result<(), RuntimeJsonError> {
        validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
        validate_literal("kind", &self.kind, "stage_result")?;
        if stage_plane(self.stage) != self.plane {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "stage must belong to plane".to_owned(),
            });
        }
        if self.node_id.is_empty() {
            self.node_id = self.stage.as_str().to_owned();
        }
        if self.stage_kind_id.is_empty() {
            self.stage_kind_id = self.stage.as_str().to_owned();
        }

        let marker = self.terminal_result.marker();
        if self.summary_status_marker != marker {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "summary_status_marker must match terminal_result".to_owned(),
            });
        }
        if let Some(detected_marker) = &self.detected_marker {
            let detected = parse_terminal_marker_for_plane(self.plane, detected_marker)?;
            if detected != self.terminal_result {
                return Err(RuntimeJsonError::InvalidDocument {
                    message: "detected_marker must match terminal_result".to_owned(),
                });
            }
        }

        if parse_time("completed_at", &self.completed_at)?
            < parse_time("started_at", &self.started_at)?
        {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "completed_at cannot precede started_at".to_owned(),
            });
        }
        if self.duration_seconds < 0.0 {
            return Err(RuntimeJsonError::InvalidField {
                field_name: "duration_seconds",
                message: "must be >= 0".to_owned(),
            });
        }
        if self.result_class == ResultClass::Success && !self.success {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "success result_class requires success=true".to_owned(),
            });
        }
        if self.result_class != ResultClass::Success && self.success {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "non-success result_class requires success=false".to_owned(),
            });
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StageResultEnvelopeRaw {
    #[serde(default = "default_schema_version")]
    schema_version: String,
    #[serde(default = "default_stage_result_kind")]
    kind: String,
    run_id: String,
    plane: Plane,
    stage: StageName,
    #[serde(default)]
    node_id: String,
    #[serde(default)]
    stage_kind_id: String,
    work_item_kind: WorkItemKind,
    work_item_id: String,
    terminal_result: String,
    result_class: ResultClass,
    summary_status_marker: String,
    success: bool,
    #[serde(default)]
    retryable: bool,
    #[serde(default)]
    exit_code: i32,
    #[serde(default)]
    duration_seconds: f64,
    prompt_artifact: Option<String>,
    report_artifact: Option<String>,
    #[serde(default)]
    artifact_paths: Vec<String>,
    detected_marker: Option<String>,
    stdout_path: Option<String>,
    stderr_path: Option<String>,
    runner_name: Option<String>,
    model_name: Option<String>,
    thinking_level: Option<String>,
    model_reasoning_effort: Option<String>,
    token_usage: Option<TokenUsage>,
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default)]
    metadata: Map<String, Value>,
    started_at: Timestamp,
    completed_at: Timestamp,
}

impl StageResultEnvelopeRaw {
    fn try_into_stage_result(self) -> Result<StageResultEnvelope, RuntimeJsonError> {
        let terminal_result = terminal_result_for_plane(self.plane, &self.terminal_result)?;
        let mut envelope = StageResultEnvelope {
            schema_version: self.schema_version,
            kind: self.kind,
            run_id: self.run_id,
            plane: self.plane,
            stage: self.stage,
            node_id: self.node_id,
            stage_kind_id: self.stage_kind_id,
            work_item_kind: self.work_item_kind,
            work_item_id: self.work_item_id,
            terminal_result,
            result_class: self.result_class,
            summary_status_marker: self.summary_status_marker,
            success: self.success,
            retryable: self.retryable,
            exit_code: self.exit_code,
            duration_seconds: self.duration_seconds,
            prompt_artifact: self.prompt_artifact,
            report_artifact: self.report_artifact,
            artifact_paths: self.artifact_paths,
            detected_marker: self.detected_marker,
            stdout_path: self.stdout_path,
            stderr_path: self.stderr_path,
            runner_name: self.runner_name,
            model_name: self.model_name,
            thinking_level: self.thinking_level,
            model_reasoning_effort: self.model_reasoning_effort,
            token_usage: self.token_usage,
            notes: self.notes,
            metadata: self.metadata,
            started_at: self.started_at,
            completed_at: self.completed_at,
        };
        envelope.validate()?;
        Ok(envelope)
    }
}

/// Runtime error context persisted for recovery routing diagnostics.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuntimeErrorContext {
    pub schema_version: String,
    pub kind: String,
    pub error_code: RuntimeErrorCode,
    pub plane: Plane,
    pub failed_stage: StageName,
    pub repair_stage: StageName,
    pub work_item_kind: WorkItemKind,
    pub work_item_id: String,
    pub run_id: String,
    pub router_action: Option<String>,
    pub terminal_result: Option<TerminalResult>,
    pub stage_result_path: Option<String>,
    pub report_path: String,
    pub exception_type: String,
    pub exception_message: String,
    pub captured_at: Timestamp,
}

impl<'de> Deserialize<'de> for RuntimeErrorContext {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        RuntimeErrorContextRaw::deserialize(deserializer)?
            .try_into_runtime_error_context()
            .map_err(serde::de::Error::custom)
    }
}

impl RuntimeErrorContext {
    /// Deserializes and validates a runtime-error JSON value with typed errors.
    pub fn from_json_value(value: Value) -> Result<Self, RuntimeJsonError> {
        let raw: RuntimeErrorContextRaw = decode_json("runtime_error_context", value)?;
        raw.try_into_runtime_error_context()
    }

    /// Deserializes and validates a runtime-error JSON string with typed errors.
    pub fn from_json_str(raw: &str) -> Result<Self, RuntimeJsonError> {
        let value = serde_json::from_str(raw).map_err(|error| RuntimeJsonError::Json {
            artifact: "runtime_error_context",
            message: error.to_string(),
        })?;
        Self::from_json_value(value)
    }

    /// Validates stage alignment and optional terminal-result alignment.
    pub fn validate(&self) -> Result<(), RuntimeJsonError> {
        validate_literal("schema_version", &self.schema_version, SCHEMA_VERSION)?;
        validate_literal("kind", &self.kind, "runtime_error_context")?;
        if stage_plane(self.failed_stage) != self.plane {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "failed_stage must belong to plane".to_owned(),
            });
        }
        if stage_plane(self.repair_stage) != self.plane {
            return Err(RuntimeJsonError::InvalidDocument {
                message: "repair_stage must belong to plane".to_owned(),
            });
        }
        if let Some(terminal_result) = self.terminal_result {
            let legal = super::legal_terminal_results(self.failed_stage);
            if !legal.contains(&terminal_result) {
                return Err(RuntimeJsonError::Contract(
                    ContractError::TerminalResultNotAllowed {
                        stage: self.failed_stage,
                        terminal_result,
                    },
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeErrorContextRaw {
    #[serde(default = "default_schema_version")]
    schema_version: String,
    #[serde(default = "default_runtime_error_context_kind")]
    kind: String,
    error_code: RuntimeErrorCode,
    plane: Plane,
    failed_stage: StageName,
    repair_stage: StageName,
    work_item_kind: WorkItemKind,
    work_item_id: String,
    run_id: String,
    router_action: Option<String>,
    terminal_result: Option<String>,
    stage_result_path: Option<String>,
    report_path: String,
    exception_type: String,
    exception_message: String,
    captured_at: Timestamp,
}

impl RuntimeErrorContextRaw {
    fn try_into_runtime_error_context(self) -> Result<RuntimeErrorContext, RuntimeJsonError> {
        let terminal_result = self
            .terminal_result
            .as_deref()
            .map(|value| terminal_result_for_plane(self.plane, value))
            .transpose()?;
        let context = RuntimeErrorContext {
            schema_version: self.schema_version,
            kind: self.kind,
            error_code: self.error_code,
            plane: self.plane,
            failed_stage: self.failed_stage,
            repair_stage: self.repair_stage,
            work_item_kind: self.work_item_kind,
            work_item_id: self.work_item_id,
            run_id: self.run_id,
            router_action: self.router_action,
            terminal_result,
            stage_result_path: self.stage_result_path,
            report_path: self.report_path,
            exception_type: self.exception_type,
            exception_message: self.exception_message,
            captured_at: self.captured_at,
        };
        context.validate()?;
        Ok(context)
    }
}

fn decode_json<T>(artifact: &'static str, value: Value) -> Result<T, RuntimeJsonError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value).map_err(|error| RuntimeJsonError::Json {
        artifact,
        message: error.to_string(),
    })
}

fn validate_literal(
    field_name: &'static str,
    actual: &str,
    expected: &'static str,
) -> Result<(), RuntimeJsonError> {
    if actual == expected {
        Ok(())
    } else {
        Err(RuntimeJsonError::InvalidLiteral {
            field_name,
            expected,
            actual: actual.to_owned(),
        })
    }
}

fn require_non_blank(field_name: &'static str, value: &str) -> Result<(), RuntimeJsonError> {
    if value.trim().is_empty() {
        Err(RuntimeJsonError::InvalidField {
            field_name,
            message: "is required".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn require_optional_non_blank(
    field_name: &'static str,
    value: &Option<String>,
) -> Result<(), RuntimeJsonError> {
    if let Some(value) = value {
        require_non_blank(field_name, value)?;
    }
    Ok(())
}

fn require_finite_non_negative(
    field_name: &'static str,
    value: f64,
) -> Result<(), RuntimeJsonError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(RuntimeJsonError::InvalidField {
            field_name,
            message: "must be a finite number >= 0".to_owned(),
        })
    }
}

fn require_percent(field_name: &'static str, value: f64) -> Result<(), RuntimeJsonError> {
    if value.is_finite() && (0.0..=100.0).contains(&value) {
        Ok(())
    } else {
        Err(RuntimeJsonError::InvalidField {
            field_name,
            message: "must be a finite percentage between 0 and 100".to_owned(),
        })
    }
}

fn parse_time(
    field_name: &'static str,
    timestamp: &Timestamp,
) -> Result<OffsetDateTime, RuntimeJsonError> {
    OffsetDateTime::parse(timestamp.as_str(), &Rfc3339).map_err(|error| {
        RuntimeJsonError::InvalidField {
            field_name,
            message: error.to_string(),
        }
    })
}

fn default_schema_version() -> String {
    SCHEMA_VERSION.to_owned()
}

fn default_runtime_snapshot_kind() -> String {
    "runtime_snapshot".to_owned()
}

fn default_recovery_counters_kind() -> String {
    "recovery_counters".to_owned()
}

fn default_mailbox_command_kind() -> String {
    "mailbox_command".to_owned()
}

fn default_compile_diagnostics_kind() -> String {
    "compile_diagnostics".to_owned()
}

fn default_stage_result_kind() -> String {
    "stage_result".to_owned()
}

fn default_runtime_error_context_kind() -> String {
    "runtime_error_context".to_owned()
}

fn default_idle_marker() -> String {
    "### IDLE".to_owned()
}

fn default_true() -> bool {
    true
}

fn default_calendar_timezone() -> String {
    "UTC".to_owned()
}

fn default_usage_governance_evaluation_boundary() -> UsageGovernanceEvaluationBoundary {
    UsageGovernanceEvaluationBoundary::BetweenStages
}

fn default_subscription_quota_provider() -> UsageGovernanceSubscriptionProvider {
    UsageGovernanceSubscriptionProvider::CodexChatGptOauth
}

fn default_subscription_quota_state() -> SubscriptionQuotaTelemetryState {
    SubscriptionQuotaTelemetryState::Disabled
}
