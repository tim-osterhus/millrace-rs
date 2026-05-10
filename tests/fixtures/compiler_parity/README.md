# Compiler Parity Fixtures

`python_compiler_parity.json` is generated from the Python `millrace-ai`
reference checkout at `../millrace-py` and currently pins the implemented
`v0.18.0..v0.18.1` compiler parity range consumed by
`tests/compiler_parity.rs`.

`auto_port_v0_18_1_compiler_contract_scout.json` is target-facing scout
evidence for Python `v0.18.0..v0.18.1`. It pins the Recon managed assets,
planning graph entry, mode runner bindings, stage-kind registry, compiler
materialization source references, and expected Rust fixture/test targets
alongside the implemented normalized compiler parity fixture.
`auto_port_v0_18_2_compiler_contract_scout.json` is target-facing scout
evidence for Python `v0.18.1..v0.18.2`. It pins the Integrator entrypoint,
Checker entrypoint update, `execution.with_integrator` graph and loop assets,
integrated Codex mode assets, Integrator stage-kind registry, Integrator core
skill, package baseline targets, compiler targets, and fixture/test targets.
The Integrator contracts/assets/compiler graph subset is now implemented and
covered by focused Rust contract, asset, materialization/export, and
workspace-baseline tests. Integrated Codex mode assets and runtime-routing
coverage are now implemented by focused compiler, CLI, serial runtime, daemon
runtime, and baseline tests. Compiler graph validation now also rejects direct
edges from Recon handoff outcomes to stage nodes so generated task/spec
promotion stays runtime-owned.
The Rust `0.3.2` release evidence in
`tests/fixtures/cli_parity/auto_port_v0_18_2_release_parity_evidence.json`
uses this compiler evidence as package-readiness proof that Integrator
entrypoints, stage-kind registry files, skills, integrated mode bindings, graph
fixtures, and the v0.18.2 compiler scout fixture are included by the crate
package boundary.

Regenerate intentionally with:

```bash
python tests/support/generate_python_compiler_parity_fixtures.py
```

The normalized compiler fixture normalizes timestamps, generated compiled-plan ids, baseline
manifest identity, compile-input fingerprints whose inputs differ between the
Python and Rust harnesses, resolved asset content hashes, and platform path
separators. It preserves the serialized compiled-plan schema, mode alias
semantics, graph and node authority, stage bindings including `thinking_level`,
resolved asset identity/path coverage, and the Python v0.18.1 graph-export
source references that define the implemented compiled-stage-graph export
parity surface.
