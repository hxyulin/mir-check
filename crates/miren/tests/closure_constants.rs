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
            "miren-closure-constants-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/closure_constants.rs")
}

fn verify(path: &Path, names: &[&str], target: Option<&str>) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
    command.args(["--verify", "--json"]);
    for name in names {
        command.args(["--entry", name]);
    }
    command.args([
        "--",
        "--crate-type=lib",
        "--edition=2024",
        "-Cpanic=abort",
        "-Coverflow-checks=yes",
    ]);
    command.arg(path).arg("--out-dir").arg(&directory.0);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn evaluated_closures_execute_real_callbacks_and_reject_captured_constant_environments() {
    let entries = [
        ("default_sum", ProofStatus::Proved),
        ("default_fold", ProofStatus::Proved),
        ("rejected_default_sum", ProofStatus::Refuted),
        ("folded_labels", ProofStatus::Proved),
        ("summed_labels", ProofStatus::Proved),
        ("summed_fractions", ProofStatus::Proved),
        ("folded_empty", ProofStatus::Proved),
        ("explicit_constant", ProofStatus::Proved),
        ("nested_constant", ProofStatus::Proved),
        ("rejected_sum", ProofStatus::Refuted),
        ("rejected_fold", ProofStatus::Refuted),
        ("rejected_constant_callback", ProofStatus::Refuted),
        ("captured_constant", ProofStatus::Unknown),
        ("captured_zero_sized_constant", ProofStatus::Unknown),
        ("mutable_environment", ProofStatus::Proved),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let report = verify(&fixture(), &names, target);
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
            if name.starts_with("captured_") {
                assert!(proof.obligations.iter().any(|obligation| {
                    obligation.detail.contains("constant closure with captures")
                }));
            }
        }
    }
}

#[test]
fn changing_a_fold_seed_refutes_the_unchanged_formula() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "labels.fold(7_u16,";
    assert!(source.contains(original));
    let directory = Directory::new();
    let path = directory.0.join("closure_constants.rs");
    std::fs::write(&path, source.replace(original, "labels.fold(8_u16,")).unwrap();
    let report = verify(&path, &["default_fold"], None);
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "default_fold")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(
        proof.status,
        ProofStatus::Refuted,
        "{:?}",
        proof.obligations
    );
    assert!(proof.obligations.iter().any(|o| o.model.is_some()));
}

#[test]
fn closure_fixtures_match_native_execution_and_replay_failures() {
    let directory = Directory::new();
    let executable = directory.0.join("closure-constants-tests");
    let output = Command::new("rustc")
        .args(["--test", "--edition=2024", "-Coverflow-checks=yes"])
        .arg(fixture())
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(executable).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
