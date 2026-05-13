use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use assert_cmd::cargo::cargo_bin;
use serde_json::Value;
use tempfile::TempDir;

#[derive(Debug)]
pub struct CommandOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn assert_success(&self) {
        assert!(
            self.status.success(),
            "command failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
            self.status.code(),
            self.stdout,
            self.stderr
        );
    }

    pub fn stdout_trimmed(&self) -> &str {
        self.stdout.trim()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct VersionLine {
    pub binary_name: String,
    pub version: String,
}

#[derive(Debug)]
pub struct ParityWorkspace {
    _temp_dir: TempDir,
    python_workspace: PathBuf,
    rust_workspace: PathBuf,
}

impl ParityWorkspace {
    pub fn new() -> std::io::Result<Self> {
        let temp_dir = TempDir::new()?;
        let python_workspace = temp_dir.path().join("python-workspace");
        let rust_workspace = temp_dir.path().join("rust-workspace");

        std::fs::create_dir_all(&python_workspace)?;
        std::fs::create_dir_all(&rust_workspace)?;

        Ok(Self {
            _temp_dir: temp_dir,
            python_workspace,
            rust_workspace,
        })
    }

    pub fn python_workspace(&self) -> &Path {
        &self.python_workspace
    }

    pub fn rust_workspace(&self) -> &Path {
        &self.rust_workspace
    }
}

pub fn parse_version_line(line: &str) -> Option<VersionLine> {
    let mut parts = line.split_whitespace();
    let binary_name = parts.next()?;
    let version = parts.next()?;

    if parts.next().is_some() {
        return None;
    }

    Some(VersionLine {
        binary_name: binary_name.to_owned(),
        version: version.to_owned(),
    })
}

pub fn python_project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../millrace-py")
}

pub fn fixture_path(relative_path: impl AsRef<Path>) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(relative_path)
}

pub fn read_fixture(relative_path: impl AsRef<Path>) -> std::io::Result<String> {
    fs::read_to_string(fixture_path(relative_path))
}

pub fn read_json_fixture(relative_path: impl AsRef<Path>) -> Value {
    let path = relative_path.as_ref().to_owned();
    let contents = read_fixture(&path).unwrap_or_else(|error| {
        panic!("read JSON fixture {}: {error}", path.display());
    });
    serde_json::from_str(&contents).unwrap_or_else(|error| {
        panic!("parse JSON fixture {}: {error}", path.display());
    })
}

pub fn run_rust_millrace<I, S>(args: I) -> std::io::Result<CommandOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(cargo_bin("millrace")).args(args).output()?;

    Ok(CommandOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

pub fn run_rust_millrace_with_env<I, S, E, K, V>(args: I, envs: E) -> std::io::Result<CommandOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    E: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    let mut command = Command::new(cargo_bin("millrace"));
    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output()?;

    Ok(CommandOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

pub fn run_python_reference_version_probe() -> std::io::Result<CommandOutput> {
    let project_root = python_project_root();
    let src_root = project_root.join("src");
    let snippet = format!(
        "import sys; sys.path.insert(0, r'{}'); import millrace_ai; print(f'millrace {{millrace_ai.__version__}}')",
        src_root.display()
    );

    let output = Command::new(python_binary())
        .args(["-c", &snippet])
        .output()?;

    Ok(CommandOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

pub fn run_python_reference_cli<I, S>(args: I) -> std::io::Result<CommandOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let project_root = python_project_root();
    let src_root = project_root.join("src");
    let mut command = Command::new(python_binary());

    command
        .env("PYTHONPATH", src_root)
        .args(["-m", "millrace_ai"])
        .args(args);

    let output = command.output()?;

    Ok(CommandOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn python_binary() -> String {
    std::env::var("MILLRACE_PYTHON").unwrap_or_else(|_| "python".to_owned())
}
