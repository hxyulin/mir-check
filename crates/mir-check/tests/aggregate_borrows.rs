#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mir-check-aggregate-borrow-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/aggregate_borrows.rs")
}

fn contract_library() -> PathBuf {
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-check")).parent().unwrap();
    let mut libraries = Vec::new();
    for entry in std::fs::read_dir(profile.join("build/mir-contracts")).unwrap() {
        let output = entry.unwrap().path().join("out");
        if !output.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(output).unwrap() {
            let path = file.unwrap().path();
            if path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("libmir_contracts-")
                && path
                    .extension()
                    .is_some_and(|ext| ext == "dylib" || ext == "so")
            {
                libraries.push((std::fs::metadata(&path).unwrap().modified().unwrap(), path));
            }
        }
    }
    libraries.sort();
    libraries.pop().unwrap().1
}

fn verify(path: &Path, names: &[&str], target: Option<&str>) -> (Output, Report) {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json"]);
    for name in names {
        command.args(["--entry", name]);
    }
    command.args(["--", "--crate-type=lib", "--edition=2024"]);
    command.arg(path).arg("--out-dir").arg(&directory.0);
    command
        .arg("--extern")
        .arg(format!("mir_contracts={}", contract_library().display()));
    command.args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    let report = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
    (output, report)
}

#[test]
fn tracked_aggregate_borrows_preserve_writes_on_host_and_arm() {
    let entries = [
        ("parcel_pair", ProofStatus::Proved),
        ("tuple_reborrow", ProofStatus::Proved),
        ("captured_counter", ProofStatus::Proved),
        ("returned_capture", ProofStatus::Proved),
        ("optional_borrow", ProofStatus::Proved),
        ("zipped_labels", ProofStatus::Proved),
        ("flattened_bins", ProofStatus::Proved),
        ("wrong_pair", ProofStatus::Refuted),
        ("wrong_capture", ProofStatus::Refuted),
        ("owned_capture_state", ProofStatus::Proved),
        ("wrong_owned_capture_state", ProofStatus::Refuted),
        ("generated_capture_state", ProofStatus::Proved),
        ("mapped_capture_state", ProofStatus::Proved),
        ("mapped_reference_elements", ProofStatus::Proved),
        ("predicate_capture_state", ProofStatus::Proved),
        ("folded_capture_state", ProofStatus::Proved),
        ("byte_capture_boundary", ProofStatus::Proved),
        ("multiple_mutable_inputs", ProofStatus::Proved),
        ("ambiguous_write", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify(&fixture(), &names, target);
        assert!(!output.status.success());
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
                proof.status,
                expected,
                "{target:?} {name}: {:?}",
                proof
                    .obligations
                    .iter()
                    .map(|o| (&o.status, &o.detail))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn swapped_aggregate_field_mutation_is_refuted() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "Pair { left, right }";
    assert_eq!(source.matches(original).count(), 1);
    let directory = Directory::new();
    let path = directory.0.join("aggregate_borrows.rs");
    std::fs::write(
        &path,
        source.replace(original, "Pair { left: right, right: left }"),
    )
    .unwrap();
    let (output, report) = verify(&path, &["parcel_pair"], None);
    assert!(!output.status.success());
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "parcel_pair")
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
fn aggregate_borrows_match_native_writes() {
    let directory = Directory::new();
    let executable = directory.0.join("aggregate-borrow-tests");
    let output = Command::new("rustc")
        .args(["--test", "--edition=2024", "-Coverflow-checks=yes"])
        .arg(fixture())
        .arg("--extern")
        .arg(format!("mir_contracts={}", contract_library().display()))
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
