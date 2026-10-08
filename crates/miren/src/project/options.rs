use crate::AnalysisLimits;
use crate::limits::LimitOption;
use serde::Deserialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(super) const HELP: &str = "Usage: miren [check] [checker and cargo options]\n\
    miren --entry module::function --lib\n\
    miren --async-entry task --bin firmware\n\
    miren inventory --lib\n\
    miren report\n\n\
    Checks the current Cargo project by default; no saved invocation is needed.\n\
    Cargo supplies the target, profile, features, dependency MIR and build environment.\n\
    Optional miren.json stores entries, Cargo arguments and explicit limit overrides.\n\
    Use --config FILE or --no-config to control project configuration.\n\
    --entry FUNCTION selects ordinary roots; --async-entry FACTORY checks construction + polls.\n\
    --startup and --allow-assumptions stay explicit; --replay opts into native execution.\n\
    --verbose shows full details; --quiet hides progress; --jsonl FILE|- exports raw reports.\n\
    Cargo arguments such as -p, --bin, --lib, --target and --features retain their meaning.\n\
    cargo miren uses the same project workflow.\n\
    Advanced: miren rustc [checker options] -- <rustc arguments>\n\
    Advanced: miren --verify --from-report FILE rechecks a captured invocation.";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    schema_version: u32,
    #[serde(default)]
    entries: Vec<String>,
    #[serde(default)]
    async_entries: Vec<String>,
    #[serde(default)]
    cargo_args: Vec<String>,
    #[serde(default)]
    limits: Limits,
    contracts: Option<PathBuf>,
    dependency_mir: Option<bool>,
    allow_assumptions: Option<bool>,
    all_failures: Option<bool>,
    induction: Option<bool>,
    startup: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Limits {
    max_steps: Option<u64>,
    max_call_depth: Option<u64>,
    max_query_bytes: Option<u64>,
    root_timeout_secs: Option<u64>,
    solver_timeout_ms: Option<u64>,
}

pub(super) struct Arguments {
    pub(super) args: Vec<OsString>,
    pub(super) cargo_args: Vec<OsString>,
}

pub(super) fn configure(mut args: Vec<OsString>) -> Result<Arguments, String> {
    let inventory = args.first().is_some_and(|arg| arg == "inventory");
    if args.first().is_some_and(|arg| arg == "check") || inventory {
        args.remove(0);
    }
    let mut config = None;
    let mut no_config = false;
    let mut manifest = None;
    let mut remaining = Vec::new();
    let mut arguments = args.into_iter();
    while let Some(arg) = arguments.next() {
        match arg.to_str() {
            Some("--") => {
                remaining.push(arg);
                remaining.extend(arguments);
                break;
            }
            Some("--config") => {
                if config.is_some() {
                    return Err("specify --config only once".into());
                }
                config = Some(PathBuf::from(
                    arguments.next().ok_or("--config needs a path")?,
                ));
            }
            Some(value) if value.starts_with("--config=") => {
                if config.is_some() {
                    return Err("specify --config only once".into());
                }
                if value[9..].is_empty() {
                    return Err("--config needs a path".into());
                }
                config = Some(PathBuf::from(&value[9..]));
            }
            Some("--no-config") => no_config = true,
            Some("--manifest-path") => {
                let path = arguments.next().ok_or("--manifest-path needs a path")?;
                manifest = Some(PathBuf::from(&path));
                remaining.extend([arg, path]);
            }
            Some(value) if value.starts_with("--manifest-path=") => {
                manifest = Some(PathBuf::from(&value[16..]));
                remaining.push(arg);
            }
            _ => remaining.push(arg),
        }
    }
    if config.is_some() && no_config {
        return Err("choose --config or --no-config".into());
    }
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    let start = manifest
        .as_ref()
        .map(|path| cwd.join(path))
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or(cwd);
    let start = start.canonicalize().unwrap_or(start);
    let path = if no_config {
        None
    } else {
        config.or_else(|| find_configuration(&start))
    };
    let mut expanded = Vec::new();
    let mut cargo_args = Vec::new();
    if let Some(path) = path {
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("configuration {}: {error}", path.display()))?;
        let configuration: Configuration = serde_json::from_slice(&bytes)
            .map_err(|error| format!("configuration {}: {error}", path.display()))?;
        configuration.validate()?;
        let cli_roots = remaining.iter().any(|arg| {
            let arg = arg.to_string_lossy();
            arg == "--entry"
                || arg == "--async-entry"
                || arg.starts_with("--entry=")
                || arg.starts_with("--async-entry=")
        });
        configuration.expand(
            &mut expanded,
            path.parent().unwrap_or(Path::new(".")),
            !cli_roots && !inventory,
            !inventory,
        );
        cargo_args.extend(configuration.cargo_args.iter().map(OsString::from));
    }
    if inventory {
        expanded.push("--inventory".into());
    }
    expanded.extend(remaining);
    Ok(Arguments {
        args: expanded,
        cargo_args,
    })
}

fn find_configuration(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|path| path.join("miren.json"))
        .find(|path| path.is_file())
}

impl Configuration {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("project configuration needs schema_version 1".into());
        }
        for entry in self.entries.iter().chain(&self.async_entries) {
            if entry.is_empty() || entry.starts_with('-') || entry.trim() != entry {
                return Err("configured entries require exact nonempty function names".into());
            }
        }
        let mut limits = AnalysisLimits::default();
        for (option, value) in self.limits.values() {
            option.apply(&mut limits, &value.to_string())?;
        }
        Ok(())
    }

    fn expand(&self, args: &mut Vec<OsString>, base: &Path, roots: bool, verify: bool) {
        if roots {
            for (flag, entries) in [
                ("--entry", &self.entries),
                ("--async-entry", &self.async_entries),
            ] {
                for entry in entries {
                    args.extend([flag.into(), entry.into()]);
                }
            }
        }
        for (option, value) in self.limits.values() {
            args.extend([option.flag().into(), value.to_string().into()]);
        }
        for (value, yes, no) in [
            (
                self.dependency_mir,
                "--dependency-mir",
                "--no-dependency-mir",
            ),
            (
                self.allow_assumptions,
                "--allow-assumptions",
                "--deny-assumptions",
            ),
            (self.all_failures, "--all-failures", "--first-failure"),
            (
                if verify { self.induction } else { None },
                "--induction",
                "--no-induction",
            ),
            (
                if verify { self.startup } else { None },
                "--startup",
                "--no-startup",
            ),
        ] {
            if let Some(value) = value {
                args.push(if value { yes } else { no }.into());
            }
        }
        if let Some(path) = &self.contracts {
            args.extend(["--contracts".into(), base.join(path).into_os_string()]);
        }
    }
}

impl Limits {
    fn values(&self) -> Vec<(LimitOption, u64)> {
        [
            (LimitOption::Steps, self.max_steps),
            (LimitOption::CallDepth, self.max_call_depth),
            (LimitOption::QueryBytes, self.max_query_bytes),
            (LimitOption::RootTimeout, self.root_timeout_secs),
            (LimitOption::SolverTimeout, self.solver_timeout_ms),
        ]
        .into_iter()
        .filter_map(|(option, value)| value.map(|value| (option, value)))
        .collect()
    }
}
