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

## Deferred Or Preview Areas

- The Rust crate does not currently claim to self-host the original port
  campaign.
- Native filesystem watcher integration and live subscription-quota integration
  remain preview/deferred surfaces.
- Live Pi RPC smoke coverage requires an operator environment with a configured
  Pi RPC CLI.

For proof of the v0.1.0 autonomous port campaign, see
`tim-osterhus/millrace-rs-port-docs`.
