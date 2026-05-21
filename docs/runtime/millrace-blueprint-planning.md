# Blueprint Planning

Blueprint Planning is the opt-in Python `v0.20.0` planning loop ported into
the Rust `0.5.0` parity boundary. It is selected by the `blueprint_codex` and
`blueprint_learning_codex` modes, which compile the `planning.blueprint` graph.

## Packaged Assets

The Rust package includes Blueprint graph, mode, stage-kind, entrypoint, skill,
document-adapter, work-item-family, runtime-effect, runtime-failure-policy, and
lifecycle assets under `src/assets/baseline/`. `millrace init` and
`millrace upgrade --apply` deploy those managed assets into workspaces through
the normal baseline manifest path.

Blueprint stage ids are:

- `manager_blueprint`
- `contractor_blueprint`
- `evaluator_blueprint`
- `mechanic_blueprint`

Blueprint work uses the `blueprint_draft` family. Drafts are runtime-owned
documents under `millrace-agents/blueprints/drafts/`.

## Runtime Lifecycle

Manager Blueprint consumes a manifest artifact and queues draft records.
Contractor Blueprint writes candidate packets for active drafts. Evaluator
Blueprint records approvals or rejections. Approved packets create generated
execution tasks plus evaluation and promotion evidence. Rejected packets create
critique evidence and route back to Contractor. Mechanic Blueprint receives
policy-routed pre-mutation failure context when Manager Blueprint artifacts are
missing.

Stages do not directly move Blueprint drafts or generated tasks. Stage results
and artifacts are evidence consumed by compiled runtime-effect handlers. The
runtime applies lifecycle intents, promotion, blocking, retry, or repair as the
single writer.

## Failure And Replay Rules

Compiled failure policy distinguishes pre-mutation and partial-mutation
failures. Pre-mutation Manager Blueprint artifact failures can route to
Mechanic Blueprint when policy permits. Duplicate manifest detection and
partial mutation diagnostics block conservatively with created-path evidence.
Idempotent replay uses `manifest_id` as manifest identity; legacy root-keyed
manifest reads remain supported when the embedded manifest id can be resolved.

## Closure

Arbiter closure is suppressed while same-lineage Blueprint drafts, packets,
evaluations, critiques, promotions, or generated execution tasks remain open.
The root can become closure-ready only after Blueprint artifacts and generated
execution work drain.

## Inspection

Operators inspect Blueprint authority and runtime evidence through local CLI
surfaces:

- `millrace compile show`
- `millrace compile graph`
- `millrace status`
- `millrace runs ls`
- `millrace runs show`
- `millrace runs tail`
- `millrace runs trace <run_id>`

The Rust crate does not add a web dashboard for Blueprint evidence. Optional
Python `millrace-web` v0.20.0 dashboard changes remain unsupported-gap
evidence.
