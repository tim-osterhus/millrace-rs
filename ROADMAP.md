# Millrace Rust Roadmap

This repository tracks the Rust crate parity roadmap for the Python
`millrace-ai` runtime. The Python package remains the production reference; the
Rust crate owns the `millrace` CLI, typed contracts, and local
`millrace-agents/` workspace artifact boundary.

## Current Release Target

The current Rust release target is `0.3.1`, aligned to Python `v0.18.1` at
commit `0396c7852793b212d31345862b38a7d6f3f02854`.

The `0.3.1` release closes the Python `v0.18.0..v0.18.1` parity delta:

- probe work documents can be imported through `add-probe` and
  `queue add-probe` as canonical markdown or JSON
- the Planning graph includes `probe -> recon`, Recon stage metadata,
  managed Recon assets, mode runner bindings, and `recon-core` skill packaging
- probe queue lifecycle, workspace paths, duplicate protection, queue depth,
  active-probe retry/clear-stale requeue, and read-only queue rendering are
  covered
- Recon stage results persist packets, move active probes to done or blocked,
  enqueue generated task/spec handoffs when requested, and record spawned-work
  run-trace evidence
- optional Python `millrace-web` `v0.18.1` package version, runtime dependency
  floor, and FastAPI app version are recorded as unsupported-gap package
  evidence for the existing Rust boundary

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
- `tests/fixtures/cli_parity/auto_port_v0_18_1_release_parity_evidence.json`
  records the final Rust `0.3.1` release evidence.

## Explicit Gaps

Rust still does not ship a `millrace-web` package, HTTP dashboard, static shell,
SSE stream, or dashboard API route. The accepted Rust inspection surface
remains local read-only CLI commands over initialized workspaces, including
`millrace compile graph` and `millrace runs trace <run_id>` as graph/trace
shadow surfaces; the Python `v0.18.1` web package version sync is recorded as
unsupported-gap evidence rather than a Rust web implementation. Native
filesystem watcher integration, live subscription-quota
provider polling, and live Codex/Pi smoke runs remain preview-only or opt-in
surfaces.
