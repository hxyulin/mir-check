#![feature(rustc_private)]
#![forbid(unsafe_code)]

extern crate rustc_abi;
extern crate rustc_attr_ir;
extern crate rustc_const_eval;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

use mir_check::cli::{Color, Progress};
use mir_check::{
    Contract, ContractConfig, ContractKind, ContractStatus, Function, ProofStatus, Report, Source,
};
use rustc_attr_ir::HasAttrs;
use rustc_driver::{Callbacks, Compilation};
use rustc_hir::def::DefKind;
use rustc_interface::interface;
use rustc_middle::ty::TyCtxt;
use rustc_span::Span;
use std::path::PathBuf;
use std::process::ExitCode;

mod contracts;
mod inventory;
mod proof;
mod solver;
mod symbolic;

struct Checker {
    json: bool,
    report_dir: Option<PathBuf>,
    error: Option<String>,
    entries: Vec<String>,
    rustc_arguments: Vec<String>,
    verify: bool,
    verbose: bool,
    color: Color,
    quiet: bool,
    jsonl: Option<PathBuf>,
    started: std::time::Instant,
    analysis_ran: bool,
    contracts_path: Option<PathBuf>,
    contract_config: Option<ContractConfig>,
    allow_assumptions: bool,
}

impl Callbacks for Checker {
    fn config(&mut self, config: &mut interface::Config) {
        // Preserve checks and prevent MIR inlining from changing the inventory boundary.
        config.opts.unstable_opts.mir_opt_level = Some(0);
    }

    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        let mut report = collect(tcx, &self.rustc_arguments);
        tcx.dcx().abort_if_errors();
        self.analysis_ran = true;
        let progress = Progress::new(
            !self.quiet,
            self.color,
            format!("Analyzing {} [{}]", report.crate_name, report.target),
        );
        report.contract_config = self.contract_config.clone();
        if let Some(config) = &self.contract_config {
            for function in &mut report.functions {
                let name = format!("{}::{}", report.crate_name, function.name);
                let id = tcx
                    .mir_keys(())
                    .iter()
                    .find(|id| tcx.def_path_str(id.to_def_id()) == function.name)
                    .expect("inventoried local function");
                let instance = rustc_middle::ty::Instance::new_raw(
                    id.to_def_id(),
                    rustc_middle::ty::GenericArgs::identity_for_item(tcx, id.to_def_id()),
                );
                for spec in &config.functions {
                    if spec.function == name
                        && spec
                            .instance
                            .as_ref()
                            .is_none_or(|expected| *expected == format!("{:?}", instance.args))
                    {
                        function.contracts.extend(spec.metadata());
                        report.matched_contracts.push(spec.selector());
                    }
                }
            }
        }
        let missing = self.entries.iter().find(|entry| {
            !report
                .functions
                .iter()
                .any(|function| mir_check::entry_matches(&report.crate_name, &function.name, entry))
        });
        if self.report_dir.is_none()
            && let Some(entry) = missing
        {
            self.error = Some(format!("entry {entry:?} has no inventoried local MIR body"));
            if entry == "main" || entry.ends_with("::main") {
                let candidates = report
                    .functions
                    .iter()
                    .filter(|function| {
                        function.name.contains("main") && !function.name.contains("{closure#")
                    })
                    .take(8)
                    .map(|function| function.name.as_str())
                    .collect::<Vec<_>>();
                if !candidates.is_empty() {
                    self.error
                        .as_mut()
                        .expect("missing entry error")
                        .push_str(&format!(
                            "; main-related MIR names: {}. Select an exact name; macro-generated \
                         startup bodies are separate from async task execution",
                            candidates.join(", ")
                        ));
                }
            }
            return Compilation::Stop;
        }
        let entries: Vec<_> = report
            .functions
            .iter()
            .filter(|function| {
                self.entries.iter().any(|entry| {
                    mir_check::entry_matches(&report.crate_name, &function.name, entry)
                })
            })
            .map(|function| function.name.clone())
            .collect();
        match mir_check::build_traces(&mut report, &entries) {
            Ok(()) => {
                let config = self.contract_config.clone().unwrap_or_default();
                if self.verify {
                    let selected = report
                        .functions
                        .iter()
                        .filter(|function| {
                            self.entries.is_empty() || entries.contains(&function.name)
                        })
                        .count();
                    let mut completed = 0;
                    for function in &mut report.functions {
                        if !self.entries.is_empty() && !entries.contains(&function.name) {
                            continue;
                        }
                        progress.update(format!(
                            "{}: checking [{}/{}] {}",
                            report.crate_name,
                            completed + 1,
                            selected,
                            function.name
                        ));
                        let id = tcx
                            .mir_keys(())
                            .iter()
                            .find(|id| tcx.def_path_str(id.to_def_id()) == function.name)
                            .expect("inventoried functions have local MIR bodies");
                        function.proof = Some(proof::verify(tcx, id.to_def_id(), &config));
                        completed += 1;
                        if function.proof.as_ref().is_some_and(|proof| {
                            matches!(
                                proof.status,
                                ProofStatus::Proved | ProofStatus::ProvedWithAssumptions
                            )
                        }) {
                            for contract in &mut function.contracts {
                                contract.status = if function.proof.as_ref().is_some_and(|proof| {
                                    proof.status == ProofStatus::ProvedWithAssumptions
                                }) {
                                    ContractStatus::VerifiedWithTrustedAssumptions
                                } else {
                                    ContractStatus::VerifiedUnderPreconditions
                                };
                            }
                        }
                    }
                }
                tcx.dcx().abort_if_errors();
                for function in &report.functions {
                    if let Some(proof) = &function.proof {
                        report
                            .matched_contracts
                            .extend(proof.matched_contracts.iter().cloned());
                    }
                }
                report.matched_contracts.sort();
                report.matched_contracts.dedup();
                report.coverage = mir_check::coverage(&report);
                progress.update(format!("Writing report for {}", report.crate_name));
                if !mir_check::cli::accepted(&report, self.allow_assumptions) {
                    self.error = Some(
                        "verification failed; inspect failed obligations and trusted assumptions"
                            .to_owned(),
                    );
                }
                if self.report_dir.is_none()
                    && let Some(config) = &self.contract_config
                {
                    let missing = config
                        .functions
                        .iter()
                        .filter(|spec| !report.matched_contracts.contains(&spec.selector()))
                        .map(|spec| spec.selector())
                        .collect::<Vec<_>>();
                    if !missing.is_empty() {
                        self.error = Some(format!(
                            "contract selectors matched no definition or analyzed call: {}",
                            missing.join(", ")
                        ));
                    }
                }
                if let Err(error) = self.emit(&report) {
                    self.error = Some(error.to_string());
                }
            }
            Err(error) => self.error = Some(error),
        }
        let elapsed_s = progress.elapsed();
        drop(progress);
        if !self.quiet {
            eprintln!(
                "mir-check: Analyzed {} in {elapsed_s:.1}s",
                report.crate_name
            );
        }
        if self.report_dir.is_some() {
            Compilation::Continue
        } else {
            Compilation::Stop
        }
    }
}

impl Checker {
    fn emit(&self, report: &Report) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(directory) = &self.report_dir {
            std::fs::create_dir_all(directory)?;
            let path = directory.join(format!("{}-{}.json", report.crate_name, std::process::id()));
            std::fs::write(path, serde_json::to_vec_pretty(report)?)?;
        } else if self.json {
            println!("{}", serde_json::to_string_pretty(report)?);
        } else {
            mir_check::cli::present(
                std::slice::from_ref(report),
                self.verbose,
                self.color,
                self.jsonl.as_deref(),
                self.error.is_none(),
                self.started.elapsed().as_secs_f64(),
                false,
            )?;
        }
        Ok(())
    }
}

fn collect(tcx: TyCtxt<'_>, arguments: &[String]) -> Report {
    let mut functions = Vec::new();
    for &id in tcx.mir_keys(()) {
        if !matches!(
            tcx.def_kind(id),
            DefKind::Fn | DefKind::AssocFn | DefKind::Closure
        ) {
            continue;
        }
        let body = tcx.optimized_mir(id);
        let contracts = id
            .get_attrs(&tcx)
            .iter()
            .filter_map(|attribute| attribute.doc_str())
            .filter_map(|doc| parse_contract(doc.as_str()))
            .collect();
        let (sites, local_calls) = inventory::collect(tcx, body);
        functions.push(Function {
            name: tcx.def_path_str(id.to_def_id()),
            source: source(tcx, tcx.def_span(id)),
            basic_blocks: body.basic_blocks.len(),
            arguments: body.arg_count,
            contracts,
            sites,
            local_calls,
            proof: None,
        });
    }
    functions.sort_by(|left, right| left.name.cmp(&right.name));
    Report {
        schema_version: 8,
        compiler: env!("MIR_CHECK_COMPILER").to_owned(),
        crate_name: tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE).to_string(),
        target: tcx.sess.opts.target_triple.to_string(),
        panic_strategy: format!("{:?}", tcx.sess.panic_strategy()).to_lowercase(),
        overflow_checks: tcx.sess.overflow_checks(),
        mir_phase: "optimized_mir with mir-opt-level=0".to_owned(),
        rustc_arguments: arguments.to_vec(),
        functions,
        traces: Vec::new(),
        coverage: mir_check::Coverage::default(),
        contract_config: None,
        matched_contracts: Vec::new(),
    }
}

fn source(tcx: TyCtxt<'_>, span: Span) -> Source {
    let location = tcx.sess.source_map().lookup_char_pos(span.lo());
    Source {
        file: location
            .file
            .name
            .prefer_local_unconditionally()
            .to_string(),
        line: location.line,
        column: location.col.0 + 1,
    }
}

fn parse_contract(doc: &str) -> Option<Contract> {
    let payload = doc
        .strip_prefix("<!-- mir-check:v1:")?
        .strip_suffix(" -->")?;
    let (kind, predicate) = if payload == "no_panic" {
        (ContractKind::NoPanic, None)
    } else if let Some(predicate) = payload.strip_prefix("requires:") {
        (ContractKind::Requires, Some(predicate.to_owned()))
    } else {
        let predicate = payload.strip_prefix("ensures:")?;
        (ContractKind::Ensures, Some(predicate.to_owned()))
    };
    Some(Contract {
        kind,
        predicate,
        status: ContractStatus::PendingVerification,
    })
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "report") {
        return match mir_check::cli::report_command(&args[2..]) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(error) => {
                eprintln!("mir-check: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let color = match Color::parse(&std::env::var("MIR_CHECK_COLOR").unwrap_or("auto".to_owned())) {
        Ok(color) => color,
        Err(error) => {
            eprintln!("mir-check: {error}");
            return ExitCode::FAILURE;
        }
    };
    let report_dir = std::env::var_os("MIR_CHECK_REPORT_DIR").map(PathBuf::from);
    let entries = if report_dir.is_some() {
        match std::env::var("MIR_CHECK_ENTRIES") {
            Ok(value) => match serde_json::from_str(&value) {
                Ok(entries) => entries,
                Err(error) => {
                    eprintln!("mir-check: invalid MIR_CHECK_ENTRIES: {error}");
                    return ExitCode::FAILURE;
                }
            },
            Err(std::env::VarError::NotPresent) => Vec::new(),
            Err(error) => {
                eprintln!("mir-check: invalid MIR_CHECK_ENTRIES: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        Vec::new()
    };
    let mut checker = Checker {
        json: false,
        report_dir,
        error: None,
        entries,
        rustc_arguments: Vec::new(),
        verify: std::env::var_os("MIR_CHECK_VERIFY").is_some(),
        verbose: false,
        color,
        quiet: std::env::var_os("MIR_CHECK_QUIET").is_some(),
        jsonl: None,
        started: std::time::Instant::now(),
        analysis_ran: false,
        contracts_path: std::env::var_os("MIR_CHECK_CONTRACTS").map(PathBuf::from),
        contract_config: None,
        allow_assumptions: std::env::var_os("MIR_CHECK_ALLOW_ASSUMPTIONS").is_some(),
    };
    let mut from_report = None;
    if checker.report_dir.is_some() {
        if args.len() > 1 {
            args.remove(1); // Cargo's wrapper argument is the real rustc executable.
        }
    } else {
        if args
            .get(1)
            .is_some_and(|arg| arg == "--help" || arg == "-h")
            || args.len() == 1
        {
            println!(
                "Usage: mir-check [--json] [--summary] [--verify] [--entry FUNCTION] \
                [--contracts FILE] [--allow-assumptions] [--verbose] [--quiet] \
                [--color auto|always|never] [--jsonl FILE|-] -- \
                <rustc arguments>\n\
                Or: mir-check --verify --from-report FILE [--entry FUNCTION] [display options]\n\
                Without --entry, --verify checks every inventoried MIR body in the crate.\n\
                --from-report recompiles with saved arguments; it does not reuse saved proofs.\n\
                --verify proves panic safety for a restricted MIR subset; unknown proofs fail.\n\
                mir-check report <file or directory> reads saved JSON/JSONL reports."
            );
            return ExitCode::SUCCESS;
        }
        loop {
            match args.get(1).map(String::as_str) {
                Some("--json") => {
                    checker.json = true;
                    args.remove(1);
                }
                Some("--verify") => {
                    checker.verify = true;
                    args.remove(1);
                }
                Some("--summary") => {
                    checker.verbose = false;
                    args.remove(1);
                }
                Some("--verbose") => {
                    checker.verbose = true;
                    args.remove(1);
                }
                Some("--quiet") => {
                    checker.quiet = true;
                    args.remove(1);
                }
                Some("--color") => {
                    args.remove(1);
                    let Some(value) = args.get(1) else {
                        eprintln!("mir-check: --color needs a value");
                        return ExitCode::FAILURE;
                    };
                    checker.color = match Color::parse(value) {
                        Ok(color) => color,
                        Err(error) => {
                            eprintln!("mir-check: {error}");
                            return ExitCode::FAILURE;
                        }
                    };
                    args.remove(1);
                }
                Some(value) if value.starts_with("--color=") => {
                    checker.color = match Color::parse(&value[8..]) {
                        Ok(color) => color,
                        Err(error) => {
                            eprintln!("mir-check: {error}");
                            return ExitCode::FAILURE;
                        }
                    };
                    args.remove(1);
                }
                Some("--jsonl") => {
                    args.remove(1);
                    let Some(path) = args.get(1) else {
                        eprintln!("mir-check: --jsonl needs a path");
                        return ExitCode::FAILURE;
                    };
                    checker.jsonl = match mir_check::cli::output_path(path) {
                        Ok(path) => Some(path),
                        Err(error) => {
                            eprintln!("mir-check: {error}");
                            return ExitCode::FAILURE;
                        }
                    };
                    args.remove(1);
                }
                Some("--from-report") => {
                    args.remove(1);
                    if args.get(1).is_none_or(|arg| arg.starts_with('-')) {
                        eprintln!("mir-check: --from-report requires a JSON report file");
                        return ExitCode::FAILURE;
                    }
                    if from_report.is_some() {
                        eprintln!("mir-check: specify --from-report only once");
                        return ExitCode::FAILURE;
                    }
                    from_report = Some(PathBuf::from(args.remove(1)));
                }
                Some("--allow-assumptions") => {
                    checker.allow_assumptions = true;
                    args.remove(1);
                }
                Some("--contracts") => {
                    args.remove(1);
                    if args.get(1).is_none_or(|arg| arg.starts_with('-')) {
                        eprintln!("mir-check: --contracts requires a JSON file");
                        return ExitCode::FAILURE;
                    }
                    checker.contracts_path = Some(PathBuf::from(args.remove(1)));
                }
                Some("--entry") => {
                    args.remove(1);
                    if args.get(1).is_none_or(|arg| arg.starts_with('-')) {
                        eprintln!("mir-check: --entry requires a function name");
                        return ExitCode::FAILURE;
                    }
                    checker.entries.push(args.remove(1));
                }
                _ => break,
            }
        }
        if args.get(1).is_some_and(|arg| arg == "--") {
            args.remove(1);
        }
    }
    if let Some(path) = from_report {
        if args.len() != 1 {
            eprintln!("mir-check: --from-report supplies rustc arguments; do not add more");
            return ExitCode::FAILURE;
        }
        let loaded = (|| -> Result<Report, Box<dyn std::error::Error>> {
            let report: Report = serde_json::from_slice(&std::fs::read(&path)?)?;
            if !(7..=8).contains(&report.schema_version) {
                return Err(format!("unsupported report schema {}", report.schema_version).into());
            }
            if report.compiler != env!("MIR_CHECK_COMPILER") {
                return Err(
                    "saved invocation requires a different compiler; rebuild its inventory".into(),
                );
            }
            if report.rustc_arguments.is_empty() {
                return Err("saved report has no compiler arguments".into());
            }
            Ok(report)
        })();
        match loaded {
            Ok(report) => {
                if !checker.quiet {
                    eprintln!(
                        "mir-check: Rechecking {} with saved compiler arguments",
                        report.crate_name
                    );
                }
                let mut saved = report.rustc_arguments.into_iter();
                while let Some(arg) = saved.next() {
                    if matches!(arg.as_str(), "--error-format" | "--json" | "--color") {
                        saved.next();
                    } else if !["--error-format=", "--json=", "--color="]
                        .iter()
                        .any(|prefix| arg.starts_with(prefix))
                    {
                        args.push(arg);
                    }
                }
                let color = if matches!(checker.color, Color::Auto)
                    && std::env::var_os("NO_COLOR").is_some()
                {
                    "never"
                } else {
                    checker.color.argument()
                };
                args.extend([
                    "--error-format=human".to_owned(),
                    format!("--color={color}"),
                ]);
            }
            Err(error) => {
                eprintln!("mir-check: {}: {error}", path.display());
                return ExitCode::FAILURE;
            }
        }
    }
    if checker.json && checker.jsonl.is_some() {
        eprintln!("mir-check: choose --json or --jsonl");
        return ExitCode::FAILURE;
    }
    if let Some(path) = &checker.contracts_path {
        match ContractConfig::read(path) {
            Ok(config) => checker.contract_config = Some(config),
            Err(error) => {
                eprintln!("mir-check: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    if !args
        .iter()
        .any(|arg| arg == "--sysroot" || arg.starts_with("--sysroot="))
    {
        args.extend(["--sysroot".to_owned(), env!("MIR_CHECK_SYSROOT").to_owned()]);
    }
    checker.rustc_arguments = args[1..].to_vec();
    let status =
        rustc_driver::catch_with_exit_code(|| rustc_driver::run_compiler(&args, &mut checker));
    if checker.verify
        && checker.report_dir.is_none()
        && !checker.analysis_ran
        && status == ExitCode::SUCCESS
    {
        eprintln!(
            "mir-check: compiler invocation completed without MIR analysis; no proof produced"
        );
        return ExitCode::FAILURE;
    }
    if let Some(error) = checker.error {
        eprintln!("mir-check: {error}");
        return ExitCode::FAILURE;
    }
    status
}
