# Fixture probe

Probe-ID: probe-fixture
Title: Fixture probe
Summary: Representative Python probe fixture
Request: Research the current codebase and route the smallest safe change.
Status-Hint: queued
Created-At: 2026-04-15T00:00:00Z
Created-By: python-fixture

Target-Paths:
- src/example/parser.py

Constraints:
- Do not implement during recon.

Acceptance:
- Recon routes the probe with a durable packet.

Risk-Notes:
- Parser changes can regress adjacent behavior.

References:
- operator request

Tags:
- probe
- parity
