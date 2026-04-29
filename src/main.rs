use std::process::ExitCode;

fn main() -> ExitCode {
    millrace_ai::cli::run(std::env::args())
}
