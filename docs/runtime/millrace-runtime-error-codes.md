# Runtime Error Codes

Runtime error context is persisted as `runtime_error_context` JSON plus a
human-readable `runtime_error_report.md`. The Rust `0.5.0` runtime keeps these
codes stable for recovery routing and run inspection.

## Persisted Codes

- `planning_work_item_completion_conflict`: Planning stage output conflicts
  with the active planning work item.
- `execution_work_item_completion_conflict`: Execution stage output conflicts
  with the active execution work item.
- `planning_post_stage_apply_failed`: Planning result application failed after
  a stage result was normalized.
- `execution_post_stage_apply_failed`: Execution result application failed
  after a stage result was normalized.
- `recon_handoff_invalid`: Recon handoff artifacts were malformed or
  inconsistent with generated work expectations.
- `stage_work_item_ownership_invalid`: A stage was paired with an unsupported
  work-item kind before runner dispatch.

## v0.20.0 Failure Evidence

Workflow primitive and Blueprint runtime failures also surface through
failure-class and runtime-effect evidence rather than always creating a new
top-level runtime error code. Run inspection distinguishes:

- artifact parse validity
- route outcome
- runtime-effect decision and result artifacts
- failure origin
- failure class
- mutation phase
- matched runtime failure policy
- source lifecycle intent
- created/generated paths

Blueprint pre-mutation failures can be routed to `mechanic_blueprint`.
Partial-mutation failures block conservatively, even if a route policy would
otherwise match the failure class. Planner disposition mismatches, missing
Blueprint artifacts, duplicate manifests, and invalid source-terminal evidence
are kept visible as Arbiter-visible runtime-effect or blocked-source evidence.
