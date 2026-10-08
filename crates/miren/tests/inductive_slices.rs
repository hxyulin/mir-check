#![forbid(unsafe_code)]

use miren::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/inductive_slices.rs")
}

const PROVED: &[&str] = &[
    "initialized_local_scalar_storage",
    "initialized_local_byte_storage",
    "returned_references_keep_their_index_and_current_value",
    "shared_slice_reads",
    "mutable_slice_writes",
    "earlier_references_keep_their_element",
    "retained_mutable_references_are_disjoint",
    "advancing_from_both_ends",
    "skipping_elements",
    "backward_skips",
    "indexed_local_borrows",
    "indexed_references_reach_helper_storage",
    "indexed_helper_snapshots",
    "cloned_cursors_advance_independently",
    "borrowed_count_exhausts_the_original_cursor",
    "skipping_past_the_end_cannot_leave_items",
    "word_array_iterator_reads",
    "word_array_iterator_writes",
    "boolean_array_iterator_writes",
];
const UNKNOWN: &[&str] = &[
    "return_layout_inference_cannot_assume_the_first_element",
    "shared_slice_bad_bound",
    "mutable_slice_bad_write",
    "a_false_retained_reference_claim",
    "an_off_by_one_indexed_borrow",
    "indexed_entry_values_are_not_current_values",
    "count_cannot_leave_a_borrowed_cursor_nonempty",
    "an_incorrect_word_array_write",
    "a_nonbyte_unbounded_slice_is_still_unknown",
    "indexed_slice_views_are_still_unknown",
];

fn run(entries: &[&str], target: Option<&str>, induction: bool) -> Report {
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
    command.args(["--verify", "--json", "--quiet"]);
    if induction {
        command.arg("--induction");
    }
    for entry in entries {
        command.args(["--entry", entry]);
    }
    command
        .args(["--", "--crate-type=lib", "--edition=2024"])
        .arg(fixture())
        .args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn indexed_references_cursors_helpers_and_mutations_work_on_host_and_arm() {
    let entries = PROVED.iter().chain(UNKNOWN).copied().collect::<Vec<_>>();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let report = run(&entries, target, true);
        for entry in entries.iter().copied() {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == entry)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            let expected = if PROVED.contains(&entry) {
                ProofStatus::Proved
            } else {
                ProofStatus::Unknown
            };
            assert_eq!(
                proof.status, expected,
                "{target:?} {entry}: {:?}",
                proof.obligations
            );
            assert!(proof.trusted_calls.is_empty());
            assert_eq!(
                proof.invariants.len(),
                usize::from(expected == ProofStatus::Proved)
            );
            if expected == ProofStatus::Proved {
                assert!(
                    proof.obligations[0]
                        .query
                        .as_ref()
                        .unwrap()
                        .contains(":pp.max_indent 0")
                );
                assert!(
                    !proof.obligations[0]
                        .query
                        .as_ref()
                        .unwrap()
                        .contains(":fp.xform.bit_blast true")
                );
            }
            if entry == "indexed_helper_snapshots" {
                assert!(
                    proof
                        .analyzed_bodies
                        .iter()
                        .any(|body| body.contains("increase"))
                );
            }
        }
    }
}

#[test]
fn complete_unrolling_and_induction_agree_on_small_reference_and_array_cases() {
    let positive = [
        "earlier_references_keep_their_element",
        "retained_mutable_references_are_disjoint",
        "indexed_local_borrows",
        "word_array_iterator_reads",
        "word_array_iterator_writes",
        "boolean_array_iterator_writes",
    ];
    let negative = [
        "a_false_retained_reference_claim",
        "an_off_by_one_indexed_borrow",
        "an_incorrect_word_array_write",
    ];
    let entries = positive
        .iter()
        .chain(&negative)
        .copied()
        .collect::<Vec<_>>();
    let report = run(&entries, None, false);
    for entry in entries {
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        let expected = if positive.contains(&entry) {
            ProofStatus::Proved
        } else {
            ProofStatus::Refuted
        };
        assert_eq!(proof.status, expected, "{entry}: {:?}", proof.obligations);
        assert!(proof.invariants.is_empty());
    }
}

#[test]
fn native_replay_checks_cursor_boundaries_retained_aliases_and_false_contract_claims() {
    let directory = std::env::temp_dir().join(format!("miren-slice-replay-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("replay");
    let output = Command::new("rustc")
        .args(["--test", "--edition=2024"])
        .arg(fixture())
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("4 passed"));
    std::fs::remove_dir_all(directory).unwrap();
}
