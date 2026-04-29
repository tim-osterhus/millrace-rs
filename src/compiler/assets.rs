//! Workspace compile-asset resolution and deterministic input fingerprints.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::contracts::{
    CompileInputFingerprint, CompilerContract, CompilerContractError, GraphLoopDefinition,
    ModeDefinition, RegisteredStageKindDefinition, ResolvedAssetRef,
    validate_graph_stage_kind_references,
};
use crate::{
    contracts::{Plane, StageName, validate_safe_identifier},
    workspace::WorkspacePaths,
};

/// Canonical mode used when no requested or configured mode is present.
pub const DEFAULT_MODE_ID: &str = "default_codex";

/// Content token used when an optional referenced asset is absent.
pub const MISSING_ASSET_TOKEN: &str = "missing";

const MODE_ALIAS_STANDARD_PLAIN: &str = "standard_plain";

/// Result type for compiler asset resolution.
pub type CompilerAssetResult<T> = Result<T, CompilerAssetError>;

/// Structured compiler asset/config failures with path context where available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompilerAssetError {
    /// A filesystem operation failed.
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Underlying IO error message.
        message: String,
    },
    /// Runtime config TOML could not be parsed.
    Toml {
        /// Config path.
        path: PathBuf,
        /// Parser error message.
        message: String,
    },
    /// A compile-relevant config field was malformed.
    InvalidConfig {
        /// Config path when the value came from a file.
        path: Option<PathBuf>,
        /// Dotted config field path.
        field: String,
        /// Human-readable reason.
        message: String,
    },
    /// A workspace asset path was malformed.
    InvalidPath {
        /// Path value involved in the failure.
        path: PathBuf,
        /// Validation error message.
        message: String,
    },
    /// A referenced asset id could not be resolved under the initialized runtime root.
    UnknownAsset {
        /// Asset family.
        asset_family: &'static str,
        /// Requested id.
        asset_id: String,
        /// Directory searched.
        searched_root: PathBuf,
    },
    /// Multiple assets declared the same id.
    DuplicateAssetId {
        /// Asset family.
        asset_family: &'static str,
        /// Duplicated id.
        asset_id: String,
        /// Candidate paths.
        paths: Vec<PathBuf>,
    },
    /// The loaded asset declared a different id or plane than the requested reference.
    AssetIdMismatch {
        /// Asset path.
        path: PathBuf,
        /// Field that mismatched.
        field: &'static str,
        /// Expected value.
        expected: String,
        /// Actual value.
        actual: String,
    },
    /// A required referenced file was missing.
    MissingReferencedAsset {
        /// Asset family.
        asset_family: &'static str,
        /// Logical reference id.
        logical_id: String,
        /// Expected path.
        path: PathBuf,
    },
    /// A referenced asset path or content was invalid.
    InvalidReferencedAsset {
        /// Asset family.
        asset_family: &'static str,
        /// Logical reference id.
        logical_id: String,
        /// Asset path.
        path: PathBuf,
        /// Human-readable reason.
        message: String,
    },
    /// A typed compiler contract failed to decode or validate.
    Contract {
        /// Asset path.
        path: PathBuf,
        /// Contract artifact name.
        artifact: &'static str,
        /// Contract error text.
        message: String,
    },
}

impl fmt::Display for CompilerAssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => {
                write!(
                    f,
                    "compiler asset filesystem error at {}: {message}",
                    path.display()
                )
            }
            Self::Toml { path, message } => {
                write!(
                    f,
                    "failed to parse runtime config {}: {message}",
                    path.display()
                )
            }
            Self::InvalidConfig {
                path,
                field,
                message,
            } => {
                if let Some(path) = path {
                    write!(
                        f,
                        "compile config field {field} is invalid in {}: {message}",
                        path.display()
                    )
                } else {
                    write!(f, "compile config field {field} is invalid: {message}")
                }
            }
            Self::InvalidPath { path, message } => {
                write!(
                    f,
                    "invalid compiler asset path {}: {message}",
                    path.display()
                )
            }
            Self::UnknownAsset {
                asset_family,
                asset_id,
                searched_root,
            } => write!(
                f,
                "unknown {asset_family} asset id {asset_id} under {}",
                searched_root.display()
            ),
            Self::DuplicateAssetId {
                asset_family,
                asset_id,
                paths,
            } => {
                let joined = paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "duplicate {asset_family} asset id {asset_id}: {joined}")
            }
            Self::AssetIdMismatch {
                path,
                field,
                expected,
                actual,
            } => write!(
                f,
                "asset {} has mismatched {field}: expected {expected}, found {actual}",
                path.display()
            ),
            Self::MissingReferencedAsset {
                asset_family,
                logical_id,
                path,
            } => write!(
                f,
                "missing referenced {asset_family} asset {logical_id}: {}",
                path.display()
            ),
            Self::InvalidReferencedAsset {
                asset_family,
                logical_id,
                path,
                message,
            } => write!(
                f,
                "invalid referenced {asset_family} asset {logical_id} at {}: {message}",
                path.display()
            ),
            Self::Contract {
                path,
                artifact,
                message,
            } => write!(
                f,
                "invalid {artifact} compiler contract at {}: {message}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for CompilerAssetError {}

/// Compile-relevant runtime config subset that affects mode selection or frozen authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveCompileConfig {
    pub runtime: EffectiveRuntimeCompileConfig,
    pub runners: EffectiveRunnersCompileConfig,
    pub recovery: EffectiveRecoveryCompileConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stages: BTreeMap<String, EffectiveStageCompileConfig>,
}

impl Default for EffectiveCompileConfig {
    fn default() -> Self {
        Self {
            runtime: EffectiveRuntimeCompileConfig::default(),
            runners: EffectiveRunnersCompileConfig::default(),
            recovery: EffectiveRecoveryCompileConfig::default(),
            stages: BTreeMap::new(),
        }
    }
}

/// Compile-relevant fields from `[runtime]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveRuntimeCompileConfig {
    pub default_mode: String,
}

impl Default for EffectiveRuntimeCompileConfig {
    fn default() -> Self {
        Self {
            default_mode: DEFAULT_MODE_ID.to_owned(),
        }
    }
}

/// Compile-relevant runner config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveRunnersCompileConfig {
    pub default_runner: String,
    pub codex: EffectiveCodexCompileConfig,
}

impl Default for EffectiveRunnersCompileConfig {
    fn default() -> Self {
        Self {
            default_runner: "codex_cli".to_owned(),
            codex: EffectiveCodexCompileConfig::default(),
        }
    }
}

/// Compile-relevant Codex runner defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EffectiveCodexCompileConfig {
    pub model_reasoning_effort: Option<String>,
}

/// Compile-relevant recovery thresholds used by graph policy materialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveRecoveryCompileConfig {
    pub max_fix_cycles: u64,
    pub max_troubleshoot_attempts_before_consult: u64,
    pub max_mechanic_attempts: u64,
}

impl Default for EffectiveRecoveryCompileConfig {
    fn default() -> Self {
        Self {
            max_fix_cycles: 2,
            max_troubleshoot_attempts_before_consult: 2,
            max_mechanic_attempts: 2,
        }
    }
}

/// Compile-relevant per-stage override config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EffectiveStageCompileConfig {
    pub runner: Option<String>,
    pub model: Option<String>,
    pub model_reasoning_effort: Option<String>,
    pub timeout_seconds: Option<u64>,
}

/// One graph-loop asset loaded for the selected mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGraphLoopAsset {
    pub plane: Plane,
    pub relative_path: String,
    pub graph_loop: GraphLoopDefinition,
}

/// One stage-kind registry asset loaded for selected graph nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedStageKindAsset {
    pub stage_kind_id: String,
    pub relative_path: String,
    pub definition: RegisteredStageKindDefinition,
}

/// Complete resolved compiler asset set for one workspace/mode/config selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCompileAssetSet {
    pub mode_id: String,
    pub requested_mode_id: Option<String>,
    pub mode_relative_path: String,
    pub mode: ModeDefinition,
    pub graph_loops: Vec<ResolvedGraphLoopAsset>,
    pub stage_kinds: Vec<ResolvedStageKindAsset>,
    pub resolved_assets: Vec<ResolvedAssetRef>,
    pub config: EffectiveCompileConfig,
    pub compile_input_fingerprint: CompileInputFingerprint,
}

/// Resolve a requested or configured mode id to its canonical built-in id.
#[must_use]
pub fn canonical_mode_id(mode_id: &str) -> String {
    let trimmed = mode_id.trim();
    if trimmed == MODE_ALIAS_STANDARD_PLAIN {
        DEFAULT_MODE_ID.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Resolve mode precedence: requested CLI mode, runtime config default, canonical default.
#[must_use]
pub fn resolve_mode_id(requested_mode_id: Option<&str>, config: &EffectiveCompileConfig) -> String {
    if let Some(requested) = requested_mode_id {
        if !requested.trim().is_empty() {
            return canonical_mode_id(requested);
        }
    }
    if !config.runtime.default_mode.trim().is_empty() {
        return canonical_mode_id(&config.runtime.default_mode);
    }
    DEFAULT_MODE_ID.to_owned()
}

/// Load the effective compile-relevant runtime config from `millrace-agents/millrace.toml`.
pub fn load_effective_compile_config(
    paths: &WorkspacePaths,
) -> CompilerAssetResult<EffectiveCompileConfig> {
    load_effective_compile_config_from_path(paths, &paths.runtime_config_file)
}

/// Load the effective compile-relevant runtime config from an explicit TOML path.
pub fn load_effective_compile_config_from_path(
    _paths: &WorkspacePaths,
    config_path: &Path,
) -> CompilerAssetResult<EffectiveCompileConfig> {
    let mut config = EffectiveCompileConfig::default();
    if !config_path.exists() {
        return Ok(config);
    }

    let raw = fs::read_to_string(config_path).map_err(|error| io_error(config_path, error))?;
    let value: toml::Value = toml::from_str(&raw).map_err(|error| CompilerAssetError::Toml {
        path: config_path.to_path_buf(),
        message: error.to_string(),
    })?;
    let Some(root) = value.as_table() else {
        return Err(invalid_config(
            Some(config_path),
            "<root>",
            "must be a TOML table",
        ));
    };

    if let Some(runtime) = child_table(root, "runtime", config_path)? {
        if let Some(default_mode) = optional_string(runtime, "default_mode", config_path)? {
            config.runtime.default_mode = default_mode.trim().to_owned();
        }
    }

    if let Some(runners) = child_table(root, "runners", config_path)? {
        if let Some(default_runner) = optional_string(runners, "default_runner", config_path)? {
            config.runners.default_runner =
                normalize_runner_name(&default_runner, "runners.default_runner", config_path)?;
        }
        if let Some(codex) = child_table(runners, "codex", config_path)? {
            config.runners.codex.model_reasoning_effort =
                optional_string(codex, "model_reasoning_effort", config_path)?
                    .map(|value| {
                        validate_reasoning_effort(
                            &value,
                            "runners.codex.model_reasoning_effort",
                            config_path,
                        )
                    })
                    .transpose()?;
        }
    }

    if let Some(recovery) = child_table(root, "recovery", config_path)? {
        if let Some(value) = optional_positive_u64(recovery, "max_fix_cycles", config_path)? {
            config.recovery.max_fix_cycles = value;
        }
        if let Some(value) = optional_positive_u64(
            recovery,
            "max_troubleshoot_attempts_before_consult",
            config_path,
        )? {
            config.recovery.max_troubleshoot_attempts_before_consult = value;
        }
        if let Some(value) = optional_positive_u64(recovery, "max_mechanic_attempts", config_path)?
        {
            config.recovery.max_mechanic_attempts = value;
        }
    }

    if let Some(stages) = child_table(root, "stages", config_path)? {
        for (stage_name, value) in stages {
            StageName::from_value(stage_name).map_err(|_| {
                invalid_config(
                    Some(config_path),
                    format!("stages.{stage_name}"),
                    "unknown stage name",
                )
            })?;
            let Some(stage_table) = value.as_table() else {
                return Err(invalid_config(
                    Some(config_path),
                    format!("stages.{stage_name}"),
                    "must be a TOML table",
                ));
            };
            reject_unknown_stage_config_keys(stage_name, stage_table, config_path)?;
            let stage_config = EffectiveStageCompileConfig {
                runner: optional_string(stage_table, "runner", config_path)?
                    .map(|value| {
                        normalize_runner_name(
                            &value,
                            format!("stages.{stage_name}.runner"),
                            config_path,
                        )
                    })
                    .transpose()?,
                model: optional_string(stage_table, "model", config_path)?
                    .map(|value| value.trim().to_owned()),
                model_reasoning_effort: optional_string(
                    stage_table,
                    "model_reasoning_effort",
                    config_path,
                )?
                .map(|value| {
                    validate_reasoning_effort(
                        &value,
                        format!("stages.{stage_name}.model_reasoning_effort"),
                        config_path,
                    )
                })
                .transpose()?,
                timeout_seconds: optional_positive_u64(
                    stage_table,
                    "timeout_seconds",
                    config_path,
                )?,
            };
            config.stages.insert(stage_name.to_owned(), stage_config);
        }
    }

    Ok(config)
}

/// Build the deterministic fingerprint for effective compile-relevant config.
#[must_use]
pub fn fingerprint_effective_compile_config(config: &EffectiveCompileConfig) -> String {
    let encoded = serde_json::to_vec(config).expect("effective compile config is serializable");
    prefixed_digest("cfg", &encoded)
}

/// Resolve all authoritative compile assets for a workspace using its config.
pub fn resolve_compile_assets(
    paths: &WorkspacePaths,
    requested_mode_id: Option<&str>,
) -> CompilerAssetResult<ResolvedCompileAssetSet> {
    let config = load_effective_compile_config(paths)?;
    resolve_compile_assets_with_config(paths, requested_mode_id, config)
}

/// Resolve all authoritative compile assets for a workspace using an explicit effective config.
pub fn resolve_compile_assets_with_config(
    paths: &WorkspacePaths,
    requested_mode_id: Option<&str>,
    config: EffectiveCompileConfig,
) -> CompilerAssetResult<ResolvedCompileAssetSet> {
    let mode_id = resolve_mode_id(requested_mode_id, &config);
    let mode_relative_path = resolve_mode_asset_path(paths, &mode_id)?;
    let mode: ModeDefinition = load_contract(paths, &mode_relative_path)?;
    require_asset_id(
        paths,
        &mode_relative_path,
        "mode_id",
        &mode_id,
        &mode.mode_id,
    )?;

    let mut graph_loops = Vec::new();
    for (plane, loop_id) in sorted_loop_ids_by_plane(&mode.loop_ids_by_plane) {
        let relative_path = resolve_graph_loop_asset_path(paths, loop_id)?;
        let graph_loop: GraphLoopDefinition = load_contract(paths, &relative_path)?;
        require_asset_id(
            paths,
            &relative_path,
            "loop_id",
            loop_id,
            &graph_loop.loop_id,
        )?;
        if graph_loop.plane != *plane {
            return Err(CompilerAssetError::AssetIdMismatch {
                path: paths.runtime_root.join(&relative_path),
                field: "plane",
                expected: plane.as_str().to_owned(),
                actual: graph_loop.plane.as_str().to_owned(),
            });
        }
        graph_loops.push(ResolvedGraphLoopAsset {
            plane: *plane,
            relative_path,
            graph_loop,
        });
    }

    let mut stage_kinds = Vec::new();
    let mut stage_kind_ids = HashSet::new();
    for graph_asset in &graph_loops {
        for node in &graph_asset.graph_loop.nodes {
            if !stage_kind_ids.insert(node.stage_kind_id.clone()) {
                continue;
            }
            let relative_path =
                resolve_stage_kind_asset_path(paths, graph_asset.plane, &node.stage_kind_id)?;
            let definition: RegisteredStageKindDefinition = load_contract(paths, &relative_path)?;
            require_asset_id(
                paths,
                &relative_path,
                "stage_kind_id",
                &node.stage_kind_id,
                &definition.stage_kind_id,
            )?;
            if definition.plane != graph_asset.plane {
                return Err(CompilerAssetError::AssetIdMismatch {
                    path: paths.runtime_root.join(&relative_path),
                    field: "plane",
                    expected: graph_asset.plane.as_str().to_owned(),
                    actual: definition.plane.as_str().to_owned(),
                });
            }
            stage_kinds.push(ResolvedStageKindAsset {
                stage_kind_id: definition.stage_kind_id.clone(),
                relative_path,
                definition,
            });
        }
    }

    let stage_kind_map: HashMap<_, _> = stage_kinds
        .iter()
        .map(|stage_kind| {
            (
                stage_kind.stage_kind_id.clone(),
                stage_kind.definition.clone(),
            )
        })
        .collect();
    for graph_asset in &graph_loops {
        validate_graph_stage_kind_references(&graph_asset.graph_loop, &stage_kind_map).map_err(
            |error| {
                contract_error(
                    paths,
                    &graph_asset.relative_path,
                    GraphLoopDefinition::ARTIFACT,
                    error,
                )
            },
        )?;
    }
    validate_mode_stage_maps(paths, &mode_relative_path, &mode, &graph_loops)?;

    let resolved_assets = build_resolved_asset_refs(
        paths,
        &mode,
        &mode_relative_path,
        &graph_loops,
        &stage_kinds,
    )?;
    let compile_input_fingerprint =
        build_compile_input_fingerprint(&config, &mode_id, &resolved_assets, paths)?;

    Ok(ResolvedCompileAssetSet {
        mode_id,
        requested_mode_id: requested_mode_id.map(ToOwned::to_owned),
        mode_relative_path,
        mode,
        graph_loops,
        stage_kinds,
        resolved_assets,
        config,
        compile_input_fingerprint,
    })
}

/// Resolve assets and return only the compile input fingerprint.
pub fn compile_input_fingerprint_for_workspace(
    paths: &WorkspacePaths,
    requested_mode_id: Option<&str>,
) -> CompilerAssetResult<CompileInputFingerprint> {
    Ok(resolve_compile_assets(paths, requested_mode_id)?.compile_input_fingerprint)
}

/// Build a compile input fingerprint from explicit config and resolved asset refs.
pub fn build_compile_input_fingerprint(
    config: &EffectiveCompileConfig,
    mode_id: &str,
    resolved_assets: &[ResolvedAssetRef],
    paths: &WorkspacePaths,
) -> CompilerAssetResult<CompileInputFingerprint> {
    let fingerprint = CompileInputFingerprint {
        mode_id: mode_id.to_owned(),
        config_fingerprint: fingerprint_effective_compile_config(config),
        assets_fingerprint: fingerprint_resolved_assets(paths, resolved_assets)?,
    };
    fingerprint
        .validate()
        .map_err(|error| CompilerAssetError::Contract {
            path: paths.runtime_root.clone(),
            artifact: "compile_input_fingerprint",
            message: error.to_string(),
        })?;
    Ok(fingerprint)
}

/// Build a deterministic fingerprint over current content for the supplied resolved asset refs.
pub fn fingerprint_resolved_assets(
    paths: &WorkspacePaths,
    resolved_assets: &[ResolvedAssetRef],
) -> CompilerAssetResult<String> {
    let mut sorted = resolved_assets.to_vec();
    sorted.sort_by(|left, right| {
        (
            left.asset_family.as_str(),
            left.logical_id.as_str(),
            left.compile_time_path.as_str(),
        )
            .cmp(&(
                right.asset_family.as_str(),
                right.logical_id.as_str(),
                right.compile_time_path.as_str(),
            ))
    });

    let mut digest = Sha256::new();
    for asset in sorted {
        let relative_path = normalize_runtime_relative_path(&asset.compile_time_path)?;
        let path = paths.runtime_root.join(&relative_path);
        digest.update(asset.asset_family.as_bytes());
        digest.update([0]);
        digest.update(asset.logical_id.as_bytes());
        digest.update([0]);
        digest.update(relative_path.as_bytes());
        digest.update([0]);
        digest.update(current_asset_content_token(&path)?.as_bytes());
        digest.update([0]);
    }
    Ok(format!("assets-{}", hex_prefix(digest.finalize(), 12)))
}

fn build_resolved_asset_refs(
    paths: &WorkspacePaths,
    mode: &ModeDefinition,
    mode_relative_path: &str,
    graph_loops: &[ResolvedGraphLoopAsset],
    stage_kinds: &[ResolvedStageKindAsset],
) -> CompilerAssetResult<Vec<ResolvedAssetRef>> {
    let mut refs = Vec::new();
    push_required_ref(
        paths,
        &mut refs,
        "mode",
        format!("mode:{}", mode.mode_id),
        mode_relative_path,
    )?;

    for graph_asset in graph_loops {
        push_required_ref(
            paths,
            &mut refs,
            "graph_loop",
            format!("graph_loop:{}", graph_asset.graph_loop.loop_id),
            &graph_asset.relative_path,
        )?;
    }

    for stage_kind in stage_kinds {
        push_required_ref(
            paths,
            &mut refs,
            "stage_kind",
            format!("stage_kind:{}", stage_kind.stage_kind_id),
            &stage_kind.relative_path,
        )?;
    }

    let stage_kind_by_id: HashMap<_, _> = stage_kinds
        .iter()
        .map(|stage_kind| (stage_kind.stage_kind_id.as_str(), &stage_kind.definition))
        .collect();
    let mut entrypoint_paths = Vec::new();
    let mut required_skill_paths = Vec::new();
    let mut attached_skill_paths = Vec::new();

    for graph_asset in graph_loops {
        for node in &graph_asset.graph_loop.nodes {
            let stage_kind = stage_kind_by_id[node.stage_kind_id.as_str()];
            let stage_name = StageName::from_value(&node.stage_kind_id).ok();

            let mut entrypoint_path = stage_kind.default_entrypoint_path.clone();
            if let Some(node_entrypoint_path) = &node.entrypoint_path {
                entrypoint_path = node_entrypoint_path.clone();
            }
            if let Some(stage_name) = stage_name {
                if let Some(mode_entrypoint_path) = mode.stage_entrypoint_overrides.get(&stage_name)
                {
                    entrypoint_path = mode_entrypoint_path.clone();
                }
            }
            let entrypoint_path = validate_runtime_markdown_path(
                "entrypoint",
                &entrypoint_path,
                "entrypoints/",
                paths,
            )?;
            push_unique(&mut entrypoint_paths, entrypoint_path);

            for skill_path in &stage_kind.required_skill_paths {
                let skill_path =
                    validate_runtime_markdown_path("skill", skill_path, "skills/", paths)?;
                push_unique(&mut required_skill_paths, skill_path);
            }

            for skill_path in &node.attached_skill_additions {
                let skill_path =
                    validate_runtime_markdown_path("skill", skill_path, "skills/", paths)?;
                push_unique(&mut attached_skill_paths, skill_path);
            }
            if let Some(stage_name) = stage_name {
                if let Some(mode_skill_paths) = mode.stage_skill_additions.get(&stage_name) {
                    for skill_path in mode_skill_paths {
                        let skill_path =
                            validate_runtime_markdown_path("skill", skill_path, "skills/", paths)?;
                        push_unique(&mut attached_skill_paths, skill_path);
                    }
                }
            }
        }
    }

    for entrypoint_path in entrypoint_paths {
        push_required_ref(
            paths,
            &mut refs,
            "entrypoint",
            format!("entrypoint:{entrypoint_path}"),
            &entrypoint_path,
        )?;
    }
    for skill_path in required_skill_paths {
        push_required_ref(
            paths,
            &mut refs,
            "skill",
            format!("skill:{skill_path}"),
            &skill_path,
        )?;
    }
    for skill_path in attached_skill_paths {
        push_optional_ref(
            paths,
            &mut refs,
            "skill",
            format!("skill:{skill_path}"),
            &skill_path,
        )?;
    }

    dedupe_asset_refs(&mut refs);
    Ok(refs)
}

fn push_required_ref(
    paths: &WorkspacePaths,
    refs: &mut Vec<ResolvedAssetRef>,
    asset_family: &'static str,
    logical_id: String,
    relative_path: &str,
) -> CompilerAssetResult<()> {
    let relative_path = normalize_runtime_relative_path(relative_path)?;
    let path = paths.runtime_root.join(&relative_path);
    if !path.is_file() {
        return Err(CompilerAssetError::MissingReferencedAsset {
            asset_family,
            logical_id,
            path,
        });
    }
    let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
    refs.push(ResolvedAssetRef {
        asset_family: asset_family.to_owned(),
        logical_id,
        compile_time_path: relative_path,
        content_sha256: sha256_hex(&bytes),
    });
    Ok(())
}

fn push_optional_ref(
    paths: &WorkspacePaths,
    refs: &mut Vec<ResolvedAssetRef>,
    asset_family: &'static str,
    logical_id: String,
    relative_path: &str,
) -> CompilerAssetResult<()> {
    let relative_path = normalize_runtime_relative_path(relative_path)?;
    let path = paths.runtime_root.join(&relative_path);
    let content_sha256 = if path.is_file() {
        sha256_hex(&fs::read(&path).map_err(|error| io_error(&path, error))?)
    } else {
        MISSING_ASSET_TOKEN.to_owned()
    };
    refs.push(ResolvedAssetRef {
        asset_family: asset_family.to_owned(),
        logical_id,
        compile_time_path: relative_path,
        content_sha256,
    });
    Ok(())
}

fn current_asset_content_token(path: &Path) -> CompilerAssetResult<String> {
    if !path.is_file() {
        return Ok(MISSING_ASSET_TOKEN.to_owned());
    }
    Ok(sha256_hex(
        &fs::read(path).map_err(|error| io_error(path, error))?,
    ))
}

fn resolve_mode_asset_path(paths: &WorkspacePaths, mode_id: &str) -> CompilerAssetResult<String> {
    let named = format!("modes/{mode_id}.json");
    if paths.runtime_root.join(&named).is_file() {
        return Ok(named);
    }
    discover_asset_by_declared_id(paths, "mode", "modes", "mode_id", mode_id)?.ok_or_else(|| {
        CompilerAssetError::UnknownAsset {
            asset_family: "mode",
            asset_id: mode_id.to_owned(),
            searched_root: paths.modes_dir.clone(),
        }
    })
}

fn resolve_graph_loop_asset_path(
    paths: &WorkspacePaths,
    loop_id: &str,
) -> CompilerAssetResult<String> {
    if let Some((plane, name)) = loop_id.split_once('.') {
        let named = format!("graphs/{plane}/{name}.json");
        if paths.runtime_root.join(&named).is_file() {
            return Ok(named);
        }
    }
    discover_asset_by_declared_id(paths, "graph_loop", "graphs", "loop_id", loop_id)?.ok_or_else(
        || CompilerAssetError::UnknownAsset {
            asset_family: "graph_loop",
            asset_id: loop_id.to_owned(),
            searched_root: paths.graphs_dir.clone(),
        },
    )
}

fn resolve_stage_kind_asset_path(
    paths: &WorkspacePaths,
    plane: Plane,
    stage_kind_id: &str,
) -> CompilerAssetResult<String> {
    let named = format!("registry/stage_kinds/{plane}/{stage_kind_id}.json");
    if paths.runtime_root.join(&named).is_file() {
        return Ok(named);
    }
    discover_asset_by_declared_id(
        paths,
        "stage_kind",
        "registry/stage_kinds",
        "stage_kind_id",
        stage_kind_id,
    )?
    .ok_or_else(|| CompilerAssetError::UnknownAsset {
        asset_family: "stage_kind",
        asset_id: stage_kind_id.to_owned(),
        searched_root: paths.stage_kind_registry_dir.clone(),
    })
}

fn discover_asset_by_declared_id(
    paths: &WorkspacePaths,
    asset_family: &'static str,
    root_relative_dir: &str,
    id_field: &str,
    expected_id: &str,
) -> CompilerAssetResult<Option<String>> {
    let root_relative_dir = normalize_runtime_relative_path(root_relative_dir)?;
    let root = paths.runtime_root.join(&root_relative_dir);
    if !root.is_dir() {
        return Ok(None);
    }

    let mut matches = Vec::new();
    for path in sorted_json_files(&root)? {
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if value.get(id_field).and_then(Value::as_str) == Some(expected_id) {
            matches.push(runtime_relative_path(paths, &path)?);
        }
    }

    if matches.len() > 1 {
        return Err(CompilerAssetError::DuplicateAssetId {
            asset_family,
            asset_id: expected_id.to_owned(),
            paths: matches
                .iter()
                .map(|relative| paths.runtime_root.join(relative))
                .collect(),
        });
    }
    Ok(matches.pop())
}

fn sorted_json_files(root: &Path) -> CompilerAssetResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_json_files(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_json_files(directory: &Path, files: &mut Vec<PathBuf>) -> CompilerAssetResult<()> {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .map_err(|error| io_error(directory, error))?
        .collect::<Result<_, _>>()
        .map_err(|error| io_error(directory, error))?;
    entries.sort_by_key(|entry: &fs::DirEntry| entry.path());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, files)?;
        } else if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
    }
    Ok(())
}

fn load_contract<T>(paths: &WorkspacePaths, relative_path: &str) -> CompilerAssetResult<T>
where
    T: CompilerContract,
{
    let relative_path = normalize_runtime_relative_path(relative_path)?;
    let path = paths.runtime_root.join(&relative_path);
    let raw = fs::read_to_string(&path).map_err(|error| io_error(&path, error))?;
    T::from_json_str(&raw)
        .map_err(|error| contract_error(paths, &relative_path, T::ARTIFACT, error))
}

fn validate_mode_stage_maps(
    paths: &WorkspacePaths,
    mode_relative_path: &str,
    mode: &ModeDefinition,
    graph_loops: &[ResolvedGraphLoopAsset],
) -> CompilerAssetResult<()> {
    let mut selected_stages = HashSet::new();
    for graph_asset in graph_loops {
        for node in &graph_asset.graph_loop.nodes {
            if let Ok(stage) = StageName::from_value(&node.stage_kind_id) {
                selected_stages.insert(stage);
            }
        }
    }

    for (map_name, stages) in [
        (
            "stage_entrypoint_overrides",
            mode.stage_entrypoint_overrides
                .keys()
                .copied()
                .collect::<Vec<_>>(),
        ),
        (
            "stage_skill_additions",
            mode.stage_skill_additions
                .keys()
                .copied()
                .collect::<Vec<_>>(),
        ),
        (
            "stage_model_bindings",
            mode.stage_model_bindings
                .keys()
                .copied()
                .collect::<Vec<_>>(),
        ),
        (
            "stage_runner_bindings",
            mode.stage_runner_bindings
                .keys()
                .copied()
                .collect::<Vec<_>>(),
        ),
    ] {
        for stage in stages {
            if !selected_stages.contains(&stage) {
                return Err(CompilerAssetError::InvalidReferencedAsset {
                    asset_family: "mode",
                    logical_id: format!("mode:{}", mode.mode_id),
                    path: paths.runtime_root.join(mode_relative_path),
                    message: format!(
                        "mode map `{map_name}` references stage outside selected loops: {}",
                        stage.as_str()
                    ),
                });
            }
        }
    }

    Ok(())
}

fn validate_runtime_markdown_path(
    asset_family: &'static str,
    relative_path: &str,
    required_prefix: &str,
    paths: &WorkspacePaths,
) -> CompilerAssetResult<String> {
    let normalized = normalize_runtime_relative_path(relative_path)?;
    if !normalized.starts_with(required_prefix) || !normalized.ends_with(".md") {
        return Err(CompilerAssetError::InvalidReferencedAsset {
            asset_family,
            logical_id: format!("{asset_family}:{normalized}"),
            path: paths.runtime_root.join(&normalized),
            message: format!("must be a markdown asset under {required_prefix}"),
        });
    }
    Ok(normalized)
}

fn require_asset_id(
    paths: &WorkspacePaths,
    relative_path: &str,
    field: &'static str,
    expected: &str,
    actual: &str,
) -> CompilerAssetResult<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(CompilerAssetError::AssetIdMismatch {
            path: paths.runtime_root.join(relative_path),
            field,
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        })
    }
}

fn sorted_loop_ids_by_plane(loop_ids_by_plane: &HashMap<Plane, String>) -> Vec<(&Plane, &String)> {
    let mut entries: Vec<_> = loop_ids_by_plane.iter().collect();
    entries.sort_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
    entries
}

fn normalize_runtime_relative_path(path: impl AsRef<str>) -> CompilerAssetResult<String> {
    let raw = path.as_ref().trim().replace('\\', "/");
    if raw.is_empty() || raw.starts_with('/') {
        return Err(CompilerAssetError::InvalidPath {
            path: PathBuf::from(path.as_ref()),
            message: "asset path must be relative and non-empty".to_owned(),
        });
    }

    let mut parts = Vec::new();
    for part in raw.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(CompilerAssetError::InvalidPath {
                path: PathBuf::from(path.as_ref()),
                message: "asset path must not contain '..'".to_owned(),
            });
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err(CompilerAssetError::InvalidPath {
            path: PathBuf::from(path.as_ref()),
            message: "asset path must not be empty".to_owned(),
        });
    }
    Ok(parts.join("/"))
}

fn runtime_relative_path(paths: &WorkspacePaths, path: &Path) -> CompilerAssetResult<String> {
    let relative =
        path.strip_prefix(&paths.runtime_root)
            .map_err(|_| CompilerAssetError::InvalidPath {
                path: path.to_path_buf(),
                message: "asset path is outside runtime root".to_owned(),
            })?;
    let raw = relative
        .to_str()
        .ok_or_else(|| CompilerAssetError::InvalidPath {
            path: path.to_path_buf(),
            message: "asset path must be valid UTF-8".to_owned(),
        })?;
    normalize_runtime_relative_path(raw)
}

fn contract_error(
    paths: &WorkspacePaths,
    relative_path: &str,
    artifact: &'static str,
    error: CompilerContractError,
) -> CompilerAssetError {
    CompilerAssetError::Contract {
        path: paths.runtime_root.join(relative_path),
        artifact,
        message: error.to_string(),
    }
}

fn io_error(path: &Path, error: io::Error) -> CompilerAssetError {
    CompilerAssetError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

fn invalid_config(
    path: Option<&Path>,
    field: impl Into<String>,
    message: impl Into<String>,
) -> CompilerAssetError {
    CompilerAssetError::InvalidConfig {
        path: path.map(Path::to_path_buf),
        field: field.into(),
        message: message.into(),
    }
}

fn normalize_runner_name(
    runner_name: &str,
    field: impl Into<String>,
    path: &Path,
) -> CompilerAssetResult<String> {
    let field = field.into();
    let runner_name = runner_name.trim();
    validate_safe_identifier(runner_name, &field)
        .map(|value| value.to_owned())
        .map_err(|error| invalid_config(Some(path), field, error.to_string()))
}

fn validate_reasoning_effort(
    value: &str,
    field: impl Into<String>,
    path: &Path,
) -> CompilerAssetResult<String> {
    let value = value.trim();
    match value {
        "low" | "medium" | "high" | "xhigh" => Ok(value.to_owned()),
        _ => Err(invalid_config(
            Some(path),
            field,
            "must be one of `low`, `medium`, `high`, or `xhigh`",
        )),
    }
}

fn reject_unknown_stage_config_keys(
    stage_name: &str,
    table: &toml::map::Map<String, toml::Value>,
    path: &Path,
) -> CompilerAssetResult<()> {
    for key in table.keys() {
        if matches!(
            key.as_str(),
            "runner" | "model" | "model_reasoning_effort" | "timeout_seconds"
        ) {
            continue;
        }
        return Err(invalid_config(
            Some(path),
            format!("stages.{stage_name}.{key}"),
            "unsupported stage override key",
        ));
    }
    Ok(())
}

fn child_table<'a>(
    table: &'a toml::map::Map<String, toml::Value>,
    key: &str,
    path: &Path,
) -> CompilerAssetResult<Option<&'a toml::map::Map<String, toml::Value>>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    value
        .as_table()
        .map(Some)
        .ok_or_else(|| invalid_config(Some(path), key, "must be a TOML table when present"))
}

fn optional_string(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    path: &Path,
) -> CompilerAssetResult<Option<String>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|value| Some(value.to_owned()))
        .ok_or_else(|| invalid_config(Some(path), key, "must be a string when present"))
}

fn optional_positive_u64(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    path: &Path,
) -> CompilerAssetResult<Option<u64>> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_integer() else {
        return Err(invalid_config(
            Some(path),
            key,
            "must be a positive integer when present",
        ));
    };
    if value <= 0 {
        return Err(invalid_config(Some(path), key, "must be greater than zero"));
    }
    Ok(Some(value as u64))
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn dedupe_asset_refs(refs: &mut Vec<ResolvedAssetRef>) {
    let mut seen = HashSet::new();
    refs.retain(|asset| {
        seen.insert((
            asset.asset_family.clone(),
            asset.logical_id.clone(),
            asset.compile_time_path.clone(),
        ))
    });
}

fn prefixed_digest(prefix: &str, payload: &[u8]) -> String {
    let digest = Sha256::digest(payload);
    format!("{prefix}-{}", hex_prefix(digest, 12))
}

fn sha256_hex(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    hex_prefix(digest, 64)
}

fn hex_prefix(digest: impl AsRef<[u8]>, hex_len: usize) -> String {
    let bytes = digest.as_ref();
    let mut rendered = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered.truncate(hex_len);
    rendered
}
