Runtime JSON fixtures in this directory are committed outputs from the adjacent
Python reference contracts under `../millrace-py/src/millrace_ai/contracts`.
`stage_result_learning_noop.json` pins the Python v0.17.4 learning no-op stage
result shape: `result_class: no_op`, `success: false`, and a learning request
work item.
`auto_port_v0_18_0_runtime_contract_scout.json` pins the Python
`v0.17.4..v0.18.0` graph-export and run-trace contract source references plus
the expected Rust contract/test targets. It remains scout evidence for the
Python source range; the compiled graph export slice adds the typed Rust graph
contracts, and the trace-runtime slice adds the typed Rust `run_trace_graph`
contracts plus runtime trace persistence and fallback-inspection coverage.
`auto_port_v0_18_1_runtime_contract_scout.json` pins the Python
`v0.18.0..v0.18.1` probe/recon runtime, mailbox, queue, work-document, and
result-application source references plus the expected Rust contract/test
targets. It is target-facing guardrail evidence for the probe/recon auto-port
lineage and does not require live Python execution.
`auto_port_v0_18_2_runtime_contract_scout.json` pins the Python
`v0.18.1..v0.18.2` status JSON, Recon invalid-handoff, graph validation,
stage/work-item ownership, runtime error, and runtime test source references
plus the expected Rust contract/test targets. It is target-facing guardrail
evidence for the Integrator/status/recon/ownership auto-port lineage and does
not require live Python execution. The status JSON portion is now backed by the
Rust `ReadOnlyStatusPayload` contract coverage for blocked-idle,
current-failure-class, latest runtime error report path, and closure-target
diagnostics. The Recon hardening portion is now backed by contract/runtime
coverage for handoff-specific emitted-id validation, `recon_handoff_invalid`
runtime error evidence, blocked active-probe disposition, and generated
task/spec id checks before queue import.
The stage/work-item ownership portion is now backed by Rust contract and
runtime coverage for the typed ownership matrix, `StageRunRequest` validation,
serial and daemon pre-runner guards, `stage_work_item_ownership_invalid`
runtime error evidence, `runtime_stage_work_item_ownership_invalid` event
evidence, active-artifact requeue behavior, active snapshot clearing, and
closure-target Arbiter exemption.
`auto_port_v0_18_3_runtime_contract_scout.json` pins the Python
`v0.18.2..v0.18.3` Librarian stage metadata, Planner-to-Librarian learning
trigger, learning request artifact metadata, runner normalization metadata, and
runtime test source references plus the expected Rust contract/test targets. It
is target-facing guardrail evidence for the Librarian/learning-trigger/runner
metadata auto-port lineage and does not require live Python execution, network,
credentials, remote skill installation, a web server, or release upload.
The v0.18.3 source-metadata slice is now backed by `stage_result_*` fixtures
that preserve active work item kind, id, and active path metadata while keeping
older optional metadata omissions backward-compatible, and the runtime lifecycle
slice is backed by serial and daemon Planner-to-Librarian tests.
`auto_port_v0_18_4_runtime_contract_scout.json` pins the Python
`v0.18.3..v0.18.4` runner failure metadata, blocked item metadata, audited
blocked-task retry, `auto_recovery` config/status, and daemon blocked-dependency
auto-recovery source references plus the expected Rust contract/test targets.
It is target-facing guardrail evidence for the blocked recovery auto-port
lineage and does not require live Python execution, network, credentials,
Codex, Pi, a web server, or release upload.
The runner failure metadata and blocked item metadata portions are now backed by
typed Rust runtime JSON contracts, runner normalization tests for retryable
timeout/provider/rate limit/network classes plus non-auto-retryable local, auth,
terminal-contract, and unknown transport failures, serial runtime persistence
tests for `millrace-agents/diagnostics/blocked/task-<TASK_ID>.json`, and
queue-store requeue primitive coverage. The manual retry CLI is now backed by
the shared `retry_blocked_task` result contract and CLI parity tests. The
auto-recovery config/status target is now backed by typed Rust startup config
coverage for defaults, explicit values, invalid values, daemon-session policy
projection, next-tick boundary classification, and `config show` parity output;
daemon recovery is backed by typed diagnostic contracts and runtime daemon tests
for successful auto-requeue, review-required skips, runtime/monitor events, and
same-cycle dependent dispatch suppression.
`mailbox_add_probe_payload.json` and `recon_packet_to_execution.json` pin the
implemented v0.18.1 add-probe mailbox payload and Recon packet contract
fixtures used by the Rust runtime JSON contract tests.
The Rust `0.3.2` release evidence in
`tests/fixtures/cli_parity/auto_port_v0_18_2_release_parity_evidence.json`
uses these fixtures as package-readiness proof that status JSON diagnostics,
Recon invalid-handoff evidence, stage/work-item ownership evidence, add-probe
mailbox contracts, and Recon packet JSON are shipped with the crate test
evidence.
The Rust `0.3.3` release evidence in
`tests/fixtures/cli_parity/auto_port_v0_18_3_release_parity_evidence.json`
uses the v0.18.3 scout and stage-result fixtures as package-readiness proof
that Librarian trigger metadata, active work item source metadata, and
Librarian complete/no-op runtime JSON evidence are shipped with the crate test
evidence.
The Rust `0.3.4` release evidence in
`tests/fixtures/cli_parity/auto_port_v0_18_4_release_parity_evidence.json`
uses the v0.18.4 runtime scout as package-readiness proof that failure
classification, blocked metadata persistence, manual queue retry behavior,
auto-recovery config/status, and daemon blocked-dependency auto-recovery
evidence are shipped with the crate test evidence.
The Rust `0.3.4` guardrail scout in
`tests/fixtures/cli_parity/auto_port_v0_18_4_parity_evidence.json` uses the
v0.18.4 runtime scout as package-readiness proof that failure classification and
blocked metadata persistence are implemented, that manual queue retry behavior
is covered, that auto-recovery config/status evidence is covered, and that
daemon blocked-dependency auto-recovery evidence is covered.

Regenerate them from the repository root with:

```sh
MILLRACE_PY_ROOT=../millrace-py python3 tests/support/generate_python_runtime_json_fixtures.py
```
