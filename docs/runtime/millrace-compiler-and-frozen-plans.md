# Compiler And Frozen Plans

Rust compile authority resolves mode, graph, registry, entrypoint, skill, and
runtime configuration inputs into a persisted `compiled_plan.json`. Runtime
startup, reload, stage activation, routing, and run inspection consume that
compiled authority instead of recomputing loop behavior ad hoc.

For Python `v0.17.4` parity, frozen plans include learning trigger destination
metadata:

- `target_skill_id`
- normalized, deduplicated `preferred_output_paths`

Compile validation rejects a learning trigger that targets Curator directly
unless one of those destination fields is present. Generic or vague success
evidence should target Analyst so the learning plane can research, no-op, or
escalate without asking Curator to infer a mutation destination.

The committed compiler parity fixture is pinned to Python `v0.17.4` and covers
`default_codex`, `default_pi`, `learning_codex`, `learning_pi`, and the
`standard_plain` compatibility alias.
