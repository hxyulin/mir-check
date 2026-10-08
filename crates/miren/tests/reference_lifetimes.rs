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
            "miren-reference-lifetimes-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/reference_lifetimes.rs")
}

fn verify(
    path: &Path,
    entries: &[(&str, ProofStatus)],
    target: Option<&str>,
    optimized: bool,
    specification: Option<serde_json::Value>,
) -> Report {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_miren"));
    command.args(["--verify", "--json", "--quiet", "--allow-assumptions"]);
    if let Some(specification) = specification {
        let config = directory.0.join("contracts.json");
        std::fs::write(&config, serde_json::to_vec(&specification).unwrap()).unwrap();
        command.arg("--contracts").arg(config);
    }
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    let crate_type = if entries.iter().any(|(name, _)| *name == "main") {
        "--crate-type=bin"
    } else {
        "--crate-type=lib"
    };
    command
        .args([
            "--",
            "--crate-name=reference_lifetimes",
            crate_type,
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
        entries.iter().all(|(_, status)| matches!(
            status,
            ProofStatus::Proved | ProofStatus::ProvedWithAssumptions
        ))
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
        if *expected == ProofStatus::Proved {
            assert!(proof.trusted_calls.is_empty());
        }
        if *expected == ProofStatus::Refuted {
            assert!(proof.obligations.iter().any(|o| o.model.is_some()));
        }
    }
    report
}

#[test]
fn lifetime_casts_preserve_tracked_writes_and_reject_escaping_or_incompatible_references() {
    let entries = [
        ("writes_reach_original", ProofStatus::Proved),
        ("projected_storage", ProofStatus::Proved),
        ("aggregate_storage", ProofStatus::Proved),
        ("wrong_original_value", ProofStatus::Refuted),
        ("local_borrow_cannot_escape", ProofStatus::Unknown),
        ("dead_storage_is_not_retained", ProofStatus::Unknown),
        ("different_pointee", ProofStatus::Unknown),
        ("different_mutability", ProofStatus::Unknown),
        ("raw_pointer_is_not_a_reference", ProofStatus::Unknown),
        (
            "shared_snapshot_is_not_a_tracked_reference",
            ProofStatus::Unknown,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized, None);
        }
    }
}

fn alias_specification() -> serde_json::Value {
    serde_json::json!({"schema_version":1,"functions":[{
        "function":"reference_lifetimes::selected_alias",
        "arguments":["left","right"],"trusted":true,"no_panic":true,
        "modifies":[],"returns_alias":"left","ensures":["result == left"],
        "reason":"Explicit test claim: returns the left argument without modifying storage"
    }]})
}

#[test]
fn explicit_return_aliases_remain_conditional_and_do_not_allow_local_borrows_to_escape() {
    let entries = [
        ("assumed_alias", ProofStatus::ProvedWithAssumptions),
        ("bad_assumed_alias", ProofStatus::Refuted),
        ("assumed_escape", ProofStatus::Unknown),
        ("selected_alias", ProofStatus::Proved),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            let report = verify(
                &fixture(),
                &entries,
                target,
                optimized,
                Some(alias_specification()),
            );
            let display = miren::cli::render_report(&report, false, false);
            assert_eq!(
                display
                    .matches("    assumes reference_lifetimes::selected_alias:")
                    .count(),
                3
            );
            assert!(!miren::cli::accepted(&report, true));
            for name in ["assumed_alias", "bad_assumed_alias", "assumed_escape"] {
                let proof = report
                    .functions
                    .iter()
                    .find(|f| f.name == name)
                    .unwrap()
                    .proof
                    .as_ref()
                    .unwrap();
                assert_eq!(proof.trusted_calls.len(), 1);
                assert_eq!(
                    proof.trusted_calls[0].contract.returns_alias.as_deref(),
                    Some("left")
                );
            }
        }
    }
}

#[test]
fn omitted_memory_effects_and_incompatible_alias_returns_are_unknown() {
    let mut invalidates = alias_specification();
    invalidates["functions"][0]
        .as_object_mut()
        .unwrap()
        .remove("modifies");
    verify(
        &fixture(),
        &[("assumed_alias", ProofStatus::Unknown)],
        None,
        false,
        Some(invalidates),
    );
    let mut incompatible = alias_specification();
    incompatible["functions"][0]["function"] =
        serde_json::json!("reference_lifetimes::incompatible_alias");
    incompatible["functions"][0]["arguments"] = serde_json::json!(["left"]);
    verify(
        &fixture(),
        &[("incompatible_caller", ProofStatus::Unknown)],
        None,
        false,
        Some(incompatible),
    );
}

#[test]
fn return_alias_predicates_observe_the_claimed_post_state_and_preserve_other_storage() {
    let mut specification = alias_specification();
    specification["functions"][0]["modifies"] = serde_json::json!(["left"]);
    specification["functions"][0]["ensures"] =
        serde_json::json!(["result == 31", "final_left == 31"]);
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &fixture(),
            &[("assumed_post_state", ProofStatus::ProvedWithAssumptions)],
            target,
            false,
            Some(specification.clone()),
        );
    }
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
fn mutated_writes_refute_and_panic_natively() {
    let source = std::fs::read_to_string(fixture()).unwrap();
    let directory = Directory::new();
    let path = directory.0.join("mutant.rs");
    std::fs::write(&path, source.replace("*alias = 23;", "*alias = 22;")).unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &path,
            &[("writes_reach_original", ProofStatus::Refuted)],
            target,
            false,
            None,
        );
    }
    assert!(!native_tests(&path, &directory));
}

#[test]
fn only_scoped_valid_aliases_replay_natively() {
    assert!(native_tests(&fixture(), &Directory::new()));
}

#[test]
fn binary_assumptions_report_missing_hashes_without_crashing_the_compiler() {
    let directory = Directory::new();
    let path = directory.0.join("binary.rs");
    let source = std::fs::read_to_string(fixture())
        .unwrap()
        .replace("#![no_std]", "");
    std::fs::write(
        &path,
        format!("{source}\nfn main() {{ assumed_alias(); }}\n"),
    )
    .unwrap();
    let report = verify(
        &path,
        &[("main", ProofStatus::ProvedWithAssumptions)],
        None,
        false,
        Some(alias_specification()),
    );
    let proof = report
        .functions
        .iter()
        .find(|f| f.name == "main")
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert_eq!(proof.trusted_calls.len(), 1);
    assert!(proof.trusted_calls[0].crate_hash.is_empty());
    let json = serde_json::to_value(&report).unwrap();
    let function = json["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "main")
        .unwrap();
    assert!(
        function["proof"]["trusted_calls"][0]
            .get("crate_hash")
            .is_none()
    );
    let restored: Report = serde_json::from_value(json).unwrap();
    assert!(miren::render(&restored).contains("hash unavailable for this build"));
}
