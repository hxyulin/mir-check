#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/inductive_iterators.rs")
}

#[test]
fn integer_ranges_tags_and_actual_custom_iterators_preserve_failure_paths() {
    let expected = [
        ("a_range_can_fill_borrowed_storage", ProofStatus::Proved),
        ("a_signed_range", ProofStatus::Proved),
        ("a_maximum_endpoint", ProofStatus::Proved),
        ("an_empty_reversed_range", ProofStatus::Proved),
        ("a_range_body_can_break", ProofStatus::Proved),
        ("tagged_loop_state", ProofStatus::Proved),
        (
            "a_custom_iterator_uses_its_actual_body",
            ProofStatus::Proved,
        ),
        ("a_wrong_range_end", ProofStatus::Unknown),
        ("an_incorrect_tagged_loop", ProofStatus::Unknown),
        (
            "a_custom_iterator_is_not_the_core_range_model",
            ProofStatus::Unknown,
        ),
        ("a_late_range_panic", ProofStatus::Unknown),
        ("unsupported_slice_iterator_storage", ProofStatus::Unknown),
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
            if status == ProofStatus::Unknown {
                assert!(proof.invariants.is_empty());
            }
            if status == ProofStatus::Proved && entry != "an_empty_reversed_range" {
                assert_eq!(proof.invariants.len(), 1);
            }
            if entry == "a_custom_iterator_uses_its_actual_body" {
                assert!(
                    proof
                        .analyzed_bodies
                        .iter()
                        .any(|name| name.contains("next"))
                );
                assert!(
                    proof
                        .models
                        .iter()
                        .all(|model| !model.contains("integer Range next"))
                );
            }
            if entry == "a_range_can_fill_borrowed_storage" {
                assert!(
                    proof
                        .models
                        .iter()
                        .any(|model| model.contains("integer Range next"))
                );
            }
        }
    }
}

#[test]
fn native_range_cases_confirm_boundaries_breaks_and_mutations() {
    let directory =
        std::env::temp_dir().join(format!("mir-check-range-replay-{}", std::process::id()));
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("4 passed"));
    std::fs::remove_dir_all(directory).unwrap();
}
