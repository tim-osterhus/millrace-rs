# CLI Parity Evidence

`slice4_cli_parity_evidence.json` records the normalized Slice 4 CLI evidence
surface used by `tests/parity_cli.rs`. `slice5_serial_runtime_parity_evidence.json`
records the normalized Slice 5 serial fake-runner runtime evidence, including
the Python runtime tests each Rust scenario is sourced from.
`slice6_daemon_runtime_parity_evidence.json` records the normalized Slice 6
fake-runner daemon runtime evidence for startup, bounded loop, supervisor
scheduling, mailbox/reload, watcher intake, monitor rendering, shutdown, lock
contention, and CLI summary scenarios. `slice7_runner_adapter_parity_evidence.json`
records the normalized Slice 7 real-runner adapter evidence for the preserved
Python `StageRunRequest -> RunnerRawResult -> StageResultEnvelope` contract,
the Python-owned artifact filenames (`runner_prompt.<request_id>.md`,
`runner_invocation.<request_id>.json`, `runner_stdout.<request_id>.txt`,
`runner_stderr.<request_id>.txt`, optional `runner_events.<request_id>.jsonl`,
and `runner_completion.<request_id>.json`), registry/dispatcher resolution,
Codex CLI command/artifact/token/timeout behavior, Pi RPC lifecycle/event-log
policy/timeout behavior, runner config validation, and runtime dispatch through
mocked adapters. The same fixture names the live-smoke surface:
`cargo test --test runners_live_smoke` checks the gate/no-op path without
starting external tools, while
`MILLRACE_REAL_CODEX_SMOKE=1 cargo test --test runners_live_smoke codex_real_adapter_live_smoke -- --ignored --nocapture`
and
`MILLRACE_REAL_PI_SMOKE=1 cargo test --test runners_live_smoke pi_real_adapter_live_smoke -- --ignored --nocapture`
run the real Codex CLI and Pi RPC adapter smokes only after the operator has
supplied binaries, credentials or subscriptions, and network access.
`slice8_e2e_handoff_parity_evidence.json` records the normalized advanced
handoff E2E surface for direct task success, checker/fixer/doublechecker repair
loops, malformed and illegal terminal recovery through Consultant handoff
incidents, planning incident re-entry into execution, lineage-drain Arbiter
completion, Arbiter remediation incidents, and repeated-remediation blocking.
`slice8_advanced_parity_evidence.json` records the consolidated Slice 8
advanced parity matrix for usage governance, subscription quota telemetry,
learning promotion, skill revision evidence, closure transitions, run
inspection depth, and E2E handoffs. It also names preserved Python-owned
contracts, preview-only live/native surfaces, and the completed validation
command set; `tests/parity_cli.rs` rejects unknown, malformed, stale, or
missing Rust test references in that fixture. `web_dashboard_parity_decision.json`
records the Python v0.17.3 optional `packages/millrace-web` surface as an
intentional unsupported Rust parity gap, records the Python v0.17.4 package
version/dependency sync, records the Python v0.18.0 graph/trace dashboard
evidence for that same gap, and records the Python v0.18.1, v0.18.2,
v0.18.3, v0.18.4, v0.18.5, and v0.18.6 package/runtime app version sync
without authorizing a Rust web implementation. It names
the workspace registry, summary DTO, queue/run/snapshot/baseline/compiled-plan/
Arbiter/usage-governance/event readers, static shell, CLI/server boundary, and
package-boundary tests, while documenting that Rust currently keeps the
accepted inspection boundary at local read-only CLI commands over initialized
workspaces.

`auto_port_v0_17_3_release_parity_evidence.json` records the final Rust
`0.2.0` auto-port consolidation evidence for Python `v0.16.1..v0.17.3`,
including version-visible CLI metadata, package include rules, managed asset
parity, docs/release notes, release-readiness commands, the web-dashboard gap,
and required Rust test references. `tests/parity_cli.rs` rejects missing,
malformed, unknown, stale, or omitted Rust test references in that final
fixture.
`auto_port_v0_17_4_parity_evidence.json` records the targeted Python
`v0.17.3..v0.17.4` parity evidence for learning no-op contracts, trigger
destination safety, committed compiler/runtime fixtures, learning no-op
lifecycle behavior, and read-only run inspection of `result_class: no_op`.
`tests/parity_cli.rs` rejects a stale target pin, missing source references, and
missing, malformed, unknown, stale, or omitted Rust test references in that
fixture.
`auto_port_v0_17_4_release_parity_evidence.json` records the final Rust
`0.2.1` release evidence for version-visible CLI metadata, package include
rules, README/changelog/roadmap/runtime docs, release-readiness commands, the
Python v0.17.4 source docs, and the `millrace-web` version/dependency sync
unsupported-gap evidence. `tests/parity_cli.rs` rejects missing, malformed,
unknown, stale, or omitted Rust test references in that release fixture.
`auto_port_v0_18_0_parity_evidence.json` records the target-facing scout
evidence for Python `v0.17.4..v0.18.0` and planned Rust `0.3.0`. It maps the
generated scout's changed Python paths to expected Rust implementation, test,
documentation, fixture, or unsupported-gap targets. The graph-export slice now
implements the compiled graph contract/projection targets, and the trace-runtime
slice now implements the run-trace contract, runtime persistence, spawned-work
edge evidence, and fallback-inspection targets. The graph/trace CLI slice now
implements the read-only `millrace compile graph` and `millrace runs trace`
shadow commands over those contracts, including text, JSON, selected-plane,
output-file, missing-plane, missing-run, malformed-trace fallback, and
absent-trace fallback coverage. The same fixture continues to gate the web-gap,
docs/version, and release evidence now represented by the final release and
web-dashboard fixtures.
`auto_port_v0_18_0_release_parity_evidence.json` records the final Rust
`0.3.0` release evidence for version-visible CLI metadata, package include
rules, README/changelog/roadmap/runtime docs, graph/trace CLI operator docs,
release-readiness commands, package-readiness dry-run evidence, and the Python
v0.18.0 `millrace-web` compiled graph/run-trace/Flow overlay unsupported-gap
evidence. `tests/parity_cli.rs` rejects missing, malformed, unknown, stale, or
omitted Rust test references in that release fixture and confirms release
evidence does not require publish, upload, push, tag, or deployment commands.
`auto_port_v0_18_1_parity_evidence.json` records target-facing guardrails for
the Python `v0.18.0..v0.18.1` to Rust `0.3.1` auto-port. It maps every
generated scout changed path to an expected Rust implementation, test,
documentation, fixture, package evidence, or unsupported-gap evidence target;
pins probe work documents, Recon packets/assets, queue, CLI, mailbox, runtime
activation/result-application, docs, and `millrace-web` version/package source
references; and requires the release-check command set before Arbiter can treat
the lineage as complete.
`auto_port_v0_18_1_release_parity_evidence.json` records the final Rust
`0.3.1` release evidence for version-visible CLI metadata, package include
rules, README/changelog/roadmap/runtime docs, Recon/probe package-readiness
evidence, release-readiness command results, the plain publish dry-run
dirty-worktree limitation, allow-dirty dry-run/offline package verification,
and the Python v0.18.1 `millrace-web` package/version unsupported-gap evidence.
`tests/parity_cli.rs` rejects missing, malformed, unknown, stale, or omitted
Rust test references in that release fixture and confirms release evidence does
not require publish, upload, push, tag, or deployment commands.
`auto_port_v0_18_2_parity_evidence.json` records target-facing guardrails for
the Python `v0.18.1..v0.18.2` to Rust `0.3.2` auto-port. It maps every
generated scout changed path to an expected Rust implementation, test,
documentation, fixture, package evidence, or unsupported-gap evidence target;
pins Integrator assets, `execution.with_integrator`, integrated modes, status
JSON diagnostics, Recon invalid-handoff hardening, graph validation guards,
stage/work-item ownership, docs, version, release-check, package dry-run, and
`millrace-web` source references; and keeps Rust `0.3.1` as the previous
baseline while Rust `0.3.2` is the target. The status JSON diagnostics targets are now
implemented and covered by `tests/parity_cli.rs` for text/JSON payload
coherence, blocked-idle and runtime-error context diagnostics, deterministic
format rejections, text-only `status watch` rejection, and no-mutation
guarantees. The Recon hardening targets are now implemented and covered by
focused contract, compiler, serial runtime, daemon runtime, and parity guardrail
tests for emitted-id validation, invalid-handoff runtime error evidence,
active-probe blocking, and direct-stage graph edge rejection. The ownership
targets are now implemented and covered by focused contract and runtime tests
for the stage/work-item matrix, request validation, serial and daemon
pre-runner guards, stale-pairing error/event evidence, safe active-artifact
requeue, snapshot clearing, and closure-target Arbiter exemption.
`auto_port_v0_18_2_release_parity_evidence.json` records the Rust `0.3.2`
final release-parity evidence for Python `v0.18.1..v0.18.2`, including
version-visible CLI metadata, generated-scout path mapping evidence, package
include rules, README/changelog/roadmap/runtime docs,
Integrator/status/Recon/ownership package-readiness evidence, required
release-readiness command results, source-package mapping, the dirty-worktree
publish dry-run limitation, allow-dirty dry-run/package substitutes, and the
Python v0.18.2 `millrace-web` package/version unsupported-gap evidence. It
also confirms Builder release evidence does not require publish, upload, push,
tag, or deployment commands.
`auto_port_v0_18_3_parity_evidence.json` records target-facing guardrails for
the Python `v0.18.2..v0.18.3` to Rust `0.3.3` auto-port. It maps every
generated scout changed path to an expected Rust implementation, test,
documentation, fixture, package evidence, or unsupported-gap evidence target;
pins Librarian contracts/assets/graph/modes, Planner-to-Librarian trigger
metadata, learning request artifact metadata, runner normalization metadata,
shipped skill lint and guidance handoff source references, docs, version,
release-check, package dry-run, and `millrace-web` source references; and keeps
Rust `0.3.2` as the previous baseline while Rust `0.3.3` is the target. The
runner normalization/artifact-metadata target is now backed by focused runtime
JSON, runner normalization, serial runtime, and daemon runtime coverage, and the
active Librarian lifecycle target is now backed by focused serial and daemon
runtime coverage. The shipped skill lint/guidance target is now backed by
recursive packaged skill lint coverage and live/baseline guidance asset
synchronization.

`auto_port_v0_18_3_release_parity_evidence.json` records the Rust `0.3.3`
final release-parity evidence for Python `v0.18.2..v0.18.3`, including
version-visible CLI metadata, generated-scout path mapping evidence, package
include rules, README/changelog/roadmap/runtime docs, Librarian learning,
runner normalization, shipped skill lint package-readiness evidence, required
Builder verification command results, source-package mapping, dirty-worktree
package verification, generated-cache package exclusion evidence, and the
Python v0.18.3 `millrace-web` package/version unsupported-gap evidence. It also
confirms Builder release evidence does not
require publish, upload, push, tag, or deployment commands.

`auto_port_v0_18_4_parity_evidence.json` records target-facing guardrails for
the Python `v0.18.3..v0.18.4` to Rust `0.3.4` auto-port. It maps every
generated scout path to an expected Rust implementation, test, documentation,
fixture, package evidence, or unsupported-gap evidence target; pins the
annotated tag/release commit evidence, runner failure classifier metadata,
blocked metadata diagnostics, audited `queue retry-blocked` behavior,
`auto_recovery` config/status defaults and change boundaries, daemon
stranded-dependency recovery gates, required checks, and the Python v0.18.4
`millrace-web` package/version unsupported-gap evidence.
The runner failure classifier metadata and blocked metadata persistence targets
are now backed by typed runtime JSON contracts, runner normalization tests,
serial runtime metadata persistence tests, and queue-store requeue primitive
coverage. The retry-blocked CLI target is now backed by parity tests for
success, audit/event/snapshot evidence, non-retryable and missing metadata
refusals, root mismatch, exhausted budget, force override, and live runtime lock
refusal. The auto-recovery config/status target is now backed by runtime daemon
tests for defaults, explicit values, invalid values, daemon-session projection,
and next-tick boundary classification, plus parity CLI tests for invalid config
validation and `config show` output. The daemon recovery target is now backed by
runtime daemon tests for successful auto-requeue, review-required skips,
diagnostics, runtime/monitor events, and same-cycle dependent dispatch
suppression. Docs/version and final release evidence are reconciled in the
final release fixture.

`auto_port_v0_18_4_release_parity_evidence.json` records the Rust `0.3.4`
final release-parity evidence for Python `v0.18.3..v0.18.4`, including
version-visible CLI metadata, generated-scout path mapping evidence, package
include rules, README/changelog/roadmap/runtime docs, blocked recovery,
retry-blocked CLI, auto-recovery config/status, daemon recovery
package-readiness evidence, required Builder verification command results,
source-package mapping, dirty-worktree package verification, generated-cache
package exclusion evidence, and the Python v0.18.4 `millrace-web`
package/version unsupported-gap evidence. It also confirms Builder release
evidence does not require publish, upload, push, tag, or deployment commands.

`auto_port_v0_18_6_parity_evidence.json` records target-facing guardrails for
the Python `v0.18.4..v0.18.6` to Rust `0.3.5` auto-port. It maps every
generated scout path to an expected Rust implementation, test, documentation,
fixture, package evidence, or unsupported-gap evidence target; pins the
v0.18.4, v0.18.5, and v0.18.6 Python release commits, Rust `0.3.4 -> 0.3.5`
transition, operator intervention mailbox contracts, archive/audit ledgers,
direct and daemon-routed control surfaces, queue/status read-only evidence,
durable idea source behavior, closure recovery evidence, required checks, and
the Python v0.18.5/v0.18.6 `millrace-web` package/version unsupported-gap
evidence.

`auto_port_v0_19_0_parity_evidence.json` records target-facing guardrails for
the Python `v0.18.6..v0.19.0` to Rust `0.4.0` auto-port. It maps every
generated scout path to an expected Rust implementation, test, documentation,
fixture, package evidence, or unsupported-gap evidence target; pins the
v0.18.6 and v0.19.0 annotated tag objects and peeled commits, Rust
`0.3.5 -> 0.4.0` transition, execution capability contracts/config, compiled
capability grants, approval storage and CLI/runtime-control routing,
pre-dispatch capability gates, runner support/evidence metadata, inspection
surfaces, required checks, and the Python v0.19.0 `millrace-web`
package/version unsupported-gap evidence.

`auto_port_v0_20_0_parity_evidence.json` records target-facing guardrails for
the Python `v0.19.0..v0.20.0` to Rust `0.5.0` auto-port. It maps the generated
249-path scout to expected Rust implementation, test, documentation, fixture,
package evidence, or unsupported-gap targets; pins the v0.19.0 and v0.20.0
annotated tag objects and peeled commits, Rust `0.4.0 -> 0.5.0` transition,
workflow primitive assets, compiler authority, schema epochs, lanes,
request-context artifacts, runtime effects/failure policy, Blueprint Planning,
CLI `run once` removal, required checks, and the Python v0.20.0
`millrace-web` unsupported-gap evidence.

`auto_port_v0_20_0_release_parity_evidence.json` records the Rust `0.5.0`
final release-parity evidence for Python `v0.19.0..v0.20.0`, including
version-visible CLI metadata, generated-scout path mapping evidence, package
include rules, README/changelog/roadmap/runtime docs, workflow primitive
authority, Blueprint Planning runtime evidence, required Builder verification
command results, dirty-worktree package verification, generated-cache package
exclusion evidence, and the Python v0.20.0 `millrace-web`
package/dashboard-summary unsupported gap. It also confirms Builder release
evidence does not require publish, upload, push, tag, or deployment commands.

`auto_port_v0_18_6_release_parity_evidence.json` records the Rust `0.3.5`
final release-parity evidence for Python `v0.18.4..v0.18.6`, including
version-visible CLI metadata, generated-scout path mapping evidence, package
include rules, README/changelog/roadmap/runtime docs, operator intervention
commands, intervention archive/audit/read-only package-readiness evidence,
durable idea-source and closure recovery evidence, required Builder
verification command results, source-package mapping, dirty-worktree package
verification, generated-cache package exclusion evidence, and the Python
v0.18.5/v0.18.6 `millrace-web` package/version unsupported gap. It also
confirms Builder release evidence does not require publish, upload, push, tag,
or deployment commands.

`auto_port_v0_19_0_release_parity_evidence.json` records the Rust `0.4.0`
final release-parity evidence for Python `v0.18.6..v0.19.0`, including
version-visible CLI metadata, generated-scout path mapping evidence, package
include rules, README/changelog/roadmap/runtime docs, capability
contracts/config, compiled grants, approval CLI/runtime-control behavior,
pre-dispatch gates, runner support/evidence metadata, run-inspection
capability output, required Builder verification command results,
source-package mapping, dirty-worktree package verification,
generated-cache package exclusion evidence, and the Python v0.19.0
`millrace-web` package/version unsupported gap. It also confirms Builder
release evidence does not require publish, upload, push, tag, or deployment
commands.

The evidence is intentionally not a byte-for-byte transcript. Paths, generated
ids, timestamps, package versions, command ids, run ids, compact run handles,
process ids, compiled plan ids, config versions, token counts, timeout
durations, runner artifact paths, and incident ids are normalized in the Rust
tests while preserving Python-owned command names, exit-code classes, key
output lines, file mutation behavior, mailbox artifacts, initialized-workspace
refusal, parse-failure classes, serial runtime state transition semantics,
advanced usage-governance and subscription-quota semantics, learning-promotion
and skill-evidence artifacts, closure-lineage and Arbiter transitions, advanced
run-inspection visibility, handoff queue/status transitions, runtime-error and
handoff incident evidence, daemon key-line or structural event parity, and
runner request/result artifact shape. Live Codex and Pi smoke coverage, live
subscription quota provider polling, and native filesystem watcher integration
remain explicitly opt-in or preview-only, and the optional Rust web dashboard
remains explicitly unsupported rather than omitted from parity evidence.
