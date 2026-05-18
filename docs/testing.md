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

During active Millrace Builder/Checker worktree validation, the `0.4.0`
release fixture records offline package verification as Builder evidence.
The non-uploading dirty-worktree substitutes are:

```bash
cargo publish --dry-run --allow-dirty
cargo package --allow-dirty --offline
```

## Test Surface

The test suite covers:

- compiler assets, contracts, materialization, persistence, and Python fixture
  parity
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
- release fixtures through Rust `0.4.0` version metadata, docs, package
  include rules including generated-cache exclusions, Python `v0.19.0` source
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
