#![doc = include_str!("../README.md")]

/// The crates.io package name.
pub const PACKAGE_NAME: &str = "millrace-ai";

/// The Rust library crate name.
pub const CRATE_NAME: &str = "millrace_ai";

/// The command-line binary name installed by this package.
pub const CLI_NAME: &str = "millrace";

/// The current development status of the Rust runtime.
pub const STABILITY: &str = "experimental";

/// Basic metadata for the Rust implementation of Millrace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeStatus {
    /// The crates.io package name.
    pub package_name: &'static str,
    /// The Rust library crate name.
    pub crate_name: &'static str,
    /// The command-line binary name.
    pub cli_name: &'static str,
    /// The Cargo package version.
    pub version: &'static str,
    /// The implementation stability label.
    pub stability: &'static str,
}

impl RuntimeStatus {
    /// Returns metadata for the currently compiled package.
    #[must_use]
    pub const fn current() -> Self {
        Self {
            package_name: PACKAGE_NAME,
            crate_name: CRATE_NAME,
            cli_name: CLI_NAME,
            version: env!("CARGO_PKG_VERSION"),
            stability: STABILITY,
        }
    }
}

/// Returns metadata for the currently compiled package.
#[must_use]
pub const fn runtime_status() -> RuntimeStatus {
    RuntimeStatus::current()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_expected_names() {
        let status = runtime_status();

        assert_eq!(status.package_name, "millrace-ai");
        assert_eq!(status.crate_name, "millrace_ai");
        assert_eq!(status.cli_name, "millrace");
        assert_eq!(status.stability, "experimental");
    }
}
