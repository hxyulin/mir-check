#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/multiple_mutable_roots.rs")
}

fn verify(path: &Path, entries: &[(&str, ProofStatus)], target: Option<&str>, induction: bool) {
    let directory = std::env::temp_dir().join(format!(
        "mir-check-mutable-inputs-{}-{}-{}-{}",
        std::process::id(),
        target.unwrap_or("host"),
        induction,
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet"]);
    if induction {
        command.arg("--induction");
    }
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    command
        .args(["--", "--crate-type=lib", "--edition=2024"])
        .arg(path)
        .arg("--out-dir")
        .arg(&directory)
        .args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    assert!(!output.status.success());
    let report: Report = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
    for (name, expected) in entries {
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
            "{target:?} induction={induction} {name}: {:?}",
            proof.obligations,
        );
        assert!(proof.trusted_calls.is_empty());
        if *expected == ProofStatus::Refuted {
            assert!(
                proof
                    .obligations
                    .iter()
                    .any(|obligation| obligation.model.is_some())
            );
        }
        if induction && *name == "iterative_writes" {
            assert_eq!(proof.invariants.len(), 1);
        }
        if *name == "a_reference_bearing_pointee" {
            assert!(proof.obligations.iter().any(|obligation| {
                obligation
                    .detail
                    .contains("cannot contain reference fields")
            }));
        }
        if *name == "an_interior_pointee" || *name == "atomic_pointees_are_not_disjoint_snapshots" {
            assert!(proof.obligations.iter().any(|obligation| {
                obligation
                    .detail
                    .contains("cannot contain interior mutation")
            }));
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn independent_mutable_roots_preserve_allocations_snapshots_and_callee_contracts() {
    let entries = [
        ("distinct_scalar_storage", ProofStatus::Proved),
        ("entry_snapshots_are_independent", ProofStatus::Proved),
        ("projected_callee_mutations", ProofStatus::Proved),
        ("shared_snapshot_stays_separate", ProofStatus::Proved),
        ("byte_slices_keep_distinct_storage", ProofStatus::Proved),
        ("interleaved_bytes_and_scalars", ProofStatus::Proved),
        ("interleaved_scalars_and_bytes", ProofStatus::Proved),
        ("iterative_writes", ProofStatus::Proved),
        (
            "a_write_does_not_change_its_entry_snapshot",
            ProofStatus::Refuted,
        ),
        ("a_callee_precondition_is_checked", ProofStatus::Refuted),
        ("a_false_independence_claim", ProofStatus::Refuted),
        ("an_off_by_one_loop", ProofStatus::Refuted),
        ("a_reference_bearing_pointee", ProofStatus::Unknown),
        ("an_interior_pointee", ProofStatus::Unknown),
        (
            "atomic_pointees_are_not_disjoint_snapshots",
            ProofStatus::Unknown,
        ),
        ("a_shared_cell_before_mutable_storage", ProofStatus::Unknown),
        ("mutable_storage_before_a_shared_cell", ProofStatus::Unknown),
        ("non_byte_slices_remain_unsupported", ProofStatus::Unknown),
        ("unresolved_generic_pointees", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(&fixture(), &entries, target, false);
    }
}

#[test]
fn induction_keeps_multiple_root_allocations_separate() {
    let entries = [
        ("iterative_writes", ProofStatus::Proved),
        ("an_off_by_one_loop", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(&fixture(), &entries, target, true);
    }
}

#[test]
fn changing_a_callee_write_refutes_the_root() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "*right += 2;";
    assert_eq!(source.matches(original).count(), 1);
    let directory = std::env::temp_dir().join(format!(
        "mir-check-multiple-mutable-mutant-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("multiple_mutable_roots.rs");
    std::fs::write(&path, source.replace(original, "*right += 3;")).unwrap();
    verify(
        &path,
        &[("projected_callee_mutations", ProofStatus::Refuted)],
        None,
        false,
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn native_disjoint_borrows_and_failure_mutations_match_the_checker() {
    let directory = std::env::temp_dir().join(format!(
        "mir-check-multiple-mutable-replay-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("replay");
    let output = Command::new(Path::new(env!("MIR_CHECK_SYSROOT")).join("bin/rustc"))
        .args(["--test", "--edition=2024", "-Coverflow-checks=yes"])
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
