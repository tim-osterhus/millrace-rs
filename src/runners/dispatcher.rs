//! Runtime-facing dispatcher that resolves stage-runner adapters.

use crate::contracts::{
    CapabilityDecisionState, CapabilityEnforcementMode, CapabilitySupportDecision,
    CapabilitySupportState, ExecutionCapabilityGrant,
};
use crate::runtime::StageRunRequest;

use super::{
    RunnerError, RunnerRawResult, RunnerRegistry, RunnerResult, StageRunnerAdapter,
    registry::normalize_runner_name,
};

const PYTHON_COMPATIBLE_FALLBACK_RUNNER: &str = "codex_cli";

/// Callable stage runner that delegates to a registry-selected adapter.
#[derive(Debug)]
pub struct StageRunnerDispatcher {
    registry: RunnerRegistry,
    default_runner: Option<String>,
}

impl StageRunnerDispatcher {
    /// Builds a dispatcher with no caller-supplied default runner.
    #[must_use]
    pub fn new(registry: RunnerRegistry) -> Self {
        Self {
            registry,
            default_runner: None,
        }
    }

    /// Builds a dispatcher with a caller-supplied runtime default runner.
    #[must_use]
    pub fn with_default_runner(
        registry: RunnerRegistry,
        default_runner: impl Into<String>,
    ) -> Self {
        let default_runner = default_runner.into();
        Self {
            registry,
            default_runner: Some(default_runner),
        }
    }

    /// Returns the Python-compatible fallback runner name.
    #[must_use]
    pub const fn fallback_runner_name() -> &'static str {
        PYTHON_COMPATIBLE_FALLBACK_RUNNER
    }

    /// Resolves the runner name using request, caller default, then Codex CLI fallback.
    #[must_use]
    pub fn resolve_runner_name(&self, request: &StageRunRequest) -> String {
        if let Some(runner_name) = non_blank(request.runner_name.as_deref()) {
            return runner_name.to_owned();
        }
        if let Some(runner_name) = non_blank(self.default_runner.as_deref()) {
            return runner_name.to_owned();
        }
        PYTHON_COMPATIBLE_FALLBACK_RUNNER.to_owned()
    }

    /// Returns the registry backing this dispatcher.
    #[must_use]
    pub fn registry(&self) -> &RunnerRegistry {
        &self.registry
    }
}

impl StageRunnerAdapter for StageRunnerDispatcher {
    fn evaluate_capability_grant(
        &self,
        grant: &ExecutionCapabilityGrant,
        request: &StageRunRequest,
    ) -> CapabilitySupportDecision {
        let resolved_runner_name = self.resolve_runner_name(request);
        let runner_name = match normalize_runner_name(resolved_runner_name.clone()) {
            Ok(runner_name) => runner_name,
            Err(_) => {
                return conservative_capability_support_decision(
                    grant,
                    request,
                    resolved_runner_name,
                    "invalid runner",
                );
            }
        };
        let Some(adapter) = self.registry.get(&runner_name) else {
            return conservative_capability_support_decision(
                grant,
                request,
                runner_name,
                "unknown runner",
            );
        };
        adapter.evaluate_capability_grant(grant, request)
    }

    fn run(&self, request: &StageRunRequest) -> RunnerResult<RunnerRawResult> {
        let runner_name = normalize_runner_name(self.resolve_runner_name(request))?;
        let available = self.registry.names();
        let Some(adapter) = self.registry.get(&runner_name) else {
            return Err(RunnerError::UnknownRunner {
                requested: runner_name,
                available,
            });
        };
        let request = adapter.request_with_capability_support(request);
        adapter.run(&request)
    }
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn conservative_capability_support_decision(
    grant: &ExecutionCapabilityGrant,
    request: &StageRunRequest,
    runner_id: String,
    reason: &str,
) -> CapabilitySupportDecision {
    if grant.decision_state != CapabilityDecisionState::Granted {
        return CapabilitySupportDecision {
            runner_id,
            invocation_context_ref: request.stage.as_str().to_owned(),
            grant_id: grant.grant_id.clone(),
            support_state: CapabilitySupportState::Unsupported,
            enforcement_mode: CapabilityEnforcementMode::NotApplicable,
            limitations: Vec::new(),
            evidence_available: Vec::new(),
            reason: format!(
                "{reason}; grant decision is {}",
                grant.decision_state.as_str()
            ),
        };
    }
    CapabilitySupportDecision {
        runner_id,
        invocation_context_ref: request.stage.as_str().to_owned(),
        grant_id: grant.grant_id.clone(),
        support_state: CapabilitySupportState::PartiallySupported,
        enforcement_mode: CapabilityEnforcementMode::AdvisoryOnly,
        limitations: vec![
            "dispatcher could not prove runner-specific enforcement boundaries".to_owned(),
        ],
        evidence_available: Vec::new(),
        reason: reason.to_owned(),
    }
}
