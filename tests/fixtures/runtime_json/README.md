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

Regenerate them from the repository root with:

```sh
MILLRACE_PY_ROOT=../millrace-py python3 tests/support/generate_python_runtime_json_fixtures.py
```
