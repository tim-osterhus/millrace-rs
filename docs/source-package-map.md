# Rust Source Package Map

This document records the Rust crate surfaces that carry the Python
`v0.19.0..v0.20.0` contract into the `millrace-ai` `0.5.0` package.

## Source Ownership

| Contract area | Rust source home | Release evidence |
| --- | --- | --- |
| CLI and read-only operator views, including probe intake, queue probe inspection, operator intervention commands, approval commands, archive lookup, capability config output, run capability evidence output, status JSON diagnostics, public `run once` rejection, bounded `run daemon --max-ticks 1`, and lane/context/effect/Blueprint run inspection | `src/cli/` | `tests/parity_cli.rs` |
| Typed contracts, execution capabilities, workflow primitives, Blueprint documents, Recon packets, work documents, work references, and runtime JSON | `src/contracts/`, `src/work_documents.rs`, `src/recon_packets.rs` | `tests/contracts_runtime_json.rs`, `tests/contracts_work_documents.rs`, `tests/contracts_capabilities.rs`, `tests/contracts_workflow_primitives.rs`, `tests/contracts_blueprint.rs` |
| Compiler, mode, graph, thinking-level materialization, execution capability grant materialization, workflow primitive loading/validation/fingerprints, compiled-stage-graph exports, Recon graph assets, opt-in Integrator graph assets, Librarian learning graph assets, and Blueprint graph/mode assets | `src/compiler/` | `tests/compiler_contracts.rs`, `tests/compiler_materialization.rs`, `tests/compiler_capability_grants.rs`, `tests/compiler_workflow_primitives.rs`, `tests/compiler_parity.rs` |
| Runtime tick, daemon, closure, governance, learning, Recon result application/hardening, Integrator routing, stage/work-item ownership, Planner-to-Librarian triggers, blocked metadata persistence, daemon stranded-dependency recovery, durable idea-source closure recovery, capability approval storage and gates, run-trace persistence, lanes, request context, runtime effects, failure policy, lifecycle intents, Blueprint effects, and monitor behavior | `src/runtime/` | `tests/runtime_serial.rs`, `tests/runtime_daemon.rs`, `tests/runtime_capability_gates.rs`, `tests/runtime_lanes.rs`, `tests/runtime_request_context.rs`, `tests/runtime_effects.rs`, `tests/runtime_failure_policy.rs`, `tests/runtime_run_inspection.rs`, `tests/blueprint_effects.rs`, `tests/blueprint_planning_loop.rs` |
| Runner request/result artifacts, Codex CLI, Pi RPC, fake runner, active work item source metadata normalization, capability support/evidence propagation, and blocked recovery failure classification | `src/runners/` | `tests/runners_codex_cli.rs`, `tests/runners_pi_rpc.rs`, `tests/runners_normalization.rs`, `tests/runners_capability_support.rs`, `tests/runners_live_smoke.rs` |
| Workspace layout, queue/state stores, managed assets, schema epoch markers/archive-reset, generic work-item adapters, compiled queue claims/lifecycle intents, lifecycle integrity, locks, audited blocked-task requeue transitions, operator intervention archive/audit transitions, and Blueprint state | `src/workspace/`, `src/workspace.rs` | `tests/workspace_assets_baseline.rs`, `tests/workspace_queue_state_stores.rs`, `tests/workspace_doctor.rs`, `tests/workspace_schema_epoch.rs`, `tests/workspace_work_item_adapters.rs` |
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

The package include rules also explicitly exclude generated Python cache
artifacts under `__pycache__/` and `*.pyc`/`*.pyo` paths so local bytecode does
not become release evidence.

The package intentionally does not include a Rust `millrace-web` equivalent.
Python `v0.17.3` added `packages/millrace-web/` as a separate optional
read-only dashboard distribution, Python `v0.17.4` synced that package's
version and runtime dependency, Python `v0.18.0` added compiled graph exports,
run-trace summaries, recent-trace Flow overlays, trace outcome labels, package
version/dependency sync, and read-only/no-lock evidence to that web surface,
and Python `v0.18.1`, `v0.18.2`, `v0.18.3`, `v0.18.4`, `v0.18.5`,
`v0.18.6`, and `v0.19.0` repeat that package/runtime version sync through
`0.19.0`. The Rust crate records those
surfaces in
`tests/fixtures/cli_parity/web_dashboard_parity_decision.json` and
`tests/fixtures/cli_parity/auto_port_v0_20_0_release_parity_evidence.json` as
an Arbiter-visible unsupported gap because this repository currently owns the
CLI plus local `millrace-agents/` workspace-artifact boundary. Rust shadows the
graph/trace read surface through `millrace compile graph` and `millrace runs
trace <run_id>` without adding web server, dashboard API, static shell, SSE, or
separate dashboard package assets.

Python `v0.20.0` changes `packages/millrace-web` package metadata, app version,
summary DTOs, queue-reader behavior, static dashboard UI, and tests. Rust
records those paths as package/unsupported-gap evidence only. The Cargo include
rules do not include `packages/millrace-web/**/*`, generated Python cache
artifacts, live `millrace-agents/**` workspaces, `ideas/**`, or `target/**`.

## Python Reference Range

The `0.5.0` release evidence is tied to Python `v0.19.0..v0.20.0`, target tag
`v0.20.0`, commit `c432786242e9e7cf9f7262ec0ec4f906f4bb7bf7`. The
fixture-backed evidence is
`tests/fixtures/cli_parity/auto_port_v0_20_0_release_parity_evidence.json`.
