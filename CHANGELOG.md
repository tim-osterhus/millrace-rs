# Changelog

All notable user-facing changes to the experimental Rust `millrace-ai` crate
are documented in this file.

## [0.2.0] - 2026-05-03

### Added

- Added Python `v0.16.1..v0.17.3` parity evidence for runner-neutral
  `thinking_level`, Codex reasoning-effort compatibility, Pi `--thinking`
  mapping, daemon monitor idle throttling, closure-target actionability, task
  lifecycle integrity, and the optional `millrace-web` dashboard decision.
- Added final auto-port release evidence in
  `tests/fixtures/cli_parity/auto_port_v0_17_3_release_parity_evidence.json`.
- Added release package include coverage for docs, managed baseline assets,
  parity fixtures, test sources, and test support code.

### Changed

- Bumped the Rust crate, lockfile package metadata, and version-visible CLI
  output from `0.1.0` to `0.2.0`.
- Updated managed runtime assets to match the Python v0.17.3 consultant
  incident guidance and Manager duplicate-task guardrail.
- Reconciled README, roadmap, source-package mapping, outline, and fixture docs
  for the completed v0.17.3 auto-port and remaining preview-only surfaces.

### Known Gaps

- The optional Python `packages/millrace-web` dashboard remains an explicit
  unsupported Rust parity gap. Rust continues to expose local read-only CLI
  inspection instead of a web server, static shell, SSE stream, or separate
  dashboard package.
- Native filesystem watcher integration, live subscription-quota provider
  polling, and live Codex/Pi smoke runs remain opt-in or preview-only.
