#![forbid(unsafe_code)]

use miren::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/inductive_storage.rs")
}

#[test]
fn typed_allocations_preserve_aliases_and_snapshots_without_hiding_mutations() {
    let expected = [
        ("persistent_struct_storage", ProofStatus::Proved),
        ("a_shared_root_borrow", ProofStatus::Proved),
        ("writes_to_a_root_borrow", ProofStatus::Proved),
        (
            "entry_snapshots_are_separate_from_mutable_storage",
            ProofStatus::Proved,
        ),
        ("a_bad_write_through_a_callee", ProofStatus::Unknown),
        ("an_off_by_one_root_write", ProofStatus::Unknown),
        (
            "a_mutation_cannot_change_the_entry_snapshot",
            ProofStatus::Unknown,
        ),
        ("interior_storage_is_not_assumed", ProofStatus::Unknown),
        ("a_changing_reference_target", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
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
            assert_eq!(
                proof.invariants.len(),
                usize::from(status == ProofStatus::Proved)
            );
            assert!(proof.trusted_calls.is_empty());
            if entry == "a_changing_reference_target" {
                assert!(proof.obligations[0].detail.contains("identity changed"));
            }
            if entry == "interior_storage_is_not_assumed" {
                assert!(proof.obligations[0].detail.contains("interior mutation"));
            }
        }
    }
}

#[test]
fn native_replay_confirms_storage_and_snapshot_mutations() {
    let directory =
        std::env::temp_dir().join(format!("miren-storage-replay-{}", std::process::id()));
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
