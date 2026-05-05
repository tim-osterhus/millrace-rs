# Millrace Rust Roadmap

This repository tracks the Rust crate parity roadmap for the Python
`millrace-ai` runtime. The Python package remains the production reference; the
Rust crate owns the `millrace` CLI, typed contracts, and local
`millrace-agents/` workspace artifact boundary.

## Current Release Target

The current Rust release target is `0.3.0`, aligned to Python `v0.18.0` at
commit `e4ccf099c8345a8b8708cdaa1ac510bdc7851387`.

The `0.3.0` release closes the Python `v0.17.4..v0.18.0` parity delta:

- compiled-stage-graph exports expose selected compiled-plan topology by plane
  without becoming routing authority
- `run_trace.json` artifacts preserve historical stage-result nodes,
  router-decision edges, artifact refs, spawned work, and trace status
- `millrace compile graph` and `millrace runs trace <run_id>` provide
  read-only text/JSON inspection plus output-file support
- older or malformed runs remain inspectable through read-only fallback trace
  rendering
- optional Python `millrace-web` `v0.18.0` graph/trace API, Flow overlay,
  trace outcome label, version/dependency sync, and read-only/no-lock changes
  are recorded as unsupported-gap and shadow-CLI evidence for the existing Rust
  boundary

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
- `tests/fixtures/cli_parity/auto_port_v0_18_0_release_parity_evidence.json`
  records the final Rust `0.3.0` release evidence.

## Explicit Gaps

Rust still does not ship a `millrace-web` package, HTTP dashboard, static shell,
SSE stream, or dashboard API route. The accepted Rust inspection surface
remains local read-only CLI commands over initialized workspaces, including
`millrace compile graph` and `millrace runs trace <run_id>` as graph/trace
shadow surfaces. Native filesystem watcher integration, live subscription-quota
provider polling, and live Codex/Pi smoke runs remain preview-only or opt-in
surfaces.
