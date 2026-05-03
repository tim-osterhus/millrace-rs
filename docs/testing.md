# Testing

The Rust crate uses integration tests as the primary parity harness.

Run the always-on suite:

```bash
cargo test --all-targets
```

Run formatting and docs checks:

```bash
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

Run the published-package verification path:

```bash
cargo package --allow-dirty
```

## Test Surface

The test suite covers:

- compiler assets, contracts, materialization, persistence, and Python fixture
  parity
- public contract exports and runtime JSON schemas
- work-document parsing and rendering
- CLI parity for init, status, config, modes, queue, skills, control, runs,
  doctor, upgrade, and run commands
- Codex CLI and Pi RPC runner adapter construction and artifact normalization
- serial runtime and daemon runtime behavior
- workspace paths, initialization, managed assets, doctor checks, queue/state
  stores, runtime control, and runtime locks

Some live smoke tests are gated because they require real local credentials,
network access, or provider CLIs:

```bash
MILLRACE_REAL_CODEX_SMOKE=1 cargo test --test runners_live_smoke codex_real_adapter_live_smoke -- --ignored
MILLRACE_REAL_PI_SMOKE=1 cargo test --test runners_live_smoke pi_real_adapter_live_smoke -- --ignored
```
