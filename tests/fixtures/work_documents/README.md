Work-document fixtures in this directory are committed outputs from the adjacent
Python reference work-document renderer under `../millrace-py/src`, including
task, probe, spec, incident, and learning-request documents.
The probe fixture is part of the Python `v0.18.1` release evidence carried into
Rust `0.3.1` and is included by the crate package boundary through
`/tests/fixtures/**/*`.

Regenerate them from the repository root with:

```sh
PYTHONPATH=../millrace-py/src python3 tests/support/generate_python_work_document_fixtures.py
```

The generator writes only under `tests/fixtures/work_documents/`; it must not
initialize or write a `millrace-agents/` workspace in the repository root.
