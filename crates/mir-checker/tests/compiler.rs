#![forbid(unsafe_code)]

use mir_checker::{ContractKind, ContractStatus, Report};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("mir-checker-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn analyze(path: &Path, directory: &Directory, extra_args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mir-checker"))
        .args(["--json", "--", "--crate-type=lib", "--edition=2024"])
        .arg(path)
        .arg("--out-dir")
        .arg(&directory.0)
        .args(extra_args)
        .output()
        .unwrap()
}

fn report(output: Output) -> Report {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn uncalled_generic_functions_and_methods_have_typed_mir() {
    let directory = Directory::new();
    let report = report(analyze(&fixture("bodies.rs"), &directory, &[]));
    for name in ["identity", "never_called", "guarded", "value"] {
        let function = report
            .functions
            .iter()
            .find(|function| function.name.ends_with(name))
            .unwrap();
        assert!(function.basic_blocks > 0);
        assert!(function.source.line > 0);
        assert!(function.source.file.ends_with("bodies.rs"));
    }
    assert_eq!(report.mir_phase, "optimized_mir with mir-opt-level=0");
}

#[test]
fn compiler_errors_fail_analysis_without_a_success_report() {
    let directory = Directory::new();
    let path = directory.0.join("broken.rs");
    std::fs::write(&path, "pub fn broken() -> u8 { false }").unwrap();
    let output = analyze(&path, &directory, &[]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mismatched types"));
}

#[test]
fn contracts_are_collected_as_unverified_metadata() {
    let directory = Directory::new();
    let profile = Path::new(env!("CARGO_BIN_EXE_mir-checker"))
        .parent()
        .unwrap();
    let contract_library = find_contract_library(profile).unwrap();
    let external = format!("mir_contracts={}", contract_library.display());
    let report = report(analyze(
        &fixture("contracts.rs"),
        &directory,
        &["--extern", &external],
    ));
    let function = report
        .functions
        .iter()
        .find(|function| function.name == "annotated")
        .unwrap();
    assert_eq!(function.contracts.len(), 3);
    assert!(
        function
            .contracts
            .iter()
            .any(|contract| matches!(contract.kind, ContractKind::NoPanic))
    );
    assert!(
        function
            .contracts
            .iter()
            .any(|contract| matches!(contract.kind, ContractKind::Requires))
    );
    let postcondition = function
        .contracts
        .iter()
        .find(|contract| matches!(contract.kind, ContractKind::Ensures))
        .unwrap();
    assert_eq!(postcondition.predicate.as_deref(), Some("result == value"));
    assert!(
        function
            .contracts
            .iter()
            .all(|contract| { matches!(contract.status, ContractStatus::PendingVerification) })
    );
}

fn find_contract_library(directory: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(directory).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(library) = find_contract_library(&path) {
                return Some(library);
            }
        } else if path
            .file_name()?
            .to_string_lossy()
            .starts_with("libmir_contracts-")
            && path
                .extension()
                .is_some_and(|extension| extension == "dylib" || extension == "so")
        {
            return Some(path);
        }
    }
    None
}

#[test]
fn cargo_analysis_revisits_a_crate_and_forwards_feature_selection() {
    let directory = Directory::new();
    std::fs::write(
        directory.0.join("Cargo.toml"),
        "[package]\nname = 'cargo_fixture'\nversion = '0.1.0'\nedition = '2024'\n\
        [lib]\npath = 'lib.rs'\n[features]\nextra = []\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(
        directory.0.join("lib.rs"),
        "#![no_std]\npub fn ordinary() {}\n#[cfg(feature = \"extra\")] pub fn extra() {}",
    )
    .unwrap();
    let mut report_directories = Vec::new();
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-mir-checker"))
            .args(["mir-checker", "--lib", "--features", "extra", "--offline"])
            .current_dir(&directory.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("extra at"), "{stdout}");
        let reports = stdout
            .lines()
            .find_map(|line| line.strip_prefix("JSON reports: "))
            .unwrap();
        let path = std::fs::read_dir(reports)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let report: Report = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(report.crate_name, "cargo_fixture");
        report_directories.push(reports.to_owned());
    }
    assert_ne!(report_directories[0], report_directories[1]);
}
