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
    let text = mir_check::render(&report);
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
            .args(["mir-check", "--lib", "--features", "extra", "--offline"])
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
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-check")).parent().unwrap();
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
        ("beyond_budget", ProofStatus::Unknown),
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
                "mutable_capture" => ProofStatus::Unknown,
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
