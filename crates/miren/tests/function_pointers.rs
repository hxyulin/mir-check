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
        let path = std::env::temp_dir().join(format!(
            "miren-function-pointers-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/function_pointers.rs")
}

fn verify(
    path: &Path,
    entries: &[(&str, ProofStatus)],
    target: Option<&str>,
    optimized: bool,
    specification: Option<serde_json::Value>,
) {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
    command.args(["--verify", "--json", "--quiet"]);
    if let Some(specification) = specification {
        let config = directory.0.join("contracts.json");
        std::fs::write(&config, serde_json::to_vec(&specification).unwrap()).unwrap();
        command.arg("--contracts").arg(config);
    }
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    command
        .args(["--", "--crate-type=lib", "--edition=2024"])
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
        entries
            .iter()
            .all(|(_, status)| *status == ProofStatus::Proved)
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
        assert!(proof.trusted_calls.is_empty());
        if *expected == ProofStatus::Refuted {
            assert!(proof.obligations.iter().any(|o| o.model.is_some()));
        }
    }
}

#[test]
fn known_function_pointers_execute_targets_and_unknown_targets_stay_unknown() {
    let entries = [
        ("returned_target", ProofStatus::Proved),
        ("selected_target", ProofStatus::Proved),
        ("stored_target", ProofStatus::Proved),
        ("generic_target", ProofStatus::Proved),
        ("mixed_generic_targets", ProofStatus::Proved),
        ("different_const_generic_shapes", ProofStatus::Proved),
        ("rust_call_adapters", ProofStatus::Proved),
        ("pointer_call_preserves_effects", ProofStatus::Proved),
        ("empty_argument_tuple", ProofStatus::Proved),
        ("pointer_call_checks_panics", ProofStatus::Refuted),
        ("wrong_return_claim", ProofStatus::Refuted),
        ("different_selected_target", ProofStatus::Refuted),
        ("empty_const_generic_target", ProofStatus::Refuted),
        ("arbitrary_pointer_is_unknown", ProofStatus::Unknown),
        ("closure_pointer_is_unknown", ProofStatus::Unknown),
        ("caller_adapter_is_unknown", ProofStatus::Unknown),
        ("numeric_pointer_is_unknown", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized, None);
        }
    }
}

#[test]
fn changing_the_reified_target_breaks_the_dependent_proof() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let mutation = source.replace("    low_bits\n}", "    unchanged\n}");
    assert_ne!(source, mutation);
    let directory = Directory::new();
    let path = directory.0.join("mutant.rs");
    std::fs::write(&path, mutation).unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &path,
            &[("returned_target", ProofStatus::Refuted)],
            target,
            false,
            None,
        );
    }
    assert!(!native_tests(&path, &directory));
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
fn known_target_proofs_replay_natively() {
    assert!(native_tests(&fixture(), &Directory::new()));
}

#[test]
fn calls_through_pointers_check_contracts_against_actual_arguments_and_returns() {
    let specification = serde_json::json!({"schema_version": 1, "functions": [{
        "function": "function_pointers::low_bits", "arguments": ["value"],
        "requires": ["value < 4"], "ensures": ["result == value"]
    }]});
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &fixture(),
            &[
                ("bounded_argument", ProofStatus::Proved),
                ("returned_target", ProofStatus::Refuted),
            ],
            target,
            false,
            Some(specification.clone()),
        );
    }
}

#[test]
fn cached_generic_signatures_do_not_skip_call_contracts() {
    let specification = serde_json::json!({"schema_version": 1, "functions": [{
        "function": "function_pointers::select", "arguments": ["value"],
        "requires": ["value < 4"], "ensures": ["result == value"]
    }]});
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &fixture(),
            &[
                ("bounded_generic_argument", ProofStatus::Proved),
                ("generic_target", ProofStatus::Refuted),
            ],
            target,
            false,
            Some(specification.clone()),
        );
    }
}

#[test]
fn mutating_a_cached_const_generic_body_breaks_both_dependent_proofs() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let mutation = source.replace("    values[0]\n", "    values[1]\n");
    assert_ne!(source, mutation);
    let directory = Directory::new();
    let path = directory.0.join("mutant.rs");
    std::fs::write(&path, mutation).unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &path,
            &[("different_const_generic_shapes", ProofStatus::Refuted)],
            target,
            false,
            None,
        );
    }
    assert!(!native_tests(&path, &directory));
}

#[test]
fn a_known_target_without_dependency_mir_remains_unknown() {
    let directory = Directory::new();
    let dependency = directory.0.join("dependency.rs");
    std::fs::write(
        &dependency,
        "#![no_std]\n#[inline(never)] pub fn narrow(value: u8) -> u8 { value & 3 }\n",
    )
    .unwrap();
    let compiler = Path::new(env!("MIREN_SYSROOT")).join("bin/rustc");
    let output = Command::new(compiler)
        .args([
            "--crate-name=callback_dependency",
            "--crate-type=rlib",
            "--edition=2024",
            "-Zalways-encode-mir=no",
        ])
        .arg(&dependency)
        .arg("--out-dir")
        .arg(&directory.0)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let source = directory.0.join("caller.rs");
    std::fs::write(
        &source,
        concat!(
            "#![no_std]\n#![forbid(unsafe_code)]\n",
            "pub fn entry(value: u8) {\n",
            "    let callback: fn(u8) -> u8 = callback_dependency::narrow;\n",
            "    assert!(callback(value) < 4);\n}\n",
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_miren"))
        .args([
            "--verify",
            "--json",
            "--quiet",
            "--entry",
            "entry",
            "--",
            "--crate-type=lib",
            "--edition=2024",
            "-Cpanic=abort",
        ])
        .arg(&source)
        .arg("--out-dir")
        .arg(&directory.0)
        .arg("--extern")
        .arg(format!(
            "callback_dependency={}",
            directory.0.join("libcallback_dependency.rlib").display()
        ))
        .output()
        .unwrap();
    let report: Report = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
    let proof = report
        .functions
        .iter()
        .find(|function| function.name == "entry")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(proof.status, ProofStatus::Unknown);
    assert!(
        proof
            .obligations
            .iter()
            .any(|obligation| obligation.detail.contains("MIR body unavailable"))
    );
}
