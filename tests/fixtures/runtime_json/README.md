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
`mailbox_add_probe_payload.json` and `recon_packet_to_execution.json` pin the
implemented v0.18.1 add-probe mailbox payload and Recon packet contract
fixtures used by the Rust runtime JSON contract tests.
The final Rust `0.3.1` release evidence in
`tests/fixtures/cli_parity/auto_port_v0_18_1_release_parity_evidence.json`
uses these fixtures as package-readiness proof that add-probe mailbox contracts
and Recon packet JSON are shipped with the crate test evidence.

Regenerate them from the repository root with:

```sh
MILLRACE_PY_ROOT=../millrace-py python3 tests/support/generate_python_runtime_json_fixtures.py
```
