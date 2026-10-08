#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    match mir_check::project::run(std::env::args_os().skip(1).collect()) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("mir-check: {error}");
            ExitCode::FAILURE
        }
    }
}
