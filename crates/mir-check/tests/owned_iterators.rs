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
            "mir-check-owned-iterator-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/owned_iterators.rs")
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
fn owned_iterator_order_effects_failures_and_boundaries_are_checked_on_host_and_arm() {
    let entries = [
        ("station_labels", ProofStatus::Proved),
        ("parcel_ends", ProofStatus::Proved),
        ("byte_count", ProofStatus::Proved),
        ("empty", ProofStatus::Proved),
        ("unit_moves", ProofStatus::Proved),
        ("owned_structs", ProofStatus::Proved),
        ("predicate_effects", ProofStatus::Proved),
        ("predicate_call_bounds", ProofStatus::Proved),
        ("ordered_fold", ProofStatus::Proved),
        ("last_and_adapters", ProofStatus::Proved),
        ("wrapped_iterator_drop", ProofStatus::Proved),
        ("borrowed_consuming_methods", ProofStatus::Proved),
        ("borrowed_shared_count", ProofStatus::Proved),
        ("callback_panic", ProofStatus::Refuted),
        ("bad_call_bound", ProofStatus::Refuted),
        ("wrong_order", ProofStatus::Refuted),
        ("bad_fold", ProofStatus::Refuted),
        ("bad_borrowed_count", ProofStatus::Refuted),
        ("bad_borrowed_shared_count", ProofStatus::Refuted),
        ("bad_borrowed_last", ProofStatus::Refuted),
        ("bad_borrowed_passthrough", ProofStatus::Refuted),
        ("identity_elements", ProofStatus::Unknown),
        ("element_destructor", ProofStatus::Unknown),
        ("enclosing_destructor", ProofStatus::Unknown),
        ("borrowed_element", ProofStatus::Unknown),
        ("mutable_capture", ProofStatus::Unknown),
        ("unsupported_view", ProofStatus::Unknown),
        ("unsupported_clone", ProofStatus::Unknown),
        ("same_named_user_iterator", ProofStatus::Refuted),
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
            if expected == ProofStatus::Proved && name != "borrowed_shared_count" {
                assert!(
                    proof
                        .models
                        .iter()
                        .any(|model| model.contains("owned array iterator"))
                );
            }
        }
    }
}

#[test]
fn reversing_the_expected_station_label_refutes_the_unchanged_cursor() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "pending.next().unwrap() == offset";
    assert!(source.contains(original));
    let directory = Directory::new();
    let path = directory.0.join("owned_iterators.rs");
    std::fs::write(
        &path,
        source.replace(original, "pending.next().unwrap() == offset + 6"),
    )
    .unwrap();
    let (output, report) = verify(&path, &["station_labels"], None);
    assert!(!output.status.success());
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "station_labels")
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
fn owned_iterators_match_native_positions_and_callback_effects() {
    let directory = Directory::new();
    let executable = directory.0.join("owned-iterator-tests");
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
