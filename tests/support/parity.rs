use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use assert_cmd::cargo::cargo_bin;
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
