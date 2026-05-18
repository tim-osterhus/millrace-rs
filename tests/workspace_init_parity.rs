mod support;

use std::fs;
use std::path::Path;

use millrace_ai::workspace::{initialize_workspace, load_baseline_manifest};
use serde_json::Value;
use support::parity::read_fixture;
use tempfile::TempDir;

fn relative_to_root(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

fn fixture() -> Value {
    serde_json::from_str(
        &read_fixture("workspace_init/python_init_reference.json")
            .expect("read Python workspace init fixture"),
    )
    .expect("parse Python workspace init fixture")
}

#[test]
fn rust_init_matches_python_required_tree_fixture() {
    let fixture = fixture();
    assert_eq!(fixture["python_package_version"], "0.19.0");

    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    let actual_directories: Vec<_> = paths
        .directories()
        .iter()
        .map(|path| relative_to_root(path, &paths.root))
        .collect();
    let expected_directories: Vec<_> = fixture["required_directories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(actual_directories, expected_directories);
    for relative_path in &expected_directories {
        assert!(
            paths.root.join(relative_path).is_dir(),
            "missing Python-required directory: {relative_path}"
        );
    }

    for value in fixture["required_files"].as_array().unwrap() {
        let relative_path = value.as_str().unwrap();
        assert!(
            paths.root.join(relative_path).is_file(),
            "missing Python-required file: {relative_path}"
        );
    }
}

#[test]
fn rust_init_matches_python_selected_bootstrap_file_fixture() {
    let fixture = fixture();
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();

    for (relative_path, expected_contents) in
        fixture["selected_bootstrap_files"].as_object().unwrap()
    {
        assert_eq!(
            fs::read_to_string(paths.root.join(relative_path)).unwrap(),
            expected_contents.as_str().unwrap(),
            "bootstrap file differs from Python fixture: {relative_path}"
        );
    }

    let snapshot: Value =
        serde_json::from_str(&fs::read_to_string(&paths.runtime_snapshot_file).unwrap()).unwrap();
    for (field, expected_value) in fixture["runtime_snapshot_fields"].as_object().unwrap() {
        assert_eq!(
            snapshot.get(field).unwrap(),
            expected_value,
            "runtime snapshot field differs from Python fixture: {field}"
        );
    }

    let recovery_counters: Value =
        serde_json::from_str(&fs::read_to_string(&paths.recovery_counters_file).unwrap()).unwrap();
    assert_eq!(recovery_counters, fixture["recovery_counters"]);
}

#[test]
fn rust_init_deploys_python_expected_managed_asset_families() {
    let fixture = fixture();
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let manifest = load_baseline_manifest(&paths).unwrap();

    let mut actual_families: Vec<_> = manifest
        .entries
        .iter()
        .map(|entry| entry.asset_family.as_str())
        .collect();
    actual_families.sort_unstable();
    actual_families.dedup();
    let expected_families: Vec<_> = fixture["managed_asset_families"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert_eq!(actual_families, expected_families);

    for value in fixture["representative_managed_assets"].as_array().unwrap() {
        let relative_path = value.as_str().unwrap();
        assert!(
            paths.runtime_root.join(relative_path).is_file(),
            "missing representative managed asset: {relative_path}"
        );
        assert!(
            manifest.entry_for(relative_path).is_some(),
            "manifest missing representative managed asset: {relative_path}"
        );
    }
}
