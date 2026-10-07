#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mir-check-atomic-fences-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/atomic_fences.rs")
}

fn verify(
    path: &Path,
    entries: &[(&str, ProofStatus)],
    target: Option<&str>,
    optimized: bool,
) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet"]);
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    command
        .args([
            "--",
            "--crate-name=atomic_fences",
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
            .find(|f| f.name == *name)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(
            proof.status, *expected,
            "{target:?} optimized={optimized} {name}: {:?}",
            proof.obligations
        );
        if *expected == ProofStatus::Proved {
            assert!(proof.trusted_calls.is_empty());
        }
        if *expected == ProofStatus::Refuted {
            assert!(proof.obligations.iter().any(|o| o.model.is_some()));
        }
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
fn fences_validate_orderings_without_creating_atomic_history_or_hiding_gaps() {
    let entries = [
        ("all_valid_fences", ProofStatus::Proved),
        ("synthetic::direct_intrinsic", ProofStatus::Proved),
        ("synthetic::invalid_intrinsic", ProofStatus::Unknown),
        ("guarded_fences", ProofStatus::Proved),
        ("fences_preserve_local_storage", ProofStatus::Proved),
        ("guarded_atomic_value", ProofStatus::Proved),
        ("dynamic_compiler_fence", ProofStatus::Refuted),
        ("dynamic_hardware_fence", ProofStatus::Refuted),
        ("relaxed_compiler_fence", ProofStatus::Refuted),
        ("relaxed_hardware_fence", ProofStatus::Refuted),
        (
            "fences_do_not_establish_atomic_history",
            ProofStatus::Refuted,
        ),
        ("unsupported_payload_after_fence", ProofStatus::Unknown),
        ("user_function_is_not_an_intrinsic", ProofStatus::Refuted),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            let report = verify(&fixture(), &entries, target, optimized);
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == "all_valid_fences")
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            for intrinsic in ["atomic_fence", "atomic_singlethreadfence"] {
                assert!(proof.models.iter().any(|model| {
                    model.starts_with(&format!("core::intrinsics::{intrinsic}:"))
                        && model.contains("no synchronization facts")
                }));
            }
        }
    }
    assert!(native_tests(&fixture(), &Directory::new()));
}

#[test]
fn relaxing_a_guarded_fence_or_changing_local_storage_never_proves() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    for (original, mutation, root) in [
        (
            "            compiler_fence(order);",
            "dynamic_compiler_fence(Ordering::Relaxed);",
            "guarded_fences",
        ),
        (
            "assert!(bytes[2] == 17);",
            "assert!(bytes[2] == 18);",
            "fences_preserve_local_storage",
        ),
    ] {
        assert!(source.contains(original));
        let directory = Directory::new();
        let path = directory.0.join("mutant.rs");
        std::fs::write(&path, source.replace(original, mutation)).unwrap();
        for target in [None, Some("thumbv7em-none-eabihf")] {
            verify(&path, &[(root, ProofStatus::Refuted)], target, false);
        }
        assert!(!native_tests(&path, &directory));
    }
}

#[test]
fn cas_preserves_result_relations_ordering_checks_and_spurious_failures() {
    let entries = [
        ("strong_cas", ProofStatus::Proved),
        ("weak_cas", ProofStatus::Proved),
        ("guarded_cas_ordering", ProofStatus::Proved),
        ("signed_cas", ProofStatus::Proved),
        ("weak_failure_can_match", ProofStatus::Refuted),
        ("bad_cas_success_claim", ProofStatus::Refuted),
        ("unchecked_cas_ordering", ProofStatus::Refuted),
        ("cas_history_remains_arbitrary", ProofStatus::Refuted),
        ("fresh_counter_claim", ProofStatus::Proved),
        ("occupied_counter_claim", ProofStatus::Refuted),
        ("pointer_cas_is_unknown", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            let report = verify(&fixture(), &entries, target, optimized);
            let occupied = report
                .functions
                .iter()
                .find(|function| function.name == "occupied_counter_claim")
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert!(occupied.obligations.iter().any(|failure| {
                failure.status == ProofStatus::Refuted
                    && failure.kind == mir_check::ObligationKind::PanicSafety
                    && failure.detail == "panic entry point is reachable"
            }));
        }
    }
    assert!(native_tests(&fixture(), &Directory::new()));
    let source = std::fs::read_to_string(fixture()).unwrap();
    let directory = Directory::new();
    let path = directory.0.join("mutant.rs");
    std::fs::write(
        &path,
        source.replace("assert!(old == expected)", "assert!(old != expected)"),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &path,
            &[("strong_cas", ProofStatus::Refuted)],
            target,
            false,
        );
    }
    assert!(!native_tests(&path, &directory));

    let path = directory.0.join("occupied.rs");
    let original = "let counter = core::sync::atomic::AtomicU16::new(0);";
    assert!(source.contains(original));
    std::fs::write(
        &path,
        source.replace(
            original,
            "let counter = core::sync::atomic::AtomicU16::new(1);",
        ),
    )
    .unwrap();
    assert!(!native_tests(&path, &directory));
}
