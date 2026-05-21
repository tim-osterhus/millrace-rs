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

The Rust `0.5.0` release target is Python `v0.20.0` at commit
`c432786242e9e7cf9f7262ec0ec4f906f4bb7bf7`, ported from the previous Rust
parity baseline of Python `v0.19.0`.

The stable surface for parity is operator-visible behavior:

- CLI command names, output shape, and exit behavior
- `millrace-agents/` workspace layout
- canonical task/probe/spec/incident/learning markdown documents
- runtime JSON artifacts
- compiled-stage-graph export artifacts and read-only graph CLI output
- run-trace artifacts and read-only trace CLI output
- probe queue lifecycle and Recon packet handoff artifacts
- compiled-plan semantics and currentness fingerprints
- daemon locking, mailbox intake, watcher intake, pause/resume/stop/reload
- serial tick and daemon scheduling behavior
- runner request/result artifacts
- Codex CLI and Pi RPC adapter contracts
- execution capability contracts, grants, gates, approvals, and evidence
- workflow primitive registries, schema epochs, lanes, request context,
  runtime effects, failure policy, and Blueprint Planning assets
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

- `CHANGELOG.md` records the Rust `0.5.0` release-facing summary.
- `ROADMAP.md` records the crate-level current release target and explicit
  gaps.
- `docs/runtime/` records Rust runtime docs for graph exports, run traces,
  learning no-op outcomes, trigger destination safety, and runtime inspection
  boundaries.
- `docs/source-package-map.md` records source ownership, package include rules,
  and the intentional absence of a Rust web-dashboard package.
- `tests/fixtures/cli_parity/auto_port_v0_18_0_parity_evidence.json` ties the
  Python `v0.17.4..v0.18.0` behavior delta to Rust tests and fixtures.
- `tests/fixtures/cli_parity/auto_port_v0_18_0_release_parity_evidence.json`
  ties the Python `v0.17.4..v0.18.0` source/test/docs/package changes to Rust
  tests, docs, package metadata, graph/trace CLI evidence, package readiness,
  and release-readiness commands.
- `tests/fixtures/cli_parity/auto_port_v0_18_1_parity_evidence.json` ties the
  Python `v0.18.0..v0.18.1` probe/Recon behavior delta to Rust tests and
  fixtures.
- `tests/fixtures/cli_parity/auto_port_v0_18_1_release_parity_evidence.json`
  ties the Python `v0.18.0..v0.18.1` source/test/docs/package changes to Rust
  tests, docs, package metadata, Recon/probe evidence, package readiness,
  release-readiness commands, and explicit web package unsupported-gap
  evidence.
- `tests/fixtures/cli_parity/auto_port_v0_18_2_parity_evidence.json` ties the
  Python `v0.18.1..v0.18.2` Integrator/status/Recon/ownership behavior delta
  to Rust tests and fixtures.
- `tests/fixtures/cli_parity/auto_port_v0_18_2_release_parity_evidence.json`
  ties the Python `v0.18.1..v0.18.2` source/test/docs/package changes to Rust
  tests, docs, package metadata, Integrator/status/Recon/ownership evidence,
  package readiness, required release-readiness command results, the
  dirty-worktree publish dry-run limitation, allow-dirty dry-run/package
  substitutes, and explicit web package unsupported-gap evidence.
- `tests/fixtures/cli_parity/auto_port_v0_18_3_parity_evidence.json` ties the
  Python `v0.18.2..v0.18.3` Librarian/learning-trigger/runner-metadata
  behavior delta to Rust tests and fixtures.
- `tests/fixtures/cli_parity/auto_port_v0_18_3_release_parity_evidence.json`
  ties the Python `v0.18.2..v0.18.3` source/test/docs/package changes to Rust
  tests, docs, package metadata, Librarian/learning-trigger/runner-metadata and
  shipped skill lint evidence, package readiness, required Builder verification
  command results, and explicit web package unsupported-gap evidence.
- `tests/fixtures/cli_parity/auto_port_v0_18_4_parity_evidence.json` ties the
  Python `v0.18.3..v0.18.4` blocked-recovery, retry CLI, auto-recovery
  config/status, and daemon recovery behavior delta to Rust tests and
  fixtures.
- `tests/fixtures/cli_parity/auto_port_v0_18_4_release_parity_evidence.json`
  ties the Python `v0.18.3..v0.18.4` source/test/docs/package changes to Rust
  tests, docs, package metadata, runner failure metadata, blocked metadata,
  audited retry CLI, auto-recovery config/status, daemon recovery evidence,
  package readiness, required Builder verification command results, and
  explicit web package unsupported-gap evidence.
- `tests/fixtures/cli_parity/auto_port_v0_18_6_parity_evidence.json` ties the
  Python `v0.18.4..v0.18.6` operator intervention and durable idea-source
  behavior delta to Rust tests and fixtures.
- `tests/fixtures/cli_parity/auto_port_v0_18_6_release_parity_evidence.json`
  ties the Python `v0.18.4..v0.18.6` source/test/docs/package changes to Rust
  tests, docs, package metadata, operator intervention evidence, durable
  idea-source and closure-recovery evidence, package readiness, required
  Builder verification command results, and explicit web package
  unsupported-gap evidence.
- `tests/fixtures/cli_parity/auto_port_v0_19_0_parity_evidence.json` ties the
  Python `v0.18.6..v0.19.0` execution capability governance delta to Rust
  tests and fixtures.
- `tests/fixtures/cli_parity/auto_port_v0_19_0_release_parity_evidence.json`
  ties the Python `v0.18.6..v0.19.0` source/test/docs/package changes to Rust
  tests, docs, package metadata, capability contracts/config/grants/gates,
  approval CLI/runtime-control behavior, runner support/evidence metadata,
  run-inspection output, package readiness, required Builder verification
  command results, and explicit web package unsupported-gap evidence.
- `tests/fixtures/cli_parity/auto_port_v0_20_0_parity_evidence.json` ties the
  Python `v0.19.0..v0.20.0` workflow primitive, compiler authority, schema
  epoch, lane, request-context, runtime-effect, failure-policy, Blueprint,
  CLI/status, docs, package, and web evidence delta to Rust tests and fixtures.
- `tests/fixtures/cli_parity/auto_port_v0_20_0_release_parity_evidence.json`
  ties the Python `v0.19.0..v0.20.0` source/test/docs/package changes to Rust
  tests, docs, package metadata, workflow primitive authority, Blueprint
  Planning runtime behavior, status/run-inspection output, package readiness,
  required Builder verification command results, and explicit web package
  unsupported-gap evidence.
- `tests/parity_cli.rs` rejects missing, malformed, unknown, stale, or omitted
  Rust test references in the final auto-port fixtures.

## Explicit Parity Gaps

- Python v0.17.3 added the optional `packages/millrace-web` read-only
  dashboard, Python v0.17.4 synced that package's version and
  `millrace-ai>=0.17.4` dependency, Python v0.18.0 adds compiled graph
  exports, run-trace summaries, recent-trace Flow overlays, trace outcome
  labels, and version/dependency sync for that package, Python v0.18.1 syncs
  the package version, `millrace-ai>=0.18.1` dependency floor, and FastAPI
  application version, Python v0.18.2, v0.18.3, v0.18.4, v0.18.5, v0.18.6,
  and v0.19.0 repeat that package/runtime version sync through `0.19.0`, and
  Python v0.20.0 adds summary model, queue-reader, and static dashboard UI
  changes while syncing the package to `0.20.0`. The Rust crate does not
  currently implement a web server, static dashboard shell, SSE event stream,
  dashboard HTTP API, or separate dashboard package. Its deferred reader
  evidence names the workspace registry, summary DTO, queue, run, snapshot,
  baseline, compiled-plan, Arbiter, usage-governance, graph, and trace reader
  surfaces. The accepted Rust inspection target remains local read-only CLI
  commands over initialized workspaces, including `millrace compile graph` and
  `millrace runs trace <run_id>`, so the dashboard is recorded as an
  intentional Arbiter-visible unsupported gap in
  `tests/fixtures/cli_parity/web_dashboard_parity_decision.json`.

## Deferred Or Preview Areas

- The Rust crate does not currently claim to self-host the original port
  campaign.
- Native filesystem watcher integration and live subscription-quota integration
  remain preview/deferred surfaces.
- Live Pi RPC smoke coverage requires an operator environment with a configured
  Pi RPC CLI.

For proof of the historical v0.1.0 autonomous port campaign, see
`tim-osterhus/millrace-rs-port-docs`. For the crate-local `0.5.0` release
evidence pass, use the fixture and changelog paths listed above.
