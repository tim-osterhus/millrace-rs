#![recursion_limit = "256"]

use std::fmt::Debug;

use serde_json::{Value, json};
use tempfile::TempDir;

use millrace_ai::contracts::{
    CompileDiagnostics, ExecutionTerminalResult, LearningTerminalResult, MailboxAddProbePayload,
    MailboxCommandEnvelope, Plane, PlanningTerminalResult, ReadOnlyStatusPayload, ReconDecision,
    ReconHandoffTarget, ReconPacketDocument, ReconPacketError, RecoveryCounters, ResultClass,
    RunTraceGraph, RuntimeErrorContext, RuntimeJsonContract, RuntimeJsonError, RuntimeMode,
    RuntimeSnapshot, StageName, StageResultEnvelope, TerminalResult, TokenUsage,
    UsageGovernanceLedgerEntry, UsageGovernanceState, UsageGovernanceSubscriptionWindow,
};
use millrace_ai::recon_packets::{parse_recon_packet, read_recon_packet, render_recon_packet};

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
        "thinking_level": "medium",
        "model_reasoning_effort": "medium",
        "token_usage": {
            "input_tokens": 100,
            "cached_input_tokens": 20,
            "output_tokens": 30,
            "thinking_tokens": 5,
            "total_tokens": 135
        },
        "notes": ["builder pass"],
        "metadata": {
            "request_id": "request-001",
            "active_work_item_kind": "task",
            "active_work_item_id": "task-001",
            "active_work_item_path": "millrace-agents/tasks/active/task-001.md"
        },
        "started_at": NOW,
        "completed_at": NOW
    })
}

fn run_trace_graph_json() -> Value {
    json!({
        "schema_version": "1.0",
        "kind": "run_trace_graph",
        "run_id": "run-001",
        "run_dir": "/tmp/workspace/millrace-agents/runs/run-001",
        "compiled_plan_id": "plan-001",
        "mode_id": "default_codex",
        "request_kind": "active_work_item",
        "work_item_kind": "task",
        "work_item_id": "task-001",
        "closure_target_root_spec_id": null,
        "status": "active",
        "started_at": NOW,
        "completed_at": NOW,
        "duration_seconds": 1.25,
        "nodes": [{
            "trace_node_id": "request-001",
            "run_id": "run-001",
            "request_id": "request-001",
            "plane": "execution",
            "stage": "builder",
            "node_id": "builder",
            "stage_kind_id": "builder",
            "compiled_plan_id": "plan-001",
            "mode_id": "default_codex",
            "request_kind": "active_work_item",
            "work_item_kind": "task",
            "work_item_id": "task-001",
            "closure_target_root_spec_id": null,
            "terminal_result": "BUILDER_COMPLETE",
            "result_class": "success",
            "failure_class": null,
            "runner_name": "codex_cli",
            "model_name": "gpt-5",
            "thinking_level": "medium",
            "model_reasoning_effort": "medium",
            "started_at": NOW,
            "completed_at": NOW,
            "duration_seconds": 1.25,
            "token_usage": {
                "input_tokens": 100,
                "cached_input_tokens": 20,
                "output_tokens": 30,
                "thinking_tokens": 5,
                "total_tokens": 135
            },
            "artifacts": [{
                "path": "stage_results/request-001.json",
                "kind": "stage_result",
                "size_bytes": 256,
                "sha256": null
            }]
        }],
        "edges": [{
            "trace_edge_id": "request-001--BUILDER_COMPLETE--checker",
            "source_trace_node_id": "request-001",
            "outcome": "BUILDER_COMPLETE",
            "edge_kind": "run_stage",
            "target_node_id": "checker",
            "target_trace_node_id": null,
            "terminal_state_id": null,
            "spawned_work": [{
                "kind": "learning_request",
                "item_id": "learn-001",
                "path": "millrace-agents/learning/requests/queue/learn-001.md",
                "reason": "learning_trigger",
                "source_stage_node_id": "builder",
                "source_terminal_result": "BUILDER_COMPLETE"
            }],
            "decision_reason": "builder:BUILDER_COMPLETE",
            "decided_at": NOW
        }],
        "notes": ["trace generated by runtime"],
        "generated_at": NOW
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
    assert_eq!(stage_result.metadata["active_work_item_kind"], "task");
    assert_eq!(stage_result.metadata["active_work_item_id"], "task-001");
    assert_eq!(
        stage_result.metadata["active_work_item_path"],
        "millrace-agents/tasks/active/task-001.md"
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
fn read_only_status_payload_serializes_python_compatible_json_fields() {
    let payload = ReadOnlyStatusPayload {
        workspace: "/tmp/workspace".to_owned(),
        runtime_mode: RuntimeMode::Daemon,
        process_running: true,
        runtime_ownership_lock: "active".to_owned(),
        paused: false,
        pause_sources: "none".to_owned(),
        stop_requested: false,
        active_mode_id: "default_codex".to_owned(),
        compiled_plan_id: "plan-001".to_owned(),
        compiled_plan_currentness: "current".to_owned(),
        active_plane: Some(Plane::Planning),
        active_stage: Some(StageName::Mechanic),
        active_node_id: Some("mechanic".to_owned()),
        active_stage_kind_id: Some("mechanic".to_owned()),
        active_work_item_kind: None,
        active_work_item_id: None,
        active_run_count: 0,
        execution_queue_depth: 0,
        planning_queue_depth: 0,
        learning_queue_depth: 0,
        execution_status_marker: "### IDLE".to_owned(),
        planning_status_marker: "### BLOCKED".to_owned(),
        learning_status_marker: "### IDLE".to_owned(),
        blocked_idle: true,
        current_failure_class: Some("recon_handoff_invalid".to_owned()),
        latest_runtime_error_report_path: Some(
            "millrace-agents/runs/run-001/runtime_error_report.md".to_owned(),
        ),
        closure_target_root_spec_id: json!("spec-root-001"),
        closure_target_open: json!(true),
        closure_target_blocked_by_lineage_work: json!(true),
        planning_root_specs_deferred_by_closure_target: json!(0),
        closure_target_latest_verdict_path: Value::Null,
        closure_target_latest_report_path: Value::Null,
    };

    let value = serde_json::to_value(&payload).expect("serialize status payload");
    assert_eq!(value["runtime_mode"], "daemon");
    assert_eq!(value["active_plane"], "planning");
    assert_eq!(value["active_stage"], "mechanic");
    assert_eq!(value["blocked_idle"], true);
    assert_eq!(value["current_failure_class"], "recon_handoff_invalid");
    assert_eq!(value["closure_target_open"], true);

    let decoded: ReadOnlyStatusPayload =
        serde_json::from_value(value).expect("decode status payload");
    assert_eq!(decoded, payload);
}

#[test]
fn python_v0_17_4_stage_result_no_op_runtime_json_fixture_round_trips_as_non_success() {
    let no_op = assert_python_stage_result_fixture_round_trips(python_model_dump_fixture(
        include_str!("fixtures/runtime_json/stage_result_learning_noop.json"),
    ));

    assert_eq!(no_op.stage, StageName::Analyst);
    assert_eq!(
        no_op.terminal_result,
        TerminalResult::Learning(LearningTerminalResult::AnalystNoop)
    );
    assert_eq!(no_op.result_class, ResultClass::NoOp);
    assert!(!no_op.success);
    assert_eq!(no_op.work_item_kind.as_str(), "learning_request");
    assert_eq!(no_op.metadata["request_kind"], "learning_request");
}

#[test]
fn auto_port_v0_18_0_runtime_contract_scout_pins_graph_and_trace_sources() {
    let fixture: Value = python_model_dump_fixture(include_str!(
        "fixtures/runtime_json/auto_port_v0_18_0_runtime_contract_scout.json"
    ));
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["kind"], "auto_port_v0_18_0_runtime_contract_scout");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.17.4");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.0");
    assert_eq!(
        fixture["python_reference"]["previous_commit"],
        "304e537964ff772c815689b87e4c1e3b805c656c"
    );
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "e4ccf099c8345a8b8708cdaa1ac510bdc7851387"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.17.4..v0.18.0"
    );

    let sources = fixture["contract_sources"]
        .as_array()
        .expect("contract source references are present");
    for source_path in [
        "../millrace-py/src/millrace_ai/contracts/graph_exports.py",
        "../millrace-py/src/millrace_ai/contracts/run_trace.py",
        "../millrace-py/src/millrace_ai/runtime/run_traces.py",
        "../millrace-py/tests/integration/test_graph_exports.py",
        "../millrace-py/tests/runtime/test_run_traces.py",
    ] {
        assert!(
            sources
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "missing v0.18.0 runtime contract source {source_path}"
        );
    }

    let targets = fixture["expected_rust_contract_targets"]
        .as_array()
        .expect("expected Rust contract targets are present");
    for target_path in [
        "src/contracts/graph_exports.rs",
        "src/contracts/run_trace.rs",
        "src/runtime/run_traces.rs",
        "tests/contracts_runtime_json.rs",
        "tests/runtime_serial.rs",
        "tests/runtime_daemon.rs",
    ] {
        assert!(
            targets
                .iter()
                .any(|value| value.as_str() == Some(target_path)),
            "missing v0.18.0 Rust contract target {target_path}"
        );
    }

    let guarantees = fixture["no_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
    ] {
        assert!(
            guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.18.0 runtime contract scout guarantee {guarantee}"
        );
    }
}

#[test]
fn auto_port_v0_18_1_runtime_contract_scout_pins_probe_recon_sources() {
    let fixture: Value = python_model_dump_fixture(include_str!(
        "fixtures/runtime_json/auto_port_v0_18_1_runtime_contract_scout.json"
    ));
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["kind"], "auto_port_v0_18_1_runtime_contract_scout");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.0");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.1");
    assert_eq!(
        fixture["python_reference"]["previous_commit"],
        "e4ccf099c8345a8b8708cdaa1ac510bdc7851387"
    );
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "0396c7852793b212d31345862b38a7d6f3f02854"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.0..v0.18.1"
    );

    let sources = fixture["contract_sources"]
        .as_array()
        .expect("contract source references are present");
    for source_path in [
        "../millrace-py/src/millrace_ai/contracts/enums.py",
        "../millrace-py/src/millrace_ai/contracts/mailbox.py",
        "../millrace-py/src/millrace_ai/contracts/recon.py",
        "../millrace-py/src/millrace_ai/contracts/stage_metadata.py",
        "../millrace-py/src/millrace_ai/contracts/work_documents.py",
        "../millrace-py/src/millrace_ai/recon_packets.py",
        "../millrace-py/src/millrace_ai/runtime/activation.py",
        "../millrace-py/src/millrace_ai/runtime/graph_authority/planning.py",
        "../millrace-py/src/millrace_ai/runtime/graph_authority/stage_mapping.py",
        "../millrace-py/src/millrace_ai/runtime/mailbox_intake.py",
        "../millrace-py/src/millrace_ai/runtime/recon_transitions.py",
        "../millrace-py/src/millrace_ai/runtime/result_application.py",
        "../millrace-py/src/millrace_ai/runtime/stage_requests.py",
        "../millrace-py/src/millrace_ai/workspace/paths.py",
        "../millrace-py/src/millrace_ai/workspace/queue_selection.py",
        "../millrace-py/src/millrace_ai/workspace/queue_store.py",
        "../millrace-py/src/millrace_ai/workspace/queue_transitions.py",
        "../millrace-py/src/millrace_ai/workspace/work_documents.py",
        "../millrace-py/tests/runtime/test_graph_authority.py",
        "../millrace-py/tests/runtime/test_result_application.py",
        "../millrace-py/tests/workspace/test_paths.py",
        "../millrace-py/tests/workspace/test_queue_store.py",
    ] {
        assert!(
            sources
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "missing v0.18.1 runtime/probe/recon source {source_path}"
        );
    }

    let targets = fixture["expected_rust_contract_targets"]
        .as_array()
        .expect("expected Rust contract targets are present");
    for target_path in [
        "src/contracts/enums.rs",
        "src/contracts/stage_metadata.rs",
        "src/contracts/work_documents.rs",
        "src/contracts/runtime_json.rs",
        "src/work_documents.rs",
        "src/workspace.rs",
        "src/workspace/queue_store.rs",
        "src/workspace/runtime_control.rs",
        "src/runtime/startup.rs",
        "src/runtime/tick.rs",
        "src/runtime/supervisor.rs",
        "tests/contracts_runtime_json.rs",
        "tests/runtime_serial.rs",
        "tests/runtime_daemon.rs",
        "tests/parity_cli.rs",
    ] {
        assert!(
            targets
                .iter()
                .any(|value| value.as_str() == Some(target_path)),
            "missing v0.18.1 Rust contract target {target_path}"
        );
    }

    let guarantees = fixture["no_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
        "no release upload",
        "no publishing",
    ] {
        assert!(
            guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.18.1 runtime contract scout guarantee {guarantee}"
        );
    }
}

#[test]
fn auto_port_v0_18_2_runtime_contract_scout_pins_status_recon_ownership_sources() {
    let fixture: Value = python_model_dump_fixture(include_str!(
        "fixtures/runtime_json/auto_port_v0_18_2_runtime_contract_scout.json"
    ));
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["kind"], "auto_port_v0_18_2_runtime_contract_scout");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.1");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.2");
    assert_eq!(
        fixture["python_reference"]["previous_commit"],
        "0396c7852793b212d31345862b38a7d6f3f02854"
    );
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "5444cb9485ea90b67b2ed6ba7e0723ae9fe7b79f"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.1..v0.18.2"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.3.1"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_version_role"],
        "previous_baseline_for_python_v0.18.1"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.2");
    assert_ne!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.18.2 runtime scout must not treat Rust 0.3.1 as the target"
    );

    let sources = fixture["contract_sources"]
        .as_array()
        .expect("contract source references are present");
    for source_path in [
        "../millrace-py/src/millrace_ai/cli/commands/status.py",
        "../millrace-py/src/millrace_ai/cli/status_view.py",
        "../millrace-py/src/millrace_ai/contracts/enums.py",
        "../millrace-py/src/millrace_ai/contracts/recon.py",
        "../millrace-py/src/millrace_ai/contracts/stage_metadata.py",
        "../millrace-py/src/millrace_ai/errors.py",
        "../millrace-py/src/millrace_ai/runtime/error_recovery.py",
        "../millrace-py/src/millrace_ai/runtime/recon_transitions.py",
        "../millrace-py/src/millrace_ai/runtime/result_application.py",
        "../millrace-py/src/millrace_ai/runtime/stage_requests.py",
        "../millrace-py/src/millrace_ai/runtime/supervisor.py",
        "../millrace-py/src/millrace_ai/runtime/tick_cycle.py",
        "../millrace-py/tests/cli/test_cli.py",
        "../millrace-py/tests/runtime/test_graph_authority.py",
        "../millrace-py/tests/runtime/test_recon_packets.py",
        "../millrace-py/tests/runtime/test_runtime.py",
    ] {
        assert!(
            sources
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "missing v0.18.2 runtime/status/recon/ownership source {source_path}"
        );
    }

    let targets = fixture["expected_rust_contract_targets"]
        .as_array()
        .expect("expected Rust contract targets are present");
    for target_path in [
        "src/contracts/enums.rs",
        "src/contracts/recon.rs",
        "src/contracts/runtime_json.rs",
        "src/contracts/stage_metadata.rs",
        "src/recon_packets.rs",
        "src/cli/parser.rs",
        "src/cli/read_only.rs",
        "src/cli/render.rs",
        "src/runtime/startup.rs",
        "src/runtime/supervisor.rs",
        "src/runtime/tick.rs",
        "src/runtime/run_traces.rs",
        "src/workspace/queue_store.rs",
        "src/workspace/state_store.rs",
        "tests/contracts_stage_metadata.rs",
        "tests/contracts_runtime_json.rs",
        "tests/runtime_serial.rs",
        "tests/runtime_daemon.rs",
        "tests/parity_cli.rs",
        "tests/workspace_queue_state_stores.rs",
    ] {
        assert!(
            targets
                .iter()
                .any(|value| value.as_str() == Some(target_path)),
            "missing v0.18.2 Rust contract target {target_path}"
        );
    }

    let guarantees = fixture["no_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
        "no release upload",
        "no publishing",
    ] {
        assert!(
            guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.18.2 runtime contract scout guarantee {guarantee}"
        );
    }
}

#[test]
fn auto_port_v0_18_3_runtime_contract_scout_pins_librarian_trigger_runner_metadata_sources() {
    let fixture: Value = python_model_dump_fixture(include_str!(
        "fixtures/runtime_json/auto_port_v0_18_3_runtime_contract_scout.json"
    ));
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["kind"], "auto_port_v0_18_3_runtime_contract_scout");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.2");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.3");
    assert_eq!(
        fixture["python_reference"]["previous_commit"],
        "5444cb9485ea90b67b2ed6ba7e0723ae9fe7b79f"
    );
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "6556e55c8463ce9256716bc425a49059b4c5981c"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.2..v0.18.3"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.3.2"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_version_role"],
        "previous_baseline_for_python_v0.18.2"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.3");
    assert_ne!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.18.3 runtime scout must not treat Rust 0.3.2 as the target"
    );

    let sources = fixture["contract_sources"]
        .as_array()
        .expect("contract source references are present");
    for source_path in [
        "../millrace-py/src/millrace_ai/contracts/enums.py",
        "../millrace-py/src/millrace_ai/contracts/stage_metadata.py",
        "../millrace-py/src/millrace_ai/runners/normalization.py",
        "../millrace-py/src/millrace_ai/runtime/learning_triggers.py",
        "../millrace-py/src/millrace_ai/runtime/stage_requests.py",
        "../millrace-py/tests/assets/test_modes.py",
        "../millrace-py/tests/assets/test_stage_kinds.py",
        "../millrace-py/tests/runtime/test_runtime.py",
    ] {
        assert!(
            sources
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "missing v0.18.3 runtime/Librarian metadata source {source_path}"
        );
    }

    let targets = fixture["expected_rust_contract_targets"]
        .as_array()
        .expect("expected Rust contract targets are present");
    for target_path in [
        "src/contracts/enums.rs",
        "src/contracts/runtime_json.rs",
        "src/contracts/stage_metadata.rs",
        "src/runners/normalization.rs",
        "src/runtime/mod.rs",
        "src/runtime/startup.rs",
        "src/runtime/supervisor.rs",
        "src/runtime/tick.rs",
        "tests/contracts_runtime_json.rs",
        "tests/contracts_stage_metadata.rs",
        "tests/runtime_daemon.rs",
        "tests/runtime_serial.rs",
        "tests/runners_normalization.rs",
        "tests/parity_cli.rs",
    ] {
        assert!(
            targets
                .iter()
                .any(|value| value.as_str() == Some(target_path)),
            "missing v0.18.3 Rust contract target {target_path}"
        );
    }

    let guarantees = fixture["no_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
        "no remote skill installation",
        "no release upload",
        "no publishing",
    ] {
        assert!(
            guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.18.3 runtime contract scout guarantee {guarantee}"
        );
    }
}

#[test]
fn recon_packet_json_fixture_and_markdown_round_trip_exactly() {
    let expected = python_model_dump_fixture(include_str!(
        "fixtures/runtime_json/recon_packet_to_execution.json"
    ));

    let packet = ReconPacketDocument::from_json_value(expected.clone()).unwrap();
    assert_eq!(packet.decision, ReconDecision::ToExecution);
    assert_eq!(packet.handoff_target, ReconHandoffTarget::Execution);
    assert_eq!(packet.emitted_task_id.as_deref(), Some("task-from-probe"));
    assert_eq!(serde_json::to_value(&packet).unwrap(), expected);

    let rendered = render_recon_packet(&packet);
    assert!(rendered.starts_with("# Recon Packet recon-probe-001\n"));
    assert!(rendered.contains("Recon-Packet-ID: recon-probe-001\n"));
    assert!(rendered.contains("Relevant-Paths:\n- src/example.rs | Likely behavior owner.\n"));
    assert!(rendered.contains("Required-Commands:\n- cargo test --test contracts_runtime_json\n"));

    let parsed = parse_recon_packet(&rendered).unwrap();
    assert_eq!(parsed, packet);

    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("recon-probe-001.md");
    std::fs::write(&path, rendered).unwrap();
    assert_eq!(read_recon_packet(&path).unwrap(), packet);
}

#[test]
fn recon_packet_contract_rejects_mismatched_or_empty_fields() {
    let fixture = || {
        python_model_dump_fixture(include_str!(
            "fixtures/runtime_json/recon_packet_to_execution.json"
        ))
    };

    let mut bad_handoff = fixture();
    bad_handoff["handoff_target"] = json!("planning");
    let error = ReconPacketDocument::from_json_value(bad_handoff).unwrap_err();
    assert!(matches!(error, ReconPacketError::InvalidDocument { .. }));
    assert!(error.to_string().contains("handoff_target"));

    let mut missing_emitted_task = fixture();
    missing_emitted_task["emitted_task_id"] = Value::Null;
    let error = ReconPacketDocument::from_json_value(missing_emitted_task).unwrap_err();
    assert!(error.to_string().contains("Emitted-Task-ID"));

    let mut to_planning_with_task = fixture();
    to_planning_with_task["decision"] = json!("to_planning");
    to_planning_with_task["handoff_target"] = json!("planning");
    to_planning_with_task["emitted_task_id"] = json!("task-from-probe");
    to_planning_with_task["emitted_spec_id"] = Value::Null;
    let error = ReconPacketDocument::from_json_value(to_planning_with_task).unwrap_err();
    assert!(error.to_string().contains("Emitted-Spec-ID"));
    assert!(error.to_string().contains("Emitted-Task-ID"));

    let mut to_execution_with_spec = fixture();
    to_execution_with_spec["emitted_task_id"] = Value::Null;
    to_execution_with_spec["emitted_spec_id"] = json!("spec-from-probe");
    let error = ReconPacketDocument::from_json_value(to_execution_with_spec).unwrap_err();
    assert!(error.to_string().contains("Emitted-Task-ID"));

    let mut invalid_emitted_task = fixture();
    invalid_emitted_task["emitted_task_id"] = json!("-bad-task");
    let error = ReconPacketDocument::from_json_value(invalid_emitted_task).unwrap_err();
    assert!(matches!(error, ReconPacketError::Contract(_)));

    let mut empty_summary = fixture();
    empty_summary["request_summary"] = json!("");
    let error = ReconPacketDocument::from_json_value(empty_summary).unwrap_err();
    assert!(matches!(
        error,
        ReconPacketError::MissingRequiredField {
            field_name: "request_summary"
        }
    ));

    let mut empty_paths = fixture();
    empty_paths["relevant_paths"] = json!([]);
    let error = ReconPacketDocument::from_json_value(empty_paths).unwrap_err();
    assert!(matches!(
        error,
        ReconPacketError::EmptyRequiredList {
            field_name: "relevant_paths"
        }
    ));

    let mut bad_path_finding = fixture();
    bad_path_finding["relevant_paths"][0]["reason"] = json!("");
    let error = ReconPacketDocument::from_json_value(bad_path_finding).unwrap_err();
    assert!(error.to_string().contains("reason"));

    let mut malformed_markdown = render_recon_packet(
        &ReconPacketDocument::from_json_value(fixture()).expect("fixture packet"),
    );
    malformed_markdown = malformed_markdown.replace(
        "- src/example.rs | Likely behavior owner.",
        "- src/example.rs",
    );
    let error = parse_recon_packet(&malformed_markdown).unwrap_err();
    assert!(error.to_string().contains("path | reason"));
}

#[test]
fn python_v0_18_1_probe_mailbox_and_recon_stage_json_contracts_round_trip() {
    let payload_value = python_model_dump_fixture(include_str!(
        "fixtures/runtime_json/mailbox_add_probe_payload.json"
    ));
    let payload = MailboxAddProbePayload::from_json_value(payload_value).unwrap();
    assert_eq!(payload.document.kind().as_str(), "probe");
    assert_eq!(payload.document.probe_id, "probe-001");
    assert_eq!(
        payload.document.status_hint.map(|status| status.as_str()),
        Some("queued")
    );

    let add_probe = round_trip_contract::<MailboxCommandEnvelope>(json!({
        "schema_version": "1.0",
        "kind": "mailbox_command",
        "command_id": "cmd-add-probe",
        "command": "add_probe",
        "issued_at": NOW,
        "issuer": "operator",
        "payload": {
            "document": {
                "probe_id": "probe-001",
                "title": "Probe mailbox payload",
                "summary": "research before routing",
                "request": "Research the repo surface and route this work safely.",
                "target_paths": ["src/example.rs"],
                "created_at": NOW,
                "created_by": "tests"
            }
        }
    }));
    assert_eq!(add_probe.command.as_str(), "add_probe");

    let mut recon_stage = stage_result_json();
    recon_stage["plane"] = json!("planning");
    recon_stage["stage"] = json!("recon");
    recon_stage["node_id"] = json!("recon");
    recon_stage["stage_kind_id"] = json!("recon");
    recon_stage["work_item_kind"] = json!("probe");
    recon_stage["work_item_id"] = json!("probe-001");
    recon_stage["terminal_result"] = json!("RECON_TO_EXECUTION");
    recon_stage["result_class"] = json!("success");
    recon_stage["summary_status_marker"] = json!("### RECON_TO_EXECUTION");
    recon_stage["detected_marker"] = json!("### RECON_TO_EXECUTION");
    recon_stage["artifact_paths"] = json!(["recon_packet.md", "generated_task.md"]);

    let decoded = round_trip_stage_result(recon_stage);
    assert_eq!(decoded.stage, StageName::Recon);
    assert_eq!(
        decoded.terminal_result,
        TerminalResult::Planning(PlanningTerminalResult::ReconToExecution)
    );
    assert_eq!(decoded.work_item_kind.as_str(), "probe");
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
fn run_trace_graph_contract_round_trips_and_validates_edge_refs() {
    let graph = round_trip_contract::<RunTraceGraph>(run_trace_graph_json());

    assert_eq!(graph.kind, "run_trace_graph");
    assert_eq!(graph.status.as_str(), "active");
    assert_eq!(graph.nodes[0].trace_node_id, "request-001");
    assert_eq!(
        graph.nodes[0].token_usage.as_ref().unwrap().total_tokens,
        135
    );
    assert_eq!(graph.edges[0].edge_kind, "run_stage");
    assert_eq!(
        graph.edges[0].spawned_work[0].kind.as_str(),
        "learning_request"
    );

    let mut bad_edge_ref = run_trace_graph_json();
    bad_edge_ref["edges"][0]["source_trace_node_id"] = json!("missing-node");
    let error = RunTraceGraph::from_json_value(bad_edge_ref).unwrap_err();
    assert!(matches!(error, RuntimeJsonError::InvalidDocument { .. }));

    let mut malformed_status = run_trace_graph_json();
    malformed_status["status"] = json!("malformed");
    let malformed = round_trip_contract::<RunTraceGraph>(malformed_status);
    assert_eq!(malformed.status.as_str(), "malformed");
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
fn python_v0_17_4_stage_result_no_op_runtime_json_round_trips_as_non_success() {
    let mut no_op = stage_result_json();
    no_op["run_id"] = json!("run-learning-noop");
    no_op["plane"] = json!("learning");
    no_op["stage"] = json!("analyst");
    no_op["node_id"] = json!("analyst");
    no_op["stage_kind_id"] = json!("analyst");
    no_op["work_item_kind"] = json!("learning_request");
    no_op["work_item_id"] = json!("learn-001");
    no_op["terminal_result"] = json!("ANALYST_NOOP");
    no_op["result_class"] = json!("no_op");
    no_op["summary_status_marker"] = json!("### ANALYST_NOOP");
    no_op["success"] = json!(false);
    no_op["detected_marker"] = json!("### ANALYST_NOOP");
    no_op["notes"] = json!(["Python v0.17.4 learning no-op contract"]);

    let decoded = round_trip_stage_result(no_op);

    assert_eq!(decoded.stage, StageName::Analyst);
    assert_eq!(
        decoded.terminal_result,
        TerminalResult::Learning(LearningTerminalResult::AnalystNoop)
    );
    assert_eq!(decoded.result_class, ResultClass::NoOp);
    assert!(!decoded.success);
}

#[test]
fn python_v0_17_4_request_driven_no_op_terminal_identity_round_trips() {
    let mut request_driven = stage_result_json();
    request_driven["plane"] = json!("learning");
    request_driven["stage"] = json!("curator");
    request_driven["node_id"] = json!("curator-review");
    request_driven["stage_kind_id"] = json!("curator");
    request_driven["work_item_kind"] = json!("learning_request");
    request_driven["work_item_id"] = json!("learn-review");
    request_driven["terminal_result"] = json!("CURATOR_NOOP");
    request_driven["result_class"] = json!("no_op");
    request_driven["summary_status_marker"] = json!("### CURATOR_NOOP");
    request_driven["success"] = json!(false);
    request_driven["detected_marker"] = json!("### CURATOR_NOOP");

    let decoded = round_trip_stage_result(request_driven);

    assert_eq!(decoded.stage, StageName::Curator);
    assert_eq!(
        decoded.terminal_result,
        TerminalResult::Learning(LearningTerminalResult::CuratorNoop)
    );
    assert_eq!(decoded.result_class, ResultClass::NoOp);
    assert!(!decoded.success);
}

#[test]
fn python_v0_18_3_librarian_stage_result_runtime_json_round_trips() {
    let mut complete = stage_result_json();
    complete["run_id"] = json!("run-librarian");
    complete["plane"] = json!("learning");
    complete["stage"] = json!("librarian");
    complete["node_id"] = json!("librarian");
    complete["stage_kind_id"] = json!("librarian");
    complete["work_item_kind"] = json!("learning_request");
    complete["work_item_id"] = json!("learn-librarian");
    complete["terminal_result"] = json!("LIBRARIAN_COMPLETE");
    complete["result_class"] = json!("success");
    complete["summary_status_marker"] = json!("### LIBRARIAN_COMPLETE");
    complete["success"] = json!(true);
    complete["detected_marker"] = json!("### LIBRARIAN_COMPLETE");

    let decoded_complete = round_trip_stage_result(complete);
    assert_eq!(decoded_complete.stage, StageName::Librarian);
    assert_eq!(
        decoded_complete.terminal_result,
        TerminalResult::Learning(LearningTerminalResult::LibrarianComplete)
    );
    assert_eq!(decoded_complete.result_class, ResultClass::Success);

    let mut no_op = stage_result_json();
    no_op["run_id"] = json!("run-librarian-noop");
    no_op["plane"] = json!("learning");
    no_op["stage"] = json!("librarian");
    no_op["node_id"] = json!("librarian");
    no_op["stage_kind_id"] = json!("librarian");
    no_op["work_item_kind"] = json!("learning_request");
    no_op["work_item_id"] = json!("learn-librarian");
    no_op["terminal_result"] = json!("LIBRARIAN_NOOP");
    no_op["result_class"] = json!("no_op");
    no_op["summary_status_marker"] = json!("### LIBRARIAN_NOOP");
    no_op["success"] = json!(false);
    no_op["detected_marker"] = json!("### LIBRARIAN_NOOP");

    let decoded_no_op = round_trip_stage_result(no_op);
    assert_eq!(decoded_no_op.stage, StageName::Librarian);
    assert_eq!(
        decoded_no_op.terminal_result,
        TerminalResult::Learning(LearningTerminalResult::LibrarianNoop)
    );
    assert_eq!(decoded_no_op.result_class, ResultClass::NoOp);
    assert!(!decoded_no_op.success);
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

    let mut no_op_class_with_success = stage_result_json();
    no_op_class_with_success["result_class"] = json!("no_op");
    let error = StageResultEnvelope::from_json_value(no_op_class_with_success).unwrap_err();
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
