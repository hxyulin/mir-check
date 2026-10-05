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

fn find_contract_library(directory: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(directory).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(library) = find_contract_library(&path) {
                return Some(library);
            }
        } else if path
            .file_name()?
            .to_string_lossy()
            .starts_with("libmir_contracts-")
            && path
                .extension()
                .is_some_and(|extension| extension == "dylib" || extension == "so")
        {
            return Some(path);
        }
    }
    None
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
