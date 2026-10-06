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
            "mir-check-iterator-audit-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/iterator_audit.rs")
}

fn verify(path: &Path, directory: &Directory, entries: &[&str], target: Option<&str>) -> Report {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--json", "--verify"]);
    for entry in entries {
        command.args(["--entry", entry]);
    }
    command.args(["--", "--crate-type=lib", "--edition=2024"]);
    command.arg(path).arg("--out-dir").arg(&directory.0);
    command.args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    assert!(
        !output.status.success(),
        "negative fixture must fail verification"
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn iterator_models_keep_skip_bounds_storage_aliases_and_unknown_views_on_host_and_arm() {
    let expected = [
        ("mixed_scores", ProofStatus::Proved),
        ("oversized_skips", ProofStatus::Proved),
        ("zero_sized_slots", ProofStatus::Proved),
        ("skipped_storage", ProofStatus::Proved),
        ("shared_tally", ProofStatus::Proved),
        ("separated_writes", ProofStatus::Proved),
        ("wrong_alias_claim", ProofStatus::Refuted),
        ("unavailable_view", ProofStatus::Unknown),
        ("owned_mutable_environment", ProofStatus::Unknown),
    ];
    let entries: Vec<_> = expected.iter().map(|(name, _)| *name).collect();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let directory = Directory::new();
        let report = verify(&fixture(), &directory, &entries, target);
        for (name, status) in expected {
            let proof = report
                .functions
                .iter()
                .find(|function| function.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, status,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
            assert!(proof.trusted_calls.is_empty());
        }
    }
}

#[test]
fn cursor_exhaustion_mutations_are_refuted() {
    let directory = Directory::new();
    let mutated = directory.0.join("iterator_audit.rs");
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "assert!(forward.next().is_none());";
    assert_eq!(source.matches(original).count(), 1);
    std::fs::write(
        &mutated,
        source.replace(original, "assert!(forward.next().is_some());"),
    )
    .unwrap();
    let report = verify(&mutated, &directory, &["oversized_skips"], None);
    let proof = report
        .functions
        .iter()
        .find(|function| function.name == "oversized_skips")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Refuted);
    assert!(
        proof
            .obligations
            .iter()
            .any(|obligation| obligation.model.is_some())
    );
}

#[test]
fn iterator_audit_agrees_with_bounded_native_cases() {
    let directory = Directory::new();
    let binary = directory.0.join("iterator-audit");
    let compiled = Command::new("rustc")
        .args(["--test", "--edition=2024", "-Coverflow-checks=yes"])
        .arg(fixture())
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let output = Command::new(binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
