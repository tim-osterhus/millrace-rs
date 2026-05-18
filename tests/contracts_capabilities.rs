use serde_json::{Value, json};
use tempfile::TempDir;

use millrace_ai::contracts::{
    ApprovalPolicyRef, CapabilityDecisionState, CapabilityEnforcementMode,
    CapabilityEvidenceStatus, CapabilityPolicyDecision, CapabilityPolicyOverride,
    CapabilityRequest, CapabilitySupportDecision, CapabilitySupportState, ExecutionCapabilityGrant,
    MailboxExecutionCapabilityApprovalPayload, capability_grant_fingerprint,
    capability_key_aliases, normalize_capability_id,
};
use millrace_ai::{
    RuntimeConfigApplyBoundary, load_runtime_startup_config,
    runtime_config_apply_boundary_for_field,
};

fn decode<T>(value: Value) -> T
where
    T: serde::de::DeserializeOwned + serde::Serialize + PartialEq + std::fmt::Debug,
{
    let decoded: T = serde_json::from_value(value).unwrap();
    let encoded = serde_json::to_value(&decoded).unwrap();
    let decoded_again: T = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded_again, decoded);
    decoded
}

#[test]
fn capability_request_normalizes_aliases_and_validates_scopes() {
    let request: CapabilityRequest = decode(json!({
        "request_id": "builder-workspace-write",
        "capability_id": "workspace_write",
        "access": "write",
        "scope": {"kind": "workspace_path", "value": "src/contracts"},
        "reason": "Builder edits contract files."
    }));

    assert_eq!(request.capability_id, "workspace.write");
    assert_eq!(request.scope.kind, "workspace_path");
    assert_eq!(request.scope.value, "src/contracts");
    assert!(request.required);
    assert_eq!(
        normalize_capability_id("package_install"),
        "package.install"
    );
    assert_eq!(capability_key_aliases()["git_mutate"], "git.mutate");

    let path_error = serde_json::from_value::<CapabilityRequest>(json!({
        "request_id": "bad-path",
        "capability_id": "workspace.read",
        "access": "read",
        "scope": {"kind": "workspace_path", "value": "../secrets"}
    }))
    .unwrap_err();
    assert!(
        path_error
            .to_string()
            .contains("workspace_path scope must stay inside workspace")
    );

    let unsafe_id = serde_json::from_value::<CapabilityRequest>(json!({
        "request_id": "unsafe-id",
        "capability_id": "../network.access",
        "access": "execute",
        "scope": {"kind": "network_class", "value": "raw"}
    }))
    .unwrap_err();
    assert!(unsafe_id.to_string().contains("capability_id"));

    let unknown = serde_json::from_value::<CapabilityRequest>(json!({
        "request_id": "unknown",
        "capability_id": "external.magic",
        "access": "execute",
        "scope": {"kind": "runner", "value": "codex_cli"}
    }))
    .unwrap_err();
    assert!(unknown.to_string().contains("unknown capability_id"));
}

#[test]
fn capability_grants_validate_approval_invariants_and_fingerprint_inputs() {
    let missing_policy = serde_json::from_value::<ExecutionCapabilityGrant>(json!({
        "grant_id": "grant-package-install",
        "request_id": "package-install",
        "capability_id": "package.install",
        "access": "execute",
        "scope": {"kind": "package_manager", "value": "uv"},
        "decision_state": "approval_required",
        "enforcement_mode": "not_applicable",
        "decision_reason": "package installs require approval",
        "resolved_by": "runtime_config"
    }))
    .unwrap_err();
    assert!(
        missing_policy
            .to_string()
            .contains("approval_required grants require approval_policy_ref")
    );

    let invalid_enforcement = serde_json::from_value::<ExecutionCapabilityGrant>(json!({
        "grant_id": "grant-network",
        "request_id": "network-raw",
        "capability_id": "network.access",
        "access": "execute",
        "scope": {"kind": "network_class", "value": "raw"},
        "decision_state": "denied",
        "enforcement_mode": "runtime_enforced",
        "decision_reason": "raw network is denied",
        "resolved_by": "runtime_config"
    }))
    .unwrap_err();
    assert!(
        invalid_enforcement
            .to_string()
            .contains("non-granted capability decisions must use not_applicable enforcement")
    );

    let grant: ExecutionCapabilityGrant = decode(json!({
        "grant_id": "grant-shell-run",
        "request_id": "shell-run",
        "capability_id": "shell.run",
        "access": "execute",
        "scope": {"kind": "command_class", "value": "test"},
        "decision_state": "granted",
        "enforcement_mode": "advisory_only",
        "evidence_requirements": ["runner_invocation", "runner_completion", "runner_invocation"],
        "decision_reason": "allowed by default policy",
        "resolved_by": "default_policy"
    }));

    assert_eq!(grant.evidence_status, CapabilityEvidenceStatus::Pending);
    assert_eq!(
        grant.evidence_requirements,
        ["runner_invocation", "runner_completion"]
    );
    assert_eq!(grant.fingerprint, capability_grant_fingerprint(&grant));

    let changed_reason: ExecutionCapabilityGrant = decode(json!({
        "grant_id": "grant-shell-run",
        "request_id": "shell-run",
        "capability_id": "shell.run",
        "access": "execute",
        "scope": {"kind": "command_class", "value": "test"},
        "decision_state": "granted",
        "enforcement_mode": "advisory_only",
        "evidence_requirements": ["runner_invocation", "runner_completion"],
        "decision_reason": "different policy reason",
        "resolved_by": "default_policy"
    }));
    assert_ne!(grant.fingerprint, changed_reason.fingerprint);
}

#[test]
fn capability_policy_support_and_approval_payload_contracts_round_trip() {
    let policy_ref: ApprovalPolicyRef = decode(json!({
        "policy_id": "operator"
    }));
    assert_eq!(policy_ref.gate_scope, "stage");
    assert_eq!(policy_ref.required_decision, "approved");

    let override_: CapabilityPolicyOverride = decode(json!({
        "capability_id": "git_mutate",
        "decision": "approval_required",
        "scope": {"kind": "git_action", "value": "commit"},
        "reason": "git mutation requires operator approval",
        "requires_enforcement": true
    }));
    assert_eq!(override_.capability_id, "git.mutate");
    assert_eq!(
        override_.decision,
        CapabilityPolicyDecision::ApprovalRequired
    );

    let support: CapabilitySupportDecision = decode(json!({
        "runner_id": "codex_cli",
        "grant_id": "grant-shell-run",
        "support_state": "partially_supported",
        "enforcement_mode": "advisory_only",
        "limitations": ["runner cannot enforce shell class directly"],
        "evidence_available": ["runner_invocation"],
        "reason": "adapter records evidence only"
    }));
    assert_eq!(
        support.support_state,
        CapabilitySupportState::PartiallySupported
    );

    let approval = MailboxExecutionCapabilityApprovalPayload::from_json_value(json!({
        "approval_id": "approval-run-001",
        "reason": "operator approved package install"
    }))
    .unwrap();
    assert_eq!(approval.approval_id, "approval-run-001");

    let unsafe_approval = MailboxExecutionCapabilityApprovalPayload::from_json_value(json!({
        "approval_id": "../approval-run-001",
        "reason": "operator approved package install"
    }))
    .unwrap_err();
    assert!(unsafe_approval.to_string().contains("approval_id"));

    let empty_reason = MailboxExecutionCapabilityApprovalPayload::from_json_value(json!({
        "approval_id": "approval-run-001",
        "reason": ""
    }))
    .unwrap_err();
    assert!(empty_reason.to_string().contains("reason"));
}

#[test]
fn execution_capabilities_config_defaults_and_overrides_match_python() {
    let temp_dir = TempDir::new().unwrap();
    let missing_config = temp_dir.path().join("missing.toml");
    let defaults = load_runtime_startup_config(&missing_config).unwrap();

    assert!(defaults.execution_capabilities.enabled);
    assert_eq!(
        defaults.execution_capabilities.default_unknown_capability,
        CapabilityPolicyDecision::Deny
    );
    assert!(defaults.execution_capabilities.allow_advisory_grants);
    assert!(!defaults.execution_capabilities.fail_required_advisory);
    assert_eq!(
        defaults.execution_capabilities.defaults["network.access"],
        CapabilityPolicyDecision::Deny
    );
    assert_eq!(
        defaults.execution_capabilities.defaults["package.install"],
        CapabilityPolicyDecision::ApprovalRequired
    );
    assert_eq!(
        defaults.execution_capabilities.defaults["git.mutate"],
        CapabilityPolicyDecision::ApprovalRequired
    );
    assert_eq!(
        defaults.execution_capabilities.defaults["shell.run"],
        CapabilityPolicyDecision::Allow
    );
    assert_eq!(
        defaults.execution_capabilities.defaults["workspace.write"],
        CapabilityPolicyDecision::Allow
    );

    for field in [
        "execution_capabilities.enabled",
        "execution_capabilities.default_unknown_capability",
        "execution_capabilities.allow_advisory_grants",
        "execution_capabilities.fail_required_advisory",
        "execution_capabilities.defaults",
    ] {
        assert_eq!(
            runtime_config_apply_boundary_for_field(field).unwrap(),
            RuntimeConfigApplyBoundary::Recompile
        );
    }

    let custom_config = temp_dir.path().join("custom.toml");
    std::fs::write(
        &custom_config,
        [
            "[execution_capabilities]",
            "enabled = false",
            "default_unknown_capability = \"approval_required\"",
            "allow_advisory_grants = false",
            "fail_required_advisory = true",
            "",
            "[execution_capabilities.defaults]",
            "package_install = \"deny\"",
            "\"workspace.write\" = \"allow\"",
        ]
        .join("\n"),
    )
    .unwrap();
    let custom = load_runtime_startup_config(&custom_config).unwrap();
    assert!(!custom.execution_capabilities.enabled);
    assert_eq!(
        custom.execution_capabilities.default_unknown_capability,
        CapabilityPolicyDecision::ApprovalRequired
    );
    assert!(!custom.execution_capabilities.allow_advisory_grants);
    assert!(custom.execution_capabilities.fail_required_advisory);
    assert_eq!(
        custom.execution_capabilities.defaults["package.install"],
        CapabilityPolicyDecision::Deny
    );

    let invalid = temp_dir.path().join("invalid.toml");
    std::fs::write(
        &invalid,
        "[execution_capabilities.defaults]\n\"../network.access\" = \"allow\"\n",
    )
    .unwrap();
    let error = load_runtime_startup_config(&invalid).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("execution_capabilities.defaults")
    );

    assert_eq!(CapabilityDecisionState::Granted.as_str(), "granted");
    assert_eq!(
        CapabilityEnforcementMode::NotApplicable.as_str(),
        "not_applicable"
    );
}
