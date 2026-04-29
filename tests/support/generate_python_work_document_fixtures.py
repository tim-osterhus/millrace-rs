from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path

from millrace_ai.contracts import (
    IncidentDocument,
    LearningRequestDocument,
    SpecDocument,
    TaskDocument,
)
from millrace_ai.work_documents import render_work_document


NOW = datetime(2026, 4, 15, tzinfo=timezone.utc)


def build_fixtures() -> dict[str, object]:
    return {
        "task.md": TaskDocument(
            task_id="task-fixture",
            title="Fixture task",
            summary="Representative Python task fixture",
            root_idea_id="idea-001",
            root_spec_id="spec-root-001",
            spec_id="spec-root-001",
            target_paths=("src/contracts/",),
            acceptance=("Rust parses this task fixture",),
            required_checks=("cargo test",),
            references=("../millrace-py/tests/runtime/test_contracts.py",),
            risk=("fixture drift",),
            depends_on=("task-prereq",),
            blocks=("task-next",),
            tags=("slice-1", "parity"),
            created_at=NOW,
            created_by="python-fixture",
        ),
        "spec.md": SpecDocument(
            spec_id="spec-fixture",
            title="Fixture spec",
            summary="Representative Python spec fixture",
            source_type="manual",
            source_id="idea-001",
            root_idea_id="idea-001",
            root_spec_id="spec-root-001",
            goals=("define typed models",),
            non_goals=("implement scheduling",),
            scope=("contract fixtures",),
            constraints=("stay deterministic",),
            assumptions=("Python reference is pinned",),
            risks=("schema drift",),
            target_paths=("src/contracts/",),
            entrypoints=("src/lib.rs",),
            required_skills=("builder-core",),
            decomposition_hints=("keep parser separate",),
            acceptance=("Rust parses this spec fixture",),
            references=("../millrace-py/src/millrace_ai/contracts/",),
            created_at=NOW,
            created_by="python-fixture",
        ),
        "incident.md": IncidentDocument(
            incident_id="inc-fixture",
            title="Fixture incident",
            summary="Representative Python incident fixture",
            root_idea_id="idea-001",
            root_spec_id="spec-root-001",
            source_task_id="task-fixture",
            source_spec_id="spec-fixture",
            source_stage="auditor",
            source_plane="planning",
            failure_class="arbiter_parity_gap",
            severity="medium",
            needs_planning=True,
            trigger_reason="parity gap found",
            observed_symptoms=("rendered markdown lost lineage",),
            failed_attempts=("builder pass",),
            consultant_decision="needs_planning",
            evidence_paths=("millrace-agents/runs/run-001/report.md",),
            related_run_ids=("run-001",),
            related_stage_results=("request-001.json",),
            references=("docs/rust-port-roadmap.md",),
            opened_at=NOW,
            opened_by="python-fixture",
        ),
        "learning_request.md": LearningRequestDocument(
            learning_request_id="learn-fixture",
            title="Fixture learning request",
            summary="Representative Python learning request fixture",
            requested_action="improve",
            target_skill_id="checker-core",
            target_stage="curator",
            source_refs=("run:run-001",),
            preferred_output_paths=(
                "millrace-agents/skills/stage/execution/checker-core/SKILL.md",
            ),
            trigger_metadata={
                "source_stage": "doublechecker",
                "terminal_result": "DOUBLECHECK_PASS",
            },
            originating_run_ids=("run-001",),
            artifact_paths=(
                "millrace-agents/runs/run-001/stage_results/request-001.json",
            ),
            references=("docs/rust-port-roadmap.md",),
            created_at=NOW,
            created_by="python-fixture",
        ),
    }


def main() -> None:
    fixture_dir = Path(__file__).resolve().parents[1] / "fixtures" / "work_documents"
    fixture_dir.mkdir(parents=True, exist_ok=True)
    for name, document in build_fixtures().items():
        (fixture_dir / name).write_text(render_work_document(document), encoding="utf-8")


if __name__ == "__main__":
    main()
