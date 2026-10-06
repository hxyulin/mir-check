#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::Path;
use std::process::Command;

#[test]
fn integer_intrinsics_preserve_signed_bounds_bit_layouts_and_zero_counts_on_host_and_arm() {
    let entries = [
        ("ticket_u8", ProofStatus::Proved),
        ("ticket_u16", ProofStatus::Proved),
        ("ticket_u32", ProofStatus::Proved),
        ("ticket_u64", ProofStatus::Proved),
        ("ticket_u128", ProofStatus::Proved),
        ("ticket_usize", ProofStatus::Proved),
        ("balance_i8", ProofStatus::Proved),
        ("balance_i16", ProofStatus::Proved),
        ("balance_i32", ProofStatus::Proved),
        ("balance_i64", ProofStatus::Proved),
        ("balance_i128", ProofStatus::Proved),
        ("balance_isize", ProofStatus::Proved),
        ("leading_scan_matches_masks", ProofStatus::Proved),
        ("trailing_scan_matches_masks", ProofStatus::Proved),
        ("wide_signed_boundary", ProofStatus::Proved),
        ("wide_unsigned_boundary", ProofStatus::Proved),
        ("transformed_word_matches_byte_layout", ProofStatus::Proved),
        ("changed_saturation_result", ProofStatus::Refuted),
        ("changed_minimum_direction", ProofStatus::Refuted),
        ("changed_zero_count", ProofStatus::Refuted),
        ("unsupported_function_pointer", ProofStatus::Unknown),
    ];
    let directory = std::env::temp_dir().join(format!("mir-check-integer-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/integer_intrinsics.rs");
    for target in [None, Some("thumbv7em-none-eabihf")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
        command.args(["--verify", "--json"]);
        for (entry, _) in entries {
            command.args(["--entry", entry]);
        }
        command
            .args(["--", "--crate-type=lib", "--edition=2024"])
            .arg(&fixture)
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
        for intrinsic in [
            "integer_min",
            "integer_max",
            "saturating_add",
            "saturating_sub",
            "ctlz",
            "cttz",
            "bswap",
            "bitreverse",
        ] {
            let prefix = format!("core::intrinsics::{intrinsic}:");
            assert!(
                report
                    .functions
                    .iter()
                    .filter_map(|function| function.proof.as_ref())
                    .flat_map(|proof| &proof.models)
                    .any(|model| model.starts_with(&prefix)),
                "{target:?}: missing {intrinsic} model",
            );
        }
    }
    let executable = directory.join("runtime");
    let output = Command::new("rustc")
        .args(["--test", "--edition=2024"])
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
    let output = Command::new(executable).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    std::fs::remove_dir_all(directory).unwrap();
}
