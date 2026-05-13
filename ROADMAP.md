# Millrace Rust Roadmap

This repository tracks the Rust crate parity roadmap for the Python
`millrace-ai` runtime. The Python package remains the production reference; the
Rust crate owns the `millrace` CLI, typed contracts, and local
`millrace-agents/` workspace artifact boundary.

## Current Release Target

The current Rust release target is `0.3.3`, aligned to Python `v0.18.3` at
commit `6556e55c8463ce9256716bc425a49059b4c5981c`.

The `0.3.3` release closes the Python `v0.18.2..v0.18.3` parity delta:

- Librarian contracts, entrypoint/core-skill assets, stage-kind metadata, and
  learning graph/loop assets are packaged.
- Learning-enabled modes include Planner-to-Librarian install trigger rules
  while default non-learning modes do not dispatch Librarian.
- Runtime learning requests preserve stage-result, Planner artifact, and source
  work-item metadata; targeted Librarian requests dispatch, complete, no-op, or
  block through the learning request lifecycle.
- Runner normalization preserves active work item kind, id, and active path
  metadata across raw, fake, Codex CLI, and Pi RPC result paths.
- Shipped `SKILL.md` assets pass recursive packaged skill lint coverage,
  including the migrated `marathon-qa-audit` contract shape and guidance
  handoff updates.
- optional Python `millrace-web` `v0.18.3` package version, runtime dependency
  floor, and FastAPI app version are recorded as unsupported-gap package
  evidence for the existing Rust boundary.

## Active Parity Boundary

Rust parity is judged at the operator-visible boundary:

- CLI command names, output shape, and exit behavior
- `millrace-agents/` workspace layout
- headed Markdown task/spec/incident/learning request documents
- runtime JSON artifacts and run inspection output
- compiled-plan semantics, graph routing, and currentness diagnostics
- daemon locking, mailbox intake, watcher intake, pause/resume/stop/reload
- serial tick, daemon scheduling, runner request/result, learning, closure,
  recovery, and usage-governance behavior

Rust source layout may differ from Python as long as these contracts stay
compatible.

## Release Evidence

- `CHANGELOG.md` records release-facing changes.
- `docs/rust-port-roadmap.md` records the port campaign and parity target.
- `docs/source-package-map.md` records package include rules and ownership.
- `docs/runtime/` records Rust runtime contract notes for operator and
  maintainer surfaces.
- `tests/fixtures/cli_parity/auto_port_v0_18_3_release_parity_evidence.json`
  records the final Rust `0.3.3` release-parity evidence, including required
  Builder verification command results and dirty-worktree package verification
  for this pass.

## Explicit Gaps

Rust still does not ship a `millrace-web` package, HTTP dashboard, static shell,
SSE stream, or dashboard API route. The accepted Rust inspection surface
remains local read-only CLI commands over initialized workspaces, including
`millrace compile graph` and `millrace runs trace <run_id>` as graph/trace
shadow surfaces; the Python `v0.18.3` web package version sync is recorded as
unsupported-gap evidence rather than a Rust web implementation. Native
filesystem watcher integration, live subscription-quota
provider polling, and live Codex/Pi smoke runs remain preview-only or opt-in
surfaces.
