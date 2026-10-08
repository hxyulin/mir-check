#![forbid(unsafe_code)]

use mir_check::cli::{Color, Progress};
use mir_check::limits::LimitOption;
use mir_check::{AnalysisLimits, ContractConfig, Report};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "mir-check") {
        args.remove(0);
    }
    if args.first().is_some_and(|arg| arg == "report") {
        let args = args[1..]
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        return Ok(if mir_check::cli::report_command(&args)? {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    if args
        .first()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "Usage: cargo mir-check [--verify] [--summary] [--entry FUNCTION] \
            [--async-entry FACTORY] \
            [--no-dependency-mir] [--contracts FILE] [--allow-assumptions] \
            [--all-failures] [--induction] [--startup] [--replay] \
            [--verbose] [--color auto|always|never] [--quiet] [--jsonl FILE|-] \
            [cargo check arguments]\n\
            Analyzes workspace members with a pinned compiler and writes JSON reports.\n\
            Repeat --entry to select exact or crate-qualified roots; missing roots fail.\n\
            --async-entry polls a fresh future factory through Pending until Ready.\n\
            Unsupported inputs or exhausted execution limits remain UNKNOWN.\n\
            Dependency MIR is retained by default; --no-dependency-mir disables retention.\n\
            Without --entry, --verify requires all local bodies to pass.\n\
            --induction uses experimental Spacer proofs for supported cyclic root bodies.\n\
            --startup assumes fresh statics for a zero-argument root; reports assumptions.\n\
            Refuted roots stop at their first counterexample; --all-failures continues them.\n\
            --replay compiles and executes supported native inputs after the build.\n\
            Default output is compact; --verbose shows the full inventory and obligations.\n\
            cargo mir-check report <file or directory> reads saved JSON/JSONL reports.\n\n{}",
            mir_check::limits::HELP
        );
        return Ok(ExitCode::SUCCESS);
    }
    let mut verify = false;
    let mut verbose = false;
    let mut color = Color::Auto;
    let mut quiet = false;
    let mut jsonl = None;
    let mut dependency_mir = true;
    let mut entries = Vec::new();
    let mut async_entries = Vec::new();
    let mut contracts_path = None;
    let mut allow_assumptions = false;
    let mut all_failures = false;
    let mut induction = false;
    let mut startup = false;
    let mut replay = false;
    let mut limits = AnalysisLimits::default();
    let mut cargo_args = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some(value) if LimitOption::parse(value).is_some() => {
                let (option, inline) = LimitOption::parse(value).expect("matched limit option");
                let value = match inline {
                    Some(value) => value.to_owned(),
                    None => args
                        .next()
                        .ok_or_else(|| format!("{} needs a value", option.flag()))?
                        .into_string()
                        .map_err(|_| format!("{} needs a UTF-8 integer", option.flag()))?,
                };
                option.apply(&mut limits, &value)?;
            }
            Some("--verify") => verify = true,
            Some("--summary") => verbose = false,
            Some("--verbose") => verbose = true,
            Some("--quiet") => {
                quiet = true;
                cargo_args.push(arg);
            }
            Some("--color") => {
                color = Color::parse(
                    &args
                        .next()
                        .ok_or("--color needs a value")?
                        .to_string_lossy(),
                )?
            }
            Some(value) if value.starts_with("--color=") => color = Color::parse(&value[8..])?,
            Some("--jsonl") => {
                jsonl = Some(mir_check::cli::output_path(
                    &args.next().ok_or("--jsonl needs a path")?.to_string_lossy(),
                )?)
            }
            Some("--no-dependency-mir") => dependency_mir = false,
            Some("--allow-assumptions") => allow_assumptions = true,
            Some("--all-failures") => all_failures = true,
            Some("--induction") => induction = true,
            Some("--startup") => startup = true,
            Some("--replay") => replay = true,
            Some("--contracts") => {
                let path = args.next().ok_or("--contracts requires a JSON file")?;
                contracts_path = Some(std::fs::canonicalize(PathBuf::from(path))?);
            }
            Some("--async-entry") => {
                let name = args.next().ok_or("--async-entry requires a factory name")?;
                let name = entry_name(name)?;
                entries.push(name.clone());
                async_entries.push(name);
            }
            Some(text) if text.starts_with("--async-entry=") => {
                let name = entry_name(OsString::from(&text[14..]))?;
                entries.push(name.clone());
                async_entries.push(name);
            }
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
    if !async_entries.is_empty() && !verify {
        return Err("--async-entry requires --verify".into());
    }
    if startup && !verify {
        return Err("--startup requires --verify".into());
    }
    if replay && !verify {
        return Err("--replay requires --verify".into());
    }
    let config = contracts_path
        .as_ref()
        .map(|path| ContractConfig::read(path))
        .transpose()?;
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
        .args(["--color", color.argument()])
        .args(cargo_args)
        .arg("--target-dir")
        .arg(root.join("build"))
        .env("RUSTC", sysroot.join("bin/rustc"))
        .env("RUSTC_WORKSPACE_WRAPPER", driver)
        .env_remove("RUSTC_WRAPPER")
        .env("MIR_CHECK_REPORT_DIR", &reports)
        .env("MIR_CHECK_ENTRIES", serde_json::to_string(&entries)?)
        .env(
            "MIR_CHECK_ASYNC_ENTRIES",
            serde_json::to_string(&async_entries)?,
        )
        .env("CARGO_INCREMENTAL", "0");
    command.env("MIR_CHECK_LIMITS", serde_json::to_string(&limits)?);
    command.env("MIR_CHECK_COLOR", color.argument());
    if quiet {
        command.env("MIR_CHECK_QUIET", "1");
    } else {
        command.env_remove("MIR_CHECK_QUIET");
    }
    command
        .env_remove("MIR_CHECK_CONTRACTS")
        .env_remove("MIR_CHECK_ALLOW_ASSUMPTIONS")
        .env_remove("MIR_CHECK_ALL_FAILURES")
        .env_remove("MIR_CHECK_INDUCTION")
        .env_remove("MIR_CHECK_STARTUP");
    if startup {
        command.env("MIR_CHECK_STARTUP", "1");
    }
    if induction {
        command.env("MIR_CHECK_INDUCTION", "1");
    }
    if all_failures {
        command.env("MIR_CHECK_ALL_FAILURES", "1");
    }
    if let Some(path) = contracts_path {
        command.env("MIR_CHECK_CONTRACTS", path);
    }
    if allow_assumptions {
        command.env("MIR_CHECK_ALLOW_ASSUMPTIONS", "1");
    }
    if dependency_mir {
        command.env("RUSTC_WRAPPER", compiler_wrapper);
    }
    if verify {
        command.env("MIR_CHECK_VERIFY", "1");
    } else {
        command.env_remove("MIR_CHECK_VERIFY");
    }
    if jsonl.as_deref() == Some(std::path::Path::new("-")) {
        command.stdout(std::process::Stdio::from(std::io::stderr()));
    }
    let build_progress = Progress::new(
        !quiet,
        color,
        "Building workspace and analyzing selected roots".to_owned(),
    );
    let status = command.status()?;
    let build_elapsed_s = build_progress.elapsed();
    drop(build_progress);
    let mut paths = std::fs::read_dir(&reports)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    if paths.is_empty() {
        if !status.success() {
            eprintln!("mir-check: Build failed before any crate reports were produced.");
            eprintln!("JSON reports: {}", reports.display());
            return Ok(ExitCode::FAILURE);
        }
        return Err("Cargo produced no inventories; check the selected targets".into());
    }
    let mut collected = Vec::new();
    let report_progress = Progress::new(
        !quiet,
        color,
        format!("Reading {} crate reports", paths.len()),
    );
    for (index, path) in paths.iter().enumerate() {
        report_progress.update(format!(
            "Reading [{}/{}] {}",
            index + 1,
            paths.len(),
            path.display()
        ));
        let mut report: Report = serde_json::from_slice(&std::fs::read(path)?)?;
        if replay {
            report_progress.update(format!("Compiling and replaying {}", report.crate_name));
            mir_check::replay::replay_report(&mut report);
            std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
        }
        collected.push(report);
    }
    let elapsed_s = build_elapsed_s + report_progress.elapsed();
    drop(report_progress);
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
    if let Some(config) = config {
        for spec in config.functions {
            if !collected
                .iter()
                .any(|report| report.matched_contracts.contains(&spec.selector()))
            {
                eprintln!(
                    "mir-check: contract selector {:?} matched no definition or analyzed call",
                    spec.selector()
                );
                missing = true;
            }
        }
    }
    let success = status.success()
        && !missing
        && collected
            .iter()
            .all(|report| mir_check::cli::accepted(report, allow_assumptions));
    mir_check::cli::present(
        &collected,
        verbose,
        color,
        jsonl.as_deref(),
        success,
        elapsed_s,
        false,
    )?;
    if success {
        if jsonl.as_deref() == Some(std::path::Path::new("-")) {
            eprintln!("JSON reports: {}", reports.display());
        } else {
            println!("JSON reports: {}", reports.display());
        }
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
