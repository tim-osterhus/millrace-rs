//! Live runtime monitor event rendering.

use std::{
    collections::HashMap,
    io::{self, Write},
};

use serde_json::{Map, Value};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::contracts::{Timestamp, stage_metadata_for_value};

const IDLE_HEARTBEAT_SECONDS: f64 = 120.0;
const RUN_HANDLE_LENGTHS: [usize; 3] = [8, 12, 16];

/// Structured live event emitted through the runtime monitor seam.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeMonitorEvent {
    /// Stable runtime event type.
    pub event_type: String,
    /// RFC 3339 event timestamp.
    pub occurred_at: Timestamp,
    /// Event-specific payload fields.
    pub payload: Map<String, Value>,
}

impl RuntimeMonitorEvent {
    /// Builds a monitor event from already-normalized fields.
    #[must_use]
    pub fn new(
        event_type: impl Into<String>,
        occurred_at: Timestamp,
        payload: Map<String, Value>,
    ) -> Self {
        Self {
            event_type: event_type.into(),
            occurred_at,
            payload,
        }
    }

    /// Converts one persisted `runtime_events.jsonl` JSON object into a monitor event.
    pub fn from_runtime_event_value(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "runtime event must be a JSON object".to_owned())?;
        let event_type = required_string(object, "event_type")?;
        let occurred_at_raw = required_string(object, "occurred_at")?;
        let occurred_at =
            Timestamp::parse("occurred_at", &occurred_at_raw).map_err(|error| error.to_string())?;
        let payload = object
            .get("payload")
            .or_else(|| object.get("data"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        Ok(Self::new(event_type, occurred_at, payload))
    }
}

/// Parses persisted runtime event JSONL into monitor events.
pub fn runtime_monitor_events_from_jsonl(raw: &str) -> Result<Vec<RuntimeMonitorEvent>, String> {
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .map_err(|error| error.to_string())
                .and_then(RuntimeMonitorEvent::from_runtime_event_value)
        })
        .collect()
}

/// Consumer for structured live runtime monitor events.
pub trait RuntimeMonitorSink {
    /// Emits one event.
    fn emit(&mut self, event: &RuntimeMonitorEvent) -> io::Result<()>;
}

/// Monitor sink that intentionally discards live events.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullRuntimeMonitorSink;

impl RuntimeMonitorSink for NullRuntimeMonitorSink {
    fn emit(&mut self, _event: &RuntimeMonitorEvent) -> io::Result<()> {
        Ok(())
    }
}

/// Fanout sink that sends each monitor event to every child sink in order.
pub struct RuntimeMonitorFanout {
    sinks: Vec<Box<dyn RuntimeMonitorSink>>,
}

impl RuntimeMonitorFanout {
    /// Builds a fanout sink from owned child sinks.
    #[must_use]
    pub fn new(sinks: Vec<Box<dyn RuntimeMonitorSink>>) -> Self {
        Self { sinks }
    }

    /// Returns the number of child sinks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sinks.len()
    }

    /// Returns true when no child sinks are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }

    /// Emits one event to every child sink.
    pub fn emit(&mut self, event: &RuntimeMonitorEvent) -> io::Result<()> {
        for sink in &mut self.sinks {
            sink.emit(event)?;
        }
        Ok(())
    }
}

impl RuntimeMonitorSink for RuntimeMonitorFanout {
    fn emit(&mut self, event: &RuntimeMonitorEvent) -> io::Result<()> {
        RuntimeMonitorFanout::emit(self, event)
    }
}

/// Concise line-oriented terminal renderer for daemon progress.
#[derive(Debug)]
pub struct BasicTerminalMonitor<W> {
    stream: W,
    renderer: BasicMonitorRenderer,
}

impl<W> BasicTerminalMonitor<W>
where
    W: Write,
{
    /// Builds a basic terminal monitor over a writable stream.
    #[must_use]
    pub fn new(stream: W) -> Self {
        Self {
            stream,
            renderer: BasicMonitorRenderer::default(),
        }
    }

    /// Emits one event as zero or more stable terminal lines.
    pub fn emit(&mut self, event: &RuntimeMonitorEvent) -> io::Result<()> {
        for line in self.renderer.render_event(event) {
            writeln!(self.stream, "{line}")?;
        }
        self.stream.flush()
    }

    /// Returns the wrapped writer.
    pub fn into_inner(self) -> W {
        self.stream
    }
}

impl<W> RuntimeMonitorSink for BasicTerminalMonitor<W>
where
    W: Write,
{
    fn emit(&mut self, event: &RuntimeMonitorEvent) -> io::Result<()> {
        BasicTerminalMonitor::emit(self, event)
    }
}

/// Stateful renderer for Python-compatible basic monitor lines.
#[derive(Debug, Default, Clone)]
pub struct BasicMonitorRenderer {
    run_state: HashMap<(String, String), RunAggregate>,
    idle_state: IdleRenderState,
    display_ids: DisplayIdRegistry,
}

impl BasicMonitorRenderer {
    /// Renders one event into zero or more terminal lines.
    pub fn render_event(&mut self, event: &RuntimeMonitorEvent) -> Vec<String> {
        let prefix = format!("[{}] ", format_timestamp_hms(&event.occurred_at));
        if !matches!(
            event.event_type.as_str(),
            "runtime_idle" | "runtime_tick_idle"
        ) {
            self.idle_state.reset();
        }

        let lines = match event.event_type.as_str() {
            "runtime_started" => render_runtime_started(&event.payload),
            "runtime_resumed_active_run" => {
                vec![render_resumed_active_run(
                    &event.payload,
                    &mut self.display_ids,
                )]
            }
            "stage_started" => {
                seed_stage_started(&event.payload, &event.occurred_at, &mut self.run_state);
                vec![render_stage_started(&event.payload, &mut self.display_ids)]
            }
            "stage_completed" => {
                let run_update = record_stage_completed(
                    &event.payload,
                    &mut self.run_state,
                    &mut self.display_ids,
                );
                vec![
                    render_stage_completed(&event.payload, &mut self.display_ids),
                    run_update,
                ]
            }
            "router_decision" => vec![render_router_decision(&event.payload)],
            "status_marker_changed" => {
                render_status_marker_changed(&event.payload, &mut self.display_ids)
                    .into_iter()
                    .collect()
            }
            "runtime_idle" | "runtime_tick_idle" => {
                let reason = string_or_default(event.payload.get("reason"), "no_work");
                if !should_render_idle(&reason, &event.occurred_at, &self.idle_state) {
                    Vec::new()
                } else {
                    self.idle_state.reason = Some(reason.clone());
                    self.idle_state.last_emitted_at = parse_datetime(event.occurred_at.as_str());
                    vec![format!("idle reason={reason}")]
                }
            }
            "runtime_paused" | "runtime_tick_paused" | "mailbox_pause_applied" => {
                vec![format!(
                    "paused reason={}",
                    string_or_default(event.payload.get("reason"), "paused")
                )]
            }
            "runtime_stopped" | "runtime_tick_stopped" | "mailbox_stop_applied" => {
                vec![format!(
                    "stopped reason={}",
                    string_or_default(event.payload.get("reason"), "stop_requested")
                )]
            }
            "runtime_config_reload_deferred" => {
                let active = plane_list(event.payload.get("active_planes"));
                vec![format!(
                    "reload deferred reason={} active={active}",
                    string_or_default(event.payload.get("reason"), "unknown")
                )]
            }
            "runtime_config_reloaded" => vec![format!(
                "reload applied mode={} plan={}",
                string(event.payload.get("mode_id")),
                string(event.payload.get("compiled_plan_id"))
            )],
            "watcher_events_consumed" => vec![format!(
                "watcher events count={} handled={} failures={}",
                number_string(event.payload.get("count")),
                number_string(event.payload.get("handled_count")),
                number_string(event.payload.get("failure_count"))
            )],
            "watcher_event_failed" => vec![format!(
                "watcher failed target={} path={} error={}",
                string(event.payload.get("target")),
                string(event.payload.get("path")),
                string(event.payload.get("error"))
            )],
            "learning_curator_promotion_deferred" => {
                let active = plane_list(event.payload.get("foreground_active_planes"));
                vec![format!("curator promotion deferred active={active}")]
            }
            "learning_curator_promotion_applied" => vec![format!(
                "curator promotion applied source={}",
                string(event.payload.get("source"))
            )],
            "usage_governance_paused" => vec![render_usage_governance_paused(&event.payload)],
            "usage_governance_blocked" => vec![render_usage_governance_blocked(&event.payload)],
            "usage_governance_resumed" => vec![render_usage_governance_resumed(&event.payload)],
            "usage_governance_degraded" => vec![render_usage_governance_degraded(&event.payload)],
            "usage_governance_reconciled" => {
                vec![render_usage_governance_reconciled(&event.payload)]
            }
            _ => Vec::new(),
        };

        lines
            .into_iter()
            .map(|line| format!("{prefix}{line}"))
            .collect()
    }
}

#[derive(Debug, Default, Clone)]
struct IdleRenderState {
    reason: Option<String>,
    last_emitted_at: Option<OffsetDateTime>,
}

impl IdleRenderState {
    fn reset(&mut self) {
        self.reason = None;
        self.last_emitted_at = None;
    }
}

#[derive(Debug, Default, Clone)]
struct RunAggregate {
    first_started_at: Option<OffsetDateTime>,
    latest_completed_at: Option<OffsetDateTime>,
    fallback_elapsed_seconds: f64,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    thinking_tokens: u64,
    total_tokens: u64,
    has_known_tokens: bool,
}

#[derive(Debug, Default, Clone)]
struct DisplayIdRegistry {
    handles_by_run_id: HashMap<String, String>,
    run_id_by_handle: HashMap<String, String>,
}

impl DisplayIdRegistry {
    fn run(&mut self, run_id: Option<&Value>) -> String {
        let raw = string(run_id);
        if let Some(handle) = self.handles_by_run_id.get(&raw) {
            return handle.clone();
        }

        for candidate in run_handle_candidates(&raw) {
            match self.run_id_by_handle.get(&candidate) {
                None => {
                    self.handles_by_run_id
                        .insert(raw.clone(), candidate.clone());
                    self.run_id_by_handle.insert(candidate.clone(), raw);
                    return candidate;
                }
                Some(existing) if existing == &raw => {
                    self.handles_by_run_id
                        .insert(raw.clone(), candidate.clone());
                    return candidate;
                }
                _ => {}
            }
        }

        self.handles_by_run_id.insert(raw.clone(), raw.clone());
        self.run_id_by_handle.insert(raw.clone(), raw.clone());
        raw
    }
}

fn render_runtime_started(payload: &Map<String, Value>) -> Vec<String> {
    let mut lines = vec![
        format!(
            "runtime started mode={} plan={} currentness={}",
            string(payload.get("mode_id")),
            string(payload.get("compiled_plan_id")),
            string(payload.get("compiled_plan_currentness"))
        ),
        format!(
            "baseline manifest={} seed_package={}",
            string(payload.get("baseline_manifest_id")),
            string(payload.get("baseline_seed_package_version"))
        ),
    ];

    if let Some(loop_ids) = object_mapping(payload.get("loop_ids_by_plane")) {
        lines.push(format!("loops {}", format_plane_mapping(loop_ids, false)));
    }

    if let Some(policy) = payload.get("concurrency_policy").and_then(Value::as_object) {
        lines.push(format!("concurrency {}", format_concurrency_policy(policy)));
    } else {
        lines.push("concurrency none".to_owned());
    }
    lines.push(format!(
        "scheduler mode={}",
        string(payload.get("scheduler_mode"))
    ));

    let mut snapshot = String::from("snapshot status");
    if let Some(statuses) = object_mapping(payload.get("status_markers_by_plane")) {
        let formatted = format_plane_mapping(statuses, true);
        if !formatted.is_empty() {
            snapshot.push(' ');
            snapshot.push_str(&formatted);
        }
    }
    if let Some(depths) = object_mapping(payload.get("queue_depths_by_plane")) {
        let formatted = format_plane_mapping(depths, false);
        if !formatted.is_empty() {
            snapshot.push_str(" queue ");
            snapshot.push_str(&formatted);
        }
    }
    lines.push(snapshot);
    lines
}

fn render_resumed_active_run(
    payload: &Map<String, Value>,
    display_ids: &mut DisplayIdRegistry,
) -> String {
    let mut parts = vec![
        "resumed active".to_owned(),
        stage_ref(
            &string(payload.get("active_plane")),
            &string(payload.get("active_stage")),
            &string(payload.get("active_node_id")),
            &string(payload.get("active_stage_kind_id")),
        ),
        format!("run={}", display_ids.run(payload.get("active_run_id"))),
    ];
    let marker = normalize_marker(&string(payload.get("status_marker")));
    if marker != "unknown" {
        parts.push(format!("status={marker}"));
    }
    parts.join(" ")
}

fn render_stage_started(
    payload: &Map<String, Value>,
    display_ids: &mut DisplayIdRegistry,
) -> String {
    let mut parts = vec![
        "stage start".to_owned(),
        stage_ref_from_payload(payload),
        format!("run={}", display_ids.run(payload.get("run_id"))),
    ];
    if let Some(work_ref) = work_ref(payload) {
        parts.push(format!("work={work_ref}"));
    }
    if let Some(status) = nonredundant_running_status(payload) {
        parts.push(format!("status={status}"));
    }
    parts.join(" ")
}

fn render_stage_completed(
    payload: &Map<String, Value>,
    display_ids: &mut DisplayIdRegistry,
) -> String {
    let mut parts = vec![
        "stage done".to_owned(),
        stage_ref_from_payload(payload),
        format!("run={}", display_ids.run(payload.get("run_id"))),
        format!("result={}", string(payload.get("terminal_result"))),
    ];
    if let Some(status) = nonredundant_terminal_status(payload) {
        parts.push(format!("status={status}"));
    }
    parts.push(format!(
        "dur={}",
        format_seconds(float_value(payload.get("duration_seconds")))
    ));
    if let Some(token_usage) = format_token_usage(payload.get("token_usage")) {
        parts.push(format!("tokens={token_usage}"));
    }
    parts.join(" ")
}

fn render_router_decision(payload: &Map<String, Value>) -> String {
    let action = string(payload.get("action"));
    let plane = string(payload.get("plane"));
    let next_stage = optional_string(payload.get("next_stage"));
    let mut parts = if action == "idle" {
        vec!["route".to_owned(), plane, "done".to_owned()]
    } else if action == "blocked" {
        vec!["route".to_owned(), plane, "blocked".to_owned()]
    } else if next_stage.is_some() {
        let mut items = vec![
            "route".to_owned(),
            plane,
            "->".to_owned(),
            route_target(payload),
        ];
        if !matches!(action.as_str(), "run_stage" | "unknown") {
            items.push(format!("action={action}"));
        }
        items
    } else {
        vec!["route".to_owned(), plane, format!("action={action}")]
    };

    if let Some(reason) = optional_string(payload.get("reason")) {
        parts.push(format!("reason={reason}"));
    }
    parts.join(" ")
}

fn render_status_marker_changed(
    payload: &Map<String, Value>,
    display_ids: &mut DisplayIdRegistry,
) -> Option<String> {
    let source = string(payload.get("source"));
    if matches!(source.as_str(), "stage_started" | "stage_completed") {
        return None;
    }
    let previous_marker = normalize_marker(&string(payload.get("previous_marker")));
    let current_marker = normalize_marker(&string(payload.get("current_marker")));
    if source == "router_idle"
        && current_marker == "IDLE"
        && previous_marker != "IDLE"
        && !previous_marker.ends_with("_RUNNING")
    {
        return None;
    }
    Some(format!(
        "status {} run={} from={} to={}",
        string(payload.get("plane")),
        display_ids.run(payload.get("run_id")),
        previous_marker,
        current_marker
    ))
}

fn render_usage_governance_paused(payload: &Map<String, Value>) -> String {
    format!(
        "governance pause source={} rule={} window={} observed={} threshold={} next_resume={}",
        string(payload.get("source")),
        string(payload.get("rule_id")),
        string(payload.get("window")),
        number_string(payload.get("observed")),
        number_string(payload.get("threshold")),
        string(payload.get("next_auto_resume_at"))
    )
}

fn render_usage_governance_blocked(payload: &Map<String, Value>) -> String {
    format!(
        "governance blocked source={} rule={} window={} observed={} threshold={} detail={}",
        string(payload.get("source")),
        string(payload.get("rule_id")),
        string(payload.get("window")),
        number_string(payload.get("observed")),
        number_string(payload.get("threshold")),
        string_or_default(payload.get("detail"), "none")
    )
}

fn render_usage_governance_resumed(payload: &Map<String, Value>) -> String {
    format!(
        "governance resume cleared_rules={}",
        string(payload.get("cleared_rules"))
    )
}

fn render_usage_governance_degraded(payload: &Map<String, Value>) -> String {
    format!(
        "governance degraded source={} policy={} detail={}",
        string(payload.get("source")),
        string(payload.get("policy")),
        string(payload.get("detail"))
    )
}

fn render_usage_governance_reconciled(payload: &Map<String, Value>) -> String {
    format!(
        "governance reconciled repaired={} ledger_entries={}",
        number_string(payload.get("repaired_count")),
        number_string(payload.get("ledger_entry_count"))
    )
}

fn seed_stage_started(
    payload: &Map<String, Value>,
    occurred_at: &Timestamp,
    run_state: &mut HashMap<(String, String), RunAggregate>,
) {
    let plane = string(payload.get("plane"));
    let run_id = string(payload.get("run_id"));
    let aggregate = run_state.entry((plane, run_id)).or_default();
    let Some(occurred_at) = parse_datetime(occurred_at.as_str()) else {
        return;
    };
    if aggregate
        .first_started_at
        .is_none_or(|started_at| occurred_at < started_at)
    {
        aggregate.first_started_at = Some(occurred_at);
    }
}

fn record_stage_completed(
    payload: &Map<String, Value>,
    run_state: &mut HashMap<(String, String), RunAggregate>,
    display_ids: &mut DisplayIdRegistry,
) -> String {
    let plane = string(payload.get("plane"));
    let run_id = string(payload.get("run_id"));
    let aggregate = run_state
        .entry((plane.clone(), run_id.clone()))
        .or_default();
    let started_at = datetime_value(payload.get("started_at"));
    let completed_at = datetime_value(payload.get("completed_at"));
    if let Some(started_at) = started_at {
        if aggregate
            .first_started_at
            .is_none_or(|existing| started_at < existing)
        {
            aggregate.first_started_at = Some(started_at);
        }
    }
    if let Some(completed_at) = completed_at {
        if aggregate
            .latest_completed_at
            .is_none_or(|existing| completed_at > existing)
        {
            aggregate.latest_completed_at = Some(completed_at);
        }
    } else {
        aggregate.fallback_elapsed_seconds += float_value(payload.get("duration_seconds"));
    }
    add_token_usage(aggregate, payload.get("token_usage"));

    let mut parts = vec![
        "run".to_owned(),
        plane,
        format!("run={}", display_ids.run(Some(&Value::String(run_id)))),
        format!(
            "elapsed={}",
            format_seconds(aggregate_elapsed_seconds(aggregate))
        ),
    ];
    if let Some(tokens) = format_aggregate_tokens(aggregate) {
        parts.push(format!("tokens={tokens}"));
    }
    parts.join(" ")
}

fn add_token_usage(aggregate: &mut RunAggregate, token_usage: Option<&Value>) {
    let Some(token_usage) = token_usage.and_then(Value::as_object) else {
        return;
    };
    aggregate.input_tokens += int_value(token_usage.get("input_tokens"));
    aggregate.cached_input_tokens += int_value(token_usage.get("cached_input_tokens"));
    aggregate.output_tokens += int_value(token_usage.get("output_tokens"));
    aggregate.thinking_tokens += int_value(token_usage.get("thinking_tokens"));
    aggregate.total_tokens += int_value(token_usage.get("total_tokens"));
    aggregate.has_known_tokens = true;
}

fn aggregate_elapsed_seconds(aggregate: &RunAggregate) -> f64 {
    if let (Some(started_at), Some(completed_at)) =
        (aggregate.first_started_at, aggregate.latest_completed_at)
    {
        return (completed_at - started_at).as_seconds_f64();
    }
    aggregate.fallback_elapsed_seconds
}

fn format_aggregate_tokens(aggregate: &RunAggregate) -> Option<String> {
    aggregate.has_known_tokens.then(|| {
        format!(
            "in={} cached={} out={} think={} total={}",
            aggregate.input_tokens,
            aggregate.cached_input_tokens,
            aggregate.output_tokens,
            aggregate.thinking_tokens,
            aggregate.total_tokens
        )
    })
}

fn format_token_usage(token_usage: Option<&Value>) -> Option<String> {
    let token_usage = token_usage.and_then(Value::as_object)?;
    Some(format!(
        "in={} cached={} out={} think={} total={}",
        int_value(token_usage.get("input_tokens")),
        int_value(token_usage.get("cached_input_tokens")),
        int_value(token_usage.get("output_tokens")),
        int_value(token_usage.get("thinking_tokens")),
        int_value(token_usage.get("total_tokens"))
    ))
}

fn should_render_idle(reason: &str, occurred_at: &Timestamp, idle_state: &IdleRenderState) -> bool {
    if idle_state.reason.as_deref() != Some(reason) {
        return true;
    }
    let Some(last_emitted_at) = idle_state.last_emitted_at else {
        return true;
    };
    let Some(occurred_at) = parse_datetime(occurred_at.as_str()) else {
        return true;
    };
    (occurred_at - last_emitted_at).as_seconds_f64() >= IDLE_HEARTBEAT_SECONDS
}

fn stage_ref_from_payload(payload: &Map<String, Value>) -> String {
    stage_ref(
        &string(payload.get("plane")),
        &string(payload.get("stage")),
        &string(payload.get("node_id")),
        &string(payload.get("stage_kind_id")),
    )
}

fn stage_ref(plane: &str, stage: &str, node_id: &str, stage_kind_id: &str) -> String {
    let mut parts = vec![format!("{plane}/{stage}")];
    if !matches!(node_id, "unknown") && node_id != stage {
        parts.push(format!("node={node_id}"));
    }
    if !matches!(stage_kind_id, "unknown") && stage_kind_id != stage && stage_kind_id != node_id {
        parts.push(format!("kind={stage_kind_id}"));
    }
    parts.join(" ")
}

fn route_target(payload: &Map<String, Value>) -> String {
    let next_stage = string(payload.get("next_stage"));
    let next_node = string(payload.get("next_node_id"));
    let next_kind = string(payload.get("next_stage_kind_id"));
    let current_plane = string(payload.get("plane"));
    let target_plane = plane_for_stage(&next_stage);
    let target = if target_plane
        .as_deref()
        .is_some_and(|target_plane| target_plane != current_plane)
    {
        format!("{}/{next_stage}", target_plane.unwrap())
    } else {
        next_stage.clone()
    };
    let mut extras = Vec::new();
    if !matches!(next_node.as_str(), "unknown") && next_node != next_stage {
        extras.push(format!("node={next_node}"));
    }
    if !matches!(next_kind.as_str(), "unknown") && next_kind != next_stage && next_kind != next_node
    {
        extras.push(format!("kind={next_kind}"));
    }
    if extras.is_empty() {
        target
    } else {
        format!("{target} {}", extras.join(" "))
    }
}

fn plane_for_stage(stage: &str) -> Option<String> {
    stage_metadata_for_value(stage)
        .ok()
        .map(|metadata| metadata.plane.as_str().to_owned())
}

fn work_ref(payload: &Map<String, Value>) -> Option<String> {
    let work_kind = optional_string(payload.get("work_item_kind"));
    let work_id = optional_string(payload.get("work_item_id"));
    match (work_kind, work_id) {
        (None, None) => None,
        (None, Some(work_id)) => Some(work_id),
        (Some(work_kind), None) => Some(work_kind),
        (Some(work_kind), Some(work_id)) => Some(format!("{work_kind}:{work_id}")),
    }
}

fn nonredundant_running_status(payload: &Map<String, Value>) -> Option<String> {
    let status = normalize_marker(&string(payload.get("status_marker")));
    let stage = string(payload.get("stage"));
    if status == "unknown" || status == format!("{}_RUNNING", stage.to_ascii_uppercase()) {
        None
    } else {
        Some(status)
    }
}

fn nonredundant_terminal_status(payload: &Map<String, Value>) -> Option<String> {
    let status = normalize_marker(&string(payload.get("summary_status_marker")));
    let terminal_result = string(payload.get("terminal_result"));
    if status == "unknown" || status == terminal_result {
        None
    } else {
        Some(status)
    }
}

fn run_handle_candidates(run_id: &str) -> Vec<String> {
    if !run_id.starts_with("run-") {
        return vec![run_id.to_owned()];
    }
    let suffix = run_id.trim_start_matches("run-");
    if suffix.len() <= RUN_HANDLE_LENGTHS[0] || !is_hex_string(suffix) {
        return vec![run_id.to_owned()];
    }
    let mut handles = RUN_HANDLE_LENGTHS
        .iter()
        .copied()
        .filter(|length| *length < suffix.len())
        .map(|length| suffix[..length].to_owned())
        .collect::<Vec<_>>();
    handles.push(run_id.to_owned());
    handles
}

fn is_hex_string(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|char| char.is_ascii_hexdigit())
}

fn format_concurrency_policy(policy: &Map<String, Value>) -> String {
    let exclusive = format_plane_groups(policy.get("mutually_exclusive_planes"));
    let concurrent = format_plane_groups(policy.get("may_run_concurrently"));
    let mut fragments = Vec::new();
    if !exclusive.is_empty() {
        fragments.push(format!("exclusive={exclusive}"));
    }
    if !concurrent.is_empty() {
        fragments.push(format!("concurrent={concurrent}"));
    }
    if fragments.is_empty() {
        "none".to_owned()
    } else {
        fragments.join(" ")
    }
}

fn format_plane_groups(value: Option<&Value>) -> String {
    let Some(groups) = value.and_then(Value::as_array) else {
        return String::new();
    };
    groups
        .iter()
        .filter_map(Value::as_array)
        .map(|group| {
            group
                .iter()
                .map(value_to_string)
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
                .join("+")
        })
        .filter(|group| !group.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

fn format_plane_mapping(mapping: &Map<String, Value>, normalize_markers: bool) -> String {
    let mut parts = Vec::new();
    for plane in ["execution", "planning", "learning"] {
        if let Some(value) = mapping.get(plane) {
            let mut text = value_to_string(value);
            if normalize_markers {
                text = normalize_marker(&text);
            }
            parts.push(format!("{plane}={text}"));
        }
    }
    let mut extra_keys = mapping
        .keys()
        .filter(|key| !matches!(key.as_str(), "execution" | "planning" | "learning"))
        .collect::<Vec<_>>();
    extra_keys.sort();
    for key in extra_keys {
        let mut text = value_to_string(&mapping[key]);
        if normalize_markers {
            text = normalize_marker(&text);
        }
        parts.push(format!("{key}={text}"));
    }
    parts.join(" ")
}

fn object_mapping(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value.and_then(Value::as_object)
}

fn plane_list(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_array)
        .map(|items| {
            let text = items
                .iter()
                .map(value_to_string)
                .collect::<Vec<_>>()
                .join(",");
            if text.is_empty() {
                "none".to_owned()
            } else {
                text
            }
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn normalize_marker(value: &str) -> String {
    value
        .strip_prefix("### ")
        .unwrap_or(value)
        .trim()
        .to_owned()
}

fn datetime_value(value: Option<&Value>) -> Option<OffsetDateTime> {
    optional_string(value).and_then(|value| parse_datetime(&value))
}

fn parse_datetime(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}

fn format_timestamp_hms(timestamp: &Timestamp) -> String {
    parse_datetime(timestamp.as_str())
        .map(|value| {
            format!(
                "{:02}:{:02}:{:02}",
                value.hour(),
                value.minute(),
                value.second()
            )
        })
        .unwrap_or_else(|| "00:00:00".to_owned())
}

fn format_seconds(value: f64) -> String {
    format!("{value:.1}s")
}

fn float_value(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0)
}

fn int_value(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

fn number_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(number)) => number.to_string(),
        Some(value) => value_to_string(value),
        None => "unknown".to_owned(),
    }
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.is_empty() => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(value) => {
            let text = value_to_string(value);
            (!text.is_empty()).then_some(text)
        }
    }
}

fn string(value: Option<&Value>) -> String {
    match optional_string(value) {
        Some(value) => value,
        None => "unknown".to_owned(),
    }
}

fn string_or_default(value: Option<&Value>, default: &str) -> String {
    match optional_string(value) {
        Some(value) => value,
        None => default.to_owned(),
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => "unknown".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn required_string(object: &Map<String, Value>, field: &str) -> Result<String, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("runtime event is missing string field `{field}`"))
}
