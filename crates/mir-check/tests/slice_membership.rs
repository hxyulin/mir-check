#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/slice_membership.rs")
}

const ENTRIES: &[(&str, ProofStatus)] = &[
    ("integer_member", ProofStatus::Proved),
    ("signed_member", ProofStatus::Proved),
    ("byte_member", ProofStatus::Proved),
    ("boolean_member", ProofStatus::Proved),
    ("character_member", ProofStatus::Proved),
    ("empty_member", ProofStatus::Proved),
    ("bounded_bytes", ProofStatus::Proved),
    ("shifted_bytes", ProofStatus::Proved),
    ("shifted_view_excludes_endpoints", ProofStatus::Proved),
    ("range_view_is_unsupported", ProofStatus::Unknown),
    ("float_member", ProofStatus::Proved),
    ("signed_zero_member", ProofStatus::Proved),
    ("nan_is_not_a_member", ProofStatus::Proved),
    ("wrong_member", ProofStatus::Refuted),
    ("nan_reflexivity", ProofStatus::Refuted),
    ("unbounded_bytes", ProofStatus::Unknown),
    ("over_budget_bytes", ProofStatus::Unknown),
    ("limit_bytes", ProofStatus::Proved),
    ("empty_custom_equality_does_not_run", ProofStatus::Proved),
    ("custom_equality_panics", ProofStatus::Refuted),
    ("custom_equality_effects", ProofStatus::Proved),
    ("custom_equality_absent_effects", ProofStatus::Proved),
    ("custom_receiver_order", ProofStatus::Proved),
    ("counterfeit_contains", ProofStatus::Refuted),
];

fn check(
    path: &Path,
    target: Option<&str>,
    entries: &[&str],
    optimized: bool,
    inlined: bool,
) -> Report {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet"]);
    for name in entries {
        command.args(["--entry", name]);
    }
    command
        .args(["--", "--crate-type=lib", "--edition=2024"])
        .arg(path)
        .args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
    if optimized {
        command.arg("-Copt-level=2");
    }
    if inlined {
        command.args(["-Zinline-mir=yes", "-Zmir-opt-level=4"]);
    }
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn primitive_membership_and_custom_comparisons_are_checked_on_host_and_arm() {
    let entries = ENTRIES.iter().map(|(name, _)| *name).collect::<Vec<_>>();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            let report = check(&fixture(), target, &entries, optimized, false);
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
                    "{target:?} optimized={optimized} {name}: {:?}",
                    proof.obligations
                );
                assert!(proof.trusted_calls.is_empty());
            }
        }
    }
}

#[test]
fn inlined_core_specializations_preserve_numeric_and_custom_equality_boundaries() {
    let entries = [
        "integer_member",
        "byte_member",
        "boolean_member",
        "custom_equality_panics",
        "custom_equality_effects",
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let report = check(&fixture(), target, &entries, true, true);
        for (name, expected) in [
            ("integer_member", ProofStatus::Proved),
            ("byte_member", ProofStatus::Proved),
            ("boolean_member", ProofStatus::Proved),
            ("custom_equality_panics", ProofStatus::Unknown),
            ("custom_equality_effects", ProofStatus::Unknown),
        ] {
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
            assert!(proof.trusted_calls.is_empty());
        }
        let integer = report
            .functions
            .iter()
            .find(|f| f.name == "integer_member")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert!(integer.models.iter().any(|model| {
            model.starts_with("<u16 as core::slice::cmp::SliceContains>::slice_contains:")
        }));
    }
}

#[test]
fn membership_mutation_is_refuted_on_host_and_arm() {
    let directory = std::env::temp_dir().join(format!(
        "mir-check-membership-mutation-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("slice_membership.rs");
    let source = std::fs::read_to_string(fixture()).unwrap();
    assert_eq!(source.matches("values.contains(&values[3])").count(), 1);
    assert_eq!(source.matches("count.get() == 1").count(), 1);
    let source = source.replace(
        "values.contains(&values[3])",
        "values.contains(&(values[3] ^ 1))",
    );
    let source = source.replace("count.get() == 1", "count.get() == 2");
    std::fs::write(&path, source).unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let report = check(
            &path,
            target,
            &["integer_member", "custom_equality_effects"],
            false,
            false,
        );
        for name in ["integer_member", "custom_equality_effects"] {
            assert_eq!(
                report
                    .functions
                    .iter()
                    .find(|f| f.name == name)
                    .unwrap()
                    .proof
                    .as_ref()
                    .unwrap()
                    .status,
                ProofStatus::Refuted
            );
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn native_membership_replay_checks_custom_effects_and_counterexamples() {
    let directory = std::env::temp_dir().join(format!(
        "mir-check-membership-replay-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("replay");
    let output = Command::new(Path::new(env!("MIR_CHECK_SYSROOT")).join("bin/rustc"))
        .args(["--test", "--edition=2024"])
        .arg(fixture())
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    std::fs::remove_dir_all(directory).unwrap();
}
