use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub enum Answer {
    Unsat,
    Sat(String),
    Unknown(String),
}

pub fn check(query: &str) -> Answer {
    match run(query) {
        Ok(output) if output.trim() == "unsat" => Answer::Unsat,
        Ok(output) if output.trim() == "sat" => match run(&format!("{query}\n(get-model)\n")) {
            Ok(model) if model.starts_with("sat\n") && !model.contains("(error") => {
                Answer::Sat(model.trim_start_matches("sat\n").trim().to_owned())
            }
            Ok(model) => Answer::Unknown(format!("solver model failed: {model}")),
            Err(error) => Answer::Unknown(error),
        },
        Ok(output) => Answer::Unknown(format!("solver did not decide the query: {output}")),
        Err(error) => Answer::Unknown(error),
    }
}

fn run(query: &str) -> Result<String, String> {
    let solver = std::env::var_os("MIR_CHECKER_Z3")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.venv/bin/z3");
            if local.is_file() {
                local
            } else {
                PathBuf::from("z3")
            }
        });
    let mut child = Command::new(solver)
        .args(["-in", "-smt2", "-T:6"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start Z3: {error}"))?;
    let write = child
        .stdin
        .take()
        .ok_or("solver stdin unavailable")?
        .write_all(query.as_bytes());
    if let Err(error) = write {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("cannot write solver query: {error}"));
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "solver failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}
