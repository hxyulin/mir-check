#![forbid(unsafe_code)]

use mir_check::{ObligationKind, ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mir-check-pointer-loads-{}-{id}",
            std::process::id()
        ));
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/pointer_atomic_loads.rs")
}

fn verify(
    path: &Path,
    entries: &[(&str, ProofStatus)],
    target: Option<&str>,
    optimized: bool,
    startup: bool,
) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
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
            "--crate-name=pointer_atomic_loads",
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
    let compiler = Path::new(env!("MIR_CHECK_SYSROOT")).join("bin/rustc");
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
fn shared_pointer_loads_check_ordering_and_preserve_only_observed_address_bits() {
    let entries = [
        (
            "one_observed_address_has_consistent_nullability",
            ProofStatus::Proved,
        ),
        (
            "a_nested_pointer_atomic_has_a_certified_storage_offset",
            ProofStatus::Proved,
        ),
        ("guarded_load_ordering", ProofStatus::Proved),
        ("release_load_ordering_panics", ProofStatus::Refuted),
        ("acquire_release_load_ordering_panics", ProofStatus::Refuted),
        ("unguarded_load_ordering_can_panic", ProofStatus::Refuted),
        (
            "an_initializer_does_not_establish_pointer_history",
            ProofStatus::Refuted,
        ),
        (
            "separate_reads_do_not_establish_a_stable_address",
            ProofStatus::Refuted,
        ),
        (
            "local_pointer_storage_supports_conservative_reads",
            ProofStatus::Proved,
        ),
        (
            "a_local_pointer_read_does_not_silently_assume_exclusivity",
            ProofStatus::Refuted,
        ),
        (
            "uninitialized_atomic_storage_is_not_a_pointer_receiver",
            ProofStatus::Unknown,
        ),
        (
            "numeric_addresses_do_not_authorize_atomic_loads",
            ProofStatus::Unknown,
        ),
        (
            "loaded_addresses_do_not_authorize_pointee_reads",
            ProofStatus::Unknown,
        ),
        (
            "a_pointer_atomic_receiver_cannot_bypass_retirement",
            ProofStatus::Unknown,
        ),
        (
            "user_load_methods_are_not_compiler_atomic_models",
            ProofStatus::Refuted,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            let report = verify(&fixture(), &entries, target, optimized, false);
            for name in [
                "release_load_ordering_panics",
                "acquire_release_load_ordering_panics",
            ] {
                let proof = report
                    .functions
                    .iter()
                    .find(|function| function.name == name)
                    .unwrap()
                    .proof
                    .as_ref()
                    .unwrap();
                assert!(proof.obligations.iter().any(|obligation| {
                    obligation.kind == ObligationKind::PanicSafety
                        && obligation.status == ProofStatus::Refuted
                        && obligation.detail.contains("non-release ordering")
                }));
            }
            let proof = report
                .functions
                .iter()
                .find(|function| {
                    function.name == "an_initializer_does_not_establish_pointer_history"
                })
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert!(proof.obligations.iter().any(|obligation| {
                obligation
                    .abstraction_reasons
                    .iter()
                    .any(|reason| reason.contains("pointer atomic reads allow arbitrary addresses"))
            }));
        }
    }
    assert!(native_tests(&fixture(), &Directory::new()));
}

#[test]
fn startup_does_not_silently_supply_pointer_provenance_or_history() {
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(
                &fixture(),
                &[
                    (
                        "one_observed_address_has_consistent_nullability",
                        ProofStatus::ProvedWithAssumptions,
                    ),
                    (
                        "an_initializer_does_not_establish_pointer_history",
                        ProofStatus::Refuted,
                    ),
                    (
                        "a_pointer_atomic_receiver_cannot_bypass_retirement",
                        ProofStatus::Unknown,
                    ),
                ],
                target,
                optimized,
                true,
            );
        }
    }
}

#[test]
fn admitting_a_release_order_into_the_guard_fails_analysis_and_native_execution() {
    let directory = Directory::new();
    let source = std::fs::read_to_string(fixture()).unwrap();
    let guarded = "Ordering::Relaxed | Ordering::Acquire | Ordering::SeqCst";
    assert!(source.contains(guarded));
    let mutant = directory.0.join("mutant.rs");
    std::fs::write(
        &mutant,
        source.replace(
            guarded,
            "Ordering::Relaxed | Ordering::Release | Ordering::SeqCst",
        ),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(
                &mutant,
                &[("guarded_load_ordering", ProofStatus::Refuted)],
                target,
                optimized,
                false,
            );
        }
    }
    assert!(!native_tests(&mutant, &directory));
}
