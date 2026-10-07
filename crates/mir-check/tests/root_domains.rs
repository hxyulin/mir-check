#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::Path;
use std::process::Command;

#[test]
fn larger_root_domains_preserve_bounds_scalar_validity_and_unknown_limits() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/root_domains.rs");
    let expected = [
        ("spectrum", ProofStatus::Proved),
        ("packed_records", ProofStatus::Proved),
        ("deep_record", ProofStatus::Proved),
        ("label_code", ProofStatus::Proved),
        ("ticket_divisor", ProofStatus::Proved),
        ("signed_ticket", ProofStatus::Proved),
        ("codepoint", ProofStatus::Proved),
        ("wrong_spectrum", ProofStatus::Refuted),
        ("wrong_label", ProofStatus::Refuted),
        ("wrong_signed_ticket", ProofStatus::Refuted),
        ("wrong_codepoint", ProofStatus::Refuted),
        ("custom_get", ProofStatus::Refuted),
        (
            "repeated_fields_beyond_the_eager_budget",
            ProofStatus::Proved,
        ),
        ("too_many_elements", ProofStatus::Unknown),
        ("too_many_variants", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
        command.args(["--verify", "--json", "--quiet"]);
        for (entry, _) in expected {
            command.args(["--entry", entry]);
        }
        command.args(["--", "--crate-type=lib", "--edition=2024"]);
        command
            .arg(&fixture)
            .args(["-Coverflow-checks=yes", "-Cpanic=abort"]);
        if let Some(target) = target {
            command.args(["--target", target]);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        let report: Report = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
        for (entry, status) in expected {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == entry)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(proof.status, status, "{target:?} {entry}");
            if status == ProofStatus::Refuted {
                assert!(proof.obligations.iter().any(|o| o.model.is_some()));
            }
        }
    }
}

#[test]
fn zeroing_a_valid_divisor_refutes_the_root_type_contract() {
    let directory =
        std::env::temp_dir().join(format!("mir-check-root-mutant-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/root_domains.rs");
    let source = std::fs::read_to_string(fixture).unwrap();
    let original = "count / divisor.get()";
    assert_eq!(source.matches(original).count(), 1);
    let changed = directory.join("root_domains.rs");
    std::fs::write(
        &changed,
        source.replace(original, "count / (divisor.get() - 1)"),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mir-check"))
        .args([
            "--verify",
            "--json",
            "--quiet",
            "--entry",
            "ticket_divisor",
            "--",
            "--crate-type=lib",
            "--edition=2024",
        ])
        .arg(&changed)
        .args(["-Coverflow-checks=yes", "-Cpanic=abort"])
        .output()
        .unwrap();
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report
            .functions
            .iter()
            .find(|f| f.name == "ticket_divisor")
            .unwrap()
            .proof
            .as_ref()
            .unwrap()
            .status,
        ProofStatus::Refuted
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn larger_records_unicode_scalars_and_nonzero_values_match_native_execution() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/root_domains.rs");
    let executable =
        std::env::temp_dir().join(format!("mir-check-root-native-{}", std::process::id()));
    let output = Command::new("rustc")
        .args(["--test", "--edition=2024", "-Coverflow-checks=yes"])
        .arg(&fixture)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(&executable).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    std::fs::remove_file(executable).unwrap();
}
