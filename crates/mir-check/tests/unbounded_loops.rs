#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/unbounded_loops.rs")
}

fn check(target: Option<&str>, induction: bool, entries: &[&str]) -> Report {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet"]);
    if induction {
        command.arg("--induction");
    }
    for entry in entries {
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
    let report: Report = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
    let passing = report
        .functions
        .iter()
        .filter_map(|function| function.proof.as_ref())
        .all(|proof| {
            matches!(
                proof.status,
                ProofStatus::Proved | ProofStatus::ProvedWithAssumptions
            )
        });
    assert_eq!(output.status.success(), passing);
    report
}

#[test]
fn inductive_models_cover_endless_loops_arrays_and_exits_without_hiding_mutations() {
    let expected = [
        ("rotating_slots", ProofStatus::Proved),
        ("guarded_cycle", ProofStatus::Proved),
        ("sampled_registers", ProofStatus::Proved),
        ("a_constrained_register", ProofStatus::Proved),
        ("a_loop_that_can_exit", ProofStatus::Proved),
        ("a_broken_mask", ProofStatus::Unknown),
        ("a_bad_history_cursor", ProofStatus::Unknown),
        ("a_late_bad_wrap", ProofStatus::Unknown),
        ("an_unresolved_loop", ProofStatus::Unknown),
        ("an_inconsistent_loop_domain", ProofStatus::Unknown),
        ("a_loop_with_a_scalar_helper", ProofStatus::Proved),
        ("a_loop_with_a_postcondition", ProofStatus::Unknown),
    ];
    let entries = expected.iter().map(|(name, _)| *name).collect::<Vec<_>>();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let report = check(target, true, &entries);
        for (entry, expected) in expected {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == entry)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {entry}: {:?}",
                proof.obligations
            );
            if expected == ProofStatus::Proved {
                assert_eq!(proof.invariants.len(), 1);
                assert!(!proof.invariants[0].contains("(error"));
                let queries = proof
                    .obligations
                    .iter()
                    .filter_map(|o| o.query.as_ref())
                    .collect::<Vec<_>>();
                assert_eq!(queries.len(), 1);
                assert!(queries[0].starts_with("(set-logic HORN)"));
                assert!(queries[0].len() < 200_000);
                if entry == "a_constrained_register" {
                    assert_eq!(proof.assumptions, ["seed <= 511"]);
                } else {
                    assert!(proof.assumptions.is_empty());
                }
                assert!(proof.trusted_calls.is_empty());
                assert!(proof.models.is_empty());
            } else {
                assert!(proof.invariants.is_empty());
            }
        }
    }
    let bounded = check(None, false, &["rotating_slots", "a_late_bad_wrap"]);
    assert!(
        bounded
            .functions
            .iter()
            .filter_map(|f| f.proof.as_ref())
            .all(|p| p.status == ProofStatus::Unknown)
    );
}

#[test]
fn native_replay_exposes_broken_bounds_and_preserves_guarded_inputs() {
    let directory =
        std::env::temp_dir().join(format!("mir-check-loop-replay-{}", std::process::id()));
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
fn cargo_forwards_induction_and_clears_an_inherited_setting() {
    let directory =
        std::env::temp_dir().join(format!("mir-check-loop-cargo-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("src")).unwrap();
    std::fs::write(
        directory.join("Cargo.toml"),
        "[package]\nname = \"loop-fixture\"\nversion = \"0.1.0\"\n\
         edition = \"2024\"\n[workspace]\n",
    )
    .unwrap();
    std::fs::copy(fixture(), directory.join("src/lib.rs")).unwrap();
    for induction in [false, true] {
        let path = directory.join(if induction {
            "inductive.jsonl"
        } else {
            "bounded.jsonl"
        });
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-mir-check"));
        command.args([
            "--verify",
            "--quiet",
            "--entry",
            "rotating_slots",
            "--jsonl",
        ]);
        command.arg(&path);
        if induction {
            command.arg("--induction");
        }
        let output = command
            .args(["--manifest-path"])
            .arg(directory.join("Cargo.toml"))
            .env("MIR_CHECK_INDUCTION", "1")
            .output()
            .unwrap();
        assert_eq!(output.status.success(), induction, "{output:?}");
        let report: Report = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == "rotating_slots")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(
            proof.status,
            if induction {
                ProofStatus::Proved
            } else {
                ProofStatus::Unknown
            }
        );
        assert_eq!(proof.invariants.len(), usize::from(induction));
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn an_ordinary_binary_main_can_be_checked_without_running_it() {
    let directory =
        std::env::temp_dir().join(format!("mir-check-loop-main-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("main.rs");
    std::fs::write(
        &source,
        "#![forbid(unsafe_code)]\nfn main() {\n\
         let mut phase = false;\nloop { phase = !phase; if phase { assert!(phase); } }\n}\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args([
            "--verify",
            "--induction",
            "--quiet",
            "--json",
            "--entry",
            "main",
            "--",
            "--edition=2024",
        ])
        .arg(source)
        .args(["--out-dir"])
        .arg(&directory)
        .arg("-Cpanic=abort")
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
        .find(|f| f.name == "main")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Proved);
    assert_eq!(proof.invariants.len(), 1);
    std::fs::remove_dir_all(directory).unwrap();
}
