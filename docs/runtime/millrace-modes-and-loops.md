# Modes And Loops

Rust ships the default Codex/Pi mode families plus learning-enabled modes that
add the Analyst, Professor, Curator, and Librarian learning plane. Default modes
remain serial. Learning modes may run one Learning stage beside one allowed
foreground Planning or Execution stage, while runtime-owned mutation remains
serialized.

The learning graph supports these no-op terminal outcomes:

- `ANALYST_NOOP`
- `PROFESSOR_NOOP`
- `CURATOR_NOOP`
- `LIBRARIAN_NOOP`

Each no-op terminal maps to result class `no_op`. It is not success and it is
not blocked. It means the evidence was reviewed and no skill candidate, patch,
or workspace-installed mutation was warranted.

Built-in generic Doublechecker success learning routes to Analyst first. Direct
Curator trigger rules are valid only when the compiled mode includes a safe
destination through `target_skill_id` or `preferred_output_paths`.

Rust can also export the selected mode's compiled graph topology for
execution, planning, and learning planes through `millrace compile graph`.
Those exports intentionally preserve recovery cycles and terminal-state labels;
they are legal-topology inspection data, not DAG assertions and not a second
routing authority.

Python `v0.18.1` parity adds the Planning-plane `probe` entry, routed to
`recon`, across default Codex/Pi and learning Codex/Pi modes. Recon terminal
results are graph-authoritative: `RECON_TO_EXECUTION` and `RECON_TO_PLANNING`
produce one generated handoff artifact, `RECON_NOOP` closes the probe without
new work, and `RECON_BLOCKED` or generic `BLOCKED` records blocked probe
evidence.

Python `v0.18.2` parity adds opt-in integrated Codex modes. `default_codex` and
`learning_codex` keep the standard execution route of Builder -> Checker.
`default_codex_integrated` and `learning_codex_integrated` use
`execution.with_integrator`, routing Builder success to Integrator and
Integrator success to Checker. Integrator blocked results route through the
compiled recovery policy like other execution stages, and run traces preserve
the Builder -> Integrator -> Checker evidence sequence for integrated runs.

Python `v0.18.3` asset/compiler-mode parity adds the Librarian learning node,
Librarian complete/no-op/blocked terminal states, `skill_install_report`
artifact metadata, and Planner-to-Librarian install trigger rules in
learning-enabled modes. Default non-learning modes still do not include
Librarian dispatch or Planner-to-Librarian learning triggers. Runtime learning
trigger enqueueing now preserves stage-result artifacts, stage-produced
artifacts such as `planner_summary.md`, and source work-item metadata for
learning requests. Targeted Librarian learning requests now dispatch to the
Librarian node; complete/no-op outcomes move requests to done with the correct
success/no-op result class, and blocked outcomes preserve recoverable-failure
evidence.
