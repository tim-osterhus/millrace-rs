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

For the Python `v0.19.0` execution capability surface, Rust now implements the
contract/config boundary, compiler grant boundary, and runner support/evidence
boundary. Public contracts cover capability scopes, approval policy refs,
capability requests, policy overrides, execution grants, runner support
decisions, capability id aliases and validation, approval-required grant
invariants, evidence normalization, and stable grant fingerprints. Runtime JSON
accepts the `approve_execution_capability` and `deny_execution_capability`
mailbox command values with safe approval-id and non-empty reason payload
validation.
`[execution_capabilities]` config defaults match the Python contract surface,
all `execution_capabilities.*` fields are recompile-boundary fields, and
`config show` exposes the three implemented status keys. Compiler
materialization resolves Python-style mode and graph-node declarations plus
stage-kind declarations into sealed per-node execution capability grants,
warnings, policy fingerprints, and plan/plane summaries; disabled capability
policy compiles zero grants, and strict required-advisory policy fails
advisory required grants such as `workspace.read`. Stage requests now preserve
compiled grants plus runner support decisions in JSON and rendered prompt
context. The runner boundary propagates grants, support decisions, capability
evidence refs, missing evidence refs, and `failure_capability_class` through
invocation/completion artifacts, raw results, and normalized stage-result
metadata; Codex CLI support reporting remains advisory for broad `maximum`
permission posture, Pi RPC remains conservative/advisory for remote
boundaries, and missing required evidence normalizes as
`capability_evidence_missing`. Runtime capability gates now evaluate compiled
grants before serial or daemon runner invocation, persist
`capability_gate.<request_id>.json`, emit `capability_gate_evaluated`, and
turn denied, unsupported, unresolved approval-required, or missing-evidence
required grants into recoverable runtime-policy failures without invoking the
runner. Approval-required grants use durable approval records under
`millrace-agents/approvals/{pending,resolved}` keyed by run/request/grant.
Approval CLI/runtime-control now lists, shows, approves, and denies those
records. Offline decisions resolve pending approvals directly; daemon-owned
decisions are applied by mailbox intake at the runtime-owned boundary with
processed/failed archive evidence and approval-decision runtime events.

For Python `v0.20.0` workflow authority parity, runtime startup validates the
workspace schema epoch marker against compiled schema authority before
dispatch. Archive/reset helpers refuse daemon-owned workspaces, move stale
mutable runtime state under `millrace-agents/archives/`, initialize clean
runtime state, and require a fresh compile before work resumes.

Queue claiming and terminal lifecycle movement now consume compiled work-item
family, queue-claim, terminal-action, and lifecycle mutation plans for tasks,
specs, probes, incidents, learning requests, and Blueprint drafts. Stage
results remain evidence; runtime-owned effect/lifecycle handlers perform the
single-writer queue mutations.

Daemon scheduling persists lane runtime state, launch-plan authority, and
pending compiled-plan evidence. Config reloads that compile while active work
exists are held as pending plans until active lanes drain, preserving the
launch plan that selected the running work.

Stage request construction writes deterministic request-context bundle and
prompt-context artifacts. Runner normalization and run inspection preserve
context refs, artifact parse validity, runtime route outcome, runtime-effect
outcome, latest failure origin, and generated path evidence as distinct fields.

Runtime effects select compiled rules from the active plan after stage-result
normalization. Effect dispatch writes decision/result artifacts, runs packaged
handlers such as Planner disposition and Blueprint promotion, applies
runtime-owned source lifecycle intents, and interprets failures through
compiled runtime failure policies by origin, class, mutation phase, handler,
source node, source terminal, plane, and family.

Blueprint Planning adds runtime state for manifests, drafts, candidate packets,
evaluations, critiques, promotions, generated execution tasks, legacy
root-keyed manifest reads, remediation manifests, duplicate detection, and
idempotent replay. Arbiter closure is suppressed until same-lineage Blueprint
artifacts and generated execution work drain.

The optional Python `millrace-web` package remains outside the accepted Rust
runtime boundary, including the Python `v0.20.0` package/runtime version sync
and dashboard summary/static UI changes. Rust inspection stays local and
read-only through CLI commands such as `queue ls/show`, `status show`,
`runs ls/show/tail`, `modes show`, `config show`, `compile show`,
`compile graph`, and `runs trace <run_id>`. Those graph/trace CLI commands
shadow Python web graph and trace readers without adding a Rust web server,
dashboard API, static shell, SSE stream, or separate dashboard package.
