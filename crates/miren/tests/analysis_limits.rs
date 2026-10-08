#![forbid(unsafe_code)]

use miren::{AnalysisLimits, ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("miren-limits-{}-{id}", std::process::id()));
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/analysis_limits.rs")
}

fn check(path: &Path, entry: &str, flags: &[&str], target: Option<&str>) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
    command
        .args(["--verify", "--json", "--quiet", "--entry", entry])
        .args(flags)
        .args(["--", "--crate-type=lib", "--edition=2024"])
        .arg(path)
        .arg("--out-dir")
        .arg(&directory.0)
        .args(["-Cpanic=abort", "-Coverflow-checks=yes"])
        .env_remove("MIREN_LIMITS");
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    let report: Report = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
    let status = proof(&report, entry).status;
    assert_eq!(output.status.success(), status == ProofStatus::Proved);
    report
}

fn proof<'a>(report: &'a Report, entry: &str) -> &'a miren::Proof {
    report
        .functions
        .iter()
        .find(|f| f.name == entry)
        .unwrap()
        .proof
        .as_ref()
        .unwrap()
}

#[test]
fn larger_budgets_complete_bounded_work_and_keep_real_failures_on_host_and_arm() {
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for (entry, flags, expected) in [
            ("completed_batch", vec![], ProofStatus::Proved),
            (
                "completed_batch",
                vec!["--max-steps", "16"],
                ProofStatus::Unknown,
            ),
            ("deep_checked", vec![], ProofStatus::Unknown),
            (
                "deep_checked",
                vec!["--max-call-depth=32"],
                ProofStatus::Proved,
            ),
            (
                "later_failure",
                vec!["--max-steps=16"],
                ProofStatus::Unknown,
            ),
            (
                "later_failure",
                vec!["--max-steps=4096"],
                ProofStatus::Refuted,
            ),
            (
                "unsupported_callback",
                vec!["--max-call-depth=64"],
                ProofStatus::Unknown,
            ),
        ] {
            let report = check(&fixture(), entry, &flags, target);
            let result = proof(&report, entry);
            assert_eq!(
                result.status, expected,
                "{target:?} {entry}: {:?}",
                result.obligations
            );
            assert!(result.trusted_calls.is_empty());
        }
        let low = check(
            &fixture(),
            "completed_batch",
            &["--max-query-bytes=30"],
            target,
        );
        assert_eq!(proof(&low, "completed_batch").status, ProofStatus::Unknown);
        assert!(
            proof(&low, "completed_batch")
                .obligations
                .iter()
                .any(|o| { o.detail.contains("query size limit") })
        );
    }
}

#[test]
fn configured_timeouts_reach_ordinary_and_horn_queries_and_root_budgets() {
    let flags = [
        "--solver-timeout-ms=1234",
        "--root-timeout-secs=2",
        "--max-query-bytes=300000",
    ];
    let ordinary = check(&fixture(), "later_failure", &flags, None);
    let limits = ordinary.analysis_limits.unwrap();
    assert_eq!(limits.solver_timeout_ms, 1234);
    assert_eq!(limits.root_timeout_secs, 2);
    assert_eq!(limits.max_query_bytes, 300_000);
    assert_eq!(
        proof(&ordinary, "later_failure").status,
        ProofStatus::Refuted
    );
    assert!(
        proof(&ordinary, "later_failure")
            .obligations
            .iter()
            .filter_map(|o| o.query.as_ref())
            .all(|q| q.contains("(set-option :timeout 1234)"))
    );
    let horn = check(
        &fixture(),
        "never_finishes",
        &["--induction", "--solver-timeout-ms=1234"],
        None,
    );
    assert_eq!(proof(&horn, "never_finishes").status, ProofStatus::Proved);
    assert!(
        proof(&horn, "never_finishes")
            .obligations
            .iter()
            .filter_map(|o| o.query.as_ref())
            .any(|q| q.contains("(set-option :timeout 1234)"))
    );
    let tiny = check(
        &fixture(),
        "never_finishes",
        &["--induction", "--max-query-bytes=30"],
        None,
    );
    assert_eq!(proof(&tiny, "never_finishes").status, ProofStatus::Unknown);
    let timed = check(
        &fixture(),
        "never_finishes",
        &["--root-timeout-secs=1", "--max-steps=1000000000"],
        None,
    );
    assert!(proof(&timed, "never_finishes").obligations.iter().any(|o| {
        o.status == ProofStatus::Unknown && o.detail.contains("1-second execution budget")
    }));
}

#[test]
fn invalid_limits_fail_before_compilation_in_both_clis() {
    for executable in [
        env!("CARGO_BIN_EXE_miren"),
        env!("CARGO_BIN_EXE_cargo-miren"),
    ] {
        for (flag, value) in [
            ("--max-steps", "0"),
            ("--max-call-depth", "-1"),
            ("--max-query-bytes", "lots"),
            ("--root-timeout-secs", "18446744073709551615"),
            ("--solver-timeout-ms", "4294967296"),
        ] {
            let output = Command::new(executable)
                .args([flag, value])
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(
                output.stdout.is_empty(),
                "invalid options must not produce a report"
            );
        }
    }
}

#[test]
fn cargo_forwards_explicit_limits_and_clears_inherited_settings() {
    let directory = Directory::new();
    std::fs::write(
        directory.0.join("Cargo.toml"),
        "[package]\nname = \"limits_sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
         [lib]\npath = \"sample.rs\"\n[workspace]\n",
    )
    .unwrap();
    std::fs::copy(fixture(), directory.0.join("sample.rs")).unwrap();
    for explicit in [false, true] {
        let output_path = directory.0.join("reports.jsonl");
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-miren"));
        command
            .current_dir(&directory.0)
            .args([
                "--verify",
                "--quiet",
                "--entry",
                "completed_batch",
                "--jsonl",
            ])
            .arg(&output_path)
            .arg("--lib")
            .env("MIREN_LIMITS", "not valid JSON");
        if explicit {
            command.args(["--max-steps", "16", "--solver-timeout-ms", "1234"]);
        }
        let output = command.output().unwrap();
        let reports: Vec<Report> = std::fs::read_to_string(output_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let report = reports
            .iter()
            .find(|r| r.crate_name == "limits_sample")
            .unwrap();
        let limits = report.analysis_limits.unwrap();
        if explicit {
            assert!(!output.status.success());
            assert_eq!(limits.max_steps, 16);
            assert_eq!(limits.solver_timeout_ms, 1234);
            assert_eq!(
                proof(report, "completed_batch").status,
                ProofStatus::Unknown
            );
        } else {
            assert!(output.status.success(), "{output:?}");
            assert_eq!(limits, AnalysisLimits::default());
        }
    }
}

#[test]
fn relaxed_limits_expose_a_mutated_callee_and_older_reports_do_not_invent_limits() {
    let directory = Directory::new();
    let source = std::fs::read_to_string(fixture()).unwrap();
    let path = directory.0.join("mutant.rs");
    std::fs::write(
        &path,
        source.replace("assert!(value < 4);", "assert!(value < 3);"),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let report = check(&path, "deep_checked", &["--max-call-depth=32"], target);
        assert_eq!(proof(&report, "deep_checked").status, ProofStatus::Refuted);
    }
    let report = check(&fixture(), "completed_batch", &[], None);
    let mut old = serde_json::to_value(&report).unwrap();
    old.as_object_mut().unwrap().remove("analysis_limits");
    let old: Report = serde_json::from_value(old).unwrap();
    assert!(old.analysis_limits.is_none());
    let compiler = Path::new(env!("MIREN_SYSROOT")).join("bin/rustc");
    let executable = directory.0.join("native");
    for (source, succeeds) in [(fixture(), true), (path, false)] {
        let compiled = Command::new(&compiler)
            .args(["--test", "--edition=2024"])
            .arg(source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(compiled.status.success(), "{compiled:?}");
        assert_eq!(
            Command::new(&executable).output().unwrap().status.success(),
            succeeds
        );
    }
}
