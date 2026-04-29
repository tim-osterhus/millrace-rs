#!/usr/bin/env python3
"""Regenerate the committed Python-derived workspace init parity fixture."""

from __future__ import annotations

import json
from pathlib import Path
from tempfile import TemporaryDirectory

import millrace_ai
from millrace_ai.workspace.baseline import load_baseline_manifest
from millrace_ai.workspace.initialization import initialize_workspace


def main() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    destination = repo_root / "tests" / "fixtures" / "workspace_init" / "python_init_reference.json"

    with TemporaryDirectory() as temp_dir:
        paths = initialize_workspace(Path(temp_dir) / "workspace")
        root = paths.root

        def rel(path: Path) -> str:
            return path.relative_to(root).as_posix()

        snapshot = json.loads(paths.runtime_snapshot_file.read_text(encoding="utf-8"))
        recovery_counters = json.loads(paths.recovery_counters_file.read_text(encoding="utf-8"))
        manifest = load_baseline_manifest(paths)

        snapshot_fields = {
            key: snapshot[key]
            for key in (
                "schema_version",
                "kind",
                "runtime_mode",
                "process_running",
                "paused",
                "pause_sources",
                "stop_requested",
                "active_mode_id",
                "execution_loop_id",
                "planning_loop_id",
                "learning_loop_id",
                "loop_ids_by_plane",
                "compiled_plan_id",
                "compiled_plan_path",
                "execution_status_marker",
                "planning_status_marker",
                "learning_status_marker",
                "status_markers_by_plane",
                "queue_depth_execution",
                "queue_depth_planning",
                "queue_depth_learning",
                "queue_depths_by_plane",
                "config_version",
                "watcher_mode",
            )
        }

        payload = {
            "source": "python millrace-ai workspace initialization reference",
            "python_package_version": millrace_ai.__version__,
            "normalized_at": "2026-04-28T00:00:00Z",
            "required_directories": [rel(path) for path in paths.directories()],
            "required_files": [
                rel(paths.outline_file),
                rel(paths.historylog_file),
                rel(paths.runtime_root / "millrace.toml"),
                rel(paths.execution_status_file),
                rel(paths.planning_status_file),
                rel(paths.learning_status_file),
                rel(paths.runtime_snapshot_file),
                rel(paths.recovery_counters_file),
                rel(paths.learning_events_file),
                rel(paths.baseline_manifest_file),
            ],
            "selected_bootstrap_files": {
                rel(paths.execution_status_file): paths.execution_status_file.read_text(encoding="utf-8"),
                rel(paths.planning_status_file): paths.planning_status_file.read_text(encoding="utf-8"),
                rel(paths.learning_status_file): paths.learning_status_file.read_text(encoding="utf-8"),
                rel(paths.runtime_root / "millrace.toml"): (paths.runtime_root / "millrace.toml").read_text(
                    encoding="utf-8"
                ),
                rel(paths.learning_events_file): paths.learning_events_file.read_text(encoding="utf-8"),
            },
            "runtime_snapshot_fields": snapshot_fields,
            "recovery_counters": recovery_counters,
            "managed_asset_families": sorted({entry.asset_family for entry in manifest.entries}),
            "representative_managed_assets": [
                "entrypoints/execution/builder.md",
                "skills/stage/execution/builder-core/SKILL.md",
                "modes/default_codex.json",
                "graphs/execution/standard.json",
                "registry/stage_kinds/execution/builder.json",
                "loops/execution/default.json",
            ],
        }

    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
