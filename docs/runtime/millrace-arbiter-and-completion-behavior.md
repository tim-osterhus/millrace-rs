# Millrace Arbiter And Completion Behavior

This document records the Rust completion model for modes that select
`planning.standard`, including `default_codex`, `default_pi`,
`learning_codex`, `learning_pi`, `default_codex_integrated`, and
`learning_codex_integrated`.

Millrace does not treat backlog drain as automatic completion. When a root
lineage has an open closure target and no queued, active, or blocked work
remains for that lineage, the compiled planning-loop `completion_behavior`
dispatches the `arbiter` stage through the normal runner contract.

## Root Lineage Model

Closure behavior is keyed by explicit root-lineage fields carried through work
documents:

- `root_spec_id`
- `root_idea_id`

Those fields live on canonical task, probe, spec, incident, and learning
request markdown documents where that lineage is meaningful. Immediate
provenance fields still exist, but Arbiter uses root lineage so it does not
guess which spec family it is judging after remediation churn.

Watcher-seeded root specs initialize both fields immediately. Rust `0.3.5`
also preserves original idea markdown under
`millrace-agents/intake/ideas/<root_idea_id>.md`; generated specs reference
that runtime-owned copy before transient `ideas/inbox/` source paths.

## Canonical Contract Sources

Arbiter judges against canonical copies under its workspace subtree:

- `millrace-agents/arbiter/contracts/ideas/<root_idea_id>.md`
- `millrace-agents/arbiter/contracts/root-specs/<root_spec_id>.md`

The runtime snapshots those copies when the root spec first enters managed
lineage. For Python `v0.18.6` parity, Rust prefers the durable
`millrace-agents/intake/ideas/` copy when creating or backfilling closure
targets, then falls back to legacy spec references and inbox paths. If no
candidate source exists during backlog-drain recovery, Planning is marked
blocked with `missing_root_idea_source` and the runtime emits
`root_idea_source_missing` instead of terminating the daemon loop.

## Closure Target State

The runtime owns one closure-target state file per root spec:

- `millrace-agents/arbiter/targets/<root_spec_id>.json`

The shipped v1 policy is one open closure target per workspace. The target file
records root lineage ids, canonical contract paths, rubric path, latest
verdict/report paths, whether closure is still open, whether remaining lineage
work still blocks closure, and the last Arbiter run id.

## Backlog-Drain Behavior

The compiled planning-loop `completion_behavior` for `planning.standard` is:

- trigger: `backlog_drained`
- readiness rule: `no_open_lineage_work`
- stage: `arbiter`
- request kind: `closure_target`
- target selector: `active_closure_target`
- blocked-work policy: `suppress`

Runtime behavior is:

1. If no closure target is open, claim normal planning, execution, or learning
   work.
2. If one closure target is open, defer unrelated queued root specs and claim
   only same-lineage execution or planning work.
3. If no same-lineage work remains, inspect the compiled completion behavior.
4. Locate the single open closure target.
5. If no open target exists, backfill one from the latest root spec that
   carries root-lineage ids and has recoverable source evidence.
6. Scan queued, active, and blocked work for matching `root_spec_id`.
7. Suppress Arbiter if lineage work remains.
8. Dispatch Arbiter when the target is eligible.

Missing root-lineage metadata or missing root idea source evidence blocks
Planning with diagnosable runtime events rather than silently idling through
required closure behavior.

## Operator Inspection Surfaces

Operators inspect this behavior through local CLI and workspace artifacts:

- `millrace compile show` prints frozen `completion_behavior`.
- `millrace status` prints the active open closure target, deferred-root count,
  and latest verdict/report paths.
- `millrace runs show` prints request kind and closure-target lineage for
  Arbiter runs.

Use those surfaces before opening raw JSON files unless you need the full
artifact payload.
