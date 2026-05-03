# Architecture

The Rust crate is organized around Millrace authority boundaries rather than a
line-for-line translation of the Python implementation.

## Domains

- `contracts`: typed public artifact models, enums, terminal markers, stage
  metadata, and work-document models.
- `workspace`: the filesystem contract for `millrace-agents/`, including init,
  baseline manifests, queue/state stores, runtime locks, managed assets, and
  doctor checks.
- `compiler`: mode and graph authority, materialized run plans, fingerprints,
  diagnostics, and currentness.
- `runtime`: startup, once-mode ticks, daemon supervisor, mailbox/watcher intake,
  monitor rendering, routing, result application, recovery, closure, and usage
  governance.
- `runners`: stage request construction, fake runner, Codex CLI adapter, Pi RPC
  adapter, process capture, token extraction, normalization, and artifacts.
- `cli`: operator command parsing and rendering.

## Contract Boundary

The stable contract is:

- the `millrace` binary
- operator-visible CLI output and exit behavior
- the on-disk workspace under `millrace-agents/`
- JSON and markdown artifact shapes

Internal Rust module boundaries may continue to evolve.
