//! Runtime request contracts and once-mode startup lifecycle.

use std::{collections::BTreeSet, fmt};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, MapAccess, Visitor},
    ser::SerializeMap,
};
use serde_json::Value;

use crate::contracts::{
    ContractError, Plane, ResultClass, StageName, WorkItemKind, allowed_result_classes_by_outcome,
    legal_terminal_markers, running_status_marker, stage_plane, terminal_result_for_plane,
};

mod monitor;
mod run_traces;
mod startup;
mod supervisor;
mod tick;
mod usage_governance;

pub use monitor::{
    BasicMonitorRenderer, BasicTerminalMonitor, NullRuntimeMonitorSink, RuntimeMonitorEvent,
    RuntimeMonitorFanout, RuntimeMonitorSink, runtime_monitor_events_from_jsonl,
};
pub use run_traces::{
    RunTraceError, RunTraceResult, derive_run_trace_from_stage_results, inspect_run_trace,
    inspect_run_trace_id, record_router_decision_trace, spawned_work_ref_from_path,
    trace_path_for_run_dir, upsert_stage_result_trace_node,
};
pub use startup::{
    RuntimeFileFingerprint, RuntimePollWatcherState, RuntimeReconciliationSignal,
    RuntimeRunnersConfig, RuntimeStageConfig, RuntimeStartupConfig, RuntimeStartupError,
    RuntimeStartupOptions, RuntimeStartupReconciliation, RuntimeStartupResult,
    RuntimeStartupSession, RuntimeWatchEvent, RuntimeWatcherSession, RuntimeWatcherTarget,
    build_runtime_runner_dispatcher, build_runtime_runner_dispatcher_for_paths,
    build_runtime_watcher_session, compiled_entry_node_for_work_item, load_runtime_startup_config,
    startup_runtime_daemon, startup_runtime_daemon_for_paths, startup_runtime_once,
    startup_runtime_once_for_paths,
};
pub use supervisor::{
    RuntimeDaemonCycleOutcome, RuntimeDaemonLoopExitReason, RuntimeDaemonLoopOptions,
    RuntimeDaemonLoopOutcome, RuntimeDaemonSleeper, RuntimeDaemonSupervisor,
    StageCompletionOutcome, StageWorkerOutcome, StageWorkerResult, ThreadRuntimeDaemonSleeper,
    apply_stage_worker_outcome, can_dispatch_plane, run_runtime_daemon_loop,
    run_runtime_daemon_loop_with_monitor, run_runtime_daemon_supervisor_loop_with_sleeper,
    run_runtime_daemon_supervisor_loop_with_sleeper_and_monitor, run_stage_worker,
};
pub use tick::{
    RouterAction, RouterDecision, RuntimeTickDispatchOutcome, RuntimeTickError, RuntimeTickOptions,
    RuntimeTickOutcome, RuntimeTickOutcomeKind, RuntimeTickResult, run_serial_runtime_tick,
    run_serial_runtime_tick_with_runner,
};
pub use usage_governance::{
    RuntimeTokenRuleConfig, RuntimeTokenRulesConfig, SubscriptionQuotaRuleConfig,
    SubscriptionQuotaRulesConfig, UsageGovernanceConfig, disabled_usage_governance_state,
    evaluate_runtime_token_rules, evaluate_subscription_quota_rules, evaluate_usage_governance,
    healthy_subscription_quota_status, ledger_entry_from_stage_result, next_auto_resume_at,
    reconcile_usage_ledger_from_stage_results, record_stage_result_usage,
    should_record_runtime_tokens, stage_result_dedupe_key, subscription_quota_status_unavailable,
};

/// Stage request category persisted in runner request payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    /// A normal active task/spec/incident stage request.
    ActiveWorkItem,
    /// A planning-plane Arbiter request for an open closure target.
    ClosureTarget,
    /// A learning-plane request for an active learning request document.
    LearningRequest,
}

impl RequestKind {
    /// Returns the canonical serialized value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ActiveWorkItem => "active_work_item",
            Self::ClosureTarget => "closure_target",
            Self::LearningRequest => "learning_request",
        }
    }
}

impl fmt::Display for RequestKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One terminal outcome and its permitted result classes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedResultClassPolicy {
    /// Terminal result token without the `### ` marker prefix.
    pub outcome: String,
    /// Result classes accepted for this terminal outcome.
    pub result_classes: Vec<ResultClass>,
}

/// Ordered Python-compatible `allowed_result_classes_by_outcome` policy map.
///
/// The Python runtime treats this as an insertion-ordered mapping. Rust keeps
/// that order internally so rendered request context lines preserve compiled
/// plan order while still serializing as the Python-shaped JSON object.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowedResultClassesByOutcome {
    entries: Vec<AllowedResultClassPolicy>,
}

impl AllowedResultClassesByOutcome {
    /// Builds the default policy from stage metadata.
    #[must_use]
    pub fn for_stage(stage: StageName) -> Self {
        Self {
            entries: allowed_result_classes_by_outcome(stage)
                .iter()
                .map(|entry| AllowedResultClassPolicy {
                    outcome: entry.terminal_result.as_str().to_owned(),
                    result_classes: entry.result_classes.to_vec(),
                })
                .collect(),
        }
    }

    /// Builds a policy from explicit ordered entries.
    #[must_use]
    pub fn new(entries: Vec<AllowedResultClassPolicy>) -> Self {
        Self { entries }
    }

    /// Returns true when no outcome policy is present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns ordered policy entries.
    #[must_use]
    pub fn entries(&self) -> &[AllowedResultClassPolicy] {
        &self.entries
    }

    /// Returns the allowed result classes for one terminal token.
    #[must_use]
    pub fn result_classes_for(&self, outcome: &str) -> Option<&[ResultClass]> {
        self.entries
            .iter()
            .find(|entry| entry.outcome == outcome)
            .map(|entry| entry.result_classes.as_slice())
    }

    /// Returns terminal markers derived from the ordered outcome keys.
    #[must_use]
    pub fn legal_terminal_markers(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| format!("### {}", entry.outcome))
            .collect()
    }

    fn reorder_to_legal_markers(&mut self, legal_terminal_markers: &[String]) -> bool {
        if legal_terminal_markers.len() != self.entries.len() {
            return false;
        }

        let mut reordered = Vec::with_capacity(self.entries.len());
        let mut used = vec![false; self.entries.len()];
        for marker in legal_terminal_markers {
            let Some(outcome) = marker.strip_prefix("### ") else {
                return false;
            };
            let Some((index, entry)) = self
                .entries
                .iter()
                .enumerate()
                .find(|(index, entry)| !used[*index] && entry.outcome == outcome)
            else {
                return false;
            };
            used[index] = true;
            reordered.push(entry.clone());
        }
        if used.iter().all(|used| *used) {
            self.entries = reordered;
            true
        } else {
            false
        }
    }

    fn validate(&self, plane: Plane) -> Result<(), StageRunRequestError> {
        let mut seen = BTreeSet::new();
        for entry in &self.entries {
            require_non_blank("allowed_result_classes_by_outcome", &entry.outcome)?;
            if !seen.insert(entry.outcome.clone()) {
                return Err(StageRunRequestError::InvalidDocument {
                    message: format!(
                        "allowed_result_classes_by_outcome contains duplicate outcome {}",
                        entry.outcome
                    ),
                });
            }
            terminal_result_for_plane(plane, &entry.outcome)?;
            if entry.result_classes.is_empty() {
                return Err(StageRunRequestError::InvalidDocument {
                    message: format!(
                        "allowed_result_classes_by_outcome.{} must not be empty",
                        entry.outcome
                    ),
                });
            }
        }
        Ok(())
    }
}

impl Serialize for AllowedResultClassesByOutcome {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for entry in &self.entries {
            map.serialize_entry(&entry.outcome, &entry.result_classes)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for AllowedResultClassesByOutcome {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PolicyVisitor;

        impl<'de> Visitor<'de> for PolicyVisitor {
            type Value = AllowedResultClassesByOutcome;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an ordered result-class policy object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut entries = Vec::new();
                let mut seen = BTreeSet::new();
                while let Some((outcome, result_classes)) =
                    map.next_entry::<String, Vec<ResultClass>>()?
                {
                    if !seen.insert(outcome.clone()) {
                        return Err(de::Error::custom(format!(
                            "duplicate terminal outcome `{outcome}`"
                        )));
                    }
                    entries.push(AllowedResultClassPolicy {
                        outcome,
                        result_classes,
                    });
                }
                Ok(AllowedResultClassesByOutcome { entries })
            }
        }

        deserializer.deserialize_map(PolicyVisitor)
    }
}

/// Typed failures for stage-run request validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageRunRequestError {
    /// A lower-level shared contract validation failed.
    Contract(ContractError),
    /// JSON decoding failed.
    Json {
        /// Serde error message.
        message: String,
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

impl fmt::Display for StageRunRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => write!(f, "{error}"),
            Self::Json { message } => write!(f, "failed to decode stage_run_request: {message}"),
            Self::InvalidField {
                field_name,
                message,
            } => write!(f, "{field_name} is invalid: {message}"),
            Self::InvalidDocument { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for StageRunRequestError {}

impl From<ContractError> for StageRunRequestError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}

/// Machine-readable request payload for one stage run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StageRunRequest {
    pub request_id: String,
    pub run_id: String,
    pub plane: Plane,
    pub stage: StageName,
    #[serde(default = "default_request_kind")]
    pub request_kind: RequestKind,

    pub mode_id: String,
    pub compiled_plan_id: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub stage_kind_id: String,
    #[serde(default)]
    pub running_status_marker: String,
    #[serde(default)]
    pub legal_terminal_markers: Vec<String>,
    #[serde(default)]
    pub allowed_result_classes_by_outcome: AllowedResultClassesByOutcome,
    pub entrypoint_path: String,
    pub entrypoint_contract_id: Option<String>,

    #[serde(default)]
    pub required_skill_paths: Vec<String>,
    #[serde(default)]
    pub attached_skill_paths: Vec<String>,

    pub active_work_item_kind: Option<WorkItemKind>,
    pub active_work_item_id: Option<String>,
    pub active_work_item_path: Option<String>,
    pub closure_target_path: Option<String>,
    pub closure_target_root_spec_id: Option<String>,
    pub closure_target_root_idea_id: Option<String>,
    pub canonical_root_spec_path: Option<String>,
    pub canonical_seed_idea_path: Option<String>,
    pub preferred_rubric_path: Option<String>,
    pub preferred_verdict_path: Option<String>,
    pub preferred_report_path: Option<String>,

    pub run_dir: String,
    pub summary_status_path: String,
    pub runtime_snapshot_path: String,
    pub recovery_counters_path: String,
    pub preferred_troubleshoot_report_path: Option<String>,
    pub runtime_error_code: Option<String>,
    pub runtime_error_report_path: Option<String>,
    pub runtime_error_catalog_path: Option<String>,
    pub skill_revision_evidence_path: Option<String>,

    pub runner_name: Option<String>,
    pub model_name: Option<String>,
    pub thinking_level: Option<String>,
    pub model_reasoning_effort: Option<String>,
    #[serde(default)]
    pub timeout_seconds: u64,
}

impl<'de> Deserialize<'de> for StageRunRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        StageRunRequestRaw::deserialize(deserializer)?
            .try_into_stage_run_request()
            .map_err(serde::de::Error::custom)
    }
}

impl StageRunRequest {
    /// Deserializes and validates a stage-run request JSON value.
    pub fn from_json_value(value: Value) -> Result<Self, StageRunRequestError> {
        serde_json::from_value(value).map_err(|error| StageRunRequestError::Json {
            message: error.to_string(),
        })
    }

    /// Deserializes and validates a stage-run request JSON string.
    pub fn from_json_str(raw: &str) -> Result<Self, StageRunRequestError> {
        serde_json::from_str(raw).map_err(|error| StageRunRequestError::Json {
            message: error.to_string(),
        })
    }

    /// Validates and fills Python-compatible request defaults.
    pub fn validate(&mut self) -> Result<(), StageRunRequestError> {
        require_non_blank("request_id", &self.request_id)?;
        require_non_blank("run_id", &self.run_id)?;
        require_non_blank("mode_id", &self.mode_id)?;
        require_non_blank("compiled_plan_id", &self.compiled_plan_id)?;
        require_non_blank("entrypoint_path", &self.entrypoint_path)?;
        require_non_blank("run_dir", &self.run_dir)?;
        require_non_blank("summary_status_path", &self.summary_status_path)?;
        require_non_blank("runtime_snapshot_path", &self.runtime_snapshot_path)?;
        require_non_blank("recovery_counters_path", &self.recovery_counters_path)?;

        if stage_plane(self.stage) != self.plane {
            return Err(StageRunRequestError::InvalidDocument {
                message: "stage must belong to plane".to_owned(),
            });
        }
        if self.node_id.is_empty() {
            self.node_id = self.stage.as_str().to_owned();
        }
        if self.stage_kind_id.is_empty() {
            self.stage_kind_id = self.stage.as_str().to_owned();
        }
        if self.running_status_marker.is_empty() {
            self.running_status_marker = running_status_marker(self.stage).to_owned();
        }
        if self.allowed_result_classes_by_outcome.is_empty() {
            self.allowed_result_classes_by_outcome =
                AllowedResultClassesByOutcome::for_stage(self.stage);
        }
        if self.legal_terminal_markers.is_empty() {
            self.legal_terminal_markers = legal_terminal_markers(self.stage);
        }

        require_non_blank("node_id", &self.node_id)?;
        require_non_blank("stage_kind_id", &self.stage_kind_id)?;
        require_non_blank("running_status_marker", &self.running_status_marker)?;
        self.allowed_result_classes_by_outcome
            .validate(self.plane)?;

        let expected_markers = self
            .allowed_result_classes_by_outcome
            .legal_terminal_markers();
        if self.legal_terminal_markers != expected_markers
            && !self
                .allowed_result_classes_by_outcome
                .reorder_to_legal_markers(&self.legal_terminal_markers)
        {
            return Err(StageRunRequestError::InvalidDocument {
                message: "legal_terminal_markers must match allowed_result_classes_by_outcome keys"
                    .to_owned(),
            });
        }

        let has_kind = self.active_work_item_kind.is_some();
        let has_id = self.active_work_item_id.is_some();
        if has_kind != has_id {
            return Err(StageRunRequestError::InvalidDocument {
                message: "active_work_item_kind and active_work_item_id must be set together"
                    .to_owned(),
            });
        }

        match self.request_kind {
            RequestKind::ActiveWorkItem => {
                if self.has_any_closure_field() {
                    return Err(StageRunRequestError::InvalidDocument {
                        message: "active_work_item requests cannot declare closure target fields"
                            .to_owned(),
                    });
                }
            }
            RequestKind::ClosureTarget => {
                if has_kind || self.active_work_item_path.is_some() {
                    return Err(StageRunRequestError::InvalidDocument {
                        message: "closure_target requests cannot declare active work item fields"
                            .to_owned(),
                    });
                }
                if !self.has_all_closure_fields() {
                    return Err(StageRunRequestError::InvalidDocument {
                        message: "closure_target requests require closure target fields".to_owned(),
                    });
                }
            }
            RequestKind::LearningRequest => {
                if self.plane != Plane::Learning {
                    return Err(StageRunRequestError::InvalidDocument {
                        message: "learning_request requests must run on the learning plane"
                            .to_owned(),
                    });
                }
                if self.active_work_item_kind != Some(WorkItemKind::LearningRequest) {
                    return Err(StageRunRequestError::InvalidDocument {
                        message:
                            "learning_request requests require learning_request work item kind"
                                .to_owned(),
                    });
                }
                if self.active_work_item_path.is_none() {
                    return Err(StageRunRequestError::InvalidDocument {
                        message: "learning_request requests require active_work_item_path"
                            .to_owned(),
                    });
                }
                if self.has_any_closure_field() {
                    return Err(StageRunRequestError::InvalidDocument {
                        message: "learning_request requests cannot declare closure target fields"
                            .to_owned(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Renders the request into the prompt context lines consumed by runner adapters.
    #[must_use]
    pub fn render_context_lines(&self) -> Vec<String> {
        let active_kind = self
            .active_work_item_kind
            .map(|kind| kind.as_str())
            .unwrap_or("none");
        let mut lines = vec![
            format!("Request ID: {}", self.request_id),
            format!("Run ID: {}", self.run_id),
            format!("Mode ID: {}", self.mode_id),
            format!("Compiled Plan ID: {}", self.compiled_plan_id),
            format!("Node ID: {}", self.node_id),
            format!("Stage Kind ID: {}", self.stage_kind_id),
            format!("Stage: {}", self.stage.as_str()),
            format!("Plane: {}", self.plane.as_str()),
            format!("Request Kind: {}", self.request_kind.as_str()),
            format!("Running Status Marker: {}", self.running_status_marker),
            format!("Entrypoint Path: {}", self.entrypoint_path),
            format!(
                "Entrypoint Contract ID: {}",
                self.entrypoint_contract_id.as_deref().unwrap_or("none")
            ),
            format!(
                "Active Work Item: {} {}",
                active_kind,
                self.active_work_item_id.as_deref().unwrap_or("none")
            ),
            format!(
                "Active Work Item Path: {}",
                self.active_work_item_path.as_deref().unwrap_or("none")
            ),
            format!(
                "Closure Target Path: {}",
                self.closure_target_path.as_deref().unwrap_or("none")
            ),
            format!(
                "Closure Target Root Spec ID: {}",
                self.closure_target_root_spec_id
                    .as_deref()
                    .unwrap_or("none")
            ),
            format!(
                "Closure Target Root Idea ID: {}",
                self.closure_target_root_idea_id
                    .as_deref()
                    .unwrap_or("none")
            ),
            format!(
                "Canonical Root Spec Path: {}",
                self.canonical_root_spec_path.as_deref().unwrap_or("none")
            ),
            format!(
                "Canonical Seed Idea Path: {}",
                self.canonical_seed_idea_path.as_deref().unwrap_or("none")
            ),
            format!(
                "Preferred Rubric Path: {}",
                self.preferred_rubric_path.as_deref().unwrap_or("none")
            ),
            format!(
                "Preferred Verdict Path: {}",
                self.preferred_verdict_path.as_deref().unwrap_or("none")
            ),
            format!(
                "Preferred Report Path: {}",
                self.preferred_report_path.as_deref().unwrap_or("none")
            ),
        ];

        push_value_list(
            &mut lines,
            "Legal Terminal Markers",
            &self.legal_terminal_markers,
        );
        push_result_class_policy(
            &mut lines,
            "Allowed Result Classes By Outcome",
            &self.allowed_result_classes_by_outcome,
        );
        push_value_list(
            &mut lines,
            "Required Skill Paths",
            &self.required_skill_paths,
        );
        push_value_list(
            &mut lines,
            "Attached Skill Paths",
            &self.attached_skill_paths,
        );
        lines.extend([
            format!("Run Directory: {}", self.run_dir),
            format!("Runtime Snapshot Path: {}", self.runtime_snapshot_path),
            format!("Recovery Counters Path: {}", self.recovery_counters_path),
            format!("Summary Status Path: {}", self.summary_status_path),
            format!(
                "Preferred Troubleshoot Report Path: {}",
                self.preferred_troubleshoot_report_path
                    .as_deref()
                    .unwrap_or("none")
            ),
            format!(
                "Runtime Error Code: {}",
                self.runtime_error_code.as_deref().unwrap_or("none")
            ),
            format!(
                "Runtime Error Report Path: {}",
                self.runtime_error_report_path.as_deref().unwrap_or("none")
            ),
            format!(
                "Runtime Error Catalog Path: {}",
                self.runtime_error_catalog_path.as_deref().unwrap_or("none")
            ),
            format!(
                "Skill Revision Evidence Path: {}",
                self.skill_revision_evidence_path
                    .as_deref()
                    .unwrap_or("none")
            ),
            format!(
                "Runner Name: {}",
                self.runner_name.as_deref().unwrap_or("none")
            ),
            format!(
                "Model Name: {}",
                self.model_name.as_deref().unwrap_or("none")
            ),
            format!(
                "Thinking Level: {}",
                self.thinking_level.as_deref().unwrap_or("none")
            ),
            format!(
                "Model Reasoning Effort: {}",
                self.model_reasoning_effort.as_deref().unwrap_or("none")
            ),
            format!("Timeout Seconds: {}", self.timeout_seconds),
        ]);
        lines
    }

    fn has_any_closure_field(&self) -> bool {
        self.closure_target_path.is_some()
            || self.closure_target_root_spec_id.is_some()
            || self.closure_target_root_idea_id.is_some()
            || self.canonical_root_spec_path.is_some()
            || self.canonical_seed_idea_path.is_some()
            || self.preferred_rubric_path.is_some()
            || self.preferred_verdict_path.is_some()
            || self.preferred_report_path.is_some()
    }

    fn has_all_closure_fields(&self) -> bool {
        self.closure_target_path.is_some()
            && self.closure_target_root_spec_id.is_some()
            && self.closure_target_root_idea_id.is_some()
            && self.canonical_root_spec_path.is_some()
            && self.canonical_seed_idea_path.is_some()
            && self.preferred_rubric_path.is_some()
            && self.preferred_verdict_path.is_some()
            && self.preferred_report_path.is_some()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StageRunRequestRaw {
    request_id: String,
    run_id: String,
    plane: Plane,
    stage: StageName,
    #[serde(default = "default_request_kind")]
    request_kind: RequestKind,

    mode_id: String,
    compiled_plan_id: String,
    #[serde(default)]
    node_id: String,
    #[serde(default)]
    stage_kind_id: String,
    #[serde(default)]
    running_status_marker: String,
    #[serde(default)]
    legal_terminal_markers: Vec<String>,
    #[serde(default)]
    allowed_result_classes_by_outcome: AllowedResultClassesByOutcome,
    entrypoint_path: String,
    entrypoint_contract_id: Option<String>,

    #[serde(default)]
    required_skill_paths: Vec<String>,
    #[serde(default)]
    attached_skill_paths: Vec<String>,

    active_work_item_kind: Option<WorkItemKind>,
    active_work_item_id: Option<String>,
    active_work_item_path: Option<String>,
    closure_target_path: Option<String>,
    closure_target_root_spec_id: Option<String>,
    closure_target_root_idea_id: Option<String>,
    canonical_root_spec_path: Option<String>,
    canonical_seed_idea_path: Option<String>,
    preferred_rubric_path: Option<String>,
    preferred_verdict_path: Option<String>,
    preferred_report_path: Option<String>,

    run_dir: String,
    summary_status_path: String,
    runtime_snapshot_path: String,
    recovery_counters_path: String,
    preferred_troubleshoot_report_path: Option<String>,
    runtime_error_code: Option<String>,
    runtime_error_report_path: Option<String>,
    runtime_error_catalog_path: Option<String>,
    skill_revision_evidence_path: Option<String>,

    runner_name: Option<String>,
    model_name: Option<String>,
    thinking_level: Option<String>,
    model_reasoning_effort: Option<String>,
    #[serde(default)]
    timeout_seconds: u64,
}

impl StageRunRequestRaw {
    fn try_into_stage_run_request(self) -> Result<StageRunRequest, StageRunRequestError> {
        let mut request = StageRunRequest {
            request_id: self.request_id,
            run_id: self.run_id,
            plane: self.plane,
            stage: self.stage,
            request_kind: self.request_kind,
            mode_id: self.mode_id,
            compiled_plan_id: self.compiled_plan_id,
            node_id: self.node_id,
            stage_kind_id: self.stage_kind_id,
            running_status_marker: self.running_status_marker,
            legal_terminal_markers: self.legal_terminal_markers,
            allowed_result_classes_by_outcome: self.allowed_result_classes_by_outcome,
            entrypoint_path: self.entrypoint_path,
            entrypoint_contract_id: self.entrypoint_contract_id,
            required_skill_paths: self.required_skill_paths,
            attached_skill_paths: self.attached_skill_paths,
            active_work_item_kind: self.active_work_item_kind,
            active_work_item_id: self.active_work_item_id,
            active_work_item_path: self.active_work_item_path,
            closure_target_path: self.closure_target_path,
            closure_target_root_spec_id: self.closure_target_root_spec_id,
            closure_target_root_idea_id: self.closure_target_root_idea_id,
            canonical_root_spec_path: self.canonical_root_spec_path,
            canonical_seed_idea_path: self.canonical_seed_idea_path,
            preferred_rubric_path: self.preferred_rubric_path,
            preferred_verdict_path: self.preferred_verdict_path,
            preferred_report_path: self.preferred_report_path,
            run_dir: self.run_dir,
            summary_status_path: self.summary_status_path,
            runtime_snapshot_path: self.runtime_snapshot_path,
            recovery_counters_path: self.recovery_counters_path,
            preferred_troubleshoot_report_path: self.preferred_troubleshoot_report_path,
            runtime_error_code: self.runtime_error_code,
            runtime_error_report_path: self.runtime_error_report_path,
            runtime_error_catalog_path: self.runtime_error_catalog_path,
            skill_revision_evidence_path: self.skill_revision_evidence_path,
            runner_name: self.runner_name,
            model_name: self.model_name,
            thinking_level: self.thinking_level,
            model_reasoning_effort: self.model_reasoning_effort,
            timeout_seconds: self.timeout_seconds,
        };
        request.validate()?;
        Ok(request)
    }
}

/// Renders request fields into runner-agnostic prompt context lines.
#[must_use]
pub fn render_stage_request_context_lines(request: &StageRunRequest) -> Vec<String> {
    request.render_context_lines()
}

fn push_value_list(lines: &mut Vec<String>, label: &str, values: &[String]) {
    if values.is_empty() {
        lines.push(format!("{label}: none"));
        return;
    }
    lines.push(format!("{label}:"));
    lines.extend(values.iter().map(|value| format!("- {value}")));
}

fn push_result_class_policy(
    lines: &mut Vec<String>,
    label: &str,
    policy: &AllowedResultClassesByOutcome,
) {
    if policy.is_empty() {
        lines.push(format!("{label}: none"));
        return;
    }
    lines.push(format!("{label}:"));
    for entry in policy.entries() {
        let rendered = entry
            .result_classes
            .iter()
            .map(|result_class| result_class.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("- {}: {}", entry.outcome, rendered));
    }
}

fn require_non_blank(field_name: &'static str, value: &str) -> Result<(), StageRunRequestError> {
    if value.trim().is_empty() {
        Err(StageRunRequestError::InvalidField {
            field_name,
            message: "is required".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn default_request_kind() -> RequestKind {
    RequestKind::ActiveWorkItem
}
