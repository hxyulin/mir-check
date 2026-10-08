#![forbid(unsafe_code)]

use miren::{ObligationKind, ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("miren-pointer-cas-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/pointer_atomic_cas.rs")
}

fn verify(
    path: &Path,
    entries: &[(&str, ProofStatus)],
    target: Option<&str>,
    optimized: bool,
    startup: bool,
) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
    command.args(["--verify", "--json", "--quiet"]);
    if startup {
        command.args(["--startup", "--allow-assumptions"]);
    }
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    command
        .args([
            "--",
            "--crate-name=pointer_atomic_cas",
            "--crate-type=lib",
            "--edition=2024",
        ])
        .arg(path)
        .arg("--out-dir")
        .arg(&directory.0)
        .args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
    if optimized {
        command.arg("-Copt-level=2");
    }
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    let report: Report = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
    assert_eq!(
        output.status.success(),
        entries.iter().all(|(_, status)| matches!(
            status,
            ProofStatus::Proved | ProofStatus::ProvedWithAssumptions
        ))
    );
    for (name, expected) in entries {
        let proof = report
            .functions
            .iter()
            .find(|function| function.name == *name)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(
            proof.status, *expected,
            "{target:?} optimized={optimized} {name}: {:?}",
            proof.obligations
        );
        assert!(proof.trusted_calls.is_empty());
        assert_eq!(!proof.entry_assumptions.is_empty(), startup);
    }
    report
}

fn native_tests(path: &Path, directory: &Directory) -> bool {
    let executable = directory.0.join("native");
    let compiler = Path::new(env!("MIREN_SYSROOT")).join("bin/rustc");
    let output = Command::new(compiler)
        .args(["--test", "--edition=2024"])
        .arg(path)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    Command::new(executable).output().unwrap().status.success()
}

#[test]
fn pointer_cas_checks_address_relations_ordering_and_storage_without_inventing_history() {
    let entries = [
        (
            "pointer_publication_invalidates_owned_atomic_history",
            ProofStatus::Refuted,
        ),
        (
            "newly_created_atomics_after_pointer_publication_are_fresh",
            ProofStatus::Proved,
        ),
        (
            "strong_success_returns_the_expected_address",
            ProofStatus::Proved,
        ),
        (
            "weak_success_returns_the_expected_address",
            ProofStatus::Proved,
        ),
        ("guarded_failure_ordering", ProofStatus::Proved),
        (
            "matching_bits_preserve_only_the_observed_address",
            ProofStatus::Proved,
        ),
        (
            "weak_failure_may_return_the_expected_address",
            ProofStatus::Refuted,
        ),
        (
            "a_local_cas_does_not_assume_an_exclusive_history",
            ProofStatus::Refuted,
        ),
        ("release_failure_ordering_panics", ProofStatus::Refuted),
        (
            "matching_bits_do_not_authorize_a_pointee_read",
            ProofStatus::Unknown,
        ),
        (
            "uninitialized_pointer_storage_is_unknown",
            ProofStatus::Unknown,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            let report = verify(&fixture(), &entries, target, optimized, false);
            let proof = report
                .functions
                .iter()
                .find(|function| function.name == "release_failure_ordering_panics")
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert!(proof.obligations.iter().any(|obligation| {
                obligation.kind == ObligationKind::PanicSafety
                    && obligation.status == ProofStatus::Refuted
                    && obligation.detail.contains("non-release failure orderings")
            }));
        }
    }
    assert!(native_tests(&fixture(), &Directory::new()));
}

#[test]
fn startup_does_not_create_pointer_cas_history() {
    verify(
        &fixture(),
        &[
            (
                "pointer_publication_invalidates_other_startup_histories",
                ProofStatus::Refuted,
            ),
            (
                "weak_failure_may_return_the_expected_address",
                ProofStatus::Refuted,
            ),
            (
                "a_local_cas_does_not_assume_an_exclusive_history",
                ProofStatus::Refuted,
            ),
        ],
        None,
        false,
        true,
    );
}

#[test]
fn wrong_success_relations_and_release_failure_guards_fail_analysis_and_native_execution() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    for (original, mutation, name) in [
        (
            "Ok(old) => assert!(old == expected)",
            "Ok(old) => assert!(old != expected)",
            "strong_success_returns_the_expected_address",
        ),
        (
            "Ordering::Relaxed | Ordering::Acquire | Ordering::SeqCst",
            "Ordering::Relaxed | Ordering::Release | Ordering::SeqCst",
            "guarded_failure_ordering",
        ),
    ] {
        assert!(source.contains(original));
        let directory = Directory::new();
        let mutant = directory.0.join("mutant.rs");
        std::fs::write(&mutant, source.replace(original, mutation)).unwrap();
        verify(&mutant, &[(name, ProofStatus::Refuted)], None, false, false);
        assert!(!native_tests(&mutant, &directory));
    }
}
