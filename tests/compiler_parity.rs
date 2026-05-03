mod support;

use std::fs;

use serde_json::{Map, Value};
use support::parity::{fixture_path, read_fixture, run_rust_millrace};
use tempfile::TempDir;

const CASES: [(&str, &str); 5] = [
    ("default_codex", "default_codex"),
    ("default_pi", "default_pi"),
    ("learning_codex", "learning_codex"),
    ("learning_pi", "learning_pi"),
    ("standard_plain", "default_codex"),
];

#[test]
fn rust_compiler_matches_python_normalized_plan_and_cli_fixtures() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("compiler_parity/python_compiler_parity.json")
            .expect("read compiler parity fixture"),
    )
    .expect("parse compiler parity fixture");

    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["kind"], "python_compiler_parity_fixture");

    for (requested_mode_id, expected_effective_mode_id) in CASES {
        let expected = fixture_case(&fixture, requested_mode_id);
        assert_eq!(expected["effective_mode_id"], expected_effective_mode_id);

        let validate_workspace = TempDir::new().expect("create validate workspace");
        let validate_root = validate_workspace.path().join("workspace");
        init_workspace(&validate_root);
        let validate = run_rust_millrace([
            "compile",
            "validate",
            "--workspace",
            validate_root.to_str().unwrap(),
            "--mode",
            requested_mode_id,
        ])
        .expect("run Rust compile validate");
        validate.assert_success();
        assert_eq!(validate.stderr, "");
        assert_eq!(
            normalize_cli_output(&validate.stdout),
            expected["normalized_validate_output"],
            "normalized compile validate output drifted for {requested_mode_id}",
        );

        let show_workspace = TempDir::new().expect("create show workspace");
        let show_root = show_workspace.path().join("workspace");
        init_workspace(&show_root);
        let show = run_rust_millrace([
            "compile",
            "show",
            "--workspace",
            show_root.to_str().unwrap(),
            "--mode",
            requested_mode_id,
        ])
        .expect("run Rust compile show");
        show.assert_success();
        assert_eq!(show.stderr, "");
        assert_eq!(
            normalize_cli_output(&show.stdout),
            expected["normalized_show_output"],
            "normalized compile show output drifted for {requested_mode_id}",
        );

        let persisted_plan_path = show_root
            .join("millrace-agents")
            .join("state")
            .join("compiled_plan.json");
        let persisted_plan: Value = serde_json::from_str(
            &fs::read_to_string(&persisted_plan_path).expect("read persisted compiled plan"),
        )
        .expect("parse persisted compiled plan");
        assert_fingerprint_shapes(&persisted_plan);
        assert_eq!(
            normalize_plan(persisted_plan),
            expected["normalized_plan"],
            "normalized compiled_plan.json drifted for {requested_mode_id}",
        );
    }
}

#[test]
fn compiler_parity_fixture_documents_regeneration_surface() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("compiler_parity/python_compiler_parity.json")
            .expect("read compiler parity fixture"),
    )
    .expect("parse compiler parity fixture");
    assert_eq!(fixture["source"]["version"], "0.17.3");
    for source_path in [
        "src/millrace_ai/config/models.py",
        "src/millrace_ai/contracts/modes.py",
        "src/millrace_ai/compilation/node_materialization.py",
        "src/millrace_ai/cli/compile_view.py",
        "tests/config/test_config.py",
        "tests/assets/test_modes.py",
        "tests/integration/test_compiler.py",
    ] {
        assert!(
            fixture["source"]["contract_sources"]
                .as_array()
                .expect("fixture source paths must be an array")
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "compiler parity fixture does not name Python source {source_path}",
        );
    }

    assert!(fixture_path("compiler_parity/README.md").is_file());
    assert!(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/support/generate_python_compiler_parity_fixtures.py")
            .is_file()
    );
}

fn init_workspace(root: &std::path::Path) {
    run_rust_millrace(["init", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace init")
        .assert_success();
}

fn fixture_case<'a>(fixture: &'a Value, requested_mode_id: &str) -> &'a Value {
    fixture["cases"]
        .as_array()
        .expect("fixture cases must be an array")
        .iter()
        .find(|case| case["requested_mode_id"] == requested_mode_id)
        .unwrap_or_else(|| panic!("missing compiler parity fixture case for {requested_mode_id}"))
}

fn normalize_plan(value: Value) -> Value {
    normalize_plan_value(value, None, None)
}

fn normalize_plan_value(value: Value, key: Option<&str>, mode_id: Option<String>) -> Value {
    match value {
        Value::Object(map) => {
            let object_mode_id = map
                .get("mode_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or(mode_id);
            let mut normalized = Map::new();
            for (child_key, child_value) in map {
                let mut child =
                    normalize_plan_value(child_value, Some(&child_key), object_mode_id.clone());
                if child_key == "compiled_plan_id" {
                    child = Value::String(format!(
                        "<compiled_plan_id:{}>",
                        object_mode_id.as_deref().unwrap_or("unknown")
                    ));
                }
                if child_key == "resolved_assets" {
                    if let Value::Array(mut assets) = child {
                        assets.sort_by_key(asset_sort_key);
                        child = Value::Array(assets);
                    }
                }
                normalized.insert(child_key, child);
            }
            Value::Object(normalized)
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|item| normalize_plan_value(item, key, mode_id.clone()))
                .collect(),
        ),
        Value::String(text) => match key {
            Some("compiled_at" | "emitted_at") => Value::String("<timestamp>".to_owned()),
            Some("config_fingerprint") => Value::String("<cfg-fingerprint>".to_owned()),
            Some("assets_fingerprint") => Value::String("<assets-fingerprint>".to_owned()),
            Some("content_sha256") if text != "missing" => {
                Value::String("<content-sha256>".to_owned())
            }
            Some("compile_time_path") => Value::String(normalize_runtime_path(&text)),
            _ => Value::String(text),
        },
        other => other,
    }
}

fn asset_sort_key(value: &Value) -> String {
    format!(
        "{}\0{}\0{}",
        value["asset_family"].as_str().unwrap_or_default(),
        value["logical_id"].as_str().unwrap_or_default(),
        value["compile_time_path"].as_str().unwrap_or_default(),
    )
}

fn normalize_cli_output(stdout: &str) -> Value {
    let mut diagnostics = Map::new();
    let mut show = Map::new();
    let mut entries = Vec::new();
    let mut completion_behavior = Map::new();
    let mut stages: Vec<Value> = Vec::new();
    let mut current_stage: Option<Map<String, Value>> = None;
    let mut in_show = false;

    for line in stdout.lines() {
        if line.starts_with("loop_id: ")
            || line.starts_with("node_order: ")
            || line.starts_with("learning_triggers: ")
            || line.starts_with("learning_trigger")
            || line.starts_with("concurrency_policy")
        {
            continue;
        }

        if line.starts_with("entry: ") {
            entries.push(Value::String(line.to_owned()));
            continue;
        }

        if let Some(completion) = line.strip_prefix("completion: ") {
            show.insert(
                "completion".to_owned(),
                Value::String(completion.to_owned()),
            );
            continue;
        }

        let Some((key, raw_value)) = line.split_once(": ") else {
            continue;
        };
        let value = normalize_cli_value(key, raw_value);

        if key == "compiled_plan_currentness" {
            in_show = true;
            show.insert(key.to_owned(), value);
            continue;
        }

        if key.starts_with("completion_behavior.") {
            completion_behavior.insert(key.to_owned(), value);
            continue;
        }

        if key == "stage" {
            if let Some(stage) = current_stage.take() {
                stages.push(Value::Object(stage));
            }
            let mut stage = Map::new();
            stage.insert("stage".to_owned(), Value::String(raw_value.to_owned()));
            current_stage = Some(stage);
            continue;
        }

        if is_stage_field(key) {
            if let Some(stage) = current_stage.as_mut() {
                stage.insert(key.to_owned(), value);
            }
            continue;
        }

        if is_diagnostic_field(key) && (!in_show || !key.starts_with("compile_input.")) {
            diagnostics.insert(key.to_owned(), value);
            continue;
        }

        if is_show_field(key) {
            show.insert(key.to_owned(), value);
            continue;
        }
    }

    if let Some(stage) = current_stage {
        stages.push(Value::Object(stage));
    }
    entries.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    stages.sort_by(|left, right| left["stage"].as_str().cmp(&right["stage"].as_str()));

    let mut result = Map::new();
    result.insert("diagnostics".to_owned(), Value::Object(diagnostics));
    if !show.is_empty() {
        show.insert("entries".to_owned(), Value::Array(entries));
        if !completion_behavior.is_empty() {
            show.insert(
                "completion_behavior".to_owned(),
                Value::Object(completion_behavior),
            );
        }
        if !stages.is_empty() {
            show.insert("stages".to_owned(), Value::Array(stages));
        }
        result.insert("show".to_owned(), Value::Object(show));
    }
    Value::Object(result)
}

fn normalize_cli_value(key: &str, value: &str) -> Value {
    let normalized = match key {
        "compiled_plan_id" => normalize_compiled_plan_id(value),
        "compile_input.config_fingerprint" | "persisted_compile_input.config_fingerprint" => {
            "<cfg-fingerprint>".to_owned()
        }
        "compile_input.assets_fingerprint" | "persisted_compile_input.assets_fingerprint" => {
            "<assets-fingerprint>".to_owned()
        }
        "baseline_manifest_id" => "<baseline_manifest_id>".to_owned(),
        "baseline_seed_package_version" => "<package_version>".to_owned(),
        "entrypoint_path" => normalize_runtime_path(value),
        "required_skills" | "attached_skills" => value
            .split(", ")
            .map(normalize_runtime_path)
            .collect::<Vec<_>>()
            .join(", "),
        _ => value.to_owned(),
    };
    Value::String(normalized)
}

fn normalize_compiled_plan_id(value: &str) -> String {
    let Some(remainder) = value.strip_prefix("plan-") else {
        return "<compiled_plan_id:unknown>".to_owned();
    };
    let Some((mode_id, digest)) = remainder.rsplit_once('-') else {
        return "<compiled_plan_id:unknown>".to_owned();
    };
    if digest.len() == 12
        && digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        format!("<compiled_plan_id:{mode_id}>")
    } else {
        "<compiled_plan_id:unknown>".to_owned()
    }
}

fn normalize_runtime_path(value: &str) -> String {
    let normalized = value.replace('\\', "/");
    normalized
        .strip_prefix("millrace-agents/")
        .unwrap_or(&normalized)
        .to_owned()
}

fn is_diagnostic_field(key: &str) -> bool {
    matches!(
        key,
        "ok" | "mode_id"
            | "used_last_known_good"
            | "compile_input.mode_id"
            | "compile_input.config_fingerprint"
            | "compile_input.assets_fingerprint"
    )
}

fn is_show_field(key: &str) -> bool {
    matches!(
        key,
        "execution_loop_id"
            | "planning_loop_id"
            | "learning_loop_id"
            | "compiled_plan_id"
            | "baseline_manifest_id"
            | "baseline_seed_package_version"
            | "compile_input.mode_id"
            | "compile_input.config_fingerprint"
            | "compile_input.assets_fingerprint"
            | "persisted_compile_input.mode_id"
            | "persisted_compile_input.config_fingerprint"
            | "persisted_compile_input.assets_fingerprint"
    )
}

fn is_stage_field(key: &str) -> bool {
    matches!(
        key,
        "stage_kind_id"
            | "running_status_marker"
            | "entrypoint_path"
            | "entrypoint_contract_id"
            | "required_skills"
            | "attached_skills"
            | "runner_name"
            | "model_name"
            | "thinking_level"
            | "model_reasoning_effort"
            | "timeout_seconds"
    )
}

fn assert_fingerprint_shapes(plan: &Value) {
    let fingerprint = &plan["compile_input_fingerprint"];
    let mode_id = fingerprint["mode_id"]
        .as_str()
        .expect("compile input mode id must be a string");
    assert_eq!(mode_id, plan["mode_id"].as_str().unwrap());
    assert!(
        fingerprint["config_fingerprint"]
            .as_str()
            .is_some_and(|value| value.starts_with("cfg-") && value.len() == 16),
        "config fingerprint shape drifted: {fingerprint:?}",
    );
    assert!(
        fingerprint["assets_fingerprint"]
            .as_str()
            .is_some_and(|value| value.starts_with("assets-") && value.len() == 19),
        "assets fingerprint shape drifted: {fingerprint:?}",
    );
    assert!(
        plan["resolved_assets"]
            .as_array()
            .expect("resolved_assets must be an array")
            .iter()
            .all(|asset| asset["content_sha256"]
                .as_str()
                .is_some_and(|value| value == "missing"
                    || (value.len() == 64
                        && value.chars().all(|character| character.is_ascii_hexdigit())))),
        "resolved asset content fingerprints drifted",
    );
}
