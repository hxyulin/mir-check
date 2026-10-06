#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::Path;
use std::process::Command;

#[test]
fn compiler_identities_distinguish_library_models_from_application_names_on_host_and_arm() {
    let entries = [
        (
            "application_atomic_names_execute_their_bodies",
            ProofStatus::Proved,
        ),
        (
            "application_cell_names_execute_their_bodies",
            ProofStatus::Proved,
        ),
        ("application_cell_result_mutation", ProofStatus::Refuted),
        (
            "application_panic_names_execute_their_bodies",
            ProofStatus::Proved,
        ),
        ("core_orderings_use_the_compiler_enum", ProofStatus::Proved),
        ("invalid_core_ordering_is_refuted", ProofStatus::Refuted),
        (
            "interior_storage_outside_the_model_remains_unknown",
            ProofStatus::Unknown,
        ),
    ];
    let directory = std::env::temp_dir().join(format!("mir-check-identity-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
        command.args(["--verify", "--json"]);
        for (entry, _) in entries {
            command.args(["--entry", entry]);
        }
        command
            .args(["--", "--crate-type=lib", "--edition=2024"])
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tests/fixtures/typed_identity.rs"),
            )
            .arg("--out-dir")
            .arg(&directory)
            .args(["-Cpanic=abort", "-Coverflow-checks=yes"]);
        if let Some(target) = target {
            command.args(["--target", target]);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        let report: Report = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!("{error}: {}", String::from_utf8_lossy(&output.stderr));
        });
        for (entry, expected) in entries {
            let proof = report
                .functions
                .iter()
                .find(|function| function.name == entry)
                .unwrap()
                .proof
                .as_ref()
                .unwrap();
            assert_eq!(
                proof.status, expected,
                "{target:?} {entry}: {:?}",
                proof.obligations
            );
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
