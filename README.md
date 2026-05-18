# Millrace Rust

`millrace-ai` is the experimental Rust implementation of Millrace, a
governed runtime for long-running agent work.

The production implementation is currently the Python package
[`millrace-ai`](https://pypi.org/project/millrace-ai/). The Rust `0.4.0`
release consolidates the Python `v0.18.6..v0.19.0` execution capability
governance, approval, runner evidence, run-inspection, docs/version, package,
and web-gap evidence pass on top of the earlier operator-intervention,
blocked-recovery, Librarian, Integrator, probe/Recon, and graph/trace ports
while the crate remains experimental.

## Package Names

```text
Cargo package: millrace-ai
Library crate: millrace_ai
CLI binary:    millrace
Repository:    https://github.com/tim-osterhus/millrace-rs
Website:       https://millrace.ai
```

## Current Status

This crate is intentionally small. It exposes a status API, a `millrace`
binary for version, status/about, `init --workspace <path>`, first
`doctor --workspace <path>` output, and Slice 3 compile validate/show/graph output
for initialized workspaces. The first Slice 4 CLI command framework now lives
behind `millrace_ai::cli`; it shares parsing, initialized-workspace checks, and
rendering while recognizing the primary operator command groups and
compatibility aliases. It implements read-only operator inspection commands for
`queue ls/show`, `status`/`status show` with text/JSON output, bounded
text-only `status watch`, `runs ls/show/tail/trace`, `modes list/show`, and
`config show` including the v0.19.0 execution-capability config keys, plus
queue intake
commands for `queue add-task`, `queue add-probe`, `queue add-spec`, `queue
add-idea`, and the top-level `add-task`/`add-probe`/`add-spec`/`add-idea`
aliases, plus `queue retry-blocked <TASK_ID>` manual blocked-task retry,
v0.18.6 queue intervention commands for `queue cancel`,
`queue archive-blocked`, `queue supersede`, and `queue retarget-dependency`,
`incident resolve`/`incident cancel`/`incident archive-invalid`, and
`queue repair-lineage` preview/apply wiring over the workspace repair boundary.
The intervention commands route through `RuntimeControl` for direct no-daemon
application or active-daemon mailbox envelopes and render the shared
control-result output. It also implements
`millrace approvals ls/show/approve/deny` for durable execution capability
approval inspection and approve/deny routing through the same direct-or-mailbox
control boundary. Read-only queue inspection now reports intervention archive
counts, `queue show` can inspect cancelled, superseded, and
operator-resolved records, `status` text/JSON includes
`latest_operator_intervention` when matching runtime event evidence exists, and
the basic monitor renders direct, mailbox-applied, and deferred intervention
events. It also implements
control commands for `pause`, `resume`, `stop`, `retry-active`,
`clear-stale-state`, and `reload-config`, the `planning retry-active` alias,
`config validate`, and `config reload`, routing mutating commands through the
runtime-control/mailbox facade. The `millrace skills` group now implements
file-backed list, show, search, local/source and fixture/cache-backed remote
install and refresh, learning-mode-gated create/improve queueing, source
promotion, and ZIP export behavior. `millrace upgrade` now implements managed
baseline preview/apply output, safe package updates, missing asset restoration,
conflict refusal, and removed-asset localization. `millrace run once` parses
its normal Rust-exposed options, requires an initialized workspace, starts the
Rust once-mode runtime session, executes exactly one serial tick through the
runtime-configured runner dispatcher, renders operator-facing outcomes, and
releases runtime ownership after covered success and failure paths. `millrace
run daemon` now starts the Rust daemon runtime session, executes the daemon loop
through the same runtime-configured runner dispatcher, renders final daemon
summary lines including `runtime_ticks` and Python-compatible `ticks`, and
supports `--monitor basic` stdout output plus append-mode `--monitor-log` file
fanout with missing parent-directory creation and without enabling stdout
monitor mode. The crate also exposes the
Slice 1 contract boundary through
`millrace_ai::contracts` and `millrace_ai::work_documents`, and the
first Slice 2 workspace substrate library surface through
`millrace_ai::workspace`, including queue and state stores plus runtime
ownership lock helpers. It also exposes the first Slice 3 compiler
contract-model, asset-resolution, compile-input fingerprint, graph
materialization, compiled execution capability grant compilation,
compiled-stage-graph export projection, persisted compiled-plan authority,
and currentness inspection boundary through
`millrace_ai::compiler`, including the Python v0.18.2 Integrator
contracts/assets/compiler graph subset for the opt-in
`execution.with_integrator` graph plus the opt-in `default_codex_integrated`
and `learning_codex_integrated` modes, and the Python v0.18.3 Librarian
learning graph/mode subset for Planner-to-Librarian optional-skill preparation.
Integrated runtime routing now preserves standard Builder -> Checker execution
while the opt-in integrated graph routes Builder success through Integrator
before Checker and records trace evidence.
The runtime contract surface also exposes
Python-compatible `run_trace_graph` contracts plus runtime-owned
`run_trace.json` persistence and read-only fallback inspection helpers. The
first Slice 5 runtime/runner contract, the first Slice 6 daemon startup/config,
supervisor/completion, bounded loop/shutdown, mailbox/reload, watcher
poll-intake, basic monitor rendering, daemon CLI execution boundaries, and Slice
7 shared runner
prompt/artifact/process/registry/dispatcher plus Codex CLI, Pi RPC, runtime
config, and runtime-dispatch integration boundaries are exposed through
`millrace_ai::runtime` and
`millrace_ai::runners`. They include typed
`StageRunRequest` models with runner-neutral `thinking_level` and legacy
Codex reasoning-effort compatibility fields, a
once-mode startup session that validates initialized workspaces, acquires
runtime ownership before compiler/state mutation, loads persisted compiled-plan
authority, projects snapshot state, and detects stale active state; a daemon
startup/config session boundary that uses the same compiled-plan authority,
projects `RuntimeMode::Daemon`, loads daemon config defaults for idle sleep,
watcher settings, runtime runner settings, stage overrides, and
`[usage_governance]` token/quota rule settings, prepares deterministic poll
watcher-session state without native filesystem watchers or work claiming, and
releases only the matching daemon lock on startup failure or close; a daemon
supervisor/completion boundary that evaluates compiled-plan plane-concurrency
policy against active-run snapshot state, keeps default modes serial without an
enabling policy, allows learning beside allowed foreground work in learning
modes, launches stage workers through the injected runner adapter, captures
typed worker outcomes, and drains completions through owner-side metadata
validation and the existing serial normalization, routing, and
result-application helpers; a bounded
daemon loop/shutdown control path that runs supervisor cycles, counts completed
ticks, supports max-tick/no-work idle/stop/process-stopped/blocked exits, uses
test-controllable idle waits, drains completed workers after cycles and during
shutdown, clears stopped daemon state, and releases the matching runtime
ownership lock; and a daemon mailbox/reload handling boundary that drains
control, intake, operator-intervention, retry, clear-stale-state, and reload
commands in deterministic order, applies v0.18.6 operator interventions through
the shared queue-store archive/audit boundary when idle, defers them while
planes are active, archives processed or failed commands with source/error
evidence, defers reload while active planes exist, applies reload after planes
drain with watcher-session, config-version, and compiled-plan snapshot updates,
and preserves the previous plan on recoverable reload diagnostics. The deterministic
tick activation path drains applicable mailbox commands, refreshes queue depths,
returns typed
no-work/paused/stopped/blocked outcomes, claims at most one
compiled-plan-authorized work item or actionable closure target, builds
`StageRunRequest` payloads from materialized runner/model/thinking fields,
writes running markers, projects active-run snapshot state, and emits runtime
events. Runner dispatch persists stage request, raw runner result, normalized
stage result,
terminal marker, router decision, and best-effort `run_trace.json` node/edge
artifacts with thinking-level evidence across success and recoverable failure
paths, routes through compiled graph policy, emits `run_trace_write_failed`
events without failing otherwise valid outcomes, persists recoverable runtime
error context/report evidence, updates last-terminal/result snapshot fields,
applies router
decisions through typed queue/state helpers for stage advancement, completion,
blocked, and handoff outcomes, mutates recovery counters, creates typed handoff
incidents, schedules post-stage application-failure recovery with runtime-error
evidence, applies closure-target Arbiter results for `ARBITER_COMPLETE` close
and `REMEDIATION_NEEDED` remediation incident outcomes, blocks repeated
remediation without intervening execution, and emits `stage_completed`,
`router_decision`, closure, handoff, and recovery events.
Slice 8 usage-governance state and runtime enforcement are implemented as
inert-by-default runtime helpers and state-store contracts for typed governance
state, blockers, stage-result token ledger entries, idempotent ledger
reconciliation from stage-result artifacts, rolling/session/calendar token
windows, subscription quota status/window telemetry, degraded fail-open/fail-closed
policy, threshold blockers, and read-only rendering. Serial and daemon dispatch
now record completed stage-result token evidence once, reconcile missing ledger
entries before new dispatch, apply governance-owned pause sources separately
from operator pauses, block new work at the runtime boundary, and clear only the
governance pause when rolling-window blockers expire. Live subscription-provider
polling remains follow-up work.
Learning promotion and skill-evidence parity is also implemented for the
runtime-owned surfaces: stage-result learning triggers enqueue learning-request
documents with trigger, artifact, `target_skill_id`, and normalized
`preferred_output_paths` evidence in queued request fields, trigger metadata,
and `learning_request_enqueued` events. Planner completion in learning-enabled
modes now creates targeted Librarian install requests with persisted
stage-result, stage-produced Planner artifact, and source work-item metadata;
default non-learning modes do not enqueue those requests. Targeted
`learning_request` work dispatches to the requested learning stage, including
Librarian, and `LIBRARIAN_COMPLETE`/`LIBRARIAN_NOOP` complete active requests
into `learning/requests/done/` with success/no-op semantics while Librarian
`BLOCKED` preserves recoverable-failure blocked evidence.
Stage requests preserve skill-revision evidence in run directories, Curator
`skill_update` artifacts create auditable learning update-candidate promotion
records, deferred records apply only after foreground execution and planning
lanes drain, rejected or blocked Curator decisions preserve decision evidence
without promotion records, and source-packaged skills remain mutable only
through explicit `millrace skills promote` operator commands with audit fields.
Closure-lineage runtime parity now creates or backfills closure targets from
root-spec claims or drained root specs, refreshes closure-target readiness
before Arbiter dispatch, prefers durable root idea-source artifacts before
legacy references and transient inbox paths, blocks Planning with
`missing_root_idea_source` plus `root_idea_source_missing` evidence when
backlog-drain recovery cannot find a valid root source, keeps daemon loops
running after that recoverable block, blocks on queued/active/blocked same-root
lineage work and lineage drift diagnostics with concrete blocking ids, treats
only unblocked open targets as actionable so blocked same-root closure targets
do not globally defer unrelated root specs, emits completion backfill and drift
events, and preserves Arbiter close, remediation incident,
repeated-remediation, and `queue repair-lineage` behavior. Status output
prefers actionable closure targets while still reporting blocked targets and
deferred root counts.
Run inspection depth is implemented for the read-only `runs ls`, `runs show`,
and `runs tail` surfaces. Run listing keeps complete, incomplete, malformed,
token-bearing, closure-target, governance-linked, skill-evidence-bearing, and
runner-artifact-bearing directories visible with stable labels. Run show
surfaces malformed stage-result paths, primary prompt/stdout/stderr/event,
stage-request, stage-result evidence including learning no-op
`result_class: no_op`, and runner invocation/completion thinking-level
evidence, skill-revision evidence, aggregate duration and token usage,
governance ledger links, closure-target metadata, remediation references, raw
runner exit metadata, and runner artifact listings. Run tail
selects report, runner stdout, runner stderr, runner event log, or parsed
stage-result payloads in order and reports missing selected artifacts without
repairing, normalizing, deleting, or archiving inspected runtime files.
The watcher poll-intake path consumes deterministic fallback events after
mailbox drain and before work claims, observes config, task queue, optional spec
queue, and optional `ideas/inbox` targets, debounces repeated writes, handles
missing and deleted paths safely, normalizes new idea markdown into headed spec
documents through `QueueStore`, writes the original markdown to the
runtime-owned `millrace-agents/intake/ideas/<root_idea_id>.md` artifact, lists
that durable source before the transient inbox reference, preserves root
lineage, emits `source_artifact` evidence, skips duplicate idea-derived specs,
and records watcher event/failure/duplicate-skip evidence without corrupting
runtime artifacts. Basic monitor rendering emits
concise key lines for daemon startup, resumed active runs, stage start and
completion, run aggregates, router decisions, status changes, six-hour repeated
no-work idle suppression with activity/reason resets, pause, stop, reload,
watcher, direct/mailbox/deferred operator interventions, and governance
pause/block/degraded/reconciled or resume events; the
CLI fans those lines out to stdout for `--monitor basic` or appends them to a
requested `--monitor-log` path, creating missing parent directories, while
keeping default daemon stdout quiet except final summary lines. The runner
layer provides
the Python-owned `StageRunRequest -> RunnerRawResult -> StageResultEnvelope`
contract, canonical stage prompt rendering with
`runner_prompt.<request_id>.md` persistence, serde-backed
`runner_invocation.<request_id>.json`,
`runner_stdout.<request_id>.txt`, `runner_stderr.<request_id>.txt`,
optional `runner_events.<request_id>.jsonl`, and
`runner_completion.<request_id>.json` artifacts, explicit
process-result and environment-delta models, duplicate-aware registry
registration, dispatcher selection by request runner, caller default, then
`codex_cli`, adapter-specific execution-capability support evaluation,
`RunnerRawResult`, `StageRunnerAdapter`, deterministic in-process
`FakeRunner` support, runner-neutral `thinking_level` and compiled execution
capability grant/support-decision propagation through raw results,
invocation/completion artifacts, and normalized stage results, capability
evidence refs, missing evidence refs, and `failure_capability_class` metadata,
Codex CLI adapter command construction that maps request thinking to legacy
`model_reasoning_effort`, permission precedence and capability-support reporting,
prompt/invocation/stdout/stderr/event/completion artifacts, JSONL token
extraction, timeout/failure evidence, mocked process execution, a real
subprocess executor, Pi RPC adapter command construction that maps request
thinking to `--thinking`, JSONL prompt lifecycle, filtered event-log policy,
final assistant text and session stats queries, timeout abort/terminate/hard-kill
evidence, mocked client/transport coverage, runtime-configured dispatcher
construction for operator once/daemon paths, and normalization into the existing
`StageResultEnvelope` contract.
Runtime startup/config loading exposes `[runners]`, `[runners.codex]`,
`[runners.pi]`, `[usage_governance]`, `[auto_recovery]`,
`[execution_capabilities]`, and
`[stages.<stage>]` settings for
adapter construction and stage overrides, validates malformed runner names,
permissions, runner-neutral thinking levels, Codex legacy reasoning aliases,
environment maps, Pi event-log policies and reserved flags, timeouts, stage
override keys, token-window rules, subscription-quota percent thresholds,
auto-recovery booleans, retry budgets, cooldown arrays, and unknown
auto-recovery keys, execution capability policy decisions, default capability
ids and aliases, and unknown execution-capability config keys,
keeps adapter-only command, permission, environment, and event-log fields out of
compile fingerprints, and builds dispatchers with `codex_cli` and `pi_rpc`
adapters for runtime operator paths. The auto-recovery apply-boundary helper
classifies every Python v0.18.4 `auto_recovery.*` field as `next_tick`.
The execution-capability apply-boundary helper classifies every
`execution_capabilities.*` field as `recompile`; compiler materialization now
seals per-node execution capability grants, warnings, policy fingerprints, and
plan/plane summaries into compiled plans and compile show/graph-export inspection
surfaces. Runner dispatch now carries compiled grants and adapter support
decisions through `StageRunRequest` prompt context, runner
invocation/completion artifacts, raw results, and normalized stage-result
metadata; Codex CLI support reporting is contextual but advisory for broad
`maximum` permission posture, Pi RPC support stays conservative/advisory for
remote boundaries, and missing required capability evidence normalizes as
`capability_evidence_missing`. Runtime capability gates now evaluate compiled
grants in serial and daemon paths before runner invocation or `stage_started`
side effects, write `capability_gate.<request_id>.json`, emit
`capability_gate_evaluated`, and normalize denied, approval-required,
unsupported, or missing-evidence required grants as recoverable runtime-policy
failures. Approval-required grants use durable pending/resolved approval
storage keyed by run/request/grant. The approvals CLI can list, show, approve,
and deny those records; direct decisions resolve approvals when no daemon owns
the workspace, while daemon-owned decisions enqueue and process
`approve_execution_capability`/`deny_execution_capability` mailbox commands at
the runtime-owned boundary.
Daemon idle-cycle recovery now uses that policy to requeue one eligible
retryable stranded blocked predecessor through the audited queue transition,
writes `diagnostics/auto-recovery/` diagnostics and runtime/monitor events,
and returns a recovered tick so queued dependents are not dispatched in the
same cycle.

The contract layer currently covers canonical enum values including planning
`recon` and learning `librarian`, probe work items/status hints, root-intake
kinds, stage metadata, legal terminal and running markers including
`LIBRARIAN_COMPLETE`, `LIBRARIAN_NOOP`, and `LIBRARIAN_RUNNING`,
result-class validation, safe identifier validation, v0.19.0 capability
decision/enforcement/evidence/policy/support enums, and
`approve_execution_capability`/`deny_execution_capability` mailbox command
values. It also includes typed task, probe, spec, incident, and
learning-request work-document contracts with headed markdown parse/render
helpers, root-intake lineage fields, a typed Arbiter closure-target-state
contract, typed Recon packet contracts and markdown helpers, v0.19.0 execution
capability scope, approval policy ref, request, policy override, grant, support
decision, capability id alias/validation, and grant fingerprint contracts, plus
serde-backed
runtime JSON contracts for runtime snapshots, recovery counters, mailbox
command envelopes and add-task/add-spec/add-idea/add-probe payload wrappers,
compile diagnostics, stage-result envelopes, runtime error contexts including
`recon_handoff_invalid` and `stage_work_item_ownership_invalid`, token usage
records, read-only status payloads,
usage-governance state/blockers, usage-governance token ledger entries,
subscription quota telemetry status/window readings, Python-compatible
compiled-stage-graph exports including per-node execution capability
grants/warnings/policy fingerprints, and Python-compatible `run_trace_graph`
contracts. The v0.18.6 mailbox intervention contract slice is implemented with
Python-compatible command values for cancel, archive, supersede, dependency
retarget, resolve/cancel incident, and invalid-incident archive commands; typed
payload contracts validate required reasons, safe ids, supersede cascade
values, optional cancellation fields, dependency retarget fields, and
single-filename invalid incident artifacts; the read-only status payload also
includes typed `latest_operator_intervention` evidence for the new intervention
event family.
The approval mailbox payload contract validates safe approval ids and
non-empty reasons for approve/deny decisions, and the runtime-control/daemon
mailbox paths use those payloads to resolve durable pending approvals.
Always-on tests cover the public exports and Python-produced markdown/JSON
fixtures, including probe documents, add-probe mailbox payloads, and Recon
packet fixtures, without requiring a live daemon. The compiler parity tests also
use a committed Python-normalized fixture so ordinary `cargo test` can compare
compiled-plan structure and key compile CLI output without probing Python; that
fixture now pins the Python `v0.18.0..v0.18.1` source range, including Recon
planning graph and graph-export references used by the Rust compiled-stage-graph
export tests. The
CLI/runtime parity suite now includes a committed Slice 4 CLI evidence matrix,
a committed Slice 5 serial runtime evidence matrix, a committed Slice 6 daemon
runtime evidence matrix, a committed Slice 7 runner adapter evidence matrix,
a committed Slice 8 E2E handoff evidence matrix, and a consolidated Slice 8
advanced parity evidence matrix, plus a Python `millrace-web` dashboard parity
decision fixture that records the optional web package as an Arbiter-visible
unsupported gap rather than a silently omitted Rust surface, plus a
target-facing Python `v0.17.4..v0.18.0` scout fixture plus final Rust `0.3.0`
release evidence for graph/trace docs, version metadata, package include
readiness, and web-gap handling, plus Python `v0.18.0..v0.18.1` guardrail
fixtures and final Rust `0.3.1` release evidence for the Recon/probe auto-port,
plus target-facing Python `v0.18.1..v0.18.2` guardrails and Rust `0.3.2`
release evidence for Integrator, integrated-mode, status JSON, Recon-hardening,
ownership, docs/version, release-check, package dry-run, and web-package
evidence slices, plus target-facing Python `v0.18.2..v0.18.3` guardrails and
final Rust `0.3.3` release evidence for Librarian, Planner-to-Librarian trigger,
learning request artifact metadata, runner normalization metadata, shipped skill
lint, guidance handoff, docs/version, package verification, release-check, and
web-package evidence, plus target-facing Python `v0.18.3..v0.18.4` guardrails
and final Rust `0.3.4` release evidence for blocked-recovery metadata, audited
`queue retry-blocked` behavior, `auto_recovery` config/status evidence, daemon
stranded-dependency recovery gates, docs/version, package verification, release
checks, and `millrace-web` package evidence, plus target-facing Python
`v0.18.4..v0.18.6` guardrails for Rust `0.3.5` operator intervention
mailbox contracts, archive/audit ledgers, daemon/read-only intervention
surfaces, durable idea-source behavior, closure recovery evidence, and
`millrace-web` v0.18.5/v0.18.6 package evidence. The final Rust `0.3.5`
release fixture now reconciles Cargo metadata, runtime docs, source-package
mapping, parity fixture docs, package include readiness, required Builder
checks, package verification, generated-cache exclusions, and Python
`millrace-web` v0.18.5/v0.18.6 package-version unsupported-gap evidence, plus
target-facing Python `v0.18.6..v0.19.0` guardrails for planned Rust `0.4.0`
execution capability contracts/config, compiled grants, approvals, gates,
runner support/evidence metadata, inspection surfaces, required checks, and
`millrace-web` v0.19.0 package evidence. The final Rust `0.4.0`
release fixture now reconciles Cargo metadata, runtime docs, source-package
mapping, parity fixture docs, package include readiness, required Builder
checks, package verification, generated-cache exclusions, run-inspection
capability output, and Python `millrace-web` v0.19.0 package-version
unsupported-gap evidence. The
v0.18.4 runner failure classifier contract, blocked metadata
persistence, manual public retry CLI, auto-recovery config/status, and daemon
stranded-dependency recovery slices are now implemented with typed runtime JSON
metadata contracts, runner normalization coverage, persisted
`millrace-agents/diagnostics/blocked/task-<TASK_ID>.json` diagnostics,
`blocked_item_metadata_written` runtime event evidence, and queue-store
requeue primitive coverage, plus parity coverage for manual retry
audit/event/snapshot behavior and refusal guards, typed `AutoRecoveryConfig`
defaults/validation, next-tick change-boundary classification, daemon-session
config projection, `config show` output for `auto_recovery.enabled`, daemon
idle-cycle recovery diagnostics under `millrace-agents/diagnostics/auto-recovery/`,
`blocked_dependency_auto_requeued` and `blocked_dependency_auto_requeue_skipped`
event/monitor evidence, and same-cycle dependent dispatch suppression.
Docs/version and final release evidence are reconciled in the release fixtures
through Rust `0.4.0`.
The runner normalization/artifact-metadata target is now implemented
with focused runtime JSON, runner normalization, serial runtime, and
daemon runtime coverage, and the shipped skill lint/guidance target is now
implemented with recursive packaged skill lint coverage plus live/baseline
guidance asset synchronization.
The Slice 5
evidence maps
Rust fake-runner startup, tick, routing,
result-application, recovery, closure, and `run once` scenarios back to the
Python runtime tests, the Slice 6 evidence maps daemon startup, bounded loop,
supervisor scheduling, mailbox/reload, watcher intake, monitor rendering,
shutdown, lock contention, and CLI summary scenarios back to Python daemon
modules and tests, and the Slice 7 evidence maps the Rust runner registry,
dispatcher, Codex CLI command/artifact/token/timeout behavior, Pi RPC
lifecycle/event policy/timeout behavior, config validation, and runtime
dispatch scenarios back to the Python runner architecture docs, runner modules,
and runner/runtime/CLI tests. The Slice 8 E2E handoff evidence maps scripted
serial Rust tests for direct task success, repair-loop fix-contract evidence,
malformed and illegal terminal recovery, planning re-entry, Arbiter
completion/remediation, and repeated-remediation blocking back to the Python
handoff tests and runtime sources. The consolidated Slice 8 evidence maps
usage governance, subscription quota telemetry, learning promotion, skill
revision evidence, closure transitions, run inspection depth, and E2E handoff
coverage to the corresponding Python modules and tests while checking that
referenced Rust tests are known, well-formed, present in source, and complete
for the fixture. The web-dashboard decision fixture names the Python
workspace-registry, DTO, queue, run, snapshot, baseline, compiled-plan,
Arbiter, usage-governance, event-stream, static-shell, CLI/server, and
package-boundary test surfaces from `packages/millrace-web/`; Rust does not
implement that server/static package in this parity slice because the accepted
Rust boundary remains local read-only CLI inspection over initialized
workspaces. The fixtures
normalize request ids, run ids,
timestamps, absolute paths, process ids, generated command ids, compact run
handles, compiled plan ids, config versions, runner artifact paths, timeout
durations, token usage, and incident ids. The v0.18.0 scout fixture maps all 52
generated Python scout paths to expected Rust implementation, test,
documentation, fixture, reference-evidence, or unsupported-gap targets, covering
compiled graph exports now implemented in the Rust contract/projection slice,
run-trace contracts/runtime persistence now implemented in the Rust trace slice,
graph/trace CLI behavior now implemented by the read-only `millrace compile
graph` and `millrace runs trace` commands, operator docs, web graph/trace gap
evidence, active guardrail tests, no-live guarantees, and the docs/version plus
final release evidence now captured in the v0.18.0 release fixture. The v0.18.1
guardrail fixture maps all 66 generated Python scout paths to expected Rust
implementation, test, documentation, fixture, package-evidence,
reference-evidence, or unsupported-gap targets for probe work documents, Recon
packets/assets, queue, CLI/mailbox, runtime activation/result application,
docs, and `millrace-web` package/version source references. The first
v0.18.1 contract, asset/compiler, workspace queue lifecycle, CLI/mailbox, and
runtime application slices now
implement probe work documents, root-intake fields, Recon packet contracts,
Recon stage metadata, add-probe runtime JSON fixtures, Recon managed assets,
planning graph `probe -> recon`, mode runner bindings, stage-kind registry,
compiler materialization/export parity, compiler parity fixture coverage,
probe and Recon workspace paths, probe queue lifecycle transitions, planning
probe depth/selection, runtime-control `add_probe`, daemon mailbox add-probe
application, active-probe retry/clear-stale handling, top-level and namespaced
probe CLI intake for canonical `.md`/`.json` documents, active-daemon mailbox
routing with Python command name `add_probe`, and read-only queue probe
depth/lifecycle rendering without mutation, plus probe activation into planning
`recon`, Recon `StageRunRequest` metadata, graph-authoritative Recon routing,
Recon packet validation/persistence, generated task/spec handoff enqueueing,
active probe done/blocked movement, mismatch recovery, and spawned-work
run-trace evidence. The Rust `0.3.1` release evidence reconciles
Cargo/lockfile metadata, docs, roadmap/source-package surfaces, package include
readiness, required release checks, the plain `cargo publish --dry-run`
dirty-worktree limitation, allow-dirty dry-run/offline package verification,
and explicit Python `millrace-web` v0.18.1 package/version unsupported-gap
evidence without adding a Rust web implementation. The v0.18.2 guardrail
fixture maps all 57 generated Python scout paths to expected Rust
implementation, test, documentation, fixture, package-evidence, or
unsupported-gap targets and keeps Rust `0.3.1` as the previous baseline while
Rust `0.3.2` is the target. It pins Integrator assets,
`execution.with_integrator`, integrated modes, status JSON diagnostics, Recon
invalid-handoff hardening, graph validation guards, stage/work-item ownership,
release-check commands, package dry-run evidence, `millrace-web` source
references, and no-live guarantees; the Integrator contracts/assets/compiler
graph subset, integrated mode/runtime-routing slice, and status JSON diagnostics
slice have landed, and Recon invalid-handoff hardening plus graph validation
guards now block malformed handoff artifacts with durable
`recon_handoff_invalid` evidence while keeping task/spec promotion
runtime-owned. Stage/work-item ownership checks now validate active runs before
serial or daemon runner dispatch, preserve closure-target Arbiter activation,
record `stage_work_item_ownership_invalid` runtime error evidence, and emit
`runtime_stage_work_item_ownership_invalid` event evidence for stale pairings;
the final Rust `0.3.2` release-parity evidence reconciles Cargo metadata,
runtime docs, source-package mapping, parity fixture docs, package include
readiness, required release-readiness checks, the dirty-worktree publish dry-run
limitation, allow-dirty dry-run/package verification, and Python
`millrace-web` v0.18.2 package/version unsupported-gap evidence. The v0.18.3
guardrail fixture maps all 50 generated Python scout paths to expected Rust
implementation, test, documentation, fixture, package-evidence, or
unsupported-gap targets while keeping Rust `0.3.2` as the previous baseline and
Rust `0.3.3` as the target. It pins Librarian contracts/assets,
learning graph/modes, Planner-to-Librarian learning triggers, learning request
artifact metadata, runner normalization metadata, shipped skill lint and guidance
handoff source references, release-check commands, package dry-run evidence,
`millrace-web` package/version source references, and no-live guarantees; the
Librarian contract metadata slice has landed with learning-request-only
ownership and complete/no-op result metadata, and the Librarian
asset/compiler-mode slice has landed with managed assets, learning graph/loop
and mode bindings, compiler materialization/export coverage, and workspace
baseline synchronization; the runner normalization/artifact-metadata slice has
landed with normalized active work item metadata plus learning-request artifact
and source metadata coverage; the active Librarian lifecycle slice has landed
with Planner-triggered install requests, targeted Librarian dispatch,
complete/no-op done transitions, blocked evidence, and daemon run-trace
coverage; the shipped skill lint/guidance slice has landed with recursive
packaged `SKILL.md` lint coverage, `marathon-qa-audit` section-contract
migration, Curator/Recon/Planner guidance updates, and live/baseline asset sync
coverage; the final Rust `0.3.3` release-parity evidence reconciles Cargo
metadata, runtime docs, source-package mapping, parity fixture docs, package
include readiness, required Builder checks, package verification, and Python
`millrace-web` v0.18.3 package/version unsupported-gap evidence. The v0.18.4
guardrail fixture maps all 28 generated Python scout paths to expected Rust
implementation, test, documentation, fixture, package-evidence, or
unsupported-gap targets while keeping Rust `0.3.3` as the previous baseline and
Rust `0.3.4` as the target. It pins runner failure classifier metadata, blocked
metadata diagnostics, audited blocked-task retry behavior, `auto_recovery`
config/status defaults and change boundaries, daemon idle-cycle recovery
evidence, release-check commands, package evidence, and `millrace-web`
package/version source references; the final Rust `0.3.4` release-parity
evidence now reconciles Cargo metadata, runtime docs, source-package mapping,
parity fixture docs, package include readiness, required Builder checks,
package verification, generated-cache exclusions, and Python `millrace-web`
v0.18.4 package/version unsupported-gap evidence. The v0.18.6 guardrail fixture
maps all 35 generated Python scout paths to expected Rust implementation, test,
documentation, fixture, package-evidence, reference-evidence, or unsupported-gap
targets while keeping Rust `0.3.4` as the previous baseline and Rust `0.3.5` as
the planned target. It pins the v0.18.5 intermediate release, Python v0.18.6 tag
object and peeled commit, operator intervention mailbox command and payload
surfaces, intervention archive/audit/event/status evidence, direct and
daemon-routed runtime-control behavior, durable watcher idea-source behavior,
closure source preference and missing-source recovery, required release checks,
repository-relative Rust target existence guardrails, no-live guarantees, and
Python `millrace-web` v0.18.5/v0.18.6 package/version unsupported-gap evidence.
The final Rust `0.3.5` release-parity evidence reconciles Cargo metadata,
runtime docs, source-package mapping, parity fixture docs, package include
readiness, required Builder checks, package verification, generated-cache
exclusions, and Python `millrace-web` v0.18.5/v0.18.6 package/version
unsupported-gap evidence. The v0.19.0 guardrail fixture maps all 61 generated
Python scout paths to expected Rust implementation, test, documentation,
fixture, package-evidence, reference-evidence, unsupported-gap, or planned-new
targets while keeping Rust `0.3.5` as the previous/current baseline and Rust
`0.4.0` as the planned target. It pins Python v0.18.6/v0.19.0 annotated tag
objects and peeled commits, execution capability contracts/config, compiled
capability grants, approval storage and CLI/runtime-control routing,
pre-dispatch capability gates, runner support/evidence metadata, inspection
surfaces, required release checks, repository-relative Rust target guardrails,
no-live guarantees, and Python `millrace-web` v0.19.0 package/version
unsupported-gap evidence.
The v0.19.0 capability contracts/config slice has landed with public
Rust contract exports, capability id aliases and scope validation,
approval-required grant invariants, stable grant fingerprints, approval mailbox
payload validation, `[execution_capabilities]` config defaults and recompile
boundaries, `config show` output for the three exposed keys, and focused
contract/runtime JSON/public export/parity tests. The compiled capability
grants slice has also landed: mode, graph-node, and stage-kind capability
declarations compile into sealed per-node grants, warnings, summaries, and
policy fingerprints; disabled capability policy produces zero grants; strict
required-advisory policy fails on advisory required grants such as
`workspace.read`; `millrace compile show` and compiled-stage-graph exports
surface the compiled grant evidence; and focused compiler/parity tests cover
the behavior. The runner support/evidence slice has also landed: stage request
context renders compiled grants and support decisions, fake/Codex/Pi runners
and the dispatcher report adapter-specific support before artifact generation,
runner invocation/completion/raw-result/stage-result metadata carry grant,
support, evidence-ref, missing-evidence, and capability-failure fields, and
focused runner/runtime tests cover conservative support reporting plus
`capability_evidence_missing` normalization. The runtime capability
gates/approval-storage slice has also landed: serial once-mode and daemon
dispatch evaluate compiled grants before runner invocation, persist
`capability_gate.<request_id>.json`, emit `capability_gate_evaluated`, block
denied, unsupported, unresolved approval-required, or missing-evidence required
grants as recoverable runtime-policy failures, and reuse durable pending or
resolved approval records by run/request/grant. The approval CLI/runtime-control
slice has also landed: `millrace approvals ls/show/approve/deny` lists and
inspects durable approval records, resolves approve/deny decisions directly
when no daemon owns the workspace, routes daemon-owned decisions through
mailbox envelopes, and covers daemon application/archive/event behavior with
focused CLI, runtime-control, serial, daemon, and runtime JSON tests.
Focused
`run once`
coverage exercises one-stage mocked Codex dispatcher execution, idle/pause/stop
outcomes, startup failures, lock contention, and run-artifact inspection, and
focused daemon startup, supervisor, loop, mailbox/reload, and watcher
poll-intake coverage exercises config defaults, daemon projection, lock
contention, watcher-session preparation, no startup-time work claiming, default
serial dispatch,
learning-plus-foreground concurrency, foreground mutual exclusion,
completed-before-new-claim ordering, metadata mismatch refusal, max-tick
execution, configured idle sleep, no-work idle exit, pause draining, stop
reset, lock release, deterministic mailbox drain, invalid/failed artifact
preservation, retry/clear-stale handling, reload deferral/application/failure,
startup idea normalization before claims, config/task/spec queue observation,
debounce suppression, missing/deleted path safety, bad idea failure
persistence, duplicate idea protection, quiet default daemon stdout, basic
stdout monitor output, six-hour idle throttle/reset coverage, nested
monitor-log fanout, daemon summary tick key lines, shared runner
prompt/artifact/process/registry/dispatcher behavior,
runtime runner config loading/validation, stage-request and runner-artifact
thinking-level propagation, `config show` runner rendering, runtime-configured
once/daemon dispatcher selection, unknown-runner recovery, mocked Codex CLI
runtime dispatch, Pi RPC mocked-client/transport adapter behavior,
usage-governance inert defaults, ledger reconciliation, token/quota
rule evaluation, governance dispatch pause/auto-resume, manual resume refusal
under active governance blockers, quota degraded fail-open/fail-closed behavior,
governance monitor lines, daemon completion-drain ordering before new claims,
configured status/config rendering without mutation, public runner exports,
learning trigger enqueueing, Curator promotion deferral/application,
rejected/blocked Curator decision evidence without source mutation,
operator-controlled source-promotion audit fields, claim-created and backfilled
closure targets, actionable closure-target selection, unrelated-root activation
while same-root closure work is blocked, queued/active/blocked lineage
suppression, closure-lineage drift diagnostics, Arbiter close/remediation and
repeated-remediation behavior, advanced read-only run
inspection for malformed stage results, runner artifacts, stage-request and
runner-artifact thinking evidence, governance ledger links, closure metadata,
closure-target actionability/deferred root status, skill evidence, tail
fallbacks, `runs trace` text/JSON/output/fallback coverage, and no-mutation
guarantees, duplicate task lifecycle doctor output
and same-root blocked-predecessor retirement, advanced E2E handoff queue/status,
runtime-error, Consultant handoff incident, planning re-entry, closure, and
remediation transitions, stale/malformed Slice 7 fixture detection, Slice 8
fixture area/source/axis/stale-or-unknown-test checks, and the no-live gate
assertions in `cargo test --test runners_live_smoke`. Live
Codex/Pi smoke runs stay opt-in: use
`MILLRACE_REAL_CODEX_SMOKE=1 cargo test --test runners_live_smoke codex_real_adapter_live_smoke -- --ignored --nocapture`
or
`MILLRACE_REAL_PI_SMOKE=1 cargo test --test runners_live_smoke pi_real_adapter_live_smoke -- --ignored --nocapture`
only when the operator has supplied the external binary, credentials or
subscription, and network access. Advanced E2E handoff parity is covered through
scripted serial runtime tests for direct task success, repair-loop
fix-contract evidence, malformed and illegal terminal recovery through
Consultant handoff incidents, planning incident re-entry, Arbiter completion,
Arbiter remediation, and repeated-remediation blocking. Consolidated Slice 8
parity evidence and docs are complete for the fixture-backed advanced surfaces,
and `tests/fixtures/cli_parity/auto_port_v0_17_3_release_parity_evidence.json`
records the historical Rust `0.2.0` auto-port evidence for version metadata,
managed assets, docs, package include rules, release-readiness commands, and
the Python `v0.16.1..v0.17.3` source/test references;
`tests/fixtures/cli_parity/auto_port_v0_17_4_parity_evidence.json` records the
targeted Python `v0.17.3..v0.17.4` parity evidence for learning no-op
contracts, trigger destination safety, compiler/runtime fixture coverage,
learning no-op lifecycle behavior, read-only run inspection of
`result_class: no_op`, source references, no-live guarantees, and Rust
test-reference guardrails; and
`tests/fixtures/runtime_json/stage_result_learning_noop.json` pins the Python
v0.17.4 `ANALYST_NOOP` stage-result shape for learning-request work items;
`tests/fixtures/cli_parity/auto_port_v0_17_4_release_parity_evidence.json`
records the final Rust `0.2.1` release evidence for version metadata,
package include rules, docs/runtime docs, release-readiness commands, and the
Python `v0.17.4` `millrace-web` version/dependency sync gap; and
`tests/fixtures/cli_parity/auto_port_v0_18_0_release_parity_evidence.json`
records the final Rust `0.3.0` release evidence for compiled graph exports,
run traces, graph/trace CLI commands, docs/runtime docs, version metadata,
package include readiness, and the Python `v0.18.0` `millrace-web`
graph/trace unsupported gap; and
`tests/fixtures/cli_parity/auto_port_v0_18_1_parity_evidence.json` records the
target-facing Rust `0.3.1` guardrails for Python `v0.18.0..v0.18.1` Recon/probe
source references, all generated scout paths, required release checks, and
no-live guarantees; and
`tests/fixtures/cli_parity/auto_port_v0_18_1_release_parity_evidence.json`
records the final Rust `0.3.1` release evidence for probe/Recon docs, version
metadata, package include readiness, required release checks, and the Python
`v0.18.1` `millrace-web` package/version unsupported gap; and
`tests/fixtures/cli_parity/auto_port_v0_18_2_parity_evidence.json` records the
target-facing Rust `0.3.2` guardrails for Python `v0.18.1..v0.18.2`
Integrator assets, integrated modes, status JSON diagnostics, completed Recon
hardening and graph validation guards, ownership checks, generated scout path
mappings, release checks, package dry-run evidence, web-package evidence, and
no-live guarantees; and
`tests/fixtures/cli_parity/auto_port_v0_18_2_release_parity_evidence.json`
records the final Rust `0.3.2` release-parity evidence for version metadata,
generated-scout path mappings, package include readiness, runtime docs,
source-package mapping, required release-readiness command results, the
dirty-worktree publish dry-run limitation, allow-dirty dry-run/package
verification, and the Python `v0.18.2` `millrace-web` package/version
unsupported gap; and
`tests/fixtures/cli_parity/auto_port_v0_18_3_parity_evidence.json` records the
target-facing Rust `0.3.3` guardrails for Python `v0.18.2..v0.18.3` Librarian
contracts/assets/graph/modes, Planner-to-Librarian trigger metadata, learning
request artifact metadata, runner normalization metadata, shipped skill lint
and guidance handoff source references, all 50 generated scout paths, required
release checks, package dry-run evidence, `millrace-web` package/version
unsupported-gap evidence, and no-live guarantees; and
`tests/fixtures/cli_parity/auto_port_v0_18_3_release_parity_evidence.json`
records the final Rust `0.3.3` release-parity evidence for version metadata,
generated-scout path mappings, package include readiness, runtime docs,
source-package mapping, required Builder verification command results, package
verification, and the Python `v0.18.3` `millrace-web` package/version
unsupported gap; and
`tests/fixtures/cli_parity/auto_port_v0_18_4_parity_evidence.json` records the
target-facing Rust `0.3.4` guardrails for Python `v0.18.3..v0.18.4` runner
failure classifier metadata, blocked metadata diagnostics, audited
`queue retry-blocked` behavior, `auto_recovery` config/status defaults and
change boundaries, daemon idle-cycle recovery evidence, all 28 generated scout
paths, required checks, `millrace-web` package/version unsupported-gap evidence,
and no-live guarantees; and
`tests/fixtures/cli_parity/auto_port_v0_18_4_release_parity_evidence.json`
records the final Rust `0.3.4` release-parity evidence for version metadata,
generated-scout path mappings, package include readiness, runtime docs,
source-package mapping, required Builder verification command results, package
verification, generated-cache exclusion evidence, and the Python `v0.18.4`
`millrace-web` package/version unsupported gap; and
`tests/fixtures/cli_parity/auto_port_v0_18_6_parity_evidence.json` records the
target-facing Rust `0.3.5` guardrails for Python `v0.18.4..v0.18.6` operator
intervention mailbox payloads, archive/audit/read-only surfaces, durable
idea-source and closure recovery behavior, all 35 generated scout paths,
required checks, `millrace-web` v0.18.5/v0.18.6 package evidence, and no-live
guarantees; and
`tests/fixtures/cli_parity/auto_port_v0_18_6_release_parity_evidence.json`
records the final Rust `0.3.5` release-parity evidence for version metadata,
generated-scout path mappings, package include readiness, runtime docs,
source-package mapping, required Builder verification command results, package
verification, generated-cache exclusion evidence, and the Python `v0.18.5` and
`v0.18.6` `millrace-web` package/version unsupported gap; and
`tests/fixtures/cli_parity/auto_port_v0_19_0_parity_evidence.json` records the
target-facing Rust `0.4.0` guardrails for Python `v0.18.6..v0.19.0` execution
capability contracts/config, compiled grants, approvals, pre-dispatch gates,
runner support/evidence metadata, inspection surfaces, all 61 generated scout
paths, required checks, `millrace-web` v0.19.0 package evidence, planned-new
Rust targets, and no-live guarantees. The optional
Python `millrace-web` dashboard
remains an explicit unsupported Rust parity gap with source references,
shadow-CLI graph/trace commands, and non-goal wording; native filesystem
watcher integration and live subscription quota integration remain
preview-only/deferred work. The runner adapter docs
do not claim broader compiled-plan, queue-state, or stage-machine changes
beyond the already implemented runtime dispatch boundary.

The compiler layer currently covers serde-backed mode definitions including
`stage_thinking_bindings`, Python-compatible execution capability request and
policy fields plus Rust stage-scoped compatibility fields, graph loop
definitions including node-level `thinking_level`, execution capability
request/policy fields, and `no_op` terminal classes, stage-kind registry entries,
learning triggers with `target_skill_id` and normalized
`preferred_output_paths`, plane concurrency policy definitions, compiled graph
and compiled run plan shapes, compiled-stage-graph export contracts and
projection helpers, resolved asset references, compile outcome data, persisted
compiled-plan authority, compiled execution capability summary/grant/warning
fields, and compiled-plan currentness data. It also
resolves authoritative
compile assets from initialized workspace `modes/`,
`graphs/`, `registry/stage_kinds/`, `entrypoints/`, and `skills/` paths,
canonicalizes `standard_plain` to `default_codex`, fingerprints compile-relevant
config and resolved asset content while excluding adapter-only runner settings,
accepts `stages.<stage>.thinking_level` while preserving legacy
`model_reasoning_effort` as a matching Codex alias, and ignores compatibility
`loops/` plus unreferenced assets. It now materializes deterministic frozen
compiled run plans for default Codex, Pi, learning, and the `standard_plain`
alias mode. The plans include graph node bindings, transitions, policies,
planning completion
behavior, learning triggers, learning no-op terminal states, direct Curator
trigger safe-destination validation, sealed execution capability grants,
warnings, policy fingerprints, and plan/plane summaries, and supported config,
skill, entrypoint, runner, model, thinking-level, Codex legacy
reasoning-effort, and timeout overrides. It persists compiler-authoritative
`compiled_plan.json` and `compile_diagnostics.json`, reports
missing/current/stale/unknown currentness from compile-input fingerprints,
preserves last-known-good plans on compile failure, and refuses stale
last-known-good plans when compile inputs drift and recompilation fails. The
`millrace compile validate`, `millrace compile show`, and `millrace compile
graph` commands require an initialized workspace, accept the built-in
Codex/Pi/learning modes and `standard_plain` alias, persist compiler artifacts,
and render diagnostics, inspectable compiled-plan fields, or stable
compiled-stage-graph text/JSON output including selected-plane and output-file
behavior plus compact execution capability summary/grant/warning lines without
invoking runtime execution behavior. The committed compiler
parity fixture is pinned to the Python `v0.18.0..v0.18.1` source range and
covers `default_codex`, `default_pi`, `learning_codex`, `learning_pi`, and
`standard_plain`, including learning no-op terminal classes,
success-to-Analyst trigger behavior, and Python graph-export source
references plus Recon managed asset, planning graph `probe -> recon`, mode
binding, stage-kind registry, materialization, and graph-export parity. The
v0.18.1 compiler scout fixture remains alongside the normalized fixture as
target-facing source evidence, and the v0.18.2 compiler scout fixture records
Integrator entrypoint/skill/registry assets, Checker asset updates,
`execution.with_integrator`, integrated Codex modes, package baseline targets,
compiler targets, and fixture/test targets. The v0.18.3 compiler scout fixture
records Librarian entrypoint/skill/stage-kind assets, learning graph/loop/mode
targets, shipped skill lint/guidance source references, package baseline
targets, compiler targets, and fixture/test targets. The Librarian
asset/compiler graph/mode slice now implements those assets and compiler
surfaces with focused compiler, parity, materialization/export, and
workspace-baseline coverage, and the runner normalization/artifact-metadata
slice now implements source metadata preservation, while the active Librarian
lifecycle slice now implements Planner-triggered Librarian requests, targeted
dispatch, complete/no-op/blocked result application, and daemon trace coverage;
the shipped skill lint/guidance slice now implements recursive skill lint and
guidance handoff coverage, with docs and release evidence reconciled for Rust
`0.3.3`. Focused contract, asset,
materialization/export, and workspace-baseline tests now cover the implemented
Integrator contracts/assets/compiler graph subset, and focused compiler, CLI,
serial runtime, daemon runtime, and baseline tests cover the opt-in integrated
Codex mode runtime-routing slice. The same contract and materialization
coverage still covers the `compiled_stage_graph` JSON contract, stable
selected-plane export ordering, learning-plane inclusion, Recon/probe planning
topology, source refs, skills, allowed result-class mappings, and missing-plane
errors.

The workspace layer currently covers canonical `<workspace>/millrace-agents/`
path resolution and idempotent initialization defaults for the directory tree,
status files, runtime snapshot, recovery counters, learning event log, runtime
config, outline, history log, managed asset deployment, baseline manifest IO,
probe lifecycle directories under `probes/{queue,active,done,blocked}`, and
Recon artifact directories under `recon/{packets,reports}` plus managed baseline
upgrade preview/apply helpers. It now also includes filesystem queue stores for
canonical task, probe, spec, incident, and learning-request headed markdown
documents, plus state stores for runtime snapshot, recovery
counter, status-file, usage-governance state, and usage-governance ledger
persistence. Task lifecycle integrity helpers detect duplicate task ids across
`tasks/queue`, `tasks/active`, `tasks/done`, and `tasks/blocked` using parsed
`Task-ID` values with filename fallback for unparseable artifacts, and task
completion retires same-root blocked predecessors under
`tasks/blocked/superseded/` with machine-readable `retirements.jsonl` audit
evidence. Offline
runtime ownership lock helpers can inspect, acquire, release, force-release, and
clear stale or invalid `runtime_daemon.lock.json` files without starting a
daemon. `RuntimeControl` uses those lock, state, and queue boundaries to apply
offline pause, resume, stop, retry-active, planning retry-active,
clear-stale-state, reload-config, and task/spec/idea/probe intake directly when
no active daemon owns the workspace, or to enqueue Python-compatible mailbox
command envelopes when an active daemon lock owns it; probe intake refreshes
planning depth and retry/clear-stale flows requeue active probes. The Rust
runtime-control intervention helpers now validate the v0.18.6
cancel/archive/supersede/retarget/incident payload family, apply direct offline
archive/audit mutations through the shared queue-store boundary with snapshot
queue-depth refresh, and mailbox-route the same commands when an active daemon
owns the workspace. Runtime-control approval helpers validate v0.19.0
approve/deny payloads, resolve durable pending approvals directly when offline,
and mailbox-route the same decisions when an active daemon owns the workspace.
The Rust
`millrace init --workspace <path>` command routes through the workspace
initialization helper, and first workspace doctor checks validate the
initialized layout, status/state parseability, baseline manifest and managed
assets, queue artifacts, duplicate task lifecycle state with workspace-relative
paths, and runtime ownership lock health. Read-only operator
CLI commands inspect queue, status, run, mode, and config artifacts without
creating or mutating workspaces. Queue intake CLI commands import task/probe/spec
markdown or JSON through typed work-document APIs, stage idea markdown, and use
`RuntimeControl` for direct offline writes or active-daemon mailbox routing.
Upgrade CLI commands compare workspace managed assets against the embedded
package baseline, apply only safe changes, and localize removed managed assets
without deleting operator content. `run once` now validates the initialized
workspace and supported run options, starts the once-mode runtime session, and
dispatches one runtime-configured serial tick; `run daemon` validates
initialized workspaces and supported daemon options, starts the daemon runtime
session, executes the runtime-configured daemon loop, renders final summary tick
lines, and supports basic stdout/log monitor sinks with log parent-directory
creation. The `queue repair-lineage` CLI now uses the file-backed
closure-lineage repair boundary to load Arbiter closure targets, scan
task/spec/incident queue/active/blocked surfaces, write preview or applied
repair reports, refuse apply while an active daemon lock or active runtime
stage is present, refresh queue-depth snapshot fields, and append the
`closure_lineage_repaired` event. The runtime library now has once-mode
startup and serial tick activation boundaries for config loading, ownership
locking, compile-plan authority, snapshot/counter loading, mailbox intake,
durable idea-source intake artifact handling, queue-depth refresh including
planning probes, no-work/paused/stopped/blocked outcomes, compiled-plan claim
activation including the planning `probe` entry, closure-target request
activation, stage/work-item ownership validation before stage request
construction, running markers, active-run projection, runtime events, and stale
active-state reconciliation. It also dispatches a ready stage
through the runner boundary, persists request/raw-result/stage-result/terminal-marker/router
decision evidence, routes through compiled graph policies, writes recoverable
runtime-error context/report evidence, applies routed results through typed
queue and state-store helpers, updates active-run state and final snapshots,
mutates recovery counters, enqueues typed handoff incidents, schedules
post-stage application-failure recovery, updates last terminal/result status,
applies closure-target Arbiter close/remediation outcomes, and emits stage,
router, closure, handoff, and recovery events. Recon handoff application
validates generated task/spec ids before import and converts invalid handoff
artifacts into `recon_handoff_invalid` error context/report evidence while
blocking the active probe, setting planning status to `### BLOCKED`, clearing
active runtime state, and avoiding ordinary Mechanic/Manager recovery. It also
has daemon-named startup entrypoints, daemon-aware config loading for run-style,
watcher, runner, stage, governance token/quota rule inputs, and the typed
`auto_recovery` policy, `RuntimeMode::Daemon` snapshot projection,
deterministic poll watcher-session preparation/rebuild hooks,
matching-session daemon lock release on startup failure or close, a daemon
supervisor/completion boundary for compiled-plan plane-concurrency, runner
adapter worker dispatch, typed completion capture, serialized owner-side application,
and bounded daemon loop/shutdown control for completed tick counting, idle
sleep, max-tick/no-work/stop/process-stopped/blocked exits, completion draining,
stopped-state reset, and matching-session lock release. It also has daemon
mailbox/reload handling for deterministic command drain, processed/failed
archives, `add_probe` application, idle v0.18.6 operator-intervention
application with active-stage deferral, v0.19.0 approval-decision application
with processed/failed archive and runtime/monitor event evidence, retry-active
and clear-stale-state, reload deferral/application,
watcher-session rebuild, retained-plan diagnostics, and reload failure
evidence, plus deterministic watcher poll intake for config, task queue,
optional spec queue, and optional `ideas/inbox` changes before work claims,
with durable source-copy persistence and durable-first spec references for
idea inbox intake.
The daemon monitor and CLI execution path now run against the runtime-configured
runner dispatcher with real Codex/Pi adapter registration, and the Slice 7
runner adapter parity evidence is committed and covered by always-on fixture
assertions; usage-governance dispatch enforcement, governance-owned pause
mutation, auto-resume, monitor evidence, and runtime-owned learning promotion
and skill evidence are implemented; closure-lineage runtime readiness, backfill,
drift, blocking-id, actionable-target selection, unrelated-root activation,
Arbiter close/remediation, status actionability, and repair-lineage regression
coverage is implemented; read-only run inspection depth for malformed,
incomplete, runner-artifact, governance-linked, closure-target, and
skill-evidence-bearing runs is implemented; advanced E2E handoff parity is
implemented with scripted fake-runner coverage and the committed
`slice8_e2e_handoff_parity_evidence.json` fixture; consolidated Slice 8
advanced parity evidence is committed in
`slice8_advanced_parity_evidence.json`; native filesystem watcher integration
and live subscription quota integration remain preview-only/deferred.

Do not depend on production runtime behavior from this crate yet. Public APIs
may change while the Rust implementation is brought toward parity with the
Python runtime.

## Documentation

- [Architecture](docs/architecture.md)
- [Technical overview](docs/millrace-technical-overview.md)
- [Testing](docs/testing.md)
- [Runtime docs](docs/runtime/README.md)
- [Release roadmap](ROADMAP.md)
- [Rust port roadmap](docs/rust-port-roadmap.md)
- [Rust source package map](docs/source-package-map.md)
- [Provenance and autonomous-build evidence](docs/provenance.md)
- [Changelog](CHANGELOG.md)

The historical public proof package for the v0.1.0 autonomous port campaign
lives in
[`tim-osterhus/millrace-rs-port-docs`](https://github.com/tim-osterhus/millrace-rs-port-docs).
The crate-local `0.4.0` release evidence lives in `CHANGELOG.md` and
`tests/fixtures/cli_parity/auto_port_v0_19_0_release_parity_evidence.json`.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
