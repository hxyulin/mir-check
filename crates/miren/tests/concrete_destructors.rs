#![forbid(unsafe_code)]

use miren::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/concrete_destructors.rs")
}

const ENTRIES: &[(&str, ProofStatus)] = &[
    ("destructor_then_fields", ProofStatus::Proved),
    ("tuple_fields_in_order", ProofStatus::Proved),
    ("branch_and_explicit_drop", ProofStatus::Proved),
    ("only_the_active_variant_drops", ProofStatus::Proved),
    ("moving_a_field_drops_it_once", ProofStatus::Proved),
    ("mutable_guard_effects", ProofStatus::Proved),
    ("bounded_destructor", ProofStatus::Proved),
    ("a_closure_drops_its_owned_capture", ProofStatus::Proved),
    ("unreachable_unsupported_drop", ProofStatus::Proved),
    ("panicking_destructor", ProofStatus::Refuted),
    ("guarded_leaf_boundary", ProofStatus::Refuted),
    ("wrong_field_order", ProofStatus::Refuted),
    ("wrong_mutable_guard_effect", ProofStatus::Refuted),
    ("slice_destructor_is_unsupported", ProofStatus::Unknown),
    (
        "unpolled_coroutines_do_not_construct_body_locals",
        ProofStatus::Proved,
    ),
];

#[test]
fn concrete_destructors_check_effects_order_moves_and_failures_on_host_and_arm() {
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
        command.args(["--verify", "--json", "--quiet"]);
        for (name, _) in ENTRIES {
            command.args(["--entry", name]);
        }
        command
            .args(["--", "--crate-type=lib", "--edition=2024"])
            .arg(fixture())
            .args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
        if let Some(target) = target {
            command.args(["--target", target]);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        let report: Report = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
        for (name, expected) in ENTRIES {
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
                "{target:?} {name}: {:?}",
                proof.obligations
            );
            assert!(proof.trusted_calls.is_empty());
        }
    }
}

#[test]
fn native_replay_checks_drop_effects_and_failing_mutations() {
    let directory = std::env::temp_dir().join(format!("miren-drop-replay-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("replay");
    let output = Command::new(Path::new(env!("MIREN_SYSROOT")).join("bin/rustc"))
        .args(["--test", "--edition=2024"])
        .arg(fixture())
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn explicit_leaf_contracts_preserve_callback_checks_and_reach_release() {
    let directory =
        std::env::temp_dir().join(format!("miren-drop-contracts-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let contracts = directory.join("contracts.json");
    std::fs::write(
        &contracts,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "functions": [
                {
                    "function": "concrete_destructors::acquire_leaf",
                    "trusted": true,
                    "no_panic": true,
                    "ensures": ["result == 0"],
                    "modifies": [],
                    "reason": "Explicit test assumption for the acquisition leaf."
                },
                {
                    "function": "concrete_destructors::release_leaf",
                    "arguments": ["restore_state"],
                    "requires": ["restore_state == 0"],
                    "trusted": true,
                    "no_panic": true,
                    "modifies": [],
                    "reason": "Explicit test assumption for the restoration leaf."
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
        command.args(["--verify", "--json", "--quiet", "--allow-assumptions"]);
        command.arg("--contracts").arg(&contracts);
        for name in ["guarded_leaf_boundary", "broken_leaf_callback"] {
            command.args(["--entry", name]);
        }
        command
            .args(["--", "--crate-type=lib", "--edition=2024"])
            .arg(fixture())
            .args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
        if let Some(target) = target {
            command.args(["--target", target]);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        let report: Report = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
        let guarded = report
            .functions
            .iter()
            .find(|f| f.name == "guarded_leaf_boundary")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(guarded.status, ProofStatus::ProvedWithAssumptions);
        for name in [
            "concrete_destructors::acquire_leaf",
            "concrete_destructors::release_leaf",
        ] {
            assert!(
                guarded
                    .trusted_calls
                    .iter()
                    .any(|call| call.contract.function == name)
            );
        }
        let broken = report
            .functions
            .iter()
            .find(|f| f.name == "broken_leaf_callback")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(broken.status, ProofStatus::Refuted);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn loosening_a_destructor_guard_refutes_the_changed_fixture() {
    let directory =
        std::env::temp_dir().join(format!("miren-drop-mutation-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "pub fn bounded_destructor(value: u8) {\n    if value < 4 {";
    assert!(source.contains(original));
    let mutated = directory.join("mutated.rs");
    std::fs::write(
        &mutated,
        source.replace(original, &original.replace("< 4", "<= 4")),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_miren"))
        .args([
            "--verify",
            "--json",
            "--quiet",
            "--entry",
            "bounded_destructor",
        ])
        .args(["--", "--crate-type=lib", "--edition=2024"])
        .arg(mutated)
        .args(["-Cpanic=abort", "-Coverflow-checks=yes"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "bounded_destructor")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Refuted);
    std::fs::remove_dir_all(directory).unwrap();
}
