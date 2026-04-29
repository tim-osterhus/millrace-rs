Work-document fixtures in this directory are committed outputs from the adjacent
Python reference work-document renderer under `../millrace-py/src`.

Regenerate them from the repository root with:

```sh
PYTHONPATH=../millrace-py/src python3 tests/support/generate_python_work_document_fixtures.py
```

The generator writes only under `tests/fixtures/work_documents/`; it must not
initialize or write a `millrace-agents/` workspace in the repository root.
