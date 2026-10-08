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
            std::env::temp_dir().join(format!("miren-async-entries-{}-{id}", std::process::id()));
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/async_entries.rs")
}

fn check(
    path: &Path,
    async_entries: &[(&str, ProofStatus)],
    target: Option<&str>,
    optimized: bool,
    extra: &[&str],
) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
    command.args(["--verify", "--json", "--quiet", "--max-steps", "2048"]);
    for (name, _) in async_entries {
        command.args(["--async-entry", name]);
    }
    command
        .args(extra)
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
        async_entries
            .iter()
            .all(|(_, expected)| *expected == ProofStatus::Proved)
            && !extra.contains(&"initialization")
    );
    for (name, expected) in async_entries {
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
        assert!(proof.async_entry);
        assert!(proof.trusted_calls.is_empty());
        assert!(proof.entry_assumptions.is_empty());
        assert!(proof.replay_inputs.is_none());
        if matches!(expected, ProofStatus::Proved | ProofStatus::Refuted) {
            assert!(
                proof
                    .analyzed_bodies
                    .iter()
                    .any(|name| name.contains("{closure#"))
            );
        }
    }
    report
}

#[test]
fn selected_async_factories_check_every_reached_poll_separately_from_initialization() {
    let entries = [
        ("guarded_samples", ProofStatus::Proved),
        ("bounded_pauses", ProofStatus::Proved),
        ("bad_after_pause", ProofStatus::Refuted),
        ("endless_pending", ProofStatus::Unknown),
        ("unsupported_callback", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            let report = check(
                &fixture(),
                &entries,
                target,
                optimized,
                &["--entry", "initialization"],
            );
            assert_eq!(report.coverage.selected_roots, 6);
            assert!(
                miren::cli::render_report(&report, false, false)
                    .contains("guarded_samples (async construction + polls)")
            );
            assert!(
                miren::cli::render_report(&report, true, false)
                    .contains("verification: PROVED (async construction + polls)")
            );
            let mut old_json = serde_json::to_value(&report).unwrap();
            for function in old_json["functions"].as_array_mut().unwrap() {
                if let Some(proof) = function["proof"].as_object_mut() {
                    proof.remove("async_entry");
                }
            }
            let old_report: Report = serde_json::from_value(old_json).unwrap();
            assert!(
                old_report
                    .functions
                    .iter()
                    .filter_map(|function| function.proof.as_ref())
                    .all(|proof| !proof.async_entry)
            );
            let initialization = report
                .functions
                .iter()
                .find(|function| function.name == "initialization")
                .unwrap();
            assert_eq!(
                initialization.proof.as_ref().unwrap().status,
                ProofStatus::Refuted
            );
            let endless = report
                .functions
                .iter()
                .find(|function| function.name == "endless_pending")
                .unwrap();
            assert!(
                endless
                    .proof
                    .as_ref()
                    .unwrap()
                    .obligations
                    .iter()
                    .any(|obligation| obligation.status == ProofStatus::Unknown)
            );
        }
    }
}

#[test]
fn async_entries_reject_startup_induction_and_arbitrary_poll_state() {
    for option in ["--startup", "--induction"] {
        let report = check(
            &fixture(),
            &[("bounded_pauses", ProofStatus::Unknown)],
            None,
            false,
            &[option],
        );
        assert!(
            report
                .functions
                .iter()
                .find(|function| function.name == "bounded_pauses")
                .unwrap()
                .proof
                .as_ref()
                .unwrap()
                .obligations
                .iter()
                .any(|obligation| obligation.detail.contains("check separately"))
        );
    }
    check(
        &fixture(),
        &[("guarded_samples::{closure#0}", ProofStatus::Unknown)],
        None,
        false,
        &[],
    );
    let report = check(
        &fixture(),
        &[],
        None,
        false,
        &["--entry", "bad_after_pause"],
    );
    let factory = report
        .functions
        .iter()
        .find(|function| function.name == "bad_after_pause")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(factory.status, ProofStatus::Proved);
    assert!(!factory.async_entry);
    assert!(
        !factory
            .analyzed_bodies
            .iter()
            .any(|name| name.contains("{closure#"))
    );
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
fn a_resumed_bounds_mutation_refutes_and_panics_natively() {
    assert!(native_tests(&fixture(), &Directory::new()));
    let source = std::fs::read_to_string(fixture()).unwrap();
    assert!(source.contains("if index < 4 {"));
    let directory = Directory::new();
    let path = directory.0.join("mutation.rs");
    std::fs::write(
        &path,
        source.replacen("if index < 4 {", "if index <= 4 {", 1),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        check(
            &path,
            &[("guarded_samples", ProofStatus::Refuted)],
            target,
            false,
            &[],
        );
    }
    assert!(!native_tests(&path, &directory));
}

#[test]
fn conflicting_entry_scopes_and_missing_async_selectors_fail() {
    for options in [
        vec![
            "--entry",
            "guarded_samples",
            "--async-entry",
            "guarded_samples",
        ],
        vec![
            "--entry",
            "async_entries::guarded_samples",
            "--async-entry",
            "guarded_samples",
        ],
        vec![
            "--entry",
            "guarded_samples",
            "--async-entry=async_entries::guarded_samples",
        ],
    ] {
        let directory = Directory::new();
        let output = Command::new(env!("CARGO_BIN_EXE_miren"))
            .args(["--verify", "--quiet"])
            .args(options)
            .args(["--", "--crate-type=lib", "--edition=2024"])
            .arg(fixture())
            .arg("--out-dir")
            .arg(&directory.0)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("selected by both --entry and --async-entry")
        );
    }
    let directory = Directory::new();
    let output = Command::new(env!("CARGO_BIN_EXE_miren"))
        .args(["--verify", "--quiet", "--async-entry", "absent_task", "--"])
        .args(["--crate-type=lib", "--edition=2024"])
        .arg(fixture())
        .arg("--out-dir")
        .arg(&directory.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("has no inventoried local MIR body"));
}
