#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/lazy_root_inputs.rs")
}

fn verify(path: &Path, entries: &[(&str, ProofStatus)], target: Option<&str>, induction: bool) {
    let directory = std::env::temp_dir().join(format!(
        "mir-check-lazy-inputs-{}-{}-{}-{}",
        std::process::id(),
        target.unwrap_or("host"),
        induction,
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet"]);
    if induction {
        command.arg("--induction");
    }
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    command
        .args(["--", "--crate-type=lib", "--edition=2024"])
        .arg(path)
        .arg("--out-dir")
        .arg(&directory)
        .args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    assert!(!output.status.success());
    let report: Report = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
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
            "{target:?} induction={induction} {name}: {:?}",
            proof.obligations,
        );
        assert!(proof.trusted_calls.is_empty());
        if *name == "cached_shapes_do_not_hide_unsupported_types" {
            assert!(
                proof
                    .obligations
                    .iter()
                    .any(|o| o.detail.contains("input field unused"))
            );
        }
        if *name == "shapes_cached_by_previous_fields_still_obey_depth" {
            assert!(
                proof
                    .obligations
                    .iter()
                    .any(|o| o.detail.contains("input field nested"))
            );
        }
        if *expected == ProofStatus::Refuted {
            assert!(
                proof
                    .obligations
                    .iter()
                    .any(|obligation| obligation.model.is_some())
            );
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn large_root_shapes_materialize_only_supported_projected_values() {
    let entries = [
        (
            "shared_payload_shapes_keep_guarded_access_safe",
            ProofStatus::Proved,
        ),
        (
            "shared_payload_shapes_do_not_prove_an_unchecked_index",
            ProofStatus::Refuted,
        ),
        (
            "reused_shapes_keep_argument_symbols_independent",
            ProofStatus::Refuted,
        ),
        ("reused_shapes_preserve_writes", ProofStatus::Proved),
        (
            "shapes_cached_by_previous_fields_still_obey_depth",
            ProofStatus::Unknown,
        ),
        (
            "cached_shapes_do_not_hide_unsupported_types",
            ProofStatus::Unknown,
        ),
        ("lazy_record_membership", ProofStatus::Proved),
        ("lazy_scalar_subarray_membership", ProofStatus::Proved),
        ("a_false_lazy_record_member", ProofStatus::Refuted),
        ("repeated_shapes_keep_distinct_storage", ProofStatus::Proved),
        (
            "shared_descriptors_do_not_make_values_equal",
            ProofStatus::Refuted,
        ),
        ("cached_shapes_still_respect_nesting", ProofStatus::Unknown),
        ("owned_lazy_records_keep_their_fields", ProofStatus::Proved),
        (
            "owned_lazy_records_do_not_satisfy_a_false_claim",
            ProofStatus::Refuted,
        ),
        (
            "oversized_owned_lazy_records_stay_unknown",
            ProofStatus::Unknown,
        ),
        ("guarded_field_index", ProofStatus::Proved),
        ("unguarded_field_index", ProofStatus::Refuted),
        ("updates_preserve_entry_snapshots", ProofStatus::Proved),
        ("a_false_snapshot_claim", ProofStatus::Refuted),
        ("copied_storage_keeps_its_value", ProofStatus::Proved),
        (
            "changing_a_copy_does_not_change_the_input",
            ProofStatus::Proved,
        ),
        ("a_false_copy_claim", ProofStatus::Refuted),
        (
            "independent_roots_have_independent_values",
            ProofStatus::Refuted,
        ),
        ("independent_mutable_storage", ProofStatus::Proved),
        (
            "projected_bytes_still_use_array_storage",
            ProofStatus::Proved,
        ),
        ("tuple_and_nested_arrays", ProofStatus::Proved),
        ("ambiguous_composite_index", ProofStatus::Unknown),
        (
            "unused_raw_address_fields_are_supported",
            ProofStatus::Proved,
        ),
        (
            "an_unused_reference_is_still_unsupported",
            ProofStatus::Unknown,
        ),
        (
            "constrained_inputs_keep_the_eager_boundary",
            ProofStatus::Unknown,
        ),
        (
            "nested_cell_access_is_still_unsupported",
            ProofStatus::Unknown,
        ),
        ("oversized_symbol_reservation", ProofStatus::Unknown),
        (
            "eager_tags_select_lazy_variant_payloads",
            ProofStatus::Proved,
        ),
        ("a_bad_variant_payload_index", ProofStatus::Refuted),
        (
            "eager_invariants_and_lazy_subtrees_coexist",
            ProofStatus::Proved,
        ),
        (
            "eager_nonzero_domain_survives_lazy_siblings",
            ProofStatus::Proved,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(&fixture(), &entries, target, false);
    }
}

#[test]
fn induction_rejects_lazy_state_rather_than_omitting_its_fields() {
    verify(
        &fixture(),
        &[(
            "lazy_loop_state_is_not_silently_omitted",
            ProofStatus::Unknown,
        )],
        None,
        true,
    );
}

#[test]
fn changing_a_guard_or_comparison_refutes_the_root() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "if index < 8 {";
    assert_eq!(source.matches(original).count(), 1);
    let directory = std::env::temp_dir().join(format!(
        "mir-check-lazy-input-mutant-{}",
        std::process::id(),
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("lazy_root_inputs.rs");
    std::fs::write(&path, source.replace(original, "if index <= 8 {")).unwrap();
    verify(
        &path,
        &[("guarded_field_index", ProofStatus::Refuted)],
        None,
        false,
    );
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &path,
            &[(
                "shared_payload_shapes_keep_guarded_access_safe",
                ProofStatus::Refuted,
            )],
            target,
            false,
        );
    }
    let comparison = "self.slot == rhs.slot";
    assert_eq!(source.matches(comparison).count(), 1);
    std::fs::write(&path, source.replace(comparison, "self.slot != rhs.slot")).unwrap();
    verify(
        &path,
        &[("lazy_record_membership", ProofStatus::Refuted)],
        None,
        false,
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn native_lazy_projection_cases_and_failure_mutations_match_the_checker() {
    let directory = std::env::temp_dir().join(format!(
        "mir-check-lazy-input-replay-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("replay");
    let output = Command::new(Path::new(env!("MIR_CHECK_SYSROOT")).join("bin/rustc"))
        .args(["--test", "--edition=2024", "-Coverflow-checks=yes"])
        .arg(fixture())
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("8 passed"));
    std::fs::remove_dir_all(directory).unwrap();
}
