#![forbid(unsafe_code)]

use miren::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/floating_intrinsics.rs")
}

const ENTRIES: &[(&str, ProofStatus)] = &[
    ("unary_binary32", ProofStatus::Proved),
    ("unary_binary64", ProofStatus::Proved),
    ("rounding_ties_and_directions", ProofStatus::Proved),
    ("rounding_preserves_zero_and_infinity", ProofStatus::Proved),
    ("square_root_edges", ProofStatus::Proved),
    ("fused_multiply_add_rounds_once", ProofStatus::Proved),
    ("returned_result_has_stable_storage", ProofStatus::Proved),
    ("floor_keeps_guarded_indices_in_range", ProofStatus::Proved),
    ("a_false_rounding_tie", ProofStatus::Refuted),
    ("a_false_square_root_domain", ProofStatus::Refuted),
    ("a_false_separate_rounding_claim", ProofStatus::Refuted),
    ("a_rounded_index_can_reach_the_end", ProofStatus::Refuted),
    (
        "an_intrinsic_name_does_not_replace_a_user_body",
        ProofStatus::Refuted,
    ),
    ("remainder_is_still_unsupported", ProofStatus::Unknown),
];

#[test]
fn exact_float_intrinsics_preserve_ieee_edges_and_reject_mutations_on_host_and_arm() {
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
        command.args(["--verify", "--json", "--quiet"]);
        for (name, _) in ENTRIES {
            command.args(["--entry", name]);
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
        for (name, expected) in ENTRIES {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == *name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, *expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
            assert!(proof.trusted_calls.is_empty());
            if name.starts_with("unary_binary") {
                assert_eq!(
                    proof
                        .models
                        .iter()
                        .filter(|m| m.contains("explicit rounding"))
                        .count(),
                    6,
                    "{target:?} {name}: {:?}",
                    proof.models,
                );
            }
        }
    }
}

#[test]
fn native_replay_checks_ieee_special_values_and_failing_mutations() {
    let directory = std::env::temp_dir().join(format!(
        "miren-float-intrinsic-replay-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("replay");
    let output = Command::new(Path::new(env!("MIREN_SYSROOT")).join("bin/rustc"))
        .args(["--test", "--edition=2024"])
        .arg(fixture())
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    std::fs::remove_dir_all(directory).unwrap();
}
