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
            "mir-check-async-coroutines-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/async_coroutines.rs")
}

fn verify(path: &Path, entries: &[(&str, ProofStatus)], target: Option<&str>, optimized: bool) {
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
fn constructed_futures_preserve_poll_states_captures_and_panics_on_host_and_arm() {
    let entries = [
        ("after_pause", ProofStatus::Proved),
        ("guarded_immediate", ProofStatus::Proved),
        ("guarded_resume", ProofStatus::Proved),
        ("only_the_first_poll_is_checked", ProofStatus::Proved),
        ("shared_slots", ProofStatus::Proved),
        ("nested_futures", ProofStatus::Proved),
        ("captures_preserve_mutable_storage", ProofStatus::Proved),
        ("a_real_context_constructor", ProofStatus::Proved),
        ("cancellation_preserves_drop_effects", ProofStatus::Proved),
        ("bad_immediate", ProofStatus::Refuted),
        ("a_later_panic_is_reachable", ProofStatus::Refuted),
        ("resuming_after_completion_panics", ProofStatus::Refuted),
        ("a_wrong_post_resume_write", ProofStatus::Refuted),
        ("context_observation_is_unknown", ProofStatus::Unknown),
        ("unbounded_polling", ProofStatus::Unknown),
        ("cancellation_checks_the_destructor", ProofStatus::Refuted),
        ("an_indirect_awaited_call_is_unknown", ProofStatus::Unknown),
        ("after_pause::{closure#0}", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized);
        }
    }
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
fn broken_saved_values_refute_and_panic_natively() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    for (original, mutation, name) in [
        (
            "assert!(saved == value);",
            "assert!(saved != value);",
            "shared_slots",
        ),
        (
            "*target = 4;",
            "*target = 3;",
            "captures_preserve_mutable_storage",
        ),
        ("if value < 6 {", "if value <= 6 {", "guarded_resume"),
        (
            "assert!(hits.get() == 1);",
            "assert!(hits.get() == 2);",
            "cancellation_preserves_drop_effects",
        ),
    ] {
        assert!(source.contains(original));
        let directory = Directory::new();
        let path = directory.0.join("mutant.rs");
        std::fs::write(&path, source.replace(original, mutation)).unwrap();
        for target in [None, Some("thumbv7em-none-eabihf")] {
            verify(&path, &[(name, ProofStatus::Refuted)], target, false);
        }
        assert!(
            !native_tests(&path, &directory),
            "the mutation must panic natively"
        );
    }
}

#[test]
fn polls_and_saved_values_replay_natively() {
    assert!(native_tests(&fixture(), &Directory::new()));
}

#[test]
fn a_binary_main_checks_the_polled_body_and_refutes_a_bad_index() {
    let fixture = fixture().with_file_name("async_main.rs");
    let source = std::fs::read_to_string(&fixture).unwrap();
    for (contents, expected) in [
        (source.clone(), ProofStatus::Proved),
        (
            source.replace("samples[2]", "samples[3]"),
            ProofStatus::Refuted,
        ),
    ] {
        let directory = Directory::new();
        let path = directory.0.join("async_main.rs");
        std::fs::write(&path, contents).unwrap();
        for optimized in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
            command
                .args(["--verify", "--quiet", "--json", "--entry", "main", "--"])
                .args(["--crate-type=bin", "--edition=2024"])
                .arg(&path)
                .arg("--out-dir")
                .arg(&directory.0)
                .args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
            if optimized {
                command.arg("-Copt-level=2");
            }
            let output = command.output().unwrap();
            let report: Report = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
                panic!("{error}: {}", String::from_utf8_lossy(&output.stderr))
            });
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == "main")
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(proof.status, expected, "{:?}", proof.obligations);
            assert_eq!(output.status.success(), expected == ProofStatus::Proved);
            assert!(proof.trusted_calls.is_empty());
            if expected == ProofStatus::Proved {
                assert!(
                    proof
                        .analyzed_bodies
                        .iter()
                        .any(|name| name.starts_with("measure::{closure"))
                );
            }
        }
        let compiler = Path::new(env!("MIR_CHECK_SYSROOT")).join("bin/rustc");
        let executable = directory.0.join("native-main");
        assert!(
            Command::new(compiler)
                .args(["--edition=2024", "-Aunconditional_panic"])
                .arg(&path)
                .arg("-o")
                .arg(&executable)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert_eq!(
            Command::new(executable).output().unwrap().status.success(),
            expected == ProofStatus::Proved
        );
    }
}
