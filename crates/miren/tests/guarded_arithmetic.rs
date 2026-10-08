#![forbid(unsafe_code)]

use miren::{ObligationKind, Proof, ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "miren-guarded-arithmetic-{}-{id}",
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

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn verify(path: &Path, entries: &[&str], target: Option<&str>, flags: &[&str]) -> (Output, Report) {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
    command.args(["--verify", "--json", "--quiet"]);
    for entry in entries {
        command.args(["--entry", entry]);
    }
    command
        .args(["--", "--crate-type=lib", "--edition=2024"])
        .arg(path)
        .arg("--out-dir")
        .arg(&directory.0)
        .arg("-Cpanic=abort")
        .args(flags);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    let report = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
    (output, report)
}

fn proof<'a>(report: &'a Report, entry: &str) -> &'a Proof {
    report
        .functions
        .iter()
        .find(|function| function.name == entry)
        .unwrap()
        .proof
        .as_ref()
        .unwrap()
}

#[test]
fn guarded_core_arithmetic_proves_validity_and_preserves_panic_and_memory_boundaries() {
    let entries = [
        ("ticket_u8", ProofStatus::Proved),
        ("ticket_u16", ProofStatus::Proved),
        ("ticket_u32", ProofStatus::Proved),
        ("ticket_u64", ProofStatus::Proved),
        ("ticket_u128", ProofStatus::Proved),
        ("ticket_usize", ProofStatus::Proved),
        ("signed_balance", ProofStatus::Proved),
        ("rejects_overflowing_ticket", ProofStatus::Refuted),
        ("rejects_missing_ticket", ProofStatus::Refuted),
        ("changed_ticket_result", ProofStatus::Refuted),
        ("flattened_ticket_rows", ProofStatus::Unknown),
        ("indirect_ticket_reader", ProofStatus::Unknown),
    ];
    let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let (output, report) = verify(
            &fixture("guarded_arithmetic.rs"),
            &names,
            target,
            &["-Coverflow-checks=yes"],
        );
        assert!(!output.status.success());
        for (entry, expected) in entries {
            let proof = proof(&report, entry);
            assert_eq!(proof.status, expected, "{target:?} {entry}: {proof:?}");
            assert!(proof.trusted_calls.is_empty());
            assert!(proof.assumptions.is_empty());
            if expected == ProofStatus::Refuted {
                assert!(
                    proof
                        .obligations
                        .iter()
                        .any(|obligation| obligation.model.is_some())
                );
            }
            if entry.starts_with("ticket_") {
                for operation in ["AddUnchecked", "SubUnchecked"] {
                    assert!(proof.obligations.iter().any(|obligation| {
                        obligation.kind == ObligationKind::Validity
                            && obligation.status == ProofStatus::Proved
                            && obligation.detail == format!("MIR {operation} must not overflow")
                    }));
                }
                assert!(proof.models.iter().any(|model| {
                    model.contains("core::intrinsics::cold_path:")
                        && model.ends_with("optimization-only cold path marker")
                }));
            }
            if entry == "flattened_ticket_rows" {
                assert!(proof.obligations.iter().any(|obligation| {
                    obligation.kind == ObligationKind::Validity
                        && obligation.status == ProofStatus::Proved
                        && obligation.detail == "MIR MulUnchecked must not overflow"
                }));
                assert!(proof.obligations.iter().any(|obligation| {
                    obligation.status == ProofStatus::Unknown
                        && (obligation
                            .detail
                            .contains("raw subobject addresses need tracked typed layout offsets")
                            || obligation
                                .detail
                                .contains("tracked raw addresses require a sized pointee"))
                }));
            }
        }
    }
}

#[test]
fn compiler_runtime_check_operands_follow_each_host_and_arm_session_configuration() {
    let entries = [
        "ub_checks_enabled",
        "ub_checks_disabled",
        "overflow_checks_enabled",
        "overflow_checks_disabled",
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for ub_checks in [false, true] {
            for overflow_checks in [false, true] {
                let (output, report) = verify(
                    &fixture("compiler_runtime_checks.rs"),
                    &entries,
                    target,
                    &[
                        if ub_checks {
                            "-Zub-checks=yes"
                        } else {
                            "-Zub-checks=no"
                        },
                        if overflow_checks {
                            "-Coverflow-checks=yes"
                        } else {
                            "-Coverflow-checks=no"
                        },
                    ],
                );
                assert!(!output.status.success());
                for (entry, enabled, expected_enabled) in [
                    (entries[0], ub_checks, true),
                    (entries[1], ub_checks, false),
                    (entries[2], overflow_checks, true),
                    (entries[3], overflow_checks, false),
                ] {
                    let expected = if enabled == expected_enabled {
                        ProofStatus::Proved
                    } else {
                        ProofStatus::Refuted
                    };
                    let proof = proof(&report, entry);
                    assert_eq!(
                        proof.status, expected,
                        "{target:?} UB={ub_checks} overflow={overflow_checks} {entry}: {proof:?}"
                    );
                    assert!(proof.trusted_calls.is_empty());
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
    }
}

#[test]
fn native_boundaries_and_a_changed_reconstruction_reject_an_incorrect_proof() {
    let directory = Directory::new();
    let executable = directory.0.join("runtime");
    let output = Command::new("rustc")
        .args(["--test", "--edition=2024"])
        .arg(fixture("guarded_arithmetic.rs"))
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(Command::new(&executable).status().unwrap().success());

    let source = std::fs::read_to_string(fixture("guarded_arithmetic.rs")).unwrap();
    let original = "remaining.checked_add(requested).unwrap() == issued";
    let replacement = "remaining.checked_add(requested.wrapping_add(1)).unwrap() == issued";
    assert!(source.contains(original));
    let mutated = directory.0.join("mutated.rs");
    std::fs::write(&mutated, source.replace(original, replacement)).unwrap();
    let (output, report) = verify(&mutated, &["ticket_u8"], None, &["-Coverflow-checks=yes"]);
    assert!(!output.status.success());
    let proof = proof(&report, "ticket_u8");
    assert_eq!(proof.status, ProofStatus::Refuted);
    assert!(
        proof
            .obligations
            .iter()
            .any(|obligation| obligation.model.is_some())
    );
}
