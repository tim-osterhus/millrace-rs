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

## v0.18.3 Learning Rules

Learning graph changes must keep Analyst, Professor, Curator, and Librarian
stage metadata aligned across graph loops, stage-kind registry entries,
entrypoints, and skills. `LIBRARIAN_NOOP` maps to `result_class: no_op`;
`LIBRARIAN_COMPLETE` is success; `BLOCKED` is recoverable failure.

Learning-enabled modes may include Planner-to-Librarian trigger rules for
optional-skill preparation. Default non-learning modes must not dispatch
Librarian or enqueue Planner-to-Librarian learning requests.

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
