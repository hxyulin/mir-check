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
            "mir-check-local-atomics-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/local_atomics.rs")
}

fn verify(
    path: &Path,
    entries: &[(&str, ProofStatus)],
    target: Option<&str>,
    optimized: bool,
    contracts: Option<&Path>,
) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet"]);
    if let Some(path) = contracts {
        command.arg("--contracts").arg(path);
    }
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    command
        .args([
            "--",
            "--crate-name=local_atomics",
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
fn local_integer_atomics_preserve_owned_history_and_reject_unsupported_escapes() {
    let entries = [
        ("constructed_counter_is_fresh", ProofStatus::Proved),
        (
            "aliases_calls_and_owned_returns_share_history",
            ProofStatus::Proved,
        ),
        (
            "separate_aggregate_fields_keep_distinct_atomic_locations",
            ProofStatus::Proved,
        ),
        (
            "strong_cas_preserves_failure_and_updates_success",
            ProofStatus::Proved,
        ),
        (
            "weak_cas_keeps_spurious_failure_history",
            ProofStatus::Proved,
        ),
        ("signed_modular_updates", ProofStatus::Proved),
        ("occupied_counter_is_not_fresh", ProofStatus::Refuted),
        ("incorrect_retained_value_is_refuted", ProofStatus::Refuted),
        ("weak_cas_can_fail_spuriously", ProofStatus::Refuted),
        ("release_load_panics", ProofStatus::Refuted),
        ("unsupported_pointer_escape", ProofStatus::Unknown),
        ("pointer_atomic_loads_are_conservative", ProofStatus::Proved),
        (
            "synthetic::local_borrow_cannot_escape",
            ProofStatus::Unknown,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized, None);
            if target.is_none() {
                verify(
                    &fixture(),
                    &[("unsupported_thread_publication", ProofStatus::Unknown)],
                    target,
                    optimized,
                    None,
                );
            }
        }
    }
    assert!(native_tests(&fixture(), &Directory::new()));
    let directory = Directory::new();
    let mutant = directory.0.join("mutant.rs");
    let source = std::fs::read_to_string(fixture()).unwrap();
    let constructor = "let counter = AtomicU8::new(0);";
    assert!(source.contains(constructor));
    std::fs::write(
        &mutant,
        source.replace(constructor, "let counter = AtomicU8::new(1);"),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &mutant,
            &[("constructed_counter_is_fresh", ProofStatus::Refuted)],
            target,
            false,
            None,
        );
    }
    assert!(!native_tests(&mutant, &directory));
}

#[test]
fn trusted_calls_cannot_certify_absence_of_atomic_interference() {
    let directory = Directory::new();
    let contracts = directory.0.join("contracts.json");
    std::fs::write(
        &contracts,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "functions": [{
                "function": "local_atomics::boundary", "arguments": ["counter"],
                "trusted": true, "no_panic": true, "modifies": [],
                "reason": "Independent synthetic external boundary"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            let report = verify(
                &fixture(),
                &[
                    ("trusted_boundary_loses_history", ProofStatus::Refuted),
                    ("a_store_cannot_restore_exclusivity", ProofStatus::Refuted),
                    (
                        "a_new_owned_allocation_after_a_boundary_is_fresh",
                        ProofStatus::ProvedWithAssumptions,
                    ),
                ],
                target,
                optimized,
                Some(&contracts),
            );
            for function in report.functions {
                if let Some(proof) = function.proof {
                    assert_eq!(proof.trusted_calls.len(), 1);
                }
            }
        }
    }
}

#[test]
fn unrestricted_trusted_effects_do_not_resurrect_atomic_storage() {
    let directory = Directory::new();
    let contracts = directory.0.join("contracts.json");
    std::fs::write(
        &contracts,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "functions": [{
                "function": "local_atomics::boundary", "arguments": ["counter"],
                "trusted": true, "no_panic": true,
                "reason": "Independent synthetic external boundary"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &fixture(),
            &[
                ("trusted_boundary_loses_history", ProofStatus::Unknown),
                ("a_store_cannot_restore_exclusivity", ProofStatus::Unknown),
            ],
            target,
            false,
            Some(&contracts),
        );
    }
}

#[test]
fn abstraction_labels_follow_the_failing_query_instead_of_all_executed_operations() {
    let unrelated = "an_unused_weak_choice_does_not_explain_a_bounds_failure";
    let report = verify(
        &fixture(),
        &[
            ("weak_cas_can_fail_spuriously", ProofStatus::Refuted),
            ("incorrect_retained_value_is_refuted", ProofStatus::Refuted),
            (unrelated, ProofStatus::Refuted),
        ],
        None,
        false,
        None,
    );
    for function in report.functions {
        let Some(proof) = function.proof else {
            continue;
        };
        let failure = proof
            .obligations
            .iter()
            .find(|obligation| obligation.status == ProofStatus::Refuted)
            .unwrap();
        if function.name == "weak_cas_can_fail_spuriously" {
            assert_eq!(
                failure.abstraction_reasons,
                ["weak compare-exchange permits spurious failure"]
            );
        } else {
            assert!(failure.abstraction_reasons.is_empty(), "{}", function.name);
        }
    }
}
