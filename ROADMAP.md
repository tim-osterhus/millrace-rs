# Millrace Rust Roadmap

This repository tracks the Rust crate parity roadmap for the Python
`millrace-ai` runtime. The Python package remains the production reference; the
Rust crate owns the `millrace` CLI, typed contracts, and local
`millrace-agents/` workspace artifact boundary.

## Current Release Target

The current Rust release target is `0.4.0`, aligned to Python `v0.19.0` at
commit `efb9c5881f524d23dcb78aecfc96fdf7cda9d26f`, ported from the Rust
`0.3.5` parity baseline for Python `v0.18.6`.

The `0.4.0` release closes the Python `v0.18.6..v0.19.0` parity delta:

- Execution capability contracts, config, compiled grants, grant fingerprints,
  support decisions, approval mailbox payloads, and approval records are typed
  and exported at the Rust contract boundary.
- Compiler materialization seals per-node execution capability grants,
  warnings, policy fingerprints, and summary counts into frozen plans and
  compiled-stage-graph exports.
- Serial and daemon runtime dispatch evaluate capability gates before invoking
  runners, write gate artifacts, emit gate events, and block denied,
  unsupported, unresolved approval-required, or missing-evidence required
  grants as recoverable runtime-policy failures.
- Runner invocation/completion/raw-result/stage-result metadata carry compiled
  grants, adapter support decisions, evidence refs, missing-evidence refs, and
  capability failure classes; `millrace runs show` renders compact
  `capability_grant` and `capability_support` lines for that evidence.
- `millrace approvals ls/show/approve/deny`, `millrace config show`, and
  `millrace compile show` expose the implemented capability-governance surfaces
  without giving stages queue or release authority.
- optional Python `millrace-web` `v0.19.0` package version, runtime dependency
  floor, and FastAPI app version are recorded as
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
- `tests/fixtures/cli_parity/auto_port_v0_19_0_release_parity_evidence.json`
  records the final Rust `0.4.0` release-parity evidence, including required
  Builder verification command results and dirty-worktree package verification
  for this pass.

## Explicit Gaps

Rust still does not ship a `millrace-web` package, HTTP dashboard, static shell,
SSE stream, or dashboard API route. The accepted Rust inspection surface
remains local read-only CLI commands over initialized workspaces, including
`millrace compile graph` and `millrace runs trace <run_id>` as graph/trace
shadow surfaces; the Python `v0.19.0` web package version sync is recorded as
unsupported-gap evidence rather than a Rust web implementation.
Native
filesystem watcher integration, live subscription-quota
provider polling, and live Codex/Pi smoke runs remain preview-only or opt-in
surfaces.
