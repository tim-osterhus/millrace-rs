//! Embedded package-managed assets for workspace initialization.

/// Runtime asset families deployed into `millrace-agents/`.
pub const RUNTIME_ASSET_FAMILIES: &[&str] = &[
    "entrypoints",
    "skills",
    "modes",
    "loops",
    "graphs",
    "registry",
];

/// One embedded managed runtime asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeAsset {
    /// Runtime-root-relative path using `/` separators.
    pub relative_path: &'static str,
    /// Top-level managed asset family.
    pub asset_family: &'static str,
    /// Original packaged file bytes.
    pub contents: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/managed_assets.rs"));

/// Returns the embedded managed runtime asset list in deterministic path order.
#[must_use]
pub fn runtime_assets() -> &'static [RuntimeAsset] {
    RUNTIME_ASSETS
}
