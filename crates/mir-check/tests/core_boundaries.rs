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
            "mir-check-core-boundaries-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/core_boundaries.rs")
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
fn compiler_option_helpers_refute_panics_and_unit_returns_keep_primitive_effects() {
    let entries = [
        ("known_ticket", ProofStatus::Proved),
        ("optional_ticket", ProofStatus::Refuted),
        ("expected_ticket", ProofStatus::Unknown),
        ("guarded_ticket", ProofStatus::Proved),
        ("application_helper_names", ProofStatus::Proved),
        ("application_message_helper", ProofStatus::Unknown),
        ("application_nonreturning_helper", ProofStatus::Unknown),
        ("bad_application_helper_names", ProofStatus::Refuted),
        ("primitive_advance", ProofStatus::Proved),
        ("unresolved_ticket", ProofStatus::Unknown),
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
            if name == "optional_ticket" {
                assert!(
                    proof
                        .obligations
                        .iter()
                        .any(|obligation| obligation.status == ProofStatus::Refuted
                            && obligation.detail == "panic entry point is reachable"
                            && obligation.model.is_some())
                );
            }
            if name == "application_nonreturning_helper" {
                assert!(
                    proof.obligations.iter().any(|obligation| obligation
                        .detail
                        .contains("symbolic execution step limit"))
                );
                assert!(
                    !proof
                        .obligations
                        .iter()
                        .any(|obligation| obligation.status == ProofStatus::Refuted)
                );
            }
            if name == "primitive_advance" {
                assert!(
                    proof
                        .analyzed_bodies
                        .iter()
                        .any(|body| body.contains("<u32 as core::ops::AddAssign>::add_assign"))
                );
            }
        }
    }
}

#[test]
fn changing_the_advance_amount_refutes_the_preserved_postcondition() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "add_assign(&mut running, 7)";
    assert!(source.contains(original));
    let directory = Directory::new();
    let path = directory.0.join("core_boundaries.rs");
    std::fs::write(
        &path,
        source.replace(original, "add_assign(&mut running, 8)"),
    )
    .unwrap();
    let (output, report) = verify(&path, &["primitive_advance"], None);
    assert!(!output.status.success());
    assert_eq!(
        report
            .functions
            .iter()
            .find(|f| f.name == "primitive_advance")
            .unwrap()
            .proof
            .as_ref()
            .unwrap()
            .status,
        ProofStatus::Refuted
    );
}

#[test]
fn ticket_guards_and_unit_returning_effects_match_native_rust() {
    let directory = Directory::new();
    let executable = directory.0.join("core-boundaries-tests");
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
