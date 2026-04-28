# Millrace Rust Port Roadmap

This document defines the working plan for bringing the Rust implementation of
Millrace to behavioral parity with the Python `millrace-ai` runtime.

The Python implementation at `../millrace-py` is the current reference
implementation. At the time this roadmap was written, that checkout is pinned
to `v0.16.1`.

## Parity Definition

Parity means the Rust `millrace` binary preserves the operator-visible contract
of the Python implementation:

- the CLI command surface and exit behavior
- the `millrace-agents/` workspace layout
- canonical markdown work documents
- JSON state, run, mailbox, compile, baseline, and governance artifacts
- compiled-plan semantics and fingerprint currentness
- deterministic runtime tick ordering
- daemon locking, mailbox intake, watcher intake, pause/resume/stop/reload
- plane-concurrent daemon scheduling with serialized result application
- runner request/result contracts
- Codex CLI and Pi RPC adapter behavior
- learning-plane, Arbiter, closure, recovery, and usage-governance behavior

Rust source layout does not need to mirror Python module layout. The stable
surface is the CLI plus the on-disk workspace contract.

## Non-Goals

- Do not make the Rust runtime self-host the port.
- Do not initialize Millrace in this repository until an operator deliberately
  chooses to do that.
- Do not translate Python files one-for-one when a Rust-native module boundary
  better preserves the contract.
- Do not let a test fixture write `millrace-agents/` into the repository root.

## Target Rust Shape

The eventual crate should be split into authority domains rather than one large
CLI implementation:

- `contracts`: typed artifact schemas, enums, stage metadata, terminal markers
- `workspace`: path model, initialization, baseline, queue/state stores, locks
- `assets`: embedded modes, graphs, entrypoints, skills, and registries
- `compiler`: mode resolution, graph materialization, fingerprints, diagnostics
- `runtime`: lifecycle, tick cycle, supervisor, routing, result application
- `runners`: request construction, adapters, normalization, artifacts
- `cli`: operator commands and rendering

Likely crates and libraries:

- `clap` for CLI parsing
- `serde`, `serde_json`, and `toml_edit` for contract and config IO
- `thiserror` and `miette` for structured errors
- `tokio` for daemon worker orchestration
- `notify` for watcher intake
- `time`, `uuid`, and `sha2` or `blake3` for runtime identity and fingerprints
- `assert_cmd`, `tempfile`, `serde_json`, and `insta` for parity tests

## Harness Strategy

The parity harness lives in Rust integration tests under `tests/`.

Always-on tests should be cheap and deterministic. They may read the Python
package version directly from `../millrace-py/src`, but they should not require
Python dependencies unless a test explicitly says so.

Full Python CLI probes should run only when a Python environment with the
reference package dependencies is available. Those probes should use temporary
workspaces and never initialize Millrace in the Rust repository checkout.

The harness compares normalized behavior rather than raw bytes when the two
implementations are expected to differ. Known acceptable differences include:

- Rust crate version vs Python package version
- absolute temporary paths
- timestamps
- generated ids
- run directory names
- ordering where the Python contract does not promise ordering

Golden snapshots are useful after a command surface stabilizes, but the first
priority is explicit structural assertions around required files, JSON fields,
state transitions, and exit codes.

## Large Slices

### Slice 0: Bootstrap Harness

Status: started.

Acceptance:

- Rust CLI exposes `millrace --version` and `millrace version`.
- Test helpers can run the Rust binary and read the Python reference version.
- Test helpers create paired temporary Python/Rust workspaces.
- No test creates `millrace-agents/` in the repository root.

### Slice 1: Contracts

Port the typed contract layer first:

- stage planes, legal markers, result classes, and stage metadata
- work document models and headed markdown parsing/rendering
- runtime snapshots, recovery counters, mailbox envelopes, compile diagnostics
- stage result envelopes and token usage models

Acceptance:

- Rust parses and renders representative Python fixture documents.
- JSON contracts round-trip against Python-produced fixtures.
- invalid terminal markers and illegal stage/result combinations fail clearly.

### Slice 2: Workspace Substrate

Implement filesystem ownership before runtime behavior:

- `millrace init`
- workspace path model
- baseline manifest creation
- managed asset deployment
- queue/state stores
- runtime lock inspection
- doctor checks

Acceptance:

- Python and Rust `init` produce equivalent required tree structure.
- selected bootstrap files match after normalization.
- queue/store unit tests cover claim, transition, block, done, and repair paths.

### Slice 3: Assets And Compiler

Port embedded assets and compile authority:

- modes, graphs, stage-kind registry, entrypoints, skills
- graph materialization
- completion behavior
- learning triggers
- plane concurrency policy
- compile-input fingerprints
- current/stale/missing inspection

Acceptance:

- Rust `compile validate` succeeds for all built-in modes.
- `compiled_plan.json` matches Python structure after normalizing ids and paths.
- stale-plan refusal cases are covered.

### Slice 4: CLI Read/Write Surface

Implement operator commands that do not require real runner execution:

- `queue`
- `config`
- `modes`
- `skills`
- `status`
- `runs`
- `doctor`
- `upgrade`
- control commands that mutate offline state or enqueue mailbox commands

Acceptance:

- command exit codes match Python.
- key output lines match normalized snapshots.
- commands never implicitly compile unless Python does.

### Slice 5: Serial Runtime

Implement `millrace run once` with a fake runner:

- startup lifecycle
- deterministic tick cycle
- claim ordering
- stage request rendering
- result persistence
- router/result application
- recovery counters
- closure target and Arbiter activation

Acceptance:

- fake-runner scenarios reproduce Python queue and state transitions.
- runtime-owned mutation remains single-writer.
- stage code never directly mutates authoritative queue state.

### Slice 6: Daemon Runtime

Implement long-running orchestration:

- daemon ownership lock
- mailbox intake
- watcher/poll intake
- stop, pause, resume, retry, reload behavior
- basic monitor rendering
- plane-concurrent scheduling
- serialized result application from worker completions

Acceptance:

- default modes remain serial.
- learning modes allow one learning lane beside permitted foreground work.
- config reload waits for active planes to drain.
- shutdown clears running state without corrupting active artifacts.

### Slice 7: Runner Adapters

Port real runner integrations after the runtime works with a fake runner:

- Codex CLI command construction
- Codex artifact capture and token extraction
- Pi RPC JSONL transport
- runner registry and dispatcher
- timeout/error normalization

Acceptance:

- adapter artifacts match the Python contract.
- fake runner remains the default CI path.
- real adapters have smoke tests gated behind explicit environment variables.

### Slice 8: Advanced Parity

Finish the high-value edge surfaces:

- usage governance
- subscription quota telemetry
- learning promotions and skill evidence
- closure lineage drift diagnostics and repair
- run inspection depth
- full E2E handoff scenarios

Acceptance:

- Python fixture scenarios can be replayed through Rust.
- docs state which surfaces are fully compatible and which are still preview.

## Using Python Millrace To Drive The Port

Python Millrace is the right orchestrator once Slice 0 creates rails and the
backlog is decomposed into acceptance-gated work items. The port should be
managed as long-running staged work against `millrace-rs`, with the Python
runtime owning queue state and progress.

That should happen later as an operator action. This repository is prepared not
to track `millrace-agents/`, but this bootstrap does not initialize Millrace.
