#![forbid(unsafe_code)]

use miren::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("miren-from-fn-{}-{id}", std::process::id()));
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/array_from_fn.rs")
}

fn contract_library() -> PathBuf {
    let profile = Path::new(env!("CARGO_BIN_EXE_miren")).parent().unwrap();
    let mut libraries = Vec::new();
    for entry in std::fs::read_dir(profile.join("build/miren-contracts")).unwrap() {
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
                .starts_with("libmiren_contracts-")
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
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
    command.args(["--verify", "--json"]);
    for name in names {
        command.args(["--entry", name]);
    }
    command.args(["--", "--crate-type=lib", "--edition=2024"]);
    command.arg(path).arg("--out-dir").arg(&directory.0);
    command
        .arg("--extern")
        .arg(format!("miren_contracts={}", contract_library().display()));
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
fn generated_arrays_execute_callbacks_in_order_and_keep_failures_and_limits_on_host_and_arm() {
    let entries = [
        ("ticket_numbers", ProofStatus::Proved),
        ("byte_labels", ProofStatus::Proved),
        ("parcel_slots", ProofStatus::Proved),
        ("callback_order", ProofStatus::Proved),
        ("caller_effects", ProofStatus::Proved),
        ("empty", ProofStatus::Proved),
        ("function_item", ProofStatus::Proved),
        ("larger_owned_array", ProofStatus::Proved),
        ("maximum_owned_array", ProofStatus::Proved),
        ("branch_results", ProofStatus::Proved),
        ("floating_samples", ProofStatus::Proved),
        ("same_named_user_function", ProofStatus::Refuted),
        ("bad_callback", ProofStatus::Refuted),
        ("bad_overflow", ProofStatus::Refuted),
        ("bad_call_bound", ProofStatus::Refuted),
        ("mutable_capture", ProofStatus::Proved),
        ("owned_mutable_capture", ProofStatus::Refuted),
        ("identity_elements", ProofStatus::Unknown),
        ("element_limit", ProofStatus::Unknown),
        ("value_budget_boundary", ProofStatus::Proved),
        ("value_limit", ProofStatus::Unknown),
        ("element_destructor", ProofStatus::Unknown),
        ("callback_destructor", ProofStatus::Unknown),
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
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
            if expected == ProofStatus::Proved {
                assert!(
                    proof
                        .models
                        .iter()
                        .any(|model| model.contains("core::array::from_fn"))
                );
            }
            match name {
                "owned_mutable_capture" => assert!(
                    proof
                        .obligations
                        .iter()
                        .any(|o| o.status == ProofStatus::Refuted && o.model.is_some())
                ),
                "element_limit" => assert!(
                    proof
                        .obligations
                        .iter()
                        .any(|o| o.detail.contains("128-element"))
                ),
                "value_limit" => assert!(
                    proof
                        .obligations
                        .iter()
                        .any(|o| o.detail.contains("256-value"))
                ),
                "element_destructor" | "callback_destructor" => assert!(
                    proof
                        .obligations
                        .iter()
                        .any(|o| o.detail.contains("destructors"))
                ),
                _ => (),
            }
        }
    }
}

#[test]
fn changing_the_generated_label_offset_refutes_the_unchanged_assertion() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "index as u8 + 1";
    assert!(source.contains(original));
    let directory = Directory::new();
    let path = directory.0.join("array_from_fn.rs");
    std::fs::write(&path, source.replace(original, "index as u8 + 2")).unwrap();
    let (output, report) = verify(&path, &["byte_labels"], None);
    assert!(!output.status.success());
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "byte_labels")
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
fn generated_arrays_match_native_rust_results_and_callback_effects() {
    let directory = Directory::new();
    let executable = directory.0.join("array-from-fn-tests");
    let output = Command::new("rustc")
        .args(["--test", "--edition=2024", "-Coverflow-checks=yes"])
        .arg(fixture())
        .arg("--extern")
        .arg(format!("miren_contracts={}", contract_library().display()))
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
