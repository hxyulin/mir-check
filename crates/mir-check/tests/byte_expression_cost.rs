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
            "mir-check-byte-expression-cost-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/byte_expression_cost.rs")
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
fn repeated_numeric_chunk_copies_keep_complete_queries_bounded_on_host_and_arm() {
    let entries = [
        ("scaled_sample_storage", ProofStatus::Proved),
        ("wrong_sample_storage", ProofStatus::Refuted),
        ("oversized_sample_storage", ProofStatus::Unknown),
    ];
    let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let (_, report) = verify(&fixture(), &names, target);
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|function| function.name.ends_with(name))
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status,
                expected,
                "{name}: {:?}",
                proof
                    .obligations
                    .iter()
                    .filter(|o| o.status != ProofStatus::Proved)
                    .map(|o| &o.detail)
                    .collect::<Vec<_>>()
            );
            if expected != ProofStatus::Unknown {
                let queries: Vec<_> = proof
                    .obligations
                    .iter()
                    .filter_map(|o| o.query.as_ref())
                    .collect();
                assert!(!queries.is_empty());
                assert!(
                    queries.iter().all(|query| query.len() < 60_000),
                    "largest query: {}",
                    queries.iter().map(|q| q.len()).max().unwrap()
                );
                assert!(queries.iter().any(|query| query.contains("(let ((t")));
                assert!(queries.iter().any(|query| query.contains("((_ extract")));
            }
        }
    }
}

#[test]
fn sample_storage_assertions_match_native_execution() {
    let directory = Directory::new();
    let executable = directory.0.join("sample-storage-tests");
    let output = Command::new("rustc")
        .args(["--test", "--edition=2024", "-Coverflow-checks=yes"])
        .arg(fixture())
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

#[test]
fn changed_trailer_encoding_is_refuted() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let directory = Directory::new();
    let path = directory.0.join("byte_expression_cost.rs");
    std::fs::write(&path, source.replace("&[61, 83]", "&[61, 84]")).unwrap();
    let (_, report) = verify(&path, &["scaled_sample_storage"], None);
    let proof = report
        .functions
        .iter()
        .find(|f| f.name.ends_with("scaled_sample_storage"))
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.status, ProofStatus::Refuted);
    assert!(proof.obligations.iter().any(|o| o.model.is_some()));
}
