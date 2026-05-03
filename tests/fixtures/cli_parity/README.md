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
intentional unsupported Rust parity gap. It names the workspace registry,
summary DTO, queue/run/snapshot/baseline/compiled-plan/Arbiter/
usage-governance/event readers, static shell, CLI/server boundary, and
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
