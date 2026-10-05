#![forbid(unsafe_code)]

use mir_check::Report;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "mir-check") {
        args.remove(0);
    }
    if args
        .first()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "Usage: cargo mir-check [--verify] [--summary] [--entry FUNCTION] \
            [--no-dependency-mir] \
            [cargo check arguments]\n\
            Analyzes workspace members with a pinned compiler and writes JSON reports.\n\
            Repeat --entry to select exact or crate-qualified roots; missing roots fail.\n\
            Dependency MIR is retained by default; --no-dependency-mir disables retention.\n\
            Without --entry, --verify requires all local bodies to pass."
        );
        return Ok(ExitCode::SUCCESS);
    }
    let mut verify = false;
    let mut summary = false;
    let mut dependency_mir = true;
    let mut entries = Vec::new();
    let mut cargo_args = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--verify") => verify = true,
            Some("--summary") => summary = true,
            Some("--no-dependency-mir") => dependency_mir = false,
            Some("--entry") => {
                let name = args.next().ok_or("--entry requires a function name")?;
                entries.push(entry_name(name)?);
            }
            Some("--") => {
                cargo_args.extend(args);
                break;
            }
            Some(text) if text.starts_with("--entry=") => {
                entries.push(entry_name(OsString::from(&text[8..]))?);
            }
            _ => cargo_args.push(arg),
        }
    }
    if cargo_args
        .iter()
        .any(|arg| arg == "--target-dir" || arg.to_string_lossy().starts_with("--target-dir="))
    {
        return Err("--target-dir is managed by mir-check to prevent stale inventories".into());
    }
    let run_id = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::current_dir()?.join(format!("target/mir-check/{run_id}"));
    let reports = root.join("reports");
    std::fs::create_dir_all(&reports)?;
    let driver = std::env::current_exe()?.with_file_name("mir-check");
    if !driver.is_file() {
        return Err("mir-check must be installed beside cargo-mir-check".into());
    }
    let compiler_wrapper = driver.with_file_name("mir-check-rustc");
    if dependency_mir && !compiler_wrapper.is_file() {
        return Err("mir-check-rustc must be installed beside cargo-mir-check".into());
    }
    let sysroot = PathBuf::from(env!("MIR_CHECK_SYSROOT"));
    let mut command = Command::new(sysroot.join("bin/cargo"));
    command
        .arg("check")
        .args(cargo_args)
        .arg("--target-dir")
        .arg(root.join("build"))
        .env("RUSTC", sysroot.join("bin/rustc"))
        .env("RUSTC_WORKSPACE_WRAPPER", driver)
        .env_remove("RUSTC_WRAPPER")
        .env("MIR_CHECK_REPORT_DIR", &reports)
        .env("MIR_CHECK_ENTRIES", serde_json::to_string(&entries)?)
        .env("CARGO_INCREMENTAL", "0");
    if dependency_mir {
        command.env("RUSTC_WRAPPER", compiler_wrapper);
    }
    if verify {
        command.env("MIR_CHECK_VERIFY", "1");
    } else {
        command.env_remove("MIR_CHECK_VERIFY");
    }
    let status = command.status()?;
    let mut paths = std::fs::read_dir(&reports)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    if paths.is_empty() {
        if !status.success() {
            eprintln!("JSON reports: {}", reports.display());
            return Ok(ExitCode::FAILURE);
        }
        return Err("Cargo produced no inventories; check the selected targets".into());
    }
    let mut collected = Vec::new();
    for path in paths {
        let report: Report = serde_json::from_slice(&std::fs::read(path)?)?;
        let rendered = if summary {
            mir_check::render_coverage(&report)
        } else {
            mir_check::render(&report)
        };
        print!("{rendered}");
        collected.push(report);
    }
    let mut missing = false;
    for entry in entries {
        if !collected.iter().any(|report| {
            report.functions.iter().any(|function| {
                mir_check::entry_matches(&report.crate_name, &function.name, &entry)
            })
        }) {
            eprintln!("mir-check: entry {entry:?} has no inventoried MIR body in selected targets");
            missing = true;
        }
    }
    if status.success() && !missing {
        println!("JSON reports: {}", reports.display());
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!("JSON reports: {}", reports.display());
        Ok(ExitCode::FAILURE)
    }
}

fn entry_name(name: OsString) -> Result<String, Box<dyn std::error::Error>> {
    let name = name
        .into_string()
        .map_err(|_| "entry names must be UTF-8")?;
    if name.is_empty() || name.starts_with('-') {
        return Err("--entry requires a function name".into());
    }
    Ok(name)
}

fn main() -> ExitCode {
    match run() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("mir-check: {error}");
            ExitCode::FAILURE
        }
    }
}
