# Execution Capabilities

Rust `0.4.0` records the Python `v0.19.0` execution capability governance
surface without expanding normal stage authority. Capability data is compile
and runtime evidence: stages still emit terminal markers, while the runtime owns
dispatch, gates, approvals, queue movement, and persisted state.

## Contracts

The public contract layer includes:

- canonical capability ids and aliases such as `runner.invoke`,
  `workspace.read`, `artifact.write`, `package.install`, `network.access`,
  `approval.request`, `evidence.emit`, and `runtime.control`
- `CapabilityScope`, `ApprovalPolicyRef`, `CapabilityRequest`,
  `CapabilityPolicyOverride`, `ExecutionCapabilityGrant`, and
  `CapabilitySupportDecision`
- decision, enforcement, evidence, policy, and support enum values
- stable `grant-<sha256-prefix>` fingerprints for resolved grants
- `approve_execution_capability` and `deny_execution_capability` mailbox
  payload validation with safe approval ids and non-empty reasons

The default policy denies unknown capabilities, denies raw network access,
requires approval for package installation and git mutation, allows shell
execution and workspace writes, and allows advisory grants unless strict
required-advisory failure is configured.

## Compile Authority

The compiler resolves stage-kind, graph-node, mode, and runtime config
declarations into sealed per-node grants. Frozen plans preserve:

- execution capability grants
- policy fingerprints
- grant warnings
- plan and per-plane summary counts

`millrace compile show` renders this evidence as summary lines plus compact
`execution_capability_grant` and `execution_capability_warning` lines.
Compiled-stage-graph JSON exports include the same grant evidence for
inspection. Graph exports do not replace the frozen plan as runtime authority.

## Runtime Gates

Serial and daemon dispatch evaluate compiled grants before runner invocation or
`stage_started` side effects. Gate evaluation writes
`capability_gate.<request_id>.json`, emits `capability_gate_evaluated`, and
blocks denied, unsupported, unresolved approval-required, or missing-evidence
required grants as recoverable runtime-policy failures.

Approval-required grants create durable records under
`millrace-agents/approvals/pending/`. `millrace approvals approve` and
`millrace approvals deny` resolve records directly when no daemon owns the
workspace, or enqueue approval mailbox commands when an active daemon owns it.
Daemon mailbox intake applies those decisions at the runtime-owned boundary and
archives processed or failed command evidence.

## Runner Evidence

Runner invocation and completion artifacts, raw runner results, and normalized
stage-result metadata carry grants, support decisions, capability evidence
refs, missing evidence refs, and `failure_capability_class`.

`millrace runs show` renders stage-result metadata as compact
`capability_grant` and `capability_support` lines when that evidence is present.
These lines are read-only inspection output; they do not grant stages publish,
upload, deploy, push, or tag authority.

