use crate::cli::{Color, Progress};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize)]
struct Metadata {
    workspace_root: PathBuf,
    workspace_members: Vec<String>,
}

pub(super) struct Workspace {
    pub(super) root: PathBuf,
    members: Vec<String>,
    manifest: Option<OsString>,
    targets: Vec<OsString>,
    cargo_options: Vec<OsString>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Run {
    schema_version: u32,
    reports: PathBuf,
    complete: bool,
    successful: bool,
}

pub(super) fn discover(args: &[OsString]) -> Result<Workspace, Box<dyn std::error::Error>> {
    let mut manifest = None;
    let mut targets = Vec::new();
    let mut metadata_args = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--manifest-path") => {
                manifest = Some(args.next().ok_or("--manifest-path needs a path")?.clone());
            }
            Some(value) if value.starts_with("--manifest-path=") => {
                manifest = Some(value[16..].into());
            }
            Some("--target") => {
                targets.push(args.next().ok_or("--target needs a target")?.clone());
            }
            Some(value) if value.starts_with("--target=") => {
                targets.push(value[9..].into());
            }
            Some("--offline" | "--locked" | "--frozen") => metadata_args.push(arg.clone()),
            Some("--config") => {
                metadata_args.push(arg.clone());
                metadata_args.push(args.next().ok_or("Cargo --config needs a value")?.clone());
            }
            Some(value) if value.starts_with("--config=") => metadata_args.push(arg.clone()),
            _ => {}
        }
    }
    let mut command = cargo();
    command.args(["metadata", "--no-deps", "--format-version=1"]);
    command.args(&metadata_args);
    if let Some(path) = &manifest {
        command.arg("--manifest-path").arg(path);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "cannot load Cargo project:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let metadata: Metadata = serde_json::from_slice(&output.stdout)?;
    Ok(Workspace {
        root: metadata.workspace_root.join("target/miren"),
        members: metadata.workspace_members,
        manifest,
        targets,
        cargo_options: metadata_args,
    })
}

pub(super) fn cargo() -> Command {
    let sysroot = Path::new(env!("MIREN_SYSROOT"));
    let mut command = Command::new(sysroot.join("bin/cargo"));
    command
        .env("RUSTC", sysroot.join("bin/rustc"))
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER");
    command
}

impl Workspace {
    pub(super) fn lock(&self, quiet: bool, color: Color) -> Result<File, std::io::Error> {
        std::fs::create_dir_all(&self.root)?;
        let file = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("build.lock"))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                let progress =
                    Progress::new(!quiet, color, "Waiting for analysis build cache".into());
                file.lock()?;
                drop(progress);
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
        Ok(file)
    }

    pub(super) fn refresh(&self, build: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let mut command = cargo();
        command
            .args(["clean", "--quiet", "--target-dir"])
            .arg(build);
        command.args(&self.cargo_options);
        if let Some(path) = &self.manifest {
            command.arg("--manifest-path").arg(path);
        }
        for target in &self.targets {
            command.arg("--target").arg(target);
        }
        for member in &self.members {
            command.arg("--package").arg(member);
        }
        let status = command.status()?;
        if !status.success() {
            return Err("could not refresh workspace analysis artifacts".into());
        }
        Ok(())
    }

    pub(super) fn record(
        &self,
        reports: &Path,
        complete: bool,
        successful: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let run = Run {
            schema_version: 1,
            reports: reports.to_path_buf(),
            complete,
            successful,
        };
        let temporary = self.root.join(format!("latest-{}.tmp", std::process::id()));
        std::fs::write(&temporary, serde_json::to_vec_pretty(&run)?)?;
        std::fs::rename(temporary, self.root.join("latest.json"))?;
        Ok(())
    }
}

pub(crate) fn latest() -> Result<(PathBuf, bool), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let path = cwd
        .ancestors()
        .map(|path| path.join("target/miren/latest.json"))
        .find(|path| path.is_file())
        .ok_or("no project run found; run miren from your Cargo project first")?;
    let run: Run = serde_json::from_slice(&std::fs::read(&path)?)?;
    if run.schema_version != 1 {
        return Err("unsupported saved run schema".into());
    }
    if !run.complete {
        return Err("the latest project run did not finish; rerun miren".into());
    }
    Ok((run.reports, run.successful))
}
