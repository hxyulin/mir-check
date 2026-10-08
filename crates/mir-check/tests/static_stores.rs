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
            "mir-check-static-stores-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/static_stores.rs")
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
            "--crate-name=static_stores",
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
fn typed_stores_and_uninitialized_addresses_keep_shared_reads_opaque() {
    let entries = [
        ("stores_owned_values", ProofStatus::Proved),
        ("non_null_preserves_static_addresses", ProofStatus::Proved),
        ("non_null_wrong_address_claim", ProofStatus::Refuted),
        (
            "non_null_integer_handles_have_verified_validity",
            ProofStatus::Proved,
        ),
        ("initialized_wrapper_addresses", ProofStatus::Proved),
        ("uninit_cell_address_chain", ProofStatus::Proved),
        ("stores_a_constructed_future", ProofStatus::Proved),
        ("caller_owned_references_are_retained", ProofStatus::Proved),
        ("writes_do_not_hide_panics", ProofStatus::Refuted),
        (
            "future_construction_checks_real_calls",
            ProofStatus::Refuted,
        ),
        (
            "writes_do_not_establish_shared_read_facts",
            ProofStatus::Unknown,
        ),
        ("read_only_field_stores_are_unknown", ProofStatus::Unknown),
        (
            "a_store_does_not_supply_uninit_read_facts",
            ProofStatus::Unknown,
        ),
        (
            "uninitialized_atomic_reads_are_unknown",
            ProofStatus::Unknown,
        ),
        ("frame_owned_references_cannot_escape", ProofStatus::Unknown),
        (
            "integer_addresses_do_not_authorize_stores",
            ProofStatus::Unknown,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            let report = verify(&fixture(), &entries, target, optimized, None);
            let proof = report
                .functions
                .iter()
                .find(|function| function.name == "frame_owned_references_cannot_escape")
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert!(proof.obligations.iter().any(|obligation| {
                obligation
                    .detail
                    .contains("frame-owned storage cannot escape")
            }));
        }
    }
}

#[test]
fn unknown_trusted_effects_keep_static_reference_escape_evidence() {
    let specification = serde_json::json!({"schema_version":1,"functions":[{
        "function":"static_stores::boundary", "trusted":true, "no_panic":true,
        "reason":"Explicit unknown-effect boundary for escape checks"
    }]});
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let report = verify(
            &fixture(),
            &[(
                "unknown_effects_do_not_erase_escape_checks",
                ProofStatus::Unknown,
            )],
            target,
            false,
            Some(specification.clone()),
        );
        let proof = report
            .functions
            .iter()
            .find(|function| function.name == "unknown_effects_do_not_erase_escape_checks")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.trusted_calls.len(), 1);
        assert!(proof.obligations.iter().any(|obligation| {
            obligation
                .detail
                .contains("frame-owned storage cannot escape")
        }));
    }
}

#[test]
fn changing_a_constructor_guard_refutes_the_dependent_store_and_fails_native_replay() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let mutation = source.replace("if value < 8 {", "if value < 9 {");
    assert_ne!(source, mutation);
    let directory = Directory::new();
    let path = directory.0.join("mutant.rs");
    std::fs::write(&path, mutation).unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &path,
            &[("stores_a_constructed_future", ProofStatus::Refuted)],
            target,
            false,
            None,
        );
    }
    assert!(!native_tests(&path, &directory));
}

#[test]
fn scoped_positive_stores_replay_natively() {
    assert!(native_tests(&fixture(), &Directory::new()));
}
