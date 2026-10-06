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
            "mir-check-function-items-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/function_item_resolution.rs")
}

fn verify(path: &Path, names: &[&str], target: Option<&str>) -> (Output, Report) {
    verify_with_contracts(path, names, target, None)
}

fn verify_with_contracts(
    path: &Path,
    names: &[&str],
    target: Option<&str>,
    contracts: Option<&Path>,
) -> (Output, Report) {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json"]);
    if let Some(contracts) = contracts {
        command.arg("--contracts").arg(contracts);
    }
    for name in names {
        command.args(["--entry", name]);
    }
    command.args(["--", "--crate-type=lib", "--edition=2024"]);
    command.arg(path).arg("--out-dir").arg(&directory.0);
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
fn trait_function_items_execute_their_resolved_impls_on_host_and_arm() {
    let entries = [
        ("one_label", ProofStatus::Proved),
        ("mapped_labels", ProofStatus::Proved),
        ("wrong_mapped_label", ProofStatus::Refuted),
        ("generated_slots", ProofStatus::Proved),
        ("accumulated_labels", ProofStatus::Proved),
        ("wrong_accumulated_label", ProofStatus::Refuted),
        ("checked_labels", ProofStatus::Proved),
        ("rejected_label", ProofStatus::Refuted),
        ("label_bytes", ProofStatus::Proved),
        ("signed_label_bytes", ProofStatus::Proved),
        ("wide_label_bytes", ProofStatus::Proved),
        ("wrong_label_bytes", ProofStatus::Refuted),
        (
            "application_encoding_names_execute_their_body",
            ProofStatus::Proved,
        ),
        ("round_trip_u8", ProofStatus::Proved),
        ("round_trip_i8", ProofStatus::Proved),
        ("round_trip_u16", ProofStatus::Proved),
        ("round_trip_i16", ProofStatus::Proved),
        ("round_trip_u32", ProofStatus::Proved),
        ("round_trip_i32", ProofStatus::Proved),
        ("round_trip_u64", ProofStatus::Proved),
        ("round_trip_i64", ProofStatus::Proved),
        ("round_trip_u128", ProofStatus::Proved),
        ("round_trip_i128", ProofStatus::Proved),
        ("round_trip_usize", ProofStatus::Proved),
        ("round_trip_isize", ProofStatus::Proved),
        ("validation_without_a_model", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let names: Vec<_> = entries.iter().map(|(name, _)| *name).collect();
        let (output, report) = verify(&fixture(), &names, target);
        assert!(!output.status.success());
        for (name, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {name}: {:?}",
                proof.obligations
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
            if name == "validation_without_a_model" {
                assert!(proof.obligations.iter().any(|obligation| {
                    obligation
                        .detail
                        .contains("MIR body unavailable for core::str::from_utf8")
                }));
            }
        }
    }
}

#[test]
fn rebuilding_core_exposes_actual_validation_mir_without_assuming_pointer_operations_safe() {
    let compiler = Command::new("rustc").arg("-vV").output().unwrap();
    let compiler = String::from_utf8(compiler.stdout).unwrap();
    let host = compiler
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap();
    for target in [host, "thumbv7em-none-eabihf"] {
        let directory = Directory::new();
        std::fs::write(
            directory.0.join("Cargo.toml"),
            "[package]\nname='label_validation'\nversion='0.1.0'\nedition='2024'\n\
             [lib]\npath='lib.rs'\n[workspace]\n[profile.dev]\npanic='abort'\n",
        )
        .unwrap();
        std::fs::write(
            directory.0.join("lib.rs"),
            "#![no_std]\n#![forbid(unsafe_code)]\n\
             pub fn accepts_label(bytes: &[u8]) -> bool { core::str::from_utf8(bytes).is_ok() }\n",
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-mir-check"))
            .args([
                "--verify",
                "--quiet",
                "--entry",
                "accepts_label",
                "-Zbuild-std=core",
                "--target",
                target,
                "--offline",
            ])
            .current_dir(&directory.0)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let runs = std::fs::read_dir(directory.0.join("target/mir-check")).unwrap();
        let reports = runs
            .flat_map(|run| std::fs::read_dir(run.unwrap().path().join("reports")).unwrap())
            .map(|file| file.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(
            reports.len(),
            1,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Report = serde_json::from_slice(&std::fs::read(&reports[0]).unwrap()).unwrap();
        let proof = report
            .functions
            .iter()
            .find(|f| f.name == "accepts_label")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, ProofStatus::Unknown);
        assert!(
            proof
                .analyzed_bodies
                .iter()
                .any(|body| body.starts_with("core::str::from_utf8 "))
        );
        assert!(proof.trusted_calls.is_empty());
        assert!(
            proof
                .obligations
                .iter()
                .any(|obligation| { obligation.detail.contains("unsupported rvalue &raw const") })
        );
        assert!(!proof.obligations.iter().any(|obligation| {
            obligation
                .detail
                .contains("MIR body unavailable for core::str::from_utf8")
        }));
        let replay = Command::new(env!("CARGO_BIN_EXE_mir-check"))
            .args([
                "--verify",
                "--json",
                "--entry",
                "accepts_label",
                "--from-report",
            ])
            .arg(&reports[0])
            .current_dir(&directory.0)
            .output()
            .unwrap();
        assert!(!replay.status.success());
        let replay: Report = serde_json::from_slice(&replay.stdout).unwrap();
        let replay = replay
            .functions
            .iter()
            .find(|f| f.name == "accepts_label")
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.status, replay.status);
        assert_eq!(proof.analyzed_bodies, replay.analyzed_bodies);
    }
}

#[test]
fn changing_the_byte_position_refutes_the_encoding_claim() {
    let directory = Directory::new();
    let source = std::fs::read_to_string(fixture()).unwrap();
    assert!(source.contains("(value >> 8) as u8"));
    let path = directory.0.join("function_item_resolution.rs");
    std::fs::write(
        &path,
        source.replace("(value >> 8) as u8", "(value >> 7) as u8"),
    )
    .unwrap();
    let (output, report) = verify(&path, &["label_bytes"], None);
    assert!(!output.status.success());
    assert_eq!(
        report
            .functions
            .iter()
            .find(|function| function.name == "label_bytes")
            .unwrap()
            .proof
            .as_ref()
            .unwrap()
            .status,
        ProofStatus::Refuted,
    );
}

#[test]
fn endian_claims_replay_for_native_integer_boundaries() {
    let directory = Directory::new();
    let path = directory.0.join("replay.rs");
    let mut source = format!(
        "#![forbid(unsafe_code)]\n#[path={}]\nmod claims;\nfn main() {{\n",
        serde_json::to_string(&fixture()).unwrap(),
    );
    for integer in [
        "u8", "i8", "u16", "i16", "u32", "i32", "u64", "i64", "u128", "i128", "usize", "isize",
    ] {
        source.push_str(&format!(
            "for value in [0_{integer}, 1, {integer}::MIN, {integer}::MAX, \
             0x55aa_33cc_7788_1199_u128 as {integer}] \
             {{ claims::round_trip_{integer}(value); }}\n",
        ));
    }
    source.push_str("for value in 0..1024 { claims::label_bytes(value); }\n}");
    std::fs::write(&path, source).unwrap();
    let executable = directory.0.join("replay");
    let build = Command::new("rustc")
        .args(["--edition=2024", "-Coverflow-checks=yes"])
        .arg(&path)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(Command::new(executable).status().unwrap().success());
}
