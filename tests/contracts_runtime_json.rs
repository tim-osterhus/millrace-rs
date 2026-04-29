#![recursion_limit = "256"]

use std::fmt::Debug;

use serde_json::{Value, json};

use millrace_ai::contracts::{
    CompileDiagnostics, ExecutionTerminalResult, MailboxCommandEnvelope, Plane,
    PlanningTerminalResult, RecoveryCounters, RuntimeErrorContext, RuntimeJsonContract,
    RuntimeJsonError, RuntimeSnapshot, StageName, StageResultEnvelope, TerminalResult, TokenUsage,
    UsageGovernanceLedgerEntry, UsageGovernanceState, UsageGovernanceSubscriptionWindow,
};

const NOW: &str = "2026-04-15T00:00:00Z";

fn round_trip_contract<T>(value: Value) -> T
where
    T: RuntimeJsonContract + PartialEq + Debug,
{
    let decoded = T::from_json_value(value).unwrap();
    let serialized = serde_json::to_value(&decoded).unwrap();
    let decoded_again = T::from_json_value(serialized).unwrap();
    assert_eq!(decoded_again, decoded);
    decoded
}

fn round_trip_stage_result(value: Value) -> StageResultEnvelope {
    let decoded = StageResultEnvelope::from_json_value(value).unwrap();
    let serialized = serde_json::to_value(&decoded).unwrap();
    let decoded_again = StageResultEnvelope::from_json_value(serialized).unwrap();
    assert_eq!(decoded_again, decoded);
    decoded
}

fn round_trip_runtime_error(value: Value) -> RuntimeErrorContext {
    let decoded = RuntimeErrorContext::from_json_value(value).unwrap();
    let serialized = serde_json::to_value(&decoded).unwrap();
    let decoded_again = RuntimeErrorContext::from_json_value(serialized).unwrap();
    assert_eq!(decoded_again, decoded);
    decoded
}

fn python_model_dump_fixture(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap()
}

fn assert_python_contract_fixture_round_trips<T>(expected: Value) -> T
where
    T: RuntimeJsonContract + PartialEq + Debug,
{
    let decoded = T::from_json_value(expected.clone()).unwrap();
    let serialized = serde_json::to_value(&decoded).unwrap();
    assert_eq!(serialized, expected);
    let decoded_again = T::from_json_value(serialized).unwrap();
    assert_eq!(decoded_again, decoded);
    decoded
}

fn assert_python_stage_result_fixture_round_trips(expected: Value) -> StageResultEnvelope {
    let decoded = StageResultEnvelope::from_json_value(expected.clone()).unwrap();
    let serialized = serde_json::to_value(&decoded).unwrap();
    assert_eq!(serialized, expected);
    let decoded_again = StageResultEnvelope::from_json_value(serialized).unwrap();
    assert_eq!(decoded_again, decoded);
    decoded
}

fn assert_python_runtime_error_fixture_round_trips(expected: Value) -> RuntimeErrorContext {
    let decoded = RuntimeErrorContext::from_json_value(expected.clone()).unwrap();
    let serialized = serde_json::to_value(&decoded).unwrap();
    assert_eq!(serialized, expected);
    let decoded_again = RuntimeErrorContext::from_json_value(serialized).unwrap();
    assert_eq!(decoded_again, decoded);
    decoded
}

fn snapshot_json() -> Value {
    json!({
        "schema_version": "1.0",
        "kind": "runtime_snapshot",
        "runtime_mode": "daemon",
        "process_running": true,
        "paused": false,
        "pause_sources": [],
        "stop_requested": false,
        "active_mode_id": "learning_codex",
        "execution_loop_id": "execution.standard",
        "planning_loop_id": "planning.standard",
        "learning_loop_id": "learning.standard",
        "loop_ids_by_plane": {
            "execution": "execution.standard",
            "planning": "planning.standard",
            "learning": "learning.standard"
        },
        "compiled_plan_id": "plan-001",
        "compiled_plan_path": "millrace-agents/state/compiled_plan.json",
        "active_plane": "execution",
        "active_stage": "builder",
        "active_node_id": "builder",
        "active_stage_kind_id": "builder",
        "active_run_id": "run-001",
        "active_work_item_kind": "task",
        "active_work_item_id": "task-001",
        "active_runs_by_plane": {
            "execution": {
                "plane": "execution",
                "stage": "builder",
                "node_id": "builder",
                "stage_kind_id": "builder",
                "run_id": "run-001",
                "request_kind": "active_work_item",
                "work_item_kind": "task",
                "work_item_id": "task-001",
                "active_since": NOW,
                "running_status_marker": "BUILDER_RUNNING"
            }
        },
        "execution_status_marker": "### BUILDER_RUNNING",
        "planning_status_marker": "### IDLE",
        "learning_status_marker": "### IDLE",
        "status_markers_by_plane": {
            "execution": "### BUILDER_RUNNING",
            "planning": "### IDLE",
            "learning": "### IDLE"
        },
        "queue_depth_execution": 2,
        "queue_depth_planning": 7,
        "queue_depth_learning": 0,
        "queue_depths_by_plane": {
            "execution": 2,
            "planning": 7,
            "learning": 0
        },
        "last_terminal_result": "UPDATE_COMPLETE",
        "last_stage_result_path": "millrace-agents/runs/run-000/stage_results/request-000.json",
        "current_failure_class": null,
        "troubleshoot_attempt_count": 0,
        "mechanic_attempt_count": 0,
        "fix_cycle_count": 0,
        "consultant_invocations": 0,
        "config_version": "cfg-001",
        "watcher_mode": "watch",
        "last_reload_outcome": null,
        "last_reload_error": null,
        "started_at": null,
        "active_since": NOW,
        "updated_at": NOW
    })
}

fn stage_result_json() -> Value {
    json!({
        "schema_version": "1.0",
        "kind": "stage_result",
        "run_id": "run-001",
        "plane": "execution",
        "stage": "builder",
        "node_id": "builder",
        "stage_kind_id": "builder",
        "work_item_kind": "task",
        "work_item_id": "task-001",
        "terminal_result": "BUILDER_COMPLETE",
        "result_class": "success",
        "summary_status_marker": "### BUILDER_COMPLETE",
        "success": true,
        "retryable": false,
        "exit_code": 0,
        "duration_seconds": 1.25,
        "prompt_artifact": "prompt.md",
        "report_artifact": "builder_summary.md",
        "artifact_paths": ["builder_summary.md"],
        "detected_marker": "### BUILDER_COMPLETE",
        "stdout_path": "stdout.txt",
        "stderr_path": "stderr.txt",
        "runner_name": "codex_cli",
        "model_name": "gpt-5",
        "model_reasoning_effort": "medium",
        "token_usage": {
            "input_tokens": 100,
            "cached_input_tokens": 20,
            "output_tokens": 30,
            "thinking_tokens": 5,
            "total_tokens": 135
        },
        "notes": ["builder pass"],
        "metadata": {"request_id": "request-001"},
        "started_at": NOW,
        "completed_at": NOW
    })
}

fn usage_governance_state_json() -> Value {
    json!({
        "version": "1.0",
        "enabled": true,
        "auto_resume": true,
        "auto_resume_possible": true,
        "evaluation_boundary": "between_stages",
        "calendar_timezone": "UTC",
        "daemon_session_id": "daemon-session",
        "last_evaluated_at": NOW,
        "active_blockers": [{
            "source": "runtime_token",
            "rule_id": "rolling-5h-default",
            "window": "rolling_5h",
            "observed": 125.0,
            "threshold": 100.0,
            "metric": "total_tokens",
            "auto_resume_possible": true,
            "next_auto_resume_at": "2026-04-15T05:00:00Z",
            "detail": ""
        }],
        "paused_by_governance": true,
        "next_auto_resume_at": "2026-04-15T05:00:00Z",
        "subscription_quota_status": {
            "enabled": true,
            "provider": "codex_chatgpt_oauth",
            "state": "healthy",
            "degraded_policy": "fail_open",
            "detail": null,
            "last_refreshed_at": NOW,
            "windows": {
                "five_hour": {
                    "window": "five_hour",
                    "percent_used": 42.0,
                    "resets_at": "2026-04-15T02:00:00Z",
                    "read_at": NOW
                }
            }
        }
    })
}

fn usage_governance_ledger_entry_json() -> Value {
    json!({
        "dedupe_key": "millrace-agents/runs/run-001/stage_results/request-001.json",
        "counted_at": NOW,
        "stage_completed_at": NOW,
        "plane": "execution",
        "run_id": "run-001",
        "stage_id": "builder",
        "work_item_kind": "task",
        "work_item_id": "task-001",
        "token_usage": {
            "input_tokens": 10,
            "cached_input_tokens": 2,
            "output_tokens": 5,
            "thinking_tokens": 1,
            "total_tokens": 16
        },
        "stage_result_path": "millrace-agents/runs/run-001/stage_results/request-001.json",
        "daemon_session_id": "daemon-session"
    })
}

#[test]
fn python_produced_runtime_json_fixtures_round_trip_against_rust_contracts() {
    // These fixtures are committed outputs from the adjacent Python
    // millrace_ai.contracts Pydantic models using model_dump_json.
    let snapshot = assert_python_contract_fixture_round_trips::<RuntimeSnapshot>(
        python_model_dump_fixture(include_str!("fixtures/runtime_json/runtime_snapshot.json")),
    );
    assert_eq!(snapshot.active_plane, Some(Plane::Execution));
    assert_eq!(
        snapshot
            .active_runs_by_plane
            .get(&Plane::Execution)
            .map(|active| active.run_id.as_str()),
        Some("run-001")
    );

    let counters = assert_python_contract_fixture_round_trips::<RecoveryCounters>(
        python_model_dump_fixture(include_str!("fixtures/runtime_json/recovery_counters.json")),
    );
    assert_eq!(counters.entries[0].failure_class, "missing_terminal_result");

    let mailbox = assert_python_contract_fixture_round_trips::<MailboxCommandEnvelope>(
        python_model_dump_fixture(include_str!(
            "fixtures/runtime_json/mailbox_command_envelope.json"
        )),
    );
    assert_eq!(mailbox.payload["reason"], "test");

    let diagnostics = assert_python_contract_fixture_round_trips::<CompileDiagnostics>(
        python_model_dump_fixture(include_str!(
            "fixtures/runtime_json/compile_diagnostics.json"
        )),
    );
    assert!(!diagnostics.ok);

    let stage_result = assert_python_stage_result_fixture_round_trips(python_model_dump_fixture(
        include_str!("fixtures/runtime_json/stage_result_envelope.json"),
    ));
    assert_eq!(
        stage_result.terminal_result,
        TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete)
    );

    let request_driven = assert_python_stage_result_fixture_round_trips(python_model_dump_fixture(
        include_str!("fixtures/runtime_json/stage_result_request_driven_terminal_identity.json"),
    ));
    assert_eq!(request_driven.stage, StageName::Builder);
    assert_eq!(
        request_driven.terminal_result,
        TerminalResult::Execution(ExecutionTerminalResult::CheckerPass)
    );
    assert_eq!(
        request_driven.detected_marker.as_deref(),
        Some("### CHECKER_PASS")
    );

    let runtime_error = assert_python_runtime_error_fixture_round_trips(python_model_dump_fixture(
        include_str!("fixtures/runtime_json/runtime_error_context.json"),
    ));
    assert_eq!(
        runtime_error.terminal_result,
        Some(TerminalResult::Planning(PlanningTerminalResult::Blocked))
    );

    let usage = assert_python_contract_fixture_round_trips::<TokenUsage>(
        python_model_dump_fixture(include_str!("fixtures/runtime_json/token_usage.json")),
    );
    assert_eq!(usage.total_tokens, 135);
}

#[test]
fn usage_governance_contracts_round_trip_and_reject_unsafe_state() {
    let state = round_trip_contract::<UsageGovernanceState>(usage_governance_state_json());
    assert_eq!(state.active_blockers[0].rule_id, "rolling-5h-default");
    assert_eq!(
        state
            .subscription_quota_status
            .windows
            .get(&UsageGovernanceSubscriptionWindow::FiveHour)
            .unwrap()
            .percent_used,
        42.0
    );

    let ledger =
        round_trip_contract::<UsageGovernanceLedgerEntry>(usage_governance_ledger_entry_json());
    assert_eq!(ledger.token_usage.total_tokens, 16);

    let mut contradictory = usage_governance_state_json();
    contradictory["paused_by_governance"] = json!(false);
    let error = UsageGovernanceState::from_json_value(contradictory).unwrap_err();
    assert!(error.to_string().contains("active governance blockers"));

    let mut bad_quota = usage_governance_state_json();
    bad_quota["subscription_quota_status"]["windows"]["five_hour"]["percent_used"] = json!(101);
    let error = UsageGovernanceState::from_json_value(bad_quota).unwrap_err();
    assert!(error.to_string().contains("percent_used"));

    let mut bad_ledger = usage_governance_ledger_entry_json();
    bad_ledger["dedupe_key"] = json!("different.json");
    let error = UsageGovernanceLedgerEntry::from_json_value(bad_ledger).unwrap_err();
    assert!(error.to_string().contains("dedupe_key"));
}

#[test]
fn runtime_snapshot_round_trips_python_shaped_active_state() {
    let snapshot = round_trip_contract::<RuntimeSnapshot>(snapshot_json());

    assert_eq!(snapshot.active_plane, Some(Plane::Execution));
    assert_eq!(snapshot.active_stage, Some(StageName::Builder));
    assert_eq!(snapshot.active_run_id.as_deref(), Some("run-001"));
    assert_eq!(
        snapshot
            .loop_ids_by_plane
            .get(&Plane::Learning)
            .map(String::as_str),
        Some("learning.standard")
    );
}

#[test]
fn runtime_snapshot_migrates_legacy_active_state() {
    let mut raw = snapshot_json();
    raw["active_runs_by_plane"] = json!({});

    let snapshot = RuntimeSnapshot::from_json_value(raw).unwrap();

    let active = snapshot
        .active_runs_by_plane
        .get(&Plane::Execution)
        .expect("execution active run should be synthesized");
    assert_eq!(active.work_item_id.as_deref(), Some("task-001"));
    assert_eq!(active.run_id, "run-001");
}

#[test]
fn simple_runtime_json_artifacts_round_trip() {
    let counters = round_trip_contract::<RecoveryCounters>(json!({
        "schema_version": "1.0",
        "kind": "recovery_counters",
        "entries": [{
            "failure_class": "missing_terminal_result",
            "work_item_id": "task-001",
            "work_item_kind": "task",
            "troubleshoot_attempt_count": 1,
            "mechanic_attempt_count": 0,
            "fix_cycle_count": 0,
            "consultant_invocations": 0,
            "last_updated_at": NOW
        }]
    }));
    assert_eq!(counters.entries[0].failure_class, "missing_terminal_result");

    let mailbox = round_trip_contract::<MailboxCommandEnvelope>(json!({
        "schema_version": "1.0",
        "kind": "mailbox_command",
        "command_id": "cmd-001",
        "command": "reload_config",
        "issued_at": NOW,
        "issuer": "operator",
        "payload": {"reason": "test"}
    }));
    assert_eq!(mailbox.payload["reason"], "test");

    let diagnostics = round_trip_contract::<CompileDiagnostics>(json!({
        "schema_version": "1.0",
        "kind": "compile_diagnostics",
        "ok": false,
        "mode_id": "standard_plain",
        "errors": ["missing loop"],
        "warnings": ["deprecated alias"],
        "emitted_at": NOW
    }));
    assert!(!diagnostics.ok);

    let usage = round_trip_contract::<TokenUsage>(json!({
        "input_tokens": 100,
        "cached_input_tokens": 20,
        "output_tokens": 30,
        "thinking_tokens": 5,
        "total_tokens": 135
    }));
    assert_eq!(usage.total_tokens, 135);
}

#[test]
fn stage_result_and_runtime_error_round_trip_with_plane_qualified_terminal_results() {
    let stage_result = round_trip_stage_result(stage_result_json());
    assert_eq!(
        stage_result.terminal_result,
        TerminalResult::Execution(ExecutionTerminalResult::BuilderComplete)
    );

    let runtime_error = round_trip_runtime_error(json!({
        "schema_version": "1.0",
        "kind": "runtime_error_context",
        "error_code": "planning_post_stage_apply_failed",
        "plane": "planning",
        "failed_stage": "manager",
        "repair_stage": "mechanic",
        "work_item_kind": "spec",
        "work_item_id": "spec-001",
        "run_id": "run-001",
        "router_action": "route_to_mechanic",
        "terminal_result": "BLOCKED",
        "stage_result_path": "millrace-agents/runs/run-001/stage_results/request-001.json",
        "report_path": "millrace-agents/runs/run-001/troubleshoot_report.md",
        "exception_type": "RuntimeError",
        "exception_message": "post-stage apply failed",
        "captured_at": NOW
    }));
    assert_eq!(
        runtime_error.terminal_result,
        Some(TerminalResult::Planning(PlanningTerminalResult::Blocked))
    );
}

#[test]
fn stage_result_accepts_request_driven_terminal_identity() {
    let mut request_driven = stage_result_json();
    request_driven["terminal_result"] = json!("CHECKER_PASS");
    request_driven["summary_status_marker"] = json!("### CHECKER_PASS");
    request_driven["detected_marker"] = json!("### CHECKER_PASS");

    let decoded = round_trip_stage_result(request_driven);

    assert_eq!(decoded.stage, StageName::Builder);
    assert_eq!(
        decoded.terminal_result,
        TerminalResult::Execution(ExecutionTerminalResult::CheckerPass)
    );
    assert!(decoded.success);
}

#[test]
fn runtime_json_contracts_reject_bad_required_enum_and_timestamp_fields() {
    let mut missing_required = snapshot_json();
    missing_required
        .as_object_mut()
        .unwrap()
        .remove("runtime_mode");
    assert!(matches!(
        RuntimeSnapshot::from_json_value(missing_required),
        Err(RuntimeJsonError::Json { .. })
    ));

    let mut bad_enum = snapshot_json();
    bad_enum["runtime_mode"] = json!("background");
    let error = RuntimeSnapshot::from_json_value(bad_enum).unwrap_err();
    assert!(error.to_string().contains("RuntimeMode"));

    let mut bad_timestamp = snapshot_json();
    bad_timestamp["updated_at"] = json!("not-a-timestamp");
    let error = RuntimeSnapshot::from_json_value(bad_timestamp).unwrap_err();
    assert!(error.to_string().contains("timestamp"));
}

#[test]
fn invalid_stage_result_semantics_fail_with_typed_errors() {
    let mut bad_marker = stage_result_json();
    bad_marker["summary_status_marker"] = json!("### CHECKER_PASS");
    let error = StageResultEnvelope::from_json_value(bad_marker).unwrap_err();
    assert!(error.to_string().contains("summary_status_marker"));

    let mut mismatched_detected = stage_result_json();
    mismatched_detected["terminal_result"] = json!("CHECKER_PASS");
    mismatched_detected["summary_status_marker"] = json!("### CHECKER_PASS");
    mismatched_detected["detected_marker"] = json!("### BUILDER_COMPLETE");
    let error = StageResultEnvelope::from_json_value(mismatched_detected).unwrap_err();
    assert!(error.to_string().contains("detected_marker"));

    let mut unknown_terminal = stage_result_json();
    unknown_terminal["terminal_result"] = json!("NOT_A_TERMINAL");
    let error = StageResultEnvelope::from_json_value(unknown_terminal).unwrap_err();
    assert!(matches!(error, RuntimeJsonError::Contract(_)));

    let mut negative_duration = stage_result_json();
    negative_duration["duration_seconds"] = json!(-1.0);
    let error = StageResultEnvelope::from_json_value(negative_duration).unwrap_err();
    assert!(matches!(
        error,
        RuntimeJsonError::InvalidField {
            field_name: "duration_seconds",
            ..
        }
    ));

    let mut backward_completion = stage_result_json();
    backward_completion["completed_at"] = json!("2026-04-14T23:59:59Z");
    let error = StageResultEnvelope::from_json_value(backward_completion).unwrap_err();
    assert!(error.to_string().contains("completed_at"));

    let mut success_class_without_success = stage_result_json();
    success_class_without_success["success"] = json!(false);
    let error = StageResultEnvelope::from_json_value(success_class_without_success).unwrap_err();
    assert!(error.to_string().contains("success result_class"));

    let mut non_success_class_with_success = stage_result_json();
    non_success_class_with_success["result_class"] = json!("blocked");
    let error = StageResultEnvelope::from_json_value(non_success_class_with_success).unwrap_err();
    assert!(error.to_string().contains("non-success result_class"));
}

#[test]
fn invalid_runtime_artifact_relationships_fail_clearly() {
    let mut snapshot = snapshot_json();
    snapshot["active_runs_by_plane"]["execution"]["stage"] = json!("planner");
    let error = RuntimeSnapshot::from_json_value(snapshot).unwrap_err();
    assert!(error.to_string().contains("active run stage"));

    let mut command = json!({
        "schema_version": "1.0",
        "kind": "mailbox_command",
        "command_id": "cmd-001",
        "command": "start",
        "issued_at": NOW,
        "issuer": "operator"
    });
    let error = MailboxCommandEnvelope::from_json_value(command.take()).unwrap_err();
    assert!(error.to_string().contains("MailboxCommand"));

    let diagnostics = json!({
        "schema_version": "1.0",
        "kind": "compile_diagnostics",
        "ok": false,
        "mode_id": "standard_plain",
        "errors": [],
        "emitted_at": NOW
    });
    let error = CompileDiagnostics::from_json_value(diagnostics).unwrap_err();
    assert!(error.to_string().contains("errors are required"));
}
