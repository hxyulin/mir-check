#![forbid(unsafe_code)]

use miren::{Proof, ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/first_counterexample.rs")
}

fn check(path: &Path, target: Option<&str>, all_failures: bool) -> Report {
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
    command.args(["--verify", "--json", "--quiet"]);
    if all_failures {
        command.arg("--all-failures");
    }
    command
        .args(["--", "--crate-type=lib", "--edition=2024"])
        .arg(path)
        .args(["-Coverflow-checks=yes", "-Cpanic=abort"]);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    assert!(!output.status.success());
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)))
}

fn proof<'a>(report: &'a Report, name: &str) -> &'a Proof {
    report
        .functions
        .iter()
        .find(|function| function.name == name)
        .unwrap()
        .proof
        .as_ref()
        .unwrap()
}

#[test]
fn a_counterexample_stops_only_its_root_and_exhaustive_mode_keeps_later_failures() {
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let first = check(&fixture(), target, false);
        let all = check(&fixture(), target, true);
        assert_eq!(first.coverage.selected_roots, 4);
        assert_eq!(first.coverage.proved, 1);
        assert_eq!(first.coverage.refuted, 2);
        assert_eq!(first.coverage.unknown, 1);
        for name in ["two_failures", "later_failure"] {
            let short = proof(&first, name);
            let long = proof(&all, name);
            assert_eq!(short.status, ProofStatus::Refuted);
            assert_eq!(long.status, short.status);
            assert!(short.stopped_after_counterexample);
            assert!(!long.stopped_after_counterexample);
            assert_eq!(short.obligations.len(), 1);
            assert!(short.obligations[0].model.is_some());
            assert_eq!(
                long.obligations
                    .iter()
                    .filter(|obligation| obligation.status == ProofStatus::Refuted)
                    .count(),
                2
            );
            assert_eq!(short.obligations[0].query, long.obligations[0].query);
        }
        for name in ["completed_batch", "unsupported_callback"] {
            assert_eq!(
                serde_json::to_value(proof(&first, name)).unwrap(),
                serde_json::to_value(proof(&all, name)).unwrap()
            );
            assert!(!proof(&first, name).stopped_after_counterexample);
        }
        assert!(miren::render(&first).contains("stopped after first counterexample"));
        assert!(miren::cli::render_report(&first, false, false).contains("stopped after first"));
        let mut legacy = serde_json::to_value(&first).unwrap();
        for function in legacy["functions"].as_array_mut().unwrap() {
            function["proof"]
                .as_object_mut()
                .unwrap()
                .remove("stopped_after_counterexample");
        }
        let legacy: Report = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.coverage.refuted, 2);
        assert!(!proof(&legacy, "two_failures").stopped_after_counterexample);
    }
}

#[test]
fn removing_the_early_failure_still_finds_the_failure_after_the_completed_loop() {
    let directory = std::env::temp_dir().join(format!("miren-first-mutant-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = std::fs::read_to_string(fixture()).unwrap();
    let source = source.replace("assert!(label == 0);", "assert!(label == label);");
    let path = directory.join("first_counterexample.rs");
    std::fs::write(&path, &source).unwrap();
    let report = check(&path, None, false);
    let proof = proof(&report, "later_failure");
    assert_eq!(proof.status, ProofStatus::Refuted);
    assert!(proof.stopped_after_counterexample);
    assert!(proof.obligations.len() > 256);
    let last = proof.obligations.last().unwrap();
    assert!(last.model.is_some());
    assert_eq!(
        last.source.line,
        source
            .lines()
            .position(|line| line.contains("assert!(index == 255)"))
            .unwrap()
            + 1
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn early_and_late_counterexamples_replay_under_native_execution() {
    let executable =
        std::env::temp_dir().join(format!("miren-first-native-{}", std::process::id()));
    let output = Command::new("rustc")
        .args(["--test", "--edition=2024", "-Coverflow-checks=yes"])
        .arg(fixture())
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = Command::new(&executable).output().unwrap();
    assert!(output.status.success());
    std::fs::remove_file(executable).unwrap();
}

#[test]
fn cargo_forwards_the_failure_policy_without_inheriting_a_previous_runs_setting() {
    let directory = std::env::temp_dir().join(format!("miren-first-cargo-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("src")).unwrap();
    std::fs::write(
        directory.join("Cargo.toml"),
        "[package]\nname = \"first-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
         [workspace]\n",
    )
    .unwrap();
    std::fs::copy(fixture(), directory.join("src/lib.rs")).unwrap();
    for all_failures in [false, true] {
        let path = directory.join(if all_failures {
            "all.jsonl"
        } else {
            "first.jsonl"
        });
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-miren"));
        command.args(["--verify", "--quiet", "--entry", "two_failures", "--jsonl"]);
        command.arg(&path);
        if all_failures {
            command.arg("--all-failures");
        }
        let output = command
            .args(["--manifest-path"])
            .arg(directory.join("Cargo.toml"))
            .env("MIREN_ALL_FAILURES", "1")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let report: Report = serde_json::from_str(&std::fs::read_to_string(path).unwrap())
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
        let proof = proof(&report, "two_failures");
        assert_eq!(proof.status, ProofStatus::Refuted);
        assert_eq!(proof.stopped_after_counterexample, !all_failures);
        assert_eq!(proof.obligations.len(), if all_failures { 2 } else { 1 });
    }
    std::fs::remove_dir_all(directory).unwrap();
}
