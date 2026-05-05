# CLI Reference

Installed command: `millrace`

## Version

Use either form to print the Rust crate version:

```bash
millrace --version
millrace version
```

For Rust `0.3.0`, both commands print `millrace 0.3.0`.

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
routes. Rust `0.3.0` shadows the accepted local inspection behavior through the
CLI commands above and keeps the optional web dashboard as an explicit
unsupported gap. No Rust web server, dashboard HTTP API, static shell, SSE
stream, separate dashboard package, or Rust-managed web asset is part of this
crate release.
