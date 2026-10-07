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
        let path =
            std::env::temp_dir().join(format!("mir-check-atomic-rmw-{}-{id}", std::process::id()));
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/atomic_rmw.rs")
}

fn verify(
    path: &Path,
    entries: &[(&str, ProofStatus)],
    target: Option<&str>,
    optimized: bool,
    startup: bool,
) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet"]);
    if startup {
        command.args(["--startup", "--allow-assumptions"]);
    }
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    command
        .args([
            "--",
            "--crate-name=atomic_rmw",
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
            .find(|function| function.name == *name)
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
        assert_eq!(!proof.entry_assumptions.is_empty(), startup);
        if *expected == ProofStatus::Refuted {
            assert!(
                proof
                    .obligations
                    .iter()
                    .any(|obligation| obligation.model.is_some())
            );
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
fn integer_bitwise_and_extrema_operations_preserve_exact_owned_history() {
    let entries = [
        (
            "bitwise_updates_retain_the_previous_value",
            ProofStatus::Proved,
        ),
        (
            "signed_extrema_compare_negative_values",
            ProofStatus::Proved,
        ),
        ("unsigned_extrema_keep_the_high_bit", ProofStatus::Proved),
        (
            "signed_bitwise_updates_keep_the_sign_bit",
            ProofStatus::Proved,
        ),
        ("unsigned_word", ProofStatus::Proved),
        ("signed_word", ProofStatus::Proved),
        ("unsigned_double_word", ProofStatus::Proved),
        ("signed_double_word", ProofStatus::Proved),
        ("unsigned_pointer_width", ProofStatus::Proved),
        ("signed_pointer_width", ProofStatus::Proved),
        (
            "signed_comparison_cannot_use_unsigned_order",
            ProofStatus::Proved,
        ),
        ("wrong_bitwise_replacement", ProofStatus::Refuted),
        (
            "a_read_modify_write_cannot_establish_exclusivity",
            ProofStatus::Refuted,
        ),
        ("unsupported_boolean_rmw", ProofStatus::Unknown),
        ("unsupported_pointer_rmw", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized, false);
            if target.is_none() {
                verify(
                    &fixture(),
                    &[
                        ("unsigned_quad_word", ProofStatus::Proved),
                        ("signed_quad_word", ProofStatus::Proved),
                    ],
                    target,
                    optimized,
                    false,
                );
            }
        }
    }
    assert!(native_tests(&fixture(), &Directory::new()));
    let directory = Directory::new();
    let mutant = directory.0.join("mutant.rs");
    let source = std::fs::read_to_string(fixture()).unwrap();
    let minimum = "counter.fetch_min(100, Ordering::Relaxed)";
    assert!(source.contains(minimum));
    std::fs::write(
        &mutant,
        source.replace(minimum, "counter.fetch_max(100, Ordering::Relaxed)"),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &mutant,
            &[(
                "signed_comparison_cannot_use_unsigned_order",
                ProofStatus::Refuted,
            )],
            target,
            false,
            false,
        );
    }
    assert!(!native_tests(&mutant, &directory));
}

#[test]
fn startup_bitwise_updates_need_the_explicit_entry_assumptions() {
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(
                &fixture(),
                &[(
                    "startup_bitwise_history",
                    ProofStatus::ProvedWithAssumptions,
                )],
                target,
                optimized,
                true,
            );
            verify(
                &fixture(),
                &[("startup_bitwise_history", ProofStatus::Refuted)],
                target,
                optimized,
                false,
            );
        }
    }
}
