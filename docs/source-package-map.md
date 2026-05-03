# Rust Source Package Map

This document records the Rust crate surfaces that carry the Python
`v0.17.3..v0.17.4` contract into the `millrace-ai` `0.2.1` package.

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
| Runtime and release docs | `README.md`, `ROADMAP.md`, `docs/` | `tests/parity_cli.rs` |

## Package Include Boundary

The crate package intentionally includes:

- `CHANGELOG.md`, `README.md`, `ROADMAP.md`, `LICENSE`, `Cargo.toml`, and
  `Cargo.lock`
- all Markdown files under `docs/`
- Rust sources under `src/`
- managed runtime assets under `src/assets/`
- always-on test sources, support helpers, and fixture evidence under `tests/`

The package intentionally does not include a Rust `millrace-web` equivalent.
Python `v0.17.3` added `packages/millrace-web/` as a separate optional read-only
dashboard distribution, and Python `v0.17.4` syncs that package's version and
runtime dependency to `0.17.4`. The Rust crate records those surfaces in
`tests/fixtures/cli_parity/web_dashboard_parity_decision.json` and
`tests/fixtures/cli_parity/auto_port_v0_17_4_release_parity_evidence.json` as
an Arbiter-visible unsupported gap because this repository currently owns the
CLI plus local `millrace-agents/` workspace-artifact boundary.

## Python Reference Range

The `0.2.1` release evidence is tied to Python `v0.17.3..v0.17.4`, target tag
`v0.17.4`, commit `304e537964ff772c815689b87e4c1e3b805c656c`. The final
fixture-backed evidence is
`tests/fixtures/cli_parity/auto_port_v0_17_4_release_parity_evidence.json`.
