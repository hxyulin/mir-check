#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/inductive_calls.rs")
}

#[test]
fn actual_callee_transitions_preserve_domains_returns_snapshots_and_failure_paths() {
    let expected = [
        ("cycling_through_a_helper", ProofStatus::Proved),
        ("a_delegating_entry", ProofStatus::Proved),
        ("checked_call_domains", ProofStatus::Proved),
        ("caller_state_survives_nested_calls", ProofStatus::Proved),
        (
            "owned_byte_buffers_keep_their_entry_snapshots",
            ProofStatus::Proved,
        ),
        ("count_down", ProofStatus::Unknown),
        ("count_up", ProofStatus::Proved),
        ("a_loop_inside_a_callee", ProofStatus::Proved),
        ("a_helper_with_a_bad_wrap", ProofStatus::Unknown),
        ("a_broken_call_domain", ProofStatus::Unknown),
        (
            "a_false_contract_cannot_hide_the_body",
            ProofStatus::Unknown,
        ),
        ("a_false_root_postcondition", ProofStatus::Unknown),
        ("an_unsupported_postcondition", ProofStatus::Unknown),
        (
            "an_unsupported_contract_on_an_endless_function",
            ProofStatus::Unknown,
        ),
        ("a_recursive_callee_remains_unknown", ProofStatus::Unknown),
        (
            "an_unsupported_callee_remains_unknown",
            ProofStatus::Unknown,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
        command.args(["--verify", "--induction", "--json", "--quiet"]);
        for (entry, _) in expected {
            command.args(["--entry", entry]);
        }
        command
            .args(["--", "--crate-type=lib", "--edition=2024"])
            .arg(fixture())
            .args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
        if let Some(target) = target {
            command.args(["--target", target]);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        let report: Report = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
        for (entry, status) in expected {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == entry)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, status,
                "{target:?} {entry}: {:?}",
                proof.obligations
            );
            assert!(proof.trusted_calls.is_empty());
            if status == ProofStatus::Proved {
                assert_eq!(proof.invariants.len(), 1);
                let query = proof.obligations[0].query.as_ref().unwrap();
                assert!(query.starts_with("(set-logic HORN)"));
                assert!(!query.contains(":fp.xform.bit_blast true"));
                if entry != "count_up" {
                    assert!(proof.analyzed_bodies.len() > 1);
                }
                if entry == "caller_state_survives_nested_calls" {
                    assert!(proof.analyzed_bodies.iter().any(|name| name.contains("u8")));
                    assert!(
                        proof
                            .analyzed_bodies
                            .iter()
                            .any(|name| name.contains("u16"))
                    );
                }
            } else {
                assert!(proof.invariants.is_empty());
            }
        }
    }
}

#[test]
fn native_cases_exercise_actual_mutations_and_the_claims_of_false_contracts() {
    let directory =
        std::env::temp_dir().join(format!("mir-check-call-replay-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("replay");
    let output = Command::new("rustc")
        .args(["--test", "--edition=2024"])
        .arg(fixture())
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("3 passed"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn existing_finite_scalar_loops_can_use_induction_without_an_iteration_budget() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/execution_limits.rs");
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args([
            "--verify",
            "--induction",
            "--quiet",
            "--json",
            "--entry",
            "completed_batches",
            "--entry",
            "larger_completed_batches",
            "--",
            "--crate-type=lib",
            "--edition=2024",
        ])
        .arg(source)
        .args(["-Cpanic=abort", "-Coverflow-checks=yes"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    let proofs = report
        .functions
        .iter()
        .filter_map(|f| f.proof.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(proofs.len(), 2);
    for proof in proofs {
        assert_eq!(proof.status, ProofStatus::Proved);
        assert_eq!(proof.invariants.len(), 1);
        assert_eq!(proof.obligations.len(), 1);
        assert!(
            proof.obligations[0]
                .query
                .as_ref()
                .unwrap()
                .starts_with("(set-logic HORN)")
        );
    }
}

#[test]
fn retained_dependency_mir_and_its_contracts_are_part_of_the_inductive_proof() {
    let directory =
        std::env::temp_dir().join(format!("mir-check-call-dependency-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("root.rs");
    std::fs::write(
        &source,
        "#![no_std]\n#![forbid(unsafe_code)]\n\
        pub fn delegated(seed: u8) -> ! { loop {\n\
        assert!(induction_dependency::count_up(seed & 31) == (seed & 31));\n} }\n\
        pub fn misleading() -> ! { loop {\n\
        assert!(induction_dependency::a_false_root_postcondition(4) < 4);\n} }\n",
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let library = directory.join("libinduction_dependency.rlib");
        let mut compile = Command::new("rustc");
        compile
            .args([
                "--crate-name",
                "induction_dependency",
                "--crate-type=rlib",
                "--edition=2024",
                "-Zalways-encode-mir=yes",
                "-Zmir-opt-level=0",
                "-Cpanic=abort",
                "-Coverflow-checks=yes",
            ])
            .arg(fixture())
            .arg("-o")
            .arg(&library);
        if let Some(target) = target {
            compile.args(["--target", target]);
        }
        let output = compile.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
        command
            .args([
                "--verify",
                "--induction",
                "--json",
                "--quiet",
                "--",
                "--crate-type=lib",
                "--edition=2024",
            ])
            .arg(&source)
            .args(["-Cpanic=abort", "-Coverflow-checks=yes", "--extern"])
            .arg(format!("induction_dependency={}", library.display()));
        if let Some(target) = target {
            command.args(["--target", target]);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        let report: Report = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
        for (entry, status) in [
            ("delegated", ProofStatus::Proved),
            ("misleading", ProofStatus::Unknown),
        ] {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == entry)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, status,
                "{target:?} {entry}: {:?}",
                proof.obligations
            );
            assert!(
                proof
                    .analyzed_bodies
                    .iter()
                    .any(|name| name.contains("induction_dependency"))
            );
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn acyclic_calls_and_panic_paths_do_not_force_induction() {
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args([
            "--verify",
            "--induction",
            "--quiet",
            "--json",
            "--entry",
            "a_pure_acyclic_entry",
            "--",
            "--crate-type=lib",
            "--edition=2024",
        ])
        .arg(fixture())
        .args(["-Cpanic=abort", "-Coverflow-checks=yes"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "a_pure_acyclic_entry")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Proved);
    assert!(proof.invariants.is_empty());
    assert!(
        proof
            .obligations
            .iter()
            .filter_map(|o| o.query.as_ref())
            .all(|query| query.starts_with("(set-logic ALL)"))
    );
}
