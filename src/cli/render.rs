use std::process::ExitCode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    exit_code: u8,
    stdout_lines: Vec<String>,
    stderr_lines: Vec<String>,
}

impl CliOutput {
    pub fn success(lines: Vec<String>) -> Self {
        Self {
            exit_code: 0,
            stdout_lines: lines,
            stderr_lines: Vec::new(),
        }
    }

    pub fn stdout_failure(message: impl Into<String>) -> Self {
        Self {
            exit_code: 1,
            stdout_lines: vec![format!("error: {}", message.into())],
            stderr_lines: Vec::new(),
        }
    }

    pub fn stderr_failure(message: impl Into<String>) -> Self {
        Self {
            exit_code: 1,
            stdout_lines: Vec::new(),
            stderr_lines: vec![format!("error: {}", message.into())],
        }
    }

    pub fn parse_error(message: impl Into<String>) -> Self {
        Self {
            exit_code: 2,
            stdout_lines: Vec::new(),
            stderr_lines: vec![format!("error: {}", message.into())],
        }
    }

    pub fn with_exit_code(lines: Vec<String>, exit_code: u8) -> Self {
        Self {
            exit_code,
            stdout_lines: lines,
            stderr_lines: Vec::new(),
        }
    }
}

pub fn render_output(output: CliOutput) -> ExitCode {
    for line in output.stdout_lines {
        println!("{line}");
    }
    for line in output.stderr_lines {
        eprintln!("{line}");
    }
    ExitCode::from(output.exit_code)
}
