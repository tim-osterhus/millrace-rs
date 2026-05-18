# Rust Runtime Docs

These notes document the Rust-owned runtime contract surfaces for the
experimental `millrace-ai` crate. They are concise Rust equivalents of the
Python runtime docs changed through `v0.19.0`; they do not claim production
status or introduce a Rust web dashboard.

Start with:

- `millrace-cli-reference.md`: version output, probe intake, status JSON,
  intervention commands, learning/Librarian surfaces, and graph/trace CLI
  inspection commands
- `millrace-execution-capabilities.md`: capability contracts, compiled grants,
  pre-dispatch gates, durable approvals, runner evidence, and read-only
  inspection lines
- `millrace-runner-architecture.md`: runner request/result/envelope flow,
  adapter support decisions, capability evidence propagation, and release
  boundary notes
- `millrace-arbiter-and-completion-behavior.md`: closure target activation,
  durable root idea-source preference, and missing-source recovery behavior
- `millrace-compiled-stage-graphs-and-run-traces.md`: compiled topology
  exports, per-run trace artifacts, fallback inspection, and web-gap boundary
- `millrace-compiler-and-frozen-plans.md`: compile authority, learning trigger
  destination safety, graph export projection, Integrator graph assets, and
  frozen plan evidence
- `millrace-modes-and-loops.md`: shipped modes, integrated Codex modes,
  learning no-op terminal outcomes, Librarian trigger routing, and
  `probe -> recon` planning topology
- `millrace-runtime-architecture.md`: runtime request/result application,
  Integrator routing, probe/Recon transitions and hardening, ownership guards,
  run-trace persistence, operator interventions, durable idea-source recovery,
  learning no-op/Librarian lifecycle, and read-only inspection boundaries
- `millrace-entrypoint-mapping.md`: Rust packaged-source to deployed-workspace
  entrypoint mapping, including Librarian
- `millrace-loop-authoring.md`: maintainer guardrails for mode, graph, loop,
  stage-kind, and skill asset edits
- `millrace-runtime-lifecycle-diagram.md`: compact lifecycle chart including
  Planner-to-Librarian request flow
- `millrace-workspace-baselines-and-upgrades.md`: managed baseline and upgrade
  behavior with v0.18.3, v0.18.4, v0.18.6, and v0.19.0 package evidence notes

The stable contract remains the `millrace` CLI plus the local
`millrace-agents/` workspace artifact format. Python `v0.19.0` remains the
reference for the release delta represented by Rust `0.4.0`.
