# Millrace Loop Authoring

This note is for maintainers changing Rust mode, graph, loop, stage-kind, or
entrypoint assets.

## Runtime Authority

Rust executes from compiled graph authority, not prompt prose. Update typed
assets first, then docs:

- `src/assets/baseline/modes/`
- `src/assets/baseline/graphs/`
- `src/assets/baseline/loops/`
- `src/assets/baseline/registry/stage_kinds/`
- `src/assets/baseline/entrypoints/`
- `src/assets/baseline/skills/`

Legacy `loops/` assets remain packaged compatibility inputs, while `graphs/`
and stage-kind assets are the authoritative topology inputs for compiled plans.

Python `v0.20.0` adds compiler-owned workflow primitive registries under
`src/assets/baseline/registry/`. When changing loop behavior, keep graph,
stage-kind, registry, entrypoint, skill, and Rust runtime handler support in
sync. Packaged runtime-effect handlers must have Rust implementations before a
mode can run; unknown handler ids or duplicate rule bindings are compile
errors.

## v0.18.3 Learning Rules

Learning graph changes must keep Analyst, Professor, Curator, and Librarian
stage metadata aligned across graph loops, stage-kind registry entries,
entrypoints, and skills. `LIBRARIAN_NOOP` maps to `result_class: no_op`;
`LIBRARIAN_COMPLETE` is success; `BLOCKED` is recoverable failure.

Learning-enabled modes may include Planner-to-Librarian trigger rules for
optional-skill preparation. Default non-learning modes must not dispatch
Librarian or enqueue Planner-to-Librarian learning requests.

## v0.20.0 Blueprint Rules

Blueprint modes use the `planning.blueprint` graph and the
`blueprint_draft` work-item family. Manager Blueprint creates drafts from a
manifest, Contractor Blueprint produces candidate packets, Evaluator Blueprint
approves packets into generated execution tasks or rejects them back to
Contractor with critique evidence, and Mechanic Blueprint handles policy-routed
pre-mutation repair. Source lifecycle, generated task promotion, duplicate
blocking, partial-mutation blocking, and closure suppression remain
runtime-effect responsibilities, not prompt instructions.

## Verification

When authoring these assets, run focused compiler and runtime checks before
broader release checks:

```bash
cargo test --test compiler_parity
cargo test --test parity_cli
cargo test --test compiler_materialization
```

Docs and evidence fixtures must stay factual about the package boundary:
managed baseline assets are packaged under `src/assets/baseline/`, while live
workspace artifacts under `millrace-agents/` are not crate package inputs.
