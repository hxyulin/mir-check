#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::Path;
use std::process::Command;

#[test]
fn unused_float_encodings_are_lazy_but_observations_recover_every_required_binding() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/lazy_float.rs");
    let expected = [
        ("gauge_codes", ProofStatus::Proved),
        ("computed_roundtrip", ProofStatus::Proved),
        ("returned_roundtrip", ProofStatus::Proved),
        ("transformed_roundtrip", ProofStatus::Proved),
        ("selected_roundtrip", ProofStatus::Proved),
        ("transitive_roundtrip", ProofStatus::Proved),
        ("stable_copied_encoding", ProofStatus::Proved),
        ("wrong_roundtrip", ProofStatus::Refuted),
        ("unsupported_remainder", ProofStatus::Unknown),
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
            assert_eq!(
                proof.status, status,
                "{target:?} {entry}: {:?}",
                proof.obligations
            );
            if status == ProofStatus::Refuted {
                assert!(proof.obligations.iter().any(|o| o.model.is_some()));
            }
            if entry == "gauge_codes" {
                assert!(proof.obligations.iter().all(|o| {
                    o.query
                        .as_ref()
                        .is_none_or(|q| !q.contains("(= ((_ to_fp 8 24) v"))
                }));
            }
        }
    }
}

#[test]
fn synthetic_gauges_and_bit_roundtrips_replay_with_native_rust() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/lazy_float.rs");
    let binary = std::env::temp_dir().join(format!(
        "mir-check-lazy-float-native-{}",
        std::process::id()
    ));
    let compile = Command::new("rustc")
        .args(["--test", "--edition=2024"])
        .arg(fixture)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let replay = Command::new(&binary).output().unwrap();
    std::fs::remove_file(binary).unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stdout)
    );
}
