# Rust Runtime Docs

These notes document the Rust-owned runtime contract surfaces for the
experimental `millrace-ai` crate. They are concise Rust equivalents of the
Python runtime docs changed in `v0.17.4`; they do not claim production status or
introduce a Rust web dashboard.

Start with:

- `millrace-compiler-and-frozen-plans.md`: compile authority, learning trigger
  destination safety, and frozen plan evidence
- `millrace-modes-and-loops.md`: shipped modes, learning no-op terminal
  outcomes, and trigger routing
- `millrace-runtime-architecture.md`: runtime request/result application,
  learning no-op lifecycle, and read-only inspection boundaries

The stable contract remains the `millrace` CLI plus the local
`millrace-agents/` workspace artifact format. Python `v0.17.4` remains the
reference for the release delta represented by Rust `0.2.1`.
