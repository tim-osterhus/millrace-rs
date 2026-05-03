# Compiler Parity Fixtures

`python_compiler_parity.json` is generated from the Python `millrace-ai`
reference checkout at `../millrace-py` and consumed by
`tests/compiler_parity.rs`.

Regenerate intentionally with:

```bash
python tests/support/generate_python_compiler_parity_fixtures.py
```

The fixture normalizes timestamps, generated compiled-plan ids, baseline
manifest identity, compile-input fingerprints whose inputs differ between the
Python and Rust harnesses, resolved asset content hashes, and platform path
separators. It preserves the serialized compiled-plan schema, mode alias
semantics, graph and node authority, stage bindings including `thinking_level`,
and resolved asset identity/path coverage.
