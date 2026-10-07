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
            "mir-check-static-views-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/static_views.rs")
}

fn verify(
    path: &Path,
    entries: &[(&str, ProofStatus)],
    target: Option<&str>,
    optimized: bool,
    specification: Option<serde_json::Value>,
) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet", "--allow-assumptions"]);
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
            "--crate-name=static_views",
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
fn restored_static_layouts_preserve_provenance_without_loading_mutable_payloads() {
    let entries = [
        ("restored", ProofStatus::Proved),
        ("direct", ProofStatus::Proved),
        ("address_checks", ProofStatus::Proved),
        ("field_addresses_preserve_offsets", ProofStatus::Proved),
        ("reads_are_not_initializer_snapshots", ProofStatus::Proved),
        ("wrong_address_claim", ProofStatus::Refuted),
        ("restored_then_panics", ProofStatus::Refuted),
        ("unaligned_is_unknown", ProofStatus::Unknown),
        ("oversized_is_unknown", ProofStatus::Unknown),
        ("unrelated_layout_is_unknown", ProofStatus::Unknown),
        ("uninitialized_payload_is_unknown", ProofStatus::Unknown),
        ("raw_writes_are_unknown", ProofStatus::Unknown),
        (
            "numeric_pointer_cannot_restore_storage",
            ProofStatus::Unknown,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            let report = verify(&fixture(), &entries, target, optimized, None);
            for function in report.functions.iter().filter(|f| f.proof.is_some()) {
                assert!(function.proof.as_ref().unwrap().trusted_calls.is_empty());
            }
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == "uninitialized_payload_is_unknown")
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert!(
                proof
                    .obligations
                    .iter()
                    .any(|o| o.detail.contains("payload reads need a state model"))
            );
        }
    }
}

#[test]
fn trusted_unknown_effects_invalidate_views_and_explicit_frames_preserve_them() {
    let mut specification = serde_json::json!({"schema_version":1,"functions":[{
        "function":"static_views::boundary", "trusted":true,"no_panic":true,
        "reason":"Explicit boundary for storage invalidation testing"
    }]});
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &fixture(),
            &[("invalidated_view_is_unknown", ProofStatus::Unknown)],
            target,
            false,
            Some(specification.clone()),
        );
    }
    specification["functions"][0]["modifies"] = serde_json::json!([]);
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &fixture(),
            &[("preserves_view", ProofStatus::ProvedWithAssumptions)],
            target,
            false,
            Some(specification.clone()),
        );
    }
}

#[test]
fn broken_address_and_restoration_claims_never_prove() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    for (original, mutation, name, expected, replay) in [
        (
            "assert!(!pointer.is_null());",
            "assert!(pointer.is_null());",
            "address_checks",
            ProofStatus::Refuted,
            true,
        ),
        (
            "assert!(pointer as usize & 7 == 0);",
            "assert!(pointer as usize & 7 != 0);",
            "address_checks",
            ProofStatus::Refuted,
            true,
        ),
        (
            "assert!(field == base + 4);",
            "assert!(field == base + 5);",
            "field_addresses_preserve_offsets",
            ProofStatus::Refuted,
            true,
        ),
        (
            "share(ERASED.bytes.get().cast::<u8>().cast::<Record>())",
            "share(UNALIGNED.get().cast::<Record>())",
            "restored",
            ProofStatus::Unknown,
            false,
        ),
    ] {
        assert!(source.contains(original));
        let directory = Directory::new();
        let path = directory.0.join("mutant.rs");
        std::fs::write(&path, source.replace(original, mutation)).unwrap();
        for target in [None, Some("thumbv7em-none-eabihf")] {
            verify(&path, &[(name, expected)], target, false, None);
        }
        if replay {
            assert!(!native_tests(&path, &directory));
        }
    }
}

#[test]
fn valid_scoped_reinterpretations_replay_natively() {
    assert!(native_tests(&fixture(), &Directory::new()));
}

#[test]
fn induction_does_not_treat_unsupported_static_state_as_success() {
    let directory = Directory::new();
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args([
            "--verify",
            "--induction",
            "--json",
            "--quiet",
            "--entry",
            "unbounded_static_state",
            "--",
            "--crate-type=lib",
            "--edition=2024",
        ])
        .arg(fixture())
        .arg("--out-dir")
        .arg(&directory.0)
        .arg("-Cpanic=abort")
        .output()
        .unwrap();
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "unbounded_static_state")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(
        proof.status,
        ProofStatus::Unknown,
        "{:?}",
        proof.obligations
    );
    assert!(!output.status.success());
}
