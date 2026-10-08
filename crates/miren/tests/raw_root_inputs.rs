#![forbid(unsafe_code)]

use miren::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("miren-raw-root-inputs-{}-{id}", std::process::id()));
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/raw_root_inputs.rs")
}

fn verify(
    path: &Path,
    entries: &[(&str, ProofStatus)],
    target: Option<&str>,
    optimized: bool,
    startup: bool,
) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
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
            "--crate-name=raw_root_inputs",
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
    }
    report
}

fn native_tests(path: &Path, directory: &Directory) -> bool {
    let executable = directory.0.join("native");
    let compiler = Path::new(env!("MIREN_SYSROOT")).join("bin/rustc");
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
fn thin_pointer_inputs_have_address_bits_and_nonnull_patterns_but_no_pointee_storage() {
    let entries = [
        (
            "raw_inputs_preserve_observed_nullability",
            ProofStatus::Proved,
        ),
        ("nonnull_inputs_have_nonzero_addresses", ProofStatus::Proved),
        ("address_fields_are_symbolic", ProofStatus::Proved),
        ("raw_inputs_need_not_be_null", ProofStatus::Refuted),
        (
            "root_addresses_do_not_grant_pointee_storage",
            ProofStatus::Unknown,
        ),
        ("fat_pointer_inputs_remain_unknown", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized, false);
        }
    }
    assert!(native_tests(&fixture(), &Directory::new()));
}

#[test]
fn removing_the_pointer_guard_fails_symbolic_and_native_checks() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "if pointer.is_null() {";
    assert!(source.contains(original));
    let directory = Directory::new();
    let mutant = directory.0.join("mutant.rs");
    std::fs::write(
        &mutant,
        source.replacen(original, "if !pointer.is_null() {", 1),
    )
    .unwrap();
    verify(
        &mutant,
        &[(
            "raw_inputs_preserve_observed_nullability",
            ProofStatus::Refuted,
        )],
        None,
        false,
        false,
    );
    assert!(!native_tests(&mutant, &directory));
}
