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
            "mir-check-slice-equality-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/slice_equality.rs")
}

fn verify(path: &Path, entries: &[(&str, ProofStatus)], target: Option<&str>, optimized: bool) {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet"]);
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    command
        .args(["--", "--crate-type=lib", "--edition=2024"])
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
        entries
            .iter()
            .all(|(_, status)| *status == ProofStatus::Proved)
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
        assert!(proof.trusted_calls.is_empty());
        if *expected == ProofStatus::Refuted {
            assert!(proof.obligations.iter().any(|o| o.model.is_some()));
        }
    }
}

#[test]
fn slice_comparisons_preserve_numeric_guards_custom_calls_and_limits_on_host_and_arm() {
    let entries = [
        ("byte_guard", ProofStatus::Proved),
        ("byte_inequality", ProofStatus::Proved),
        ("prefixes", ProofStatus::Proved),
        ("different_tails", ProofStatus::Proved),
        ("unequal_lengths_skip_elements", ProofStatus::Proved),
        ("empty_slices_are_equal", ProofStatus::Proved),
        ("array_and_slice", ProofStatus::Proved),
        ("subviews", ProofStatus::Unknown),
        ("floats", ProofStatus::Proved),
        ("characters", ProofStatus::Proved),
        ("boolean_slices", ProofStatus::Proved),
        ("custom_effects", ProofStatus::Proved),
        ("custom_mismatch", ProofStatus::Proved),
        ("custom_unequal_lengths", ProofStatus::Proved),
        ("cross_type_order", ProofStatus::Proved),
        ("wrong_byte_guard", ProofStatus::Refuted),
        ("custom_panics", ProofStatus::Refuted),
        ("unbounded", ProofStatus::Unknown),
        ("over_budget", ProofStatus::Unknown),
        ("unsupported_comparison", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized);
        }
    }
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
fn broken_guards_and_equality_effects_refute_and_panic_natively() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    for (original, mutation, name) in [
        (
            "assert!(left[index] == right[index]);",
            "assert!(left[index] != right[index]);",
            "byte_guard",
        ),
        (
            "assert!(count.get() == 2);",
            "assert!(count.get() == 1);",
            "custom_effects",
        ),
        (
            "let right = [\n        Seen {\n            digit: 1,",
            "let right = [\n        Seen {\n            digit: 0,",
            "custom_mismatch",
        ),
    ] {
        assert!(source.contains(original));
        let directory = Directory::new();
        let path = directory.0.join("mutant.rs");
        std::fs::write(&path, source.replace(original, mutation)).unwrap();
        for target in [None, Some("thumbv7em-none-eabihf")] {
            verify(&path, &[(name, ProofStatus::Refuted)], target, false);
        }
        assert!(
            !native_tests(&path, &directory),
            "the mutation must panic natively"
        );
    }
}

#[test]
fn slice_guards_and_custom_inequality_replay_natively() {
    assert!(native_tests(&fixture(), &Directory::new()));
}
