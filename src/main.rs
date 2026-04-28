use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);

    match args.next().as_deref() {
        Some("--version" | "-V" | "version") => {
            println!("millrace {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--status" | "about") | None => {
            print_status();
            ExitCode::SUCCESS
        }
        Some(command) => {
            eprintln!("error: unknown command `{command}`");
            ExitCode::from(2)
        }
    }
}

fn print_status() {
    let status = millrace_ai::runtime_status();

    println!("Millrace Rust runtime {}", status.version);
    println!("package: {}", status.package_name);
    println!("crate: {}", status.crate_name);
    println!("binary: {}", status.cli_name);
    println!("status: {}", status.stability);
    println!("production runtime: Python package millrace-ai");
}
