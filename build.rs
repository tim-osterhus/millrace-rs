use std::{
    env, fs,
    path::{Path, PathBuf},
};

const ASSET_FAMILIES: &[&str] = &[
    "entrypoints",
    "skills",
    "modes",
    "loops",
    "graphs",
    "registry",
];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let baseline_root = manifest_dir.join("src/assets/baseline");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let generated_path = out_dir.join("managed_assets.rs");

    println!("cargo:rerun-if-changed={}", baseline_root.display());

    let mut entries = Vec::new();
    for family in ASSET_FAMILIES {
        let family_root = baseline_root.join(family);
        if !family_root.exists() {
            continue;
        }
        collect_family_entries(&baseline_root, &family_root, &mut entries);
    }
    entries.sort();

    let mut rendered = String::from("pub static RUNTIME_ASSETS: &[RuntimeAsset] = &[\n");
    for relative_path in entries {
        let family = relative_path
            .split_once('/')
            .map(|(family, _)| family)
            .expect("asset path includes a family");
        rendered.push_str("    RuntimeAsset {\n");
        rendered.push_str(&format!("        relative_path: {:?},\n", relative_path));
        rendered.push_str(&format!("        asset_family: {:?},\n", family));
        rendered.push_str(&format!(
            "        contents: include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {:?})),\n",
            format!("/src/assets/baseline/{relative_path}")
        ));
        rendered.push_str("    },\n");
    }
    rendered.push_str("];\n");

    fs::write(generated_path, rendered).expect("write generated managed asset table");
}

fn collect_family_entries(baseline_root: &Path, directory: &Path, entries: &mut Vec<String>) {
    let mut children: Vec<_> = fs::read_dir(directory)
        .expect("read managed asset directory")
        .map(|entry| entry.expect("read managed asset entry"))
        .collect();
    children.sort_by_key(|entry| entry.path());

    for child in children {
        let path = child.path();
        if path.is_dir() {
            collect_family_entries(baseline_root, &path, entries);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let relative_path = path
            .strip_prefix(baseline_root)
            .expect("managed asset is under baseline root");
        if should_skip_runtime_asset_path(relative_path) {
            continue;
        }
        entries.push(normalize_relative_path(relative_path));
    }
}

fn should_skip_runtime_asset_path(relative_path: &Path) -> bool {
    let normalized = normalize_relative_path(relative_path);
    let parts: Vec<_> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();

    parts.iter().any(|part| part.starts_with('.'))
        || parts.contains(&"__pycache__")
        || normalized.ends_with(".pyc")
        || normalized.ends_with(".pyo")
}

fn normalize_relative_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
