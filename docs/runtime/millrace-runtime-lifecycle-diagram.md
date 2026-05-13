# Millrace Runtime Lifecycle Diagram

This is a compact Rust lifecycle chart for the current compiled-plan runtime.

```mermaid
flowchart TB
    Init["initialized workspace"] --> Config["load config"]
    Config --> Compile["load or compile plan"]
    Compile --> Snapshot["load snapshot and counters"]
    Snapshot --> Mailbox["drain mailbox"]
    Mailbox --> Queues["refresh queues"]
    Queues --> Governance["evaluate governance"]
    Governance --> Claim{"claim or resume work"}
    Claim -->|task| Builder["execution: builder"]
    Claim -->|probe| Recon["planning: recon"]
    Claim -->|spec| Planner["planning: planner"]
    Claim -->|incident| Auditor["planning: auditor"]
    Claim -->|learning request| Learning["learning: analyst/professor/curator/librarian"]
    Claim -->|closure ready| Arbiter["planning: arbiter"]
    Builder --> Dispatch["runner dispatch"]
    Recon --> Dispatch
    Planner --> Dispatch
    Auditor --> Dispatch
    Learning --> Dispatch
    Arbiter --> Dispatch
    Dispatch --> Result["persist request/result/artifacts"]
    Result --> Route["apply compiled router decision"]
    Route --> Trace["write run_trace evidence"]
    Trace --> Snapshot
```

## v0.18.3 Learning Path

In learning-enabled modes, a Planner `PLANNER_COMPLETE` result can enqueue a
targeted Librarian request. The request preserves the stage-result artifact,
Planner-produced artifacts, and source work-item metadata. A later learning
claim dispatches Librarian from the compiled learning graph. Librarian complete
and no-op outcomes move the learning request to done; blocked outcomes preserve
recoverable-failure evidence.

Default modes remain serial and do not enqueue Planner-to-Librarian learning
requests. Daemon learning concurrency still applies only through compiled
plane-concurrency policy.
