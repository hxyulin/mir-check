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
            std::env::temp_dir().join(format!("miren-startup-atomics-{}-{id}", std::process::id()));
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/startup_atomics.rs")
}

fn verify(
    path: &Path,
    entries: &[(&str, ProofStatus)],
    target: Option<&str>,
    optimized: bool,
    startup: bool,
    allow: bool,
    extra: &[&str],
) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
    command.args(["--verify", "--json", "--quiet"]);
    if startup {
        command.arg("--startup");
    }
    if allow {
        command.arg("--allow-assumptions");
    }
    command.args(extra);
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    command
        .args([
            "--",
            "--crate-name=startup_atomics",
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
        entries
            .iter()
            .all(|(_, status)| matches!(status, ProofStatus::Proved)
                || (allow && *status == ProofStatus::ProvedWithAssumptions))
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
        assert_eq!(proof.entry_assumptions.len(), if startup { 2 } else { 0 });
        if *expected == ProofStatus::Proved || *expected == ProofStatus::ProvedWithAssumptions {
            assert!(proof.trusted_calls.is_empty());
        }
        if *expected == ProofStatus::Refuted {
            assert!(proof.obligations.iter().any(|o| o.model.is_some()));
        }
    }
    report
}

#[test]
fn fresh_startup_verifies_initializers_and_tracks_calls_branches_and_disjoint_locations() {
    let entries = [
        ("first_claim", ProofStatus::ProvedWithAssumptions),
        (
            "distinct_static_locations_do_not_share_history",
            ProofStatus::ProvedWithAssumptions,
        ),
        (
            "spurious_failure_paths_keep_independent_histories",
            ProofStatus::ProvedWithAssumptions,
        ),
        (
            "callback_updates_keep_static_history",
            ProofStatus::ProvedWithAssumptions,
        ),
        (
            "overlapping_views_do_not_keep_independent_histories",
            ProofStatus::Refuted,
        ),
        (
            "a_static_store_invalidates_startup_history",
            ProofStatus::Refuted,
        ),
        ("repeated_claim_panics", ProofStatus::Refuted),
        ("an_occupied_initializer_panics", ProofStatus::Refuted),
        ("weak_claim_can_fail_spuriously", ProofStatus::Refuted),
        (
            "argument_roots_need_a_separate_startup_domain",
            ProofStatus::Unknown,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized, true, true, &[]);
        }
        verify(
            &fixture(),
            &[("first_claim", ProofStatus::Refuted)],
            target,
            false,
            false,
            false,
            &[],
        );
        verify(
            &fixture(),
            &[("first_claim", ProofStatus::ProvedWithAssumptions)],
            target,
            false,
            true,
            false,
            &[],
        );
    }
}

#[test]
fn opaque_boundaries_invalidate_known_and_unseen_static_history() {
    let directory = Directory::new();
    let contracts = directory.0.join("contracts.json");
    std::fs::write(&contracts, serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "functions": [{
            "function": "startup_atomics::boundary", "trusted": true,
            "no_panic": true, "modifies": [], "reason": "Independent opaque publication boundary"
        }]
    })).unwrap()).unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let report = verify(
            &fixture(),
            &[
                (
                    "a_boundary_invalidates_existing_history",
                    ProofStatus::Refuted,
                ),
                (
                    "a_boundary_cannot_restore_an_unseen_initializer",
                    ProofStatus::Refuted,
                ),
            ],
            target,
            false,
            true,
            true,
            &["--contracts", contracts.to_str().unwrap()],
        );
        for proof in report
            .functions
            .iter()
            .filter_map(|function| function.proof.as_ref())
        {
            assert_eq!(proof.trusted_calls.len(), 1);
            assert!(proof.obligations.iter().any(|obligation| {
                obligation
                    .abstraction_reasons
                    .iter()
                    .any(|reason| reason.contains("shared atomic reads allow arbitrary old values"))
            }));
        }
    }
}

#[test]
fn initializer_mutation_refutes_and_native_runs_confirm_both_startup_outcomes() {
    let directory = Directory::new();
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "static FIRST: AtomicU16 = AtomicU16::new(0);";
    assert!(source.contains(original));
    let mutant = directory.0.join("mutant.rs");
    std::fs::write(
        &mutant,
        source.replacen(original, "static FIRST: AtomicU16 = AtomicU16::new(1);", 1),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &mutant,
            &[("first_claim", ProofStatus::Refuted)],
            target,
            false,
            true,
            true,
            &[],
        );
    }
    for (path, entry, expected) in [
        (fixture(), "first_claim", true),
        (mutant, "first_claim", false),
        (fixture(), "repeated_claim_panics", false),
    ] {
        let caller = directory.0.join("caller.rs");
        std::fs::write(
            &caller,
            format!(
                "#![feature(custom_mir, core_intrinsics, sync_unsafe_cell)]\n\
             #[path = {:?}] mod application; fn main() {{ application::{entry}(); }}",
                path,
            ),
        )
        .unwrap();
        let compiler = Path::new(env!("MIREN_SYSROOT")).join("bin/rustc");
        let binary = directory.0.join("native");
        let build = Command::new(compiler)
            .arg("--edition=2024")
            .arg(&caller)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(build.status.success(), "{build:?}");
        assert_eq!(
            Command::new(binary).output().unwrap().status.success(),
            expected
        );
    }
}

#[test]
fn startup_requires_verification_and_declines_unsupported_inductive_storage() {
    let output = Command::new(env!("CARGO_BIN_EXE_miren"))
        .args(["--startup", "--", "--crate-type=lib"])
        .arg(fixture())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--startup requires --verify"));
    let report = verify(
        &fixture(),
        &[("unsupported_startup_loop", ProofStatus::Unknown)],
        None,
        false,
        true,
        true,
        &["--induction"],
    );
    assert!(
        report
            .functions
            .iter()
            .filter_map(|function| function.proof.as_ref())
            .flat_map(|proof| &proof.obligations)
            .any(|obligation| obligation.detail.contains("not yet supported by induction"))
    );
}

#[test]
fn cargo_forwards_the_explicit_startup_domain_and_saved_reports_keep_its_assumptions() {
    let directory = Directory::new();
    std::fs::create_dir(directory.0.join("src")).unwrap();
    std::fs::write(
        directory.0.join("Cargo.toml"),
        "[package]\nname='startup-atoms'\nversion='0.0.0'\nedition='2024'\n[workspace]\n",
    )
    .unwrap();
    std::fs::copy(fixture(), directory.0.join("src/lib.rs")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-miren"))
        .args([
            "--verify",
            "--startup",
            "--allow-assumptions",
            "--quiet",
            "--no-dependency-mir",
            "--entry",
            "first_claim",
            "--",
            "--manifest-path",
        ])
        .arg(directory.0.join("Cargo.toml"))
        .current_dir(&directory.0)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("PROVED_WITH_ASSUMPTIONS first_claim"));
    assert!(text.contains("entry assumption: Fresh startup"));
    let reports = text
        .lines()
        .find_map(|line| line.strip_prefix("JSON reports: "))
        .unwrap();
    let path = std::fs::read_dir(reports)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let reader = Command::new(env!("CARGO_BIN_EXE_miren"))
        .args(["report", "--allow-assumptions", "--quiet"])
        .arg(path)
        .output()
        .unwrap();
    assert!(reader.status.success(), "{reader:?}");
    assert!(String::from_utf8_lossy(&reader.stdout).contains("entry assumption: Fresh startup"));
}
