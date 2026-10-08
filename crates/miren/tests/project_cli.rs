#![forbid(unsafe_code)]

use miren::{ProofStatus, Report};
use serde_json::json;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("miren-project-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("Cargo.toml"),
            "[package]\nname='packet_project'\nversion='0.1.0'\nedition='2024'\n\
             [lib]\npath='lib.rs'\n[features]\nextra=[]\n[workspace]\n",
        )
        .unwrap();
        std::fs::write(
            path.join("lib.rs"),
            "#![no_std]\nconst _: &str = env!(\"CARGO_MANIFEST_DIR\");\n\
             pub fn guarded(index: usize) -> u8 {\n\
                 if index < 4 { [2,4,6,8][index] } else { 0 }\n\
             }\n\
             pub fn bad() { panic!(\"bad\"); }\n\
             pub fn opaque(read: fn() -> u8) -> u8 { read() }\n\
             pub async fn task() {}\n\
             #[cfg(feature=\"extra\")] pub fn extra() {}\n",
        )
        .unwrap();
        Self(path)
    }

    fn configure(&self, value: serde_json::Value) {
        std::fs::write(
            self.0.join("miren.json"),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }

    fn run(&self, executable: &str, args: &[&str]) -> Output {
        Command::new(executable)
            .args(args)
            .current_dir(&self.0)
            .env_remove("CARGO_TARGET_DIR")
            .output()
            .unwrap()
    }

    fn reports(&self) -> Vec<Report> {
        let latest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(self.0.join("target/miren/latest.json")).unwrap(),
        )
        .unwrap();
        let mut paths: Vec<_> = std::fs::read_dir(latest["reports"].as_str().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        paths
            .iter()
            .map(|path| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
            .collect()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn assert_status(project: &Project, name: &str, expected: ProofStatus) {
    let reports = project.reports();
    let function = reports
        .iter()
        .flat_map(|report| &report.functions)
        .find(|function| function.name == name)
        .unwrap();
    assert_eq!(
        function.proof.as_ref().unwrap().status,
        expected,
        "{:?}",
        function.proof.as_ref().unwrap().obligations
    );
}

#[test]
fn both_project_commands_check_current_source_without_a_saved_invocation_or_manual_environment() {
    for executable in [
        env!("CARGO_BIN_EXE_miren"),
        env!("CARGO_BIN_EXE_cargo-miren"),
    ] {
        let project = Project::new();
        for (entry, expected) in [
            ("guarded", ProofStatus::Proved),
            ("bad", ProofStatus::Refuted),
            ("opaque", ProofStatus::Unknown),
        ] {
            let output = project.run(executable, &["--entry", entry, "--lib", "--offline"]);
            assert_eq!(
                output.status.success(),
                expected == ProofStatus::Proved,
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_status(&project, entry, expected);
        }
        let output = project.run(executable, &["--async-entry", "task", "--lib", "--offline"]);
        assert!(output.status.success(), "{output:?}");
        assert_status(&project, "task", ProofStatus::Proved);
        assert!(String::from_utf8_lossy(&output.stdout).contains("async construction + polls"));
        let source = std::fs::read_to_string(project.0.join("lib.rs")).unwrap();
        std::fs::write(
            project.0.join("lib.rs"),
            source.replace("index < 4", "index <= 4"),
        )
        .unwrap();
        let output = project.run(executable, &["--entry", "guarded", "--lib", "--offline"]);
        assert!(!output.status.success());
        assert_status(&project, "guarded", ProofStatus::Refuted);
    }
}

#[test]
fn flat_configuration_shortens_commands_and_explicit_roots_and_limits_override_it() {
    let project = Project::new();
    project.configure(json!({"schema_version": 1, "entries": ["bad"],
        "cargo_args": ["--lib", "--offline"], "limits": {"solver_timeout_ms": 1234}}));
    let executable = env!("CARGO_BIN_EXE_miren");
    let output = project.run(executable, &[]);
    assert!(!output.status.success());
    assert_status(&project, "bad", ProofStatus::Refuted);
    let output = project.run(
        executable,
        &["--entry", "guarded", "--solver-timeout-ms", "2345"],
    );
    assert!(output.status.success(), "{output:?}");
    let reports = project.reports();
    assert_eq!(reports[0].analysis_limits.unwrap().solver_timeout_ms, 2345);
    assert_eq!(reports[0].coverage.selected_roots, 1);
    assert_status(&project, "guarded", ProofStatus::Proved);
    let output = project.run(executable, &["--entry", "extra", "--features", "extra"]);
    assert!(output.status.success(), "{output:?}");
    assert_status(&project, "extra", ProofStatus::Proved);
    let output = project.run(executable, &["inventory"]);
    assert!(output.status.success(), "{output:?}");
    assert!(
        project
            .reports()
            .iter()
            .all(|report| report.coverage.selected_roots == 0)
    );
}

#[test]
fn malformed_configuration_never_builds_or_silently_ignores_a_setting() {
    let executable = env!("CARGO_BIN_EXE_miren");
    for value in [
        json!({"schema_version": 1, "limtis": {"max_steps": 32}}),
        json!({"schema_version": 1, "limits": {"max_steps": 0}}),
        json!({"schema_version": 1, "replay": true}),
        json!({"schema_version": 2}),
    ] {
        let project = Project::new();
        project.configure(value);
        let output = project.run(executable, &[]);
        assert!(!output.status.success());
        assert!(!project.0.join("target/miren").exists());
    }
    let project = Project::new();
    project.configure(json!({"schema_version": 2}));
    let output = project.run(
        executable,
        &["--no-config", "--entry", "guarded", "--lib", "--offline"],
    );
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn latest_reports_preserve_failures_and_never_reuse_success_after_a_failed_build() {
    let project = Project::new();
    project.configure(json!({"schema_version": 1, "entries": ["guarded"],
        "cargo_args": ["--lib", "--offline"]}));
    let executable = env!("CARGO_BIN_EXE_miren");
    assert!(project.run(executable, &[]).status.success());
    let output = project.run(executable, &["report", "--quiet"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("compiler and solver were not rerun"));
    assert!(
        !project
            .run(executable, &["--entry", "bad"])
            .status
            .success()
    );
    let output = project.run(executable, &["report", "--quiet"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("REFUTED"));
    std::fs::write(project.0.join("lib.rs"), "this is not Rust").unwrap();
    assert!(!project.run(executable, &[]).status.success());
    let output = project.run(executable, &["report", "--quiet"]);
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PROVED"));
}

#[test]
fn repeated_arm_checks_revisit_roots_selectors_and_source_mutations() {
    for target_args in [
        vec!["--target", "thumbv7em-none-eabihf"],
        vec!["--config", "build.target='thumbv7em-none-eabihf'"],
    ] {
        let project = Project::new();
        let mut cargo_args = vec!["--lib", "--offline"];
        cargo_args.extend(target_args);
        project.configure(json!({"schema_version": 1, "entries": ["guarded"],
            "cargo_args": cargo_args}));
        let executable = env!("CARGO_BIN_EXE_miren");
        for _ in 0..2 {
            let output = project.run(executable, &[]);
            assert!(output.status.success(), "{output:?}");
            assert_status(&project, "guarded", ProofStatus::Proved);
            assert_eq!(project.reports()[0].target, "thumbv7em-none-eabihf");
        }
        let output = project.run(executable, &["--entry", "bad"]);
        assert!(!output.status.success());
        assert_status(&project, "bad", ProofStatus::Refuted);
        let source = std::fs::read_to_string(project.0.join("lib.rs")).unwrap();
        std::fs::write(
            project.0.join("lib.rs"),
            source.replace("index < 4", "index <= 4"),
        )
        .unwrap();
        let output = project.run(executable, &[]);
        assert!(!output.status.success());
        assert_status(&project, "guarded", ProofStatus::Refuted);
    }
}

#[test]
fn dependency_builds_are_cached_but_roots_selectors_and_dependency_mutations_are_rechecked() {
    let project = Project::new();
    std::fs::write(
        project.0.join("Cargo.toml"),
        "[package]\nname='packet_project'\nversion='0.1.0'\nedition='2024'\n\
         [lib]\npath='lib.rs'\n[dependencies]\nproject_helper={path='helper'}\n\
         [workspace]\nexclude=['helper']\n",
    )
    .unwrap();
    let helper = project.0.join("helper");
    std::fs::create_dir_all(&helper).unwrap();
    std::fs::write(
        helper.join("Cargo.toml"),
        "[package]\nname='project_helper'\nversion='0.1.0'\nedition='2024'\n\
         [lib]\npath='lib.rs'\n",
    )
    .unwrap();
    std::fs::write(
        helper.join("lib.rs"),
        "#![no_std]\npub fn selected() -> usize { 0 }",
    )
    .unwrap();
    std::fs::write(
        helper.join("build.rs"),
        "fn main() {\n\
         let path=std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(\"build-count\");\n\
         let old=std::fs::read_to_string(&path).ok().and_then(|s|s.parse::<u32>().ok());\n\
         std::fs::write(path,(old.unwrap_or(0)+1).to_string()).unwrap();\n\
         println!(\"cargo:rerun-if-changed=build.rs\");\n}\n",
    )
    .unwrap();
    std::fs::write(
        project.0.join("lib.rs"),
        "#![no_std]\npub fn guarded() -> u8 { [4,8][project_helper::selected()] }",
    )
    .unwrap();
    project.configure(json!({"schema_version": 1, "entries": ["guarded"],
        "cargo_args": ["--lib", "--offline"]}));
    let executable = env!("CARGO_BIN_EXE_miren");
    let mut previous = None;
    for _ in 0..2 {
        let output = project.run(executable, &[]);
        assert!(output.status.success(), "{output:?}");
        assert_status(&project, "guarded", ProofStatus::Proved);
        assert_eq!(
            std::fs::read_to_string(helper.join("build-count")).unwrap(),
            "1"
        );
        let record = std::fs::read(project.0.join("target/miren/latest.json")).unwrap();
        assert_ne!(previous.as_ref(), Some(&record));
        previous = Some(record);
    }
    let output = project.run(executable, &["--entry", "missing"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no inventoried MIR body"));
    let output = project.run(executable, &["report", "--quiet"]);
    assert!(!output.status.success());
    std::fs::write(
        helper.join("lib.rs"),
        "#![no_std]\npub fn selected() -> usize { 2 }",
    )
    .unwrap();
    let output = project.run(executable, &[]);
    assert!(!output.status.success());
    assert_status(&project, "guarded", ProofStatus::Refuted);
}
