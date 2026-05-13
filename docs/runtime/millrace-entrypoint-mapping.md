# Millrace Entrypoint Mapping

This document maps the Rust package baseline entrypoint sources to the
workspace paths used by compiled stage requests.

## Packaged Source To Workspace

During `millrace init`, Rust copies managed entrypoints from
`src/assets/baseline/entrypoints/` into
`<workspace>/millrace-agents/entrypoints/`. `millrace upgrade --apply` refreshes
safe managed updates through the baseline manifest.

Execution entrypoints include Builder, Checker, Fixer, Doublechecker, Updater,
Troubleshooter, Consultant, and the opt-in Integrator entrypoint. Planning
entrypoints include Recon, Planner, Manager, Mechanic, Auditor, and Arbiter.
Learning entrypoints include Analyst, Professor, Curator, and Librarian.

The v0.18.3 parity line adds:

- `src/assets/baseline/entrypoints/learning/librarian.md`
- `millrace-agents/entrypoints/learning/librarian.md`
- `src/assets/baseline/registry/stage_kinds/learning/librarian.json`
- `src/assets/baseline/skills/stage/learning/librarian-core/SKILL.md`

## Request Contract

Stage requests point at deployed workspace entrypoints and carry the active
work item path supplied by the runtime. For v0.18.3 Librarian dispatch,
learning requests use:

- `active_work_item_kind = learning_request`
- `active_work_item_path = millrace-agents/learning/requests/active/<ID>.md`
- legal markers `### LIBRARIAN_COMPLETE`, `### LIBRARIAN_NOOP`, and
  `### BLOCKED`
- running marker `LIBRARIAN_RUNNING`

Required stage-core skills are compile-time stage metadata. Optional secondary
skills remain advisory and must be present in the packaged or installed skills
surface before an entrypoint can reference them.
