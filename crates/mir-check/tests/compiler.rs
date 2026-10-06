#![forbid(unsafe_code)]

use mir_check::{ContractKind, ContractStatus, ProofStatus, Report, SiteKind, SiteStatus};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("mir-check-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn analyze(path: &Path, directory: &Directory, extra_args: &[&str]) -> Output {
    analyze_from(path, directory, &[], extra_args)
}

fn analyze_from(
    path: &Path,
    directory: &Directory,
    checker_args: &[&str],
    rustc_args: &[&str],
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .arg("--json")
        .args(checker_args)
        .args(["--", "--crate-type=lib", "--edition=2024"])
        .arg(path)
        .arg("--out-dir")
        .arg(&directory.0)
        .args(rustc_args)
        .output()
        .unwrap()
}

fn has_site(report: &Report, name: &str, kind: SiteKind) -> bool {
    report
        .functions
        .iter()
        .find(|function| function.name == name)
        .unwrap()
        .sites
        .iter()
        .any(|site| site.kind == kind && site.enabled && site.cfg_reachable)
}

#[test]
fn bounds_arithmetic_and_explicit_panics_remain_visible_with_aborting_panics() {
    let directory = Directory::new();
    let report = report(analyze(
        &fixture("panics.rs"),
        &directory,
        &["-Cpanic=abort", "-Coverflow-checks=yes"],
    ));
    assert_eq!(report.panic_strategy, "abort");
    for (function, kind) in [
        ("indexed", SiteKind::BoundsCheck),
        ("sum", SiteKind::Overflow),
        ("quotient", SiteKind::DivisionByZero),
        ("quotient", SiteKind::Overflow),
        ("remainder", SiteKind::RemainderByZero),
        ("explicit_panic", SiteKind::PanicCall),
        ("assertion", SiteKind::PanicCall),
    ] {
        assert!(
            has_site(&report, function, kind),
            "missing inventory for {function}"
        );
    }
    assert!(
        report
            .functions
            .iter()
            .flat_map(|f| &f.sites)
            .all(|site| matches!(site.status, SiteStatus::Unverified))
    );
}

#[test]
fn overflow_configuration_disables_optional_checks_but_preserves_division_failures() {
    let directory = Directory::new();
    let report = report(analyze(
        &fixture("panics.rs"),
        &directory,
        &["-Coverflow-checks=no"],
    ));
    assert!(!report.overflow_checks);
    assert!(!has_site(&report, "sum", SiteKind::Overflow));
    assert!(has_site(&report, "quotient", SiteKind::Overflow));
    assert!(has_site(&report, "quotient", SiteKind::DivisionByZero));
    assert!(has_site(&report, "indexed", SiteKind::BoundsCheck));
}

#[test]
fn inventories_record_dispatch_and_drop_boundaries_without_claiming_proof() {
    let directory = Directory::new();
    let report = report(analyze(&fixture("panics.rs"), &directory, &[]));
    for (function, kind) in [
        ("unwrap_option", SiteKind::ExternalCall),
        ("clamp", SiteKind::ExternalCall),
        ("dynamic", SiteKind::TraitCall),
        ("generic", SiteKind::TraitCall),
        ("indirect", SiteKind::IndirectCall),
        ("implicit_destructor", SiteKind::Drop),
    ] {
        assert!(
            has_site(&report, function, kind),
            "missing boundary for {function}"
        );
    }
    assert!(!has_site(&report, "misleading_name", SiteKind::PanicCall));
    assert!(
        report
            .functions
            .iter()
            .find(|f| f.name == "misleading_name")
            .unwrap()
            .local_calls
            .iter()
            .any(|call| call.callee == "panic_fmt")
    );
}

#[test]
fn inventory_keeps_guarded_accesses_unverified() {
    let directory = Directory::new();
    let report = report(analyze(&fixture("bodies.rs"), &directory, &[]));
    assert!(has_site(&report, "guarded", SiteKind::BoundsCheck));
    let text = mir_check::render(&report);
    assert!(text.contains("inventory only; no proof"));
    assert!(!text.contains("verification: PROVED"));
    assert_eq!(report.coverage.selected_roots, 0);
    assert!(
        report
            .functions
            .iter()
            .all(|function| function.proof.is_none())
    );
}

#[test]
fn entry_traces_follow_local_calls_and_terminate_for_recursion() {
    let directory = Directory::new();
    let report = report(analyze_from(
        &fixture("panics.rs"),
        &directory,
        &["--entry", "root", "--entry", "recursive"],
        &[],
    ));
    assert!(report.traces.iter().any(|trace| {
        trace.functions == ["root", "middle", "indexed"] && trace.kind == SiteKind::BoundsCheck
    }));
    assert!(report.traces.iter().any(|trace| {
        trace.functions == ["recursive", "indexed"] && trace.kind == SiteKind::BoundsCheck
    }));
    assert!(report.traces.iter().all(|trace| trace.functions.len() <= 3));
    assert!(mir_check::render(&report).contains("feasibility unverified"));
}

#[test]
fn an_entry_without_a_local_body_fails_instead_of_producing_an_empty_success() {
    let directory = Directory::new();
    let output = analyze_from(
        &fixture("panics.rs"),
        &directory,
        &["--entry", "absent"],
        &[],
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no inventoried local MIR body"));
}

fn report(output: Output) -> Report {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn uncalled_generic_functions_and_methods_have_typed_mir() {
    let directory = Directory::new();
    let report = report(analyze(&fixture("bodies.rs"), &directory, &[]));
    for name in ["identity", "never_called", "guarded", "value"] {
        let function = report
            .functions
            .iter()
            .find(|function| function.name.ends_with(name))
            .unwrap();
        assert!(function.basic_blocks > 0);
        assert!(function.source.line > 0);
        assert!(function.source.file.ends_with("bodies.rs"));
    }
    assert_eq!(report.mir_phase, "optimized_mir with mir-opt-level=0");
}

#[test]
fn compiler_errors_fail_analysis_without_a_success_report() {
    let directory = Directory::new();
    let path = directory.0.join("broken.rs");
    std::fs::write(&path, "pub fn broken() -> u8 { false }").unwrap();
    let output = analyze(&path, &directory, &[]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mismatched types"));
}

#[test]
fn a_report_write_failure_fails_the_compiler_wrapper() {
    let directory = Directory::new();
    let blocked = directory.0.join("not_a_directory");
    std::fs::write(&blocked, "blocked").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args(["rustc", "--crate-type=lib", "--edition=2024"])
        .arg(fixture("bodies.rs"))
        .arg("--out-dir")
        .arg(&directory.0)
        .env("MIR_CHECK_REPORT_DIR", blocked)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mir-check:"));
}

#[test]
fn malformed_contracts_fail_compilation_without_a_success_report() {
    let directory = Directory::new();
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-check")).parent().unwrap();
    let library = find_contract_library(profile).unwrap();
    let external = format!("mir_contracts={}", library.display());
    for (attribute, item) in [
        ("no_panic(value > 0)", "pub fn invalid(value: u8) {}"),
        ("requires(value >)", "pub fn invalid(value: u8) {}"),
        ("requires(true)", "pub struct Invalid;"),
    ] {
        let path = directory.0.join("invalid_contract.rs");
        std::fs::write(&path, format!("#[mir_contracts::{attribute}]\n{item}")).unwrap();
        let output = analyze(&path, &directory, &["--extern", &external]);
        assert!(!output.status.success(), "accepted {attribute}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn contracts_are_collected_as_unverified_metadata() {
    let directory = Directory::new();
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-check")).parent().unwrap();
    let contract_library = find_contract_library(profile).unwrap();
    let external = format!("mir_contracts={}", contract_library.display());
    let report = report(analyze(
        &fixture("contracts.rs"),
        &directory,
        &["--extern", &external],
    ));
    let function = report
        .functions
        .iter()
        .find(|function| function.name == "annotated")
        .unwrap();
    assert_eq!(function.contracts.len(), 3);
    assert!(
        function
            .contracts
            .iter()
            .any(|contract| matches!(contract.kind, ContractKind::NoPanic))
    );
    assert!(
        function
            .contracts
            .iter()
            .any(|contract| matches!(contract.kind, ContractKind::Requires))
    );
    let postcondition = function
        .contracts
        .iter()
        .find(|contract| matches!(contract.kind, ContractKind::Ensures))
        .unwrap();
    assert_eq!(postcondition.predicate.as_deref(), Some("result == value"));
    assert!(
        function
            .contracts
            .iter()
            .all(|contract| { matches!(contract.status, ContractStatus::PendingVerification) })
    );
}

fn find_contract_library(profile: &Path) -> Option<PathBuf> {
    // The pinned Cargo uses build/<crate>/<hash>/out; deps can contain stable ABI artifacts.
    let mut libraries = Vec::new();
    for entry in std::fs::read_dir(profile.join("build/mir-contracts")).ok()? {
        let output = entry.ok()?.path().join("out");
        if !output.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(output).ok()? {
            let path = file.ok()?.path();
            if path
                .file_name()?
                .to_string_lossy()
                .starts_with("libmir_contracts-")
                && path
                    .extension()
                    .is_some_and(|ext| ext == "dylib" || ext == "so")
            {
                let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
                libraries.push((modified, path));
            }
        }
    }
    libraries.sort();
    libraries.pop().map(|(_, path)| path)
}

fn verify_entry(name: &str, rustc_args: &[&str]) -> (Output, Report) {
    let directory = Directory::new();
    let output = analyze_from(
        &fixture("proofs.rs"),
        &directory,
        &["--verify", "--entry", name],
        rustc_args,
    );
    let report = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
    (output, report)
}

#[test]
fn guards_and_valid_local_calls_discharge_all_panic_obligations() {
    for name in [
        "guarded",
        "array_guarded",
        "next_byte",
        "guarded_sum",
        "guarded_division",
        "valid_call",
        "truncated_index",
    ] {
        let (output, report) = verify_entry(name, &["-Cpanic=abort", "-Coverflow-checks=yes"]);
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, ProofStatus::Proved, "{name}");
        assert!(
            !proof.obligations.is_empty(),
            "{name} must exercise a real check"
        );
        assert!(
            proof
                .obligations
                .iter()
                .all(|o| o.status == ProofStatus::Proved)
        );
        assert!(mir_check::render(&report).contains("verification: PROVED"));
    }
}

#[test]
fn broken_guards_arithmetic_and_call_arguments_fail_with_solver_models() {
    for name in [
        "off_by_one",
        "overflowing_sum",
        "unguarded_division",
        "invalid_call",
        "stale_guard",
        "false_assertion",
    ] {
        let (output, report) = verify_entry(name, &["-Coverflow-checks=yes"]);
        assert!(!output.status.success(), "accepted {name}");
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, ProofStatus::Refuted, "{name}");
        assert!(
            proof.obligations.iter().any(|o| o.model.is_some()),
            "{name}"
        );
    }
}

#[test]
fn reachable_loops_and_missing_solvers_fail_as_unknown() {
    let (output, report) = verify_entry("loop_unknown", &[]);
    assert!(!output.status.success());
    assert_eq!(
        report
            .functions
            .iter()
            .find(|f| f.name == "loop_unknown")
            .unwrap()
            .proof
            .as_ref()
            .unwrap()
            .status,
        ProofStatus::Unknown
    );
    let directory = Directory::new();
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args([
            "--verify",
            "--json",
            "--entry",
            "guarded",
            "--",
            "--crate-type=lib",
            "--edition=2024",
        ])
        .arg(fixture("proofs.rs"))
        .env("MIR_CHECK_Z3", directory.0.join("missing_solver"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "guarded")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Unknown);
    assert!(
        proof
            .obligations
            .iter()
            .any(|o| o.detail.contains("cannot start Z3"))
    );
}

#[test]
fn mir_lint_errors_prevent_emission_of_a_proof_report() {
    let directory = Directory::new();
    let path = directory.0.join("invalid.rs");
    std::fs::write(&path, "pub fn invalid() -> u8 { let a = [0_u8; 1]; a[2] }").unwrap();
    let output = analyze_from(&path, &directory, &["--verify"], &[]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("will panic at runtime"));
}

#[test]
fn the_proved_subset_agrees_with_exhaustive_host_checks_and_negative_replays() {
    let directory = Directory::new();
    let source = std::fs::read_to_string(fixture("proofs.rs")).unwrap();
    let source = source
        .replace("#![no_std]\n", "")
        .replace("#![forbid(unsafe_code)]\n", "");
    let harness = format!(
        "#![forbid(unsafe_code)]\nmod sample {{ {source} }}\n{}",
        r#"
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    for left in 0..=u8::MAX {
        for right in 0..=u8::MAX {
            sample::guarded_sum(left, right);
        }
    }
    let bytes = [1, 2, 3, 4];
    for length in 0..=bytes.len() {
        for index in 0..=8 {
            sample::guarded(&bytes[..length], index);
            sample::next_byte(&bytes[..length], index);
        }
    }
    assert!(std::panic::catch_unwind(|| sample::off_by_one(&bytes, 4)).is_err());
    assert!(std::panic::catch_unwind(|| sample::invalid_call(&bytes)).is_err());
    assert!(std::panic::catch_unwind(|| sample::overflowing_sum(255, 1)).is_err());
    assert!(std::panic::catch_unwind(|| sample::unguarded_division(i32::MIN, -1)).is_err());
    assert!(std::panic::catch_unwind(|| sample::stale_guard(&bytes, 0)).is_err());
}
"#
    );
    let path = directory.0.join("replay.rs");
    std::fs::write(&path, harness).unwrap();
    let executable = directory.0.join("replay");
    let output = Command::new(Path::new(env!("MIR_CHECK_SYSROOT")).join("bin/rustc"))
        .args(["--edition=2024", "-Coverflow-checks=yes"])
        .arg(path)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(Command::new(executable).status().unwrap().success());
}

#[test]
fn cargo_analysis_revisits_a_crate_and_forwards_feature_selection() {
    let directory = Directory::new();
    std::fs::write(
        directory.0.join("Cargo.toml"),
        "[package]\nname = 'cargo_fixture'\nversion = '0.1.0'\nedition = '2024'\n\
        [lib]\npath = 'lib.rs'\n[features]\nextra = []\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(
        directory.0.join("lib.rs"),
        "#![no_std]\npub fn ordinary() {}\n#[cfg(feature = \"extra\")] pub fn extra() {}",
    )
    .unwrap();
    let mut report_directories = Vec::new();
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-mir-check"))
            .args([
                "mir-check",
                "--verbose",
                "--lib",
                "--features",
                "extra",
                "--offline",
            ])
            .current_dir(&directory.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("extra at"), "{stdout}");
        let reports = stdout
            .lines()
            .find_map(|line| line.strip_prefix("JSON reports: "))
            .unwrap();
        let path = std::fs::read_dir(reports)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let report: Report = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(report.crate_name, "cargo_fixture");
        report_directories.push(reports.to_owned());
    }
    assert_ne!(report_directories[0], report_directories[1]);
}

fn verify_contract(name: &str, rustc_args: &[&str]) -> (Output, Report) {
    verify_contract_with_failures(name, rustc_args, false)
}

fn verify_contract_with_failures(
    name: &str,
    rustc_args: &[&str],
    all_failures: bool,
) -> (Output, Report) {
    let directory = Directory::new();
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-check")).parent().unwrap();
    let library = find_contract_library(profile).unwrap();
    let external = format!("mir_contracts={}", library.display());
    let mut args = vec!["--extern", external.as_str(), "-Coverflow-checks=yes"];
    args.extend_from_slice(rustc_args);
    let mut checker_args = vec!["--verify", "--entry", name];
    if all_failures {
        checker_args.push("--all-failures");
    }
    let output = analyze_from(
        &fixture("verified_contracts.rs"),
        &directory,
        &checker_args,
        &args,
    );
    let report = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{name}: {error}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output, report)
}

#[test]
fn callee_domains_caller_bounds_and_return_values_are_proved_separately() {
    for name in [
        "read",
        "guarded_read",
        "bounded",
        "guarded_bound",
        "snapshot",
        "signed_bound",
        "valid_signed_call",
        "boolean_contract",
        "array_read",
    ] {
        let (output, report) = verify_contract(name, &["-Cpanic=abort"]);
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let function = report
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        let proof = function.proof.as_ref().unwrap();
        assert_eq!(proof.status, ProofStatus::Proved, "{name}");
        assert!(!proof.obligations.is_empty(), "{name}");
        assert!(
            function.contracts.iter().all(|contract| matches!(
                contract.status,
                ContractStatus::VerifiedUnderPreconditions
            ))
        );
        if name == "read" {
            assert_eq!(proof.assumptions, ["index < bytes.len()"]);
            assert!(proof.inputs.contains_key("index"));
        }
        if name == "guarded_read" {
            assert!(proof.assumptions.is_empty());
            assert!(proof.obligations.iter().any(|obligation| matches!(
                obligation.kind,
                mir_check::ObligationKind::CallPrecondition
            )));
            let callee = report
                .functions
                .iter()
                .find(|function| function.name == "read")
                .unwrap();
            assert!(callee.proof.is_none());
            assert!(
                callee
                    .contracts
                    .iter()
                    .all(|contract| matches!(contract.status, ContractStatus::PendingVerification))
            );
        }
    }
}

#[test]
fn violating_a_call_bound_fails_even_when_the_callee_cannot_panic() {
    for name in ["bad_bound", "bad_read", "bad_signed_call", "bad_array_read"] {
        let (output, report) = verify_contract(name, &[]);
        assert!(!output.status.success(), "accepted {name}");
        let proof = report
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, ProofStatus::Refuted, "{name}");
        assert!(
            proof.obligations.iter().any(|obligation| matches!(
                obligation.kind,
                mir_check::ObligationKind::CallPrecondition
            ) && obligation.status
                == ProofStatus::Refuted
                && obligation.model.is_some()),
            "{name}"
        );
        if name == "bad_bound" {
            // value=16 violates the contract, although bounded(16) simply returns 16.
            assert!(proof.obligations.iter().any(|obligation| {
                obligation
                    .model
                    .as_ref()
                    .is_some_and(|model| model.contains("(_ bv16 8)"))
            }));
        }
    }
}

#[test]
fn annotations_are_never_trusted_in_place_of_body_or_postcondition_proofs() {
    for name in [
        "lying_no_panic",
        "lying_postcondition",
        "use_lying_postcondition",
        "changed_snapshot",
    ] {
        let (output, report) = verify_contract(name, &[]);
        assert!(!output.status.success(), "accepted {name}");
        let function = report
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        let proof = function.proof.as_ref().unwrap();
        assert_eq!(proof.status, ProofStatus::Refuted, "{name}");
        assert!(
            function
                .contracts
                .iter()
                .all(|contract| matches!(contract.status, ContractStatus::PendingVerification))
        );
        if name != "lying_no_panic" {
            assert!(
                proof.obligations.iter().any(|obligation| matches!(
                    obligation.kind,
                    mir_check::ObligationKind::Postcondition
                ) && obligation.status
                    == ProofStatus::Refuted),
                "{name}"
            );
        }
        if name == "use_lying_postcondition" {
            assert!(proof.stopped_after_counterexample);
            let (output, report) = verify_contract_with_failures(name, &[], true);
            assert!(!output.status.success());
            let proof = report
                .functions
                .iter()
                .find(|function| function.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(proof.status, ProofStatus::Refuted);
            assert!(!proof.stopped_after_counterexample);
            assert!(proof.obligations.iter().any(|obligation| matches!(
                obligation.kind,
                mir_check::ObligationKind::Postcondition
            ) && obligation.status
                == ProofStatus::Refuted));
            assert!(proof.obligations.iter().any(|obligation| matches!(
                obligation.kind,
                mir_check::ObligationKind::PanicSafety
            ) && obligation.status
                == ProofStatus::Refuted));
        }
    }
}

#[test]
fn inconsistent_ill_typed_and_impure_contracts_cannot_produce_a_proof() {
    for name in [
        "inconsistent",
        "missing_name",
        "out_of_range",
        "wrong_type",
        "impure",
        "arithmetic",
    ] {
        let (output, report) = verify_contract(name, &[]);
        assert!(!output.status.success(), "accepted {name}");
        let proof = report
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, ProofStatus::Unknown, "{name}");
    }
}

#[test]
fn proofs_use_the_target_width_and_record_the_actual_overflow_configuration() {
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let bits = if target.is_some() { 32 } else { usize::BITS };
        let mut args = vec!["-Cpanic=abort", "-Coverflow-checks=yes"];
        if let Some(target) = target {
            args.extend(["--target", target]);
        }
        let (output, report) = verify_entry("next_byte", &args);
        assert!(
            output.status.success(),
            "{target:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if let Some(target) = target {
            assert_eq!(report.target, target);
        }
        let proof = report
            .functions
            .iter()
            .find(|function| function.name == "next_byte")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, ProofStatus::Proved);
        assert!(proof.obligations.iter().any(|obligation| {
            obligation
                .query
                .as_ref()
                .is_some_and(|query| query.contains(&format!("BitVec {bits}")))
        }));
    }
    for panic in ["abort", "unwind"] {
        for checks in ["yes", "no"] {
            let panic_arg = format!("-Cpanic={panic}");
            let checks_arg = format!("-Coverflow-checks={checks}");
            let (output, report) = verify_entry("overflowing_sum", &[&panic_arg, &checks_arg]);
            assert_eq!(output.status.success(), checks == "no");
            assert_eq!(report.panic_strategy, panic);
            assert_eq!(report.overflow_checks, checks == "yes");
        }
    }
}

#[test]
fn panicking_options_and_unknown_dispatch_boundaries_fail_verification() {
    for (name, expected) in [
        ("unwrap_option", ProofStatus::Refuted),
        ("dynamic", ProofStatus::Unknown),
        ("generic", ProofStatus::Unknown),
        ("indirect", ProofStatus::Unknown),
        ("destructor", ProofStatus::Refuted),
        ("implicit_destructor", ProofStatus::Refuted),
    ] {
        let directory = Directory::new();
        let output = analyze_from(
            &fixture("panics.rs"),
            &directory,
            &["--verify", "--entry", name],
            &[],
        );
        assert!(!output.status.success(), "accepted {name}");
        let report: Report = serde_json::from_slice(&output.stdout).unwrap();
        let proof = report
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, expected, "{name}");
    }
}

#[test]
fn cargo_contract_verification_returns_failure_and_preserves_its_report() {
    let directory = Directory::new();
    let dependency = Path::new(env!("CARGO_MANIFEST_DIR")).join("../mir-contracts");
    std::fs::write(
        directory.0.join("Cargo.toml"),
        format!(
            "[package]\nname='contract_fixture'\nversion='0.1.0'\nedition='2024'\n\
         [lib]\npath='lib.rs'\n[workspace]\n[dependencies]\nmir-contracts={{path={}}}\n",
            serde_json::to_string(&dependency).unwrap(),
        ),
    )
    .unwrap();
    for (guard, success) in [("<", true), ("<=", false)] {
        std::fs::write(
            directory.0.join("lib.rs"),
            format!(
                "#![no_std]\n#![forbid(unsafe_code)]\n\
             use mir_contracts::{{requires,ensures,no_panic}};\n\
             #[requires(value < 16)] #[ensures(result == value)] #[no_panic]\n\
             pub fn bounded(value:u8)->u8{{value}}\n\
             pub fn caller(value:u8)->u8{{if value {guard} 16 {{bounded(value)}}else{{0}}}}\n",
            ),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-mir-check"))
            .args(["mir-check", "--verify", "--lib", "--offline"])
            .current_dir(&directory.0)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            success,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(if success {
            output.stdout
        } else {
            output.stderr
        })
        .unwrap();
        let reports = text
            .lines()
            .find_map(|line| line.strip_prefix("JSON reports: "))
            .unwrap();
        let path = std::fs::read_dir(reports)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let report: Report = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let caller = report
            .functions
            .iter()
            .find(|function| function.name == "caller")
            .unwrap();
        assert_eq!(
            caller.proof.as_ref().unwrap().status,
            if success {
                ProofStatus::Proved
            } else {
                ProofStatus::Refuted
            }
        );
    }
}

#[test]
fn the_vendored_frame_methods_keep_the_original_function_bodies() {
    fn methods(source: &str) -> std::collections::BTreeMap<String, String> {
        let mut bodies = std::collections::BTreeMap::new();
        for item in syn::parse_file(source).unwrap().items {
            if let syn::Item::Impl(implementation) = item {
                let syn::Type::Path(ty) = *implementation.self_ty else {
                    panic!("named type expected")
                };
                let name = ty.path.get_ident().unwrap().to_string();
                for item in implementation.items {
                    if let syn::ImplItem::Fn(method) = item {
                        use quote::ToTokens;
                        bodies.insert(
                            format!("{name}::{}", method.sig.ident),
                            method.block.to_token_stream().to_string(),
                        );
                    }
                }
            }
        }
        bodies
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/can-frame/src");
    let original = methods(&std::fs::read_to_string(root.join("upstream.rs")).unwrap());
    let annotated = methods(&std::fs::read_to_string(root.join("lib.rs")).unwrap());
    assert_eq!(original.len(), 6);
    assert_eq!(original, annotated);
}

fn vendored_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/can-frame/src/lib.rs")
}

fn verify_vendored(path: &Path, entries: &[&str], rustc_args: &[&str]) -> (Output, Report) {
    let sibling = path.with_file_name("bus.rs");
    if !sibling.exists()
        && std::fs::read_to_string(path)
            .unwrap()
            .contains("pub mod bus;")
    {
        std::fs::copy(vendored_source().with_file_name("bus.rs"), sibling).unwrap();
    }
    let directory = Directory::new();
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-check")).parent().unwrap();
    let library = find_contract_library(profile).unwrap();
    let external = format!("mir_contracts={}", library.display());
    let mut checker = vec!["--verify"];
    for entry in entries {
        checker.extend(["--entry", entry]);
    }
    let mut args = vec![
        "--extern",
        external.as_str(),
        "-Coverflow-checks=yes",
        "-Cpanic=abort",
    ];
    args.extend_from_slice(rustc_args);
    let output = analyze_from(path, &directory, &checker, &args);
    let report = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{entries:?}: {error}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output, report)
}

#[test]
fn vendored_constructors_accessors_and_payload_round_trips_prove_on_host_and_arm() {
    let entries = [
        "Frame::new",
        "Frame::data",
        "Frame::id",
        "FdFrame::new",
        "FdFrame::data",
        "FdFrame::id",
        "classic_payload_round_trip",
        "fd_payload_round_trip",
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let (output, report) = verify_vendored(&vendored_source(), &entries, &args);
        let selected: Vec<_> = report
            .functions
            .iter()
            .filter(|function| function.proof.is_some())
            .collect();
        assert_eq!(selected.len(), entries.len());
        for function in selected {
            let proof = function.proof.as_ref().unwrap();
            assert_eq!(
                proof.status,
                ProofStatus::Proved,
                "{} {target:?}",
                function.name
            );
            assert!(!proof.obligations.is_empty());
            assert!(function.contracts.iter().all(|contract| matches!(
                contract.status,
                ContractStatus::VerifiedUnderPreconditions
            )));
            if function.name.ends_with("::new") {
                assert!(proof.assumptions.is_empty());
                assert!(
                    proof
                        .models
                        .iter()
                        .any(|model| model.contains("copy_from_slice"))
                );
                assert!(proof.obligations.iter().any(|obligation| matches!(
                    obligation.kind,
                    mir_check::ObligationKind::Postcondition
                )));
            }
            if function.name.ends_with("::data") {
                assert_eq!(proof.assumptions.len(), 1);
                assert!(proof.inputs.contains_key("self.len"));
            }
            if function.name.ends_with("round_trip") {
                assert!(proof.obligations.iter().any(|obligation| matches!(
                    obligation.kind,
                    mir_check::ObligationKind::CallPrecondition
                )
                    && obligation.detail.contains("::data requires")));
            }
        }
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn a_caller_cannot_pass_a_frame_with_an_invalid_stored_length() {
    for entry in ["classic_bad_length", "fd_bad_length"] {
        let (output, report) = verify_vendored(&vendored_source(), &[entry], &[]);
        assert!(!output.status.success(), "accepted {entry}");
        let proof = report
            .functions
            .iter()
            .find(|function| function.name == entry)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, ProofStatus::Refuted, "{entry}");
        assert!(proof.obligations.iter().any(|obligation| matches!(
            obligation.kind,
            mir_check::ObligationKind::CallPrecondition
        ) && obligation.status
            == ProofStatus::Refuted
            && obligation.model.is_some()));
    }
}

#[test]
fn broken_real_code_guards_lengths_and_payload_copies_are_rejected() {
    let original = std::fs::read_to_string(vendored_source()).unwrap();
    for (entry, before, after) in [
        (
            "Frame::new",
            "if id > 0x7FF || data.len() > 8",
            "if id > 0x7FF || data.len() > 9",
        ),
        ("Frame::new", "len: data.len() as u8,", "len: 9,"),
        (
            "Frame::new",
            "if id > 0x7FF || data.len() > 8",
            "if id > 0x800 || data.len() > 8",
        ),
        (
            "Frame::new",
            "bytes[..data.len()].copy_from_slice(data);",
            "bytes[..7].copy_from_slice(data);",
        ),
        (
            "classic_payload_round_trip",
            "bytes[..data.len()].copy_from_slice(data);",
            "let _ = &mut bytes;",
        ),
        (
            "FdFrame::new",
            "let valid = len <= 8 || matches!(len, 12 | 16 | 20 | 24 | 32 | 48 | 64);",
            "let valid = len <= 64;",
        ),
    ] {
        assert!(original.contains(before));
        let directory = Directory::new();
        let path = directory.0.join("mutated.rs");
        std::fs::write(&path, original.replacen(before, after, 1)).unwrap();
        let (output, report) = verify_vendored(&path, &[entry], &[]);
        assert!(!output.status.success(), "accepted mutation: {after}");
        let proof = report
            .functions
            .iter()
            .find(|function| function.name == entry)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(
            proof.status,
            ProofStatus::Refuted,
            "mutation {after}: {:?}",
            proof.obligations
        );
        assert!(
            proof
                .obligations
                .iter()
                .any(|obligation| obligation.status == ProofStatus::Refuted
                    && obligation.model.is_some())
        );
    }
}

#[test]
fn accessor_bounds_are_explicit_and_cannot_be_inferred_from_private_fields_alone() {
    let original = std::fs::read_to_string(vendored_source()).unwrap();
    let directory = Directory::new();
    let path = directory.0.join("without_domain.rs");
    std::fs::write(&path, original.replace("#[requires(self.len <= 8)]", "")).unwrap();
    let (output, report) = verify_vendored(&path, &["Frame::data"], &[]);
    assert!(!output.status.success());
    let proof = report
        .functions
        .iter()
        .find(|function| function.name == "Frame::data")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Refuted);
    assert!(proof.assumptions.is_empty());
}

#[test]
fn core_models_do_not_trust_similarly_named_user_methods() {
    let directory = Directory::new();
    let path = directory.0.join("fake_copy.rs");
    std::fs::write(
        &path,
        r#"
#![no_std]
struct Fake { value: u8 }
impl Fake {
    fn copy_from_slice(&self, source: &[u8]) {
        let _ = (self.value, source);
        panic!();
    }
}
pub fn caller(value: u8, source: &[u8]) {
    Fake { value }.copy_from_slice(source);
}
"#,
    )
    .unwrap();
    let (output, report) = verify_vendored(&path, &["caller"], &[]);
    assert!(!output.status.success());
    let proof = report
        .functions
        .iter()
        .find(|function| function.name == "caller")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Refuted);
    assert!(proof.models.is_empty());
}

#[test]
fn mutable_array_borrows_preserve_writes_across_a_call_boundary() {
    let directory = Directory::new();
    let path = directory.0.join("mutable_call.rs");
    std::fs::write(
        &path,
        r#"
#![no_std]
fn fill(bytes: &mut [u8], value: u8) { bytes[0] = value; }
pub fn caller(value: u8) -> u8 {
    let mut bytes = [0; 8];
    fill(&mut bytes, value);
    assert!(bytes[0] == value);
    bytes[0]
}
"#,
    )
    .unwrap();
    let (output, report) = verify_vendored(&path, &["caller"], &[]);
    assert!(output.status.success());
    let proof = report
        .functions
        .iter()
        .find(|function| function.name == "caller")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Proved);
    assert!(
        proof
            .analyzed_bodies
            .iter()
            .any(|body| body.starts_with("fill "))
    );
}

#[test]
fn the_vendored_bus_validator_and_helpers_keep_the_original_bodies() {
    fn bodies(source: &str) -> std::collections::BTreeMap<String, String> {
        use quote::ToTokens;
        let mut bodies = std::collections::BTreeMap::new();
        for item in syn::parse_file(source).unwrap().items {
            match item {
                syn::Item::Fn(function) if function.sig.ident == "check" => {
                    bodies.insert(
                        "check".to_owned(),
                        function.block.to_token_stream().to_string(),
                    );
                }
                syn::Item::Impl(implementation) => {
                    for item in implementation.items {
                        if let syn::ImplItem::Fn(method) = item {
                            bodies.insert(
                                method.sig.ident.to_string(),
                                method.block.to_token_stream().to_string(),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        bodies
    }
    let root = vendored_source().parent().unwrap().to_owned();
    let original = bodies(&std::fs::read_to_string(root.join("bus_upstream.rs")).unwrap());
    let annotated = bodies(&std::fs::read_to_string(root.join("bus.rs")).unwrap());
    assert_eq!(original.len(), 3);
    assert_eq!(original, annotated);
}

#[test]
fn nested_bus_loops_prove_for_symbolic_ids_and_slots_on_host_and_arm() {
    let entries = ["bus::shared_bus", "bus::fd_bus"];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let (output, report) = verify_vendored(&vendored_source(), &entries, &args);
        for entry in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == entry)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status,
                ProofStatus::Proved,
                "{entry}: {:?}",
                proof.obligations
            );
            assert!(!proof.assumptions.is_empty());
            assert!(
                proof
                    .obligations
                    .iter()
                    .any(|obligation| obligation.function == "bus::check"
                        && matches!(obligation.kind, mir_check::ObligationKind::PanicSafety))
            );
        }
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn invalid_bus_ids_slots_collisions_and_fd_compatibility_are_refuted() {
    let entries = [
        "bus::duplicate_slot",
        "bus::invalid_slot",
        "bus::incompatible_fd",
        "bus::duplicate_id",
        "bus::invalid_id",
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let (output, report) = verify_vendored(&vendored_source(), &entries, &args);
        assert!(!output.status.success());
        for entry in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == entry)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status,
                ProofStatus::Refuted,
                "{entry}: {:?}",
                proof.obligations
            );
            assert!(
                proof
                    .obligations
                    .iter()
                    .any(|obligation| obligation.function == "bus::check"
                        && obligation.status == ProofStatus::Refuted
                        && obligation.model.is_some())
            );
        }
    }
}

#[test]
fn finite_loops_finish_and_later_panics_are_not_hidden_by_unrolling() {
    let directory = Directory::new();
    let path = directory.0.join("loops.rs");
    std::fs::write(
        &path,
        r#"
#![no_std]
use mir_contracts::{requires, ensures};
#[requires(count <= 4)]
#[ensures(result == count)]
pub fn finite(count: u8) -> u8 {
    let mut done = 0;
    while done < count { done += 1; }
    done
}
pub fn late_panic() {
    let mut done = 0;
    while done < 3 { done += 1; }
    assert!(done < 3);
}
pub fn nonterminating() { loop {} }
pub fn beyond_budget() {
    let mut done = 0;
    while done < 300 { done += 1; }
}
"#,
    )
    .unwrap();
    let entries = ["finite", "late_panic", "nonterminating", "beyond_budget"];
    let (output, report) = verify_vendored(&path, &entries, &[]);
    assert!(!output.status.success());
    for (entry, status) in [
        ("finite", ProofStatus::Proved),
        ("late_panic", ProofStatus::Refuted),
        ("nonterminating", ProofStatus::Unknown),
        ("beyond_budget", ProofStatus::Proved),
    ] {
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, status, "{entry}: {:?}", proof.obligations);
        if status == ProofStatus::Unknown {
            assert!(
                proof
                    .obligations
                    .iter()
                    .any(|obligation| obligation.detail.contains("step limit"))
            );
        }
    }
}

#[test]
fn arbitrary_enum_slices_and_ambiguous_enum_indices_remain_unknown() {
    let (output, report) = verify_vendored(&vendored_source(), &["bus::check"], &[]);
    assert!(!output.status.success());
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "bus::check")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Unknown);

    let directory = Directory::new();
    let path = directory.0.join("ambiguous_index.rs");
    std::fs::write(
        &path,
        r#"
#![no_std]
use mir_contracts::requires;
#[derive(Clone, Copy)]
enum Entry { First(u16), Second(u16) }
#[requires(index < 2)]
pub fn read(first: u16, second: u16, index: usize) -> u16 {
    let values = [Entry::First(first), Entry::Second(second)];
    match values[index] { Entry::First(value) | Entry::Second(value) => value }
}
"#,
    )
    .unwrap();
    let (output, report) = verify_vendored(&path, &["read"], &[]);
    assert!(!output.status.success());
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "read")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Unknown);
    assert!(
        proof
            .obligations
            .iter()
            .any(|obligation| obligation.detail.contains("not uniquely determined"))
    );
}

#[test]
fn static_panic_payloads_are_modeled_without_trusting_user_or_dynamic_formatting() {
    let directory = Directory::new();
    let path = directory.0.join("formatting.rs");
    std::fs::write(
        &path,
        r#"
#![no_std]
use mir_contracts::requires;
#[requires(value == 1)]
pub const fn literal(value: u8) {
    assert!(value == 0, "expected zero");
}
struct User;
impl User {
    fn from_str(_: &'static str) -> Self { panic!(); }
}
pub fn fake() { let _ = User::from_str("text"); }
pub fn dynamic(value: u8) { panic!("value: {value}"); }
"#,
    )
    .unwrap();
    let entries = ["literal", "fake", "dynamic"];
    let (output, report) = verify_vendored(&path, &entries, &[]);
    assert!(!output.status.success());
    for (entry, status) in [
        ("literal", ProofStatus::Refuted),
        ("fake", ProofStatus::Refuted),
        ("dynamic", ProofStatus::Unknown),
    ] {
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, status, "{entry}: {:?}", proof.obligations);
        if entry == "literal" {
            assert!(
                proof
                    .models
                    .iter()
                    .any(|model| model.contains("static formatting arguments"))
            );
        } else {
            assert!(proof.models.is_empty());
        }
    }
}

fn dr16_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/dr16/src/lib.rs")
}

#[test]
fn the_vendored_dr16_parser_keeps_its_original_body() {
    use quote::ToTokens;
    fn body(source: &str) -> String {
        syn::parse_file(source)
            .unwrap()
            .items
            .into_iter()
            .find_map(|item| {
                if let syn::Item::Impl(implementation) = item {
                    implementation.items.into_iter().find_map(|item| {
                        if let syn::ImplItem::Fn(method) = item {
                            Some(method.block.to_token_stream().to_string())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            })
            .unwrap()
    }
    let original = std::fs::read_to_string(dr16_source().with_file_name("upstream.rs")).unwrap();
    assert_eq!(
        body(&original),
        body(&std::fs::read_to_string(dr16_source()).unwrap())
    );
}

#[test]
fn the_dr16_parser_proves_without_entry_bounds_on_host_and_arm() {
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let (output, report) = verify_vendored(&dr16_source(), &["Raw::parse"], &args);
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == "Raw::parse")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, ProofStatus::Proved, "{:?}", proof.obligations);
        assert!(output.status.success());
        assert!(proof.assumptions.is_empty());
        for body in [
            "Result::<T, E>::ok",
            "as core::ops::Try>::branch",
            "closure#0",
            "closure#1",
            "closure#2",
        ] {
            assert!(
                proof.analyzed_bodies.iter().any(|name| name.contains(body)),
                "missing {body}"
            );
        }
        for model in [
            "slice to array",
            "integer endian decoding",
            "fixed array map",
        ] {
            assert!(
                proof.models.iter().any(|name| name.contains(model)),
                "missing {model}"
            );
        }
        assert!(
            proof
                .obligations
                .iter()
                .any(|obligation| matches!(obligation.kind, mir_check::ObligationKind::Validity))
        );
    }
}

#[test]
fn incorrect_dr16_indices_and_channel_masks_are_rejected() {
    let original = std::fs::read_to_string(dr16_source()).unwrap();
    for (before, after) in [("u(17)", "u(18)"), ("raw & 0x7FF", "raw & 0xFFFF")] {
        let directory = Directory::new();
        let path = directory.0.join("mutated_dr16.rs");
        assert!(original.contains(before));
        std::fs::write(&path, original.replacen(before, after, 1)).unwrap();
        let (output, report) = verify_vendored(&path, &["Raw::parse"], &[]);
        assert!(!output.status.success());
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == "Raw::parse")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(
            proof.status,
            ProofStatus::Refuted,
            "{after}: {:?}",
            proof.obligations
        );
        assert!(
            proof
                .obligations
                .iter()
                .any(|obligation| obligation.status == ProofStatus::Refuted
                    && obligation.model.is_some())
        );
    }
}

#[test]
fn concrete_generics_static_traits_and_read_only_closures_are_interpreted() {
    let directory = Directory::new();
    let path = directory.0.join("generic_calls.rs");
    std::fs::write(
        &path,
        r#"
#![no_std]
use mir_contracts::{requires, ensures};
#[requires(value < 16)]
#[ensures(result == value)]
fn identity<T: Copy>(value: T) -> T { value }
trait Read { fn read(&self) -> u16; }
struct Number { value: u16 }
impl Read for Number { fn read(&self) -> u16 { self.value } }
fn read<T: Read>(number: &T) -> u16 { number.read() }
fn apply<F: FnOnce(u16) -> u16>(function: F, value: u16) -> u16 { function(value) }
fn same(value: u16) -> u16 { value }
#[requires(value < 16)]
#[ensures(result == value)]
pub fn generic(value: u16) -> u16 { identity::<u16>(value) }
#[requires(value == 16)]
pub fn invalid_generic(value: u16) -> u16 { identity::<u16>(value) }
#[ensures(result == value)]
pub fn static_trait(value: u16) -> u16 { read(&Number { value }) }
#[ensures(result == value)]
pub fn captured(value: u16) -> u16 {
    let saved = value;
    apply(|_| saved, value)
}
#[ensures(result == value)]
pub fn function_item(value: u16) -> u16 { apply(same, value) }
#[ensures(result == value)]
pub fn mapped_item(value: u16) -> u16 { [value].map(same)[0] }
pub fn mutable_capture(value: u16) -> u16 {
    let mut saved = value;
    apply(|_| { saved = 0; saved }, value)
}
"#,
    )
    .unwrap();
    let entries = [
        "generic",
        "invalid_generic",
        "static_trait",
        "captured",
        "function_item",
        "mapped_item",
        "mutable_capture",
    ];
    let (output, report) = verify_vendored(&path, &entries, &[]);
    assert!(!output.status.success());
    for entry in entries {
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(
            proof.status,
            match entry {
                "invalid_generic" => ProofStatus::Refuted,
                "mutable_capture" => ProofStatus::Proved,
                _ => ProofStatus::Proved,
            },
            "{entry}: {:?}",
            proof.obligations
        );
    }
}

#[test]
fn scalar_arrays_boolean_casts_and_endian_decoding_preserve_values() {
    let directory = Directory::new();
    let path = directory.0.join("scalar_arrays.rs");
    std::fs::write(
        &path,
        r#"
#![no_std]
use mir_contracts::{requires, ensures};
#[requires(index < 3)]
#[ensures(result >= 0 && result <= 3)]
pub fn integers(index: usize, values: [u16; 3]) -> u16 {
    values.map(|value| value & 3)[index]
}
#[requires(index < 3)]
#[ensures(result == value)]
pub fn repeat(index: usize, value: i16) -> i16 { [value; 3][index] }
#[requires(index < 3)]
#[ensures(result <= 1)]
pub fn booleans(index: usize, values: [bool; 3]) -> u8 { values[index] as u8 }
#[ensures(result == value)]
pub fn mapped_byte(value: u8) -> u8 { [value].map(|byte| byte)[0] }
pub fn little(first: u8, second: u8) -> u16 {
    let value = u16::from_le_bytes([first, second]);
    assert!(value == (first as u16 | (second as u16) << 8));
    value
}
pub fn big(first: u8, second: u8) -> u16 {
    let value = u16::from_be_bytes([first, second]);
    assert!(value == ((first as u16) << 8 | second as u16));
    value
}
#[ensures(result >= 0 && result <= 3)]
pub fn shifted(value: i16) -> i16 { (value >> 14) & 3 }
"#,
    )
    .unwrap();
    let entries = [
        "integers",
        "repeat",
        "booleans",
        "mapped_byte",
        "little",
        "big",
        "shifted",
    ];
    let (output, report) = verify_vendored(&path, &entries, &[]);
    for entry in entries {
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(
            proof.status,
            ProofStatus::Proved,
            "{entry}: {:?}",
            proof.obligations
        );
    }
    assert!(output.status.success());
}

#[test]
fn available_dependency_bodies_are_executed_with_concrete_type_arguments() {
    let directory = Directory::new();
    let dependency = directory.0.join("dependency.rs");
    std::fs::write(
        &dependency,
        r#"
#![no_std]
pub fn identity<T: Copy>(value: T) -> T { value }
#[inline]
pub fn increment(value: u8) -> u8 { value + 1 }
"#,
    )
    .unwrap();
    let library = directory.0.join("libdependency.rlib");
    let build = Command::new("rustup")
        .args([
            "run",
            "nightly-2026-09-22",
            "rustc",
            "--crate-type=rlib",
            "--edition=2024",
            "--crate-name=dependency",
            "-Cpanic=abort",
            "-Coverflow-checks=yes",
        ])
        .arg(&dependency)
        .arg("-o")
        .arg(&library)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let source = directory.0.join("consumer.rs");
    std::fs::write(
        &source,
        r#"
#![no_std]
use mir_contracts::{requires, ensures};
#[ensures(result == value)]
pub fn generic(value: u16) -> u16 { dependency::identity(value) }
#[requires(value < 255)]
#[ensures(result > value)]
pub fn increment(value: u8) -> u8 { dependency::increment(value) }
#[requires(value == 255)]
pub fn overflow(value: u8) -> u8 { dependency::increment(value) }
"#,
    )
    .unwrap();
    let external = format!("dependency={}", library.display());
    let entries = ["generic", "increment", "overflow"];
    let (output, report) = verify_vendored(&source, &entries, &["--extern", &external]);
    assert!(!output.status.success());
    for entry in entries {
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(
            proof.status,
            if entry == "overflow" {
                ProofStatus::Refuted
            } else {
                ProofStatus::Proved
            },
            "{entry}: {:?}",
            proof.obligations
        );
        assert!(
            proof
                .analyzed_bodies
                .iter()
                .any(|body| body.starts_with("dependency::"))
        );
    }
}

#[test]
fn result_question_mark_preserves_success_and_error_payloads() {
    let directory = Directory::new();
    let path = directory.0.join("result_calls.rs");
    std::fs::write(
        &path,
        r#"
#![no_std]
fn inner(value: u8) -> Result<u8, u8> {
    if value <= 8 { Ok(value) } else { Err(value) }
}

fn outer(value: u8) -> Result<u8, u8> { Ok(inner(value)?) }
pub fn payload(value: u8) {
    match outer(value) {
        Ok(result) => { assert!(value <= 8); assert!(result == value); }
        Err(error) => { assert!(value > 8); assert!(error == value); }
    }
}
"#,
    )
    .unwrap();
    let (output, report) = verify_vendored(&path, &["payload"], &[]);
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "payload")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Proved, "{:?}", proof.obligations);
    assert!(output.status.success());
    assert!(
        proof
            .analyzed_bodies
            .iter()
            .any(|body| body.contains("as core::ops::Try>::branch"))
    );
}

#[test]
fn cargo_entries_select_workspace_roots_and_missing_or_failed_roots_cannot_pass() {
    let directory = Directory::new();
    std::fs::write(
        directory.0.join("Cargo.toml"),
        "[workspace]\nmembers=['first','second']\nresolver='3'\n",
    )
    .unwrap();
    for name in ["first", "second"] {
        let member = directory.0.join(name);
        std::fs::create_dir(&member).unwrap();
        std::fs::write(
            member.join("Cargo.toml"),
            format!(
                "[package]\nname='{name}'\nversion='0.1.0'\nedition='2024'\n\
                 [lib]\npath='lib.rs'\n"
            ),
        )
        .unwrap();
        std::fs::write(
            member.join("lib.rs"),
            if name == "first" {
                "#![no_std]\npub fn read(value: u8) -> u8 { value }\n\
                 pub fn bad(value: u8) -> u8 { value + 1 }\n"
            } else {
                "#![no_std]\npub fn read(value: fn(u8) -> u8) -> u8 { value(0) }\n"
            },
        )
        .unwrap();
    }
    for (entries, expected, counts) in [
        (vec!["first::read"], true, (1, 0, 0)),
        (vec!["read"], false, (1, 0, 1)),
        (vec!["first::read", "first::bad"], false, (1, 1, 0)),
        (vec!["first::read", "absent"], false, (1, 0, 0)),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-mir-check"));
        command.args([
            "mir-check",
            "--verify",
            "--summary",
            "--workspace",
            "--offline",
        ]);
        for entry in &entries {
            if entries.len() == 1 {
                command.arg(format!("--entry={entry}"));
            } else {
                command.args(["--entry", entry]);
            }
        }
        let output = command.current_dir(&directory.0).output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(output.status.success(), expected, "{entries:?}: {stderr}");
        let text = if expected { &stdout } else { &stderr };
        let reports = text
            .lines()
            .find_map(|line| line.strip_prefix("JSON reports: "))
            .unwrap();
        let reports: Vec<Report> = std::fs::read_dir(reports)
            .unwrap()
            .map(|entry| {
                serde_json::from_slice(&std::fs::read(entry.unwrap().path()).unwrap()).unwrap()
            })
            .collect();
        assert_eq!(reports.len(), 2);
        let actual = reports.iter().fold((0, 0, 0), |total, report| {
            (
                total.0 + report.coverage.proved,
                total.1 + report.coverage.refuted,
                total.2 + report.coverage.unknown,
            )
        });
        assert_eq!(actual, counts, "{entries:?}: {stdout}");
        assert!(stdout.contains("not line coverage or whole-crate safety"));
        if entries.contains(&"read") {
            assert!(stdout.contains("unsupported argument type"));
        }
        if entries.contains(&"absent") {
            assert!(stderr.contains("entry \"absent\" has no inventoried MIR body"));
        }
    }
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-mir-check"))
        .args(["--verify", "--entry", "--lib"])
        .current_dir(&directory.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a function name"));
}

#[test]
fn coverage_counts_root_results_separately_from_unselected_and_interpreted_bodies() {
    let directory = Directory::new();
    let output = analyze_from(
        &fixture("proofs.rs"),
        &directory,
        &[
            "--verify",
            "--entry",
            "next_byte",
            "--entry",
            "overflowing_sum",
            "--entry",
            "loop_unknown",
        ],
        &["-Coverflow-checks=yes"],
    );
    assert!(!output.status.success());
    let analysis: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(analysis.schema_version, 8);
    let coverage = &analysis.coverage;
    assert_eq!(coverage.selected_roots, 3);
    assert_eq!(
        (coverage.proved, coverage.refuted, coverage.unknown),
        (1, 1, 1)
    );
    assert_eq!(coverage.unselected_bodies, analysis.functions.len() - 3);
    assert!(coverage.interpreted_instances >= 3);
    assert!(
        coverage
            .gaps
            .iter()
            .any(|gap| gap.roots == ["loop_unknown"])
    );
    let inventory = report(analyze(&fixture("proofs.rs"), &directory, &[]));
    assert_eq!(inventory.coverage.selected_roots, 0);
    assert_eq!(inventory.coverage.proved, 0);
    assert_eq!(
        inventory.coverage.unselected_bodies,
        inventory.functions.len()
    );
}

#[test]
fn nested_structs_tuples_and_shared_fields_preserve_call_bounds_on_host_and_arm() {
    let directory = Directory::new();
    let path = directory.0.join("aggregate_inputs.rs");
    std::fs::write(
        &path,
        r#"
#![no_std]
use mir_contracts::{requires, ensures};
pub struct Header { index: usize }
pub struct Wrapper<T> { value: T }
pub struct Packet<'a> { header: Wrapper<Header>, bytes: &'a [u8] }
pub struct Pair(pub u16, pub bool);
#[requires(index < bytes.len())]
fn read(bytes: &[u8], index: usize) -> u8 { bytes[index] }
pub fn guarded(packet: &Packet<'_>) -> u8 {
    if packet.header.value.index < packet.bytes.len() {
        read(packet.bytes, packet.header.value.index)
    } else { 0 }
}
pub fn invalid(packet: &Packet<'_>) -> u8 {
    if packet.header.value.index <= packet.bytes.len() {
        read(packet.bytes, packet.header.value.index)
    } else { 0 }
}
#[requires(packet.header.value.index < packet.bytes.len())]
pub fn precondition(packet: Packet<'_>) -> u8 {
    read(packet.bytes, packet.header.value.index)
}
#[requires(value.0 < 16 && value.1.0 <= 7)]
#[ensures(result.0 == value.0 && result.1 == value.1.0)]
pub fn tuple(value: &(u16, (u8, bool))) -> (u16, u8) { (value.0, value.1.0) }
#[ensures(result == value.0)]
pub fn tuple_struct(value: Pair) -> u16 { value.0 }
#[requires(index == 1)]
#[ensures(result == values[1].index)]
pub fn fixed_structs(values: [Header; 2], index: usize) -> usize { values[index].index }
"#,
    )
    .unwrap();
    let entries = [
        "guarded",
        "invalid",
        "precondition",
        "tuple",
        "tuple_struct",
        "fixed_structs",
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let (output, report) = verify_vendored(&path, &entries, &args);
        assert!(!output.status.success());
        for entry in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == entry)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status,
                if entry == "invalid" {
                    ProofStatus::Refuted
                } else {
                    ProofStatus::Proved
                },
                "{entry}: {:?}",
                proof.obligations
            );
            if entry == "guarded" {
                assert!(proof.inputs.contains_key("packet.header.value.index"));
                assert!(proof.inputs.contains_key("packet.bytes"));
                assert!(proof.obligations.iter().any(
                    |obligation| obligation.kind == mir_check::ObligationKind::CallPrecondition
                ));
            }
            if entry == "tuple" {
                assert!(proof.inputs.contains_key("value.1.0"));
            }
        }
    }
}

#[test]
fn recursive_and_mutable_shapes_stay_unknown_while_larger_bounded_inputs_prove() {
    let directory = Directory::new();
    let path = directory.0.join("unknown_inputs.rs");
    std::fs::write(
        &path,
        r#"
#![no_std]
pub struct Node<'a> { next: &'a Node<'a> }
pub struct Mutable<'a> { bytes: &'a mut [u8; 4] }
pub fn recursive<'a>(node: &'a Node<'a>) -> &'a Node<'a> { node }
pub fn large(values: [([u16; 16], [u16; 16]); 8]) -> u16 { values[0].0[0] }
pub fn mutable(packet: Mutable<'_>) { packet.bytes[0] = 1; }
"#,
    )
    .unwrap();
    let entries = ["recursive", "large", "mutable"];
    let (output, report) = verify_vendored(&path, &entries, &[]);
    assert!(!output.status.success());
    for entry in entries {
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(
            proof.status,
            if entry == "large" {
                ProofStatus::Proved
            } else {
                ProofStatus::Unknown
            },
            "{entry}: {:?}",
            proof.obligations
        );
        if entry != "large" {
            assert!(proof.obligations.iter().any(|obligation| {
                obligation.detail.contains(if entry == "mutable" {
                    "unsupported argument type"
                } else {
                    "input shape exceeds"
                })
            }));
        }
    }
}

#[test]
fn cargo_selected_real_parser_and_nested_packet_prove_on_host_and_arm() {
    let directory = Directory::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (example, entry) in [("dr16", "Raw::parse"), ("contracts", "guarded_packet_read")] {
        let manifest = root.join(format!("examples/{example}/Cargo.toml"));
        for target in [None, Some("thumbv7em-none-eabihf")] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-mir-check"));
            command
                .args([
                    "--verify",
                    "--summary",
                    "--entry",
                    entry,
                    "--lib",
                    "--offline",
                ])
                .arg("--manifest-path")
                .arg(&manifest);
            if let Some(target) = target {
                command.args(["--target", target]);
            }
            let output = command.current_dir(&directory.0).output().unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(
                output.status.success(),
                "{example}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(stdout.contains(&format!("PROVED {entry}")));
            let reports = stdout
                .lines()
                .find_map(|line| line.strip_prefix("JSON reports: "))
                .unwrap();
            let report_file = std::fs::read_dir(reports)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let report: Report =
                serde_json::from_slice(&std::fs::read(report_file).unwrap()).unwrap();
            assert_eq!(report.coverage.selected_roots, 1);
            assert_eq!(report.coverage.proved, 1);
            assert!(report.coverage.unselected_bodies > 0);
            if let Some(target) = target {
                assert_eq!(report.target, target);
            } else {
                assert!(report.target.contains(std::env::consts::ARCH));
            }
        }
    }
}

#[test]
fn floating_paths_preserve_nan_zero_rounding_saturation_and_call_bounds_on_host_and_arm() {
    let path = fixture("floating.rs");
    let entries = [
        ("arithmetic", ProofStatus::Proved),
        ("float_methods", ProofStatus::Proved),
        ("bad_min_zero", ProofStatus::Refuted),
        ("guarded_index", ProofStatus::Proved),
        ("bad_index", ProofStatus::Refuted),
        ("bad_nan", ProofStatus::Refuted),
        ("bad_zero", ProofStatus::Refuted),
        ("cast_edges", ProofStatus::Proved),
        ("casts_and_rounding", ProofStatus::Proved),
        ("bounded", ProofStatus::Proved),
        ("guarded_call", ProofStatus::Proved),
        ("bad_call", ProofStatus::Refuted),
        ("unsupported_remainder", ProofStatus::Unknown),
        ("unsupported_bits", ProofStatus::Proved),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify_vendored(&path, &names, &args);
        assert!(!output.status.success());
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
        }
    }
    let nan = f64::NAN;
    let equal = |left: f64, right: f64| left == right;
    assert!(!equal(nan, nan));
    assert_eq!(1.0_f32 / -0.0, f32::NEG_INFINITY);
    assert_eq!(nan as i32, 0);
    assert_eq!(f64::INFINITY as i32, i32::MAX);
    assert_eq!(f64::NEG_INFINITY as i32, i32::MIN);
    assert_eq!((16_777_217_u32 as f32) as u32, 16_777_216);
    assert_eq!(f32::MAX as u128, u128::MAX - ((1_u128 << 104) - 1));
}

#[test]
fn population_counts_are_exact_for_signed_and_target_width_integers_on_host_and_arm() {
    let entries = [
        ("byte", ProofStatus::Proved),
        ("widths", ProofStatus::Proved),
        ("signed", ProofStatus::Proved),
        ("masked", ProofStatus::Proved),
        ("bad_bound", ProofStatus::Refuted),
        ("bad_signed", ProofStatus::Refuted),
        ("user_method", ProofStatus::Refuted),
        ("unsupported", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify_vendored(&fixture("population_count.rs"), &names, &args);
        assert!(!output.status.success());
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
            if expected == ProofStatus::Proved {
                assert!(
                    proof
                        .models
                        .iter()
                        .any(|model| model.contains("exact integer population"))
                );
            }
            if name == "user_method" {
                assert!(
                    !proof
                        .models
                        .iter()
                        .any(|model| model.contains("population count"))
                );
            }
        }
    }
    for value in 0_u8..=u8::MAX {
        let expected: u32 = (0..8).map(|bit| u32::from((value >> bit) & 1)).sum();
        assert_eq!(value.count_ones(), expected);
        assert_eq!((value as i8).count_ones(), expected);
        assert_eq!(value.count_zeros(), 8 - expected);
    }
}

#[test]
fn float_clamp_checks_bounds_and_preserves_nan_and_zero_on_host_and_arm() {
    let entries = [
        ("guarded", ProofStatus::Proved),
        ("guarded_double", ProofStatus::Proved),
        ("special_values", ProofStatus::Proved),
        ("unchecked", ProofStatus::Refuted),
        ("reversed", ProofStatus::Refuted),
        ("nan_min", ProofStatus::Refuted),
        ("nan_max", ProofStatus::Refuted),
        ("bad_nan", ProofStatus::Refuted),
        ("bad_zero", ProofStatus::Refuted),
        ("user_method", ProofStatus::Refuted),
        ("unsupported", ProofStatus::Proved),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify_vendored(&fixture("float_clamp.rs"), &names, &args);
        assert!(!output.status.success());
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
            if name == "user_method" {
                assert!(
                    !proof
                        .models
                        .iter()
                        .any(|model| model.contains("floating-point clamp"))
                );
            } else {
                assert!(
                    proof
                        .models
                        .iter()
                        .any(|model| model.contains("checked ordered bounds"))
                );
            }
        }
    }
    assert!(f32::NAN.clamp(-1.0, 1.0).is_nan());
    assert_eq!((-0.0_f32).clamp(0.0, 0.0).to_bits(), (-0.0_f32).to_bits());
    assert_eq!(0.0_f64.clamp(-0.0, -0.0).to_bits(), 0.0_f64.to_bits());
    assert_eq!(f64::INFINITY.clamp(-1.0, 1.0), 1.0);
}

#[test]
fn changed_population_bounds_and_reversed_clamp_guards_are_rejected() {
    let directory = Directory::new();
    for (file, entry, before, after) in [
        (
            "population_count.rs",
            "masked",
            "active <= 3",
            "active <= 2",
        ),
        ("float_clamp.rs", "guarded", "min <= max", "min >= max"),
    ] {
        let source = std::fs::read_to_string(fixture(file)).unwrap();
        assert!(source.contains(before));
        let path = directory.0.join(file);
        std::fs::write(&path, source.replace(before, after)).unwrap();
        let (output, report) = verify_vendored(&path, &[entry], &[]);
        assert!(!output.status.success());
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(
            proof.status,
            ProofStatus::Refuted,
            "{file}: {:?}",
            proof.obligations
        );
        assert!(
            proof
                .obligations
                .iter()
                .any(|obligation| obligation.model.is_some())
        );
    }
}

#[test]
fn slice_iterators_preserve_order_cursors_and_predicate_calls_on_host_and_arm() {
    let entries = [
        ("ordered", ProofStatus::Proved),
        ("skips", ProofStatus::Proved),
        ("clone_cursor", ProofStatus::Proved),
        ("enumerate", ProofStatus::Proved),
        ("borrowed_array", ProofStatus::Proved),
        ("adapters", ProofStatus::Proved),
        ("bounded_bytes", ProofStatus::Proved),
        ("all_any", ProofStatus::Proved),
        ("short_circuit", ProofStatus::Proved),
        ("no_callback_after_stopping", ProofStatus::Proved),
        ("predicate_effects", ProofStatus::Proved),
        ("shared_cells", ProofStatus::Proved),
        ("floats", ProofStatus::Proved),
        ("float_classes", ProofStatus::Proved),
        ("bad_finite", ProofStatus::Refuted),
        ("composite", ProofStatus::Proved),
        ("units", ProofStatus::Proved),
        ("bad_order", ProofStatus::Refuted),
        ("bad_callback", ProofStatus::Refuted),
        ("exhausted", ProofStatus::Refuted),
        ("unbounded", ProofStatus::Unknown),
        ("unsupported_view", ProofStatus::Unknown),
        ("user_method", ProofStatus::Refuted),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify_vendored(&fixture("slice_iterators.rs"), &names, &args);
        assert!(!output.status.success());
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
            if name == "user_method" {
                assert!(
                    !proof
                        .models
                        .iter()
                        .any(|model| model.contains("tracked cursor"))
                );
            }
        }
    }
}

#[test]
fn changing_iterator_order_or_short_circuit_behavior_refutes_the_passing_roots() {
    let source = std::fs::read_to_string(fixture("slice_iterators.rs")).unwrap();
    let directory = Directory::new();
    for (entry, original, mutation) in [
        ("ordered", "== values[0]", "== values[1]"),
        (
            "no_callback_after_stopping",
            "        false\n",
            "        true\n",
        ),
    ] {
        assert!(source.contains(original));
        let path = directory.0.join("slice_iterators.rs");
        std::fs::write(&path, source.replace(original, mutation)).unwrap();
        let (output, report) = verify_vendored(&path, &[entry], &[]);
        assert!(!output.status.success());
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, ProofStatus::Refuted);
        assert!(proof.obligations.iter().any(|o| o.model.is_some()));
    }
}

#[test]
fn slice_iterator_proofs_agree_with_exhaustive_host_cases() {
    let directory = Directory::new();
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-check")).parent().unwrap();
    let library = find_contract_library(profile).unwrap();
    for file in ["slice_iterators.rs", "mutable_iterators.rs"] {
        let binary = directory.0.join("slice-iterator-tests");
        let build = Command::new("rustup")
            .args([
                "run",
                "nightly-2026-09-22",
                "rustc",
                "--edition=2024",
                "--test",
            ])
            .arg(fixture(file))
            .arg("--extern")
            .arg(format!("mir_contracts={}", library.display()))
            .arg("-L")
            .arg(format!("dependency={}", profile.join("deps").display()))
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{file}: {}",
            String::from_utf8_lossy(&build.stderr)
        );
        let run = Command::new(binary).output().unwrap();
        assert!(
            run.status.success(),
            "{file}: {}",
            String::from_utf8_lossy(&run.stdout)
        );
    }
}

#[test]
fn mutable_slice_iterators_preserve_source_writes_and_disjoint_elements_on_host_and_arm() {
    let entries = [
        ("set_all", ProofStatus::Proved),
        ("borrowed_array", ProofStatus::Proved),
        ("disjoint", ProofStatus::Proved),
        ("diagonal", ProofStatus::Proved),
        ("pairs", ProofStatus::Proved),
        ("prefix_bytes", ProofStatus::Proved),
        ("bounded_bytes", ProofStatus::Proved),
        ("mutable_predicate", ProofStatus::Proved),
        ("bad_alias", ProofStatus::Refuted),
        ("bad_overflow", ProofStatus::Refuted),
        ("ambiguous", ProofStatus::Unknown),
        ("escaping", ProofStatus::Proved),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify_vendored(&fixture("mutable_iterators.rs"), &names, &args);
        assert!(!output.status.success());
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status,
                expected,
                "{target:?} {name}: {:?}",
                proof
                    .obligations
                    .iter()
                    .map(|o| (&o.detail, o.status))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn changing_mutable_enumeration_writes_to_the_wrong_column_refutes_the_matrix() {
    let source = std::fs::read_to_string(fixture("mutable_iterators.rs")).unwrap();
    let original = "row[i] =";
    assert!(source.contains(original));
    let directory = Directory::new();
    let path = directory.0.join("mutable_iterators.rs");
    std::fs::write(&path, source.replace(original, "row[(i + 1) % 6] =")).unwrap();
    let (output, report) = verify_vendored(&path, &["diagonal"], &[]);
    assert!(!output.status.success());
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "diagonal")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Refuted);
    assert!(proof.obligations.iter().any(|o| o.model.is_some()));
}

#[test]
fn owned_aggregate_repeats_preserve_independent_copies_and_keep_storage_limits_on_host_and_arm() {
    let entries = [
        ("matrix", ProofStatus::Proved),
        ("diagonal", ProofStatus::Proved),
        ("structures", ProofStatus::Proved),
        ("tuples", ProofStatus::Proved),
        ("variants", ProofStatus::Proved),
        ("empty_tuple", ProofStatus::Proved),
        ("larger", ProofStatus::Proved),
        ("maximum", ProofStatus::Proved),
        ("large_input", ProofStatus::Proved),
        ("value_budget_limit", ProofStatus::Proved),
        ("over_value_budget", ProofStatus::Unknown),
        ("bad_copy", ProofStatus::Refuted),
        ("too_many", ProofStatus::Unknown),
        ("too_large", ProofStatus::Unknown),
        ("ambiguous", ProofStatus::Unknown),
        ("references", ProofStatus::Unknown),
        ("interior", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify_vendored(&fixture("owned_repeats.rs"), &names, &args);
        assert!(!output.status.success());
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
            if name == "too_large" {
                assert!(
                    proof
                        .obligations
                        .iter()
                        .any(|o| o.detail.contains("256-value budget"))
                );
            }
        }
    }
}

#[test]
fn changing_a_nested_repeat_write_to_another_row_refutes_the_original_assertion() {
    let source = std::fs::read_to_string(fixture("owned_repeats.rs")).unwrap();
    let original = "cells[2][3] = 7.0;";
    assert!(source.contains(original));
    let directory = Directory::new();
    let path = directory.0.join("owned_repeats.rs");
    std::fs::write(&path, source.replace(original, "cells[1][3] = 7.0;")).unwrap();
    let (output, report) = verify_vendored(&path, &["matrix"], &[]);
    assert!(!output.status.success());
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "matrix")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(
        proof.status,
        ProofStatus::Refuted,
        "{:?}",
        proof.obligations
    );
    assert!(proof.obligations.iter().any(|o| o.model.is_some()));
}

#[test]
fn aggregate_constants_preserve_variants_fields_and_initialized_memory_on_host_and_arm() {
    let entries = [
        ("guarded_as_ref", ProofStatus::Proved),
        ("bad_as_ref", ProofStatus::Refuted),
        ("none", ProofStatus::Proved),
        ("some", ProofStatus::Proved),
        ("wrong_payload", ProofStatus::Refuted),
        ("niche", ProofStatus::Proved),
        ("niche_get", ProofStatus::Proved),
        ("niche_reference", ProofStatus::Proved),
        ("discriminants", ProofStatus::Proved),
        ("nested", ProofStatus::Proved),
        ("promoted", ProofStatus::Proved),
        ("static_array", ProofStatus::Proved),
        ("constant_slice", ProofStatus::Proved),
        ("maximum_bytes", ProofStatus::Proved),
        ("uninitialized", ProofStatus::Unknown),
        ("inactive_uninitialized", ProofStatus::Proved),
        ("active_uninitialized", ProofStatus::Unknown),
        ("interior_mutable", ProofStatus::Proved),
        ("mutable_static_storage", ProofStatus::Refuted),
        ("initialized_union", ProofStatus::Unknown),
        ("raw_pointer", ProofStatus::Unknown),
        ("oversized_array", ProofStatus::Proved),
        ("oversized_bytes", ProofStatus::Unknown),
        ("oversized_shape", ProofStatus::Unknown),
        ("deep_shape", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify_vendored(&fixture("aggregate_constants.rs"), &names, &args);
        assert!(!output.status.success());
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
        }
        let guarded = report
            .functions
            .iter()
            .find(|f| f.name == "guarded_as_ref")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert!(
            guarded
                .analyzed_bodies
                .iter()
                .any(|name| name.contains("as_ref"))
        );
        let mutable_static = report
            .functions
            .iter()
            .find(|f| f.name == "mutable_static_storage")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert!(
            mutable_static
                .obligations
                .iter()
                .any(|o| { o.status == ProofStatus::Refuted })
        );
        let directory = Directory::new();
        let mutated = directory.0.join("mutated.rs");
        std::fs::write(
            &mutated,
            std::fs::read_to_string(fixture("aggregate_constants.rs"))
                .unwrap()
                .replace("index: Some(2)", "index: Some(4)"),
        )
        .unwrap();
        let (output, report) = verify_vendored(&mutated, &["nested"], &args);
        assert!(!output.status.success());
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == "nested")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, ProofStatus::Refuted);
        assert!(
            proof
                .obligations
                .iter()
                .any(|o| { o.status == ProofStatus::Refuted && o.detail.contains("BoundsCheck") })
        );
    }
}

#[test]
fn enum_inputs_preserve_tags_payloads_and_option_contracts_on_host_and_arm() {
    let entries = [
        ("discriminants", ProofStatus::Proved),
        ("bad_variant", ProofStatus::Refuted),
        ("guarded", ProofStatus::Proved),
        ("bad_payload", ProofStatus::Refuted),
        ("bounded_option", ProofStatus::Proved),
        ("guarded_option", ProofStatus::Proved),
        ("bad_option", ProofStatus::Refuted),
        ("snapshot", ProofStatus::Proved),
        ("result_payload", ProofStatus::Proved),
        ("generic_enum", ProofStatus::Unknown),
        ("mutable_enum", ProofStatus::Proved),
        ("enum_slice", ProofStatus::Unknown),
        ("large", ProofStatus::Proved),
        ("empty", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify_vendored(&fixture("enum_inputs.rs"), &names, &args);
        assert!(!output.status.success());
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
        }
        assert!(
            report
                .functions
                .iter()
                .find(|f| f.name == "bounded_option")
                .unwrap()
                .proof
                .as_ref()
                .unwrap()
                .inputs
                .contains_key("index.discriminant")
        );
    }
}

#[test]
fn dependency_defined_inputs_preserve_nested_fields_and_enum_payloads_on_host_and_arm() {
    let directory = Directory::new();
    let dependency = directory.0.join("dependency.rs");
    std::fs::write(
        &dependency,
        r#"
#![no_std]
pub struct Packet { pub index: Option<usize>, pub bytes: [u8; 4], pub gain: f32 }
pub enum Message { Missing, Sample(Packet) }
"#,
    )
    .unwrap();
    let source = directory.0.join("consumer.rs");
    std::fs::write(&source, r#"
#![no_std]
pub fn read(packet: &dependency::Packet) -> u8 {
    match packet.index {
        Some(index) if packet.gain > 0.0 && index < packet.bytes.len() => packet.bytes[index],
        _ => 0,
    }
}

pub fn bad(packet: &dependency::Packet) -> u8 {
    match packet.index {
        Some(index) if packet.gain > 0.0 && index <= packet.bytes.len() => packet.bytes[index],
        _ => 0,
    }
}
pub fn message(value: &dependency::Message) -> u8 {
    match value { dependency::Message::Missing => 0, dependency::Message::Sample(packet) => read(packet) }
}
"#).unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let library = directory.0.join(if target.is_some() {
            "libarm.rlib"
        } else {
            "libhost.rlib"
        });
        let mut build = Command::new("rustup");
        build.args([
            "run",
            "nightly-2026-09-22",
            "rustc",
            "--crate-type=rlib",
            "--edition=2024",
            "--crate-name=dependency",
            "-Cpanic=abort",
        ]);
        if let Some(target) = target {
            build.args(["--target", target]);
        }
        let output = build
            .arg(&dependency)
            .arg("-o")
            .arg(&library)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let external = format!("dependency={}", library.display());
        let mut args = vec!["--extern", &external];
        if let Some(target) = target {
            args.extend(["--target", target]);
        }
        let (output, report) = verify_vendored(&source, &["read", "bad", "message"], &args);
        assert!(!output.status.success());
        for name in ["read", "bad", "message"] {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status,
                if name == "bad" {
                    ProofStatus::Refuted
                } else {
                    ProofStatus::Proved
                },
                "{target:?} {name}: {:?}",
                proof.obligations
            );
        }
    }
}

#[test]
fn array_patterns_preserve_minimum_lengths_and_start_and_end_offsets_on_host_and_arm() {
    let entries = [
        ("array", ProofStatus::Proved),
        ("slice", ProofStatus::Proved),
        ("bad_slice", ProofStatus::Refuted),
        ("unsupported_slice", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify_vendored(&fixture("array_patterns.rs"), &names, &args);
        assert!(!output.status.success());
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
        }
    }
}

#[test]
fn dependency_mir_retention_preserves_transitive_bodies_bounds_and_build_flags_on_host_and_arm() {
    let directory = Directory::new();
    let consumer = directory.0.join("consumer");
    for name in ["consumer", "dependency", "leaf"] {
        std::fs::create_dir(directory.0.join(name)).unwrap();
        std::fs::write(
            directory.0.join(name).join("Cargo.toml"),
            format!(
                "[package]\nname='{name}'\nversion='0.1.0'\nedition='2024'\n\
                 [lib]\npath='lib.rs'\n[workspace]\n[profile.dev]\nopt-level=2\n\
                 debug=0\noverflow-checks=true\n{}",
                match name {
                    "consumer" => "[dependencies]\ndependency={path='../dependency'}\n",
                    "dependency" => "[dependencies]\nleaf={path='../leaf'}\n",
                    "leaf" => "",
                    _ => unreachable!(),
                }
            ),
        )
        .unwrap();
    }
    std::fs::write(
        directory.0.join("leaf/lib.rs"),
        r#"
#![no_std]
#![forbid(unsafe_code)]
#[cfg(not(configured_build))]
compile_error!("dependency build flags were lost");
pub fn increment(value: u8) -> u8 { value + 1 }
pub fn bits(value: f32) -> u32 { value.to_bits() }
"#,
    )
    .unwrap();
    std::fs::write(
        directory.0.join("dependency/lib.rs"),
        r#"
#![no_std]
#![forbid(unsafe_code)]
#[doc = "<!-- mir-check:v1:requires:value < 255 -->"]
#[doc = "<!-- mir-check:v1:ensures:result > value -->"]
pub fn increment(value: u8) -> u8 { leaf::increment(value) }
pub fn unchecked(value: u8) -> u8 { leaf::increment(value) }
pub fn bits(value: f32) -> u32 { leaf::bits(value) }
"#,
    )
    .unwrap();
    std::fs::write(
        consumer.join("lib.rs"),
        r#"
#![no_std]
#![forbid(unsafe_code)]
#[cfg(not(configured_build))]
compile_error!("workspace build flags were lost");
#[cfg(ignored_build)]
compile_error!("RUSTFLAGS must not override CARGO_ENCODED_RUSTFLAGS");
pub fn guarded(value: u8) -> u8 {
    if value < 255 { dependency::increment(value) } else { 0 }
}
pub fn bad_call() -> u8 { dependency::increment(255) }
pub fn overflow() -> u8 { dependency::unchecked(255) }
pub fn unsupported(value: f32) -> u32 { dependency::bits(value) }
"#,
    )
    .unwrap();
    std::fs::create_dir(consumer.join(".cargo")).unwrap();
    std::fs::write(
        consumer.join(".cargo/config.toml"),
        "[build]\nrustflags=['--cfg=configured_build','-Cpanic=abort',\
         '-Coverflow-checks=yes','-Zmir-opt-level=3']\n",
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for (retain, encoded) in [(false, false), (true, false), (true, true)] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-mir-check"));
            command
                .args(["--verify", "--summary", "--lib", "--offline"])
                .current_dir(&consumer)
                .env_remove("RUSTFLAGS")
                .env_remove("CARGO_ENCODED_RUSTFLAGS");
            for entry in ["guarded", "bad_call", "overflow", "unsupported"] {
                command.args(["--entry", entry]);
            }
            if !retain {
                command.arg("--no-dependency-mir");
            }
            if encoded {
                command.env("RUSTFLAGS", "--cfg=ignored_build");
                command.env(
                    "CARGO_ENCODED_RUSTFLAGS",
                    [
                        "--cfg=configured_build",
                        "--cfg=encoded_build=\"two words\"",
                        "-Cpanic=abort",
                        "-Coverflow-checks=yes",
                        "-Zmir-opt-level=3",
                    ]
                    .join("\u{1f}"),
                );
            }
            if let Some(target) = target {
                command.args(["--target", target]);
            }
            let output = command.output().unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(!output.status.success(), "{stdout}");
            let reports = stderr
                .lines()
                .find_map(|line| line.strip_prefix("JSON reports: "))
                .unwrap_or_else(|| panic!("{target:?} {retain} {encoded}: {stderr}"));
            let paths: Vec<_> = std::fs::read_dir(reports)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            assert_eq!(
                paths.len(),
                1,
                "dependencies must not become root inventories"
            );
            let report: Report =
                serde_json::from_slice(&std::fs::read(&paths[0]).unwrap()).unwrap();
            assert_eq!(report.crate_name, "consumer");
            assert!(report.overflow_checks);
            assert_eq!(report.panic_strategy, "abort");
            assert!(
                report
                    .rustc_arguments
                    .iter()
                    .any(|arg| arg == "-Zmir-opt-level=3")
            );
            if retain {
                assert!(
                    report
                        .rustc_arguments
                        .iter()
                        .any(|arg| arg == "-Zalways-encode-mir=yes")
                );
                assert!(
                    report
                        .rustc_arguments
                        .iter()
                        .any(|arg| arg == "-Zmir-opt-level=0")
                );
            }
            if encoded {
                assert!(
                    report
                        .rustc_arguments
                        .iter()
                        .any(|arg| arg == "--cfg=encoded_build=\"two words\"")
                );
            }
            for (name, expected) in [
                ("guarded", ProofStatus::Proved),
                ("bad_call", ProofStatus::Refuted),
                ("overflow", ProofStatus::Refuted),
                ("unsupported", ProofStatus::Proved),
            ] {
                let proof = report
                    .functions
                    .iter()
                    .find(|f| f.name == name)
                    .unwrap()
                    .proof
                    .as_ref()
                    .unwrap();
                assert_eq!(
                    proof.status,
                    if retain {
                        expected
                    } else {
                        ProofStatus::Unknown
                    },
                    "{target:?} {retain} {encoded} {name}: {:?}",
                    proof.obligations
                );
                if retain && name == "guarded" {
                    assert!(
                        proof
                            .analyzed_bodies
                            .iter()
                            .any(|body| body.starts_with("leaf::increment"))
                    );
                    assert!(
                        proof.obligations.iter().any(|obligation| obligation.kind
                            == mir_check::ObligationKind::CallPrecondition)
                    );
                }
                if retain && name == "unsupported" {
                    assert!(
                        proof
                            .analyzed_bodies
                            .iter()
                            .any(|body| body.starts_with("leaf::bits"))
                    );
                }
                if !retain {
                    assert!(
                        proof
                            .obligations
                            .iter()
                            .any(|obligation| obligation.detail.contains("MIR body unavailable"))
                    );
                }
            }
        }
    }
}

#[test]
fn mutable_storage_preserves_call_writes_branch_states_and_entry_snapshots_on_host_and_arm() {
    let entries = [
        ("increment", ProofStatus::Proved),
        ("bad_increment", ProofStatus::Refuted),
        ("calls", ProofStatus::Proved),
        ("bad_calls", ProofStatus::Refuted),
        ("branches", ProofStatus::Proved),
        ("local_reborrows", ProofStatus::Proved),
        ("read_after_write", ProofStatus::Proved),
        ("arrays", ProofStatus::Proved),
        ("bytes", ProofStatus::Proved),
        ("bad_bytes", ProofStatus::Refuted),
        ("two_mutable", ProofStatus::Proved),
        ("nested_reference", ProofStatus::Unknown),
        ("escaping_reference", ProofStatus::Proved),
        ("ambiguous_array", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify_vendored(&fixture("memory.rs"), &names, &args);
        assert!(!output.status.success());
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
        }
    }
}

#[test]
fn the_vendored_pid_updates_and_resets_preserve_writes_and_require_valid_limits_on_host_and_arm() {
    let fixture_text = std::fs::read_to_string(fixture("pid_memory.rs")).unwrap();
    let excerpt = fixture_text
        .split("pub struct Pid {")
        .nth(1)
        .unwrap()
        .split("pub fn configured")
        .next()
        .unwrap()
        .replace(
            "    #[requires(self.integral_max >= 0.0 && self.output_max >= 0.0)]\n",
            "",
        )
        .replace("    #[ensures(final_self.integral == 0.0)]\n", "");
    let source = include_str!("../../../tests/vendor/controller-pid.rs");
    assert_eq!(
        excerpt.trim(),
        source
            .split("pub struct Pid {")
            .nth(1)
            .unwrap()
            .split("/// A scalar loop:")
            .next()
            .unwrap()
            .trim()
    );
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let (_, report) = verify_vendored(
            &fixture("pid_memory.rs"),
            &["Pid::reset", "configured", "bad_limits"],
            &args,
        );
        for (name, expected) in [
            ("Pid::reset", ProofStatus::Proved),
            ("configured", ProofStatus::Proved),
            ("bad_limits", ProofStatus::Refuted),
        ] {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
        }
    }
    let directory = Directory::new();
    let mutated = directory.0.join("mutated.rs");
    std::fs::write(
        &mutated,
        std::fs::read_to_string(fixture("pid_memory.rs"))
            .unwrap()
            .replace("self.integral = 0.0;", "self.integral = 1.0;"),
    )
    .unwrap();
    let (_, report) = verify_vendored(&mutated, &["Pid::reset"], &[]);
    assert_eq!(
        report
            .functions
            .iter()
            .find(|f| f.name == "Pid::reset")
            .unwrap()
            .proof
            .as_ref()
            .unwrap()
            .status,
        ProofStatus::Refuted
    );
}

#[test]
fn cells_preserve_alias_writes_and_atomics_check_orderings_without_assuming_history_on_host_and_arm()
 {
    let entries = [
        ("cell_aliases", ProofStatus::Proved),
        ("cell_callback", ProofStatus::Proved),
        ("cell_calls", ProofStatus::Proved),
        ("bad_cell", ProofStatus::Refuted),
        ("cell_branches", ProofStatus::Proved),
        ("two_cells", ProofStatus::Unknown),
        ("site", ProofStatus::Proved),
        ("Site::fail", ProofStatus::Proved),
        ("counters", ProofStatus::Proved),
        ("wrong_increment", ProofStatus::Refuted),
        ("unsupported_history", ProofStatus::Refuted),
        ("bad_load", ProofStatus::Refuted),
        ("bad_store", ProofStatus::Refuted),
        ("guarded_ordering", ProofStatus::Proved),
        ("bad_ordering", ProofStatus::Refuted),
        ("refcell_conflict", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let args = target
            .map(|target| vec!["--target", target])
            .unwrap_or_default();
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify_vendored(&fixture("interior.rs"), &names, &args);
        assert!(!output.status.success());
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
        }
        assert!(
            report
                .functions
                .iter()
                .find(|f| f.name == "site")
                .unwrap()
                .proof
                .as_ref()
                .unwrap()
                .models
                .iter()
                .any(|model| model.contains("possible interference"))
        );
    }
}

#[test]
fn external_contracts_check_bounds_and_label_trusted_returns_and_effects_on_host_and_arm() {
    let directory = Directory::new();
    let config = directory.0.join("contracts.json");
    let spec = serde_json::json!({"schema_version":1,"functions":[{
        "function":"external_contracts::write_float",
        "arguments":["writer","value","precision"],
        "requires":["precision <= 6"],"trusted":true,"no_panic":true,"modifies":["writer"],
        "ensures":["final_writer.len <= 32"],
        "reason":"Accepted float formatting boundary for this test"
    }, {
        "function":"external_contracts::increment",
        "requires":["value < 255"],"ensures":["result > value"]
    }]});
    std::fs::write(&config, serde_json::to_vec_pretty(&spec).unwrap()).unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let rustc_args = target
            .map(|target| vec!["--target", target, "-Cpanic=abort", "-Coverflow-checks=yes"])
            .unwrap_or_else(|| vec!["-Cpanic=abort", "-Coverflow-checks=yes"]);
        let checker = [
            "--verify",
            "--contracts",
            config.to_str().unwrap(),
            "--entry",
            "caller",
            "--entry",
            "bad_bound",
            "--entry",
            "result_failure",
            "--entry",
            "stale",
            "--entry",
            "increment",
            "--entry",
            "checked_caller",
            "--entry",
            "bad_checked_caller",
            "--entry",
            "unrelated",
        ];
        let output = analyze_from(
            &fixture("external_contracts.rs"),
            &directory,
            &checker,
            &rustc_args,
        );
        assert!(
            !output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Report = serde_json::from_slice(&output.stdout).unwrap();
        for (name, expected) in [
            ("caller", ProofStatus::ProvedWithAssumptions),
            ("bad_bound", ProofStatus::Refuted),
            ("result_failure", ProofStatus::Refuted),
            ("stale", ProofStatus::Refuted),
            ("increment", ProofStatus::Proved),
            ("checked_caller", ProofStatus::Proved),
            ("bad_checked_caller", ProofStatus::Refuted),
            ("unrelated", ProofStatus::Proved),
        ] {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
        }
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == "caller")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.trusted_calls.len(), 1);
        assert_eq!(
            proof.trusted_calls[0].contract.modifies,
            Some(vec!["writer".to_owned()])
        );
        assert!(!proof.trusted_calls[0].crate_hash.is_empty());
        assert_eq!(report.coverage.proved_with_assumptions, 1);
        assert!(mir_check::render(&report).contains("USER TRUSTED"));
        let allowed = analyze_from(
            &fixture("external_contracts.rs"),
            &directory,
            &[
                "--verify",
                "--allow-assumptions",
                "--contracts",
                config.to_str().unwrap(),
                "--entry",
                "caller",
            ],
            &rustc_args,
        );
        assert!(
            allowed.status.success(),
            "{}",
            String::from_utf8_lossy(&allowed.stderr)
        );
        let strict = analyze_from(
            &fixture("external_contracts.rs"),
            &directory,
            &[
                "--verify",
                "--contracts",
                config.to_str().unwrap(),
                "--entry",
                "caller",
            ],
            &rustc_args,
        );
        assert!(!strict.status.success());
    }
}

#[test]
fn summary_defaults_invalidate_storage_and_never_prove_the_assumed_body() {
    let directory = Directory::new();
    let config = directory.0.join("contracts.json");
    let mut spec = serde_json::json!({"schema_version":1,"functions":[{
        "function":"external_contracts::write_float","arguments":["writer","value","precision"],
        "trusted":true,"no_panic":true,"reason":"Explicit test boundary"
    }, {
        "function":"external_contracts::assumed","arguments":["value"],"trusted":true,
        "no_panic":true,"modifies":[],"ensures":["result == value"],
        "reason":"Deliberately false assumption to verify reporting"
    }]});
    std::fs::write(&config, serde_json::to_vec(&spec).unwrap()).unwrap();
    let output = analyze_from(
        &fixture("external_contracts.rs"),
        &directory,
        &[
            "--verify",
            "--allow-assumptions",
            "--contracts",
            config.to_str().unwrap(),
            "--entry",
            "ignored",
            "--entry",
            "stale",
            "--entry",
            "assumed",
            "--entry",
            "assumed_caller",
        ],
        &["-Cpanic=abort"],
    );
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    for (name, expected) in [
        ("ignored", ProofStatus::ProvedWithAssumptions),
        ("stale", ProofStatus::Unknown),
        ("assumed", ProofStatus::Refuted),
        ("assumed_caller", ProofStatus::ProvedWithAssumptions),
    ] {
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, expected, "{name}: {:?}", proof.obligations);
    }
    spec["functions"][0]["modifies"] = serde_json::json!(["writer"]);
    spec["functions"][0]["ensures"] = serde_json::json!(["result == 0"]);
    std::fs::write(&config, serde_json::to_vec(&spec).unwrap()).unwrap();
    let output = analyze_from(
        &fixture("external_contracts.rs"),
        &directory,
        &[
            "--verify",
            "--allow-assumptions",
            "--contracts",
            config.to_str().unwrap(),
            "--entry",
            "ignored",
        ],
        &["-Cpanic=abort"],
    );
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report
            .functions
            .iter()
            .find(|f| f.name == "ignored")
            .unwrap()
            .proof
            .as_ref()
            .unwrap()
            .status,
        ProofStatus::Unknown
    );
}

#[test]
fn external_contracts_reject_stale_selectors_unsupported_aliases_and_inconsistent_summaries() {
    let directory = Directory::new();
    let config = directory.0.join("contracts.json");
    for (function, entry, instance, ensures, expected) in [
        (
            "generic",
            "generic_call",
            None,
            Vec::new(),
            ProofStatus::Unknown,
        ),
        (
            "generic",
            "generic_call",
            Some("[u8]"),
            vec!["result == value"],
            ProofStatus::ProvedWithAssumptions,
        ),
        (
            "reference",
            "reference_call",
            None,
            Vec::new(),
            ProofStatus::Unknown,
        ),
        (
            "assumed",
            "assumed_caller",
            None,
            vec!["result != result"],
            ProofStatus::Unknown,
        ),
    ] {
        let spec = serde_json::json!({"schema_version":1,"functions":[{
            "function":format!("external_contracts::{function}"), "instance":instance,
            "arguments":["value"],"trusted":true,"no_panic":true,"modifies":[],
            "ensures":ensures,"reason":"Explicit regression boundary"
        }]});
        std::fs::write(&config, serde_json::to_vec(&spec).unwrap()).unwrap();
        let output = analyze_from(
            &fixture("external_contracts.rs"),
            &directory,
            &[
                "--verify",
                "--allow-assumptions",
                "--contracts",
                config.to_str().unwrap(),
                "--entry",
                entry,
            ],
            &[],
        );
        let report: Report = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report
                .functions
                .iter()
                .find(|f| f.name == entry)
                .unwrap()
                .proof
                .as_ref()
                .unwrap()
                .status,
            expected,
            "{function}: {}",
            mir_check::render(&report)
        );
        assert_eq!(
            output.status.success(),
            expected == ProofStatus::ProvedWithAssumptions
        );
    }
    let spec = serde_json::json!({"schema_version":1,"functions":[{
        "function":"external_contracts::missing", "no_panic":true
    }]});
    std::fs::write(&config, serde_json::to_vec(&spec).unwrap()).unwrap();
    let output = analyze_from(
        &fixture("external_contracts.rs"),
        &directory,
        &[
            "--verify",
            "--contracts",
            config.to_str().unwrap(),
            "--entry",
            "unrelated",
        ],
        &[],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("matched no definition"));
}

#[test]
fn malformed_contract_configuration_fails_before_compilation() {
    let directory = Directory::new();
    let config = directory.0.join("contracts.json");
    let valid = serde_json::json!({"schema_version":1,"functions":[{
        "function":"external_contracts::assumed", "arguments":["value"],
        "trusted":true,"no_panic":true,"modifies":[],"reason":"Test boundary"
    }]});
    for (field, value) in [
        ("reason", serde_json::json!(" ")),
        ("no_panic", serde_json::json!(false)),
        ("modifies", serde_json::json!(["typo"])),
        ("arguments", serde_json::json!(["result"])),
        ("function", serde_json::json!("external_contracts::*")),
        ("requires", serde_json::json!(["value <"])),
        ("misspelt_option", serde_json::json!(true)),
    ] {
        let mut invalid = valid.clone();
        invalid["functions"][0][field] = value;
        std::fs::write(&config, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let output = analyze_from(
            &fixture("external_contracts.rs"),
            &directory,
            &[
                "--verify",
                "--contracts",
                config.to_str().unwrap(),
                "--entry",
                "unrelated",
            ],
            &[],
        );
        assert!(!output.status.success(), "{field}");
        assert!(
            output.stdout.is_empty(),
            "{field}: report must not imply success"
        );
    }
    for invalid in [
        serde_json::json!({"schema_version":2,"functions":[]}),
        serde_json::json!({"schema_version":1,"functions":[
            valid["functions"][0].clone(), valid["functions"][0].clone()]}),
    ] {
        std::fs::write(&config, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(mir_check::ContractConfig::read(&config).is_err());
    }
}

#[test]
fn cargo_forwards_trusted_contracts_without_dependency_mir_and_stays_strict_by_default() {
    let directory = Directory::new();
    for name in ["consumer", "dependency"] {
        std::fs::create_dir(directory.0.join(name)).unwrap();
        let dependency = if name == "consumer" {
            "[dependencies]\ndependency={path='../dependency'}\n"
        } else {
            ""
        };
        std::fs::write(
            directory.0.join(name).join("Cargo.toml"),
            format!(
                "[package]\nname='{name}'\nversion='0.1.0'\nedition='2024'\n\
             [lib]\npath='lib.rs'\n[workspace]\n{dependency}"
            ),
        )
        .unwrap();
    }
    std::fs::write(
        directory.0.join("dependency/lib.rs"),
        "#![no_std]\n#![forbid(unsafe_code)]\npub fn increment(value: u8) -> u8 { value + 1 }",
    )
    .unwrap();
    std::fs::write(
        directory.0.join("consumer/lib.rs"),
        "#![no_std]\n#![forbid(unsafe_code)]\npub fn guarded(value: u8) -> u8 { \
         if value < 255 { dependency::increment(value) } else { 0 } }",
    )
    .unwrap();
    let config = directory.0.join("contracts.json");
    let spec = serde_json::json!({"schema_version":1,"functions":[{
        "function":"dependency::increment","arguments":["value"],"requires":["value < 255"],
        "ensures":["result > value"],"trusted":true,"no_panic":true,"modifies":[],
        "reason":"Test unavailable ordinary dependency body"
    }]});
    std::fs::write(&config, serde_json::to_vec(&spec).unwrap()).unwrap();
    for allow in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-mir-check"));
        command.current_dir(directory.0.join("consumer")).args([
            "--verify",
            "--summary",
            "--entry",
            "guarded",
            "--contracts",
            "../contracts.json",
            "--no-dependency-mir",
            "--lib",
            "--offline",
        ]);
        if allow {
            command.arg("--allow-assumptions");
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.success(),
            allow,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("PROVED_WITH_ASSUMPTIONS"));
        let console = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let reports = console
            .lines()
            .find_map(|line| line.strip_prefix("JSON reports: "))
            .unwrap();
        let path = std::fs::read_dir(reports)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let report: Report = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(report.matched_contracts, ["dependency::increment"]);
        assert_eq!(report.coverage.proved, 0);
        assert_eq!(report.coverage.proved_with_assumptions, 1);
    }
}

#[test]
fn jsonl_and_saved_report_commands_keep_machine_output_clean_and_preserve_full_proofs() {
    let directory = Directory::new();
    let raw = directory.0.join("results.jsonl");
    let output = analyze_from(
        &fixture("proofs.rs"),
        &directory,
        &[
            "--verify",
            "--jsonl",
            raw.to_str().unwrap(),
            "--entry",
            "off_by_one",
        ],
        &[],
    );
    // analyze_from supplies --json; incompatible output modes must fail before compiling.
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args([
            "--verify",
            "--jsonl",
            "-",
            "--color",
            "always",
            "--entry",
            "guarded",
            "--entry",
            "off_by_one",
            "--",
            "--crate-type=lib",
            "--edition=2024",
        ])
        .arg(fixture("proofs.rs"))
        .arg("--out-dir")
        .arg(&directory.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!output.stdout.contains(&27));
    let lines = String::from_utf8(output.stdout).unwrap();
    assert_eq!(lines.lines().count(), 1);
    let report: Report = serde_json::from_str(lines.trim()).unwrap();
    assert_eq!(report.coverage.proved, 1);
    assert_eq!(report.coverage.refuted, 1);
    assert!(
        report
            .functions
            .iter()
            .filter_map(|f| f.proof.as_ref())
            .flat_map(|p| &p.obligations)
            .any(|o| o.model.is_some())
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Run failed"));
    std::fs::write(&raw, lines).unwrap();
    for binary in [
        env!("CARGO_BIN_EXE_mir-check"),
        env!("CARGO_BIN_EXE_cargo-mir-check"),
    ] {
        let output = Command::new(binary)
            .args(["report", "--color", "never", "--quiet"])
            .arg(&raw)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Saved report results"));
        assert!(text.contains("REFUTED off_by_one"));
        assert!(text.find("REFUTED off_by_one").unwrap() < text.find("PROVED guarded").unwrap());
        assert!(!text.contains('\u{1b}'));
        assert!(output.stderr.is_empty());
    }
    let malformed = directory.0.join("malformed.jsonl");
    std::fs::write(&malformed, "{not a report}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .arg("report")
        .arg(&malformed)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("malformed.jsonl:1"));
}

#[test]
fn saved_reports_accept_old_schema_and_keep_trusted_results_distinct_in_color_and_exit_status() {
    let directory = Directory::new();
    let config = directory.0.join("contracts.json");
    let spec = serde_json::json!({"schema_version":1,"functions":[{
        "function":"external_contracts::assumed","arguments":["value"],"trusted":true,
        "no_panic":true,"modifies":[],"reason":"Explicit test summary"
    }]});
    std::fs::write(&config, serde_json::to_vec(&spec).unwrap()).unwrap();
    let output = analyze_from(
        &fixture("external_contracts.rs"),
        &directory,
        &[
            "--verify",
            "--contracts",
            config.to_str().unwrap(),
            "--entry",
            "assumed_caller",
        ],
        &[],
    );
    let file = directory.0.join("assumed.json");
    std::fs::write(&file, output.stdout).unwrap();
    for allow in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
        command
            .args(["report", "--verbose", "--color", "always", "--quiet"])
            .arg(&file);
        if allow {
            command.arg("--allow-assumptions");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.success(), allow);
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("\u{1b}[35mPROVED_WITH_ASSUMPTIONS\u{1b}[0m"));
        assert!(!text.contains("\u{1b}[32mPROVED_WITH_ASSUMPTIONS"));
        assert!(text.contains("Explicit test summary"));
    }
    let report = report(analyze_from(
        &fixture("bodies.rs"),
        &directory,
        &["--verify", "--entry", "identity"],
        &[],
    ));
    let mut legacy = serde_json::to_value(report).unwrap();
    legacy["schema_version"] = serde_json::json!(7);
    legacy.as_object_mut().unwrap().remove("contract_config");
    legacy.as_object_mut().unwrap().remove("matched_contracts");
    legacy["coverage"]["proved"] = serde_json::json!(999); // Recompute untrusted cached counts.
    std::fs::write(&file, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args(["report", "--quiet"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 selected root: 1 proved"));
    legacy["schema_version"] = serde_json::json!(999);
    std::fs::write(&file, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args(["report", "--quiet"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn cargo_jsonl_stdout_excludes_compiler_messages_and_quiet_suppresses_progress() {
    let directory = Directory::new();
    std::fs::write(
        directory.0.join("Cargo.toml"),
        "[package]\nname='jsonl_fixture'\nversion='0.1.0'\nedition='2024'\n\
         [lib]\npath='lib.rs'\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(
        directory.0.join("lib.rs"),
        "#![no_std]\npub fn identity(value: u8) -> u8 { value }",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-mir-check"))
        .args([
            "--verify",
            "--entry",
            "identity",
            "--jsonl",
            "-",
            "--quiet",
            "--color",
            "always",
            "--message-format=json",
            "--lib",
            "--offline",
        ])
        .current_dir(&directory.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let raw = String::from_utf8(output.stdout).unwrap();
    assert_eq!(raw.lines().count(), 1);
    assert!(!raw.contains('\u{1b}'));
    let report: Report = serde_json::from_str(raw.trim()).unwrap();
    assert_eq!(report.coverage.proved, 1);
    let human = String::from_utf8(output.stderr).unwrap();
    assert!(human.contains("Verification passed"));
    assert!(!human.contains("mir-check: Analyzing"));
    assert!(!human.contains("mir-check: Reading"));
}

#[test]
fn long_solver_queries_show_the_active_root_and_elapsed_time_before_finishing() {
    use std::os::unix::fs::PermissionsExt;
    let directory = Directory::new();
    let solver = directory.0.join("slow-solver");
    std::fs::write(&solver, "#!/bin/sh\nsleep 7\n").unwrap();
    std::fs::set_permissions(&solver, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args([
            "--verify",
            "--entry",
            "identity",
            "--",
            "--crate-type=lib",
            "--edition=2024",
        ])
        .arg(fixture("bodies.rs"))
        .arg("--out-dir")
        .arg(&directory.0)
        .env("MIR_CHECK_Z3", &solver)
        .env_remove("MIR_CHECK_QUIET")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let progress = String::from_utf8(output.stderr).unwrap();
    assert!(progress.contains("checking [1/1] identity"), "{progress}");
    assert!(progress.contains("elapsed"), "{progress}");
    assert!(progress.contains("Analyzed bodies"), "{progress}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("UNKNOWN identity"));
}

#[test]
fn rechecking_saved_arguments_checks_current_source_and_can_select_a_binary_main_or_whole_crate() {
    let directory = Directory::new();
    let source = directory.0.join("application.rs");
    std::fs::write(
        &source,
        "pub fn read(bytes: [u8; 4], index: usize) -> u8 { bytes[index] }\n\
         fn main() { let _ = read([1, 2, 3, 4], 2); }\n",
    )
    .unwrap();
    let inventory = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args(["--json", "--", "--edition=2024"])
        .arg(&source)
        .arg("--out-dir")
        .arg(&directory.0)
        .output()
        .unwrap();
    assert!(inventory.status.success());
    let config = directory.0.join("invocation.json");
    std::fs::write(&config, inventory.stdout).unwrap();
    let recheck = |entries: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
        command
            .args(["--json", "--verify", "--from-report"])
            .arg(&config);
        for entry in entries {
            command.args(["--entry", entry]);
        }
        command.output().unwrap()
    };
    let output = recheck(&[]);
    assert!(!output.status.success());
    let whole: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(whole.coverage.selected_roots, 2);
    assert_eq!(whole.coverage.proved, 1);
    assert_eq!(whole.coverage.refuted, 1);
    let output = recheck(&["main"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let selected: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(selected.coverage.selected_roots, 1);
    assert_eq!(selected.coverage.proved, 1);
    assert_eq!(selected.coverage.unselected_bodies, 1);
    // Reuse a saved successful proof as input, then break its actual source.
    std::fs::write(&config, output.stdout).unwrap();
    std::fs::write(
        &source,
        std::fs::read_to_string(&source)
            .unwrap()
            .replace("read([1, 2, 3, 4], 2)", "read([1, 2, 3, 4], 4)"),
    )
    .unwrap();
    let output = recheck(&["main"]);
    assert!(!output.status.success());
    let mutated: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(mutated.coverage.refuted, 1);
}

#[test]
fn rechecking_rejects_invalid_invocations_and_verification_requires_actual_mir_analysis() {
    let directory = Directory::new();
    let original = report(analyze(&fixture("bodies.rs"), &directory, &[]));
    let original = serde_json::to_value(original).unwrap();
    let file = directory.0.join("invocation.json");
    for (field, value) in [
        ("schema_version", serde_json::json!(999)),
        ("compiler", serde_json::json!("another compiler")),
        ("rustc_arguments", serde_json::json!([])),
    ] {
        let mut invalid = original.clone();
        invalid[field] = value;
        std::fs::write(&file, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
            .args(["--verify", "--json", "--from-report"])
            .arg(&file)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{field}");
        assert!(output.stdout.is_empty(), "{field}");
    }
    std::fs::write(&file, serde_json::to_vec(&original).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args(["--verify", "--from-report"])
        .arg(&file)
        .args(["--", "--version"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args(["--verify", "--", "--version"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("without MIR analysis"));
    let mut legacy = original;
    legacy["schema_version"] = serde_json::json!(7);
    legacy["rustc_arguments"].as_array_mut().unwrap().extend([
        serde_json::json!("--error-format=json"),
        serde_json::json!("--json=artifacts"),
    ]);
    std::fs::write(&file, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args(["--verify", "--json", "--entry", "identity", "--from-report"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(output.status.success());
    let refreshed: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        refreshed
            .rustc_arguments
            .iter()
            .any(|arg| arg == "--error-format=human")
    );
    assert!(
        !refreshed
            .rustc_arguments
            .iter()
            .any(|arg| arg.starts_with("--json="))
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("$message_type"));
}

#[test]
fn main_selection_does_not_treat_coroutine_construction_as_execution_and_lists_macro_related_names()
{
    let directory = Directory::new();
    let source = directory.0.join("application.rs");
    std::fs::write(
        &source,
        "async fn task() { panic!(\"inside future\"); }\nfn main() { let _future = task(); }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args([
            "--verify",
            "--json",
            "--entry",
            "main",
            "--",
            "--edition=2024",
        ])
        .arg(&source)
        .arg("--out-dir")
        .arg(&directory.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.coverage.unknown, 1);
    std::fs::write(&source, "pub fn generated_main() {}\n").unwrap();
    let output = analyze_from(&source, &directory, &["--verify", "--entry", "main"], &[]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("main-related MIR names: generated_main")
    );
}
