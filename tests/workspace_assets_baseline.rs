use std::fs;
use std::path::Path;

use millrace_ai::assets::RUNTIME_ASSET_FAMILIES;
use millrace_ai::workspace::{
    BaselineManifest, BaselineManifestEntry, UpgradeDisposition, apply_baseline_upgrade,
    build_baseline_manifest, build_baseline_manifest_from_source,
    deploy_runtime_assets_from_source, initialize_workspace, load_baseline_manifest,
    preview_baseline_upgrade, write_baseline_manifest,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn sha256_hex(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    let mut rendered = String::with_capacity(digest.len() * 2);
    for byte in digest {
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}

fn write_fixture(root: &Path, relative_path: &str, contents: &[u8]) {
    let path = root.join(relative_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn canonical_manifest_id(manifest: &BaselineManifest) -> String {
    #[derive(Serialize)]
    struct CanonicalPayload<'a> {
        entries: Vec<CanonicalEntry<'a>>,
        schema_version: &'a str,
        seed_package_version: &'a str,
    }

    #[derive(Serialize)]
    struct CanonicalEntry<'a> {
        asset_family: &'a str,
        original_sha256: &'a str,
        relative_path: &'a str,
    }

    let payload = CanonicalPayload {
        entries: manifest
            .entries
            .iter()
            .map(|entry| CanonicalEntry {
                asset_family: &entry.asset_family,
                original_sha256: &entry.original_sha256,
                relative_path: &entry.relative_path,
            })
            .collect(),
        schema_version: &manifest.schema_version,
        seed_package_version: &manifest.seed_package_version,
    };
    let encoded = serde_json::to_vec(&payload).unwrap();
    sha256_hex(&encoded)
}

fn manifest_entry_mut<'a>(
    manifest: &'a mut BaselineManifest,
    relative_path: &str,
) -> &'a mut BaselineManifestEntry {
    manifest
        .entries
        .iter_mut()
        .find(|entry| entry.relative_path == relative_path)
        .unwrap_or_else(|| panic!("manifest entry exists for {relative_path}"))
}

#[test]
fn packaged_baseline_manifest_is_sorted_hashed_and_deterministic() {
    let manifest = build_baseline_manifest();
    let rebuilt = build_baseline_manifest();

    assert_eq!(manifest.schema_version, "1.0");
    assert_eq!(manifest.seed_package_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(manifest.manifest_id, rebuilt.manifest_id);
    assert_eq!(manifest.manifest_id, canonical_manifest_id(&manifest));

    let mut sorted_paths: Vec<_> = manifest
        .entries
        .iter()
        .map(|entry| entry.relative_path.as_str())
        .collect();
    let actual_paths = sorted_paths.clone();
    sorted_paths.sort_unstable();
    assert_eq!(actual_paths, sorted_paths);

    for family in RUNTIME_ASSET_FAMILIES {
        assert!(
            manifest
                .entries
                .iter()
                .any(|entry| entry.asset_family == *family),
            "missing manifest family: {family}"
        );
    }

    let builder_entry = manifest
        .entry_for("entrypoints/execution/builder.md")
        .expect("builder entrypoint is packaged");
    assert_eq!(builder_entry.asset_family, "entrypoints");
    assert_eq!(
        builder_entry.original_sha256,
        sha256_hex(include_bytes!(
            "../src/assets/baseline/entrypoints/execution/builder.md"
        ))
    );
}

#[test]
fn initialize_workspace_deploys_managed_assets_and_manifest_io() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");

    let paths = initialize_workspace(&root).unwrap();

    let representative_assets = [
        "entrypoints/execution/builder.md",
        "skills/stage/execution/builder-core/SKILL.md",
        "modes/default_codex.json",
        "graphs/execution/standard.json",
        "registry/stage_kinds/execution/builder.json",
        "loops/execution/default.json",
    ];

    let manifest = load_baseline_manifest(&paths).unwrap();
    for relative_path in representative_assets {
        let deployed_path = paths.runtime_root.join(relative_path);
        assert!(
            deployed_path.is_file(),
            "missing deployed asset: {relative_path}"
        );

        let entry = manifest
            .entry_for(relative_path)
            .expect("manifest tracks representative asset");
        assert_eq!(
            entry.original_sha256,
            sha256_hex(&fs::read(&deployed_path).unwrap())
        );
    }

    let rendered_manifest = fs::read_to_string(&paths.baseline_manifest_file).unwrap();
    assert!(rendered_manifest.ends_with('\n'));

    let manifest_before = rendered_manifest;
    write_baseline_manifest(&paths, &manifest).unwrap();
    let round_tripped = load_baseline_manifest(&paths).unwrap();
    assert_eq!(round_tripped, manifest);
    assert_eq!(
        fs::read_to_string(&paths.baseline_manifest_file).unwrap(),
        manifest_before
    );

    fs::write(
        paths.runtime_root.join("entrypoints/execution/builder.md"),
        "operator edit\n",
    )
    .unwrap();
    initialize_workspace(&root).unwrap();
    assert_eq!(
        fs::read_to_string(paths.runtime_root.join("entrypoints/execution/builder.md")).unwrap(),
        "operator edit\n"
    );
    assert_eq!(load_baseline_manifest(&paths).unwrap(), manifest);
}

#[test]
fn initialized_workspace_learning_assets_match_packaged_noop_trigger_baseline() {
    let temp_dir = TempDir::new().unwrap();
    let paths = initialize_workspace(temp_dir.path().join("workspace")).unwrap();
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/assets/baseline");
    let learning_assets = [
        "graphs/learning/standard.json",
        "loops/learning/default.json",
        "modes/learning_codex.json",
        "modes/learning_pi.json",
        "registry/stage_kinds/learning/analyst.json",
        "registry/stage_kinds/learning/professor.json",
        "registry/stage_kinds/learning/curator.json",
        "entrypoints/learning/analyst.md",
        "entrypoints/learning/professor.md",
        "entrypoints/learning/curator.md",
        "skills/stage/learning/analyst-core/SKILL.md",
        "skills/stage/learning/professor-core/SKILL.md",
        "skills/stage/learning/curator-core/SKILL.md",
    ];

    for relative_path in learning_assets {
        assert_eq!(
            fs::read(paths.runtime_root.join(relative_path)).unwrap(),
            fs::read(source_root.join(relative_path)).unwrap(),
            "workspace asset drifted from packaged baseline: {relative_path}",
        );
    }

    let learning_graph: Value = serde_json::from_slice(
        &fs::read(paths.runtime_root.join("graphs/learning/standard.json")).unwrap(),
    )
    .unwrap();
    let terminal_states = learning_graph["terminal_states"].as_array().unwrap();
    assert!(terminal_states.iter().any(|state| {
        state["terminal_state_id"] == "analyst_noop" && state["terminal_class"] == "no_op"
    }));
    assert!(terminal_states.iter().any(|state| {
        state["terminal_state_id"] == "professor_noop" && state["terminal_class"] == "no_op"
    }));
    assert!(terminal_states.iter().any(|state| {
        state["terminal_state_id"] == "curator_noop" && state["terminal_class"] == "no_op"
    }));

    for mode in ["learning_codex.json", "learning_pi.json"] {
        let mode_text = fs::read_to_string(paths.runtime_root.join("modes").join(mode)).unwrap();
        assert!(!mode_text.contains("success-to-curator"));
        assert!(mode_text.contains("success-to-analyst"));
    }
}

#[test]
fn source_manifest_ignores_cache_metadata_and_normalizes_paths_without_rewriting_bytes() {
    let temp_dir = TempDir::new().unwrap();
    let source_root = temp_dir.path().join("assets");

    write_fixture(
        &source_root,
        "entrypoints/execution/first.md",
        b"first line\r\nsecond line\r\n",
    );
    write_fixture(
        &source_root,
        "entrypoints/execution\\windows-name.md",
        b"windows-style separator bytes\n",
    );
    write_fixture(&source_root, "entrypoints/.DS_Store", b"metadata");
    write_fixture(&source_root, "entrypoints/__pycache__/ignored.py", b"cache");
    write_fixture(&source_root, "entrypoints/execution/ignored.pyc", b"pyc");
    write_fixture(&source_root, "skills/.hidden/secret.md", b"hidden");
    write_fixture(
        &source_root,
        "modes/custom.json",
        b"{\"mode_id\":\"custom\"}\n",
    );

    let manifest = build_baseline_manifest_from_source(&source_root, "seed-version").unwrap();

    let paths: Vec<_> = manifest
        .entries
        .iter()
        .map(|entry| entry.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "entrypoints/execution/first.md",
            "entrypoints/execution/windows-name.md",
            "modes/custom.json",
        ]
    );
    assert_eq!(manifest.manifest_id, canonical_manifest_id(&manifest));
    assert_eq!(
        manifest
            .entry_for("entrypoints/execution/first.md")
            .unwrap()
            .original_sha256,
        sha256_hex(b"first line\r\nsecond line\r\n")
    );

    let workspace = temp_dir.path().join("workspace");
    let workspace_paths = initialize_workspace(&workspace).unwrap();
    deploy_runtime_assets_from_source(&workspace_paths, &source_root).unwrap();
    assert_eq!(
        fs::read(
            workspace_paths
                .runtime_root
                .join("entrypoints/execution/first.md")
        )
        .unwrap(),
        b"first line\r\nsecond line\r\n"
    );
    assert!(
        !workspace_paths
            .runtime_root
            .join("entrypoints/execution/ignored.pyc")
            .exists()
    );
    assert!(
        !workspace_paths
            .runtime_root
            .join("entrypoints/.DS_Store")
            .exists()
    );
}

#[test]
fn baseline_upgrade_preview_is_read_only_and_apply_updates_safe_and_missing_assets() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    let paths = initialize_workspace(&root).unwrap();
    let mut manifest = load_baseline_manifest(&paths).unwrap();
    let candidate_manifest = build_baseline_manifest();
    let safe_path = "entrypoints/execution/builder.md";
    let missing_path = "modes/default_codex.json";
    let candidate_safe_bytes = fs::read(paths.runtime_root.join(safe_path)).unwrap();
    let old_safe_bytes = b"old package builder\n";

    fs::write(paths.runtime_root.join(safe_path), old_safe_bytes).unwrap();
    manifest_entry_mut(&mut manifest, safe_path).original_sha256 = sha256_hex(old_safe_bytes);
    manifest
        .entries
        .retain(|entry| entry.relative_path != missing_path);
    fs::remove_file(paths.runtime_root.join(missing_path)).unwrap();
    manifest.manifest_id = "workspace-old-baseline".to_owned();
    write_baseline_manifest(&paths, &manifest).unwrap();

    let manifest_before = fs::read(&paths.baseline_manifest_file).unwrap();
    let safe_before = fs::read(paths.runtime_root.join(safe_path)).unwrap();
    let preview = preview_baseline_upgrade(&paths, &[]).unwrap();

    assert!(!preview.applied);
    assert_eq!(preview.baseline_manifest_id, "workspace-old-baseline");
    assert_eq!(
        preview.candidate_manifest_id,
        candidate_manifest.manifest_id
    );
    assert_eq!(
        preview.disposition_for(safe_path),
        Some(UpgradeDisposition::SafePackageUpdate)
    );
    assert_eq!(
        preview.disposition_for(missing_path),
        Some(UpgradeDisposition::Missing)
    );
    assert!(
        preview
            .counts_by_disposition()
            .contains(&(UpgradeDisposition::SafePackageUpdate, 1))
    );
    assert!(
        preview
            .counts_by_disposition()
            .contains(&(UpgradeDisposition::Missing, 1))
    );
    assert_eq!(
        fs::read(&paths.baseline_manifest_file).unwrap(),
        manifest_before
    );
    assert_eq!(
        fs::read(paths.runtime_root.join(safe_path)).unwrap(),
        safe_before
    );
    assert!(!paths.runtime_root.join(missing_path).exists());

    let applied = apply_baseline_upgrade(&paths, &[]).unwrap();

    assert!(applied.applied);
    assert_eq!(
        applied.disposition_for(safe_path),
        Some(UpgradeDisposition::SafePackageUpdate)
    );
    assert_eq!(
        fs::read(paths.runtime_root.join(safe_path)).unwrap(),
        candidate_safe_bytes
    );
    assert!(paths.runtime_root.join(missing_path).is_file());
    assert_eq!(load_baseline_manifest(&paths).unwrap(), candidate_manifest);
}

#[test]
fn baseline_upgrade_apply_refuses_conflicts_and_preserves_workspace_state() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    let paths = initialize_workspace(&root).unwrap();
    let mut manifest = load_baseline_manifest(&paths).unwrap();
    let safe_path = "entrypoints/execution/builder.md";
    let operator_bytes = b"operator edit\n";

    manifest_entry_mut(&mut manifest, safe_path).original_sha256 = sha256_hex(b"old package\n");
    manifest.manifest_id = "workspace-conflict-baseline".to_owned();
    write_baseline_manifest(&paths, &manifest).unwrap();
    fs::write(paths.runtime_root.join(safe_path), operator_bytes).unwrap();
    let manifest_before = fs::read(&paths.baseline_manifest_file).unwrap();

    let preview = preview_baseline_upgrade(&paths, &[]).unwrap();
    assert_eq!(
        preview.disposition_for(safe_path),
        Some(UpgradeDisposition::Conflict)
    );
    let error = apply_baseline_upgrade(&paths, &[]).unwrap_err();

    assert!(error.to_string().contains("upgrade conflict(s) detected"));
    assert_eq!(
        fs::read(paths.runtime_root.join(safe_path)).unwrap(),
        operator_bytes
    );
    assert_eq!(
        fs::read(&paths.baseline_manifest_file).unwrap(),
        manifest_before
    );
}

#[test]
fn baseline_upgrade_localizes_removed_assets_without_deleting_operator_content() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    let paths = initialize_workspace(&root).unwrap();
    let mut manifest = load_baseline_manifest(&paths).unwrap();
    let removed_path = "entrypoints/removed-package.md";
    let removed_bytes = b"operator keeps this removed package file\n";

    fs::write(paths.runtime_root.join(removed_path), removed_bytes).unwrap();
    manifest.entries.push(BaselineManifestEntry {
        relative_path: removed_path.to_owned(),
        asset_family: "entrypoints".to_owned(),
        original_sha256: sha256_hex(removed_bytes),
    });
    manifest.manifest_id = "workspace-removed-baseline".to_owned();
    write_baseline_manifest(&paths, &manifest).unwrap();

    let preview = preview_baseline_upgrade(&paths, &[]).unwrap();
    assert_eq!(
        preview.disposition_for(removed_path),
        Some(UpgradeDisposition::Conflict)
    );

    let localize_removed = vec![removed_path.to_owned()];
    let localized_preview = preview_baseline_upgrade(&paths, &localize_removed).unwrap();
    assert_eq!(
        localized_preview.disposition_for(removed_path),
        Some(UpgradeDisposition::LocalizedRemoved)
    );
    let error =
        preview_baseline_upgrade(&paths, &["entrypoints/not-removed.md".to_owned()]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("localize-removed path is not a removed managed asset")
    );

    apply_baseline_upgrade(&paths, &localize_removed).unwrap();

    assert_eq!(
        fs::read(paths.runtime_root.join(removed_path)).unwrap(),
        removed_bytes
    );
    assert!(
        load_baseline_manifest(&paths)
            .unwrap()
            .entry_for(removed_path)
            .is_none()
    );
}

#[test]
fn baseline_upgrade_reports_missing_or_malformed_manifest() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    let paths = initialize_workspace(&root).unwrap();

    fs::remove_file(&paths.baseline_manifest_file).unwrap();
    let missing = preview_baseline_upgrade(&paths, &[]).unwrap_err();
    assert!(missing.to_string().contains("baseline_manifest.json"));

    fs::write(&paths.baseline_manifest_file, "{not-json").unwrap();
    let malformed = preview_baseline_upgrade(&paths, &[]).unwrap_err();
    assert!(malformed.to_string().contains("baseline_manifest"));
}
