#![forbid(unsafe_code)]

use mir_check::{Proof, ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/contract_work.rs")
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

fn check(path: &Path, entries: &[&str], target: Option<&str>, config: Option<&Path>) -> Report {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet"]);
    for entry in entries {
        command.args(["--entry", entry]);
    }
    if let Some(config) = config {
        command.arg("--contracts").arg(config);
    }
    command
        .args([
            "--",
            "--crate-type=lib",
            "--crate-name=contract_work",
            "--edition=2024",
            "-Coverflow-checks=yes",
            "-Cpanic=abort",
        ])
        .arg(path);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().unwrap();
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn ordinary_calls_skip_unused_names_and_hints_check_both_independent_outcomes() {
    let expected = [
        ("bounded_hint", ProofStatus::Proved),
        ("unchecked_hint", ProofStatus::Refuted),
        ("independent_hints", ProofStatus::Refuted),
        ("unrelated_hint", ProofStatus::Refuted),
        ("hint_pointer", ProofStatus::Unknown),
        ("guarded_power", ProofStatus::Unknown),
    ];
    let entries = expected.map(|(name, _)| name);
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let report = check(&fixture(), &entries, target, None);
        for (name, status) in expected {
            let proof = proof(&report, name);
            assert_eq!(proof.status, status, "{target:?} {name}");
            assert!(proof.trusted_calls.is_empty());
            assert!(proof.assumptions.is_empty());
        }
        let safe = proof(&report, "bounded_hint");
        assert!(safe.models.iter().any(|name| {
            name.ends_with("static-value optimization hint; independent Boolean per call")
        }));
        assert!(
            !proof(&report, "unrelated_hint")
                .models
                .iter()
                .any(|name| { name.contains("static-value optimization hint") })
        );
        let power = proof(&report, "guarded_power");
        assert!(
            power
                .analyzed_bodies
                .iter()
                .any(|body| body.starts_with("core::num::<impl u8>::checked_pow "))
        );
        assert!(
            power
                .obligations
                .iter()
                .any(|obligation| { obligation.detail.contains("unsupported transmute from u8") })
        );
        assert!(
            !power
                .obligations
                .iter()
                .any(|obligation| { obligation.detail.contains("ambiguous argument name") })
        );
    }
}

#[test]
fn declared_aliases_and_predicates_remain_checked_at_the_call_boundary() {
    let directory =
        std::env::temp_dir().join(format!("mir-check-contract-work-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let config = directory.join("contracts.json");
    let cases = [
        (
            serde_json::json!({"arguments":["left","right"], "requires":["left < 255"],
                "ensures":["result.0 == left", "result.1 == right"]}),
            ProofStatus::Refuted,
            "requires left < 255",
        ),
        (
            serde_json::json!({"arguments":["left","right"],
                "ensures":["result.0 == left", "result.1 == right"]}),
            ProofStatus::Proved,
            "result.0 == left",
        ),
        (
            serde_json::json!({"arguments":["left","right"], "ensures":["result.0 > left"]}),
            ProofStatus::Refuted,
            "result.0 > left",
        ),
        (
            serde_json::json!({"arguments":["only_one"]}),
            ProofStatus::Unknown,
            "contract argument count",
        ),
        (
            serde_json::json!({"arguments":["second","tail"]}),
            ProofStatus::Unknown,
            "argument alias conflicts",
        ),
    ];
    for (mut spec, status, detail) in cases {
        spec["function"] = serde_json::json!("contract_work::alias_pair");
        spec["no_panic"] = serde_json::json!(true);
        std::fs::write(
            &config,
            serde_json::to_vec(&serde_json::json!({"schema_version":1,"functions":[spec]}))
                .unwrap(),
        )
        .unwrap();
        let report = check(&fixture(), &["calls_alias_pair"], None, Some(&config));
        let proof = proof(&report, "calls_alias_pair");
        assert_eq!(proof.status, status, "{detail}: {proof:?}");
        assert!(
            proof
                .obligations
                .iter()
                .any(|obligation| { obligation.detail.contains(detail) })
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn optimization_paths_match_native_execution_and_a_changed_guard_exposes_overflow() {
    let directory =
        std::env::temp_dir().join(format!("mir-check-hint-native-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("native");
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
    assert!(Command::new(&executable).output().unwrap().status.success());
    let source = std::fs::read_to_string(fixture()).unwrap();
    let before = "if value < u8::MAX";
    assert_eq!(source.matches(before).count(), 1);
    let path = directory.join("mutant.rs");
    std::fs::write(&path, source.replace(before, "if value <= u8::MAX")).unwrap();
    let report = check(&path, &["bounded_hint"], None, None);
    let proof = proof(&report, "bounded_hint");
    assert_eq!(proof.status, ProofStatus::Refuted);
    assert!(
        proof
            .obligations
            .iter()
            .any(|obligation| obligation.model.is_some())
    );
    std::fs::remove_dir_all(directory).unwrap();
}
