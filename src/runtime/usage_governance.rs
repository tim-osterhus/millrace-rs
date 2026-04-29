//! Usage-governance state, ledger, token-window, and quota evaluation helpers.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use time::{Duration, OffsetDateTime, Time, format_description::well_known::Rfc3339};

use crate::{
    contracts::{
        StageResultEnvelope, SubscriptionQuotaStatus, SubscriptionQuotaTelemetryState,
        SubscriptionQuotaWindowReading, Timestamp, UsageGovernanceBlocker,
        UsageGovernanceBlockerSource, UsageGovernanceDegradedPolicy,
        UsageGovernanceEvaluationBoundary, UsageGovernanceLedgerEntry,
        UsageGovernanceRuntimeTokenMetric, UsageGovernanceRuntimeTokenWindow, UsageGovernanceState,
        UsageGovernanceSubscriptionProvider, UsageGovernanceSubscriptionWindow,
    },
    workspace::{
        StateStoreError, StateStoreResult, WorkspacePaths, append_usage_governance_ledger_entry,
        load_usage_governance_ledger, save_usage_governance_state,
    },
};

const QUOTA_UNAVAILABLE_DETAIL: &str = "quota_telemetry_unavailable";

/// Runtime config for usage governance.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageGovernanceConfig {
    /// Enables usage-governance evaluation.
    pub enabled: bool,
    /// Allows governance-owned pauses to clear when blockers disappear.
    pub auto_resume: bool,
    /// Evaluation boundary.
    pub evaluation_boundary: UsageGovernanceEvaluationBoundary,
    /// Calendar timezone name preserved from config.
    pub calendar_timezone: String,
    /// Runtime token ledger rules.
    pub runtime_token_rules: RuntimeTokenRulesConfig,
    /// Subscription quota telemetry rules.
    pub subscription_quota_rules: SubscriptionQuotaRulesConfig,
}

impl Default for UsageGovernanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_resume: true,
            evaluation_boundary: UsageGovernanceEvaluationBoundary::BetweenStages,
            calendar_timezone: "UTC".to_owned(),
            runtime_token_rules: RuntimeTokenRulesConfig::default(),
            subscription_quota_rules: SubscriptionQuotaRulesConfig::default(),
        }
    }
}

/// Runtime token rule collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTokenRulesConfig {
    /// Enables runtime token ledger evaluation when top-level governance is enabled.
    pub enabled: bool,
    /// Ordered token rules.
    pub rules: Vec<RuntimeTokenRuleConfig>,
}

impl Default for RuntimeTokenRulesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rules: vec![
                RuntimeTokenRuleConfig {
                    rule_id: "rolling-5h-default".to_owned(),
                    window: UsageGovernanceRuntimeTokenWindow::Rolling5h,
                    metric: UsageGovernanceRuntimeTokenMetric::TotalTokens,
                    threshold: 750_000,
                },
                RuntimeTokenRuleConfig {
                    rule_id: "calendar-week-default".to_owned(),
                    window: UsageGovernanceRuntimeTokenWindow::CalendarWeek,
                    metric: UsageGovernanceRuntimeTokenMetric::TotalTokens,
                    threshold: 5_000_000,
                },
            ],
        }
    }
}

/// One runtime token rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTokenRuleConfig {
    /// Stable rule id projected into blockers.
    pub rule_id: String,
    /// Token window.
    pub window: UsageGovernanceRuntimeTokenWindow,
    /// Token metric.
    pub metric: UsageGovernanceRuntimeTokenMetric,
    /// Inclusive pause threshold.
    pub threshold: u64,
}

/// Subscription quota rule collection.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionQuotaRulesConfig {
    /// Enables subscription quota evaluation when top-level governance is enabled.
    pub enabled: bool,
    /// Quota provider id.
    pub provider: UsageGovernanceSubscriptionProvider,
    /// Degraded telemetry policy.
    pub degraded_policy: UsageGovernanceDegradedPolicy,
    /// Minimum telemetry refresh interval.
    pub refresh_interval_seconds: u64,
    /// Ordered quota rules.
    pub rules: Vec<SubscriptionQuotaRuleConfig>,
}

impl Default for SubscriptionQuotaRulesConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: UsageGovernanceSubscriptionProvider::CodexChatGptOauth,
            degraded_policy: UsageGovernanceDegradedPolicy::FailOpen,
            refresh_interval_seconds: 60,
            rules: vec![
                SubscriptionQuotaRuleConfig {
                    rule_id: "codex-five-hour-default".to_owned(),
                    window: UsageGovernanceSubscriptionWindow::FiveHour,
                    pause_at_percent_used: 95.0,
                },
                SubscriptionQuotaRuleConfig {
                    rule_id: "codex-weekly-default".to_owned(),
                    window: UsageGovernanceSubscriptionWindow::Weekly,
                    pause_at_percent_used: 95.0,
                },
            ],
        }
    }
}

/// One subscription quota threshold rule.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionQuotaRuleConfig {
    /// Stable rule id projected into blockers.
    pub rule_id: String,
    /// Quota window.
    pub window: UsageGovernanceSubscriptionWindow,
    /// Inclusive pause threshold.
    pub pause_at_percent_used: f64,
}

/// Builds an inert disabled state without writing workspace artifacts.
#[must_use]
pub fn disabled_usage_governance_state(now: Timestamp) -> UsageGovernanceState {
    UsageGovernanceState {
        version: "1.0".to_owned(),
        enabled: false,
        auto_resume: true,
        auto_resume_possible: true,
        evaluation_boundary: UsageGovernanceEvaluationBoundary::BetweenStages,
        calendar_timezone: "UTC".to_owned(),
        daemon_session_id: None,
        last_evaluated_at: now,
        active_blockers: Vec::new(),
        paused_by_governance: false,
        next_auto_resume_at: None,
        subscription_quota_status: SubscriptionQuotaStatus::default(),
    }
}

/// Evaluates governance contracts and persists governance state when enabled.
pub fn evaluate_usage_governance(
    paths: &WorkspacePaths,
    config: &UsageGovernanceConfig,
    now: Timestamp,
    daemon_session_id: Option<String>,
    paused_by_governance: bool,
    stage_result: Option<(&StageResultEnvelope, &Path)>,
    subscription_status: Option<SubscriptionQuotaStatus>,
) -> StateStoreResult<UsageGovernanceState> {
    if !config.enabled {
        let mut state = disabled_usage_governance_state(now);
        state.auto_resume = config.auto_resume;
        state.calendar_timezone = config.calendar_timezone.clone();
        state.daemon_session_id = daemon_session_id;
        if paths.usage_governance_state_file.exists() || paused_by_governance {
            save_usage_governance_state(paths, &state)?;
        }
        return Ok(state);
    }

    if let Some((stage_result, stage_result_path)) = stage_result {
        record_stage_result_usage(
            paths,
            config,
            stage_result,
            stage_result_path,
            now.clone(),
            daemon_session_id.as_deref(),
        )?;
    }
    reconcile_usage_ledger_from_stage_results(
        paths,
        config,
        now.clone(),
        daemon_session_id.as_deref(),
    )?;

    let ledger_entries = load_usage_governance_ledger(paths)?;
    let mut active_blockers = Vec::new();
    if config.runtime_token_rules.enabled {
        active_blockers.extend(evaluate_runtime_token_rules(
            &ledger_entries,
            &config.runtime_token_rules,
            &now,
            daemon_session_id.as_deref(),
            &config.calendar_timezone,
        )?);
    }

    let quota_status = match subscription_status {
        Some(status) => status,
        None => {
            subscription_quota_status_for_evaluation(paths, &config.subscription_quota_rules, &now)?
        }
    };
    active_blockers.extend(evaluate_subscription_quota_rules(
        &quota_status,
        &config.subscription_quota_rules,
    ));

    let auto_resume_possible = active_blockers
        .iter()
        .all(|blocker| blocker.auto_resume_possible);
    let next_auto_resume_at = next_auto_resume_at(&active_blockers)?;
    let has_active_blockers = !active_blockers.is_empty();
    let state = UsageGovernanceState {
        version: "1.0".to_owned(),
        enabled: true,
        auto_resume: config.auto_resume,
        auto_resume_possible,
        evaluation_boundary: config.evaluation_boundary,
        calendar_timezone: config.calendar_timezone.clone(),
        daemon_session_id,
        last_evaluated_at: now,
        active_blockers,
        paused_by_governance: paused_by_governance || has_active_blockers,
        next_auto_resume_at,
        subscription_quota_status: quota_status,
    };
    save_usage_governance_state(paths, &state)?;
    Ok(state)
}

/// Returns true when a stage result should be counted in the runtime token ledger.
#[must_use]
pub fn should_record_runtime_tokens(
    config: &UsageGovernanceConfig,
    stage_result: &StageResultEnvelope,
) -> bool {
    config.enabled && config.runtime_token_rules.enabled && stage_result.token_usage.is_some()
}

/// Records one stage result token usage entry idempotently.
pub fn record_stage_result_usage(
    paths: &WorkspacePaths,
    config: &UsageGovernanceConfig,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
    counted_at: Timestamp,
    daemon_session_id: Option<&str>,
) -> StateStoreResult<bool> {
    if !should_record_runtime_tokens(config, stage_result) {
        return Ok(false);
    }

    let dedupe_key = stage_result_dedupe_key(paths, stage_result_path);
    let existing_keys: BTreeSet<_> = load_usage_governance_ledger(paths)?
        .into_iter()
        .map(|entry| entry.dedupe_key)
        .collect();
    if existing_keys.contains(&dedupe_key) {
        return Ok(false);
    }

    let entry = ledger_entry_from_stage_result(
        paths,
        stage_result,
        stage_result_path,
        counted_at,
        daemon_session_id,
    )?;
    append_usage_governance_ledger_entry(paths, &entry)?;
    Ok(true)
}

/// Rebuilds missing ledger entries from persisted stage-result artifacts.
pub fn reconcile_usage_ledger_from_stage_results(
    paths: &WorkspacePaths,
    config: &UsageGovernanceConfig,
    counted_at: Timestamp,
    daemon_session_id: Option<&str>,
) -> StateStoreResult<usize> {
    if !config.enabled || !config.runtime_token_rules.enabled {
        return Ok(0);
    }

    let mut existing_keys: BTreeSet<_> = load_usage_governance_ledger(paths)?
        .into_iter()
        .map(|entry| entry.dedupe_key)
        .collect();
    let mut repaired = 0;
    for stage_result_path in stage_result_paths(paths) {
        let dedupe_key = stage_result_dedupe_key(paths, &stage_result_path);
        if existing_keys.contains(&dedupe_key) {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&stage_result_path) else {
            continue;
        };
        let Ok(stage_result) = StageResultEnvelope::from_json_str(&raw) else {
            continue;
        };
        if !should_record_runtime_tokens(config, &stage_result) {
            continue;
        }
        let entry = ledger_entry_from_stage_result(
            paths,
            &stage_result,
            &stage_result_path,
            counted_at.clone(),
            daemon_session_id,
        )?;
        append_usage_governance_ledger_entry(paths, &entry)?;
        existing_keys.insert(dedupe_key);
        repaired += 1;
    }
    Ok(repaired)
}

/// Builds one ledger entry from a token-bearing stage result.
#[must_use]
pub fn ledger_entry_from_stage_result(
    paths: &WorkspacePaths,
    stage_result: &StageResultEnvelope,
    stage_result_path: &Path,
    counted_at: Timestamp,
    daemon_session_id: Option<&str>,
) -> StateStoreResult<UsageGovernanceLedgerEntry> {
    let stage_result_path = stage_result_dedupe_key(paths, stage_result_path);
    let token_usage =
        stage_result
            .token_usage
            .clone()
            .ok_or_else(|| StateStoreError::StatusMarker {
                message: "stage result token_usage is required for ledger entries".to_owned(),
            })?;
    Ok(UsageGovernanceLedgerEntry {
        dedupe_key: stage_result_path.clone(),
        counted_at,
        stage_completed_at: stage_result.completed_at.clone(),
        plane: stage_result.plane,
        run_id: stage_result.run_id.clone(),
        stage_id: stage_result.stage_kind_id.clone(),
        work_item_kind: stage_result.work_item_kind,
        work_item_id: stage_result.work_item_id.clone(),
        token_usage,
        stage_result_path,
        daemon_session_id: daemon_session_id.map(str::to_owned),
    })
}

/// Stable ledger dedupe key for a stage-result path.
#[must_use]
pub fn stage_result_dedupe_key(paths: &WorkspacePaths, stage_result_path: &Path) -> String {
    let root = fs::canonicalize(&paths.root).unwrap_or_else(|_| paths.root.clone());
    let stage_result_path =
        fs::canonicalize(stage_result_path).unwrap_or_else(|_| stage_result_path.to_path_buf());
    stage_result_path
        .strip_prefix(&root)
        .unwrap_or(&stage_result_path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Evaluates configured runtime token rules against ledger entries.
pub fn evaluate_runtime_token_rules(
    entries: &[UsageGovernanceLedgerEntry],
    config: &RuntimeTokenRulesConfig,
    now: &Timestamp,
    daemon_session_id: Option<&str>,
    calendar_timezone: &str,
) -> StateStoreResult<Vec<UsageGovernanceBlocker>> {
    let now_time = parse_timestamp("last_evaluated_at", now)?;
    let mut blockers = Vec::new();
    for rule in &config.rules {
        let window_entries = entries_for_runtime_window(
            entries,
            rule.window,
            now_time,
            daemon_session_id,
            calendar_timezone,
        )?;
        let observed = observed_metric(&window_entries, rule.metric);
        if observed < rule.threshold {
            continue;
        }
        let next_resume = runtime_rule_next_resume(
            &window_entries,
            rule.window,
            rule.metric,
            rule.threshold,
            now_time,
            calendar_timezone,
        )?;
        blockers.push(UsageGovernanceBlocker {
            source: UsageGovernanceBlockerSource::RuntimeToken,
            rule_id: rule.rule_id.clone(),
            window: rule.window.as_str().to_owned(),
            observed: observed as f64,
            threshold: rule.threshold as f64,
            metric: Some(rule.metric),
            auto_resume_possible: matches!(
                rule.window,
                UsageGovernanceRuntimeTokenWindow::Rolling5h
                    | UsageGovernanceRuntimeTokenWindow::CalendarWeek
            ),
            next_auto_resume_at: next_resume,
            detail: String::new(),
        });
    }
    Ok(blockers)
}

/// Evaluates subscription quota telemetry into blockers.
#[must_use]
pub fn evaluate_subscription_quota_rules(
    status: &SubscriptionQuotaStatus,
    config: &SubscriptionQuotaRulesConfig,
) -> Vec<UsageGovernanceBlocker> {
    if !config.enabled {
        return Vec::new();
    }
    if status.state == SubscriptionQuotaTelemetryState::Degraded {
        if config.degraded_policy != UsageGovernanceDegradedPolicy::FailClosed {
            return Vec::new();
        }
        return vec![UsageGovernanceBlocker {
            source: UsageGovernanceBlockerSource::SubscriptionQuota,
            rule_id: "subscription-quota-degraded-fail-closed".to_owned(),
            window: "degraded".to_owned(),
            observed: 100.0,
            threshold: 100.0,
            metric: None,
            auto_resume_possible: false,
            next_auto_resume_at: None,
            detail: status
                .detail
                .clone()
                .unwrap_or_else(|| QUOTA_UNAVAILABLE_DETAIL.to_owned()),
        }];
    }

    let mut blockers = Vec::new();
    for rule in &config.rules {
        let Some(reading) = status.windows.get(&rule.window) else {
            continue;
        };
        if reading.percent_used < rule.pause_at_percent_used {
            continue;
        }
        blockers.push(UsageGovernanceBlocker {
            source: UsageGovernanceBlockerSource::SubscriptionQuota,
            rule_id: rule.rule_id.clone(),
            window: rule.window.as_str().to_owned(),
            observed: reading.percent_used,
            threshold: rule.pause_at_percent_used,
            metric: None,
            auto_resume_possible: reading.resets_at.is_some(),
            next_auto_resume_at: reading.resets_at.clone(),
            detail: String::new(),
        });
    }
    blockers
}

/// Fixture-friendly degraded quota status.
#[must_use]
pub fn subscription_quota_status_unavailable(
    config: &SubscriptionQuotaRulesConfig,
    now: Timestamp,
) -> SubscriptionQuotaStatus {
    if !config.enabled {
        return SubscriptionQuotaStatus {
            enabled: false,
            provider: config.provider,
            state: SubscriptionQuotaTelemetryState::Disabled,
            degraded_policy: Some(config.degraded_policy),
            detail: None,
            last_refreshed_at: None,
            windows: BTreeMap::new(),
        };
    }
    SubscriptionQuotaStatus {
        enabled: true,
        provider: config.provider,
        state: SubscriptionQuotaTelemetryState::Degraded,
        degraded_policy: Some(config.degraded_policy),
        detail: Some(QUOTA_UNAVAILABLE_DETAIL.to_owned()),
        last_refreshed_at: Some(now),
        windows: BTreeMap::new(),
    }
}

/// Fixture-friendly healthy quota status.
#[must_use]
pub fn healthy_subscription_quota_status(
    config: &SubscriptionQuotaRulesConfig,
    now: Timestamp,
    windows: BTreeMap<UsageGovernanceSubscriptionWindow, SubscriptionQuotaWindowReading>,
) -> SubscriptionQuotaStatus {
    SubscriptionQuotaStatus {
        enabled: config.enabled,
        provider: config.provider,
        state: if config.enabled {
            SubscriptionQuotaTelemetryState::Healthy
        } else {
            SubscriptionQuotaTelemetryState::Disabled
        },
        degraded_policy: Some(config.degraded_policy),
        detail: None,
        last_refreshed_at: Some(now),
        windows,
    }
}

fn subscription_quota_status_for_evaluation(
    paths: &WorkspacePaths,
    config: &SubscriptionQuotaRulesConfig,
    now: &Timestamp,
) -> StateStoreResult<SubscriptionQuotaStatus> {
    if !config.enabled {
        return Ok(subscription_quota_status_unavailable(config, now.clone()));
    }

    let previous = crate::workspace::load_usage_governance_state(paths)?;
    if let Some(last_refreshed_at) = previous
        .subscription_quota_status
        .last_refreshed_at
        .as_ref()
    {
        let now_time = parse_timestamp("last_evaluated_at", now)?;
        let refreshed_at = parse_timestamp(
            "subscription_quota_status.last_refreshed_at",
            last_refreshed_at,
        )?;
        let refresh_interval =
            Duration::seconds(config.refresh_interval_seconds.min(i64::MAX as u64) as i64);
        if now_time - refreshed_at < refresh_interval {
            return Ok(previous.subscription_quota_status);
        }
    }

    Ok(subscription_quota_status_unavailable(config, now.clone()))
}

/// Returns the earliest common auto-resume time for active blockers.
pub fn next_auto_resume_at(
    blockers: &[UsageGovernanceBlocker],
) -> StateStoreResult<Option<Timestamp>> {
    if blockers.is_empty() || blockers.iter().any(|blocker| !blocker.auto_resume_possible) {
        return Ok(None);
    }
    let mut candidates = Vec::new();
    for blocker in blockers {
        let Some(candidate) = &blocker.next_auto_resume_at else {
            return Ok(None);
        };
        candidates.push(parse_timestamp("next_auto_resume_at", candidate)?);
    }
    let Some(next_resume) = candidates.into_iter().min() else {
        return Ok(None);
    };
    Ok(Some(timestamp_from_offset(
        "next_auto_resume_at",
        next_resume,
    )?))
}

fn entries_for_runtime_window<'a>(
    entries: &'a [UsageGovernanceLedgerEntry],
    window: UsageGovernanceRuntimeTokenWindow,
    now: OffsetDateTime,
    daemon_session_id: Option<&str>,
    calendar_timezone: &str,
) -> StateStoreResult<Vec<&'a UsageGovernanceLedgerEntry>> {
    let cutoff = match window {
        UsageGovernanceRuntimeTokenWindow::Rolling5h => Some(now - Duration::hours(5)),
        UsageGovernanceRuntimeTokenWindow::CalendarWeek => {
            Some(calendar_week_start(now, calendar_timezone))
        }
        UsageGovernanceRuntimeTokenWindow::DaemonSession
        | UsageGovernanceRuntimeTokenWindow::PerRun => None,
    };

    let mut selected = Vec::new();
    for entry in entries {
        if window == UsageGovernanceRuntimeTokenWindow::DaemonSession
            && entry.daemon_session_id.as_deref() != daemon_session_id
        {
            continue;
        }
        if let Some(cutoff) = cutoff {
            let completed = parse_timestamp("stage_completed_at", &entry.stage_completed_at)?;
            if completed < cutoff {
                continue;
            }
        }
        selected.push(entry);
    }
    Ok(selected)
}

fn runtime_rule_next_resume(
    entries: &[&UsageGovernanceLedgerEntry],
    window: UsageGovernanceRuntimeTokenWindow,
    metric: UsageGovernanceRuntimeTokenMetric,
    threshold: u64,
    now: OffsetDateTime,
    calendar_timezone: &str,
) -> StateStoreResult<Option<Timestamp>> {
    match window {
        UsageGovernanceRuntimeTokenWindow::Rolling5h => {
            let mut ordered = entries.to_vec();
            ordered.sort_by_key(|entry| entry.stage_completed_at.as_str().to_owned());
            let mut remaining = observed_metric(entries, metric);
            for entry in ordered {
                remaining = remaining.saturating_sub(metric_value(entry, metric));
                let candidate = parse_timestamp("stage_completed_at", &entry.stage_completed_at)?
                    + Duration::hours(5);
                if remaining < threshold {
                    return Ok(Some(timestamp_from_offset(
                        "next_auto_resume_at",
                        candidate,
                    )?));
                }
            }
            Ok(None)
        }
        UsageGovernanceRuntimeTokenWindow::CalendarWeek => Ok(Some(timestamp_from_offset(
            "next_auto_resume_at",
            calendar_week_start(now, calendar_timezone) + Duration::days(7),
        )?)),
        UsageGovernanceRuntimeTokenWindow::DaemonSession
        | UsageGovernanceRuntimeTokenWindow::PerRun => Ok(None),
    }
}

fn observed_metric(
    entries: &[&UsageGovernanceLedgerEntry],
    metric: UsageGovernanceRuntimeTokenMetric,
) -> u64 {
    entries
        .iter()
        .map(|entry| metric_value(entry, metric))
        .sum()
}

fn metric_value(
    entry: &UsageGovernanceLedgerEntry,
    metric: UsageGovernanceRuntimeTokenMetric,
) -> u64 {
    match metric {
        UsageGovernanceRuntimeTokenMetric::TotalTokens => entry.token_usage.total_tokens,
    }
}

fn calendar_week_start(now: OffsetDateTime, _calendar_timezone: &str) -> OffsetDateTime {
    let days_from_monday = now.weekday().number_days_from_monday() as i64;
    now.replace_time(Time::MIDNIGHT) - Duration::days(days_from_monday)
}

fn stage_result_paths(paths: &WorkspacePaths) -> Vec<PathBuf> {
    let mut discovered = Vec::new();
    let Ok(run_dirs) = fs::read_dir(&paths.runs_dir) else {
        return discovered;
    };
    for run_dir in run_dirs.flatten() {
        let stage_results_dir = run_dir.path().join("stage_results");
        let Ok(entries) = fs::read_dir(stage_results_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
            {
                discovered.push(path);
            }
        }
    }
    discovered.sort();
    discovered
}

fn parse_timestamp(
    field_name: &'static str,
    timestamp: &Timestamp,
) -> StateStoreResult<OffsetDateTime> {
    OffsetDateTime::parse(timestamp.as_str(), &Rfc3339).map_err(|error| {
        StateStoreError::StatusMarker {
            message: format!("{field_name}: {error}"),
        }
    })
}

fn timestamp_from_offset(
    field_name: &'static str,
    timestamp: OffsetDateTime,
) -> StateStoreResult<Timestamp> {
    let rendered = timestamp
        .format(&Rfc3339)
        .map_err(|error| StateStoreError::StatusMarker {
            message: format!("{field_name}: {error}"),
        })?;
    Timestamp::parse(field_name, &rendered).map_err(|error| StateStoreError::StatusMarker {
        message: error.to_string(),
    })
}
