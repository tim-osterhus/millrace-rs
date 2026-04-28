fn main() {
    let status = millrace_ai::runtime_status();

    println!("Millrace Rust runtime {}", status.version);
    println!("package: {}", status.package_name);
    println!("crate: {}", status.crate_name);
    println!("binary: {}", status.cli_name);
    println!("status: {}", status.stability);
    println!("production runtime: Python package millrace-ai");
}
