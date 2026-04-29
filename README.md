# Millrace Rust

`millrace-ai` is the experimental Rust implementation of Millrace, a
governed runtime for long-running agent work.

The production implementation is currently the Python package
[`millrace-ai`](https://pypi.org/project/millrace-ai/). The initial Rust
`0.1.x` releases establish the first broad Rust parity surface while
contract-parity, workspace-substrate, compiler, operator-CLI, runtime, daemon,
and runner work progress.

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
`doctor --workspace <path>` output, and Slice 3 compile validate/show output
for initialized workspaces. The first Slice 4 CLI command framework now lives
behind `millrace_ai::cli`; it shares parsing, initialized-workspace checks, and
rendering while recognizing the primary operator command groups and
compatibility aliases. It implements read-only operator inspection commands for
`queue ls/show`, `status`/`status show`/bounded `status watch`, `runs
ls/show/tail`, `modes list/show`, and `config show`, plus queue intake commands
for `queue add-task`, `queue add-spec`, `queue add-idea`, and the top-level
`add-task`/`add-spec`/`add-idea` aliases, plus `queue repair-lineage`
preview/apply wiring over the workspace repair boundary. It also implements
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
materialization, persisted compiled-plan authority, and currentness inspection
boundary through `millrace_ai::compiler`. The first Slice 5 runtime/runner
contract, the first Slice 6 daemon startup/config, supervisor/completion,
bounded loop/shutdown, mailbox/reload, watcher poll-intake, basic monitor
rendering, daemon CLI execution boundaries, and Slice 7 shared runner
prompt/artifact/process/registry/dispatcher plus Codex CLI, Pi RPC, runtime
config, and runtime-dispatch integration boundaries are exposed through
`millrace_ai::runtime` and
`millrace_ai::runners`. They include typed
`StageRunRequest` models, a
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
control, intake, retry, clear-stale-state, and reload commands in deterministic
order, archives processed or failed commands with source/error evidence, defers
reload while active planes exist, applies reload after planes drain with
watcher-session, config-version, and compiled-plan snapshot updates, and
preserves the previous plan on recoverable reload diagnostics. The deterministic
tick activation path drains applicable mailbox commands, refreshes queue depths,
returns typed
no-work/paused/stopped/blocked outcomes, claims at most one
compiled-plan-authorized work item or closure target, builds `StageRunRequest`
payloads, writes running markers, projects active-run snapshot state, and emits
runtime events. Runner dispatch persists stage request, raw runner result,
normalized stage result, terminal marker, and router decision artifacts, routes
through compiled graph policy, persists recoverable runtime error context/report
evidence, updates last-terminal/result snapshot fields, applies router
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
documents with trigger and artifact evidence, stage requests preserve
skill-revision evidence in run directories, Curator `skill_update` artifacts
create auditable learning update-candidate promotion records, deferred records
apply only after foreground execution and planning lanes drain, rejected or
blocked Curator decisions preserve decision evidence without promotion records,
and source-packaged skills remain mutable only through explicit
`millrace skills promote` operator commands with audit fields.
Closure-lineage runtime parity now creates or backfills closure targets from
root-spec claims or drained root specs, refreshes closure-target readiness
before Arbiter dispatch, blocks on queued/active/blocked same-root lineage work
and lineage drift diagnostics with concrete blocking ids, emits completion
backfill and drift events, and preserves Arbiter close, remediation incident,
repeated-remediation, and `queue repair-lineage` behavior.
Run inspection depth is implemented for the read-only `runs ls`, `runs show`,
and `runs tail` surfaces. Run listing keeps complete, incomplete, malformed,
token-bearing, closure-target, governance-linked, skill-evidence-bearing, and
runner-artifact-bearing directories visible with stable labels. Run show
surfaces malformed stage-result paths, primary prompt/stdout/stderr/event,
runner invocation/completion, skill-revision evidence, aggregate duration and
token usage, governance ledger links, closure-target metadata, remediation
references, raw runner exit metadata, and runner artifact listings. Run tail
selects report, runner stdout, runner stderr, runner event log, or parsed
stage-result payloads in order and reports missing selected artifacts without
repairing, normalizing, deleting, or archiving inspected runtime files.
The watcher poll-intake path consumes deterministic fallback events after
mailbox drain and before work claims, observes config, task queue, optional spec
queue, and optional `ideas/inbox` targets, debounces repeated writes, handles
missing and deleted paths safely, normalizes new idea markdown into headed spec
documents through `QueueStore`, preserves root lineage and references, skips
duplicate idea-derived specs, and records watcher event/failure/duplicate-skip
evidence without corrupting runtime artifacts. Basic monitor rendering emits
concise key lines for daemon startup, resumed active runs, stage start and
completion, run aggregates, router decisions, status changes, idle suppression,
pause, stop, reload, watcher, and governance pause/block/degraded/reconciled
or resume events; the CLI fans those lines out to stdout for `--monitor basic`
or appends them to a requested `--monitor-log` path, creating missing parent
directories, while keeping default daemon stdout quiet except final summary
lines. The runner
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
`codex_cli`, `RunnerRawResult`, `StageRunnerAdapter`, deterministic in-process
`FakeRunner` support, Codex CLI adapter command construction, permission
precedence, prompt/invocation/stdout/stderr/event/completion artifacts, JSONL
token extraction, timeout/failure evidence, mocked process execution, a real
subprocess executor, Pi RPC adapter command construction, JSONL prompt
lifecycle, filtered event-log policy, final assistant text and session stats
queries, timeout abort/terminate/hard-kill evidence, mocked client/transport
coverage, runtime-configured dispatcher construction for operator once/daemon
paths, and normalization into the existing `StageResultEnvelope` contract.
Runtime startup/config loading exposes `[runners]`, `[runners.codex]`,
`[runners.pi]`, `[usage_governance]`, and `[stages.<stage>]` settings for
adapter construction, validates malformed runner names, permissions, reasoning,
environment maps, Pi event-log policies and reserved flags, timeouts, stage
override keys, token-window rules, and subscription-quota percent thresholds,
keeps adapter-only command, permission, environment, and event-log fields out of
compile fingerprints, and builds dispatchers with `codex_cli` and `pi_rpc`
adapters for runtime operator paths.

The contract layer currently covers canonical enum values, stage metadata,
legal terminal and running markers, result-class validation, and safe
identifier validation. It also includes typed task, spec, incident, and
learning-request work-document contracts with headed markdown parse/render
helpers, a typed Arbiter closure-target-state contract, plus serde-backed
runtime JSON contracts for runtime snapshots, recovery counters, mailbox
command envelopes and add-task/add-spec/add-idea payload wrappers, compile
diagnostics, stage-result envelopes, runtime error contexts, token usage
records, usage-governance state/blockers, usage-governance token ledger entries,
and subscription quota telemetry status/window readings.
Always-on tests cover the public exports and Python-produced markdown/JSON
fixtures without requiring a live daemon. The compiler parity tests also use a
committed Python-normalized fixture so ordinary `cargo test` can compare
compiled-plan structure and key compile CLI output without probing Python. The
CLI/runtime parity suite now includes a committed Slice 4 CLI evidence matrix,
a committed Slice 5 serial runtime evidence matrix, a committed Slice 6 daemon
runtime evidence matrix, a committed Slice 7 runner adapter evidence matrix,
a committed Slice 8 E2E handoff evidence matrix, and a consolidated Slice 8
advanced parity evidence matrix. The Slice 5 evidence maps
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
for the fixture. The fixtures normalize request ids, run ids,
timestamps, absolute paths, process ids, generated command ids, compact run
handles, compiled plan ids, config versions, runner artifact paths, timeout
durations, token usage, and incident ids. Focused `run once`
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
stdout monitor output, nested monitor-log fanout, daemon summary tick key
lines, shared runner prompt/artifact/process/registry/dispatcher behavior,
runtime runner config loading/validation, `config show` runner rendering,
runtime-configured once/daemon dispatcher selection, unknown-runner recovery,
mocked Codex CLI runtime dispatch, Pi RPC mocked-client/transport adapter
behavior, usage-governance inert defaults, ledger reconciliation, token/quota
rule evaluation, governance dispatch pause/auto-resume, manual resume refusal
under active governance blockers, quota degraded fail-open/fail-closed behavior,
governance monitor lines, daemon completion-drain ordering before new claims,
configured status/config rendering without mutation, public runner exports,
learning trigger enqueueing, Curator promotion deferral/application,
rejected/blocked Curator decision evidence without source mutation,
operator-controlled source-promotion audit fields, claim-created and backfilled
closure targets, queued/active/blocked lineage suppression, closure-lineage
drift diagnostics, Arbiter close/remediation/repeated-remediation behavior,
advanced read-only run inspection for malformed stage results, runner
artifacts, governance ledger links, closure metadata, skill evidence, tail
fallbacks, and no-mutation guarantees, advanced E2E handoff queue/status,
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
parity evidence and docs are complete for the fixture-backed advanced surfaces;
native filesystem watcher integration and live subscription quota integration
remain preview-only/deferred work. The runner adapter docs
do not claim broader compiled-plan, queue-state, or stage-machine changes
beyond the already implemented runtime dispatch boundary.

The compiler layer currently covers serde-backed mode definitions, graph loop
definitions, stage-kind registry entries, learning triggers, plane concurrency
policy definitions, compiled graph and compiled run plan shapes, resolved asset
references, compile outcome data, persisted compiled-plan authority, and
compiled-plan currentness data. It also resolves authoritative compile assets
from initialized workspace `modes/`,
`graphs/`, `registry/stage_kinds/`, `entrypoints/`, and `skills/` paths,
canonicalizes `standard_plain` to `default_codex`, fingerprints compile-relevant
config and resolved asset content while excluding adapter-only runner settings,
and ignores compatibility `loops/` plus unreferenced assets. It now materializes
deterministic frozen compiled run
plans for default Codex, Pi, learning, and `standard_plain` alias modes,
including graph node bindings, transitions, policies, planning completion
behavior, learning triggers, and supported config, skill, entrypoint, runner,
model, reasoning, and timeout overrides. It persists compiler-authoritative
`compiled_plan.json` and `compile_diagnostics.json`, reports
missing/current/stale/unknown currentness from compile-input fingerprints,
preserves last-known-good plans on compile failure, and refuses stale
last-known-good plans when compile inputs drift and recompilation fails. The
`millrace compile validate` and `millrace compile show` commands require an
initialized workspace, accept the built-in Codex/Pi/learning modes and
`standard_plain` alias, persist compiler artifacts, and render diagnostics plus
inspectable compiled-plan fields without invoking runtime execution behavior.
The committed compiler parity fixture covers `default_codex`,
`default_pi`, `learning_codex`, `learning_pi`, and `standard_plain`.

The workspace layer currently covers canonical `<workspace>/millrace-agents/`
path resolution and idempotent initialization defaults for the directory tree,
status files, runtime snapshot, recovery counters, learning event log, runtime
config, outline, history log, managed asset deployment, and baseline manifest
IO plus managed baseline upgrade preview/apply helpers. It now also includes
filesystem queue stores for canonical task, spec, incident, and learning-request
headed markdown documents, plus state stores for runtime snapshot, recovery
counter, status-file, usage-governance state, and usage-governance ledger
persistence. Offline
runtime ownership lock helpers can inspect, acquire, release, force-release, and
clear stale or invalid `runtime_daemon.lock.json` files without starting a
daemon. `RuntimeControl` uses those lock, state, and queue boundaries to apply
offline pause, resume, stop, retry-active, planning retry-active,
clear-stale-state, reload-config, and task/spec/idea intake directly when no
active daemon owns the workspace, or to enqueue Python-compatible mailbox
command envelopes when an active daemon lock owns it. The Rust
`millrace init --workspace <path>` command routes through the workspace
initialization helper, and first workspace doctor checks validate the
initialized layout, status/state parseability, baseline manifest and managed
assets, queue artifacts, and runtime ownership lock health. Read-only operator
CLI commands inspect queue, status, run, mode, and config artifacts without
creating or mutating workspaces. Queue intake CLI commands import task/spec
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
queue-depth refresh, no-work/paused/stopped/blocked outcomes, compiled-plan
claim activation, closure-target request activation, stage request
construction, running markers, active-run projection, runtime events, and stale
active-state reconciliation. It also dispatches a ready stage through the
runner boundary, persists request/raw-result/stage-result/terminal-marker/router
decision evidence, routes through compiled graph policies, writes recoverable
runtime-error context/report evidence, applies routed results through typed
queue and state-store helpers, updates active-run state and final snapshots,
mutates recovery counters, enqueues typed handoff incidents, schedules
post-stage application-failure recovery, updates last terminal/result status,
applies closure-target Arbiter close/remediation outcomes, and emits stage,
router, closure, handoff, and recovery events. It also has daemon-named startup
entrypoints, daemon-aware config loading for run-style, watcher, runner, stage,
and governance token/quota rule inputs, `RuntimeMode::Daemon` snapshot projection,
deterministic poll watcher-session preparation/rebuild hooks,
matching-session daemon lock release on startup failure or close, a daemon
supervisor/completion boundary for compiled-plan plane-concurrency, runner
adapter worker dispatch, typed completion capture, serialized owner-side application,
and bounded daemon loop/shutdown control for completed tick counting, idle
sleep, max-tick/no-work/stop/process-stopped/blocked exits, completion draining,
stopped-state reset, and matching-session lock release. It also has daemon
mailbox/reload handling for deterministic command drain, processed/failed
archives, retry-active and clear-stale-state, reload deferral/application,
watcher-session rebuild, retained-plan diagnostics, and reload failure
evidence, plus deterministic watcher poll intake for config, task queue,
optional spec queue, and optional `ideas/inbox` changes before work claims.
The daemon monitor and CLI execution path now run against the runtime-configured
runner dispatcher with real Codex/Pi adapter registration, and the Slice 7
runner adapter parity evidence is committed and covered by always-on fixture
assertions; usage-governance dispatch enforcement, governance-owned pause
mutation, auto-resume, monitor evidence, and runtime-owned learning promotion
and skill evidence are implemented; closure-lineage runtime readiness, backfill,
drift, blocking-id, Arbiter close/remediation, and repair-lineage regression
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

## Rust Port Roadmap

The behavioral parity plan lives in [docs/rust-port-roadmap.md](docs/rust-port-roadmap.md).

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
