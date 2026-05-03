#!/usr/bin/env python3
"""Regenerate normalized Python compiler parity fixtures.

The generated fixture is intentionally normalized so ordinary Rust tests can
compare stable compiler semantics without requiring a live Python environment.
Run this script from the Rust repository root when the Python reference at
../millrace-py is intentionally refreshed.
"""

from __future__ import annotations

import contextlib
import io
import json
import re
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
PYTHON_ROOT = (REPO_ROOT / "../millrace-py").resolve()
PYTHON_SRC = PYTHON_ROOT / "src"
FIXTURE_PATH = REPO_ROOT / "tests/fixtures/compiler_parity/python_compiler_parity.json"
FIXED_COMPILED_AT = datetime(2026, 4, 28, 15, 30, 0, tzinfo=timezone.utc)
MODES = (
    "default_codex",
    "default_pi",
    "learning_codex",
    "learning_pi",
    "standard_plain",
)

sys.path.insert(0, str(PYTHON_SRC))

import millrace_ai  # noqa: E402
from millrace_ai.cli.compile_view import (  # noqa: E402
    _render_compile_diagnostics,
    _render_compile_show_lines,
)
from millrace_ai.compiler import compile_and_persist_workspace_plan  # noqa: E402
from millrace_ai.config import load_runtime_config  # noqa: E402
from millrace_ai.paths import bootstrap_workspace  # noqa: E402


def main() -> None:
    fixture = {
        "schema_version": "1.0",
        "kind": "python_compiler_parity_fixture",
        "source": {
            "package": "millrace-ai",
            "version": millrace_ai.__version__,
            "python_root": "../millrace-py",
            "contract_sources": [
                "src/millrace_ai/config/models.py",
                "src/millrace_ai/contracts/modes.py",
                "src/millrace_ai/architecture/loop_graphs.py",
                "src/millrace_ai/architecture/materialization.py",
                "src/millrace_ai/compilation/node_materialization.py",
                "src/millrace_ai/compilation/validation.py",
                "src/millrace_ai/cli/compile_view.py",
                "tests/config/test_config.py",
                "tests/assets/test_modes.py",
                "tests/assets/test_loop_graphs.py",
                "tests/integration/test_compiler.py",
            ],
        },
        "normalization": {
            "timestamps": "<timestamp>",
            "compiled_plan_ids": "<compiled_plan_id:{effective_mode_id}>",
            "compile_input_fingerprints": "<cfg-fingerprint> / <assets-fingerprint>",
            "baseline_manifest": "<baseline_manifest_id> / <package_version>",
            "paths": "runtime-root-relative with forward slashes",
            "resolved_asset_content_sha256": "<content-sha256> unless missing",
        },
        "cases": [build_case(mode) for mode in MODES],
    }

    FIXTURE_PATH.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE_PATH.write_text(json.dumps(fixture, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {FIXTURE_PATH.relative_to(REPO_ROOT)}")


def build_case(requested_mode_id: str) -> dict[str, Any]:
    with tempfile.TemporaryDirectory() as temp_dir:
        workspace_root = Path(temp_dir) / "workspace"
        paths = bootstrap_workspace(workspace_root)
        config = load_runtime_config(paths.runtime_root / "millrace.toml")
        outcome = compile_and_persist_workspace_plan(
            paths,
            config=config,
            requested_mode_id=requested_mode_id,
            assets_root=paths.runtime_root,
            now=FIXED_COMPILED_AT,
        )
        if outcome.active_plan is None:
            raise RuntimeError(f"Python compiler produced no active plan for {requested_mode_id}")

        validate_lines = render_diagnostics_lines(outcome)
        show_lines = validate_lines + list(_render_compile_show_lines(paths, outcome))
        plan = outcome.active_plan.model_dump(mode="json")

        return {
            "requested_mode_id": requested_mode_id,
            "effective_mode_id": outcome.active_plan.mode_id,
            "normalized_plan": normalize_plan(plan),
            "normalized_validate_output": normalize_cli_output(validate_lines),
            "normalized_show_output": normalize_cli_output(show_lines),
        }


def render_diagnostics_lines(outcome: Any) -> list[str]:
    stdout = io.StringIO()
    with contextlib.redirect_stdout(stdout):
        exit_code = _render_compile_diagnostics(outcome)
    if exit_code != 0:
        raise RuntimeError(f"unexpected Python compile diagnostics exit code: {exit_code}")
    return stdout.getvalue().splitlines()


def normalize_plan(value: Any, key: str | None = None, mode_id: str | None = None) -> Any:
    if isinstance(value, dict):
        object_mode_id = value.get("mode_id") if isinstance(value.get("mode_id"), str) else mode_id
        normalized: dict[str, Any] = {}
        for child_key, child_value in value.items():
            normalized[child_key] = normalize_plan(child_value, child_key, object_mode_id)
        if isinstance(normalized.get("resolved_assets"), list):
            normalized["resolved_assets"] = sorted(
                normalized["resolved_assets"],
                key=lambda item: (
                    item.get("asset_family", ""),
                    item.get("logical_id", ""),
                    item.get("compile_time_path", ""),
                ),
            )
        return normalized

    if isinstance(value, list) or isinstance(value, tuple):
        return [normalize_plan(item, key, mode_id) for item in value]

    if key in {"compiled_at", "emitted_at"}:
        return "<timestamp>"
    if key == "compiled_plan_id":
        return f"<compiled_plan_id:{mode_id or 'unknown'}>"
    if key == "config_fingerprint":
        return "<cfg-fingerprint>"
    if key == "assets_fingerprint":
        return "<assets-fingerprint>"
    if key == "compile_time_path" and isinstance(value, str):
        return normalize_runtime_path(value)
    if key == "content_sha256" and isinstance(value, str) and value != "missing":
        return "<content-sha256>"

    return value


def normalize_cli_output(lines: list[str]) -> dict[str, Any]:
    diagnostics: dict[str, Any] = {}
    show: dict[str, Any] = {}
    entries: list[str] = []
    completion_behavior: dict[str, Any] = {}
    stages: list[dict[str, Any]] = []
    current_stage: dict[str, Any] | None = None
    in_show = False

    for line in lines:
        if (
            line.startswith("loop_id: ")
            or line.startswith("node_order: ")
            or line.startswith("learning_triggers: ")
            or line.startswith("learning_trigger")
            or line.startswith("concurrency_policy")
        ):
            continue

        if line.startswith("entry: "):
            entries.append(line)
            continue

        if line.startswith("completion: "):
            show["completion"] = line.removeprefix("completion: ")
            continue

        if ": " not in line:
            continue
        key, raw_value = line.split(": ", 1)
        value = normalize_cli_value(key, raw_value)

        if key == "compiled_plan_currentness":
            in_show = True
            show[key] = value
            continue

        if key.startswith("completion_behavior."):
            completion_behavior[key] = value
            continue

        if key == "stage":
            if current_stage is not None:
                stages.append(current_stage)
            current_stage = {"stage": raw_value}
            continue

        if key in STAGE_FIELDS:
            if current_stage is not None:
                current_stage[key] = value
            continue

        if key in DIAGNOSTIC_FIELDS and (not in_show or not key.startswith("compile_input.")):
            diagnostics[key] = value
            continue

        if key in SHOW_FIELDS:
            show[key] = value
            continue

    if current_stage is not None:
        stages.append(current_stage)

    result: dict[str, Any] = {"diagnostics": diagnostics}
    if show:
        show["entries"] = sorted(entries)
        if completion_behavior:
            show["completion_behavior"] = completion_behavior
        if stages:
            show["stages"] = sorted(stages, key=lambda item: item["stage"])
        result["show"] = show
    return result


DIAGNOSTIC_FIELDS = {
    "ok",
    "mode_id",
    "used_last_known_good",
    "compile_input.mode_id",
    "compile_input.config_fingerprint",
    "compile_input.assets_fingerprint",
}

SHOW_FIELDS = {
    "execution_loop_id",
    "planning_loop_id",
    "learning_loop_id",
    "compiled_plan_id",
    "baseline_manifest_id",
    "baseline_seed_package_version",
    "compile_input.mode_id",
    "compile_input.config_fingerprint",
    "compile_input.assets_fingerprint",
    "persisted_compile_input.mode_id",
    "persisted_compile_input.config_fingerprint",
    "persisted_compile_input.assets_fingerprint",
}

STAGE_FIELDS = {
    "stage_kind_id",
    "running_status_marker",
    "entrypoint_path",
    "entrypoint_contract_id",
    "required_skills",
    "attached_skills",
    "runner_name",
    "model_name",
    "thinking_level",
    "model_reasoning_effort",
    "timeout_seconds",
}


def normalize_cli_value(key: str, value: str) -> str:
    if key == "compiled_plan_id":
        return normalize_compiled_plan_id(value)
    if key in {
        "compile_input.config_fingerprint",
        "persisted_compile_input.config_fingerprint",
    }:
        return "<cfg-fingerprint>"
    if key in {
        "compile_input.assets_fingerprint",
        "persisted_compile_input.assets_fingerprint",
    }:
        return "<assets-fingerprint>"
    if key == "baseline_manifest_id":
        return "<baseline_manifest_id>"
    if key == "baseline_seed_package_version":
        return "<package_version>"
    if key == "entrypoint_path":
        return normalize_runtime_path(value)
    if key in {"required_skills", "attached_skills"}:
        return ", ".join(normalize_runtime_path(part.strip()) for part in value.split(", "))
    return value


def normalize_compiled_plan_id(value: str) -> str:
    match = re.fullmatch(r"plan-(?P<mode>.+)-[0-9a-f]{12}", value)
    if not match:
        return "<compiled_plan_id:unknown>"
    return f"<compiled_plan_id:{match.group('mode')}>"


def normalize_runtime_path(value: str) -> str:
    normalized = value.replace("\\", "/")
    if normalized.startswith("millrace-agents/"):
        normalized = normalized.removeprefix("millrace-agents/")
    return normalized


if __name__ == "__main__":
    main()
