# Runtime Architecture

The Rust runtime keeps queue and run mutation behind runtime-owned boundaries:
startup loads compiled-plan authority, tick activation claims at most one
eligible work item or closure target, stage dispatch persists request/result
artifacts, and router decisions apply queue/state transitions through typed
helpers.

Learning request activation uses active request documents under
`millrace-agents/learning/requests/active/`. For Python `v0.17.4` parity,
stage-specific no-op terminal results move the active learning request to
`millrace-agents/learning/requests/done/`, not `blocked/`, while preserving the
stage-result, terminal-marker, router-decision, and run-inspection evidence.

Runtime-generated learning requests copy compiled trigger destination metadata
into both queued work documents and trigger metadata. This preserves
`target_skill_id` and `preferred_output_paths` for downstream learning stages
without allowing destination-less direct Curator requests.

The optional Python `millrace-web` package remains outside the accepted Rust
runtime boundary. Rust inspection stays local and read-only through CLI commands
such as `queue ls/show`, `status show`, `runs ls/show/tail`, `modes show`,
`config show`, and `compile show`.
