#![forbid(unsafe_code)]

use mir_checker::Report;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "mir-checker") {
        args.remove(0);
    }
    if args
        .first()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "Usage: cargo mir-checker [cargo check arguments]\n\
            Analyzes workspace members with a pinned compiler and writes JSON reports.\n\
            Each run uses a fresh target directory. No properties are proved yet."
        );
        return Ok(ExitCode::SUCCESS);
    }
    if args
        .iter()
        .any(|arg| arg == "--target-dir" || arg.to_string_lossy().starts_with("--target-dir="))
    {
        return Err("--target-dir is managed by mir-checker to prevent stale inventories".into());
    }
    let run_id = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::current_dir()?.join(format!("target/mir-checker/{run_id}"));
    let reports = root.join("reports");
    std::fs::create_dir_all(&reports)?;
    let driver = std::env::current_exe()?.with_file_name("mir-checker");
    if !driver.is_file() {
        return Err("mir-checker must be installed beside cargo-mir-checker".into());
    }
    let sysroot = PathBuf::from(env!("MIR_CHECKER_SYSROOT"));
    let status = Command::new(sysroot.join("bin/cargo"))
        .arg("check")
        .args(args)
        .arg("--target-dir")
        .arg(root.join("build"))
        .env("RUSTC", sysroot.join("bin/rustc"))
        .env("RUSTC_WORKSPACE_WRAPPER", driver)
        .env_remove("RUSTC_WRAPPER")
        .env("MIR_CHECKER_REPORT_DIR", &reports)
        .env("CARGO_INCREMENTAL", "0")
        .status()?;
    if !status.success() {
        return Ok(ExitCode::FAILURE);
    }
    let mut paths = std::fs::read_dir(&reports)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    if paths.is_empty() {
        return Err("Cargo produced no inventories; check the selected targets".into());
    }
    for path in paths {
        let report: Report = serde_json::from_slice(&std::fs::read(path)?)?;
        print!("{}", mir_checker::render(&report));
    }
    println!("JSON reports: {}", reports.display());
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    match run() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("mir-checker: {error}");
            ExitCode::FAILURE
        }
    }
}
