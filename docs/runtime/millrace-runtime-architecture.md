# Runtime Architecture

The Rust runtime keeps queue and run mutation behind runtime-owned boundaries:
startup loads compiled-plan authority, tick activation claims at most one
eligible work item or closure target, stage dispatch persists request/result
artifacts, and router decisions apply queue/state transitions through typed
helpers.

For Python `v0.18.0` parity, dispatch also writes best-effort
`run_trace.json` evidence under `millrace-agents/runs/<run_id>/` after
stage-result persistence and authoritative router decisions. Trace nodes name
the stage result, terminal/result class, runner/model/thinking metadata,
duration, token usage, and artifact refs. Trace edges name the router action,
reason, next node or terminal state, and spawned learning-request or incident
refs when routing creates follow-up work. Trace write failures emit runtime
events without changing otherwise valid stage or routing outcomes.

For Python `v0.18.1` parity, startup can claim queued probe documents into the
Planning plane as `recon` stage requests. Recon results validate and persist
`recon_packet.md` under `millrace-agents/recon/packets/`, then move the active
probe to `done/` or `blocked/`. Handoff results enqueue exactly one generated
task or spec with probe/recon references and root-intake lineage; no-op results
close the probe without downstream work. Packet/result mismatches schedule
planning recovery without moving the active probe.

For Python `v0.18.2` parity, invalid Recon handoff artifacts now block the
active probe with `recon_handoff_invalid` runtime error evidence instead of
falling into ordinary planning recovery. Handoff-specific emitted-id validation,
generated task/spec id checks, malformed packet handling, and direct-stage
graph-edge rejection keep generated-work promotion runtime-owned.

The same parity line adds stage/work-item ownership validation before serial or
daemon runner dispatch. Stale active pairings are rejected before a runner is
invoked, the active artifact is safely requeued or blocked through queue
helpers, runtime error/report evidence uses
`stage_work_item_ownership_invalid`, and closure-target Arbiter activation
remains valid without an active work item.

Learning request activation uses active request documents under
`millrace-agents/learning/requests/active/`. For Python `v0.17.4` parity,
stage-specific no-op terminal results move the active learning request to
`millrace-agents/learning/requests/done/`, not `blocked/`, while preserving the
stage-result, terminal-marker, router-decision, and run-inspection evidence.

Runtime-generated learning requests copy compiled trigger destination metadata
into both queued work documents and trigger metadata. This preserves
`target_skill_id` and `preferred_output_paths` for downstream learning stages
without allowing destination-less direct Curator requests.

For Python `v0.18.3` parity, Planner completion in learning-enabled modes can
enqueue targeted Librarian install requests. Those requests preserve the
stage-result artifact, Planner-produced artifacts, and source work-item
metadata. Targeted Librarian dispatch uses learning-request active paths,
`LIBRARIAN_COMPLETE` and `LIBRARIAN_NOOP` move requests to done, and Librarian
`BLOCKED` preserves recoverable-failure blocked evidence.

For Python `v0.18.4` parity, runner normalization can classify retryable
blocked failures as `network_unavailable`, `provider_unavailable`,
`provider_rate_limited`, or `runner_timeout`, with blocked-origin,
failure-scope, auto-requeue candidacy, and classifier-code metadata. Runtime
routing persists blocked item diagnostics under
`millrace-agents/diagnostics/blocked/task-<TASK_ID>.json`, emits
`blocked_item_metadata_written`, and exposes the shared audited blocked-task
requeue transition used by both manual retry and daemon auto-recovery.
`millrace queue retry-blocked <TASK_ID>` applies safe-id, root-spec,
retryability, retry-budget, force, and live-lock guards before moving a blocked
task back to queue. Daemon idle cycles use the typed `[auto_recovery]` policy
to requeue at most one eligible stranded predecessor, write
`diagnostics/auto-recovery/` evidence, emit
`blocked_dependency_auto_requeued` or `blocked_dependency_auto_requeue_skipped`,
and return a recovered tick without dispatching queued dependents in that same
cycle.

For the Python `v0.18.6` operator intervention surface, daemon mailbox intake
applies the same cancel, archive, supersede, retarget, resolve, and invalid
incident archive commands through the shared queue-store mutation boundary when
the daemon is idle, archives processed or failed mailbox commands with
evidence, and defers intervention commands while runtime planes are active.
Read-only queue inspection now exposes intervention archive counters and can
render cancelled, superseded, and operator-resolved records. Status text/JSON
projects the latest operator intervention from runtime event evidence, and the
basic monitor renders direct, mailbox-applied, and deferred intervention
events without changing the underlying event names.

The same `v0.18.6` parity line preserves watcher-seeded idea sources under
`millrace-agents/intake/ideas/<root_idea_id>.md`. Generated root specs list
that durable runtime-owned artifact before the transient `ideas/inbox` path,
and `idea_normalized_to_spec` events include `source_artifact` evidence.
Closure-target creation and backlog backfill prefer durable idea artifacts
before legacy spec references or inbox fallbacks. When backlog-drain recovery
cannot find any valid root idea source candidate, Planning is marked blocked
with `missing_root_idea_source`, the runtime emits `root_idea_source_missing`
candidate evidence, and the daemon loop continues through normal bounded-cycle
behavior.

The optional Python `millrace-web` package remains outside the accepted Rust
runtime boundary, including the Python `v0.18.5` and `v0.18.6`
package/runtime version syncs.
Rust inspection stays local and read-only through CLI commands
such as `queue ls/show`, `status show`, `runs ls/show/tail`, `modes show`,
`config show`, `compile show`, `compile graph`, and `runs trace <run_id>`.
Those graph/trace CLI commands shadow Python web graph and trace readers
without adding a Rust web server, dashboard API, static shell, SSE stream, or
separate dashboard package.
