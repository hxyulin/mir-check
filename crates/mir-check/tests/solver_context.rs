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
            "mir-check-solver-context-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/solver_context.rs")
}

fn verify(path: &Path, entries: &[(&str, ProofStatus)], target: Option<&str>) {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet"]);
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    command
        .args(["--", "--crate-type=lib", "--edition=2024"])
        .arg(path)
        .arg("--out-dir")
        .arg(&directory.0)
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
            "{target:?} {name}: {:?}",
            proof.obligations
        );
        assert!(proof.trusted_calls.is_empty());
        if *expected == ProofStatus::Refuted {
            assert!(proof.obligations.iter().any(|o| o.model.is_some()));
        }
    }
}

#[test]
fn integer_probes_preserve_float_domains_and_real_failures_on_host_and_arm() {
    let entries = [
        (
            "a_float_branch_keeps_its_integer_index_guard",
            ProofStatus::Proved,
        ),
        (
            "a_float_constraint_can_be_needed_for_an_integer_proof",
            ProofStatus::Proved,
        ),
        (
            "a_full_float_counterexample_must_keep_the_integer_input",
            ProofStatus::Refuted,
        ),
        (
            "unsupported_float_operations_stay_unknown",
            ProofStatus::Unknown,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(&fixture(), &entries, target);
    }
}

#[test]
fn relaxing_the_integer_guard_refutes_the_same_float_branch_on_host_and_arm() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "scale.is_finite() && index < 6";
    assert!(source.contains(original));
    let directory = Directory::new();
    let path = directory.0.join("solver_context.rs");
    std::fs::write(
        &path,
        source.replace(original, "scale.is_finite() && index <= 6"),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &path,
            &[(
                "a_float_branch_keeps_its_integer_index_guard",
                ProofStatus::Refuted,
            )],
            target,
        );
    }
}

#[test]
fn the_original_fixture_and_guard_mutation_replay_natively() {
    let directory = Directory::new();
    let executable = directory.0.join("native");
    let compiler = Path::new(env!("MIR_CHECK_SYSROOT")).join("bin/rustc");
    let output = Command::new(&compiler)
        .args(["--test", "--edition=2024"])
        .arg(fixture())
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = Command::new(&executable).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let source = std::fs::read_to_string(fixture()).unwrap();
    let path = directory.0.join("mutant.rs");
    std::fs::write(
        &path,
        source.replace(
            "scale.is_finite() && index < 6",
            "scale.is_finite() && index <= 6",
        ),
    )
    .unwrap();
    let output = Command::new(compiler)
        .args(["--test", "--edition=2024"])
        .arg(path)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = Command::new(executable).output().unwrap();
    assert!(
        !output.status.success(),
        "the off-by-one mutation must panic natively"
    );
}
