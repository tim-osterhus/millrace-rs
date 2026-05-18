//! Once-mode runtime startup lifecycle.

use std::{
    collections::{BTreeMap, HashMap},
    fmt, fs, io,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::UNIX_EPOCH,
};

use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    compiler::{
        CompileWorkspaceOptions, CompiledRunPlan, CompilerPersistenceError, FrozenGraphPlanePlan,
        GraphLoopEntryKey, MaterializedGraphNodePlan, compile_and_persist_workspace_plan_for_paths,
        load_persisted_compiled_plan,
    },
    contracts::{
        ActiveRunRequestKind, ActiveRunState, CapabilityPolicyDecision, Plane, RecoveryCounters,
        RuntimeMode, RuntimeSnapshot, StageName, Timestamp, UsageGovernanceDegradedPolicy,
        UsageGovernanceEvaluationBoundary, UsageGovernanceRuntimeTokenMetric,
        UsageGovernanceRuntimeTokenWindow, UsageGovernanceSubscriptionProvider,
        UsageGovernanceSubscriptionWindow, WatcherMode, WorkDocumentError, WorkItemKind,
        validate_capability_id, validate_safe_identifier,
    },
    runners::{
        CodexCliConfig, CodexCliRunnerAdapter, CodexPermissionLevel, PiEventLogPolicy, PiRpcConfig,
        PiRpcRunnerAdapter, RunnerRegistry, RunnerResult, StageRunnerDispatcher,
    },
    workspace::{
        QueueStoreError, RuntimeOwnershipLockError, RuntimeOwnershipLockOptions,
        RuntimeOwnershipRecord, StaleActiveState, StateStoreError, WorkspaceError, WorkspacePaths,
        detect_execution_stale_state, detect_learning_stale_state, detect_planning_stale_state,
        load_recovery_counters, load_snapshot, release_runtime_ownership_lock,
        require_initialized_workspace, require_initialized_workspace_paths, save_recovery_counters,
        save_snapshot,
    },
};

use super::usage_governance::{
    RuntimeTokenRuleConfig, RuntimeTokenRulesConfig, SubscriptionQuotaRuleConfig,
    SubscriptionQuotaRulesConfig, UsageGovernanceConfig,
};

static RUN_COUNTER: AtomicU64 = AtomicU64::new(0);

const STALE_ACTIVE_FAILURE_CLASS: &str = "stale_active_ownership";

/// Result type for once-mode runtime startup.
pub type RuntimeStartupResult<T> = Result<T, RuntimeStartupError>;

/// Caller inputs for the once-mode runtime startup boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStartupOptions {
    /// Optional requested runtime mode id. This mirrors `millrace run once --mode`.
    pub requested_mode_id: Option<String>,
    /// Optional runtime config path. This mirrors `millrace run once --config`.
    pub config_path: Option<PathBuf>,
    /// Runtime invocation mode projected into the startup snapshot.
    pub runtime_mode: RuntimeMode,
    /// Deterministic lock options for tests; defaults to current process metadata.
    pub lock_options: Option<RuntimeOwnershipLockOptions>,
    /// Deterministic timestamp for tests; defaults to current UTC time.
    pub now: Option<Timestamp>,
    /// Deterministic recovery run id for tests when startup reconciles stale active state.
    pub recovery_run_id: Option<String>,
}

impl Default for RuntimeStartupOptions {
    fn default() -> Self {
        Self {
            requested_mode_id: None,
            config_path: None,
            runtime_mode: RuntimeMode::Once,
            lock_options: None,
            now: None,
            recovery_run_id: None,
        }
    }
}

impl RuntimeStartupOptions {
    /// Build startup options for a selected mode id.
    #[must_use]
    pub fn for_mode(mode_id: impl Into<String>) -> Self {
        Self {
            requested_mode_id: Some(mode_id.into()),
            ..Self::default()
        }
    }

    /// Build startup options for daemon mode.
    #[must_use]
    pub fn daemon() -> Self {
        Self {
            runtime_mode: RuntimeMode::Daemon,
            ..Self::default()
        }
    }
}

/// Runtime config fields needed during startup before full daemon support exists.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeStartupConfig {
    /// Runtime config path used for startup.
    pub config_path: PathBuf,
    /// Configured default mode before `requested_mode_id` override resolution.
    pub default_mode: String,
    /// Configured runtime run style.
    pub run_style: RuntimeMode,
    /// Idle sleep duration used by daemon loops between empty ticks.
    pub idle_sleep_seconds: f64,
    /// Runtime runner adapter settings.
    pub runners: RuntimeRunnersConfig,
    /// Per-stage runtime config input, including compile-relevant stage identity fields.
    pub stages: BTreeMap<String, RuntimeStageConfig>,
    /// Whether watchers are enabled by config.
    pub watchers_enabled: bool,
    /// Poll watcher debounce window in milliseconds.
    pub watchers_debounce_ms: u64,
    /// Whether the daemon watcher should observe `ideas/inbox`.
    pub watchers_watch_ideas_inbox: bool,
    /// Whether the daemon watcher should observe queued specs.
    pub watchers_watch_specs_queue: bool,
    /// Runtime auto-recovery policy.
    pub auto_recovery: AutoRecoveryConfig,
    /// Whether usage governance is enabled in config.
    pub usage_governance_enabled: bool,
    /// Full usage-governance runtime config.
    pub usage_governance: UsageGovernanceConfig,
    /// Full execution-capability config defaults.
    pub execution_capabilities: ExecutionCapabilitiesConfig,
    /// Stable config content fingerprint projected into the snapshot.
    pub config_version: String,
}

/// Runtime-configured auto-recovery policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoRecoveryConfig {
    /// Whether automatic recovery behavior is enabled.
    pub enabled: bool,
    /// Whether blocked dependencies may be automatically retried.
    pub blocked_dependency_retry_enabled: bool,
    /// Maximum automatic requeues per work item before requiring operator action.
    pub max_auto_requeues_per_work_item: u64,
    /// Backoff windows used by blocked-dependency auto-recovery.
    pub cooldown_seconds: Vec<u64>,
}

impl Default for AutoRecoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            blocked_dependency_retry_enabled: true,
            max_auto_requeues_per_work_item: 3,
            cooldown_seconds: vec![300, 900, 3600],
        }
    }
}

/// Runtime config defaults for execution capability governance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionCapabilitiesConfig {
    /// Enables execution capability grant compilation and later enforcement.
    pub enabled: bool,
    /// Decision used for capability ids without an explicit default.
    pub default_unknown_capability: CapabilityPolicyDecision,
    /// Allows advisory-only grants when a runner or adapter cannot enforce directly.
    pub allow_advisory_grants: bool,
    /// Fails required advisory grants strictly during grant compilation.
    pub fail_required_advisory: bool,
    /// Capability-specific default policy decisions.
    pub defaults: BTreeMap<String, CapabilityPolicyDecision>,
}

impl Default for ExecutionCapabilitiesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_unknown_capability: CapabilityPolicyDecision::Deny,
            allow_advisory_grants: true,
            fail_required_advisory: false,
            defaults: BTreeMap::from([
                ("network.access".to_owned(), CapabilityPolicyDecision::Deny),
                (
                    "package.install".to_owned(),
                    CapabilityPolicyDecision::ApprovalRequired,
                ),
                (
                    "git.mutate".to_owned(),
                    CapabilityPolicyDecision::ApprovalRequired,
                ),
                ("shell.run".to_owned(), CapabilityPolicyDecision::Allow),
                (
                    "workspace.write".to_owned(),
                    CapabilityPolicyDecision::Allow,
                ),
            ]),
        }
    }
}

/// Reload/apply boundary for runtime config fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeConfigApplyBoundary {
    /// The field can apply synchronously.
    Immediate,
    /// The field applies on the next runtime tick.
    NextTick,
    /// The field requires recompiling runtime graph assets.
    Recompile,
    /// The field requires restarting the owning runtime.
    Restart,
}

impl RuntimeConfigApplyBoundary {
    /// Stable snake-case boundary token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::NextTick => "next_tick",
            Self::Recompile => "recompile",
            Self::Restart => "restart",
        }
    }
}

/// Returns the reload/apply boundary for one dotted runtime config field.
pub fn runtime_config_apply_boundary_for_field(
    field_path: &str,
) -> Result<RuntimeConfigApplyBoundary, String> {
    let boundary = match field_path {
        "runtime.default_mode" => RuntimeConfigApplyBoundary::Recompile,
        "runtime.run_style" | "runtime.idle_sleep_seconds" => RuntimeConfigApplyBoundary::NextTick,
        "runners.default_runner" | "runners.codex" | "runners.pi" => {
            RuntimeConfigApplyBoundary::NextTick
        }
        "recovery.max_fix_cycles"
        | "recovery.max_troubleshoot_attempts_before_consult"
        | "recovery.max_mechanic_attempts"
        | "recovery.stale_state_recovery_enabled"
        | "watchers.enabled"
        | "watchers.debounce_ms"
        | "watchers.watch_ideas_inbox"
        | "watchers.watch_specs_queue"
        | "usage_governance.enabled"
        | "usage_governance.auto_resume"
        | "usage_governance.evaluation_boundary"
        | "usage_governance.calendar_timezone"
        | "usage_governance.runtime_token_rules"
        | "usage_governance.subscription_quota_rules"
        | "auto_recovery.enabled"
        | "auto_recovery.blocked_dependency_retry_enabled"
        | "auto_recovery.max_auto_requeues_per_work_item"
        | "auto_recovery.cooldown_seconds" => RuntimeConfigApplyBoundary::NextTick,
        "execution_capabilities.enabled"
        | "execution_capabilities.default_unknown_capability"
        | "execution_capabilities.allow_advisory_grants"
        | "execution_capabilities.fail_required_advisory"
        | "execution_capabilities.defaults" => RuntimeConfigApplyBoundary::Recompile,
        _ if field_path.starts_with("stages.") => {
            let parts = field_path.split('.').collect::<Vec<_>>();
            if parts.len() != 3 {
                return Err(format!(
                    "No apply boundary declared for config field: {field_path}"
                ));
            }
            StageName::from_value(parts[1])
                .map_err(|_| format!("Unknown stage name in config field: {}", parts[1]))?;
            match parts[2] {
                "runner"
                | "model"
                | "thinking_level"
                | "model_reasoning_effort"
                | "timeout_seconds" => RuntimeConfigApplyBoundary::Recompile,
                _ => {
                    return Err(format!(
                        "No apply boundary declared for config field: {field_path}"
                    ));
                }
            }
        }
        _ => {
            return Err(format!(
                "No apply boundary declared for config field: {field_path}"
            ));
        }
    };
    Ok(boundary)
}

/// Runtime runner adapter settings loaded from `[runners]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRunnersConfig {
    /// Dispatcher fallback runner when a compiled request does not name one.
    pub default_runner: String,
    /// Codex CLI adapter settings.
    pub codex: CodexCliConfig,
    /// Pi RPC adapter settings.
    pub pi: PiRpcConfig,
}

impl Default for RuntimeRunnersConfig {
    fn default() -> Self {
        Self {
            default_runner: "codex_cli".to_owned(),
            codex: CodexCliConfig::default(),
            pi: PiRpcConfig::default(),
        }
    }
}

/// Per-stage config loaded from `[stages.<stage>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStageConfig {
    /// Optional runtime runner override.
    pub runner: Option<String>,
    /// Optional model override.
    pub model: Option<String>,
    /// Optional runner-neutral thinking level override.
    pub thinking_level: Option<String>,
    /// Optional Codex reasoning effort override.
    pub model_reasoning_effort: Option<String>,
    /// Stage timeout in seconds.
    pub timeout_seconds: u64,
}

impl Default for RuntimeStageConfig {
    fn default() -> Self {
        Self {
            runner: None,
            model: None,
            thinking_level: None,
            model_reasoning_effort: None,
            timeout_seconds: 3600,
        }
    }
}

/// One deterministic watcher target prepared during daemon startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWatcherTarget {
    /// Stable target label consumed by later watcher-intake code.
    pub target: String,
    /// Directory containing watched files.
    pub root: PathBuf,
    /// Glob-like filename pattern for watched files.
    pub pattern: String,
    /// True when existing files should be emitted on first poll.
    pub emit_existing_on_startup: bool,
}

/// One normalized watcher event emitted by deterministic poll fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWatchEvent {
    /// Stable target label that produced this event.
    pub target: String,
    /// Canonical file path observed by polling.
    pub path: PathBuf,
    /// Stable event kind. Poll fallback currently emits changed events only.
    pub event_kind: String,
    /// Runtime timestamp used for this poll.
    pub observed_at: Timestamp,
}

/// Poll fingerprint for one watched file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFileFingerprint {
    /// File modification timestamp in nanoseconds since the Unix epoch.
    pub modified_unix_nanos: u128,
    /// File size in bytes.
    pub size_bytes: u64,
}

/// Deterministic poll-mode watcher state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimePollWatcherState {
    /// Last observed file fingerprints keyed by canonical path.
    pub fingerprints: BTreeMap<String, RuntimeFileFingerprint>,
    /// Last event emission time keyed by target and canonical path.
    pub last_emitted_at_millis: BTreeMap<(String, String), i128>,
}

impl RuntimePollWatcherState {
    pub(crate) fn create(targets: &[RuntimeWatcherTarget]) -> Self {
        let mut state = Self::default();
        state.prime(targets);
        state
    }

    pub(crate) fn poll_once(
        &mut self,
        targets: &[RuntimeWatcherTarget],
        observed_at: Timestamp,
        observed_at_millis: i128,
        debounce_ms: u64,
    ) -> Vec<RuntimeWatchEvent> {
        let mut seen_paths = BTreeMap::new();
        let mut emitted = Vec::new();
        let mut ordered_targets = targets.to_vec();
        ordered_targets.sort_by(|left, right| {
            left.target
                .cmp(&right.target)
                .then_with(|| left.root.cmp(&right.root))
        });

        for target in ordered_targets {
            for (path, fingerprint) in target_files(&target) {
                let canonical = watch_path_key(&path);
                seen_paths.insert(canonical.clone(), ());
                if self.fingerprints.get(&canonical) == Some(&fingerprint) {
                    continue;
                }

                let debounce_key = (target.target.clone(), canonical.clone());
                if self
                    .last_emitted_at_millis
                    .get(&debounce_key)
                    .is_some_and(|last| {
                        observed_at_millis.saturating_sub(*last) < i128::from(debounce_ms)
                    })
                {
                    continue;
                }

                self.fingerprints.insert(canonical.clone(), fingerprint);
                self.last_emitted_at_millis
                    .insert(debounce_key, observed_at_millis);
                emitted.push(RuntimeWatchEvent {
                    target: target.target.clone(),
                    path,
                    event_kind: "changed".to_owned(),
                    observed_at: observed_at.clone(),
                });
            }
        }

        let stale_paths = self
            .fingerprints
            .keys()
            .filter(|path| !seen_paths.contains_key(*path))
            .cloned()
            .collect::<Vec<_>>();
        for stale_path in stale_paths {
            self.fingerprints.remove(&stale_path);
            self.last_emitted_at_millis
                .retain(|(_, path), _| path != &stale_path);
        }

        emitted.sort_by(|left, right| {
            left.target
                .cmp(&right.target)
                .then_with(|| left.path.cmp(&right.path))
        });
        emitted
    }

    fn prime(&mut self, targets: &[RuntimeWatcherTarget]) {
        for target in targets {
            if target.emit_existing_on_startup {
                continue;
            }
            for (path, fingerprint) in target_files(target) {
                self.fingerprints.insert(watch_path_key(&path), fingerprint);
            }
        }
    }
}

/// Startup-owned watcher session state without native filesystem watchers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWatcherSession {
    /// Effective watcher mode projected into the runtime snapshot.
    pub mode: WatcherMode,
    /// Deterministic ordered watch targets.
    pub targets: Vec<RuntimeWatcherTarget>,
    /// Debounce window in milliseconds.
    pub debounce_ms: u64,
    /// True when deterministic polling is available for this session.
    pub poll_fallback_ready: bool,
    /// Mutable deterministic poll fallback state.
    pub poller: Option<RuntimePollWatcherState>,
}

impl RuntimeWatcherSession {
    fn off(debounce_ms: u64) -> Self {
        Self {
            mode: WatcherMode::Off,
            targets: Vec::new(),
            debounce_ms,
            poll_fallback_ready: false,
            poller: None,
        }
    }
}

/// One deterministic reconciliation signal emitted at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeReconciliationSignal {
    /// Deterministic signal code.
    pub code: String,
    /// Recovery failure class.
    pub failure_class: String,
    /// Plane that owns the stale state when known.
    pub plane: Option<Plane>,
    /// Runtime stage prepared for later dispatch when available.
    pub recommended_stage: Option<StageName>,
    /// Human-readable signal detail.
    pub message: String,
}

/// Startup reconciliation evidence across queue surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStartupReconciliation {
    /// Execution active-state detection.
    pub execution: StaleActiveState,
    /// Planning active-state detection.
    pub planning: StaleActiveState,
    /// Learning active-state detection.
    pub learning: StaleActiveState,
    /// Startup-level reconciliation signals applied to snapshot state.
    pub signals: Vec<RuntimeReconciliationSignal>,
}

/// Open startup session holding runtime ownership.
#[derive(Debug, Clone)]
pub struct RuntimeStartupSession {
    /// Resolved initialized workspace paths.
    pub paths: WorkspacePaths,
    /// Startup config projection.
    pub config: RuntimeStartupConfig,
    /// Persisted compiled plan used as runtime authority.
    pub compiled_plan: CompiledRunPlan,
    /// Loaded and startup-updated runtime snapshot.
    pub snapshot: RuntimeSnapshot,
    /// Loaded and possibly startup-updated recovery counters.
    pub counters: RecoveryCounters,
    /// Runtime ownership record held by this session.
    pub lock_record: RuntimeOwnershipRecord,
    /// Watcher state prepared for daemon mode or disabled for once mode.
    pub watcher_session: RuntimeWatcherSession,
    /// Reconciliation evidence collected during startup.
    pub reconciliation: RuntimeStartupReconciliation,
    closed: bool,
}

impl RuntimeStartupSession {
    /// Rebuild watcher state from the session config without starting native watchers.
    pub fn rebuild_watcher_session(&mut self) -> RuntimeStartupResult<()> {
        self.watcher_session =
            build_runtime_watcher_session(&self.paths, &self.config, self.snapshot.runtime_mode);
        self.snapshot.watcher_mode = self.watcher_session.mode;
        save_snapshot(&self.paths, &self.snapshot)?;
        Ok(())
    }

    /// Close watcher resources tracked by the startup boundary.
    pub fn close_watcher_session(&mut self) {
        self.watcher_session = RuntimeWatcherSession::off(self.config.watchers_debounce_ms);
    }

    /// Mark the startup session closed and release runtime ownership.
    pub fn close(&mut self) -> RuntimeStartupResult<bool> {
        if self.closed {
            return Ok(false);
        }

        let now = utc_now_timestamp("updated_at")?;
        let save_result = if self.snapshot.process_running {
            self.snapshot.process_running = false;
            self.snapshot.updated_at = now;
            save_snapshot(&self.paths, &self.snapshot)
        } else {
            Ok(())
        };
        let release_result = release_runtime_ownership_lock(
            &self.paths,
            Some(&self.lock_record.owner_session_id),
            false,
        );

        save_result?;
        let released = release_result?;
        self.close_watcher_session();
        self.closed = true;
        Ok(released)
    }

    /// Consume the session after normal once-mode completion and release ownership.
    pub fn finish(mut self) -> RuntimeStartupResult<bool> {
        self.close()
    }
}

/// Failures produced by once-mode runtime startup.
#[derive(Debug)]
pub enum RuntimeStartupError {
    /// Workspace initialization or path validation failed.
    Workspace(WorkspaceError),
    /// Runtime config loading failed.
    Config {
        /// Config path being read.
        path: PathBuf,
        /// Dotted field path when known.
        field: Option<String>,
        /// Human-readable failure reason.
        message: String,
    },
    /// Compiler facade failed.
    Compiler(CompilerPersistenceError),
    /// Compiler completed without a usable active plan.
    MissingActiveCompiledPlan {
        /// Path where diagnostics would be stored when persistence is enabled.
        diagnostics_path: PathBuf,
        /// Compile diagnostics errors.
        errors: Vec<String>,
    },
    /// Persisted compiled-plan authority was absent after a successful compile path.
    MissingPersistedCompiledPlan {
        /// Expected persisted plan path.
        path: PathBuf,
    },
    /// Runtime ownership lock acquisition or release failed.
    RuntimeLock(RuntimeOwnershipLockError),
    /// Runtime snapshot or counter state failed to load or persist.
    StateStore(StateStoreError),
    /// Queue active-state detection failed.
    Queue(QueueStoreError),
    /// A deterministic timestamp could not be produced.
    Time {
        /// Timestamp field being built.
        field_name: &'static str,
        /// Human-readable failure reason.
        message: String,
    },
}

impl fmt::Display for RuntimeStartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(error) => write!(f, "{error}"),
            Self::Config {
                path,
                field,
                message,
            } => {
                if let Some(field) = field {
                    write!(
                        f,
                        "runtime config field {field} is invalid in {}: {message}",
                        path.display()
                    )
                } else {
                    write!(f, "runtime config error at {}: {message}", path.display())
                }
            }
            Self::Compiler(error) => write!(f, "{error}"),
            Self::MissingActiveCompiledPlan {
                diagnostics_path,
                errors,
            } => {
                let joined = if errors.is_empty() {
                    "compile failed".to_owned()
                } else {
                    errors.join(", ")
                };
                write!(
                    f,
                    "startup could not load an active compiled plan; diagnostics path {}: {joined}",
                    diagnostics_path.display()
                )
            }
            Self::MissingPersistedCompiledPlan { path } => {
                write!(
                    f,
                    "startup compiled successfully but persisted compiled plan is missing at {}",
                    path.display()
                )
            }
            Self::RuntimeLock(error) => write!(f, "{error}"),
            Self::StateStore(error) => write!(f, "{error}"),
            Self::Queue(error) => write!(f, "{error}"),
            Self::Time {
                field_name,
                message,
            } => write!(f, "failed to build timestamp {field_name}: {message}"),
        }
    }
}

impl std::error::Error for RuntimeStartupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::Compiler(error) => Some(error),
            Self::RuntimeLock(error) => Some(error),
            Self::StateStore(error) => Some(error),
            Self::Queue(error) => Some(error),
            Self::Config { .. }
            | Self::MissingActiveCompiledPlan { .. }
            | Self::MissingPersistedCompiledPlan { .. }
            | Self::Time { .. } => None,
        }
    }
}

impl From<WorkspaceError> for RuntimeStartupError {
    fn from(value: WorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

impl From<CompilerPersistenceError> for RuntimeStartupError {
    fn from(value: CompilerPersistenceError) -> Self {
        Self::Compiler(value)
    }
}

impl From<RuntimeOwnershipLockError> for RuntimeStartupError {
    fn from(value: RuntimeOwnershipLockError) -> Self {
        Self::RuntimeLock(value)
    }
}

impl From<StateStoreError> for RuntimeStartupError {
    fn from(value: StateStoreError) -> Self {
        Self::StateStore(value)
    }
}

impl From<QueueStoreError> for RuntimeStartupError {
    fn from(value: QueueStoreError) -> Self {
        Self::Queue(value)
    }
}

/// Start a once-mode runtime session for an initialized workspace root.
pub fn startup_runtime_once(
    root: impl AsRef<Path>,
    options: RuntimeStartupOptions,
) -> RuntimeStartupResult<RuntimeStartupSession> {
    let paths = require_initialized_workspace(root)?;
    startup_runtime_once_for_paths(&paths, options)
}

/// Start a daemon runtime session for an initialized workspace root.
pub fn startup_runtime_daemon(
    root: impl AsRef<Path>,
    mut options: RuntimeStartupOptions,
) -> RuntimeStartupResult<RuntimeStartupSession> {
    options.runtime_mode = RuntimeMode::Daemon;
    let paths = require_initialized_workspace(root)?;
    startup_runtime_daemon_for_paths(&paths, options)
}

/// Start a daemon runtime session for already resolved workspace paths.
pub fn startup_runtime_daemon_for_paths(
    paths: &WorkspacePaths,
    mut options: RuntimeStartupOptions,
) -> RuntimeStartupResult<RuntimeStartupSession> {
    options.runtime_mode = RuntimeMode::Daemon;
    startup_runtime_once_for_paths(paths, options)
}

/// Start a once-mode runtime session for already resolved workspace paths.
pub fn startup_runtime_once_for_paths(
    paths: &WorkspacePaths,
    options: RuntimeStartupOptions,
) -> RuntimeStartupResult<RuntimeStartupSession> {
    require_initialized_workspace_paths(paths)?;
    let now = match &options.now {
        Some(now) => now.clone(),
        None => utc_now_timestamp("updated_at")?,
    };
    let config_path = options
        .config_path
        .clone()
        .unwrap_or_else(|| paths.runtime_config_file.clone());
    let config = load_runtime_startup_config(&config_path)?;
    let lock_options = options
        .lock_options
        .clone()
        .unwrap_or_else(RuntimeOwnershipLockOptions::current);
    let lock_record =
        crate::workspace::acquire_runtime_ownership_lock_with_options(paths, lock_options)?;

    let result = startup_after_lock(paths, options, config, now, lock_record.clone());
    match result {
        Ok(session) => Ok(session),
        Err(error) => {
            let _ =
                release_runtime_ownership_lock(paths, Some(&lock_record.owner_session_id), false);
            Err(error)
        }
    }
}

/// Load startup config from a TOML file without mutating workspace state.
pub fn load_runtime_startup_config(path: &Path) -> RuntimeStartupResult<RuntimeStartupConfig> {
    let mut default_mode = "default_codex".to_owned();
    let mut run_style = RuntimeMode::Daemon;
    let mut idle_sleep_seconds = 1.0;
    let mut runners = RuntimeRunnersConfig::default();
    let mut stages = BTreeMap::new();
    let mut watchers_enabled = true;
    let mut watchers_debounce_ms = 250;
    let mut watchers_watch_ideas_inbox = true;
    let mut watchers_watch_specs_queue = true;
    let mut auto_recovery = AutoRecoveryConfig::default();
    let mut usage_governance = UsageGovernanceConfig::default();
    let mut execution_capabilities = ExecutionCapabilitiesConfig::default();

    let raw = match fs::read_to_string(path) {
        Ok(raw) => Some(raw),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(RuntimeStartupError::Config {
                path: path.to_path_buf(),
                field: None,
                message: error.to_string(),
            });
        }
    };

    if let Some(raw) = &raw {
        let value: toml::Value =
            toml::from_str(raw).map_err(|error| RuntimeStartupError::Config {
                path: path.to_path_buf(),
                field: None,
                message: error.to_string(),
            })?;
        let root = value
            .as_table()
            .ok_or_else(|| config_error(path, "<root>", "must be a TOML table"))?;
        if let Some(runtime) = child_table(root, "runtime", path)? {
            if let Some(value) = optional_string(runtime, "default_mode", path)? {
                default_mode = value;
            }
            if let Some(value) = optional_string(runtime, "run_style", path)? {
                run_style = RuntimeMode::from_value(&value).map_err(|_| {
                    config_error(path, "runtime.run_style", "must be `once` or `daemon`")
                })?;
            }
            if let Some(value) = optional_positive_f64(
                runtime,
                "idle_sleep_seconds",
                "runtime.idle_sleep_seconds",
                path,
            )? {
                idle_sleep_seconds = value;
            }
        }
        if let Some(runners_table) = child_table(root, "runners", path)? {
            if let Some(default_runner) = optional_string_at(
                runners_table,
                "default_runner",
                "runners.default_runner",
                path,
            )? {
                runners.default_runner =
                    normalize_runner_config_name(&default_runner, "runners.default_runner", path)?;
            }
            if let Some(codex) = child_table_at(runners_table, "codex", "runners.codex", path)? {
                load_codex_runner_config(codex, &mut runners.codex, path)?;
            }
            if let Some(pi) = child_table_at(runners_table, "pi", "runners.pi", path)? {
                load_pi_runner_config(pi, &mut runners.pi, path)?;
            }
        }
        if let Some(watchers) = child_table(root, "watchers", path)? {
            if let Some(value) = optional_bool(watchers, "enabled", path)? {
                watchers_enabled = value;
            }
            if let Some(value) =
                optional_positive_u64(watchers, "debounce_ms", "watchers.debounce_ms", path)?
            {
                watchers_debounce_ms = value;
            }
            if let Some(value) = optional_bool(watchers, "watch_ideas_inbox", path)? {
                watchers_watch_ideas_inbox = value;
            }
            if let Some(value) = optional_bool(watchers, "watch_specs_queue", path)? {
                watchers_watch_specs_queue = value;
            }
        }
        if let Some(auto_recovery_table) =
            child_table_at(root, "auto_recovery", "auto_recovery", path)?
        {
            load_auto_recovery_config(auto_recovery_table, &mut auto_recovery, path)?;
        }
        if let Some(usage_governance_table) =
            child_table_at(root, "usage_governance", "usage_governance", path)?
        {
            load_usage_governance_config(usage_governance_table, &mut usage_governance, path)?;
        }
        if let Some(execution_capabilities_table) = child_table_at(
            root,
            "execution_capabilities",
            "execution_capabilities",
            path,
        )? {
            load_execution_capabilities_config(
                execution_capabilities_table,
                &mut execution_capabilities,
                path,
            )?;
        }
        if let Some(stages_table) = child_table(root, "stages", path)? {
            stages = load_stage_configs(stages_table, path)?;
        }
    }

    let config_version = config_fingerprint(raw.as_deref().unwrap_or_default());
    Ok(RuntimeStartupConfig {
        config_path: path.to_path_buf(),
        default_mode,
        run_style,
        idle_sleep_seconds,
        runners,
        stages,
        watchers_enabled,
        watchers_debounce_ms,
        watchers_watch_ideas_inbox,
        watchers_watch_specs_queue,
        auto_recovery,
        usage_governance_enabled: usage_governance.enabled,
        usage_governance,
        execution_capabilities,
        config_version,
    })
}

/// Build the runtime-configured dispatcher used by operator serial and daemon paths.
pub fn build_runtime_runner_dispatcher(
    session: &RuntimeStartupSession,
) -> RunnerResult<StageRunnerDispatcher> {
    build_runtime_runner_dispatcher_for_paths(&session.paths, &session.config)
}

/// Build the runtime-configured dispatcher from explicit workspace paths and config.
pub fn build_runtime_runner_dispatcher_for_paths(
    paths: &WorkspacePaths,
    config: &RuntimeStartupConfig,
) -> RunnerResult<StageRunnerDispatcher> {
    let mut registry = RunnerRegistry::new();
    registry.register(
        "codex_cli",
        CodexCliRunnerAdapter::new(config.runners.codex.clone(), paths.root.clone()),
    )?;
    registry.register(
        "pi_rpc",
        PiRpcRunnerAdapter::new(config.runners.pi.clone(), paths.root.clone()),
    )?;
    Ok(StageRunnerDispatcher::with_default_runner(
        registry,
        config.runners.default_runner.clone(),
    ))
}

/// Build deterministic watcher startup state without starting native filesystem watchers.
#[must_use]
pub fn build_runtime_watcher_session(
    paths: &WorkspacePaths,
    config: &RuntimeStartupConfig,
    runtime_mode: RuntimeMode,
) -> RuntimeWatcherSession {
    if runtime_mode != RuntimeMode::Daemon || !config.watchers_enabled {
        return RuntimeWatcherSession::off(config.watchers_debounce_ms);
    }

    let mut targets = Vec::new();
    targets.push(RuntimeWatcherTarget {
        target: "config".to_owned(),
        root: config
            .config_path
            .parent()
            .unwrap_or(&paths.runtime_root)
            .to_path_buf(),
        pattern: config
            .config_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("millrace.toml")
            .to_owned(),
        emit_existing_on_startup: false,
    });
    targets.push(RuntimeWatcherTarget {
        target: "tasks_queue".to_owned(),
        root: paths.tasks_queue_dir.clone(),
        pattern: "*.md".to_owned(),
        emit_existing_on_startup: false,
    });
    if config.watchers_watch_specs_queue {
        targets.push(RuntimeWatcherTarget {
            target: "specs_queue".to_owned(),
            root: paths.specs_queue_dir.clone(),
            pattern: "*.md".to_owned(),
            emit_existing_on_startup: false,
        });
    }
    if config.watchers_watch_ideas_inbox {
        targets.push(RuntimeWatcherTarget {
            target: "ideas_inbox".to_owned(),
            root: paths.root.join("ideas").join("inbox"),
            pattern: "*.md".to_owned(),
            emit_existing_on_startup: true,
        });
    }

    let poller = RuntimePollWatcherState::create(&targets);

    RuntimeWatcherSession {
        mode: WatcherMode::Poll,
        targets,
        debounce_ms: config.watchers_debounce_ms,
        poll_fallback_ready: true,
        poller: Some(poller),
    }
}

fn target_files(target: &RuntimeWatcherTarget) -> Vec<(PathBuf, RuntimeFileFingerprint)> {
    if !target.root.is_dir() {
        return Vec::new();
    }

    let Ok(entries) = fs::read_dir(&target.root) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !watch_target_matches(target, &path) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Some(fingerprint) = fingerprint_for_metadata(&metadata) else {
            continue;
        };
        let path = fs::canonicalize(&path).unwrap_or(path);
        files.push((path, fingerprint));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn watch_target_matches(target: &RuntimeWatcherTarget, path: &Path) -> bool {
    if let Some(extension) = target.pattern.strip_prefix("*.") {
        return path.extension().and_then(|value| value.to_str()) == Some(extension);
    }
    path.file_name().and_then(|value| value.to_str()) == Some(target.pattern.as_str())
}

fn fingerprint_for_metadata(metadata: &fs::Metadata) -> Option<RuntimeFileFingerprint> {
    let modified = metadata.modified().ok()?;
    let modified_unix_nanos = modified
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    Some(RuntimeFileFingerprint {
        modified_unix_nanos,
        size_bytes: metadata.len(),
    })
}

fn watch_path_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Return the materialized node authority for a work item entry key.
#[must_use]
pub fn compiled_entry_node_for_work_item(
    plan: &CompiledRunPlan,
    work_item_kind: WorkItemKind,
) -> Option<&MaterializedGraphNodePlan> {
    let (plane, entry_key) = match work_item_kind {
        WorkItemKind::Task => (Plane::Execution, GraphLoopEntryKey::Task),
        WorkItemKind::Probe => (Plane::Planning, GraphLoopEntryKey::Probe),
        WorkItemKind::Spec => (Plane::Planning, GraphLoopEntryKey::Spec),
        WorkItemKind::Incident => (Plane::Planning, GraphLoopEntryKey::Incident),
        WorkItemKind::LearningRequest => (Plane::Learning, GraphLoopEntryKey::LearningRequest),
    };
    let graph = graph_for_plane(plan, plane)?;
    let entry = graph
        .compiled_entries
        .iter()
        .find(|entry| entry.entry_key == entry_key)?;
    graph
        .nodes
        .iter()
        .find(|node| node.node_id == entry.node_id)
}

fn startup_after_lock(
    paths: &WorkspacePaths,
    options: RuntimeStartupOptions,
    config: RuntimeStartupConfig,
    now: Timestamp,
    lock_record: RuntimeOwnershipRecord,
) -> RuntimeStartupResult<RuntimeStartupSession> {
    let compiler_options = CompileWorkspaceOptions {
        requested_mode_id: options.requested_mode_id.clone(),
        compiled_at: Some(now.clone()),
        compile_if_needed: true,
        refuse_stale_last_known_good: true,
        config_path: Some(config.config_path.clone()),
        persist_failure_diagnostics: false,
    };
    let compile_outcome = compile_and_persist_workspace_plan_for_paths(paths, compiler_options)?;
    if compile_outcome.active_plan.is_none() {
        return Err(RuntimeStartupError::MissingActiveCompiledPlan {
            diagnostics_path: paths.compile_diagnostics_file.clone(),
            errors: compile_outcome.diagnostics.errors,
        });
    }
    let compiled_plan = load_persisted_compiled_plan(paths)?.ok_or_else(|| {
        RuntimeStartupError::MissingPersistedCompiledPlan {
            path: paths.compiled_plan_file.clone(),
        }
    })?;

    let mut snapshot = load_snapshot(paths)?;
    let mut counters = load_recovery_counters(paths)?;
    let mut reconciliation =
        detect_startup_reconciliation(paths, &snapshot, &compiled_plan, &counters)?;
    let watcher_session = build_runtime_watcher_session(paths, &config, options.runtime_mode);
    let counters_changed = apply_startup_reconciliation(
        &compiled_plan,
        &mut snapshot,
        &mut counters,
        &mut reconciliation,
        &now,
        options.recovery_run_id.as_deref(),
    );

    project_startup_snapshot(
        paths,
        &config,
        &compiled_plan,
        &watcher_session,
        &mut snapshot,
        options.runtime_mode,
        &now,
    )?;
    if counters_changed {
        save_recovery_counters(paths, &counters)?;
    }
    save_snapshot(paths, &snapshot)?;

    Ok(RuntimeStartupSession {
        paths: paths.clone(),
        config,
        compiled_plan,
        snapshot,
        counters,
        lock_record,
        watcher_session,
        reconciliation,
        closed: false,
    })
}

fn detect_startup_reconciliation(
    paths: &WorkspacePaths,
    snapshot: &RuntimeSnapshot,
    compiled_plan: &CompiledRunPlan,
    counters: &RecoveryCounters,
) -> RuntimeStartupResult<RuntimeStartupReconciliation> {
    let execution_snapshot = snapshot_active_work_item_for_plane(snapshot, Plane::Execution)
        .and_then(|(kind, id)| (kind == WorkItemKind::Task).then_some(id));
    let planning_snapshot = snapshot_active_work_item_for_plane(snapshot, Plane::Planning)
        .and_then(|(kind, id)| {
            matches!(kind, WorkItemKind::Spec | WorkItemKind::Incident).then_some((kind, id))
        });
    let learning_snapshot = snapshot_active_work_item_for_plane(snapshot, Plane::Learning)
        .and_then(|(kind, id)| (kind == WorkItemKind::LearningRequest).then_some(id));

    let mut signals = Vec::new();
    if snapshot.active_stage.is_some() && !snapshot.process_running {
        let plane = snapshot.active_plane;
        let recommended_stage =
            plane.and_then(|plane| stale_active_recovery_stage(plane, snapshot, counters));
        signals.push(RuntimeReconciliationSignal {
            code: STALE_ACTIVE_FAILURE_CLASS.to_owned(),
            failure_class: STALE_ACTIVE_FAILURE_CLASS.to_owned(),
            plane,
            recommended_stage,
            message: "runtime snapshot has active ownership while process is not running"
                .to_owned(),
        });
    }

    let mut reconciliation = RuntimeStartupReconciliation {
        execution: detect_execution_stale_state(paths, execution_snapshot.as_deref())?,
        planning: match planning_snapshot {
            Some((kind, id)) => detect_planning_stale_state(paths, Some(kind), Some(&id))?,
            None => detect_planning_stale_state(paths, None, None)?,
        },
        learning: detect_learning_stale_state(paths, learning_snapshot.as_deref())?,
        signals,
    };

    if compiled_entry_node_for_work_item(compiled_plan, WorkItemKind::Task).is_none() {
        reconciliation
            .execution
            .reasons
            .push("compiled_entry_missing".to_owned());
        reconciliation.execution.is_stale = true;
    }

    Ok(reconciliation)
}

fn apply_startup_reconciliation(
    compiled_plan: &CompiledRunPlan,
    snapshot: &mut RuntimeSnapshot,
    counters: &mut RecoveryCounters,
    reconciliation: &mut RuntimeStartupReconciliation,
    now: &Timestamp,
    recovery_run_id: Option<&str>,
) -> bool {
    let Some(signal) = reconciliation.signals.first().cloned() else {
        return false;
    };
    let Some(plane) = signal.plane else {
        return false;
    };
    let Some(recovery_stage) = signal.recommended_stage else {
        return false;
    };
    let (node_id, stage_kind_id) = stage_identity(compiled_plan, plane, recovery_stage);
    let Some(active_run) = active_run_for_plane(snapshot, plane) else {
        return false;
    };

    let run_id = recovery_run_id
        .map(ToOwned::to_owned)
        .unwrap_or_else(new_recovery_run_id);
    let mut updated_active_run = active_run.clone();
    updated_active_run.stage = recovery_stage;
    updated_active_run.node_id = node_id;
    updated_active_run.stage_kind_id = stage_kind_id;
    updated_active_run.run_id = run_id;
    updated_active_run.active_since = now.clone();
    updated_active_run.running_status_marker = None;
    snapshot
        .active_runs_by_plane
        .insert(plane, updated_active_run);
    project_foreground_active_run(snapshot);
    snapshot.current_failure_class = Some(signal.failure_class);
    snapshot.updated_at = now.clone();

    increment_recovery_counter_for_stage(snapshot, counters, recovery_stage, now)
}

fn project_startup_snapshot(
    paths: &WorkspacePaths,
    config: &RuntimeStartupConfig,
    compiled_plan: &CompiledRunPlan,
    watcher_session: &RuntimeWatcherSession,
    snapshot: &mut RuntimeSnapshot,
    runtime_mode: RuntimeMode,
    now: &Timestamp,
) -> RuntimeStartupResult<()> {
    let execution_depth = count_markdown_files(&paths.tasks_queue_dir)?;
    let planning_depth = count_markdown_files(&paths.probes_queue_dir)?
        + count_markdown_files(&paths.specs_queue_dir)?
        + count_markdown_files(&paths.incidents_incoming_dir)?;
    let learning_depth = count_markdown_files(&paths.learning_requests_queue_dir)?;

    snapshot.runtime_mode = runtime_mode;
    snapshot.process_running = true;
    snapshot.active_mode_id = compiled_plan.mode_id.clone();
    snapshot.execution_loop_id = compiled_plan.execution_loop_id.clone();
    snapshot.planning_loop_id = compiled_plan.planning_loop_id.clone();
    snapshot.learning_loop_id = compiled_plan.learning_loop_id.clone();
    snapshot.loop_ids_by_plane = compiled_plan.loop_ids_by_plane.clone();
    snapshot.compiled_plan_id = compiled_plan.compiled_plan_id.clone();
    snapshot.compiled_plan_path = workspace_relative_path(paths, &paths.compiled_plan_file);
    snapshot.queue_depth_execution = execution_depth;
    snapshot.queue_depth_planning = planning_depth;
    snapshot.queue_depth_learning = learning_depth;
    snapshot.queue_depths_by_plane = HashMap::from([
        (Plane::Execution, execution_depth),
        (Plane::Planning, planning_depth),
        (Plane::Learning, learning_depth),
    ]);
    snapshot.config_version = config.config_version.clone();
    snapshot.watcher_mode = watcher_session.mode;
    snapshot.last_reload_outcome = None;
    snapshot.last_reload_error = None;
    snapshot.updated_at = now.clone();
    Ok(())
}

fn increment_recovery_counter_for_stage(
    snapshot: &mut RuntimeSnapshot,
    counters: &mut RecoveryCounters,
    stage: StageName,
    now: &Timestamp,
) -> bool {
    let (Some(work_item_kind), Some(work_item_id)) = (
        snapshot.active_work_item_kind,
        snapshot.active_work_item_id.clone(),
    ) else {
        return false;
    };

    let field = match stage {
        StageName::Troubleshooter => "troubleshoot_attempt_count",
        StageName::Mechanic => "mechanic_attempt_count",
        _ => return false,
    };
    let entry = match counters.entries.iter_mut().find(|entry| {
        entry.failure_class == STALE_ACTIVE_FAILURE_CLASS
            && entry.work_item_kind == work_item_kind
            && entry.work_item_id == work_item_id
    }) {
        Some(entry) => entry,
        None => {
            counters
                .entries
                .push(crate::contracts::RecoveryCounterEntry {
                    failure_class: STALE_ACTIVE_FAILURE_CLASS.to_owned(),
                    work_item_kind,
                    work_item_id: work_item_id.clone(),
                    troubleshoot_attempt_count: 0,
                    mechanic_attempt_count: 0,
                    fix_cycle_count: 0,
                    consultant_invocations: 0,
                    last_updated_at: now.clone(),
                });
            counters
                .entries
                .last_mut()
                .expect("counter entry was just pushed")
        }
    };

    match field {
        "troubleshoot_attempt_count" => {
            entry.troubleshoot_attempt_count += 1;
            snapshot.troubleshoot_attempt_count = entry.troubleshoot_attempt_count;
        }
        "mechanic_attempt_count" => {
            entry.mechanic_attempt_count += 1;
            snapshot.mechanic_attempt_count = entry.mechanic_attempt_count;
        }
        _ => unreachable!("counter field is selected above"),
    }
    entry.last_updated_at = now.clone();
    true
}

fn stale_active_recovery_stage(
    plane: Plane,
    snapshot: &RuntimeSnapshot,
    counters: &RecoveryCounters,
) -> Option<StageName> {
    match plane {
        Plane::Planning => Some(StageName::Mechanic),
        Plane::Execution => {
            let attempts = stale_active_troubleshoot_attempts(snapshot, counters);
            if attempts >= 2 {
                Some(StageName::Consultant)
            } else {
                Some(StageName::Troubleshooter)
            }
        }
        Plane::Learning => None,
    }
}

fn stale_active_troubleshoot_attempts(
    snapshot: &RuntimeSnapshot,
    counters: &RecoveryCounters,
) -> u64 {
    let (Some(kind), Some(id)) = (
        snapshot.active_work_item_kind,
        &snapshot.active_work_item_id,
    ) else {
        return 0;
    };
    counters
        .entries
        .iter()
        .filter(|entry| {
            entry.failure_class == STALE_ACTIVE_FAILURE_CLASS
                && entry.work_item_kind == kind
                && entry.work_item_id == *id
        })
        .map(|entry| entry.troubleshoot_attempt_count)
        .max()
        .unwrap_or(0)
}

fn snapshot_active_work_item_for_plane(
    snapshot: &RuntimeSnapshot,
    plane: Plane,
) -> Option<(WorkItemKind, String)> {
    snapshot
        .active_runs_by_plane
        .get(&plane)
        .and_then(|active_run| {
            Some((
                active_run.work_item_kind?,
                active_run.work_item_id.as_ref()?.clone(),
            ))
        })
}

fn active_run_for_plane(snapshot: &RuntimeSnapshot, plane: Plane) -> Option<&ActiveRunState> {
    snapshot.active_runs_by_plane.get(&plane)
}

fn project_foreground_active_run(snapshot: &mut RuntimeSnapshot) {
    let active_run = [Plane::Planning, Plane::Execution, Plane::Learning]
        .into_iter()
        .find_map(|plane| snapshot.active_runs_by_plane.get(&plane));
    if let Some(active_run) = active_run {
        snapshot.active_plane = Some(active_run.plane);
        snapshot.active_stage = Some(active_run.stage);
        snapshot.active_node_id = Some(active_run.node_id.clone());
        snapshot.active_stage_kind_id = Some(active_run.stage_kind_id.clone());
        snapshot.active_run_id = Some(active_run.run_id.clone());
        snapshot.active_work_item_kind = active_run.work_item_kind;
        snapshot.active_work_item_id = active_run.work_item_id.clone();
        snapshot.active_since = Some(active_run.active_since.clone());
    } else {
        snapshot.active_plane = None;
        snapshot.active_stage = None;
        snapshot.active_node_id = None;
        snapshot.active_stage_kind_id = None;
        snapshot.active_run_id = None;
        snapshot.active_work_item_kind = None;
        snapshot.active_work_item_id = None;
        snapshot.active_since = None;
    }
}

fn stage_identity(plan: &CompiledRunPlan, plane: Plane, stage: StageName) -> (String, String) {
    stage_plan_for(plan, plane, stage)
        .map(|node| (node.node_id.clone(), node.stage_kind_id.clone()))
        .unwrap_or_else(|| {
            let stage_id = stage.as_str().to_owned();
            (stage_id.clone(), stage_id)
        })
}

fn stage_plan_for(
    plan: &CompiledRunPlan,
    plane: Plane,
    stage: StageName,
) -> Option<&MaterializedGraphNodePlan> {
    graph_for_plane(plan, plane)?.nodes.iter().find(|node| {
        node.node_id == stage.as_str()
            || StageName::from_value(&node.stage_kind_id)
                .ok()
                .is_some_and(|stage_name| stage_name == stage)
    })
}

fn graph_for_plane(plan: &CompiledRunPlan, plane: Plane) -> Option<&FrozenGraphPlanePlan> {
    match plane {
        Plane::Execution => Some(&plan.execution_graph),
        Plane::Planning => Some(&plan.planning_graph),
        Plane::Learning => plan.learning_graph.as_ref(),
    }
}

fn count_markdown_files(directory: &Path) -> RuntimeStartupResult<u64> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(RuntimeStartupError::StateStore(StateStoreError::Io {
                path: directory.to_path_buf(),
                message: error.to_string(),
            }));
        }
    };
    let mut count = 0;
    for entry in entries {
        let entry = entry.map_err(|error| {
            RuntimeStartupError::StateStore(StateStoreError::Io {
                path: directory.to_path_buf(),
                message: error.to_string(),
            })
        })?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|extension| extension == "md") {
            count += 1;
        }
    }
    Ok(count)
}

fn workspace_relative_path(paths: &WorkspacePaths, path: &Path) -> String {
    path.strip_prefix(&paths.root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn new_recovery_run_id() -> String {
    let counter = RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("run-recovery-{}-{counter}", process::id())
}

fn utc_now_timestamp(field_name: &'static str) -> RuntimeStartupResult<Timestamp> {
    let rendered = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| RuntimeStartupError::Time {
            field_name,
            message: error.to_string(),
        })?;
    Timestamp::parse(field_name, &rendered).map_err(|error| time_error(field_name, error))
}

fn time_error(field_name: &'static str, error: WorkDocumentError) -> RuntimeStartupError {
    RuntimeStartupError::Time {
        field_name,
        message: error.to_string(),
    }
}

fn config_fingerprint(raw: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(raw.as_bytes());
    format!("cfg-{}", hex_prefix(digest.finalize(), 12))
}

fn hex_prefix(bytes: impl AsRef<[u8]>, chars: usize) -> String {
    bytes
        .as_ref()
        .iter()
        .flat_map(|byte| {
            let rendered = format!("{byte:02x}");
            rendered.into_bytes()
        })
        .take(chars)
        .map(char::from)
        .collect()
}

fn child_table<'a>(
    table: &'a toml::map::Map<String, toml::Value>,
    key: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<&'a toml::map::Map<String, toml::Value>>> {
    child_table_at(table, key, key, path)
}

fn child_table_at<'a>(
    table: &'a toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<&'a toml::map::Map<String, toml::Value>>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    value
        .as_table()
        .map(Some)
        .ok_or_else(|| config_error(path, field, "must be a TOML table when present"))
}

fn optional_string(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<String>> {
    optional_string_at(table, key, key, path)
}

fn optional_string_at(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<String>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|value| Some(value.trim().to_owned()))
        .ok_or_else(|| config_error(path, field, "must be a string when present"))
}

fn optional_bool(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<bool>> {
    optional_bool_at(table, key, key, path)
}

fn optional_bool_at(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<bool>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| config_error(path, field, "must be a boolean when present"))
}

fn optional_string_vec(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<Vec<String>>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let Some(values) = value.as_array() else {
        return Err(config_error(
            path,
            field,
            "must be an array of strings when present",
        ));
    };
    let mut parsed = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let Some(value) = value.as_str() else {
            return Err(config_error(
                path,
                format!("{field}[{index}]"),
                "must be a string",
            ));
        };
        parsed.push(value.to_owned());
    }
    Ok(Some(parsed))
}

fn optional_string_map(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<BTreeMap<String, String>>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let Some(values) = value.as_table() else {
        return Err(config_error(
            path,
            field,
            "must be a TOML table when present",
        ));
    };
    let mut parsed = BTreeMap::new();
    for (name, value) in values {
        let Some(value) = value.as_str() else {
            return Err(config_error(
                path,
                format!("{field}.{name}"),
                "must be a string",
            ));
        };
        parsed.insert(name.clone(), value.to_owned());
    }
    Ok(Some(parsed))
}

fn optional_permission_by_stage(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<BTreeMap<String, CodexPermissionLevel>>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let Some(values) = value.as_table() else {
        return Err(config_error(
            path,
            field,
            "must be a TOML table when present",
        ));
    };
    let mut parsed = BTreeMap::new();
    for (stage_name, value) in values {
        StageName::from_value(stage_name).map_err(|_| {
            config_error(path, format!("{field}.{stage_name}"), "unknown stage name")
        })?;
        parsed.insert(
            stage_name.clone(),
            parse_permission_value(value, &format!("{field}.{stage_name}"), path)?,
        );
    }
    Ok(Some(parsed))
}

fn optional_permission_by_model(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<BTreeMap<String, CodexPermissionLevel>>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let Some(values) = value.as_table() else {
        return Err(config_error(
            path,
            field,
            "must be a TOML table when present",
        ));
    };
    let mut parsed = BTreeMap::new();
    for (model_name, value) in values {
        parsed.insert(
            model_name.clone(),
            parse_permission_value(value, &format!("{field}.{model_name}"), path)?,
        );
    }
    Ok(Some(parsed))
}

fn optional_capability_policy_decision_map(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<BTreeMap<String, CapabilityPolicyDecision>>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let Some(values) = value.as_table() else {
        return Err(config_error(
            path,
            field,
            "must be a TOML table when present",
        ));
    };
    let mut parsed = BTreeMap::new();
    for (capability_id, value) in values {
        let normalized = validate_capability_id(capability_id).map_err(|error| {
            config_error(path, format!("{field}.{capability_id}"), error.to_string())
        })?;
        parsed.insert(
            normalized,
            parse_capability_policy_decision_value(
                value,
                &format!("{field}.{capability_id}"),
                path,
            )?,
        );
    }
    Ok(Some(parsed))
}

fn optional_positive_f64(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<f64>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let parsed = if let Some(value) = value.as_float() {
        value
    } else if let Some(value) = value.as_integer() {
        value as f64
    } else {
        return Err(config_error(path, field, "must be a number when present"));
    };
    if parsed <= 0.0 {
        return Err(config_error(path, field, "must be greater than 0"));
    }
    Ok(Some(parsed))
}

fn optional_positive_u64(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<u64>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let Some(parsed) = value.as_integer() else {
        return Err(config_error(path, field, "must be an integer when present"));
    };
    if parsed <= 0 {
        return Err(config_error(path, field, "must be greater than 0"));
    }
    Ok(Some(parsed as u64))
}

fn optional_non_negative_u64(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<u64>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let Some(parsed) = value.as_integer() else {
        return Err(config_error(path, field, "must be an integer when present"));
    };
    if parsed < 0 {
        return Err(config_error(
            path,
            field,
            "must be greater than or equal to 0",
        ));
    }
    Ok(Some(parsed as u64))
}

fn optional_non_negative_u64_vec(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<Vec<u64>>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let Some(values) = value.as_array() else {
        return Err(config_error(
            path,
            field,
            "must be an array of integers when present",
        ));
    };
    if values.is_empty() {
        return Err(config_error(path, field, "must not be empty"));
    }
    let mut parsed = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let item_field = format!("{field}[{index}]");
        let Some(seconds) = value.as_integer() else {
            return Err(config_error(path, item_field, "must be an integer"));
        };
        if seconds < 0 {
            return Err(config_error(
                path,
                item_field,
                "must be greater than or equal to 0",
            ));
        }
        parsed.push(seconds as u64);
    }
    Ok(Some(parsed))
}

fn load_auto_recovery_config(
    table: &toml::map::Map<String, toml::Value>,
    config: &mut AutoRecoveryConfig,
    path: &Path,
) -> RuntimeStartupResult<()> {
    reject_unknown_auto_recovery_config_keys(table, path)?;
    if let Some(enabled) = optional_bool_at(table, "enabled", "auto_recovery.enabled", path)? {
        config.enabled = enabled;
    }
    if let Some(enabled) = optional_bool_at(
        table,
        "blocked_dependency_retry_enabled",
        "auto_recovery.blocked_dependency_retry_enabled",
        path,
    )? {
        config.blocked_dependency_retry_enabled = enabled;
    }
    if let Some(max_auto_requeues) = optional_non_negative_u64(
        table,
        "max_auto_requeues_per_work_item",
        "auto_recovery.max_auto_requeues_per_work_item",
        path,
    )? {
        config.max_auto_requeues_per_work_item = max_auto_requeues;
    }
    if let Some(cooldown_seconds) = optional_non_negative_u64_vec(
        table,
        "cooldown_seconds",
        "auto_recovery.cooldown_seconds",
        path,
    )? {
        config.cooldown_seconds = cooldown_seconds;
    }
    Ok(())
}

fn reject_unknown_auto_recovery_config_keys(
    table: &toml::map::Map<String, toml::Value>,
    path: &Path,
) -> RuntimeStartupResult<()> {
    for key in table.keys() {
        if !matches!(
            key.as_str(),
            "enabled"
                | "blocked_dependency_retry_enabled"
                | "max_auto_requeues_per_work_item"
                | "cooldown_seconds"
        ) {
            return Err(config_error(
                path,
                format!("auto_recovery.{key}"),
                "unknown auto_recovery config key",
            ));
        }
    }
    Ok(())
}

fn load_execution_capabilities_config(
    table: &toml::map::Map<String, toml::Value>,
    config: &mut ExecutionCapabilitiesConfig,
    path: &Path,
) -> RuntimeStartupResult<()> {
    reject_unknown_execution_capabilities_config_keys(table, path)?;
    if let Some(enabled) =
        optional_bool_at(table, "enabled", "execution_capabilities.enabled", path)?
    {
        config.enabled = enabled;
    }
    if let Some(decision) = table.get("default_unknown_capability") {
        config.default_unknown_capability = parse_capability_policy_decision_value(
            decision,
            "execution_capabilities.default_unknown_capability",
            path,
        )?;
    }
    if let Some(allow_advisory_grants) = optional_bool_at(
        table,
        "allow_advisory_grants",
        "execution_capabilities.allow_advisory_grants",
        path,
    )? {
        config.allow_advisory_grants = allow_advisory_grants;
    }
    if let Some(fail_required_advisory) = optional_bool_at(
        table,
        "fail_required_advisory",
        "execution_capabilities.fail_required_advisory",
        path,
    )? {
        config.fail_required_advisory = fail_required_advisory;
    }
    if let Some(defaults) = optional_capability_policy_decision_map(
        table,
        "defaults",
        "execution_capabilities.defaults",
        path,
    )? {
        config.defaults = defaults;
    }
    Ok(())
}

fn reject_unknown_execution_capabilities_config_keys(
    table: &toml::map::Map<String, toml::Value>,
    path: &Path,
) -> RuntimeStartupResult<()> {
    for key in table.keys() {
        if !matches!(
            key.as_str(),
            "enabled"
                | "default_unknown_capability"
                | "allow_advisory_grants"
                | "fail_required_advisory"
                | "defaults"
        ) {
            return Err(config_error(
                path,
                format!("execution_capabilities.{key}"),
                "unknown execution_capabilities config key",
            ));
        }
    }
    Ok(())
}

fn load_usage_governance_config(
    table: &toml::map::Map<String, toml::Value>,
    config: &mut UsageGovernanceConfig,
    path: &Path,
) -> RuntimeStartupResult<()> {
    if let Some(enabled) = optional_bool_at(table, "enabled", "usage_governance.enabled", path)? {
        config.enabled = enabled;
    }
    if let Some(auto_resume) =
        optional_bool_at(table, "auto_resume", "usage_governance.auto_resume", path)?
    {
        config.auto_resume = auto_resume;
    }
    if let Some(boundary) = table.get("evaluation_boundary") {
        config.evaluation_boundary = parse_usage_governance_evaluation_boundary(
            boundary,
            "usage_governance.evaluation_boundary",
            path,
        )?;
    }
    if let Some(timezone) = optional_string_at(
        table,
        "calendar_timezone",
        "usage_governance.calendar_timezone",
        path,
    )? {
        if timezone.is_empty() {
            return Err(config_error(
                path,
                "usage_governance.calendar_timezone",
                "must not be empty",
            ));
        }
        config.calendar_timezone = timezone;
    }

    if let Some(runtime_rules) = child_table_at(
        table,
        "runtime_token_rules",
        "usage_governance.runtime_token_rules",
        path,
    )? {
        load_runtime_token_rules_config(runtime_rules, &mut config.runtime_token_rules, path)?;
    }
    if let Some(quota_rules) = child_table_at(
        table,
        "subscription_quota_rules",
        "usage_governance.subscription_quota_rules",
        path,
    )? {
        load_subscription_quota_rules_config(
            quota_rules,
            &mut config.subscription_quota_rules,
            path,
        )?;
    }
    Ok(())
}

fn load_runtime_token_rules_config(
    table: &toml::map::Map<String, toml::Value>,
    config: &mut RuntimeTokenRulesConfig,
    path: &Path,
) -> RuntimeStartupResult<()> {
    if let Some(enabled) = optional_bool_at(
        table,
        "enabled",
        "usage_governance.runtime_token_rules.enabled",
        path,
    )? {
        config.enabled = enabled;
    }
    if let Some(rules) = table.get("rules") {
        let Some(rules) = rules.as_array() else {
            return Err(config_error(
                path,
                "usage_governance.runtime_token_rules.rules",
                "must be an array of tables when present",
            ));
        };
        let mut parsed = Vec::with_capacity(rules.len());
        for (index, value) in rules.iter().enumerate() {
            let field = format!("usage_governance.runtime_token_rules.rules[{index}]");
            let Some(rule_table) = value.as_table() else {
                return Err(config_error(path, field, "must be a TOML table"));
            };
            parsed.push(parse_runtime_token_rule(rule_table, &field, path)?);
        }
        config.rules = parsed;
    }
    Ok(())
}

fn parse_runtime_token_rule(
    table: &toml::map::Map<String, toml::Value>,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<RuntimeTokenRuleConfig> {
    let rule_id = required_config_string(table, "rule_id", &format!("{field}.rule_id"), path)?;
    validate_safe_identifier(&rule_id, &format!("{field}.rule_id"))
        .map_err(|error| config_error(path, format!("{field}.rule_id"), error.to_string()))?;
    let window =
        match required_config_string(table, "window", &format!("{field}.window"), path)?.as_str() {
            "rolling_5h" => UsageGovernanceRuntimeTokenWindow::Rolling5h,
            "calendar_week" => UsageGovernanceRuntimeTokenWindow::CalendarWeek,
            "daemon_session" => UsageGovernanceRuntimeTokenWindow::DaemonSession,
            "per_run" => UsageGovernanceRuntimeTokenWindow::PerRun,
            _ => {
                return Err(config_error(
                    path,
                    format!("{field}.window"),
                    "must be one of `rolling_5h`, `calendar_week`, `daemon_session`, or `per_run`",
                ));
            }
        };
    let metric = match optional_string_at(table, "metric", &format!("{field}.metric"), path)?
        .unwrap_or_else(|| "total_tokens".to_owned())
        .as_str()
    {
        "total_tokens" => UsageGovernanceRuntimeTokenMetric::TotalTokens,
        _ => {
            return Err(config_error(
                path,
                format!("{field}.metric"),
                "must be `total_tokens`",
            ));
        }
    };
    let threshold =
        required_positive_config_u64(table, "threshold", &format!("{field}.threshold"), path)?;
    Ok(RuntimeTokenRuleConfig {
        rule_id,
        window,
        metric,
        threshold,
    })
}

fn load_subscription_quota_rules_config(
    table: &toml::map::Map<String, toml::Value>,
    config: &mut SubscriptionQuotaRulesConfig,
    path: &Path,
) -> RuntimeStartupResult<()> {
    if let Some(enabled) = optional_bool_at(
        table,
        "enabled",
        "usage_governance.subscription_quota_rules.enabled",
        path,
    )? {
        config.enabled = enabled;
    }
    if let Some(provider) = table.get("provider") {
        config.provider = parse_subscription_provider(
            provider,
            "usage_governance.subscription_quota_rules.provider",
            path,
        )?;
    }
    if let Some(policy) = table.get("degraded_policy") {
        config.degraded_policy = parse_degraded_policy(
            policy,
            "usage_governance.subscription_quota_rules.degraded_policy",
            path,
        )?;
    }
    if let Some(refresh) = optional_positive_u64(
        table,
        "refresh_interval_seconds",
        "usage_governance.subscription_quota_rules.refresh_interval_seconds",
        path,
    )? {
        config.refresh_interval_seconds = refresh;
    }
    if let Some(rules) = table.get("rules") {
        let Some(rules) = rules.as_array() else {
            return Err(config_error(
                path,
                "usage_governance.subscription_quota_rules.rules",
                "must be an array of tables when present",
            ));
        };
        let mut parsed = Vec::with_capacity(rules.len());
        for (index, value) in rules.iter().enumerate() {
            let field = format!("usage_governance.subscription_quota_rules.rules[{index}]");
            let Some(rule_table) = value.as_table() else {
                return Err(config_error(path, field, "must be a TOML table"));
            };
            parsed.push(parse_subscription_quota_rule(rule_table, &field, path)?);
        }
        config.rules = parsed;
    }
    Ok(())
}

fn parse_subscription_quota_rule(
    table: &toml::map::Map<String, toml::Value>,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<SubscriptionQuotaRuleConfig> {
    let rule_id = required_config_string(table, "rule_id", &format!("{field}.rule_id"), path)?;
    validate_safe_identifier(&rule_id, &format!("{field}.rule_id"))
        .map_err(|error| config_error(path, format!("{field}.rule_id"), error.to_string()))?;
    let window =
        match required_config_string(table, "window", &format!("{field}.window"), path)?.as_str() {
            "five_hour" => UsageGovernanceSubscriptionWindow::FiveHour,
            "weekly" => UsageGovernanceSubscriptionWindow::Weekly,
            _ => {
                return Err(config_error(
                    path,
                    format!("{field}.window"),
                    "must be one of `five_hour` or `weekly`",
                ));
            }
        };
    let pause_at_percent_used = required_positive_config_f64(
        table,
        "pause_at_percent_used",
        &format!("{field}.pause_at_percent_used"),
        path,
    )?;
    if pause_at_percent_used > 100.0 {
        return Err(config_error(
            path,
            format!("{field}.pause_at_percent_used"),
            "must be <= 100",
        ));
    }
    Ok(SubscriptionQuotaRuleConfig {
        rule_id,
        window,
        pause_at_percent_used,
    })
}

fn required_config_string(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<String> {
    optional_string_at(table, key, field, path)?
        .ok_or_else(|| config_error(path, field, "is required"))
}

fn required_positive_config_u64(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<u64> {
    optional_positive_u64(table, key, field, path)?
        .ok_or_else(|| config_error(path, field, "is required"))
}

fn required_positive_config_f64(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<f64> {
    optional_positive_f64(table, key, field, path)?
        .ok_or_else(|| config_error(path, field, "is required"))
}

fn parse_usage_governance_evaluation_boundary(
    value: &toml::Value,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<UsageGovernanceEvaluationBoundary> {
    let Some(value) = value.as_str() else {
        return Err(config_error(path, field, "must be a string when present"));
    };
    match value {
        "between_stages" => Ok(UsageGovernanceEvaluationBoundary::BetweenStages),
        _ => Err(config_error(path, field, "must be `between_stages`")),
    }
}

fn parse_subscription_provider(
    value: &toml::Value,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<UsageGovernanceSubscriptionProvider> {
    let Some(value) = value.as_str() else {
        return Err(config_error(path, field, "must be a string when present"));
    };
    match value {
        "codex_chatgpt_oauth" => Ok(UsageGovernanceSubscriptionProvider::CodexChatGptOauth),
        _ => Err(config_error(path, field, "must be `codex_chatgpt_oauth`")),
    }
}

fn parse_degraded_policy(
    value: &toml::Value,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<UsageGovernanceDegradedPolicy> {
    let Some(value) = value.as_str() else {
        return Err(config_error(path, field, "must be a string when present"));
    };
    match value {
        "fail_open" => Ok(UsageGovernanceDegradedPolicy::FailOpen),
        "fail_closed" => Ok(UsageGovernanceDegradedPolicy::FailClosed),
        _ => Err(config_error(
            path,
            field,
            "must be one of `fail_open` or `fail_closed`",
        )),
    }
}

fn load_codex_runner_config(
    table: &toml::map::Map<String, toml::Value>,
    config: &mut CodexCliConfig,
    path: &Path,
) -> RuntimeStartupResult<()> {
    if let Some(command) = optional_string_at(table, "command", "runners.codex.command", path)? {
        config.command = command;
    }
    if let Some(args) = optional_string_vec(table, "args", "runners.codex.args", path)? {
        config.args = args;
    }
    if let Some(profile) = optional_string_at(table, "profile", "runners.codex.profile", path)? {
        config.profile = Some(profile);
    }
    if let Some(permission) = table.get("permission_default") {
        config.permission_default =
            parse_permission_value(permission, "runners.codex.permission_default", path)?;
    }
    if let Some(permission_by_stage) = optional_permission_by_stage(
        table,
        "permission_by_stage",
        "runners.codex.permission_by_stage",
        path,
    )? {
        config.permission_by_stage = permission_by_stage;
    }
    if let Some(permission_by_model) = optional_permission_by_model(
        table,
        "permission_by_model",
        "runners.codex.permission_by_model",
        path,
    )? {
        config.permission_by_model = permission_by_model;
    }
    if let Some(reasoning) = table.get("model_reasoning_effort") {
        config.model_reasoning_effort = Some(parse_reasoning_effort_value(
            reasoning,
            "runners.codex.model_reasoning_effort",
            path,
        )?);
    }
    if let Some(skip_git_repo_check) = optional_bool_at(
        table,
        "skip_git_repo_check",
        "runners.codex.skip_git_repo_check",
        path,
    )? {
        config.skip_git_repo_check = skip_git_repo_check;
    }
    if let Some(extra_config) =
        optional_string_vec(table, "extra_config", "runners.codex.extra_config", path)?
    {
        config.extra_config = extra_config;
    }
    if let Some(env) = optional_string_map(table, "env", "runners.codex.env", path)? {
        config.env = env;
    }
    Ok(())
}

fn load_pi_runner_config(
    table: &toml::map::Map<String, toml::Value>,
    config: &mut PiRpcConfig,
    path: &Path,
) -> RuntimeStartupResult<()> {
    if let Some(command) = optional_string_at(table, "command", "runners.pi.command", path)? {
        config.command = command;
    }
    if let Some(args) = optional_string_vec(table, "args", "runners.pi.args", path)? {
        config.args = args;
    }
    if let Some(provider) = optional_string_at(table, "provider", "runners.pi.provider", path)? {
        config.provider = Some(provider);
    }
    if let Some(thinking) = optional_string_at(table, "thinking", "runners.pi.thinking", path)? {
        config.thinking = Some(thinking);
    }
    if let Some(disable_context_files) = optional_bool_at(
        table,
        "disable_context_files",
        "runners.pi.disable_context_files",
        path,
    )? {
        config.disable_context_files = disable_context_files;
    }
    if let Some(disable_skills) =
        optional_bool_at(table, "disable_skills", "runners.pi.disable_skills", path)?
    {
        config.disable_skills = disable_skills;
    }
    if let Some(policy) = table.get("event_log_policy") {
        config.event_log_policy =
            parse_pi_event_log_policy_value(policy, "runners.pi.event_log_policy", path)?;
    }
    if let Some(env) = optional_string_map(table, "env", "runners.pi.env", path)? {
        config.env = env;
    }
    config
        .validate()
        .map_err(|error| config_error(path, "runners.pi.args", error.to_string()))?;
    Ok(())
}

fn load_stage_configs(
    table: &toml::map::Map<String, toml::Value>,
    path: &Path,
) -> RuntimeStartupResult<BTreeMap<String, RuntimeStageConfig>> {
    let mut stages = BTreeMap::new();
    for (stage_name, value) in table {
        StageName::from_value(stage_name).map_err(|_| {
            config_error(path, format!("stages.{stage_name}"), "unknown stage name")
        })?;
        let Some(stage_table) = value.as_table() else {
            return Err(config_error(
                path,
                format!("stages.{stage_name}"),
                "must be a TOML table",
            ));
        };
        reject_unknown_stage_config_keys(stage_name, stage_table, path)?;
        let field_prefix = format!("stages.{stage_name}");
        let runner = optional_string_at(
            stage_table,
            "runner",
            &format!("{field_prefix}.runner"),
            path,
        )?
        .map(|runner| {
            normalize_runner_config_name(&runner, &format!("{field_prefix}.runner"), path)
        })
        .transpose()?;
        let model =
            optional_string_at(stage_table, "model", &format!("{field_prefix}.model"), path)?;
        let model_reasoning_effort = match stage_table.get("model_reasoning_effort") {
            Some(value) => Some(parse_reasoning_effort_value(
                value,
                &format!("{field_prefix}.model_reasoning_effort"),
                path,
            )?),
            None => None,
        };
        let thinking_level = normalize_stage_thinking_aliases(
            optional_string_at(
                stage_table,
                "thinking_level",
                &format!("{field_prefix}.thinking_level"),
                path,
            )?,
            model_reasoning_effort.as_deref(),
            &field_prefix,
            path,
        )?;
        let timeout_seconds = optional_positive_u64(
            stage_table,
            "timeout_seconds",
            &format!("{field_prefix}.timeout_seconds"),
            path,
        )?
        .unwrap_or(3600);

        stages.insert(
            stage_name.clone(),
            RuntimeStageConfig {
                runner,
                model,
                thinking_level,
                model_reasoning_effort,
                timeout_seconds,
            },
        );
    }
    Ok(stages)
}

fn reject_unknown_stage_config_keys(
    stage_name: &str,
    table: &toml::map::Map<String, toml::Value>,
    path: &Path,
) -> RuntimeStartupResult<()> {
    for key in table.keys() {
        if matches!(
            key.as_str(),
            "runner" | "model" | "thinking_level" | "model_reasoning_effort" | "timeout_seconds"
        ) {
            continue;
        }
        return Err(config_error(
            path,
            format!("stages.{stage_name}.{key}"),
            "unsupported stage override key",
        ));
    }
    Ok(())
}

fn normalize_stage_thinking_aliases(
    thinking_level: Option<String>,
    model_reasoning_effort: Option<&str>,
    field_prefix: &str,
    path: &Path,
) -> RuntimeStartupResult<Option<String>> {
    if let Some(thinking_level) = thinking_level {
        if thinking_level.trim().is_empty() {
            return Err(config_error(
                path,
                format!("{field_prefix}.thinking_level"),
                "must not be empty",
            ));
        }
        if let Some(model_reasoning_effort) = model_reasoning_effort {
            if thinking_level != model_reasoning_effort {
                return Err(config_error(
                    path,
                    format!("{field_prefix}.thinking_level"),
                    "thinking_level and model_reasoning_effort must match when both are set",
                ));
            }
        }
        return Ok(Some(thinking_level));
    }
    Ok(model_reasoning_effort.map(ToOwned::to_owned))
}

fn parse_permission_value(
    value: &toml::Value,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<CodexPermissionLevel> {
    let Some(value) = value.as_str() else {
        return Err(config_error(path, field, "must be a string when present"));
    };
    match value {
        "basic" => Ok(CodexPermissionLevel::Basic),
        "elevated" => Ok(CodexPermissionLevel::Elevated),
        "maximum" => Ok(CodexPermissionLevel::Maximum),
        _ => Err(config_error(
            path,
            field,
            "must be one of `basic`, `elevated`, or `maximum`",
        )),
    }
}

fn parse_capability_policy_decision_value(
    value: &toml::Value,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<CapabilityPolicyDecision> {
    let Some(value) = value.as_str() else {
        return Err(config_error(path, field, "must be a string when present"));
    };
    CapabilityPolicyDecision::from_value(value).map_err(|_| {
        config_error(
            path,
            field,
            "must be one of `allow`, `deny`, or `approval_required`",
        )
    })
}

fn parse_reasoning_effort_value(
    value: &toml::Value,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<String> {
    let Some(value) = value.as_str() else {
        return Err(config_error(path, field, "must be a string when present"));
    };
    match value {
        "low" | "medium" | "high" | "xhigh" => Ok(value.to_owned()),
        _ => Err(config_error(
            path,
            field,
            "must be one of `low`, `medium`, `high`, or `xhigh`",
        )),
    }
}

fn parse_pi_event_log_policy_value(
    value: &toml::Value,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<PiEventLogPolicy> {
    let Some(value) = value.as_str() else {
        return Err(config_error(path, field, "must be a string when present"));
    };
    match value {
        "failure_full" => Ok(PiEventLogPolicy::FailureFull),
        "full" => Ok(PiEventLogPolicy::Full),
        _ => Err(config_error(
            path,
            field,
            "must be one of `failure_full` or `full`",
        )),
    }
}

fn normalize_runner_config_name(
    runner_name: &str,
    field: &str,
    path: &Path,
) -> RuntimeStartupResult<String> {
    let runner_name = runner_name.trim();
    validate_safe_identifier(runner_name, field)
        .map(|value| value.to_owned())
        .map_err(|error| config_error(path, field, error.to_string()))
}

fn config_error(
    path: &Path,
    field: impl Into<String>,
    message: impl Into<String>,
) -> RuntimeStartupError {
    RuntimeStartupError::Config {
        path: path.to_path_buf(),
        field: Some(field.into()),
        message: message.into(),
    }
}

#[allow(dead_code)]
fn active_run_request_kind_for_work_item(kind: WorkItemKind) -> ActiveRunRequestKind {
    if kind == WorkItemKind::LearningRequest {
        ActiveRunRequestKind::LearningRequest
    } else {
        ActiveRunRequestKind::ActiveWorkItem
    }
}
