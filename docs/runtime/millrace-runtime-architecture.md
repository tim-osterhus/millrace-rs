# Runtime Architecture

The Rust runtime keeps queue and run mutation behind runtime-owned boundaries:
startup loads compiled-plan authority, tick activation claims at most one
eligible work item or closure target, stage dispatch persists request/result
artifacts, and router decisions apply queue/state transitions through typed
helpers.

For Python `v0.18.0` parity, dispatch also writes best-effort
`run_trace.json` evidence under `millrace-agents/runs/<run_id>/` after
stage-result persistence and authoritative router decisions. Trace nodes name
the stage result, terminal/result class, runner/model/thinking metadata,
duration, token usage, and artifact refs. Trace edges name the router action,
reason, next node or terminal state, and spawned learning-request or incident
refs when routing creates follow-up work. Trace write failures emit runtime
events without changing otherwise valid stage or routing outcomes.

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
`config show`, `compile show`, `compile graph`, and `runs trace <run_id>`.
Those graph/trace CLI commands shadow Python web graph and trace readers
without adding a Rust web server, dashboard API, static shell, SSE stream, or
separate dashboard package.
