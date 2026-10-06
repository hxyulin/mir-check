#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mir-check-constant-table-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/constant_tables.rs")
}

fn contract_library() -> PathBuf {
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-check")).parent().unwrap();
    let mut libraries = Vec::new();
    for entry in std::fs::read_dir(profile.join("build/mir-contracts")).unwrap() {
        let output = entry.unwrap().path().join("out");
        if !output.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(output).unwrap() {
            let path = file.unwrap().path();
            if path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("libmir_contracts-")
                && path
                    .extension()
                    .is_some_and(|ext| ext == "dylib" || ext == "so")
            {
                libraries.push((std::fs::metadata(&path).unwrap().modified().unwrap(), path));
            }
        }
    }
    libraries.sort();
    libraries.pop().unwrap().1
}

fn verify(path: &Path, names: &[&str], target: Option<&str>) -> (Output, Report) {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json"]);
    for name in names {
        command.args(["--entry", name]);
    }
    command.args(["--", "--crate-type=lib", "--edition=2024"]);
    command.arg(path).arg("--out-dir").arg(&directory.0);
    command
        .arg("--extern")
        .arg(format!("mir_contracts={}", contract_library().display()));
    command.args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    let report = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
    (output, report)
}

#[test]
fn evaluated_constant_tables_keep_bounds_storage_bits_and_shape_limits_on_host_and_arm() {
    let expected = [
        ("code_at", ProofStatus::Proved),
        ("edge_codes", ProofStatus::Proved),
        ("sliced_codes", ProofStatus::Proved),
        ("inspection_at", ProofStatus::Proved),
        ("sample_encoding", ProofStatus::Proved),
        ("exhibit_at", ProofStatus::Proved),
        ("row_at", ProofStatus::Proved),
        ("wrong_code", ProofStatus::Refuted),
        ("unbounded_code", ProofStatus::Refuted),
        ("wrong_sample_encoding", ProofStatus::Refuted),
        ("element_limit", ProofStatus::Unknown),
        ("value_limit", ProofStatus::Unknown),
        ("ambiguous_exhibit", ProofStatus::Unknown),
        ("root_array_limit", ProofStatus::Proved),
        ("interior_table", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let entries: Vec<_> = expected.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify(&fixture(), &entries, target);
        assert!(!output.status.success());
        for (name, status) in expected {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, status,
                "{target:?} {name}: {:?}",
                proof.obligations
            );
            if status == ProofStatus::Refuted {
                assert!(proof.obligations.iter().any(|o| o.model.is_some()));
            }
        }
    }
}

#[test]
fn changing_the_evaluated_shelf_spacing_refutes_the_independent_formula() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "codes[position] = position as u16 * 3 + 7;";
    assert_eq!(source.matches(original).count(), 1);
    let directory = Directory::new();
    let mutated = directory.0.join("constant_tables.rs");
    std::fs::write(
        &mutated,
        source.replace(original, "codes[position] = position as u16 * 5 + 7;"),
    )
    .unwrap();
    let (output, report) = verify(&mutated, &["code_at"], None);
    assert!(!output.status.success());
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "code_at")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Refuted);
    assert!(proof.obligations.iter().any(|o| o.model.is_some()));
    let native = directory.0.join("mutated-table-tests");
    let compiled = Command::new("rustc")
        .args(["--test", "--edition=2024", "-Coverflow-checks=yes"])
        .arg(&mutated)
        .arg("--extern")
        .arg(format!("mir_contracts={}", contract_library().display()))
        .arg("-o")
        .arg(&native)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    assert!(!Command::new(native).output().unwrap().status.success());
}
#[test]
fn constant_tables_match_native_slots_and_payloads() {
    let directory = Directory::new();
    let executable = directory.0.join("constant-table-tests");
    let output = Command::new("rustc")
        .args(["--test", "--edition=2024", "-Coverflow-checks=yes"])
        .arg(fixture())
        .arg("--extern")
        .arg(format!("mir_contracts={}", contract_library().display()))
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(executable).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
