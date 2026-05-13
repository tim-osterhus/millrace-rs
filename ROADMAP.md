# Millrace Rust Roadmap

This repository tracks the Rust crate parity roadmap for the Python
`millrace-ai` runtime. The Python package remains the production reference; the
Rust crate owns the `millrace` CLI, typed contracts, and local
`millrace-agents/` workspace artifact boundary.

## Current Release Target

The current Rust release target is `0.3.5`, aligned to Python `v0.18.6` at
commit `63e623bc6fcfcf74ae0cc2ce5605a12ae4179873`. The Python `v0.18.5`
intermediate release at `51374def7e9ea8225f52d95d25abc2fd43f85c9a` is included
in the same Rust patch release.

The `0.3.5` release closes the Python `v0.18.4..v0.18.6` parity delta:

- `millrace queue cancel`, `queue archive-blocked`, `queue supersede`,
  `queue retarget-dependency`, `incident resolve`, `incident cancel`, and
  `incident archive-invalid` archive runtime artifacts instead of deleting
  them, write `operator_intervention` audit records, emit runtime events, and
  refresh queue-depth snapshots.
- Mutating intervention commands route through `RuntimeControl`: they apply
  directly when no daemon owns the workspace and are mailbox-routed when an
  active daemon owns it. Active-stage direct mutation is refused or deferred at
  safe runtime boundaries.
- `queue ls`, `queue show`, `status`, and the basic monitor expose cancelled,
  superseded, operator-resolved, mailbox-applied, and deferred intervention
  evidence without repairing archived artifacts.
- Watcher idea intake preserves original markdown under
  `millrace-agents/intake/ideas/<root_idea_id>.md`, and generated root specs
  reference that durable runtime-owned copy before transient inbox paths.
- Closure-target creation and backfill prefer durable idea sources; missing
  root idea sources during backlog-drain recovery block Planning with
  `missing_root_idea_source` and `root_idea_source_missing` evidence while the
  daemon loop continues.
- optional Python `millrace-web` `v0.18.5` and `v0.18.6` package versions,
  runtime dependency floor, and FastAPI app version are recorded as
  unsupported-gap package evidence for the existing Rust boundary.

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
- `tests/fixtures/cli_parity/auto_port_v0_18_6_release_parity_evidence.json`
  records the final Rust `0.3.5` release-parity evidence, including required
  Builder verification command results and dirty-worktree package verification
  for this pass.

## Explicit Gaps

Rust still does not ship a `millrace-web` package, HTTP dashboard, static shell,
SSE stream, or dashboard API route. The accepted Rust inspection surface
remains local read-only CLI commands over initialized workspaces, including
`millrace compile graph` and `millrace runs trace <run_id>` as graph/trace
shadow surfaces; the Python `v0.18.5` and `v0.18.6` web package version syncs
are recorded as unsupported-gap evidence rather than a Rust web implementation.
Native
filesystem watcher integration, live subscription-quota
provider polling, and live Codex/Pi smoke runs remain preview-only or opt-in
surfaces.
