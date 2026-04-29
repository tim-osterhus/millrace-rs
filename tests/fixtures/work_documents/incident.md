# Fixture incident

Incident-ID: inc-fixture
Title: Fixture incident
Summary: Representative Python incident fixture
Root-Idea-ID: idea-001
Root-Spec-ID: spec-root-001
Source-Task-ID: task-fixture
Source-Spec-ID: spec-fixture
Source-Stage: auditor
Source-Plane: planning
Failure-Class: arbiter_parity_gap
Trigger-Reason: parity gap found
Consultant-Decision: needs_planning
Opened-At: 2026-04-15T00:00:00Z
Opened-By: python-fixture

Observed-Symptoms:
- rendered markdown lost lineage

Failed-Attempts:
- builder pass

Evidence-Paths:
- millrace-agents/runs/run-001/report.md

Related-Run-IDs:
- run-001

Related-Stage-Results:
- request-001.json

References:
- docs/rust-port-roadmap.md
