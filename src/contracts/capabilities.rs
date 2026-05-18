//! Python-compatible execution capability request and grant contracts.

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::{
    CapabilityDecisionState, CapabilityEnforcementMode, CapabilityEvidenceStatus,
    CapabilityPolicyDecision, CapabilitySupportState,
};

/// Canonical base execution capability ids accepted by the v0.19.0 surface.
pub const BASE_EXECUTION_CAPABILITY_IDS: &[&str] = &[
    "workspace.read",
    "workspace.write",
    "artifact.read",
    "artifact.write",
    "runner.invoke",
    "shell.run",
    "git.read",
    "git.mutate",
    "package.install",
    "network.access",
    "approval.request",
    "evidence.emit",
    "runtime.control",
];

const CAPABILITY_SCOPE_KINDS: &[&str] = &[
    "workspace",
    "workspace_path",
    "artifact_kind",
    "artifact_ref",
    "runner",
    "command_class",
    "git_action",
    "package_manager",
    "network_class",
    "approval_policy_ref",
    "runtime_action",
];

const CAPABILITY_RUNTIME_ACTION_SCOPE_VALUES: &[&str] = &[
    "enqueue",
    "pause",
    "resume",
    "cancel",
    "retry",
    "repair",
    "approve",
    "deny",
    "reload_config",
];

const CAPABILITY_ACCESS_VALUES: &[&str] =
    &["read", "write", "execute", "mutate", "request", "emit"];

const APPROVAL_GATE_SCOPES: &[&str] = &["stage", "run", "work_item"];

/// Returns the Python-compatible capability key aliases.
#[must_use]
pub fn capability_key_aliases() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("workspace_read", "workspace.read"),
        ("workspace_write", "workspace.write"),
        ("artifact_read", "artifact.read"),
        ("artifact_write", "artifact.write"),
        ("runner_invoke", "runner.invoke"),
        ("shell_run", "shell.run"),
        ("git_read", "git.read"),
        ("git_mutate", "git.mutate"),
        ("package_install", "package.install"),
        ("network_access", "network.access"),
        ("approval_request", "approval.request"),
        ("evidence_emit", "evidence.emit"),
        ("runtime_control", "runtime.control"),
    ])
}

/// Normalizes a capability id or config key alias into the canonical dotted id.
#[must_use]
pub fn normalize_capability_id(value: &str) -> String {
    let normalized = value.trim();
    match normalized {
        "workspace_read" => "workspace.read",
        "workspace_write" => "workspace.write",
        "artifact_read" => "artifact.read",
        "artifact_write" => "artifact.write",
        "runner_invoke" => "runner.invoke",
        "shell_run" => "shell.run",
        "git_read" => "git.read",
        "git_mutate" => "git.mutate",
        "package_install" => "package.install",
        "network_access" => "network.access",
        "approval_request" => "approval.request",
        "evidence_emit" => "evidence.emit",
        "runtime_control" => "runtime.control",
        _ => normalized,
    }
    .to_owned()
}

/// Returns true when the id is one of the canonical base execution capabilities.
#[must_use]
pub fn is_base_execution_capability_id(value: &str) -> bool {
    BASE_EXECUTION_CAPABILITY_IDS.contains(&value)
}

/// Validates and normalizes one execution capability id.
pub fn validate_capability_id(value: &str) -> Result<String, CapabilityContractError> {
    let normalized = normalize_capability_id(value);
    validate_safe_capability_id(&normalized)?;
    if !is_base_execution_capability_id(&normalized) {
        return Err(CapabilityContractError::new(format!(
            "unknown capability_id: {value}"
        )));
    }
    Ok(normalized)
}

/// One scoped target attached to a capability request, override, or grant.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityScope {
    pub kind: String,
    pub value: String,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

impl CapabilityScope {
    pub fn validate(&self) -> Result<(), CapabilityContractError> {
        if !CAPABILITY_SCOPE_KINDS.contains(&self.kind.as_str()) {
            return Err(CapabilityContractError::new(format!(
                "unknown capability scope kind: {}",
                self.kind
            )));
        }
        if self.value.trim().is_empty() {
            return Err(CapabilityContractError::new(
                "capability scope value is required",
            ));
        }
        if self.kind == "workspace_path" && !workspace_path_scope_stays_inside(&self.value) {
            return Err(CapabilityContractError::new(
                "workspace_path scope must stay inside workspace",
            ));
        }
        if self.kind == "runtime_action"
            && !CAPABILITY_RUNTIME_ACTION_SCOPE_VALUES.contains(&self.value.as_str())
        {
            return Err(CapabilityContractError::new(format!(
                "unknown runtime_action scope: {}",
                self.value
            )));
        }
        Ok(())
    }

    fn from_raw(raw: RawCapabilityScope) -> Result<Self, CapabilityContractError> {
        let scope = Self {
            kind: raw.kind.trim().to_owned(),
            value: raw.value.trim().to_owned(),
            metadata: raw.metadata,
        };
        scope.validate()?;
        Ok(scope)
    }
}

impl<'de> Deserialize<'de> for CapabilityScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_raw(RawCapabilityScope::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCapabilityScope {
    kind: String,
    value: String,
    #[serde(default)]
    metadata: Map<String, Value>,
}

/// Approval policy referenced by approval-required execution capability grants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalPolicyRef {
    pub policy_id: String,
    #[serde(default = "default_approval_gate_scope")]
    pub gate_scope: String,
    #[serde(default)]
    pub expiration_seconds: Option<u64>,
    #[serde(default = "default_required_approval_decision")]
    pub required_decision: String,
}

impl ApprovalPolicyRef {
    pub fn validate(&self) -> Result<(), CapabilityContractError> {
        if self.policy_id.trim().is_empty() {
            return Err(CapabilityContractError::new("policy_id is required"));
        }
        if !APPROVAL_GATE_SCOPES.contains(&self.gate_scope.as_str()) {
            return Err(CapabilityContractError::new(format!(
                "unknown approval policy gate_scope: {}",
                self.gate_scope
            )));
        }
        if self.expiration_seconds == Some(0) {
            return Err(CapabilityContractError::new(
                "expiration_seconds must be greater than 0",
            ));
        }
        if self.required_decision != "approved" {
            return Err(CapabilityContractError::new(
                "required_decision must be approved",
            ));
        }
        Ok(())
    }

    fn from_raw(raw: RawApprovalPolicyRef) -> Result<Self, CapabilityContractError> {
        let reference = Self {
            policy_id: raw.policy_id.trim().to_owned(),
            gate_scope: raw.gate_scope,
            expiration_seconds: raw.expiration_seconds,
            required_decision: raw.required_decision,
        };
        reference.validate()?;
        Ok(reference)
    }
}

impl<'de> Deserialize<'de> for ApprovalPolicyRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_raw(RawApprovalPolicyRef::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawApprovalPolicyRef {
    policy_id: String,
    #[serde(default = "default_approval_gate_scope")]
    gate_scope: String,
    #[serde(default)]
    expiration_seconds: Option<u64>,
    #[serde(default = "default_required_approval_decision")]
    required_decision: String,
}

/// One capability request produced from a stage, graph node, mode, or config.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequest {
    pub request_id: String,
    pub capability_id: String,
    pub access: String,
    pub scope: CapabilityScope,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub requires_enforcement: bool,
    #[serde(default)]
    pub reason: String,
    #[serde(default = "default_requested_by")]
    pub requested_by: String,
    #[serde(default)]
    pub policy_source: Option<String>,
}

impl CapabilityRequest {
    pub fn validate(&self) -> Result<(), CapabilityContractError> {
        require_non_empty("request_id", &self.request_id)?;
        validate_capability_id(&self.capability_id)?;
        validate_access(&self.access)?;
        self.scope.validate()?;
        require_non_empty("requested_by", &self.requested_by)?;
        Ok(())
    }

    fn from_raw(raw: RawCapabilityRequest) -> Result<Self, CapabilityContractError> {
        let request = Self {
            request_id: raw.request_id.trim().to_owned(),
            capability_id: validate_capability_id(&raw.capability_id)?,
            access: raw.access,
            scope: raw.scope,
            required: raw.required,
            requires_enforcement: raw.requires_enforcement,
            reason: raw.reason,
            requested_by: raw.requested_by,
            policy_source: raw.policy_source,
        };
        request.validate()?;
        Ok(request)
    }
}

impl<'de> Deserialize<'de> for CapabilityRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_raw(RawCapabilityRequest::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCapabilityRequest {
    request_id: String,
    capability_id: String,
    access: String,
    scope: CapabilityScope,
    #[serde(default = "default_true")]
    required: bool,
    #[serde(default)]
    requires_enforcement: bool,
    #[serde(default)]
    reason: String,
    #[serde(default = "default_requested_by")]
    requested_by: String,
    #[serde(default)]
    policy_source: Option<String>,
}

/// One policy override for a capability id and optional scope.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityPolicyOverride {
    pub capability_id: String,
    pub decision: CapabilityPolicyDecision,
    #[serde(default)]
    pub scope: Option<CapabilityScope>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub requires_enforcement: Option<bool>,
}

impl CapabilityPolicyOverride {
    pub fn validate(&self) -> Result<(), CapabilityContractError> {
        validate_capability_id(&self.capability_id)?;
        if let Some(scope) = &self.scope {
            scope.validate()?;
        }
        Ok(())
    }

    fn from_raw(raw: RawCapabilityPolicyOverride) -> Result<Self, CapabilityContractError> {
        let override_ = Self {
            capability_id: validate_capability_id(&raw.capability_id)?,
            decision: raw.decision,
            scope: raw.scope,
            reason: raw.reason,
            requires_enforcement: raw.requires_enforcement,
        };
        override_.validate()?;
        Ok(override_)
    }
}

impl<'de> Deserialize<'de> for CapabilityPolicyOverride {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_raw(RawCapabilityPolicyOverride::deserialize(deserializer)?)
            .map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCapabilityPolicyOverride {
    capability_id: String,
    decision: CapabilityPolicyDecision,
    #[serde(default)]
    scope: Option<CapabilityScope>,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    requires_enforcement: Option<bool>,
}

/// Resolved grant for one execution capability request.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCapabilityGrant {
    pub grant_id: String,
    pub request_id: String,
    pub capability_id: String,
    pub access: String,
    pub scope: CapabilityScope,
    #[serde(default = "default_true")]
    pub required: bool,
    pub decision_state: CapabilityDecisionState,
    pub enforcement_mode: CapabilityEnforcementMode,
    #[serde(default)]
    pub approval_policy_ref: Option<ApprovalPolicyRef>,
    #[serde(default)]
    pub evidence_requirements: Vec<String>,
    pub evidence_status: CapabilityEvidenceStatus,
    pub decision_reason: String,
    pub resolved_by: String,
    #[serde(default)]
    pub fingerprint: String,
}

impl ExecutionCapabilityGrant {
    pub fn validate(&self) -> Result<(), CapabilityContractError> {
        require_non_empty("grant_id", &self.grant_id)?;
        require_non_empty("request_id", &self.request_id)?;
        validate_capability_id(&self.capability_id)?;
        validate_access(&self.access)?;
        self.scope.validate()?;
        if self.decision_state == CapabilityDecisionState::ApprovalRequired
            && self.approval_policy_ref.is_none()
        {
            return Err(CapabilityContractError::new(
                "approval_required grants require approval_policy_ref",
            ));
        }
        if self.decision_state != CapabilityDecisionState::Granted
            && self.enforcement_mode != CapabilityEnforcementMode::NotApplicable
        {
            return Err(CapabilityContractError::new(
                "non-granted capability decisions must use not_applicable enforcement",
            ));
        }
        if let Some(policy_ref) = &self.approval_policy_ref {
            policy_ref.validate()?;
        }
        validate_evidence_requirements(&self.evidence_requirements)?;
        require_non_empty("decision_reason", &self.decision_reason)?;
        require_non_empty("resolved_by", &self.resolved_by)?;
        if !self.fingerprint.is_empty() && !is_safe_fingerprint(&self.fingerprint) {
            return Err(CapabilityContractError::new(
                "fingerprint must use the grant-<hex> format",
            ));
        }
        Ok(())
    }

    fn from_raw(raw: RawExecutionCapabilityGrant) -> Result<Self, CapabilityContractError> {
        let mut grant = Self {
            grant_id: raw.grant_id.trim().to_owned(),
            request_id: raw.request_id.trim().to_owned(),
            capability_id: validate_capability_id(&raw.capability_id)?,
            access: raw.access,
            scope: raw.scope,
            required: raw.required,
            decision_state: raw.decision_state,
            enforcement_mode: raw.enforcement_mode,
            approval_policy_ref: raw.approval_policy_ref,
            evidence_requirements: normalize_evidence_requirements(
                raw.evidence_requirements.clone(),
            )?,
            evidence_status: raw.evidence_status.unwrap_or_else(|| {
                if raw.decision_state == CapabilityDecisionState::Granted
                    && !raw.evidence_requirements.is_empty()
                {
                    CapabilityEvidenceStatus::Pending
                } else {
                    CapabilityEvidenceStatus::NotRequired
                }
            }),
            decision_reason: raw.decision_reason.trim().to_owned(),
            resolved_by: raw.resolved_by.trim().to_owned(),
            fingerprint: raw.fingerprint,
        };
        grant.validate()?;
        if grant.fingerprint.is_empty() {
            grant.fingerprint = capability_grant_fingerprint(&grant);
        }
        Ok(grant)
    }
}

impl<'de> Deserialize<'de> for ExecutionCapabilityGrant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_raw(RawExecutionCapabilityGrant::deserialize(deserializer)?)
            .map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExecutionCapabilityGrant {
    grant_id: String,
    request_id: String,
    capability_id: String,
    access: String,
    scope: CapabilityScope,
    #[serde(default = "default_true")]
    required: bool,
    decision_state: CapabilityDecisionState,
    enforcement_mode: CapabilityEnforcementMode,
    #[serde(default)]
    approval_policy_ref: Option<ApprovalPolicyRef>,
    #[serde(default, deserialize_with = "deserialize_evidence_requirements")]
    evidence_requirements: Vec<String>,
    #[serde(default)]
    evidence_status: Option<CapabilityEvidenceStatus>,
    decision_reason: String,
    resolved_by: String,
    #[serde(default)]
    fingerprint: String,
}

/// Runner support decision for a resolved capability grant.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySupportDecision {
    pub runner_id: String,
    #[serde(default)]
    pub invocation_context_ref: String,
    pub grant_id: String,
    pub support_state: CapabilitySupportState,
    pub enforcement_mode: CapabilityEnforcementMode,
    #[serde(default)]
    pub limitations: Vec<String>,
    #[serde(default)]
    pub evidence_available: Vec<String>,
    pub reason: String,
}

impl CapabilitySupportDecision {
    pub fn validate(&self) -> Result<(), CapabilityContractError> {
        require_non_empty("runner_id", &self.runner_id)?;
        require_non_empty("grant_id", &self.grant_id)?;
        validate_string_list("limitations", &self.limitations)?;
        validate_string_list("evidence_available", &self.evidence_available)?;
        require_non_empty("reason", &self.reason)
    }

    fn from_raw(raw: RawCapabilitySupportDecision) -> Result<Self, CapabilityContractError> {
        let decision = Self {
            runner_id: raw.runner_id.trim().to_owned(),
            invocation_context_ref: raw.invocation_context_ref,
            grant_id: raw.grant_id.trim().to_owned(),
            support_state: raw.support_state,
            enforcement_mode: raw.enforcement_mode,
            limitations: normalize_string_list(raw.limitations)?,
            evidence_available: normalize_string_list(raw.evidence_available)?,
            reason: raw.reason.trim().to_owned(),
        };
        decision.validate()?;
        Ok(decision)
    }
}

impl<'de> Deserialize<'de> for CapabilitySupportDecision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_raw(RawCapabilitySupportDecision::deserialize(deserializer)?)
            .map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCapabilitySupportDecision {
    runner_id: String,
    #[serde(default)]
    invocation_context_ref: String,
    grant_id: String,
    support_state: CapabilitySupportState,
    enforcement_mode: CapabilityEnforcementMode,
    #[serde(default)]
    limitations: Vec<String>,
    #[serde(default)]
    evidence_available: Vec<String>,
    reason: String,
}

/// Operator-facing warning emitted while compiling or checking execution capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCapabilityWarning {
    pub warning_id: String,
    pub capability_id: String,
    pub severity: String,
    pub message: String,
}

impl ExecutionCapabilityWarning {
    pub fn validate(&self) -> Result<(), CapabilityContractError> {
        require_non_empty("warning_id", &self.warning_id)?;
        validate_capability_id(&self.capability_id)?;
        require_non_empty("severity", &self.severity)?;
        require_non_empty("message", &self.message)
    }
}

/// Stable, human-readable validation failure for capability contracts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityContractError {
    message: String,
}

impl CapabilityContractError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CapabilityContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CapabilityContractError {}

/// Computes the Python-compatible stable grant fingerprint.
#[must_use]
pub fn capability_grant_fingerprint(grant: &ExecutionCapabilityGrant) -> String {
    let mut value = serde_json::to_value(grant).expect("capability grant is serializable");
    if let Value::Object(object) = &mut value {
        object.remove("fingerprint");
    }
    let encoded = canonical_json(&value);
    let digest = Sha256::digest(encoded.as_bytes());
    format!("grant-{:x}", digest)[..18].to_owned()
}

fn workspace_path_scope_stays_inside(value: &str) -> bool {
    !value.starts_with('/') && !value.split('/').any(|part| part == "..")
}

fn validate_safe_capability_id(value: &str) -> Result<(), CapabilityContractError> {
    if value.trim() != value || value.is_empty() {
        return Err(CapabilityContractError::new(
            "capability_id must not include surrounding whitespace and must not be empty",
        ));
    }
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(CapabilityContractError::new("capability_id is required"));
    };
    if !first.is_ascii_alphanumeric()
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(CapabilityContractError::new(
            "capability_id must start with an ASCII alphanumeric character and then contain only ASCII alphanumeric characters, '.', '_', or '-'",
        ));
    }
    Ok(())
}

fn require_non_empty(field_name: &'static str, value: &str) -> Result<(), CapabilityContractError> {
    if value.trim().is_empty() {
        Err(CapabilityContractError::new(format!(
            "{field_name} is required"
        )))
    } else {
        Ok(())
    }
}

fn validate_access(value: &str) -> Result<(), CapabilityContractError> {
    if CAPABILITY_ACCESS_VALUES.contains(&value) {
        Ok(())
    } else {
        Err(CapabilityContractError::new(format!(
            "unknown capability access: {value}"
        )))
    }
}

fn deserialize_evidence_requirements<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let raw = match value {
        Value::Null => Vec::new(),
        Value::String(value) => vec![value],
        Value::Array(values) => values
            .into_iter()
            .map(value_to_requirement_string)
            .collect(),
        other => vec![value_to_requirement_string(other)],
    };
    normalize_evidence_requirements(raw).map_err(de::Error::custom)
}

fn value_to_requirement_string(value: Value) -> String {
    match value {
        Value::String(value) => value,
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(&value).unwrap_or_default(),
    }
}

fn normalize_evidence_requirements(
    values: Vec<String>,
) -> Result<Vec<String>, CapabilityContractError> {
    let mut normalized = Vec::new();
    for value in values {
        let evidence = value.trim();
        if evidence.is_empty() {
            return Err(CapabilityContractError::new(
                "evidence requirement values must not be empty",
            ));
        }
        if !normalized.iter().any(|existing| existing == evidence) {
            normalized.push(evidence.to_owned());
        }
    }
    Ok(normalized)
}

fn validate_evidence_requirements(values: &[String]) -> Result<(), CapabilityContractError> {
    validate_string_list("evidence_requirements", values)
}

fn normalize_string_list(values: Vec<String>) -> Result<Vec<String>, CapabilityContractError> {
    let mut normalized = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(CapabilityContractError::new(
                "list values must not be empty",
            ));
        }
        if !normalized.iter().any(|existing| existing == trimmed) {
            normalized.push(trimmed.to_owned());
        }
    }
    Ok(normalized)
}

fn validate_string_list(
    field_name: &'static str,
    values: &[String],
) -> Result<(), CapabilityContractError> {
    for value in values {
        if value.trim().is_empty() {
            return Err(CapabilityContractError::new(format!(
                "{field_name} values must not be empty"
            )));
        }
    }
    Ok(())
}

fn is_safe_fingerprint(value: &str) -> bool {
    let Some(suffix) = value.strip_prefix("grant-") else {
        return false;
    };
    suffix.len() == 12 && suffix.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            serde_json::to_string(value).expect("JSON scalar is serializable")
        }
        Value::Array(values) => {
            let inner = values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{inner}]")
        }
        Value::Object(values) => {
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            let inner = entries
                .into_iter()
                .map(|(key, value)| {
                    let key = serde_json::to_string(key).expect("JSON key is serializable");
                    format!("{key}:{}", canonical_json(value))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_requested_by() -> String {
    "stage".to_owned()
}

fn default_approval_gate_scope() -> String {
    "stage".to_owned()
}

fn default_required_approval_decision() -> String {
    "approved".to_owned()
}
