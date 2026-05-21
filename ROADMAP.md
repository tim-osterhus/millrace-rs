# Millrace Rust Roadmap

This repository tracks the Rust crate parity roadmap for the Python
`millrace-ai` runtime. The Python package remains the production reference; the
Rust crate owns the `millrace` CLI, typed contracts, and local
`millrace-agents/` workspace artifact boundary.

## Current Release Target

The current Rust release target is `0.5.0`, aligned to Python `v0.20.0` at
commit `c432786242e9e7cf9f7262ec0ec4f906f4bb7bf7`, ported from the Rust
`0.4.0` parity baseline for Python `v0.19.0`.

The `0.5.0` release closes the Python `v0.19.0..v0.20.0` parity delta:

- Workflow primitive registries are compiler-validated runtime authority for
  work-item families, adapters, artifact contracts, queue claims, terminal
  actions, lifecycle mutation plans, runtime effect rules, failure policies,
  request-context profiles, lanes, completion behavior, and schema epochs.
- Workspace schema epoch markers, safe archive/reset behavior, generic
  work-item adapters, and runtime-owned terminal-action lifecycle intents are
  implemented across the built-in families and Blueprint draft sources.
- Scheduler lanes, launch-plan preservation, pending-plan evidence, and
  deterministic request-context bundles are visible through status, monitor,
  runner artifacts, stage results, and run inspection.
- Runtime effects select compiled rules, persist decision/result artifacts,
  apply source lifecycle intents, and route or block failures by compiled
  failure policy.
- Blueprint Planning ships as opt-in graph/mode/stage-kind/entrypoint/skill
  assets with runtime state for manifests, drafts, packets, evaluations,
  critiques, promotions, generated task promotion, and same-lineage closure
  suppression.
- The public `millrace run once` command is removed; `millrace run daemon
  --max-ticks 1` is the bounded one-tick operator path.
- Optional Python `millrace-web` `v0.20.0` package, summary, queue-reader, and
  static UI changes are recorded as unsupported-gap evidence for the existing
  Rust boundary.

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
- `tests/fixtures/cli_parity/auto_port_v0_20_0_release_parity_evidence.json`
  records the final Rust `0.5.0` release-parity evidence, including required
  Builder verification command results and dirty-worktree package verification
  for this pass.

## Explicit Gaps

Rust still does not ship a `millrace-web` package, HTTP dashboard, static shell,
SSE stream, or dashboard API route. The accepted Rust inspection surface
remains local read-only CLI commands over initialized workspaces, including
`millrace compile graph` and `millrace runs trace <run_id>` as graph/trace
shadow surfaces; the Python `v0.20.0` web package/dashboard summary changes
are recorded as unsupported-gap evidence rather than a Rust web implementation.
Native
filesystem watcher integration, live subscription-quota
provider polling, and live Codex/Pi smoke runs remain preview-only or opt-in
surfaces.
