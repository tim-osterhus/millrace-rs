Runtime JSON fixtures in this directory are committed outputs from the adjacent
Python reference contracts under `../millrace-py/src/millrace_ai/contracts`.

Regenerate them from the repository root with:

```sh
PYTHONPATH=../millrace-py/src python3 tests/support/generate_python_runtime_json_fixtures.py
```
