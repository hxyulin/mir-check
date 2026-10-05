#![feature(rustc_private)]
#![forbid(unsafe_code)]

extern crate rustc_abi;
extern crate rustc_attr_ir;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

use mir_check::{Contract, ContractKind, ContractStatus, Function, ProofStatus, Report, Source};
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
    summary: bool,
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
                if self.verify {
                    for function in &mut report.functions {
                        if !self.entries.is_empty() && !entries.contains(&function.name) {
                            continue;
                        }
                        let id = tcx
                            .mir_keys(())
                            .iter()
                            .find(|id| tcx.def_path_str(id.to_def_id()) == function.name)
                            .expect("inventoried functions have local MIR bodies");
                        function.proof = Some(proof::verify(tcx, id.to_def_id()));
                        if function
                            .proof
                            .as_ref()
                            .is_some_and(|proof| proof.status == ProofStatus::Proved)
                        {
                            for contract in &mut function.contracts {
                                contract.status = ContractStatus::VerifiedUnderPreconditions;
                            }
                        }
                    }
                }
                tcx.dcx().abort_if_errors();
                report.coverage = mir_check::coverage(&report);
                if let Err(error) = self.emit(&report) {
                    self.error = Some(error.to_string());
                } else if report.functions.iter().any(|function| {
                    function
                        .proof
                        .as_ref()
                        .is_some_and(|proof| proof.status != ProofStatus::Proved)
                }) {
                    self.error = Some(
                        "verification failed; inspect REFUTED and UNKNOWN obligations".to_owned(),
                    );
                }
            }
            Err(error) => self.error = Some(error),
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
        } else if self.summary {
            print!("{}", mir_check::render_coverage(report));
        } else {
            print!("{}", mir_check::render(report));
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
        schema_version: 7,
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
        summary: false,
    };
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
                "Usage: mir-check [--json] [--summary] [--verify] [--entry FUNCTION] -- \
                <rustc arguments>\n\
                --verify proves panic safety for a restricted MIR subset; unknown proofs fail."
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
                    checker.summary = true;
                    args.remove(1);
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
    if !args
        .iter()
        .any(|arg| arg == "--sysroot" || arg.starts_with("--sysroot="))
    {
        args.extend(["--sysroot".to_owned(), env!("MIR_CHECK_SYSROOT").to_owned()]);
    }
    checker.rustc_arguments = args[1..].to_vec();
    let status =
        rustc_driver::catch_with_exit_code(|| rustc_driver::run_compiler(&args, &mut checker));
    if let Some(error) = checker.error {
        eprintln!("mir-check: {error}");
        return ExitCode::FAILURE;
    }
    status
}
