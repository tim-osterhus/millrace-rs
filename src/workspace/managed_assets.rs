use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::assets;

use super::{WorkspaceError, WorkspacePaths, WorkspaceResult};

const BASELINE_SCHEMA_VERSION: &str = "1.0";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// One file recorded in the managed baseline manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineManifestEntry {
    /// Runtime-root-relative managed file path using `/` separators.
    pub relative_path: String,
    /// Top-level managed asset family.
    pub asset_family: String,
    /// SHA-256 of the originally deployed package bytes.
    pub original_sha256: String,
}

/// Manifest describing the package-managed baseline deployed into a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineManifest {
    /// Manifest schema version.
    pub schema_version: String,
    /// Deterministic hash identity for this manifest payload.
    pub manifest_id: String,
    /// Runtime package version that seeded the manifest.
    pub seed_package_version: String,
    /// Sorted manifest entries.
    pub entries: Vec<BaselineManifestEntry>,
}

/// Managed asset disposition reported by `millrace upgrade`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeDisposition {
    /// Workspace file still matches the original baseline and current package.
    Unchanged,
    /// Workspace file matches the original baseline and can be replaced by the package candidate.
    SafePackageUpdate,
    /// Workspace file was edited locally while the package candidate did not change.
    LocalOnlyModification,
    /// Workspace file already matches the package candidate.
    AlreadyConverged,
    /// Package removed the managed file and the operator requested localization.
    LocalizedRemoved,
    /// Workspace state cannot be updated safely without operator action.
    Conflict,
    /// Managed package file is absent from the workspace and can be restored.
    Missing,
}

impl UpgradeDisposition {
    /// Python-compatible rendering order for upgrade counts.
    pub const ALL: [Self; 7] = [
        Self::Unchanged,
        Self::SafePackageUpdate,
        Self::LocalOnlyModification,
        Self::AlreadyConverged,
        Self::LocalizedRemoved,
        Self::Conflict,
        Self::Missing,
    ];

    /// Stable CLI value for this disposition.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::SafePackageUpdate => "safe_package_update",
            Self::LocalOnlyModification => "local_only_modification",
            Self::AlreadyConverged => "already_converged",
            Self::LocalizedRemoved => "localized_removed",
            Self::Conflict => "conflict",
            Self::Missing => "missing",
        }
    }
}

/// One managed asset considered by an upgrade preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineUpgradeEntry {
    /// Runtime-root-relative managed file path using `/` separators.
    pub relative_path: String,
    /// Top-level managed asset family.
    pub asset_family: String,
    /// Upgrade classification.
    pub disposition: UpgradeDisposition,
    /// Hash from the workspace baseline manifest, if the path was previously managed.
    pub original_sha256: Option<String>,
    /// Hash of the current workspace file, when it exists as a regular file.
    pub current_sha256: Option<String>,
    /// Hash from the embedded package candidate manifest, if the path is still managed.
    pub candidate_sha256: Option<String>,
}

/// Result of comparing or applying a managed baseline upgrade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineUpgradePreview {
    /// True when safe changes were applied.
    pub applied: bool,
    /// Manifest id currently recorded by the workspace.
    pub baseline_manifest_id: String,
    /// Manifest id for the embedded package candidate.
    pub candidate_manifest_id: String,
    /// Sorted entry-level dispositions.
    pub entries: Vec<BaselineUpgradeEntry>,
}

impl BaselineUpgradePreview {
    /// Return counts in Python-compatible disposition order.
    #[must_use]
    pub fn counts_by_disposition(&self) -> Vec<(UpgradeDisposition, usize)> {
        UpgradeDisposition::ALL
            .iter()
            .copied()
            .map(|disposition| {
                let count = self
                    .entries
                    .iter()
                    .filter(|entry| entry.disposition == disposition)
                    .count();
                (disposition, count)
            })
            .collect()
    }

    /// Return the disposition for a runtime-root-relative path.
    #[must_use]
    pub fn disposition_for(&self, relative_path: &str) -> Option<UpgradeDisposition> {
        self.entries
            .iter()
            .find(|entry| entry.relative_path == relative_path)
            .map(|entry| entry.disposition)
    }
}

impl BaselineManifest {
    fn new(
        seed_package_version: impl Into<String>,
        mut entries: Vec<BaselineManifestEntry>,
    ) -> Self {
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        let seed_package_version = seed_package_version.into();
        let manifest_id =
            manifest_id_for_entries(BASELINE_SCHEMA_VERSION, &seed_package_version, &entries);

        Self {
            schema_version: BASELINE_SCHEMA_VERSION.to_owned(),
            manifest_id,
            seed_package_version,
            entries,
        }
    }

    /// Returns the manifest entry for a runtime-root-relative path.
    #[must_use]
    pub fn entry_for(&self, relative_path: &str) -> Option<&BaselineManifestEntry> {
        self.entries
            .iter()
            .find(|entry| entry.relative_path == relative_path)
    }
}

/// Build a baseline manifest from the embedded package-managed asset source.
#[must_use]
pub fn build_baseline_manifest() -> BaselineManifest {
    let entries = assets::runtime_assets()
        .iter()
        .map(|asset| BaselineManifestEntry {
            relative_path: asset.relative_path.to_owned(),
            asset_family: asset.asset_family.to_owned(),
            original_sha256: sha256_hex(asset.contents),
        })
        .collect();

    BaselineManifest::new(env!("CARGO_PKG_VERSION"), entries)
}

/// Build a baseline manifest from a filesystem asset source root.
pub fn build_baseline_manifest_from_source(
    source_root: impl AsRef<Path>,
    seed_package_version: impl Into<String>,
) -> WorkspaceResult<BaselineManifest> {
    let source_root = source_root.as_ref();
    let mut entries = Vec::new();

    for family in assets::RUNTIME_ASSET_FAMILIES {
        let family_root = source_root.join(family);
        if !family_root.exists() {
            continue;
        }
        collect_source_manifest_entries(source_root, &family_root, &mut entries)?;
    }

    Ok(BaselineManifest::new(seed_package_version, entries))
}

/// Preview a managed baseline upgrade against the embedded package assets.
pub fn preview_baseline_upgrade(
    paths: &WorkspacePaths,
    localize_removed_paths: &[String],
) -> WorkspaceResult<BaselineUpgradePreview> {
    let baseline_manifest = load_baseline_manifest(paths)?;
    let candidate_manifest = build_baseline_manifest();
    preview_baseline_upgrade_with_candidate(
        paths,
        &baseline_manifest,
        &candidate_manifest,
        localize_removed_paths,
    )
}

/// Apply safe managed baseline changes from the embedded package assets.
pub fn apply_baseline_upgrade(
    paths: &WorkspacePaths,
    localize_removed_paths: &[String],
) -> WorkspaceResult<BaselineUpgradePreview> {
    let preview = preview_baseline_upgrade(paths, localize_removed_paths)?;
    let conflicts: Vec<_> = preview
        .entries
        .iter()
        .filter(|entry| entry.disposition == UpgradeDisposition::Conflict)
        .map(|entry| entry.relative_path.as_str())
        .collect();
    if !conflicts.is_empty() {
        return Err(WorkspaceError::Upgrade {
            message: format!("upgrade conflict(s) detected: {}", conflicts.join(", ")),
        });
    }

    for entry in &preview.entries {
        if !matches!(
            entry.disposition,
            UpgradeDisposition::SafePackageUpdate | UpgradeDisposition::Missing
        ) {
            continue;
        }
        let contents = embedded_asset_contents(&entry.relative_path).ok_or_else(|| {
            WorkspaceError::Upgrade {
                message: format!(
                    "candidate managed asset is missing: {}",
                    entry.relative_path
                ),
            }
        })?;
        let destination = runtime_relative_path(paths, &entry.relative_path)?;
        write_bytes_atomically(&destination, contents)?;
    }

    write_baseline_manifest(paths, &build_baseline_manifest())?;
    Ok(BaselineUpgradePreview {
        applied: true,
        baseline_manifest_id: preview.baseline_manifest_id,
        candidate_manifest_id: preview.candidate_manifest_id,
        entries: preview.entries,
    })
}

/// Copy missing embedded managed runtime assets into an initialized workspace.
pub fn deploy_runtime_assets(paths: &WorkspacePaths) -> WorkspaceResult<()> {
    for asset in assets::runtime_assets() {
        let destination = paths.runtime_root.join(asset.relative_path);
        write_bytes_if_missing(&destination, asset.contents)?;
    }

    Ok(())
}

/// Copy missing managed runtime assets from a filesystem asset source root.
pub fn deploy_runtime_assets_from_source(
    paths: &WorkspacePaths,
    source_root: impl AsRef<Path>,
) -> WorkspaceResult<()> {
    let source_root = source_root.as_ref();

    for family in assets::RUNTIME_ASSET_FAMILIES {
        let family_root = source_root.join(family);
        if !family_root.exists() {
            continue;
        }
        deploy_source_family(paths, &family_root, &family_root, family)?;
    }

    Ok(())
}

/// Read a deployed baseline manifest from `millrace-agents/state/baseline_manifest.json`.
pub fn load_baseline_manifest(paths: &WorkspacePaths) -> WorkspaceResult<BaselineManifest> {
    let raw = fs::read_to_string(&paths.baseline_manifest_file)
        .map_err(|error| WorkspaceError::io(&paths.baseline_manifest_file, error))?;
    serde_json::from_str(&raw).map_err(|error| WorkspaceError::Json {
        artifact: "baseline_manifest",
        message: error.to_string(),
    })
}

/// Write a deployed baseline manifest to `millrace-agents/state/baseline_manifest.json`.
pub fn write_baseline_manifest(
    paths: &WorkspacePaths,
    manifest: &BaselineManifest,
) -> WorkspaceResult<PathBuf> {
    let rendered = serde_json::to_string_pretty(manifest)
        .map(|mut rendered| {
            rendered.push('\n');
            rendered
        })
        .map_err(|error| WorkspaceError::Json {
            artifact: "baseline_manifest",
            message: error.to_string(),
        })?;

    write_text_atomically(&paths.baseline_manifest_file, &rendered)?;

    Ok(paths.baseline_manifest_file.clone())
}

/// Return true for local/cache paths that must never become managed assets.
#[must_use]
pub fn should_skip_runtime_asset_path(relative_path: impl AsRef<Path>) -> bool {
    let normalized = normalize_path_lossy(relative_path.as_ref());
    let parts: Vec<_> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();

    parts.iter().any(|part| part.starts_with('.'))
        || parts.iter().any(|part| *part == "__pycache__")
        || normalized.ends_with(".pyc")
        || normalized.ends_with(".pyo")
}

fn collect_source_manifest_entries(
    source_root: &Path,
    directory: &Path,
    entries: &mut Vec<BaselineManifestEntry>,
) -> WorkspaceResult<()> {
    let mut children = read_dir_sorted(directory)?;

    for child in children.drain(..) {
        let path = child.path();
        if path.is_dir() {
            collect_source_manifest_entries(source_root, &path, entries)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }

        let relative_path = path.strip_prefix(source_root).map_err(|_| {
            invalid_path(
                &path,
                "asset file is not under the managed asset source root",
            )
        })?;
        if should_skip_runtime_asset_path(relative_path) {
            continue;
        }

        let relative_path = normalize_manifest_relative_path(relative_path)?;
        let asset_family = relative_path
            .split_once('/')
            .map(|(family, _)| family.to_owned())
            .ok_or_else(|| {
                invalid_path(relative_path.as_str(), "asset path must include a family")
            })?;
        let bytes = fs::read(&path).map_err(|error| WorkspaceError::io(&path, error))?;
        entries.push(BaselineManifestEntry {
            relative_path,
            asset_family,
            original_sha256: sha256_hex(&bytes),
        });
    }

    Ok(())
}

fn deploy_source_family(
    paths: &WorkspacePaths,
    family_root: &Path,
    directory: &Path,
    family: &str,
) -> WorkspaceResult<()> {
    let mut children = read_dir_sorted(directory)?;

    for child in children.drain(..) {
        let path = child.path();
        if path.is_dir() {
            deploy_source_family(paths, family_root, &path, family)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }

        let relative_to_family = path.strip_prefix(family_root).map_err(|_| {
            invalid_path(
                &path,
                "asset file is not under the managed asset family root",
            )
        })?;
        if should_skip_runtime_asset_path(relative_to_family) {
            continue;
        }
        let destination_relative =
            normalize_manifest_relative_path(Path::new(family).join(relative_to_family))?;
        let bytes = fs::read(&path).map_err(|error| WorkspaceError::io(&path, error))?;
        write_bytes_if_missing(&paths.runtime_root.join(destination_relative), &bytes)?;
    }

    Ok(())
}

fn read_dir_sorted(directory: &Path) -> WorkspaceResult<Vec<fs::DirEntry>> {
    let mut children: Vec<_> = fs::read_dir(directory)
        .map_err(|error| WorkspaceError::io(directory, error))?
        .collect::<Result<_, _>>()
        .map_err(|error| WorkspaceError::io(directory, error))?;
    children.sort_by_key(|entry| entry.path());
    Ok(children)
}

fn write_bytes_if_missing(path: &Path, contents: &[u8]) -> WorkspaceResult<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| WorkspaceError::io(parent, error))?;
    }
    fs::write(path, contents).map_err(|error| WorkspaceError::io(path, error))
}

fn preview_baseline_upgrade_with_candidate(
    paths: &WorkspacePaths,
    baseline_manifest: &BaselineManifest,
    candidate_manifest: &BaselineManifest,
    localize_removed_paths: &[String],
) -> WorkspaceResult<BaselineUpgradePreview> {
    let original_by_path = manifest_entries_by_path(baseline_manifest)?;
    let candidate_by_path = manifest_entries_by_path(candidate_manifest)?;
    let localize_removed_set = normalize_localize_removed_paths(localize_removed_paths)?;
    let mut paths_union: BTreeSet<String> = original_by_path.keys().cloned().collect();
    paths_union.extend(candidate_by_path.keys().cloned());

    let mut entries = Vec::new();
    let mut removed_original_paths = BTreeSet::new();
    for relative_path in paths_union {
        let original_entry = original_by_path.get(&relative_path);
        let candidate_entry = candidate_by_path.get(&relative_path);
        let current_path = runtime_relative_path(paths, &relative_path)?;
        let current_sha256 = current_file_sha256(&current_path)?;

        let disposition = if current_path.exists() && !current_path.is_file() {
            UpgradeDisposition::Conflict
        } else if original_entry.is_none() {
            let candidate_entry =
                candidate_entry.expect("candidate entry exists for candidate-only path");
            match current_sha256.as_deref() {
                None => UpgradeDisposition::Missing,
                Some(current) if current == candidate_entry.original_sha256 => {
                    UpgradeDisposition::AlreadyConverged
                }
                Some(_) => UpgradeDisposition::Conflict,
            }
        } else if candidate_entry.is_none() {
            removed_original_paths.insert(relative_path.clone());
            if localize_removed_set.contains(&relative_path) {
                UpgradeDisposition::LocalizedRemoved
            } else {
                UpgradeDisposition::Conflict
            }
        } else if current_sha256.is_none() {
            UpgradeDisposition::Missing
        } else {
            let original_sha256 = &original_entry.expect("checked above").original_sha256;
            let candidate_sha256 = &candidate_entry.expect("checked above").original_sha256;
            let current_sha256 = current_sha256.as_ref().expect("checked above");
            if current_sha256 == original_sha256 && original_sha256 == candidate_sha256 {
                UpgradeDisposition::Unchanged
            } else if current_sha256 == original_sha256 && candidate_sha256 != original_sha256 {
                UpgradeDisposition::SafePackageUpdate
            } else if current_sha256 != original_sha256 && candidate_sha256 == original_sha256 {
                UpgradeDisposition::LocalOnlyModification
            } else if current_sha256 == candidate_sha256 && candidate_sha256 != original_sha256 {
                UpgradeDisposition::AlreadyConverged
            } else {
                UpgradeDisposition::Conflict
            }
        };

        let asset_family = candidate_entry
            .or(original_entry)
            .map(|entry| entry.asset_family.clone())
            .unwrap_or_else(|| {
                relative_path
                    .split_once('/')
                    .map(|(family, _)| family.to_owned())
                    .unwrap_or_default()
            });
        entries.push(BaselineUpgradeEntry {
            relative_path,
            asset_family,
            disposition,
            original_sha256: original_entry.map(|entry| entry.original_sha256.clone()),
            current_sha256,
            candidate_sha256: candidate_entry.map(|entry| entry.original_sha256.clone()),
        });
    }

    let invalid_localize_paths: Vec<_> = localize_removed_set
        .difference(&removed_original_paths)
        .cloned()
        .collect();
    if !invalid_localize_paths.is_empty() {
        return Err(WorkspaceError::Upgrade {
            message: format!(
                "localize-removed path is not a removed managed asset: {}",
                invalid_localize_paths.join(", ")
            ),
        });
    }

    Ok(BaselineUpgradePreview {
        applied: false,
        baseline_manifest_id: baseline_manifest.manifest_id.clone(),
        candidate_manifest_id: candidate_manifest.manifest_id.clone(),
        entries,
    })
}

fn manifest_entries_by_path(
    manifest: &BaselineManifest,
) -> WorkspaceResult<BTreeMap<String, BaselineManifestEntry>> {
    let mut entries = BTreeMap::new();
    for entry in &manifest.entries {
        validate_manifest_entry_path(entry)?;
        entries.insert(entry.relative_path.clone(), entry.clone());
    }
    Ok(entries)
}

fn validate_manifest_entry_path(entry: &BaselineManifestEntry) -> WorkspaceResult<()> {
    let normalized = normalize_manifest_relative_path(&entry.relative_path)?;
    if normalized != entry.relative_path {
        return Err(invalid_path(
            &entry.relative_path,
            "manifest asset path must be normalized",
        ));
    }
    if !entry
        .relative_path
        .split_once('/')
        .is_some_and(|(family, _)| family == entry.asset_family)
    {
        return Err(invalid_path(
            &entry.relative_path,
            "manifest asset family must match the first path component",
        ));
    }
    Ok(())
}

fn normalize_localize_removed_paths(paths: &[String]) -> WorkspaceResult<BTreeSet<String>> {
    let mut normalized = BTreeSet::new();
    for value in paths {
        let cleaned = value.trim();
        if cleaned.is_empty() {
            continue;
        }
        normalized.insert(normalize_manifest_relative_path(cleaned)?);
    }
    Ok(normalized)
}

fn runtime_relative_path(paths: &WorkspacePaths, relative_path: &str) -> WorkspaceResult<PathBuf> {
    Ok(paths
        .runtime_root
        .join(normalize_manifest_relative_path(relative_path)?))
}

fn current_file_sha256(path: &Path) -> WorkspaceResult<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|error| WorkspaceError::io(path, error))?;
    Ok(Some(sha256_hex(&bytes)))
}

fn embedded_asset_contents(relative_path: &str) -> Option<&'static [u8]> {
    assets::runtime_assets()
        .iter()
        .find(|asset| asset.relative_path == relative_path)
        .map(|asset| asset.contents)
}

fn write_text_atomically(path: &Path, payload: &str) -> WorkspaceResult<()> {
    write_bytes_atomically(path, payload.as_bytes())
}

fn write_bytes_atomically(path: &Path, contents: &[u8]) -> WorkspaceResult<()> {
    let parent = path.parent().ok_or_else(|| {
        invalid_path(
            path,
            "managed asset destination must have a parent directory",
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| WorkspaceError::io(parent, error))?;
    let temp_path = temp_path_for(path)?;

    let result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, path)
    })();

    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            Err(WorkspaceError::io(path, error))
        }
    }
}

fn temp_path_for(path: &Path) -> WorkspaceResult<PathBuf> {
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            invalid_path(path, "managed asset destination must have a UTF-8 filename")
        })?;
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(path.with_file_name(format!(".{filename}.tmp-{}-{counter}", process::id())))
}

fn manifest_id_for_entries(
    schema_version: &str,
    seed_package_version: &str,
    entries: &[BaselineManifestEntry],
) -> String {
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
        entries: entries
            .iter()
            .map(|entry| CanonicalEntry {
                asset_family: &entry.asset_family,
                original_sha256: &entry.original_sha256,
                relative_path: &entry.relative_path,
            })
            .collect(),
        schema_version,
        seed_package_version,
    };
    let encoded = serde_json::to_vec(&payload).expect("manifest id payload is serializable");

    sha256_hex(&encoded)
}

fn sha256_hex(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    let mut rendered = String::with_capacity(digest.len() * 2);
    for byte in digest {
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}

fn normalize_manifest_relative_path(path: impl AsRef<Path>) -> WorkspaceResult<String> {
    let path = path.as_ref();
    if path.is_absolute() {
        return Err(invalid_path(path, "asset path must be relative"));
    }

    let raw = normalize_path_lossy(path);
    let mut parts = Vec::new();
    for part in raw.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(invalid_path(path, "asset path must not contain '..'"));
        }
        parts.push(part);
    }

    if parts.is_empty() {
        return Err(invalid_path(path, "asset path must not be empty"));
    }

    Ok(parts.join("/"))
}

fn normalize_path_lossy(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn invalid_path(path: impl AsRef<Path>, message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::InvalidPath {
        path: path.as_ref().to_path_buf(),
        message: message.into(),
    }
}
