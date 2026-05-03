# Millrace Rust Roadmap

This repository tracks the Rust crate parity roadmap for the Python
`millrace-ai` runtime. The Python package remains the production reference; the
Rust crate owns the `millrace` CLI, typed contracts, and local
`millrace-agents/` workspace artifact boundary.

## Current Release Target

The current Rust release target is `0.2.1`, aligned to Python `v0.17.4` at
commit `304e537964ff772c815689b87e4c1e3b805c656c`.

The `0.2.1` patch release closes the Python `v0.17.3..v0.17.4` parity delta:

- learning stages support first-class no-op terminal outcomes for Analyst,
  Professor, and Curator
- `no_op` is preserved as a non-success result class in contracts, runtime
  JSON, run inspection, and parity fixtures
- generic success-triggered learning starts at Analyst
- direct Curator learning triggers require `target_skill_id` or
  `preferred_output_paths`
- optional Python `millrace-web` `v0.17.4` changes are recorded as version and
  dependency sync evidence for the existing unsupported Rust dashboard gap

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
- `tests/fixtures/cli_parity/auto_port_v0_17_4_release_parity_evidence.json`
  records the final Rust `0.2.1` release evidence.

## Explicit Gaps

Rust still does not ship a `millrace-web` package, HTTP dashboard, static shell,
or SSE stream. The accepted Rust inspection surface remains local read-only CLI
commands over initialized workspaces. Native filesystem watcher integration,
live subscription-quota provider polling, and live Codex/Pi smoke runs remain
preview-only or opt-in surfaces.
