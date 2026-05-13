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

The committed compiler parity fixture is pinned to the Python
`v0.18.0..v0.18.1` source range and covers `default_codex`, `default_pi`,
`learning_codex`, `learning_pi`, and the `standard_plain` compatibility alias.

For Python `v0.18.0` parity, Rust also projects compiled-stage-graph exports
from the frozen `CompiledRunPlan.graphs_by_plane` data instead of reparsing
managed graph assets. `millrace compile graph` can render all selected planes
in stable order, filter to one plane, emit JSON, or write output to a file. The
exported graph names source refs, entries, nodes, edges, terminal states,
legal outcome result-class mappings, skills, runner/model/thinking metadata,
timeouts, and declared output artifacts. It remains inspection evidence only;
`compiled_plan.json` is still the runtime routing authority.

For Python `v0.18.1` compiler parity, the planning graph also includes the
`probe` entry routed to Recon. The exported and materialized graph evidence
preserves the Recon node, `recon-core` skill path, legal Recon terminal
outcomes/classes, timeout metadata, mode runner bindings, and terminal states
without making graph exports runtime routing authority.

For Python `v0.18.2` compiler parity, Rust also packages the opt-in
`execution.with_integrator` graph and loop assets, the Integrator execution
stage metadata, the Integrator entrypoint/core skill, and the
`default_codex_integrated` and `learning_codex_integrated` mode assets. Default
execution modes continue to route Builder success directly to Checker; only the
integrated modes route Builder success through Integrator before Checker.
Compiler graph validation also rejects direct Recon handoff edges to stage
nodes, preserving runtime ownership of generated task/spec promotion.

For Python `v0.18.3` asset/compiler-mode parity, Rust also packages the
Librarian learning entrypoint, `librarian-core` skill, learning stage-kind
registry, learning graph/loop terminal states, and learning mode bindings with
`planning.planner.complete-to-librarian` install trigger rules. Materialized
plans and compiled-stage-graph exports preserve the Librarian node, legal
terminal classes including `LIBRARIAN_NOOP` as `no_op`, required skill path,
runner metadata, timeout metadata, and `skill_install_report` artifact metadata
without making graph exports runtime routing authority. Runtime learning
triggers now preserve the persisted stage-result artifact, stage-produced
artifacts such as `planner_summary.md`, and source work-item kind/id/active-path
metadata when enqueueing learning requests. Targeted Librarian learning-request
claims dispatch to the Librarian node, complete `LIBRARIAN_COMPLETE` and
`LIBRARIAN_NOOP` requests into done with success/no-op semantics, and preserve
recoverable blocked evidence for Librarian `BLOCKED`.
