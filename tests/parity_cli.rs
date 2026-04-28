mod support;

use support::parity::{
    ParityWorkspace, parse_version_line, run_python_reference_cli,
    run_python_reference_version_probe, run_rust_millrace,
};

#[test]
fn rust_version_command_has_millrace_shape() {
    let output = run_rust_millrace(["--version"]).expect("run Rust millrace --version");

    output.assert_success();

    let version_line =
        parse_version_line(output.stdout_trimmed()).expect("parse Rust version line");
    assert_eq!(version_line.binary_name, "millrace");
    assert_eq!(version_line.version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn rust_version_subcommand_matches_version_flag() {
    let flag = run_rust_millrace(["--version"]).expect("run Rust millrace --version");
    let subcommand = run_rust_millrace(["version"]).expect("run Rust millrace version");

    flag.assert_success();
    subcommand.assert_success();
    assert_eq!(flag.stdout_trimmed(), subcommand.stdout_trimmed());
}

#[test]
fn python_reference_version_probe_is_pinned_to_0_16_1() {
    let output = run_python_reference_version_probe().expect("run Python reference version probe");

    output.assert_success();

    let version_line =
        parse_version_line(output.stdout_trimmed()).expect("parse Python version line");
    assert_eq!(version_line.binary_name, "millrace");
    assert_eq!(version_line.version, "0.16.1");
}

#[test]
fn version_shape_matches_python_reference_even_when_versions_differ() {
    let rust = run_rust_millrace(["--version"]).expect("run Rust millrace --version");
    let python = run_python_reference_version_probe().expect("run Python reference version probe");

    rust.assert_success();
    python.assert_success();

    let rust_line = parse_version_line(rust.stdout_trimmed()).expect("parse Rust version line");
    let python_line =
        parse_version_line(python.stdout_trimmed()).expect("parse Python version line");

    assert_eq!(rust_line.binary_name, python_line.binary_name);
    assert_ne!(rust_line.version, python_line.version);
}

#[test]
fn parity_workspace_fixture_does_not_initialize_millrace() {
    let workspace = ParityWorkspace::new().expect("create parity workspace fixture");

    assert!(
        !workspace
            .python_workspace()
            .join("millrace-agents")
            .exists()
    );
    assert!(!workspace.rust_workspace().join("millrace-agents").exists());
}

#[test]
#[ignore = "requires a Python environment with millrace-ai CLI dependencies installed"]
fn python_reference_cli_probe() {
    let output = run_python_reference_cli(["--version"]).expect("run Python reference CLI");

    output.assert_success();
    assert_eq!(output.stdout_trimmed(), "millrace 0.16.1");
}
