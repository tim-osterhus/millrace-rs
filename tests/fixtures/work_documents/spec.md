# Fixture spec

Spec-ID: spec-fixture
Title: Fixture spec
Summary: Representative Python spec fixture
Source-Type: manual
Source-ID: idea-001
Root-Idea-ID: idea-001
Root-Spec-ID: spec-root-001
Created-At: 2026-04-15T00:00:00Z
Created-By: python-fixture

Goals:
- define typed models

Non-Goals:
- implement scheduling

Scope:
- contract fixtures

Constraints:
- stay deterministic

Assumptions:
- Python reference is pinned

Risks:
- schema drift

Target-Paths:
- src/contracts/

Entrypoints:
- src/lib.rs

Required-Skills:
- builder-core

Decomposition-Hints:
- keep parser separate

Acceptance:
- Rust parses this spec fixture

References:
- ../millrace-py/src/millrace_ai/contracts/
