# Millrace Rust Roadmap

This repository tracks the Rust crate parity roadmap for the Python
`millrace-ai` runtime. The Python package remains the production reference; the
Rust crate owns the `millrace` CLI, typed contracts, and local
`millrace-agents/` workspace artifact boundary.

## Current Release Target

The current Rust release target is `0.3.2`, aligned to Python `v0.18.2` at
commit `5444cb9485ea90b67b2ed6ba7e0723ae9fe7b79f`.

The `0.3.2` release closes the Python `v0.18.1..v0.18.2` parity delta:

- Integrator contracts, entrypoint/core-skill assets, stage-kind metadata, and
  the opt-in `execution.with_integrator` graph/loop assets are packaged.
- `default_codex_integrated` and `learning_codex_integrated` route Builder
  success through Integrator before Checker while default modes stay
  Builder -> Checker.
- `millrace status` and `millrace status show` support text and JSON output
  with blocked-idle, current-failure-class, latest-runtime-error, and
  closure-target diagnostics.
- Invalid Recon handoff artifacts block the active probe with
  `recon_handoff_invalid`, and compiler graph validation rejects direct Recon
  handoff edges to stage nodes.
- Stage/work-item ownership checks reject stale active pairings before serial
  or daemon runner invocation and record `stage_work_item_ownership_invalid`
  runtime error/event evidence.
- optional Python `millrace-web` `v0.18.2` package version, runtime dependency
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
- `tests/fixtures/cli_parity/auto_port_v0_18_2_release_parity_evidence.json`
  records the final Rust `0.3.2` release-parity evidence, including required
  release-readiness command results and the dirty-worktree publish dry-run
  limitation for this Builder pass.

## Explicit Gaps

Rust still does not ship a `millrace-web` package, HTTP dashboard, static shell,
SSE stream, or dashboard API route. The accepted Rust inspection surface
remains local read-only CLI commands over initialized workspaces, including
`millrace compile graph` and `millrace runs trace <run_id>` as graph/trace
shadow surfaces; the Python `v0.18.2` web package version sync is recorded as
unsupported-gap evidence rather than a Rust web implementation. Native
filesystem watcher integration, live subscription-quota
provider polling, and live Codex/Pi smoke runs remain preview-only or opt-in
surfaces.
