# Rust Port Roadmap

This crate is the Rust rebuild of the Python `millrace-ai` runtime. The Python
package remains the reference implementation while the Rust crate matures.

The historical autonomous-build campaign, seeded ideas, parity-run metrics, and
raw evidence verification scripts live in the separate public documentation
repository:

```text
https://github.com/tim-osterhus/millrace-rs-port-docs
```

## Current Parity Target

The Rust `0.2.1` release target is Python `v0.17.4` at commit
`304e537964ff772c815689b87e4c1e3b805c656c`, ported from the previous Rust
parity baseline of Python `v0.17.3`.

The stable surface for parity is operator-visible behavior:

- CLI command names, output shape, and exit behavior
- `millrace-agents/` workspace layout
- canonical task/spec/incident/learning markdown documents
- runtime JSON artifacts
- compiled-plan semantics and currentness fingerprints
- daemon locking, mailbox intake, watcher intake, pause/resume/stop/reload
- serial tick and daemon scheduling behavior
- runner request/result artifacts
- Codex CLI and Pi RPC adapter contracts
- learning, Arbiter, closure, recovery, and usage-governance behavior

Rust source layout does not need to mirror the Python module layout. The contract
is the CLI plus the on-disk workspace format.

## Implementation Domains

- `contracts`: typed enums, stage metadata, runtime JSON, work documents
- `workspace`: paths, initialization, baseline assets, queue/state stores, locks
- `compiler`: mode resolution, graph materialization, fingerprints, diagnostics
- `runtime`: startup, tick, supervisor, routing, result application, monitoring
- `runners`: fake runner, Codex CLI, Pi RPC, dispatcher, artifacts
- `cli`: parser, rendering, read-only commands, intake/control/skills surfaces

## Release Evidence

- `CHANGELOG.md` records the Rust `0.2.1` release-facing summary.
- `ROADMAP.md` records the crate-level current release target and explicit
  gaps.
- `docs/runtime/` records Rust runtime docs for learning no-op outcomes,
  trigger destination safety, and runtime inspection boundaries.
- `docs/source-package-map.md` records source ownership, package include rules,
  and the intentional absence of a Rust web-dashboard package.
- `tests/fixtures/cli_parity/auto_port_v0_17_4_parity_evidence.json` ties the
  Python `v0.17.3..v0.17.4` behavior delta to Rust tests and fixtures.
- `tests/fixtures/cli_parity/auto_port_v0_17_4_release_parity_evidence.json`
  ties the Python `v0.17.3..v0.17.4` source/test/docs/package changes to Rust
  tests, docs, package metadata, managed assets, and release-readiness
  commands.
- `tests/parity_cli.rs` rejects missing, malformed, unknown, stale, or omitted
  Rust test references in the final auto-port fixtures.

## Explicit Parity Gaps

- Python v0.17.3 added the optional `packages/millrace-web` read-only
  dashboard, and Python v0.17.4 only syncs that package's version and
  `millrace-ai>=0.17.4` dependency. The Rust crate does not currently implement
  a web server, static dashboard shell, SSE event stream, or separate dashboard
  package. Its deferred reader evidence names the workspace registry, summary
  DTO, queue, run, snapshot, baseline, compiled-plan, Arbiter, and
  usage-governance readers. The accepted Rust inspection target remains local
  read-only CLI commands over initialized workspaces, so the dashboard is
  recorded as an intentional Arbiter-visible unsupported gap in
  `tests/fixtures/cli_parity/web_dashboard_parity_decision.json`.

## Deferred Or Preview Areas

- The Rust crate does not currently claim to self-host the original port
  campaign.
- Native filesystem watcher integration and live subscription-quota integration
  remain preview/deferred surfaces.
- Live Pi RPC smoke coverage requires an operator environment with a configured
  Pi RPC CLI.

For proof of the historical v0.1.0 autonomous port campaign, see
`tim-osterhus/millrace-rs-port-docs`. For the crate-local `0.2.1` release
parity pass, use the fixture and changelog paths listed above.
