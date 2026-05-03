# Rust Source Package Map

This document records the Rust crate surfaces that carry the Python
`v0.16.1..v0.17.3` contract into the `millrace-ai` `0.2.0` package.

## Source Ownership

| Contract area | Rust source home | Release evidence |
| --- | --- | --- |
| CLI and read-only operator views | `src/cli/` | `tests/parity_cli.rs` |
| Typed contracts and runtime JSON | `src/contracts/`, `src/work_documents.rs` | `tests/contracts_runtime_json.rs`, `tests/contracts_work_documents.rs` |
| Compiler, mode, graph, and thinking-level materialization | `src/compiler/` | `tests/compiler_contracts.rs`, `tests/compiler_materialization.rs`, `tests/compiler_parity.rs` |
| Runtime tick, daemon, closure, governance, learning, and monitor behavior | `src/runtime/` | `tests/runtime_serial.rs`, `tests/runtime_daemon.rs` |
| Runner request/result artifacts, Codex CLI, Pi RPC, and fake runner | `src/runners/` | `tests/runners_codex_cli.rs`, `tests/runners_pi_rpc.rs`, `tests/runners_live_smoke.rs` |
| Workspace layout, queue/state stores, managed assets, lifecycle integrity, and locks | `src/workspace/`, `src/workspace.rs` | `tests/workspace_assets_baseline.rs`, `tests/workspace_queue_state_stores.rs`, `tests/workspace_doctor.rs` |
| Managed runtime assets | `src/assets/baseline/` | `tests/workspace_assets_baseline.rs`, `tests/parity_cli.rs` |
| Consolidated parity fixtures | `tests/fixtures/` | `tests/parity_cli.rs`, `tests/compiler_parity.rs` |

## Package Include Boundary

The crate package intentionally includes:

- `CHANGELOG.md`, `README.md`, `LICENSE`, `Cargo.toml`, and `Cargo.lock`
- all Markdown files under `docs/`
- Rust sources under `src/`
- managed runtime assets under `src/assets/`
- always-on test sources, support helpers, and fixture evidence under `tests/`

The package intentionally does not include a Rust `millrace-web` equivalent.
Python `v0.17.3` added `packages/millrace-web/` as a separate optional read-only
dashboard distribution; the Rust crate records that surface in
`tests/fixtures/cli_parity/web_dashboard_parity_decision.json` as an
Arbiter-visible unsupported gap because this repository currently owns the CLI
plus local `millrace-agents/` workspace-artifact boundary.

## Python Reference Range

The `0.2.0` release evidence is tied to Python `v0.16.1..v0.17.3`, target tag
`v0.17.3`, commit `a0d6b1bd5b71284eab7e9a5dcc9f76cee6580aaf`. The final
fixture-backed evidence is
`tests/fixtures/cli_parity/auto_port_v0_17_3_release_parity_evidence.json`.
