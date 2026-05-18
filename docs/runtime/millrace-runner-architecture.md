# Runner Architecture

Rust runner dispatch preserves the Python-owned
`StageRunRequest -> RunnerRawResult -> StageResultEnvelope` boundary. The
runtime prepares one stage request from frozen plan authority, the selected
runner adapter executes or simulates that request, and normalization turns the
raw result into the canonical stage-result envelope consumed by routing.

## Request Context

`StageRunRequest` includes runner-neutral model, thinking, timeout, active work
item, closure target, artifact, legal terminal, skill, and execution capability
context. For `0.4.0`, the request also carries compiled execution capability
grants and adapter support decisions so the prompt and runner artifacts can
explain which operations are granted, unsupported, advisory, or approval-bound.

## Adapter Boundary

The shipped adapters share one `StageRunnerAdapter` contract:

- fake runner for deterministic tests
- Codex CLI adapter with prompt/stdout/stderr/event/completion artifacts,
  thinking-to-reasoning mapping, timeout evidence, and advisory capability
  support under broad permission posture
- Pi RPC adapter with JSONL prompt lifecycle, event-log policy, session stats,
  timeout evidence, and conservative advisory support for remote boundaries

Dispatcher selection uses configured runner names and does not let individual
stages choose queue movement or release actions.

## Artifact Flow

Runner invocation artifacts persist the request context, grants, support
decisions, and initial evidence refs. Completion artifacts persist observed
exit data, stdout/stderr/event paths, capability evidence refs, missing
evidence refs, and capability failure class. Normalization copies the same
capability metadata into stage-result metadata for read-only inspection.

Missing required capability evidence normalizes to
`capability_evidence_missing`. Denied or unsupported grants are blocked earlier
by runtime capability gates, before runner invocation.

## Release Boundary

Normal Millrace stages do not publish, upload, deploy, push, or tag release
artifacts. The runner boundary records what a stage did and what evidence it
produced; release validation remains an operator/package-readiness concern
recorded in fixtures and command outputs.

