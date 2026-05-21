# Testing

The Rust crate uses integration tests as the primary parity harness.

Run the always-on suite:

```bash
cargo test --all
```

Run formatting and docs checks:

```bash
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

Run the published-package verification path on a clean release candidate:

```bash
cargo publish --dry-run
```

During active Millrace Builder/Checker worktree validation, the `0.5.0`
release fixture records offline package verification as Builder evidence.
The non-uploading dirty-worktree substitutes are:

```bash
cargo publish --dry-run --allow-dirty
cargo package --allow-dirty --offline
```

## Test Surface

The test suite covers:

- compiler assets, contracts, workflow primitive authority, materialization,
  persistence, and Python fixture parity
- public contract exports and runtime JSON schemas
- work-document parsing and rendering
- CLI parity for init, status text/JSON diagnostics, config, modes, queue,
  skills, control, runs, doctor, upgrade, and run commands
- Codex CLI and Pi RPC runner adapter construction and artifact normalization
- serial runtime and daemon runtime behavior
- workspace paths, initialization, managed assets, doctor checks, queue/state
  stores, runtime control, and runtime locks
- Integrator contracts/assets/compiler graph coverage for execution stage
  metadata, managed live/baseline assets, `execution.with_integrator`
  materialization/export, workspace baseline synchronization, opt-in integrated
  mode resolution, runtime routing, and run-trace evidence
- release fixtures through Rust `0.5.0` version metadata, docs, package
  include rules including generated-cache exclusions, Python `v0.20.0` source
  references, required release-readiness command results, package verification
  evidence, and explicit web-gap evidence
- target-facing Python `v0.18.2..v0.18.3` guardrail fixtures for Rust `0.3.3`
  Librarian contracts/assets/graph/modes, Planner-to-Librarian triggers,
  learning request artifact metadata, runner normalization metadata, shipped
  skill lint guidance, docs/version, final release checks, package dry-run
  evidence, web-package evidence, generated scout mappings, and no-live
  guarantees, with those behavior targets now implemented
- target-facing Python `v0.18.3..v0.18.4` guardrail fixtures and final Rust
  `0.3.4` release evidence for blocked metadata diagnostics, audited `queue retry-blocked` behavior,
  `auto_recovery` config/status defaults and next-tick change boundaries,
  daemon stranded-dependency recovery gates, release checks, generated scout
  mappings, web-package evidence, and no-live guarantees, with the runner
  failure classifier metadata, blocked metadata persistence, manual retry CLI,
  auto-recovery config/status, and daemon recovery slices now implemented
  through typed runtime JSON contracts, runner normalization coverage, serial
  runtime persistence tests, queue-store requeue primitive coverage, focused
  `AutoRecoveryConfig` daemon startup/config tests, config-boundary tests,
  `config show` parity coverage in `tests/parity_cli.rs`, daemon
  auto-requeue diagnostics/event coverage, and same-cycle dependent dispatch
  suppression; docs/version and final release evidence are reconciled in
  `tests/fixtures/cli_parity/auto_port_v0_18_4_release_parity_evidence.json`
- target-facing Python `v0.18.4..v0.18.6` guardrail fixtures and final Rust
  `0.3.5` release evidence for operator intervention mailbox payloads,
  archive/audit ledgers, direct and daemon-routed runtime-control behavior,
  queue/status read-only evidence, durable idea-source behavior, closure
  recovery evidence, required checks, generated scout mappings,
  `millrace-web` v0.18.5/v0.18.6 package evidence, and no-live guarantees,
  with those behavior targets now implemented through contract, CLI,
  queue-store, runtime-control, serial runtime, daemon runtime, and fixture
  tests; docs/version and final release evidence are reconciled in
  `tests/fixtures/cli_parity/auto_port_v0_18_6_release_parity_evidence.json`
- target-facing Python `v0.18.6..v0.19.0` guardrail fixtures for planned Rust
  `0.4.0` execution capability contracts/config, compiled grants, approvals,
  runtime gates, runner support/evidence metadata, inspection surfaces,
  generated scout mappings, required checks, `millrace-web` v0.19.0 package
  evidence, and no-live guarantees, with the capability contracts/config slice,
  compiled capability grant slice, runner support/evidence slice, and runtime
  capability gate/approval-storage slice now implemented through focused
  capability contract tests, runtime JSON approval payload tests, public export
  tests, `tests/compiler_capability_grants.rs`,
  `tests/runners_capability_support.rs`, `tests/runtime_capability_gates.rs`,
  runner adapter and normalization tests, serial/daemon pre-dispatch gate
  coverage, approval CLI/runtime-control direct and daemon-mailbox coverage,
  runtime prompt/artifact metadata coverage, and `config show`/`compile show`
  parity coverage; docs/version and final release evidence are reconciled in
  `tests/fixtures/cli_parity/auto_port_v0_19_0_release_parity_evidence.json`
- target-facing Python `v0.19.0..v0.20.0` guardrail fixtures for planned Rust
  `0.5.0` workflow primitive assets, compiler authority, schema epochs, lanes,
  request context, runtime effects/failure policy, Blueprint Planning, CLI
  `run once` removal, all 249 generated scout paths, required checks,
  `millrace-web` v0.20.0 package evidence, and no-live guarantees, with the
  workflow primitive contracts/assets slice now implemented through focused
  workflow primitive, Blueprint, runtime JSON, public export, compiler asset,
  and workspace initialization tests, and with compiler authority validation now
  implemented through `tests/compiler_workflow_primitives.rs`, compiler
  contracts/assets/materialization/persistence/parity coverage, and
  `compile show`/`compile graph` parity checks for primitive fingerprints, lane
  policy, request-context profiles, terminal action mappings, runtime-effect
  rules, completion behavior, workspace schema epoch authority, Blueprint
  references, and pending-plan evidence. The workspace schema epoch and generic
  lifecycle runtime-consumer slice is now implemented through
  `tests/workspace_schema_epoch.rs`, `tests/workspace_work_item_adapters.rs`,
  `tests/workspace_doctor.rs`, `tests/workspace_queue_state_stores.rs`,
  `tests/runtime_serial.rs`, and `tests/workspace_init_parity.rs`, covering
  marker persistence, daemon-owned archive/reset refusal, clean mutable-state
  initialization, startup compatibility checks, generic work-item adapters,
  queue claim metadata, and compiled terminal-action lifecycle moves.
  The lanes/request-context inspection slice is now implemented through
  `tests/runtime_lanes.rs`, `tests/runtime_request_context.rs`,
  `tests/runtime_run_inspection.rs`, `tests/runtime_daemon.rs`,
  `tests/runtime_serial.rs`, `tests/runners_normalization.rs`, and
  `tests/parity_cli.rs`, covering durable lane state, lane conflict dispatch,
  pending-plan and launch-plan preservation, deterministic context bundles,
  runner/stage-result context metadata, and status/monitor/run-inspection
  evidence. The runtime effects/failure-policy slice is now implemented through
  `tests/runtime_effects.rs`, `tests/runtime_failure_policy.rs`,
  `tests/runtime_serial.rs`, `tests/runtime_daemon.rs`, and
  `tests/runtime_run_inspection.rs`, covering compiled rule selection,
  decision/result artifacts, Planner disposition handling, runtime-owned source
  lifecycle intents, failure-policy matching by origin/class/phase/handler and
  source terminal state, monitor/status/run-inspection evidence, and run-trace
  artifacts. The Blueprint Planning runtime slice is now implemented through
  `tests/blueprint_contracts.rs`, `tests/blueprint_effects.rs`,
  `tests/blueprint_planning_loop.rs`, `tests/runtime_effects.rs`,
  `tests/runtime_request_context.rs`, and
  `tests/compiler_workflow_primitives.rs`, covering manifest/draft state,
  Manager/Contractor/Evaluator Blueprint effects, approved packet/evaluation
  and promotion records, generated execution task promotion, rejection
  critique persistence and route-back, duplicate/partial-mutation blocking,
  Planner disposition mismatch blocking, same-lineage closure blockers, and
  drained Arbiter readiness. CLI/status run-once removal and bounded
  `run daemon --max-ticks 1` parity are covered in `tests/parity_cli.rs`;
  docs/version and final release evidence are reconciled in
  `tests/fixtures/cli_parity/auto_port_v0_20_0_release_parity_evidence.json`.
- Recon invalid-handoff hardening coverage for handoff-specific emitted-id
  validation, generated task/spec id checks before import, durable
  `recon_handoff_invalid` runtime error evidence, active-probe blocking, and
  compiler graph validation that rejects direct Recon handoff edges to stage
  nodes
- stage/work-item ownership coverage for the typed ownership matrix,
  `StageRunRequest` validation, serial and daemon pre-runner guards, stale
  pairing runtime error/event evidence, active-artifact requeue behavior,
  snapshot clearing, and closure-target Arbiter exemption
- Librarian lifecycle coverage for Planner-triggered install requests,
  targeted Librarian dispatch, complete/no-op done transitions, blocked
  recoverable-failure evidence, runner source metadata preservation, and daemon
  trace evidence

Some live smoke tests are gated because they require real local credentials,
network access, or provider CLIs:

```bash
MILLRACE_REAL_CODEX_SMOKE=1 cargo test --test runners_live_smoke codex_real_adapter_live_smoke -- --ignored
MILLRACE_REAL_PI_SMOKE=1 cargo test --test runners_live_smoke pi_real_adapter_live_smoke -- --ignored
```
