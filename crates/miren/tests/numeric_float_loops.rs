#![forbid(unsafe_code)]

use miren::{ProofStatus, Report};
use std::path::Path;
use std::process::Command;

#[test]
fn numeric_only_float_work_does_not_exhaust_the_query_budget_or_discard_index_guards() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/numeric_float_loops.rs");
    let expected = [
        ("repeated_numeric_work", ProofStatus::Proved),
        ("numeric_work_keeps_its_guard", ProofStatus::Proved),
        ("numeric_work_with_bad_index", ProofStatus::Refuted),
        ("unsupported_numeric_work", ProofStatus::Unknown),
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
                .find(|function| function.name == entry)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, status,
                "{target:?} {entry}: {:?}",
                proof.obligations
            );
            if status == ProofStatus::Refuted {
                assert!(
                    proof
                        .obligations
                        .iter()
                        .any(|obligation| obligation.model.is_some())
                );
            }
        }
    }
}
