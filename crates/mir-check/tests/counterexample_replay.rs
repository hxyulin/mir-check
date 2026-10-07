#![forbid(unsafe_code)]

use mir_check::replay::ReplayStatus;
use mir_check::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mir-check-replay-tests-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/counterexample_replay.rs")
}

fn verify(source: &Path, entry: &str, replay: bool, target: Option<&str>) -> Report {
    verify_with_strategy(source, entry, replay, target, "abort")
}

fn verify_with_strategy(
    source: &Path,
    entry: &str,
    replay: bool,
    target: Option<&str>,
    panic_strategy: &str,
) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet", "--entry", entry]);
    if replay {
        command.arg("--replay");
    }
    command
        .args([
            "--",
            "--crate-name=counterexample_replay",
            "--crate-type=lib",
            "--edition=2024",
            "-Coverflow-checks=yes",
        ])
        .arg(format!("-Cpanic={panic_strategy}"))
        .arg(source)
        .arg("--out-dir")
        .arg(&directory.0);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: stdout {} stderr {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn check(report: &Report, entry: &str, expected: Option<ReplayStatus>) {
    let proof = report
        .functions
        .iter()
        .find(|function| function.name == entry)
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Refuted);
    let obligation = proof
        .obligations
        .iter()
        .find(|obligation| obligation.status == ProofStatus::Refuted)
        .unwrap();
    assert_eq!(
        obligation.replay.as_ref().map(|result| result.status),
        expected,
        "{entry}: {:?}",
        obligation.replay
    );
    assert!(["abort", "unwind"].contains(&report.panic_strategy.as_str()));
    if let Some(result) = &obligation.replay {
        assert_eq!(result.panic_strategy, report.panic_strategy);
        assert_eq!(
            result.uncontrolled_abstractions,
            obligation.abstraction_reasons
        );
        if result.status == ReplayStatus::ConfirmedPanic {
            assert!(result.panic_source.is_some());
        }
    }
}

#[test]
fn replay_does_not_claim_to_control_spurious_failures_or_arithmetic_nan_choices() {
    for (entry, reason) in [
        (
            "weak_exchange_is_not_guaranteed_to_succeed",
            "weak compare-exchange permits spurious failure",
        ),
        (
            "arithmetic_nan_encoding_is_not_guaranteed",
            "arithmetic NaNs allow every payload and sign",
        ),
    ] {
        let report = verify(&fixture(), entry, true, None);
        let proof = report
            .functions
            .iter()
            .find(|function| function.name == entry)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, ProofStatus::Refuted);
        let obligation = proof
            .obligations
            .iter()
            .find(|obligation| obligation.status == ProofStatus::Refuted)
            .unwrap();
        let replay = obligation.replay.as_ref().unwrap();
        assert!(matches!(
            replay.status,
            ReplayStatus::ConfirmedPanic | ReplayStatus::NotReproduced
        ));
        assert_eq!(
            replay.uncontrolled_abstractions,
            obligation.abstraction_reasons
        );
        assert!(
            replay
                .uncontrolled_abstractions
                .iter()
                .any(|choice| choice.contains(reason))
        );
        let display = mir_check::cli::render_report(&report, false, false);
        assert!(display.contains("query abstraction choices were not forced"));
        assert!(!mir_check::cli::accepted(&report, true));
    }
}

#[test]
fn native_model_inputs_confirm_panics_without_changing_proof_status() {
    for entry in [
        "checked_index",
        "signed_division",
        "signed_minimum_panic",
        "unsigned_maximum_panic",
        "boolean_panic",
        "panic_strategy_changes_behavior",
        "scalar_array",
        "occupied_atomic_claim",
    ] {
        let report = verify(&fixture(), entry, true, None);
        check(&report, entry, Some(ReplayStatus::ConfirmedPanic));
    }
    let report = verify(&fixture(), "guarded_index", true, None);
    let proof = report
        .functions
        .iter()
        .find(|function| function.name == "guarded_index")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Proved);
    assert!(
        proof
            .obligations
            .iter()
            .all(|obligation| obligation.replay.is_none())
    );
}

#[test]
fn native_replay_preserves_abort_configuration_and_can_also_catch_unwind_panics() {
    let report = verify_with_strategy(&fixture(), "checked_index", true, None, "unwind");
    check(&report, "checked_index", Some(ReplayStatus::ConfirmedPanic));
    assert_eq!(report.panic_strategy, "unwind");
    let report = verify_with_strategy(
        &fixture(),
        "panic_strategy_changes_behavior",
        true,
        None,
        "unwind",
    );
    let proof = report
        .functions
        .iter()
        .find(|function| function.name == "panic_strategy_changes_behavior")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Proved);
    assert!(
        proof
            .obligations
            .iter()
            .all(|obligation| obligation.replay.is_none())
    );
}

#[test]
fn atomic_abstraction_failure_is_not_reproduced_and_mutation_confirms_a_panic() {
    let report = verify(&fixture(), "fresh_atomic_claim", true, None);
    check(
        &report,
        "fresh_atomic_claim",
        Some(ReplayStatus::NotReproduced),
    );
    check(
        &verify(&fixture(), "fresh_claim_returns_owned_value", true, None),
        "fresh_claim_returns_owned_value",
        Some(ReplayStatus::NotReproduced),
    );
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "fresh_atomic_claim")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    let failure = proof
        .obligations
        .iter()
        .find(|o| o.status == ProofStatus::Refuted)
        .unwrap();
    assert!(
        failure
            .abstraction_reasons
            .iter()
            .any(|reason| { reason.contains("shared atomic reads allow arbitrary old values") })
    );
    assert_eq!(
        failure.replay.as_ref().unwrap().uncontrolled_abstractions,
        failure.abstraction_reasons
    );
    let display = mir_check::cli::render_report(&report, false, false);
    assert!(display.contains("root inputs only; query abstraction choices were not forced"));
    assert!(display.contains("A normal return does not validate every shared-state history"));
    let directory = Directory::new();
    let mutant = directory.0.join("mutant.rs");
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "static FRESH_STATE: AtomicU16 = AtomicU16::new(0);";
    assert!(source.contains(original));
    std::fs::write(
        &mutant,
        source.replacen(
            original,
            "static FRESH_STATE: AtomicU16 = AtomicU16::new(1);",
            1,
        ),
    )
    .unwrap();
    check(
        &verify(&mutant, "fresh_atomic_claim", true, None),
        "fresh_atomic_claim",
        Some(ReplayStatus::ConfirmedPanic),
    );
}

#[test]
fn replay_is_explicit_and_foreign_or_unsupported_inputs_are_not_executed() {
    check(
        &verify(&fixture(), "checked_index", false, None),
        "checked_index",
        None,
    );
    for entry in ["unsupported_reference_input", "private_root"] {
        check(
            &verify(&fixture(), entry, true, None),
            entry,
            Some(ReplayStatus::Unsupported),
        );
    }
    check(
        &verify(
            &fixture(),
            "checked_index",
            true,
            Some("thumbv7em-none-eabihf"),
        ),
        "checked_index",
        Some(ReplayStatus::Unsupported),
    );
}

#[test]
fn stale_sources_and_missing_input_assignments_remain_unsupported() {
    let directory = Directory::new();
    let source = directory.0.join("original.rs");
    std::fs::copy(fixture(), &source).unwrap();
    let mut report = verify(&source, "checked_index", false, None);
    std::fs::write(
        &source,
        "#![forbid(unsafe_code)]\npub fn checked_index(_: u8) -> u8 { 0 }\n",
    )
    .unwrap();
    mir_check::replay::replay_report(&mut report);
    check(&report, "checked_index", Some(ReplayStatus::Unsupported));
    let mut report = verify(&fixture(), "checked_index", false, None);
    for function in &mut report.functions {
        if let Some(proof) = &mut function.proof {
            for obligation in &mut proof.obligations {
                obligation.model = Some("()".to_owned());
            }
        }
    }
    mir_check::replay::replay_report(&mut report);
    check(&report, "checked_index", Some(ReplayStatus::Unsupported));
}

#[test]
fn nonpanic_exit_and_nontermination_never_confirm_the_abstract_panic() {
    for entry in ["nonpanic_exit", "nonterminating_after_claim"] {
        check(
            &verify(&fixture(), entry, true, None),
            entry,
            Some(ReplayStatus::ToolFailure),
        );
    }
}

#[test]
fn cargo_member_replay_preserves_the_original_directory_and_package_environment() {
    let directory = Directory::new();
    let member = directory.0.join("member");
    std::fs::create_dir_all(member.join("src")).unwrap();
    std::fs::write(
        directory.0.join("Cargo.toml"),
        "[workspace]\nmembers=['member']\nresolver='3'\n\
         [profile.dev]\npanic='abort'\noverflow-checks=true\n",
    )
    .unwrap();
    std::fs::write(
        member.join("Cargo.toml"),
        "[package]\nname='replay-member'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    std::fs::write(
        member.join("src/lib.rs"),
        "#![no_std]\n#![forbid(unsafe_code)]\n\
         const PACKAGE_LEN: usize = env!(\"CARGO_PKG_NAME\").len();\n\
         pub fn indexed(index: u8) -> u8 {\n\
             if PACKAGE_LEN == 13 { [3_u8, 5, 8][usize::from(index)] } else { 0 }\n\
         }\n",
    )
    .unwrap();
    let report_path = directory.0.join("report.jsonl");
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-mir-check"))
        .current_dir(&directory.0)
        .args([
            "--verify", "--replay", "--quiet", "--entry", "indexed", "--lib", "--jsonl",
        ])
        .arg(&report_path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let raw = std::fs::read_to_string(report_path).unwrap();
    let reports: Vec<Report> = raw
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let report = reports
        .iter()
        .find(|report| report.crate_name == "replay_member")
        .unwrap_or_else(|| {
            panic!(
                "missing report: {}",
                String::from_utf8_lossy(&output.stderr)
            )
        });
    check(report, "indexed", Some(ReplayStatus::ConfirmedPanic));
    let proof = report
        .functions
        .iter()
        .find(|function| function.name == "indexed")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    let recipe = proof.replay_inputs.as_ref().unwrap();
    assert_eq!(recipe.build_environment["CARGO_PKG_NAME"], "replay-member");
    assert_eq!(
        Path::new(recipe.working_directory.as_ref().unwrap()),
        std::fs::canonicalize(&directory.0).unwrap()
    );
    assert_eq!(
        std::fs::canonicalize(&recipe.build_environment["CARGO_MANIFEST_DIR"]).unwrap(),
        std::fs::canonicalize(&member).unwrap()
    );
    assert!(
        recipe
            .source_files
            .keys()
            .all(|file| Path::new(file).is_absolute())
    );
}
