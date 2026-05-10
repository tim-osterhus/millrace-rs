from __future__ import annotations

import os
import sys
import json
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PYTHON_ROOT = Path(os.environ.get("MILLRACE_PY_ROOT", REPO_ROOT / "../millrace-py")).resolve()
sys.path.insert(0, str(PYTHON_ROOT / "src"))

from millrace_ai.contracts.compile_diagnostics import CompileDiagnostics
from millrace_ai.contracts.enums import (
    ExecutionStageName,
    ExecutionTerminalResult,
    LearningStageName,
    LearningTerminalResult,
    MailboxCommand,
    Plane,
    PlanningStageName,
    PlanningTerminalResult,
    ResultClass,
    RuntimeErrorCode,
    RuntimeMode,
    WatcherMode,
    WorkItemKind,
)
from millrace_ai.contracts.mailbox import MailboxCommandEnvelope
from millrace_ai.contracts.recovery import RecoveryCounterEntry, RecoveryCounters
from millrace_ai.contracts.runtime_errors import RuntimeErrorContext
from millrace_ai.contracts.runtime_snapshot import ActiveRunState, RuntimeSnapshot
from millrace_ai.contracts.stage_results import StageResultEnvelope
from millrace_ai.contracts.token_usage import TokenUsage


NOW = datetime(2026, 4, 15, tzinfo=timezone.utc)


def build_fixtures() -> dict[str, object]:
    token_usage = TokenUsage(
        input_tokens=100,
        cached_input_tokens=20,
        output_tokens=30,
        thinking_tokens=5,
        total_tokens=135,
    )

    return {
        "runtime_snapshot.json": RuntimeSnapshot(
            runtime_mode=RuntimeMode.DAEMON,
            process_running=True,
            paused=False,
            active_mode_id="learning_codex",
            execution_loop_id="execution.standard",
            planning_loop_id="planning.standard",
            learning_loop_id="learning.standard",
            loop_ids_by_plane={
                Plane.EXECUTION: "execution.standard",
                Plane.PLANNING: "planning.standard",
                Plane.LEARNING: "learning.standard",
            },
            compiled_plan_id="plan-001",
            compiled_plan_path="millrace-agents/state/compiled_plan.json",
            active_runs_by_plane={
                Plane.EXECUTION: ActiveRunState(
                    plane=Plane.EXECUTION,
                    stage=ExecutionStageName.BUILDER,
                    node_id="builder",
                    stage_kind_id="builder",
                    run_id="run-001",
                    request_kind="active_work_item",
                    work_item_kind=WorkItemKind.TASK,
                    work_item_id="task-001",
                    active_since=NOW,
                    running_status_marker="BUILDER_RUNNING",
                )
            },
            execution_status_marker="### BUILDER_RUNNING",
            planning_status_marker="### IDLE",
            learning_status_marker="### IDLE",
            status_markers_by_plane={
                Plane.EXECUTION: "### BUILDER_RUNNING",
                Plane.PLANNING: "### IDLE",
                Plane.LEARNING: "### IDLE",
            },
            queue_depth_execution=2,
            queue_depth_planning=7,
            queue_depth_learning=0,
            queue_depths_by_plane={
                Plane.EXECUTION: 2,
                Plane.PLANNING: 7,
                Plane.LEARNING: 0,
            },
            last_terminal_result=ExecutionTerminalResult.UPDATE_COMPLETE,
            last_stage_result_path=(
                "millrace-agents/runs/run-000/stage_results/request-000.json"
            ),
            config_version="cfg-001",
            watcher_mode=WatcherMode.WATCH,
            updated_at=NOW,
        ),
        "recovery_counters.json": RecoveryCounters(
            entries=(
                RecoveryCounterEntry(
                    failure_class="missing_terminal_result",
                    work_item_id="task-001",
                    work_item_kind=WorkItemKind.TASK,
                    troubleshoot_attempt_count=1,
                    mechanic_attempt_count=0,
                    fix_cycle_count=0,
                    consultant_invocations=0,
                    last_updated_at=NOW,
                ),
            )
        ),
        "mailbox_command_envelope.json": MailboxCommandEnvelope(
            command_id="cmd-001",
            command=MailboxCommand.RELOAD_CONFIG,
            issued_at=NOW,
            issuer="operator",
            payload={"reason": "test"},
        ),
        "compile_diagnostics.json": CompileDiagnostics(
            ok=False,
            mode_id="standard_plain",
            errors=("missing loop",),
            warnings=("deprecated alias",),
            emitted_at=NOW,
        ),
        "stage_result_envelope.json": StageResultEnvelope(
            run_id="run-001",
            plane=Plane.EXECUTION,
            stage=ExecutionStageName.BUILDER,
            node_id="builder",
            stage_kind_id="builder",
            work_item_kind=WorkItemKind.TASK,
            work_item_id="task-001",
            terminal_result=ExecutionTerminalResult.BUILDER_COMPLETE,
            result_class=ResultClass.SUCCESS,
            summary_status_marker="### BUILDER_COMPLETE",
            success=True,
            retryable=False,
            exit_code=0,
            duration_seconds=1.25,
            prompt_artifact="prompt.md",
            report_artifact="builder_summary.md",
            artifact_paths=("builder_summary.md",),
            detected_marker="### BUILDER_COMPLETE",
            stdout_path="stdout.txt",
            stderr_path="stderr.txt",
            runner_name="codex_cli",
            model_name="gpt-5",
            model_reasoning_effort="medium",
            token_usage=token_usage,
            notes=("builder pass",),
            metadata={"request_id": "request-001"},
            started_at=NOW,
            completed_at=NOW,
        ),
        "stage_result_request_driven_terminal_identity.json": StageResultEnvelope(
            run_id="run-001",
            plane=Plane.EXECUTION,
            stage=ExecutionStageName.BUILDER,
            node_id="builder",
            stage_kind_id="builder",
            work_item_kind=WorkItemKind.TASK,
            work_item_id="task-001",
            terminal_result=ExecutionTerminalResult.CHECKER_PASS,
            result_class=ResultClass.SUCCESS,
            summary_status_marker="### CHECKER_PASS",
            success=True,
            retryable=False,
            exit_code=0,
            duration_seconds=1.25,
            prompt_artifact="prompt.md",
            report_artifact="checker_summary.md",
            artifact_paths=("checker_summary.md",),
            detected_marker="### CHECKER_PASS",
            stdout_path="stdout.txt",
            stderr_path="stderr.txt",
            runner_name="codex_cli",
            model_name="gpt-5",
            model_reasoning_effort="medium",
            token_usage=token_usage,
            notes=("request-driven terminal identity",),
            metadata={"request_id": "request-001"},
            started_at=NOW,
            completed_at=NOW,
        ),
        "stage_result_learning_noop.json": StageResultEnvelope(
            run_id="run-learning-noop",
            plane=Plane.LEARNING,
            stage=LearningStageName.ANALYST,
            node_id="analyst",
            stage_kind_id="analyst",
            work_item_kind=WorkItemKind.LEARNING_REQUEST,
            work_item_id="learn-001",
            terminal_result=LearningTerminalResult.ANALYST_NOOP,
            result_class=ResultClass.NO_OP,
            summary_status_marker="### ANALYST_NOOP",
            success=False,
            retryable=False,
            exit_code=0,
            duration_seconds=1.25,
            prompt_artifact="prompt.md",
            report_artifact="analyst_summary.md",
            artifact_paths=("analyst_summary.md",),
            detected_marker="### ANALYST_NOOP",
            stdout_path="stdout.txt",
            stderr_path="stderr.txt",
            runner_name="codex_cli",
            model_name="gpt-5",
            model_reasoning_effort="medium",
            token_usage=token_usage,
            notes=("learning request required no changes",),
            metadata={
                "request_id": "request-learning-noop",
                "request_kind": "learning_request",
            },
            started_at=NOW,
            completed_at=NOW,
        ),
        "runtime_error_context.json": RuntimeErrorContext(
            error_code=RuntimeErrorCode.PLANNING_POST_STAGE_APPLY_FAILED,
            plane=Plane.PLANNING,
            failed_stage=PlanningStageName.MANAGER,
            repair_stage=PlanningStageName.MECHANIC,
            work_item_kind=WorkItemKind.SPEC,
            work_item_id="spec-001",
            run_id="run-001",
            router_action="route_to_mechanic",
            terminal_result=PlanningTerminalResult.BLOCKED,
            stage_result_path=(
                "millrace-agents/runs/run-001/stage_results/request-001.json"
            ),
            report_path="millrace-agents/runs/run-001/troubleshoot_report.md",
            exception_type="RuntimeError",
            exception_message="post-stage apply failed",
            captured_at=NOW,
        ),
        "token_usage.json": token_usage,
        "auto_port_v0_18_0_runtime_contract_scout.json": {
            "schema_version": "1.0",
            "kind": "auto_port_v0_18_0_runtime_contract_scout",
            "python_reference": {
                "previous_tag": "v0.17.4",
                "previous_commit": "304e537964ff772c815689b87e4c1e3b805c656c",
                "target_tag": "v0.18.0",
                "target_commit": "e4ccf099c8345a8b8708cdaa1ac510bdc7851387",
                "diff_range": "v0.17.4..v0.18.0",
            },
            "contract_sources": [
                "../millrace-py/src/millrace_ai/contracts/graph_exports.py",
                "../millrace-py/src/millrace_ai/contracts/run_trace.py",
                "../millrace-py/src/millrace_ai/runtime/run_traces.py",
                "../millrace-py/tests/integration/test_graph_exports.py",
                "../millrace-py/tests/runtime/test_run_traces.py",
            ],
            "expected_rust_contract_targets": [
                "src/contracts/graph_exports.rs",
                "src/contracts/run_trace.rs",
                "src/runtime/run_traces.rs",
                "tests/contracts_runtime_json.rs",
                "tests/runtime_serial.rs",
                "tests/runtime_daemon.rs",
            ],
            "no_live_guarantees": [
                "no live Codex runner",
                "no live Pi runner",
                "no network",
                "no credentials",
                "no web server",
            ],
        },
        "auto_port_v0_18_1_runtime_contract_scout.json": {
            "schema_version": "1.0",
            "kind": "auto_port_v0_18_1_runtime_contract_scout",
            "python_reference": {
                "previous_tag": "v0.18.0",
                "previous_commit": "e4ccf099c8345a8b8708cdaa1ac510bdc7851387",
                "target_tag": "v0.18.1",
                "target_commit": "0396c7852793b212d31345862b38a7d6f3f02854",
                "diff_range": "v0.18.0..v0.18.1",
            },
            "contract_sources": [
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
                "../millrace-py/src/millrace_ai/runtime/work_item_transitions.py",
                "../millrace-py/src/millrace_ai/workspace/paths.py",
                "../millrace-py/src/millrace_ai/workspace/queue_selection.py",
                "../millrace-py/src/millrace_ai/workspace/queue_store.py",
                "../millrace-py/src/millrace_ai/workspace/queue_transitions.py",
                "../millrace-py/src/millrace_ai/workspace/work_documents.py",
                "../millrace-py/tests/runtime/test_graph_authority.py",
                "../millrace-py/tests/runtime/test_result_application.py",
                "../millrace-py/tests/workspace/test_paths.py",
                "../millrace-py/tests/workspace/test_queue_store.py",
            ],
            "expected_rust_contract_targets": [
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
            ],
            "no_live_guarantees": [
                "no live Codex runner",
                "no live Pi runner",
                "no network",
                "no credentials",
                "no web server",
                "no release upload",
                "no publishing",
            ],
        },
        "auto_port_v0_18_2_runtime_contract_scout.json": {
            "schema_version": "1.0",
            "kind": "auto_port_v0_18_2_runtime_contract_scout",
            "python_reference": {
                "previous_tag": "v0.18.1",
                "previous_commit": "0396c7852793b212d31345862b38a7d6f3f02854",
                "target_tag": "v0.18.2",
                "target_commit": "5444cb9485ea90b67b2ed6ba7e0723ae9fe7b79f",
                "diff_range": "v0.18.1..v0.18.2",
            },
            "rust_reference": {
                "current_repo_crate_version": "0.3.1",
                "current_repo_version_role": "previous_baseline_for_python_v0.18.1",
                "previous_repo_crate_version": "0.3.1",
                "previous_repo_version_role": "released_target_for_python_v0.18.1",
                "planned_crate_version": "0.3.2",
                "planned_version_role": "target_release_for_python_v0.18.2",
            },
            "contract_sources": [
                "../millrace-py/src/millrace_ai/cli/commands/status.py",
                "../millrace-py/src/millrace_ai/cli/status_view.py",
                "../millrace-py/src/millrace_ai/contracts/enums.py",
                "../millrace-py/src/millrace_ai/contracts/recon.py",
                "../millrace-py/src/millrace_ai/contracts/stage_metadata.py",
                "../millrace-py/src/millrace_ai/errors.py",
                "../millrace-py/src/millrace_ai/runtime/engine.py",
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
            ],
            "expected_rust_contract_targets": [
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
            ],
            "no_live_guarantees": [
                "no live Codex runner",
                "no live Pi runner",
                "no network",
                "no credentials",
                "no web server",
                "no release upload",
                "no publishing",
            ],
        },
    }


def main() -> None:
    fixture_dir = Path(__file__).resolve().parents[1] / "fixtures" / "runtime_json"
    fixture_dir.mkdir(parents=True, exist_ok=True)
    for name, model in build_fixtures().items():
        if hasattr(model, "model_dump_json"):
            rendered = model.model_dump_json(indent=2)
        else:
            rendered = json.dumps(model, indent=2, sort_keys=True)
        (fixture_dir / name).write_text(rendered + "\n")


if __name__ == "__main__":
    main()
