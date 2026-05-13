# CLI Reference

Installed command: `millrace`

## Version

Use either form to print the Rust crate version:

```bash
millrace --version
millrace version
```

For Rust `0.3.3`, both commands print `millrace 0.3.3`.

## Probe Intake And Inspection

Rust `0.3.1` adds the Python `v0.18.1` probe intake surface. Operators can
import canonical probe markdown or JSON either through the top-level alias or
the grouped queue command:

```bash
millrace add-probe probe.md --workspace <workspace>
millrace queue add-probe probe.json --workspace <workspace>
```

When a daemon owns the workspace, both forms route through the mailbox command
name `add_probe`; otherwise they write directly through the same runtime-control
boundary. `millrace queue ls` reports probe queue and lifecycle counts, and
`millrace queue show <probe-id>` renders canonical probe fields without moving
or normalizing the inspected document.

## Status JSON Diagnostics

Rust `0.3.2` adds the Python `v0.18.2` status JSON diagnostics surface.
`millrace status` and `millrace status show` accept `--format text|json`; text
remains the default, while JSON reports the shared read-only status payload,
including active state, queue depths, closure-target diagnostics,
`blocked_idle`, `current_failure_class`, and the latest runtime error report
path. `millrace status watch` remains text-only and rejects JSON format
requests deterministically.

## Graph And Trace Inspection

`millrace compile graph` exports the selected compiled-plan topology as text or
JSON. It supports initialized workspaces, the built-in mode/config selection
used by adjacent compile commands, optional plane filtering, and output-file
writes.

```bash
millrace compile graph --workspace <workspace>
millrace compile graph --workspace <workspace> --mode learning_codex
millrace compile graph --workspace <workspace> --plane planning --format json
millrace compile graph --workspace <workspace> --output planning-graph.json
```

`millrace runs trace <run_id>` inspects one persisted run as text or JSON. New
runs can include `run_trace.json`; older or malformed runs are rendered through
read-only fallback inspection from `stage_results/*.json`.

```bash
millrace runs trace <run_id> --workspace <workspace>
millrace runs trace <run_id> --workspace <workspace> --format json
millrace runs trace <run_id> --workspace <workspace> --output run-trace.json
```

Both commands are inspection surfaces. They do not acquire runtime ownership,
do not add or move queue items, and do not repair or normalize inspected run
artifacts.

## Web Boundary

Python `millrace-web` exposes graph and trace data through read-only dashboard
routes, and Python `v0.18.3` syncs that optional package to version `0.18.3`.
Rust `0.3.3` shadows the accepted local inspection behavior through the CLI
commands above and keeps the optional web dashboard as an explicit unsupported
gap. No Rust web server, dashboard HTTP API, static shell, SSE stream, separate
dashboard package, or Rust-managed web asset is part of this crate release.

## Librarian And Learning Evidence

Rust `0.3.3` adds the Python `v0.18.3` Librarian learning-stage parity surface.
There is no new operator web or server command. Operators inspect the result
through the existing local CLI and workspace artifacts:

- `millrace compile show` and `millrace compile graph` expose the compiled
  Librarian node, legal terminals, required skill, and learning trigger rules.
- `millrace runs show`, `millrace runs tail`, and `millrace runs trace` expose
  persisted Librarian request/result/trace evidence.
- `millrace status` continues to report learning queue depths and active run
  state without mutating the workspace.
