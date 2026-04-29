# Fixture task

Task-ID: task-fixture
Title: Fixture task
Summary: Representative Python task fixture
Root-Idea-ID: idea-001
Root-Spec-ID: spec-root-001
Spec-ID: spec-root-001
Created-At: 2026-04-15T00:00:00Z
Created-By: python-fixture

Depends-On:
- task-prereq

Blocks:
- task-next

Tags:
- slice-1
- parity

Target-Paths:
- src/contracts/

Acceptance:
- Rust parses this task fixture

Required-Checks:
- cargo test

References:
- ../millrace-py/tests/runtime/test_contracts.py

Risk:
- fixture drift
