# Provenance

`millrace-rs` is intentionally kept as a utilitarian Rust crate repository:
source, tests, crate metadata, and concise operator/developer documentation.

The larger Rust-port proof package lives separately:

```text
https://github.com/tim-osterhus/millrace-rs-port-docs
```

That repository contains:

- the v0.1.0 autonomous-build proof
- the original seeded ideas
- the full parity roadmap and campaign notes
- generated token/timing/stage metrics
- scripts that recompute the proof metrics from raw artifacts
- instructions for publishing the raw evidence bundle as a release asset

The split keeps this repo focused on the Rust implementation while preserving a
public audit trail for how the Python Millrace runtime produced the v0.1.0 Rust
port.
