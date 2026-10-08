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
        let path = std::env::temp_dir().join(format!("miren-nonnull-{}-{id}", std::process::id()));
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/nonnull_handles.rs")
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
            "--crate-name=nonnull_handles",
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
fn thin_nonnull_conversions_prove_validity_without_granting_dereference_permission() {
    let entries = [
        (
            "checked_handles_preserve_the_observed_address",
            ProofStatus::Proved,
        ),
        (
            "a_nonnull_handle_does_not_require_pointee_alignment",
            ProofStatus::Proved,
        ),
        (
            "loaded_pointer_handles_keep_only_the_observed_address",
            ProofStatus::Proved,
        ),
        (
            "handles_of_caller_storage_preserve_their_address",
            ProofStatus::Proved,
        ),
        (
            "a_guard_establishes_a_nonnull_conversion",
            ProofStatus::Proved,
        ),
        (
            "a_nonnull_wrapper_can_return_its_checked_pointer_bits",
            ProofStatus::Proved,
        ),
        (
            "a_nonnull_handle_cannot_change_its_address",
            ProofStatus::Refuted,
        ),
        ("zero_is_not_a_valid_nonnull_handle", ProofStatus::Refuted),
        (
            "a_numeric_nonnull_handle_does_not_authorize_a_read",
            ProofStatus::Unknown,
        ),
        (
            "checked_nonnull_values_can_be_published",
            ProofStatus::Proved,
        ),
        (
            "compiler_range_valid_values_can_be_published",
            ProofStatus::Proved,
        ),
        (
            "published_pointer_payloads_remain_opaque",
            ProofStatus::Unknown,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            let report = verify(&fixture(), &entries, target, optimized, false);
            let proof = report
                .functions
                .iter()
                .find(|function| function.name == "zero_is_not_a_valid_nonnull_handle")
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert!(proof.obligations.iter().any(|obligation| {
                obligation.kind == ObligationKind::Validity
                    && obligation.status == ProofStatus::Refuted
                    && obligation.detail.contains("nonzero address")
            }));
        }
    }
    assert!(native_tests(&fixture(), &Directory::new()));
}

#[test]
fn allowing_zero_through_a_nonnull_guard_breaks_the_validity_proof() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "if address != 0 {";
    assert!(source.contains(original));
    let directory = Directory::new();
    let path = directory.0.join("mutant.rs");
    std::fs::write(&path, source.replace(original, "if address == 0 {")).unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(
                &path,
                &[(
                    "a_guard_establishes_a_nonnull_conversion",
                    ProofStatus::Refuted,
                )],
                target,
                optimized,
                false,
            );
        }
    }
}

#[test]
fn a_wrong_address_round_trip_is_refuted_and_panics_natively() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "assert!(handle.as_ptr() as usize == address);";
    assert!(source.contains(original));
    let directory = Directory::new();
    let path = directory.0.join("mutant.rs");
    std::fs::write(
        &path,
        source.replace(original, "assert!(handle.as_ptr() as usize != address);"),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(
                &path,
                &[(
                    "checked_handles_preserve_the_observed_address",
                    ProofStatus::Refuted,
                )],
                target,
                optimized,
                false,
            );
        }
    }
    assert!(!native_tests(&path, &directory));
}
