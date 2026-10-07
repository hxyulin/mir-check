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
    Command::new(executable)
        .arg("--test-threads=1")
        .output()
        .unwrap()
        .status
        .success()
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

#[test]
fn static_slices_preserve_element_references_and_execute_short_circuit_callbacks() {
    let entries = [
        ("rows", ProofStatus::Proved),
        ("static_slice_cursor_offsets", ProofStatus::Proved),
        ("static_array_index", ProofStatus::Proved),
        (
            "array_of_references_preserves_reference_values",
            ProofStatus::Proved,
        ),
        ("static_find_map_skips_later_panic", ProofStatus::Proved),
        ("static_find_map_reaches_later_panic", ProofStatus::Refuted),
        ("static_find_map_exhausts", ProofStatus::Proved),
        (
            "static_find_map_returns_later_reference",
            ProofStatus::Proved,
        ),
        (
            "static_find_map_callback_read_is_unknown",
            ProofStatus::Unknown,
        ),
        ("static_slice_payload_read_is_unknown", ProofStatus::Unknown),
        ("static_slice_budget_is_unknown", ProofStatus::Unknown),
        ("empty_static_slice", ProofStatus::Proved),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized, None);
        }
    }
}

#[test]
fn empty_static_slices_keep_their_effect_invalidation_marker() {
    let mut specification = serde_json::json!({"schema_version":1,"functions":[{
        "function":"static_views::boundary", "trusted":true,"no_panic":true,
        "reason":"Explicit boundary for empty storage invalidation testing"
    }]});
    for (framed, status) in [
        (false, ProofStatus::Unknown),
        (true, ProofStatus::ProvedWithAssumptions),
    ] {
        if framed {
            specification["functions"][0]["modifies"] = serde_json::json!([]);
        }
        for target in [None, Some("thumbv7em-none-eabihf")] {
            verify(
                &fixture(),
                &[
                    ("empty_static_slice_is_invalidated", status),
                    ("empty_static_iterator_is_invalidated", status),
                ],
                target,
                false,
                Some(specification.clone()),
            );
        }
    }
}

#[test]
fn incorrect_static_slice_offsets_and_short_circuit_claims_fail() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    for (original, mutation, name) in [
        (
            "as usize + 32",
            "as usize + 31",
            "static_slice_cursor_offsets",
        ),
        (
            "assert!(calls == 1);",
            "assert!(calls == 2);",
            "static_find_map_skips_later_panic",
        ),
        (
            "assert!(calls == 3);",
            "assert!(calls == 2);",
            "static_find_map_exhausts",
        ),
    ] {
        assert!(source.contains(original));
        let directory = Directory::new();
        let path = directory.0.join("mutant.rs");
        std::fs::write(&path, source.replace(original, mutation)).unwrap();
        for target in [None, Some("thumbv7em-none-eabihf")] {
            verify(&path, &[(name, ProofStatus::Refuted)], target, false, None);
        }
        assert!(!native_tests(&path, &directory));
    }
}

#[test]
fn atomic_overlays_require_dense_initialized_storage_and_preserve_arbitrary_values() {
    let entries = [
        ("atomic_overlay_load", ProofStatus::Proved),
        ("atomic_overlay_reborrow", ProofStatus::Proved),
        ("atomic_overlay_at_nonzero_offset", ProofStatus::Proved),
        ("atomic_bool_array_overlay", ProofStatus::Proved),
        ("atomic_overlay_wrong_bound", ProofStatus::Refuted),
        (
            "atomic_overlay_is_not_an_initializer_snapshot",
            ProofStatus::Refuted,
        ),
        ("atomic_overlay_padding_is_unknown", ProofStatus::Unknown),
        (
            "atomic_overlay_misaligned_field_is_unknown",
            ProofStatus::Unknown,
        ),
        (
            "atomic_overlay_uninitialized_is_unknown",
            ProofStatus::Unknown,
        ),
        (
            "atomic_overlay_plain_storage_is_unknown",
            ProofStatus::Unknown,
        ),
        ("atomic_overlay_cannot_extend_a_field", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized, None);
        }
    }
}

#[test]
fn atomic_storage_layout_mutations_respect_the_accessed_footprint() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    for (original, mutation, root, status, replay) in [
        (
            "#[repr(C, align(4))]\nstruct Lanes",
            "#[repr(C, align(8))]\nstruct Lanes",
            "atomic_overlay_load",
            ProofStatus::Proved,
            false,
        ),
        (
            "assert!(pointer as usize == base + 4);",
            "assert!(pointer as usize == base + 5);",
            "atomic_overlay_at_nonzero_offset",
            ProofStatus::Refuted,
            true,
        ),
    ] {
        assert!(source.contains(original));
        let directory = Directory::new();
        let path = directory.0.join("mutant.rs");
        std::fs::write(&path, source.replace(original, mutation)).unwrap();
        for target in [None, Some("thumbv7em-none-eabihf")] {
            verify(&path, &[(root, status)], target, false, None);
        }
        if replay {
            assert!(!native_tests(&path, &directory));
        }
    }
    let directory = Directory::new();
    let path = directory.0.join("internal-padding.rs");
    let mutation = source
        .replace(
            "low: core::sync::atomic::AtomicU16,",
            "low: core::sync::atomic::AtomicU8,",
        )
        .replace(
            "low: core::sync::atomic::AtomicU16::new(9)",
            "low: core::sync::atomic::AtomicU8::new(9)",
        );
    assert_ne!(mutation, source);
    std::fs::write(&path, mutation).unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &path,
            &[("atomic_overlay_load", ProofStatus::Unknown)],
            target,
            false,
            None,
        );
    }
}

#[test]
fn atomic_prefixes_opaque_addresses_and_reference_slots_keep_their_boundaries() {
    let entries = [
        (
            "opaque_fields_outside_an_atomic_prefix",
            ProofStatus::Proved,
        ),
        ("atomic_prefix_cannot_read_a_union", ProofStatus::Unknown),
        ("atomic_prefix_cannot_read_a_pointer", ProofStatus::Unknown),
        ("maybe_uninit_payload_address", ProofStatus::Proved),
        (
            "maybe_uninit_address_does_not_prove_initialization",
            ProofStatus::Unknown,
        ),
        (
            "zero_arg_closure_captures_static_reference",
            ProofStatus::Proved,
        ),
        (
            "zero_arg_then_captures_static_reference",
            ProofStatus::Proved,
        ),
        ("zero_arg_then_calls_function_item", ProofStatus::Proved),
        (
            "a_mutable_reference_slot_preserves_static_views",
            ProofStatus::Proved,
        ),
        (
            "mutable_static_addresses_are_supported",
            ProofStatus::Proved,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized, None);
        }
    }
}

#[test]
fn raw_get_preserves_certificates_without_promoting_casts_or_initializing_payloads() {
    let entries = [
        (
            "certified_raw_get_preserves_an_address",
            ProofStatus::Proved,
        ),
        (
            "a_certified_raw_get_cannot_return_null",
            ProofStatus::Refuted,
        ),
        (
            "raw_get_cannot_certify_an_unrelated_pointee",
            ProofStatus::Unknown,
        ),
        ("raw_get_cannot_initialize_a_payload", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized, None);
        }
    }
}

#[test]
fn mutable_static_borrows_preserve_addresses_without_readable_or_initialization_facts() {
    let entries = [
        (
            "mutable_static_addresses_are_supported",
            ProofStatus::Proved,
        ),
        ("mutable_static_stores_remain_opaque", ProofStatus::Proved),
        (
            "a_mutable_static_array_keeps_its_typed_address",
            ProofStatus::Proved,
        ),
        (
            "mutable_static_slice_coercions_remain_unknown",
            ProofStatus::Unknown,
        ),
        (
            "changing_pointer_spelling_does_not_grant_write_access",
            ProofStatus::Unknown,
        ),
        (
            "a_mutable_static_borrow_does_not_retain_a_payload",
            ProofStatus::Unknown,
        ),
        (
            "a_panic_after_a_mutable_static_borrow_is_reachable",
            ProofStatus::Refuted,
        ),
        (
            "uninitialized_static_payloads_cannot_be_borrowed_mutably",
            ProofStatus::Unknown,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized, None);
        }
    }
    assert!(native_tests(&fixture(), &Directory::new()));
    let directory = Directory::new();
    let mutant = directory.0.join("mutant.rs");
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "raw_word_container(&INNER_WORD).cast()";
    assert!(source.contains(original));
    std::fs::write(
        &mutant,
        source.replacen(original, "UNINITIALIZED_WORD.get().cast()", 1),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(
                &mutant,
                &[(
                    "certified_raw_get_preserves_an_address",
                    ProofStatus::Unknown,
                )],
                target,
                optimized,
                None,
            );
        }
    }
}
