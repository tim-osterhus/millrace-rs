# CLI Reference

Installed command: `millrace`

## Version

Use either form to print the Rust crate version:

```bash
millrace --version
millrace version
```

For Rust `0.3.5`, both commands print `millrace 0.3.5`.

## Probe Intake And Inspection

Rust `0.3.1` adds the Python `v0.18.1` probe intake surface. Operators can
import canonical probe markdown or JSON either through the top-level alias or
the grouped queue command:

```bash
millrace add-probe probe.md --workspace <workspace>
millrace queue add-probe probe.json --workspace <workspace>
```

When a daemon owns the workspace, both forms route through the mailbox command
name `add_probe`; otherwise they write directly through the same runtime-control
boundary. `millrace queue ls` reports probe queue and lifecycle counts, and
`millrace queue show <probe-id>` renders canonical probe fields without moving
or normalizing the inspected document.

## Operator Intervention Inspection

The v0.18.6 operator intervention surface keeps mutating queue and incident
actions behind `RuntimeControl` while exposing their archived outcomes through
read-only commands. `millrace queue ls` includes archive counters for
`cancelled_task_count`, `superseded_task_count`,
`cancelled_incident_count`, and `operator_resolved_incident_count`.
`millrace queue show <id>` can inspect cancelled, superseded, and
operator-resolved archive records across the supported task, probe, spec, and
incident lifecycle surfaces without moving or repairing them.

`millrace status` and `millrace status show --format json` include
`latest_operator_intervention` when matching runtime event evidence exists.
The payload records the event type, timestamp, optional work item kind/id, and
destination path. Basic daemon monitor output also renders direct operator
intervention events, mailbox-applied intervention events, and deferred
intervention events without renaming the underlying runtime event types.

## Operator Intervention Commands

Rust `0.3.5` adds the Python `v0.18.5` operator intervention command family.
These commands are for bad intake or stale queue cleanup, not retryable
transient failures:

```bash
millrace queue cancel <WORK_ITEM_ID> --workspace <workspace> --reason "bad intake"
millrace queue archive-blocked <TASK_ID> --workspace <workspace> --reason "do not retry"
millrace queue supersede <OLD_TASK_ID> --workspace <workspace> --replacement <NEW_TASK_ID> --reason "superseded" --cascade retarget
millrace queue retarget-dependency <TASK_ID> --workspace <workspace> --from <OLD_DEPENDENCY_ID> --to <NEW_DEPENDENCY_ID> --reason "replacement ready"
millrace incident resolve <INCIDENT_ID> --workspace <workspace> --reason "operator handled"
millrace incident cancel <INCIDENT_ID> --workspace <workspace> --reason "duplicate"
millrace incident archive-invalid <FILENAME> --workspace <workspace> --reason "malformed artifact"
```

All commands validate safe identifiers and non-empty reasons, render the shared
control-result fields, route through the daemon mailbox when a daemon owns the
workspace, and archive artifacts with `operator_intervention` ledger and
runtime-event evidence. `queue archive-blocked` is intentionally separate from
`queue retry-blocked`: archive-blocked retires a blocked task as operator
cleanup, while retry-blocked requeues a retryable transient blocked task.

## Status JSON Diagnostics

Rust `0.3.2` adds the Python `v0.18.2` status JSON diagnostics surface.
`millrace status` and `millrace status show` accept `--format text|json`; text
remains the default, while JSON reports the shared read-only status payload,
including active state, queue depths, closure-target diagnostics,
`blocked_idle`, `current_failure_class`, the latest runtime error report path,
and `latest_operator_intervention` when intervention event evidence exists.
`millrace status watch` remains text-only and rejects JSON format requests
deterministically.

## Graph And Trace Inspection

`millrace compile graph` exports the selected compiled-plan topology as text or
JSON. It supports initialized workspaces, the built-in mode/config selection
used by adjacent compile commands, optional plane filtering, and output-file
writes.

```bash
millrace compile graph --workspace <workspace>
millrace compile graph --workspace <workspace> --mode learning_codex
millrace compile graph --workspace <workspace> --plane planning --format json
millrace compile graph --workspace <workspace> --output planning-graph.json
```

`millrace runs trace <run_id>` inspects one persisted run as text or JSON. New
runs can include `run_trace.json`; older or malformed runs are rendered through
read-only fallback inspection from `stage_results/*.json`.

```bash
millrace runs trace <run_id> --workspace <workspace>
millrace runs trace <run_id> --workspace <workspace> --format json
millrace runs trace <run_id> --workspace <workspace> --output run-trace.json
```

Both commands are inspection surfaces. They do not acquire runtime ownership,
do not add or move queue items, and do not repair or normalize inspected run
artifacts.

## Web Boundary

Python `millrace-web` exposes graph and trace data through read-only dashboard
routes, and Python `v0.18.5`/`v0.18.6` sync that optional package through
version `0.18.6`.
Rust `0.3.5` shadows the accepted local inspection behavior through the CLI
commands above and keeps the optional web dashboard as an explicit unsupported
gap. No Rust web server, dashboard HTTP API, static shell, SSE stream, separate
dashboard package, or Rust-managed web asset is part of this crate release.

## Librarian And Learning Evidence

Rust `0.3.3` adds the Python `v0.18.3` Librarian learning-stage parity surface.
There is no new operator web or server command. Operators inspect the result
through the existing local CLI and workspace artifacts:

- `millrace compile show` and `millrace compile graph` expose the compiled
  Librarian node, legal terminals, required skill, and learning trigger rules.
- `millrace runs show`, `millrace runs tail`, and `millrace runs trace` expose
  persisted Librarian request/result/trace evidence.
- `millrace status` continues to report learning queue depths and active run
  state without mutating the workspace.

## Blocked Recovery And Auto-Recovery Evidence

Rust `0.3.4` adds the Python `v0.18.4` blocked recovery parity surface.
Operators can manually requeue a blocked task through the audited retry command:

```bash
millrace queue retry-blocked <TASK_ID> --workspace <workspace> --reason "retry after provider outage"
millrace queue retry-blocked <TASK_ID> --workspace <workspace> --reason "operator override" --force
millrace queue retry-blocked <TASK_ID> --workspace <workspace> --reason "same-root retry" --root-spec-id <ROOT_SPEC_ID>
```

The command refuses unsafe work item ids, active daemon ownership, non-blocked
tasks, exhausted retry budgets, root-spec mismatches, and non-retryable blocked
metadata unless `--force` is explicit. Successful retries write queue audit
JSONL evidence, refresh queue depths, and emit `blocked_task_requeued`.

`millrace config show` includes the Python-exposed `auto_recovery.enabled`
status key. The Rust config loader accepts `[auto_recovery]` with
`enabled`, `blocked_dependency_retry_enabled`,
`max_auto_requeues_per_work_item`, and `cooldown_seconds`; every
`auto_recovery.*` field applies at the next daemon tick.

When daemon auto-recovery is enabled and the daemon is otherwise idle, Rust can
requeue one eligible retryable blocked predecessor for queued same-lineage
execution work. The daemon writes `millrace-agents/diagnostics/auto-recovery/`
evidence, emits `blocked_dependency_auto_requeued` or
`blocked_dependency_auto_requeue_skipped`, renders basic monitor lines, and
does not dispatch the dependent task in the same recovered cycle.
