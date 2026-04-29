//! Compiler facade for persisted frozen plans, diagnostics, and currentness.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{
    assets::{
        CompilerAssetError, DEFAULT_MODE_ID, EffectiveCompileConfig,
        build_compile_input_fingerprint, canonical_mode_id, fingerprint_effective_compile_config,
        fingerprint_resolved_assets, load_effective_compile_config,
        load_effective_compile_config_from_path, resolve_compile_assets_with_config,
        resolve_mode_id,
    },
    contracts::{
        CompileInputFingerprint, CompileOutcome, CompiledPlanCurrentness,
        CompiledPlanCurrentnessState, CompiledRunPlan, CompilerContract, CompilerContractError,
    },
    materialization::{CompilerMaterializationError, materialize_compiled_run_plan},
};
use crate::{
    contracts::{
        CompileDiagnostics, RuntimeJsonContract, RuntimeJsonError, Timestamp, WorkDocumentError,
    },
    workspace::{
        StateStoreError, WorkspaceError, WorkspacePaths, atomic_write_text, initialize_workspace,
    },
};

/// Result type for compiler persistence facade operations.
pub type CompilerPersistenceResult<T> = Result<T, CompilerPersistenceError>;

/// Options for compiling and persisting one workspace plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileWorkspaceOptions {
    /// Optional requested mode id. The `standard_plain` alias resolves to `default_codex`.
    pub requested_mode_id: Option<String>,
    /// Timestamp to embed in diagnostics and compiled plans. Defaults to current UTC time.
    pub compiled_at: Option<Timestamp>,
    /// When true, return a current last-known-good plan without rewriting persisted artifacts.
    pub compile_if_needed: bool,
    /// When true, do not return a stale last-known-good plan after a failed recompile.
    pub refuse_stale_last_known_good: bool,
    /// Optional config path override used by `millrace config validate --config`.
    pub config_path: Option<PathBuf>,
    /// When true, failed compile diagnostics are persisted even if no active plan is returned.
    pub persist_failure_diagnostics: bool,
}

impl Default for CompileWorkspaceOptions {
    fn default() -> Self {
        Self {
            requested_mode_id: None,
            compiled_at: None,
            compile_if_needed: false,
            refuse_stale_last_known_good: false,
            config_path: None,
            persist_failure_diagnostics: true,
        }
    }
}

impl CompileWorkspaceOptions {
    /// Build options for a specific requested mode.
    #[must_use]
    pub fn for_mode(mode_id: impl Into<String>) -> Self {
        Self {
            requested_mode_id: Some(mode_id.into()),
            ..Self::default()
        }
    }
}

/// Failures that prevent the facade from returning a typed compile/currentness result.
#[derive(Debug)]
pub enum CompilerPersistenceError {
    /// Workspace initialization or path handling failed.
    Workspace(WorkspaceError),
    /// A state-store atomic write failed.
    StateStore(StateStoreError),
    /// A filesystem read failed.
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Underlying IO error message.
        message: String,
    },
    /// JSON syntax failed before typed contract validation.
    JsonSyntax {
        /// Path being decoded.
        path: PathBuf,
        /// Serde error message.
        message: String,
    },
    /// JSON serialization failed before persistence.
    JsonRender {
        /// Path being encoded.
        path: PathBuf,
        /// Serde error message.
        message: String,
    },
    /// A persisted compiler artifact violated its typed contract.
    CompilerContract {
        /// Path being decoded or encoded.
        path: PathBuf,
        /// Typed compiler contract error.
        source: CompilerContractError,
    },
    /// A persisted runtime JSON artifact violated its typed contract.
    RuntimeJson {
        /// Path being decoded or encoded.
        path: PathBuf,
        /// Typed runtime JSON contract error.
        source: RuntimeJsonError,
    },
    /// A timestamp could not be created.
    Time {
        /// Human-readable failure reason.
        message: String,
    },
}

impl CompilerPersistenceError {
    fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: error.to_string(),
        }
    }

    fn compiler_contract(path: impl Into<PathBuf>, source: CompilerContractError) -> Self {
        Self::CompilerContract {
            path: path.into(),
            source,
        }
    }

    fn runtime_json(path: impl Into<PathBuf>, source: RuntimeJsonError) -> Self {
        Self::RuntimeJson {
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for CompilerPersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(error) => write!(f, "{error}"),
            Self::StateStore(error) => write!(f, "{error}"),
            Self::Io { path, message } => {
                write!(
                    f,
                    "compiler persistence filesystem error at {}: {message}",
                    path.display()
                )
            }
            Self::JsonSyntax { path, message } => {
                write!(f, "failed to decode JSON at {}: {message}", path.display())
            }
            Self::JsonRender { path, message } => {
                write!(f, "failed to encode JSON at {}: {message}", path.display())
            }
            Self::CompilerContract { path, source } => {
                write!(f, "compiler contract error at {}: {source}", path.display())
            }
            Self::RuntimeJson { path, source } => {
                write!(
                    f,
                    "runtime JSON contract error at {}: {source}",
                    path.display()
                )
            }
            Self::Time { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for CompilerPersistenceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::StateStore(error) => Some(error),
            Self::CompilerContract { source, .. } => Some(source),
            Self::RuntimeJson { source, .. } => Some(source),
            Self::Io { .. }
            | Self::JsonSyntax { .. }
            | Self::JsonRender { .. }
            | Self::Time { .. } => None,
        }
    }
}

impl From<WorkspaceError> for CompilerPersistenceError {
    fn from(value: WorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

impl From<StateStoreError> for CompilerPersistenceError {
    fn from(value: StateStoreError) -> Self {
        Self::StateStore(value)
    }
}

/// Compile and persist the selected workspace plan.
pub fn compile_and_persist_workspace_plan(
    root: impl AsRef<Path>,
    requested_mode_id: Option<&str>,
) -> CompilerPersistenceResult<CompileOutcome> {
    let options = CompileWorkspaceOptions {
        requested_mode_id: requested_mode_id.map(ToOwned::to_owned),
        ..CompileWorkspaceOptions::default()
    };
    compile_and_persist_workspace_plan_with_options(root, options)
}

/// Compile and persist the selected workspace plan using explicit facade options.
pub fn compile_and_persist_workspace_plan_with_options(
    root: impl AsRef<Path>,
    options: CompileWorkspaceOptions,
) -> CompilerPersistenceResult<CompileOutcome> {
    let paths = initialize_workspace(root)?;
    compile_and_persist_workspace_plan_for_paths(&paths, options)
}

/// Compile and persist the selected workspace plan for already resolved workspace paths.
pub fn compile_and_persist_workspace_plan_for_paths(
    paths: &WorkspacePaths,
    options: CompileWorkspaceOptions,
) -> CompilerPersistenceResult<CompileOutcome> {
    let compiled_at = match options.compiled_at {
        Some(timestamp) => timestamp,
        None => utc_now_timestamp()?,
    };
    let requested_mode_id = options.requested_mode_id.as_deref();
    let config = load_compile_config(paths, options.config_path.as_deref());
    let mode_id = compile_mode_id(requested_mode_id, config.as_ref().ok());
    let last_known_good = load_persisted_compiled_plan(paths).ok().flatten();
    let last_known_good_fingerprint =
        last_known_good
            .as_ref()
            .and_then(|plan| match config.as_ref() {
                Ok(config) => existing_plan_input_fingerprint(paths, config, &mode_id, plan).ok(),
                Err(_) => None,
            });

    if options.compile_if_needed {
        if let (Some(plan), Some(expected_fingerprint)) =
            (&last_known_good, &last_known_good_fingerprint)
        {
            if plan.compile_input_fingerprint == *expected_fingerprint {
                let diagnostics = compile_diagnostics(true, &mode_id, Vec::new(), compiled_at);
                return Ok(outcome_from_plan(
                    Some(plan.clone()),
                    diagnostics,
                    false,
                    Some(expected_fingerprint.clone()),
                ));
            }
        }
    }

    let compile_result = config
        .and_then(|config| resolve_compile_assets_with_config(paths, requested_mode_id, config))
        .map_err(CompileFailure::Asset)
        .and_then(|resolved| {
            let fingerprint = resolved.compile_input_fingerprint.clone();
            materialize_compiled_run_plan(&resolved, compiled_at.clone())
                .map(|plan| (plan, fingerprint))
                .map_err(CompileFailure::Materialization)
        });

    match compile_result {
        Ok((plan, fingerprint)) => {
            let diagnostics = compile_diagnostics(true, &plan.mode_id, Vec::new(), compiled_at);
            save_compiled_plan(paths, &plan)?;
            save_compile_diagnostics(paths, &diagnostics)?;
            Ok(outcome_from_plan(
                Some(plan),
                diagnostics,
                false,
                Some(fingerprint),
            ))
        }
        Err(error) => {
            let diagnostics =
                compile_diagnostics(false, &mode_id, vec![error.to_string()], compiled_at);
            if options.persist_failure_diagnostics {
                save_compile_diagnostics(paths, &diagnostics)?;
            }

            let mut active_plan = last_known_good;
            let mut used_last_known_good = active_plan.is_some();
            if options.refuse_stale_last_known_good {
                if let Some(plan) = &active_plan {
                    if Some(&plan.compile_input_fingerprint) != last_known_good_fingerprint.as_ref()
                    {
                        active_plan = None;
                        used_last_known_good = false;
                    }
                }
            }

            Ok(outcome_from_plan(
                active_plan,
                diagnostics,
                used_last_known_good,
                last_known_good_fingerprint,
            ))
        }
    }
}

/// Inspect whether the persisted plan still matches current compile inputs.
pub fn inspect_workspace_plan_currentness(
    root: impl AsRef<Path>,
    requested_mode_id: Option<&str>,
) -> CompilerPersistenceResult<CompiledPlanCurrentness> {
    let paths = initialize_workspace(root)?;
    inspect_workspace_plan_currentness_for_paths(&paths, requested_mode_id)
}

/// Inspect currentness for already resolved workspace paths.
pub fn inspect_workspace_plan_currentness_for_paths(
    paths: &WorkspacePaths,
    requested_mode_id: Option<&str>,
) -> CompilerPersistenceResult<CompiledPlanCurrentness> {
    let persisted = read_persisted_compiled_plan(paths);
    match persisted {
        Ok(Some(plan)) => {
            let config = load_effective_compile_config(paths);
            let mode_id = compile_mode_id(requested_mode_id, config.as_ref().ok());
            let expected_fingerprint = match config {
                Ok(config) => existing_plan_input_fingerprint(paths, &config, &mode_id, &plan),
                Err(error) => Err(error),
            };
            let (state, expected_fingerprint) = match expected_fingerprint {
                Ok(expected) => {
                    let state = if plan.compile_input_fingerprint == expected {
                        CompiledPlanCurrentnessState::Current
                    } else {
                        CompiledPlanCurrentnessState::Stale
                    };
                    (state, expected)
                }
                Err(_) => (
                    CompiledPlanCurrentnessState::Unknown,
                    fallback_expected_fingerprint(paths, requested_mode_id),
                ),
            };
            validated_currentness(CompiledPlanCurrentness {
                state,
                expected_fingerprint,
                persisted_plan_id: Some(plan.compiled_plan_id),
                persisted_fingerprint: Some(plan.compile_input_fingerprint),
            })
        }
        Ok(None) => validated_currentness(CompiledPlanCurrentness {
            state: CompiledPlanCurrentnessState::Missing,
            expected_fingerprint: expected_current_fingerprint(paths, requested_mode_id)
                .unwrap_or_else(|_| fallback_expected_fingerprint(paths, requested_mode_id)),
            persisted_plan_id: None,
            persisted_fingerprint: None,
        }),
        Err(_) => validated_currentness(CompiledPlanCurrentness {
            state: CompiledPlanCurrentnessState::Unknown,
            expected_fingerprint: expected_current_fingerprint(paths, requested_mode_id)
                .unwrap_or_else(|_| fallback_expected_fingerprint(paths, requested_mode_id)),
            persisted_plan_id: None,
            persisted_fingerprint: None,
        }),
    }
}

/// Load and validate the persisted compiled plan, if present.
pub fn load_persisted_compiled_plan(
    paths: &WorkspacePaths,
) -> CompilerPersistenceResult<Option<CompiledRunPlan>> {
    read_persisted_compiled_plan(paths)
}

/// Load and validate persisted compile diagnostics, if present.
pub fn load_persisted_compile_diagnostics(
    paths: &WorkspacePaths,
) -> CompilerPersistenceResult<Option<CompileDiagnostics>> {
    let path = &paths.compile_diagnostics_file;
    if !path.is_file() {
        return Ok(None);
    }
    let raw =
        fs::read_to_string(path).map_err(|error| CompilerPersistenceError::io(path, error))?;
    let value: Value =
        serde_json::from_str(&raw).map_err(|error| CompilerPersistenceError::JsonSyntax {
            path: path.clone(),
            message: error.to_string(),
        })?;
    CompileDiagnostics::from_json_value(value)
        .map(Some)
        .map_err(|source| CompilerPersistenceError::runtime_json(path, source))
}

/// Persist a validated compiled plan atomically.
pub fn save_compiled_plan(
    paths: &WorkspacePaths,
    plan: &CompiledRunPlan,
) -> CompilerPersistenceResult<()> {
    save_compiler_contract(&paths.compiled_plan_file, plan)
}

/// Persist validated compile diagnostics atomically.
pub fn save_compile_diagnostics(
    paths: &WorkspacePaths,
    diagnostics: &CompileDiagnostics,
) -> CompilerPersistenceResult<()> {
    save_runtime_json_contract(&paths.compile_diagnostics_file, diagnostics)
}

fn read_persisted_compiled_plan(
    paths: &WorkspacePaths,
) -> CompilerPersistenceResult<Option<CompiledRunPlan>> {
    let path = &paths.compiled_plan_file;
    if !path.is_file() {
        return Ok(None);
    }
    let raw =
        fs::read_to_string(path).map_err(|error| CompilerPersistenceError::io(path, error))?;
    let value: Value =
        serde_json::from_str(&raw).map_err(|error| CompilerPersistenceError::JsonSyntax {
            path: path.clone(),
            message: error.to_string(),
        })?;
    CompiledRunPlan::from_json_value(value)
        .map(Some)
        .map_err(|source| CompilerPersistenceError::compiler_contract(path, source))
}

fn expected_current_fingerprint(
    paths: &WorkspacePaths,
    requested_mode_id: Option<&str>,
) -> Result<CompileInputFingerprint, CompilerAssetError> {
    let config = load_effective_compile_config(paths)?;
    Ok(
        resolve_compile_assets_with_config(paths, requested_mode_id, config)?
            .compile_input_fingerprint,
    )
}

fn existing_plan_input_fingerprint(
    paths: &WorkspacePaths,
    config: &EffectiveCompileConfig,
    mode_id: &str,
    plan: &CompiledRunPlan,
) -> Result<CompileInputFingerprint, CompilerAssetError> {
    build_compile_input_fingerprint(config, mode_id, &plan.resolved_assets, paths).or_else(|_| {
        let fingerprint = CompileInputFingerprint {
            mode_id: mode_id.to_owned(),
            config_fingerprint: fingerprint_effective_compile_config(config),
            assets_fingerprint: fingerprint_resolved_assets(paths, &plan.resolved_assets)?,
        };
        Ok(fingerprint)
    })
}

fn fallback_expected_fingerprint(
    paths: &WorkspacePaths,
    requested_mode_id: Option<&str>,
) -> CompileInputFingerprint {
    match load_effective_compile_config(paths) {
        Ok(config) => CompileInputFingerprint {
            mode_id: compile_mode_id(requested_mode_id, Some(&config)),
            config_fingerprint: fingerprint_effective_compile_config(&config),
            assets_fingerprint: "assets-unknown".to_owned(),
        },
        Err(_) => CompileInputFingerprint {
            mode_id: requested_mode_id
                .map(canonical_mode_id)
                .unwrap_or_else(|| DEFAULT_MODE_ID.to_owned()),
            config_fingerprint: "cfg-unknown".to_owned(),
            assets_fingerprint: "assets-unknown".to_owned(),
        },
    }
}

fn compile_mode_id(
    requested_mode_id: Option<&str>,
    config: Option<&EffectiveCompileConfig>,
) -> String {
    match config {
        Some(config) => resolve_mode_id(requested_mode_id, config),
        None => requested_mode_id
            .filter(|mode_id| !mode_id.trim().is_empty())
            .map(canonical_mode_id)
            .unwrap_or_else(|| DEFAULT_MODE_ID.to_owned()),
    }
}

fn load_compile_config(
    paths: &WorkspacePaths,
    config_path: Option<&Path>,
) -> Result<EffectiveCompileConfig, CompilerAssetError> {
    match config_path {
        Some(config_path) => load_effective_compile_config_from_path(paths, config_path),
        None => load_effective_compile_config(paths),
    }
}

fn compile_diagnostics(
    ok: bool,
    mode_id: &str,
    errors: Vec<String>,
    emitted_at: Timestamp,
) -> CompileDiagnostics {
    CompileDiagnostics {
        schema_version: "1.0".to_owned(),
        kind: "compile_diagnostics".to_owned(),
        ok,
        mode_id: mode_id.to_owned(),
        errors,
        warnings: Vec::new(),
        emitted_at,
    }
}

fn outcome_from_plan(
    active_plan: Option<CompiledRunPlan>,
    diagnostics: CompileDiagnostics,
    used_last_known_good: bool,
    compile_input_fingerprint: Option<CompileInputFingerprint>,
) -> CompileOutcome {
    let compiled_plan_id = active_plan
        .as_ref()
        .map(|plan| plan.compiled_plan_id.clone());
    let resolved_assets = active_plan
        .as_ref()
        .map(|plan| plan.resolved_assets.clone())
        .unwrap_or_default();
    CompileOutcome {
        active_plan,
        diagnostics,
        used_last_known_good,
        compiled_plan_id,
        resolved_assets,
        compile_input_fingerprint,
    }
}

fn validated_currentness(
    currentness: CompiledPlanCurrentness,
) -> CompilerPersistenceResult<CompiledPlanCurrentness> {
    currentness
        .validate()
        .map_err(|source| CompilerPersistenceError::compiler_contract("currentness", source))?;
    Ok(currentness)
}

fn save_compiler_contract<T>(path: &Path, model: &T) -> CompilerPersistenceResult<()>
where
    T: CompilerContract + Clone + Serialize,
{
    let mut validated = model.clone();
    validated
        .validate_contract()
        .map_err(|source| CompilerPersistenceError::compiler_contract(path, source))?;
    let mut payload = serde_json::to_string_pretty(&validated).map_err(|error| {
        CompilerPersistenceError::JsonRender {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    payload.push('\n');
    atomic_write_text(path, &payload)?;
    Ok(())
}

fn save_runtime_json_contract<T>(path: &Path, model: &T) -> CompilerPersistenceResult<()>
where
    T: RuntimeJsonContract + Clone + Serialize,
{
    let mut validated = model.clone();
    validated
        .validate_contract()
        .map_err(|source| CompilerPersistenceError::runtime_json(path, source))?;
    let mut payload = serde_json::to_string_pretty(&validated).map_err(|error| {
        CompilerPersistenceError::JsonRender {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    payload.push('\n');
    atomic_write_text(path, &payload)?;
    Ok(())
}

fn utc_now_timestamp() -> CompilerPersistenceResult<Timestamp> {
    let rendered = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| CompilerPersistenceError::Time {
            message: error.to_string(),
        })?;
    Timestamp::parse("emitted_at", &rendered).map_err(|error| time_error(error))
}

fn time_error(error: WorkDocumentError) -> CompilerPersistenceError {
    CompilerPersistenceError::Time {
        message: error.to_string(),
    }
}

#[derive(Debug)]
enum CompileFailure {
    Asset(CompilerAssetError),
    Materialization(CompilerMaterializationError),
}

impl fmt::Display for CompileFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Asset(error) => write!(f, "{error}"),
            Self::Materialization(error) => write!(f, "{error}"),
        }
    }
}
