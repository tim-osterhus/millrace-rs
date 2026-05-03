# Modes And Loops

Rust ships the default Codex/Pi mode families plus learning-enabled modes that
add the Analyst, Professor, and Curator learning plane. Default modes remain
serial. Learning modes may run one Learning stage beside one allowed foreground
Planning or Execution stage, while runtime-owned mutation remains serialized.

The learning graph supports these no-op terminal outcomes:

- `ANALYST_NOOP`
- `PROFESSOR_NOOP`
- `CURATOR_NOOP`

Each no-op terminal maps to result class `no_op`. It is not success and it is
not blocked. It means the evidence was reviewed and no skill candidate, patch,
or workspace-installed mutation was warranted.

Built-in generic Doublechecker success learning routes to Analyst first. Direct
Curator trigger rules are valid only when the compiled mode includes a safe
destination through `target_skill_id` or `preferred_output_paths`.
