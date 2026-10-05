#![forbid(unsafe_code)]

use mir_checker::{ContractKind, ContractStatus, ProofStatus, Report, SiteKind, SiteStatus};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("mir-checker-{}-{id}", std::process::id()));
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
    Command::new(env!("CARGO_BIN_EXE_mir-checker"))
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
fn unresolved_calls_and_destructors_are_explicit_unknown_boundaries() {
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
fn guarded_accesses_remain_unverified_until_a_proof_engine_exists() {
    let directory = Directory::new();
    let report = report(analyze(&fixture("bodies.rs"), &directory, &[]));
    assert!(has_site(&report, "guarded", SiteKind::BoundsCheck));
    let text = mir_checker::render(&report);
    assert!(text.contains("inventory only; no proof"));
    assert!(!text.contains("PROVED"));
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
    assert!(mir_checker::render(&report).contains("feasibility unverified"));
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
    let output = Command::new(env!("CARGO_BIN_EXE_mir-checker"))
        .args(["rustc", "--crate-type=lib", "--edition=2024"])
        .arg(fixture("bodies.rs"))
        .arg("--out-dir")
        .arg(&directory.0)
        .env("MIR_CHECKER_REPORT_DIR", blocked)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mir-checker:"));
}

#[test]
fn malformed_contracts_fail_compilation_without_a_success_report() {
    let directory = Directory::new();
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-checker"))
        .parent()
        .unwrap();
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
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-checker"))
        .parent()
        .unwrap();
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
        assert!(mir_checker::render(&report).contains("verification: PROVED"));
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
    let output = Command::new(env!("CARGO_BIN_EXE_mir-checker"))
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
        .env("MIR_CHECKER_Z3", directory.0.join("missing_solver"))
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
    let output = Command::new(Path::new(env!("MIR_CHECKER_SYSROOT")).join("bin/rustc"))
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
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-mir-checker"))
            .args(["mir-checker", "--lib", "--features", "extra", "--offline"])
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
    let directory = Directory::new();
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-checker"))
        .parent()
        .unwrap();
    let library = find_contract_library(profile).unwrap();
    let external = format!("mir_contracts={}", library.display());
    let mut args = vec!["--extern", external.as_str(), "-Coverflow-checks=yes"];
    args.extend_from_slice(rustc_args);
    let output = analyze_from(
        &fixture("verified_contracts.rs"),
        &directory,
        &["--verify", "--entry", name],
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
                mir_checker::ObligationKind::CallPrecondition
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
                mir_checker::ObligationKind::CallPrecondition
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
                    mir_checker::ObligationKind::Postcondition
                ) && obligation.status
                    == ProofStatus::Refuted),
                "{name}"
            );
        }
        if name == "use_lying_postcondition" {
            assert!(proof.obligations.iter().any(|obligation| matches!(
                obligation.kind,
                mir_checker::ObligationKind::PanicSafety
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
fn unknown_memory_and_dispatch_boundaries_fail_verification() {
    for name in [
        "unwrap_option",
        "clamp",
        "dynamic",
        "generic",
        "indirect",
        "destructor",
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
        assert_eq!(proof.status, ProofStatus::Unknown, "{name}");
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
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-mir-checker"))
            .args(["mir-checker", "--verify", "--lib", "--offline"])
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
    let directory = Directory::new();
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-checker"))
        .parent()
        .unwrap();
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
                    mir_checker::ObligationKind::Postcondition
                )));
            }
            if function.name.ends_with("::data") {
                assert_eq!(proof.assumptions.len(), 1);
                assert!(proof.inputs.contains_key("self.len"));
            }
            if function.name.ends_with("round_trip") {
                assert!(proof.obligations.iter().any(|obligation| matches!(
                    obligation.kind,
                    mir_checker::ObligationKind::CallPrecondition
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
            mir_checker::ObligationKind::CallPrecondition
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
fn mutable_array_borrows_cannot_cross_an_unmodeled_call_boundary() {
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
    bytes[0]
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
    assert_eq!(proof.status, ProofStatus::Unknown);
    assert!(
        proof
            .obligations
            .iter()
            .any(|obligation| obligation.detail.contains("mutable"))
    );
}
