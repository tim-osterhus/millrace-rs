# CLI Reference

Installed command: `millrace`

## Version

Use either form to print the Rust crate version:

```bash
millrace --version
millrace version
```

For Rust `0.5.0`, both commands print `millrace 0.5.0`.

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

## Approval Commands

The v0.19.0 execution capability approval surface exposes durable approval
inspection and decisions through the same initialized-workspace CLI boundary:

```bash
millrace approvals ls --workspace <workspace>
millrace approvals show <APPROVAL_ID> --workspace <workspace>
millrace approvals approve <APPROVAL_ID> --workspace <workspace> --reason "approved"
millrace approvals deny <APPROVAL_ID> --workspace <workspace> --reason "denied"
```

`approvals ls` renders pending and resolved records with stable grant context,
and `approvals show` prints the full approval JSON without mutating state.
Approve/deny commands validate safe approval ids and non-empty reasons, render
the shared control-result fields, resolve the approval directly when no daemon
owns the workspace, and route through `approve_execution_capability` or
`deny_execution_capability` mailbox envelopes when a daemon owns it.

## Status JSON Diagnostics

Rust `0.3.2` adds the Python `v0.18.2` status JSON diagnostics surface.
`millrace status` and `millrace status show` accept `--format text|json`; text
remains the default, while JSON reports the shared read-only status payload,
including active state, queue depths, closure-target diagnostics,
`blocked_idle`, `current_failure_class`, the latest runtime error report path,
and `latest_operator_intervention` when intervention event evidence exists.
`millrace status watch` remains text-only and rejects JSON format requests
deterministically.

For Python `v0.20.0` parity, status and run inspection also expose compiled
lane state, pending compiled-plan evidence, latest failure origin,
runtime-effect decision/result refs, source lifecycle intent evidence,
Blueprint counters/artifacts, generated task refs, request-context bundle refs,
artifact parse validity, and runtime route/effect outcome without mutating the
inspected workspace.

## Run Commands

The public `millrace run once` command is removed for Python `v0.20.0` parity.
Use daemon mode with one tick when an operator needs a bounded single-cycle
run:

```bash
millrace run daemon --workspace <workspace> --max-ticks 1
```

`--max-ticks 1` preserves the one-tick operator workflow through daemon startup,
runtime ownership, compiled-plan authority, runner dispatch, and final daemon
summary rendering. The removed command is rejected instead of silently aliasing
to daemon mode.

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

Python `millrace-web` exposes graph, trace, summary, queue-reader, and static
dashboard data through read-only dashboard routes, and Python `v0.20.0` syncs
that optional package through version `0.20.0`.
Rust `0.5.0` shadows the accepted local inspection behavior through the CLI
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

## Execution Capability Config

The v0.19.0 capability contracts/config slice adds Rust config loading for
`[execution_capabilities]`. Defaults match the Python surface for the
implemented contract boundary: capability governance is enabled, unknown
capabilities are denied, advisory grants are allowed, strict required-advisory
failure is disabled, raw network access is denied, package install and git
mutation require approval, and shell execution plus workspace writes are
allowed.

`millrace config show` exposes the three operator-facing keys implemented in
this slice:

```text
execution_capabilities.enabled
execution_capabilities.allow_advisory_grants
execution_capabilities.fail_required_advisory
```

All `execution_capabilities.*` config fields are recompile-boundary fields.
Runtime capability gates now enforce compiled grants before serial or daemon
runner invocation, write `capability_gate.<request_id>.json`, emit
`capability_gate_evaluated`, and use durable
`millrace-agents/approvals/{pending,resolved}` records for approval-required
grants. `millrace approvals` commands now list/show those records and resolve
approve/deny decisions directly or through daemon-routed approval mailbox
commands.

`millrace compile show` now surfaces the compiler-owned grant evidence without
running any stage. The output includes plan and per-plane
`execution_capabilities.*` summary counts, each stage's
`execution_capability_policy_fingerprint`, compact
`execution_capability_grant` lines, and any `execution_capability_warning`
lines such as required advisory grants. These lines describe sealed compiled
plan authority only; the runtime gate consumes that compiled authority during
dispatch rather than making `compile show` a mutating command.

`millrace runs show` now renders stage-result capability metadata, when present,
as compact `capability_grant` and `capability_support` lines. The lines are
read-only evidence from runner artifacts and normalized stage results; they do
not allow stages to publish, upload, deploy, push, tag, or otherwise perform
release actions.
