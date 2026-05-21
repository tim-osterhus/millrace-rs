# Compiler Parity Fixtures

`python_compiler_parity.json` is generated from the Python `millrace-ai`
reference checkout at `../millrace-py` and currently pins the implemented
Python `v0.19.0..v0.20.0` compiler authority range consumed by
`tests/compiler_parity.rs`, including workflow primitive registry authority,
Blueprint compile assets, primitive fingerprints, lane/request-context/
terminal-action/runtime-effect/schema/completion evidence, and pending-plan
metadata.

`auto_port_v0_18_1_compiler_contract_scout.json` is target-facing scout
evidence for Python `v0.18.0..v0.18.1`. It pins the Recon managed assets,
planning graph entry, mode runner bindings, stage-kind registry, compiler
materialization source references, and expected Rust fixture/test targets
alongside the historical compiled-stage-graph parity surface.
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
`auto_port_v0_18_3_compiler_contract_scout.json` is target-facing scout
evidence for Python `v0.18.2..v0.18.3`. It pins the Librarian entrypoint,
learning graph and loop additions, learning mode trigger-bearing assets,
Librarian stage-kind registry, librarian-core skill, shipped skill lint
surfaces, compiler materialization source references, and expected Rust
fixture/test targets. The Librarian asset/compiler graph/mode slice now lands
the managed assets, compiler materialization/export coverage, normalized
compiler parity fixture updates, and workspace-baseline synchronization; the
runner normalization/artifact-metadata slice now lands source metadata
preservation; and the active Librarian lifecycle slice now lands focused serial
and daemon runtime coverage. The shipped skill lint/guidance slice now lands
recursive packaged `SKILL.md` lint coverage, `marathon-qa-audit`
section-contract migration, Curator/Recon/Planner guidance updates, and
live/baseline asset sync coverage.
The Rust `0.3.3` release evidence in
`tests/fixtures/cli_parity/auto_port_v0_18_3_release_parity_evidence.json`
uses this compiler evidence as package-readiness proof that Librarian
entrypoints, stage-kind registry files, skills, learning graph/loop assets,
learning mode trigger bindings, shipped skill lint assets, and the v0.18.3
compiler scout fixture are included by the crate package boundary.
`auto_port_v0_20_0_compiler_contract_scout.json` is target-facing scout
evidence for Python `v0.19.0..v0.20.0`. It pins workflow primitive registry
collections, Blueprint graph/mode assets, compiler validation source
references, persisted primitive authority fields, compile inspection fields,
and expected Rust compiler/test targets for the Rust `0.5.0` workflow
authority auto-port lineage.

Regenerate intentionally with:

```bash
python tests/support/generate_python_compiler_parity_fixtures.py
```

The normalized compiler fixture normalizes timestamps, generated compiled-plan ids, baseline
manifest identity, compile-input fingerprints whose inputs differ between the
Python and Rust harnesses, resolved asset content hashes, and platform path
separators. It preserves the serialized compiled-plan schema, mode alias
semantics, graph and node authority, stage bindings including `thinking_level`,
resolved asset identity/path coverage, workflow primitive fingerprints,
lane/request-context/terminal-action/runtime-effect/schema/completion and
pending-plan fields, and the Python v0.20.0 source references that define the
implemented compiler authority surface.
