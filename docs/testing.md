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

During active Millrace Builder/Checker worktree validation, the `0.3.1`
release fixture records the plain dry-run dirty-worktree limitation alongside
the non-uploading substitutes:

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
- CLI parity for init, status, config, modes, queue, skills, control, runs,
  doctor, upgrade, and run commands
- Codex CLI and Pi RPC runner adapter construction and artifact normalization
- serial runtime and daemon runtime behavior
- workspace paths, initialization, managed assets, doctor checks, queue/state
  stores, runtime control, and runtime locks
- release fixtures through Rust `0.3.1` version metadata, docs, package
  include rules, Python `v0.18.1` source references, required release-check
  results, package verification evidence, and explicit web-gap evidence

Some live smoke tests are gated because they require real local credentials,
network access, or provider CLIs:

```bash
MILLRACE_REAL_CODEX_SMOKE=1 cargo test --test runners_live_smoke codex_real_adapter_live_smoke -- --ignored
MILLRACE_REAL_PI_SMOKE=1 cargo test --test runners_live_smoke pi_real_adapter_live_smoke -- --ignored
```
