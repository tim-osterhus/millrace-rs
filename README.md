# Millrace Rust

`millrace-ai` is the experimental Rust implementation of Millrace, a
governed runtime for long-running agent work.

The production implementation is currently the Python package
[`millrace-ai`](https://pypi.org/project/millrace-ai/). The initial Rust
`0.0.x` releases establish the package, library crate, and CLI surface while
contract-parity work begins.

## Package Names

```text
Cargo package: millrace-ai
Library crate: millrace_ai
CLI binary:    millrace
Repository:    https://github.com/tim-osterhus/millrace-rs
Website:       https://millrace.ai
```

## Current Status

This crate is intentionally small. It exposes a status API and a `millrace`
binary that report the Rust runtime's experimental state.

Do not depend on runtime behavior from this crate yet. Public APIs may change
while the Rust implementation is brought toward parity with the Python runtime.

## Rust Port Roadmap

The behavioral parity plan lives in [docs/rust-port-roadmap.md](docs/rust-port-roadmap.md).

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
