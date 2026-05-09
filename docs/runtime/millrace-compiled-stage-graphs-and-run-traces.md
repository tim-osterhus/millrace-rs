# Compiled Stage Graphs And Run Traces

Rust exposes two graph-shaped inspection surfaces for the Python `v0.18.0`
parity line, and they have different authority.

The compiled stage graph is reusable control-flow topology produced from the
selected mode, graph-loop, stage-kind, entrypoint, skill, config, and managed
asset inputs. It is projected from the persisted
`millrace-agents/state/compiled_plan.json` authority and remains a read-only
inspection export. It does not replace compiled-plan routing authority.

The run trace graph is historical evidence for one concrete run. It records
stage-result nodes, runtime router-decision edges, artifact refs, spawned work,
and trace status. New Rust-dispatched runs write
`millrace-agents/runs/<run_id>/run_trace.json` as best-effort runtime-owned
evidence.

Do not describe the compiled topology as a DAG. Shipped control-flow graphs can
contain intentional recovery cycles. A run trace is usually acyclic because it
records events that already happened, but it is still an inspection artifact,
not a second routing source.

## Compiled Graph CLI

Use `millrace compile graph` to inspect legal topology for the selected mode:

```bash
millrace compile graph --workspace <workspace>
millrace compile graph --workspace <workspace> --plane execution
millrace compile graph --workspace <workspace> --format json
millrace compile graph --workspace <workspace> --output compiled-graphs.json
```

The JSON output is a list of `compiled_stage_graph` exports. Each export names
the compiled plan, mode, loop, plane, source refs, entries, nodes, edges,
terminal states, legal outcome result-class mappings, skill paths,
runner/model/thinking metadata, timeouts, and declared output artifacts. The
command uses the same compile path as `compile validate` and `compile show` and
does not mutate queue or runtime snapshot state.

## Run Trace CLI

Use `millrace runs trace <run_id>` when diagnosing what one run actually did:

```bash
millrace runs trace <run_id> --workspace <workspace>
millrace runs trace <run_id> --workspace <workspace> --format json
millrace runs trace <run_id> --workspace <workspace> --output run-trace.json
```

The JSON output is a `run_trace_graph`. Trace nodes represent concrete stage
results. Trace edges represent the authoritative router decision applied after
each result and can point to a next compiled node, terminal state,
blocked/handoff status, or spawned work such as a planning incident or learning
request.

Older runs remain inspectable. If `run_trace.json` is absent, Rust derives a
read-only `incomplete` fallback trace from `stage_results/*.json`. If
`run_trace.json` is malformed, Rust renders a `malformed` fallback trace with a
diagnostic note. Inspection never repairs, deletes, normalizes, or rewrites the
run directory.

Trace writing is best-effort and runtime-owned. Stage workers do not write trace
artifacts directly, and a trace write failure emits runtime event evidence
without converting an otherwise valid stage result or router decision into
failure.

## Web Gap Boundary

Python `packages/millrace-web` `v0.18.0` uses the same graph and trace reader
contracts for compiled-plan graph APIs, compact run-trace summaries, recent
trace Flow overlays, trace outcome labels, package version/dependency sync, and
read-only/no-lock dashboard guarantees.
Python `v0.18.1` syncs the optional web package version, `millrace-ai>=0.18.1`
dependency floor, and FastAPI app version without changing Rust's accepted
package boundary.

Rust records that as unsupported-gap and shadow-CLI evidence. The accepted Rust
surface is the CLI plus local `millrace-agents/` artifacts, so `millrace compile
graph` and `millrace runs trace <run_id>` are implemented, while a Rust web
server, dashboard HTTP API, static shell, SSE stream, separate dashboard
package, or Rust-managed web assets are not added.
