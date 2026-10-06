#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::Path;
use std::process::Command;

#[test]
fn shared_terms_prove_complete_paths_and_preserve_counterexamples_and_unknowns() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/term_sharing.rs");
    let directory =
        std::env::temp_dir().join(format!("mir-check-term-sharing-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let entries = [
        (
            "the_accumulated_signal_matches_its_scale",
            ProofStatus::Proved,
        ),
        ("an_incorrect_scale_is_detected", ProofStatus::Refuted),
        ("an_unresolved_callback_stays_unknown", ProofStatus::Unknown),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
        command.args(["--verify", "--json"]);
        for (entry, _) in entries {
            command.args(["--entry", entry]);
        }
        command
            .args(["--", "--crate-type=lib", "--edition=2024"])
            .arg(&source)
            .arg("--out-dir")
            .arg(&directory)
            .args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
        if let Some(target) = target {
            command.args(["--target", target]);
        }
        let output = command.output().unwrap();
        let report: Report = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
        for (entry, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|f| f.name == entry)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(proof.status, expected, "{entry}: {:?}", proof.obligations);
            if expected != ProofStatus::Unknown {
                let queries: Vec<_> = proof
                    .obligations
                    .iter()
                    .filter_map(|o| o.query.as_ref())
                    .collect();
                assert!(!queries.is_empty());
                assert!(queries.iter().all(|q| q.len() < 5000), "{entry}");
                if expected == ProofStatus::Refuted {
                    assert!(queries.iter().any(|q| q.contains("(let ((t")), "{entry}");
                }
            }
        }
        assert!(!output.status.success());
    }
    let executable = directory.join("native-tests");
    let output = Command::new("rustc")
        .args(["--test", "--edition=2024"])
        .arg(&source)
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
    assert!(output.status.success(), "{output:?}");
    std::fs::remove_dir_all(directory).unwrap();
}
