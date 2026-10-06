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
            "mir-check-float-storage-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/float_storage.rs")
}

fn verify(path: &Path, names: &[&str], target: Option<&str>) -> (Output, Report) {
    verify_with_contracts(path, names, target, None)
}

fn verify_with_contracts(
    path: &Path,
    names: &[&str],
    target: Option<&str>,
    contracts: Option<&Path>,
) -> (Output, Report) {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json"]);
    if let Some(contracts) = contracts {
        command.arg("--contracts").arg(contracts);
    }
    for name in names {
        command.args(["--entry", name]);
    }
    command.args(["--", "--crate-type=lib", "--edition=2024"]);
    command.arg(path).arg("--out-dir").arg(&directory.0);
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
fn storage_labels_preserve_ieee_encodings_and_keep_arithmetic_nan_bits_unconstrained() {
    let entries = [
        ("archive_word", ProofStatus::Proved),
        ("archive_wide_word", ProofStatus::Proved),
        ("restore_sample", ProofStatus::Proved),
        ("restore_wide_sample", ProofStatus::Proved),
        ("signed_zero_labels", ProofStatus::Proved),
        ("special_labels", ProofStatus::Proved),
        ("mirrored_label", ProofStatus::Proved),
        ("unsigned_label", ProofStatus::Proved),
        ("chosen_sample", ProofStatus::Proved),
        ("bounded_label", ProofStatus::Proved),
        ("stable_calculation_label", ProofStatus::Proved),
        ("rounded_label", ProofStatus::Proved),
        ("integer_cast_label", ProofStatus::Proved),
        ("bad_sign_label", ProofStatus::Refuted),
        ("bad_special_label", ProofStatus::Refuted),
        ("bad_calculated_nan_label", ProofStatus::Refuted),
        ("unsupported_remainder", ProofStatus::Unknown),
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
            if expected == ProofStatus::Refuted {
                assert!(
                    proof
                        .obligations
                        .iter()
                        .any(|obligation| obligation.model.is_some())
                );
            }
        }
    }
}

#[test]
fn changing_the_sign_mask_refutes_the_preservation_assertion() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "bits ^ 0x8000_0000";
    assert!(source.contains(original));
    let directory = Directory::new();
    let path = directory.0.join("float_storage.rs");
    std::fs::write(&path, source.replace(original, "bits ^ 0x4000_0000")).unwrap();
    let (output, report) = verify(&path, &["mirrored_label"], None);
    assert!(!output.status.success());
    assert_eq!(
        report
            .functions
            .iter()
            .find(|f| f.name == "mirrored_label")
            .unwrap()
            .proof
            .as_ref()
            .unwrap()
            .status,
        ProofStatus::Refuted
    );
}

#[test]
fn storage_labels_match_native_rust_including_signaling_nan_inputs() {
    let directory = Directory::new();
    let executable = directory.0.join("float-storage-tests");
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

#[test]
fn checked_core_bodies_execute_typed_transmutes_without_library_summaries() {
    let directory = Directory::new();
    let config = directory.0.join("contracts.json");
    let mut functions = Vec::new();
    for width in ["f32", "f64"] {
        for method in ["to_bits", "from_bits"] {
            functions.push(serde_json::json!({
                "function": format!("core::{width}::<impl {width}>::{method}"),
                "requires": ["true"]
            }));
        }
    }
    std::fs::write(
        &config,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "functions": functions
        }))
        .unwrap(),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let (output, report) = verify_with_contracts(
            &fixture(),
            &["archive_word", "archive_wide_word", "bad_sign_label"],
            target,
            Some(&config),
        );
        assert!(!output.status.success());
        for name in ["archive_word", "archive_wide_word", "bad_sign_label"] {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            let expected = if name == "bad_sign_label" {
                ProofStatus::Refuted
            } else {
                ProofStatus::Proved
            };
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
            assert!(proof.models.is_empty());
            assert!(
                proof
                    .analyzed_bodies
                    .iter()
                    .any(|body| body.contains("::from_bits"))
            );
            assert!(
                proof
                    .analyzed_bodies
                    .iter()
                    .any(|body| body.contains("::to_bits"))
            );
            assert!(proof.trusted_calls.is_empty());
        }
    }
}
