# Changelog

All notable user-facing changes to the experimental Rust `millrace-ai` crate
are documented in this file.

## [0.3.4] - 2026-05-13

### Added

- Added Rust release evidence for the Python `v0.18.3..v0.18.4` parity pass,
  covering runner failure classifier metadata, blocked item metadata,
  `queue retry-blocked`, `[auto_recovery]` config/status, daemon
  stranded-dependency auto-recovery, docs, package verification, and the
  optional `millrace-web` `v0.18.4` package/version unsupported-gap surface.
- Added release fixture coverage for v0.18.4 parity fixtures, runtime docs,
  source package mapping, required Builder verification command results,
  package verification, generated-cache package exclusions, and forbidden
  release actions.

### Changed

- Bumped the Rust crate, lockfile package metadata, and version-visible CLI
  output from `0.3.3` to `0.3.4`.
- Updated README, roadmap, source-package map, runtime docs, outline, fixture
  docs, and release evidence to target Python `v0.18.4` at
  `516e947e90155b6436dbc9efcf932254f34bc39c`.

### Known Gaps

- The optional Python `packages/millrace-web` `v0.18.4` package version,
  `millrace-ai>=0.18.4` dependency floor, and FastAPI app version are recorded
  as explicit package/version evidence for the existing unsupported Rust web
  gap. Rust still does not ship a web server, static dashboard shell, SSE
  stream, dashboard HTTP API, or separate `millrace-web` package.
- Native filesystem watcher integration, live subscription-quota provider
  polling, and live Codex/Pi smoke runs remain opt-in or preview-only.

## [0.3.3] - 2026-05-12

### Added

- Added Rust release evidence for the Python `v0.18.2..v0.18.3` parity pass,
  covering Librarian contracts/assets, learning graph and mode bindings,
  Planner-to-Librarian trigger metadata, learning request artifact metadata,
  runner normalization metadata, shipped skill lint guidance, docs, package
  include readiness, and the optional `millrace-web` `v0.18.3`
  package/version unsupported-gap surface.
- Added release fixture coverage for v0.18.3 parity fixtures, runtime docs,
  source package mapping, required Builder verification command results,
  package verification, and forbidden release actions.

### Changed

- Bumped the Rust crate, lockfile package metadata, and version-visible CLI
  output from `0.3.2` to `0.3.3`.
- Updated README, roadmap, source-package map, runtime docs, outline, fixture
  docs, and release evidence to target Python `v0.18.3` at
  `6556e55c8463ce9256716bc425a49059b4c5981c`.

### Known Gaps

- The optional Python `packages/millrace-web` `v0.18.3` package version,
  `millrace-ai>=0.18.3` dependency floor, and FastAPI app version are recorded
  as explicit package/version evidence for the existing unsupported Rust web
  gap. Rust still does not ship a web server, static dashboard shell, SSE
  stream, dashboard HTTP API, or separate `millrace-web` package.
- Native filesystem watcher integration, live subscription-quota provider
  polling, and live Codex/Pi smoke runs remain opt-in or preview-only.

## [0.3.2] - 2026-05-10

### Added

- Added Rust release evidence for the Python `v0.18.1..v0.18.2` parity pass,
  covering Integrator contracts/assets, opt-in integrated Codex modes, status
  JSON diagnostics, Recon invalid-handoff hardening, graph validation guards,
  stage/work-item ownership validation, package include readiness, and the
  optional `millrace-web` `v0.18.2` package/version unsupported-gap surface.
- Added release fixture coverage for the new Integrator managed assets,
  integrated mode assets, runtime docs, v0.18.2 parity fixtures, source package
  mapping, required release-readiness command results, the plain publish
  dry-run dirty-worktree limitation, allow-dirty dry-run/package substitutes,
  and forbidden release actions.

### Changed

- Bumped the Rust crate, lockfile package metadata, and version-visible CLI
  output from `0.3.1` to `0.3.2`.
- Updated README, roadmap, source-package map, runtime docs, outline, fixture
  docs, and release evidence to target Python `v0.18.2` at
  `5444cb9485ea90b67b2ed6ba7e0723ae9fe7b79f`.

### Known Gaps

- The optional Python `packages/millrace-web` `v0.18.2` package version,
  `millrace-ai>=0.18.2` dependency floor, and FastAPI app version are recorded
  as explicit package/version evidence for the existing unsupported Rust web
  gap. Rust still does not ship a web server, static dashboard shell, SSE
  stream, dashboard HTTP API, or separate `millrace-web` package.
- The plain clean-worktree `cargo publish --dry-run` check remains recorded as
  a dirty-worktree limitation for this uncommitted Builder pass; normal
  Millrace stages do not perform publish, tag, push, upload, or deployment
  actions.
- Native filesystem watcher integration, live subscription-quota provider
  polling, and live Codex/Pi smoke runs remain opt-in or preview-only.

## [0.3.1] - 2026-05-09

### Added

- Added Python `v0.18.0..v0.18.1` release evidence for probe work documents,
  Recon packet contracts, Recon managed assets, probe queue lifecycle,
  add-probe CLI/mailbox/read-only behavior, runtime Recon routing/result
  application, package include readiness, and the optional `millrace-web`
  `v0.18.1` package/version unsupported-gap surface.
- Added release fixture coverage for Recon entrypoint, stage-kind registry,
  `recon-core` skill, mode runner bindings, probe/recon parity fixtures,
  docs, version metadata, and required release-readiness checks.

### Changed

- Bumped the Rust crate, lockfile package metadata, and version-visible CLI
  output from `0.3.0` to `0.3.1`.
- Updated README, roadmap, source-package map, runtime docs, outline, fixture
  docs, and release evidence to target Python `v0.18.1` at
  `0396c7852793b212d31345862b38a7d6f3f02854`.

### Known Gaps

- The optional Python `packages/millrace-web` `v0.18.1` package version,
  `millrace-ai>=0.18.1` dependency floor, and FastAPI app version are recorded
  as explicit package/version evidence for the existing unsupported Rust web
  gap. Rust still does not ship a web server, static dashboard shell, SSE
  stream, dashboard HTTP API, or separate `millrace-web` package.
- Native filesystem watcher integration, live subscription-quota provider
  polling, and live Codex/Pi smoke runs remain opt-in or preview-only.

## [0.3.0] - 2026-05-05

### Added

- Added Python `v0.17.4..v0.18.0` release evidence for compiled-stage-graph
  exports, run-trace persistence and inspection, read-only graph/trace CLI
  commands, runtime docs, package include readiness, and the optional
  `millrace-web` graph/trace unsupported-gap surface.
- Added Rust runtime docs for compiled stage graph exports and per-run
  `run_trace.json` inspection, including fallback behavior for older or
  malformed run directories and the distinction between compiled topology
  authority and historical trace evidence.

### Changed

- Bumped the Rust crate, lockfile package metadata, and version-visible CLI
  output from `0.2.1` to `0.3.0`.
- Updated README, roadmap, source-package map, outline, fixture docs, and
  release evidence to target Python `v0.18.0` at
  `e4ccf099c8345a8b8708cdaa1ac510bdc7851387`.

### Known Gaps

- The optional Python `packages/millrace-web` `v0.18.0` graph/trace changes
  are represented as explicit unsupported-gap and shadow-CLI evidence for the
  existing Rust CLI/workspace boundary. Rust still does not ship a web server,
  static dashboard shell, SSE stream, dashboard HTTP API, or separate
  `millrace-web` package.
- Native filesystem watcher integration, live subscription-quota provider
  polling, and live Codex/Pi smoke runs remain opt-in or preview-only.

## [0.2.1] - 2026-05-03

### Added

- Added Python `v0.17.3..v0.17.4` release evidence for learning no-op
  terminal outcomes, the `no_op` result class, Analyst-first generic learning,
  direct Curator trigger destination safety, and run-inspection/runtime JSON
  no-op coverage.
- Added Rust runtime docs and release evidence for the `0.2.1` parity release,
  including package include coverage for the crate roadmap and runtime docs.

### Changed

- Bumped the Rust crate, lockfile package metadata, and version-visible CLI
  output from `0.2.0` to `0.2.1`.
- Updated README, roadmap, source-package map, outline, fixture docs, and
  release evidence to target Python `v0.17.4` at
  `304e537964ff772c815689b87e4c1e3b805c656c`.

### Known Gaps

- The optional Python `packages/millrace-web` `v0.17.4` changes are represented
  as version/dependency sync evidence for the existing unsupported Rust
  dashboard gap. Rust still does not ship a web server, static dashboard shell,
  SSE stream, or separate `millrace-web` package.
- Native filesystem watcher integration, live subscription-quota provider
  polling, and live Codex/Pi smoke runs remain opt-in or preview-only.

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
