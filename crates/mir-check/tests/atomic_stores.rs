#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mir-check-atomic-stores-{}-{id}",
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/atomic_stores.rs")
}

fn verify(path: &Path, entries: &[(&str, ProofStatus)], target: Option<&str>, optimized: bool) {
    let directory = Directory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mir-check"));
    command.args(["--verify", "--json", "--quiet"]);
    for (name, _) in entries {
        command.args(["--entry", name]);
    }
    command
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
        entries
            .iter()
            .all(|(_, status)| *status == ProofStatus::Proved)
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
        assert!(proof.trusted_calls.is_empty());
        if *expected == ProofStatus::Refuted {
            assert!(proof.obligations.iter().any(|o| o.model.is_some()));
        }
    }
}

#[test]
fn atomic_stores_check_ordering_provenance_types_and_escape_on_host_and_arm() {
    let entries = [
        ("numeric_handles_can_be_published", ProofStatus::Proved),
        (
            "an_initialized_integer_prefix_can_receive_an_atomic_store",
            ProofStatus::Proved,
        ),
        (
            "caller_storage_keeps_its_reference_evidence",
            ProofStatus::Proved,
        ),
        ("guarded_ordering", ProofStatus::Proved),
        ("invalid_ordering_panics", ProofStatus::Refuted),
        (
            "a_frame_pointer_cannot_escape_through_an_atomic_store",
            ProofStatus::Unknown,
        ),
        (
            "address_bits_do_not_authorize_an_atomic_destination",
            ProofStatus::Unknown,
        ),
        (
            "equal_size_does_not_certify_a_destination_type",
            ProofStatus::Unknown,
        ),
        ("volatile_stores_remain_unknown", ProofStatus::Unknown),
        (
            "pointer_loads_preserve_only_observed_address_bits",
            ProofStatus::Proved,
        ),
    ];
    for target in [None, Some("thumbv7em-none-eabihf")] {
        for optimized in [false, true] {
            verify(&fixture(), &entries, target, optimized);
        }
    }
}
fn native_tests(path: &Path, directory: &Directory) -> bool {
    let executable = directory.0.join("native");
    let compiler = Path::new(env!("MIR_CHECK_SYSROOT")).join("bin/rustc");
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
fn changing_a_guarded_ordering_fails_static_and_native_checks() {
    assert!(native_tests(&fixture(), &Directory::new()));
    let directory = Directory::new();
    let mutant = directory.0.join("mutant.rs");
    let source = std::fs::read_to_string(fixture()).unwrap();
    let original = "Ordering::Relaxed | Ordering::Release | Ordering::SeqCst";
    assert!(source.contains(original));
    std::fs::write(
        &mutant,
        source.replacen(
            original,
            "Ordering::Relaxed | Ordering::Release | Ordering::SeqCst | Ordering::Acquire",
            1,
        ),
    )
    .unwrap();
    for target in [None, Some("thumbv7em-none-eabihf")] {
        verify(
            &mutant,
            &[("guarded_ordering", ProofStatus::Refuted)],
            target,
            false,
        );
    }
    assert!(!native_tests(&mutant, &directory));
}
