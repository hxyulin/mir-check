#![forbid(unsafe_code)]

use std::process::{Command, ExitCode};

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let compiler = args
        .next()
        .ok_or("expected a compiler or workspace wrapper")?;
    let status = Command::new(compiler)
        .args(args)
        .args(["-Zalways-encode-mir=yes", "-Zmir-opt-level=0"])
        .status()?;
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("miren-rustc: {error}");
            ExitCode::FAILURE
        }
    }
}
