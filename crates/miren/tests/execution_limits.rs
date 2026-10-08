#![forbid(unsafe_code)]

use miren::{ProofStatus, Report};
use std::path::Path;
use std::process::Command;

#[test]
fn larger_budgets_finish_bounded_work_and_never_hide_late_failures_or_unfinished_paths() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/execution_limits.rs");
    let expected = [
        ("completed_batches", ProofStatus::Proved),
        ("wrong_final_batch", ProofStatus::Refuted),
        ("larger_completed_batches", ProofStatus::Proved),
        ("larger_late_failure", ProofStatus::Refuted),
        ("unfinished_batches", ProofStatus::Unknown),
        ("bounded_recursion", ProofStatus::Proved),
        ("wrong_recursive_result", ProofStatus::Refuted),
        ("unfinished_recursion", ProofStatus::Unknown),
        ("different_instances", ProofStatus::Proved),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
        command.args(["--verify", "--json", "--quiet"]);
        for (entry, _) in expected {
            command.args(["--entry", entry]);
        }
        command.args(["--", "--crate-type=lib", "--edition=2024"]);
        command
            .arg(&fixture)
            .args(["-Coverflow-checks=yes", "-Cpanic=abort"]);
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
            assert_eq!(proof.status, status, "{target:?} {entry}");
            if status == ProofStatus::Refuted {
                assert!(proof.obligations.iter().any(|o| o.model.is_some()));
            }
            if entry == "completed_batches" {
                assert!(
                    proof
                        .obligations
                        .iter()
                        .all(|o| o.query.as_ref().is_none_or(|query| query.len() < 4096))
                );
            }
        }
    }
}
