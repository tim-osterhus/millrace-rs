# Millrace Technical Overview

This document is the high-density Rust system map for the experimental
`millrace-ai` crate. The Python package remains the production reference; Rust
`0.4.0` is the crate-local parity target for the Python `v0.18.6..v0.19.0`
release delta.

## Runtime Model

Millrace is a filesystem-backed runtime for long-running agent work. The stable
contract is the `millrace` CLI plus the local `millrace-agents/` workspace
artifact tree. Stage agents do bounded work and emit legal terminal markers;
the runtime owns claim selection, capability gates, approval routing, runner
dispatch, routing, queue movement, recovery counters, closure-target state,
learning request movement, and run evidence.

The Rust crate now exposes typed contract, compiler, workspace, runtime,
runner, and CLI boundaries for the implemented parity surface. It does not
claim production replacement status for the Python runtime.

## Workspace And State

Initialized workspaces contain a managed baseline under
`<workspace>/millrace-agents/`. Runtime-owned state includes:

- task, probe, spec, incident, and learning-request queue lifecycles
- `runtime_snapshot.json`, recovery counters, status markers, compiled-plan
  authority, and baseline manifest evidence
- run-scoped stage request, raw runner result, stage result, terminal marker,
  router decision, runner artifacts, and best-effort `run_trace.json`
- closure-target and Arbiter evidence
- usage-governance state when configured

Stages do not directly mutate queue or status authority. They report terminal
results; the runtime applies state changes through typed helpers.

## Compiler Authority

Rust compiles mode, graph, stage-kind, entrypoint, skill, runner, model,
thinking, timeout, learning-trigger, and completion-behavior inputs into
`compiled_plan.json`. Runtime startup, reload, stage activation, recovery, and
routing consume that frozen plan.

The v0.19.0 parity line adds execution capability requests, policies, grants,
warnings, policy fingerprints, and summary counts to frozen plans. Compile
inspection and graph exports expose that evidence, while runtime dispatch
continues to consume the persisted compiled plan as authority.

## Learning And Librarian

Learning-enabled modes may run the learning plane beside allowed foreground
work according to compiled concurrency policy. Analyst, Professor, Curator, and
Librarian no-op terminals map to `result_class: no_op`.

Planner completion in learning-enabled modes now creates a targeted Librarian
learning request with persisted stage-result evidence, Planner-produced
artifact paths such as `planner_summary.md`, and source work-item kind, id, and
active-path metadata. Librarian complete/no-op results move the request to
done; blocked results preserve recoverable blocked evidence.

## Package Boundary

The Rust crate packages docs, Rust source, managed baseline assets, always-on
tests, fixtures, and support helpers. It intentionally excludes live
`millrace-agents/` runtime workspace artifacts.

Python `packages/millrace-web` v0.19.0 syncs the optional web package version,
runtime dependency floor, and FastAPI app version. Rust records that as
unsupported-gap package evidence and does not add a Rust web server, dashboard
API, static shell, SSE stream, or separate `millrace-web` package.
