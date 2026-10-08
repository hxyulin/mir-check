#![forbid(unsafe_code)]

use miren::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "miren-error-formatting-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/error_formatting.rs")
}

fn verify(
    path: &Path,
    entries: &[(&str, ProofStatus)],
    target: Option<&str>,
    optimized: bool,
    specification: Option<serde_json::Value>,
) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
    command.args(["--verify", "--json", "--quiet"]);
    if let Some(specification) = specification {
        let config = directory.0.join("contracts.json");
        std::fs::write(&config, serde_json::to_vec(&specification).unwrap()).unwrap();
        command.arg("--contracts").arg(config);
    }
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    command
        .args([
            "--",
            "--crate-name=error_formatting",
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
fn result_failure_paths_refute_and_opaque_debug_references_preserve_their_boundaries() {
    let entries = [
        ("guarded_unwrap", ProofStatus::Proved),
        ("guarded_expect", ProofStatus::Proved),
        ("error_payload", ProofStatus::Proved),
        (
            "coercing_and_passing_debug_references_does_not_call_the_formatter",
            ProofStatus::Proved,
        ),
        (
            "reference_return_preserves_caller_storage",
            ProofStatus::Proved,
        ),
        ("unchecked_unwrap", ProofStatus::Refuted),
        ("unchecked_expect", ProofStatus::Refuted),
        ("unwrap_err_on_success", ProofStatus::Refuted),
        ("expect_err_on_success", ProofStatus::Refuted),
        (
            "debug_constructor_does_not_hide_a_later_panic",
            ProofStatus::Refuted,
        ),
        ("dynamic_formatting_remains_unknown", ProofStatus::Unknown),
        ("display_coercions_remain_unknown", ProofStatus::Unknown),
        (
            "mutable_debug_coercions_remain_unknown",
            ProofStatus::Unknown,
        ),
        (
            "similarly_named_traits_are_not_core_debug",
            ProofStatus::Unknown,
        ),
        (
            "similarly_named_helpers_are_not_panic_boundaries",
            ProofStatus::Unknown,
        ),
        (
            "debug_erasure_cannot_hide_frame_owned_references",
            ProofStatus::Unknown,
        ),
        (
            "debug_erasure_does_not_restore_numeric_pointer_provenance",
            ProofStatus::Unknown,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            let report = verify(&fixture(), &entries, target, optimized, None);
            for function in report
                .functions
                .iter()
                .filter(|function| function.proof.is_some())
            {
                let proof = function.proof.as_ref().unwrap();
                assert!(proof.trusted_calls.is_empty());
            }
        }
    }
}

#[test]
fn widening_the_guard_reaches_the_panic_and_fails_native_replay() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let mutation = source.replace(
        "if value < 8 {\n        assert!",
        "if value < 9 {\n        assert!",
    );
    assert_ne!(source, mutation);
    let directory = Directory::new();
    let path = directory.0.join("mutant.rs");
    std::fs::write(&path, mutation).unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &path,
            &[
                ("guarded_unwrap", ProofStatus::Refuted),
                ("guarded_expect", ProofStatus::Refuted),
            ],
            target,
            false,
            None,
        );
    }
    assert!(!native_tests(&path, &directory));
}

#[test]
fn supported_results_and_reference_construction_replay_natively() {
    assert!(native_tests(&fixture(), &Directory::new()));
}

#[test]
fn induction_rejects_unsupported_debug_reference_state() {
    let directory = Directory::new();
    let output = Command::new(env!("CARGO_BIN_EXE_miren"))
        .args([
            "--verify",
            "--induction",
            "--json",
            "--quiet",
            "--entry",
            "indefinite_debug_loop",
            "--",
            "--crate-type=lib",
            "--edition=2024",
            "-Cpanic=abort",
        ])
        .arg(fixture())
        .arg("--out-dir")
        .arg(&directory.0)
        .output()
        .unwrap();
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    let proof = report
        .functions
        .iter()
        .find(|function| function.name == "indefinite_debug_loop")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(proof.status, ProofStatus::Unknown);
}
