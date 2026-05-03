Runtime JSON fixtures in this directory are committed outputs from the adjacent
Python reference contracts under `../millrace-py/src/millrace_ai/contracts`.
`stage_result_learning_noop.json` pins the Python v0.17.4 learning no-op stage
result shape: `result_class: no_op`, `success: false`, and a learning request
work item.

Regenerate them from the repository root with:

```sh
MILLRACE_PY_ROOT=../millrace-py python3 tests/support/generate_python_runtime_json_fixtures.py
```
