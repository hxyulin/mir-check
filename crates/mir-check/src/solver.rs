use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
const MAX_OUTPUT_BYTES: usize = 262_144;
const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 1024;

pub enum Answer {
    Unsat,
    Sat(String),
    Unknown(String),
}

#[derive(Clone, Copy)]
enum Decision {
    Unsat,
    Sat,
}

pub struct Solver {
    executable: PathBuf,
    custom: bool,
    session: Option<Session>,
    decisions: HashMap<String, Decision>,
    cache_bytes: usize,
}

impl Default for Solver {
    fn default() -> Self {
        let custom = std::env::var_os("MIR_CHECK_Z3");
        let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.venv/bin/z3");
        let executable = custom.clone().map(PathBuf::from).unwrap_or_else(|| {
            if local.is_file() {
                local
            } else {
                PathBuf::from("z3")
            }
        });
        Self {
            executable,
            custom: custom.is_some(),
            session: None,
            decisions: HashMap::new(),
            cache_bytes: 0,
        }
    }
}

impl Solver {
    pub fn feasible(&mut self, query: &str) -> Result<bool, String> {
        let decision = if let Some(decision) = self.decisions.get(query) {
            *decision
        } else {
            let decision = self.decide(query)?;
            self.remember(query, decision);
            decision
        };
        Ok(matches!(decision, Decision::Sat))
    }

    pub fn check(&mut self, query: &str) -> Answer {
        if matches!(self.decisions.get(query), Some(Decision::Unsat)) {
            return Answer::Unsat;
        }
        match self.decide(query) {
            Ok(Decision::Unsat) => {
                self.remember(query, Decision::Unsat);
                Answer::Unsat
            }
            Ok(Decision::Sat) => {
                let model = if self.custom {
                    run(&self.executable, &format!("{query}\n(get-model)\n")).and_then(|output| {
                        output
                            .strip_prefix("sat\n")
                            .map(str::to_owned)
                            .ok_or_else(|| format!("solver model failed: {output}"))
                    })
                } else {
                    self.request("(get-model)\n")
                };
                match model.and_then(validate_model) {
                    Ok(model) => {
                        self.remember(query, Decision::Sat);
                        Answer::Sat(model)
                    }
                    Err(error) => {
                        self.session = None;
                        Answer::Unknown(error)
                    }
                }
            }
            Err(error) => Answer::Unknown(error),
        }
    }

    fn decide(&mut self, query: &str) -> Result<Decision, String> {
        let output = if self.custom {
            run(&self.executable, query)?
        } else {
            self.request(&format!("(reset)\n{query}"))?
        };
        match output.trim() {
            "unsat" => Ok(Decision::Unsat),
            "sat" => Ok(Decision::Sat),
            _ => {
                self.session = None;
                Err(format!("solver did not decide the query: {output}"))
            }
        }
    }

    fn request(&mut self, commands: &str) -> Result<String, String> {
        if self.session.is_none() {
            self.session = Some(Session::start(&self.executable)?);
        }
        let result = self.session.as_mut().unwrap().request(commands);
        if result.is_err() {
            self.session = None;
        }
        result
    }

    fn remember(&mut self, query: &str, decision: Decision) {
        if query.len() > MAX_CACHE_BYTES || self.decisions.contains_key(query) {
            return;
        }
        if self.cache_bytes + query.len() > MAX_CACHE_BYTES
            || self.decisions.len() == MAX_CACHE_ENTRIES
        {
            self.decisions.clear();
            self.cache_bytes = 0;
        }
        self.cache_bytes += query.len();
        self.decisions.insert(query.to_owned(), decision);
    }
}

fn validate_model(model: String) -> Result<String, String> {
    let model = model.trim();
    if !model.starts_with('(') || !model.ends_with(')') || model.contains("(error") {
        return Err(format!("solver model failed: {model}"));
    }
    Ok(model.to_owned())
}

enum Event {
    Line(String),
    Error(String),
}

struct Session {
    child: Child,
    commands: Sender<String>,
    events: Receiver<Event>,
    sequence: u64,
}

impl Session {
    fn start(executable: &PathBuf) -> Result<Self, String> {
        let mut child = Command::new(executable)
            .args(["-in", "-smt2"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("cannot start Z3: {error}"))?;
        let mut stdin = child.stdin.take().ok_or("solver stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("solver stdout unavailable")?;
        let (commands, input) = mpsc::channel::<String>();
        let (output, events) = mpsc::sync_channel(64);
        let errors = output.clone();
        std::thread::spawn(move || {
            while let Ok(command) = input.recv() {
                if let Err(error) = stdin
                    .write_all(command.as_bytes())
                    .and_then(|()| stdin.flush())
                {
                    let _ =
                        errors.send(Event::Error(format!("cannot write solver query: {error}")));
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                let event = match reader
                    .by_ref()
                    .take(MAX_OUTPUT_BYTES as u64 + 1)
                    .read_line(&mut line)
                {
                    Ok(0) => {
                        Event::Error("solver closed its output before the response marker".into())
                    }
                    Ok(_) if line.len() > MAX_OUTPUT_BYTES => {
                        Event::Error("solver response size limit reached".into())
                    }
                    Ok(_) => Event::Line(line),
                    Err(error) => Event::Error(format!("cannot read solver response: {error}")),
                };
                let failed = matches!(event, Event::Error(_));
                if output.send(event).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            commands,
            events,
            sequence: 0,
        })
    }

    fn request(&mut self, commands: &str) -> Result<String, String> {
        self.sequence += 1;
        let marker = format!("mir_check_response_{}", self.sequence);
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        self.commands
            .send(format!("{commands}\n(echo \"{marker}\")\n"))
            .map_err(|error| format!("cannot send solver query: {error}"))?;
        let mut response = String::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(remaining) {
                Ok(Event::Line(line)) if line.trim_end() == marker => return Ok(response),
                Ok(Event::Line(line)) => {
                    if response.len() + line.len() > MAX_OUTPUT_BYTES {
                        return Err("solver response size limit reached".into());
                    }
                    response.push_str(&line);
                }
                Ok(Event::Error(error)) => return Err(error),
                Err(error) => {
                    return Err(format!(
                        "solver response failed within six seconds: {error}"
                    ));
                }
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run(solver: &PathBuf, query: &str) -> Result<String, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn persistent() -> Solver {
        Solver {
            custom: false,
            ..Solver::default()
        }
    }

    fn query(assertion: &str) -> String {
        format!(
            "(set-logic ALL)\n(set-option :timeout 5000)\n\
             (declare-fun v0 () (_ BitVec 8))\n(assert {assertion})\n(check-sat)\n"
        )
    }

    #[test]
    fn resetting_a_session_keeps_declarations_and_assertions_local_to_each_query() {
        let mut solver = persistent();
        assert!(solver.feasible(&query("(= v0 (_ bv17 8))")).unwrap());
        let process = solver.session.as_ref().unwrap().child.id();
        assert!(solver.feasible(&query("(= v0 (_ bv93 8))")).unwrap());
        assert!(matches!(
            solver.check(&query("(distinct v0 v0)")),
            Answer::Unsat
        ));
        assert!(matches!(
            solver.check("(set-logic ALL)\n(declare-fun v0 () Bool)\n(assert v0)\n(check-sat)\n"),
            Answer::Sat(_)
        ));
        assert_eq!(solver.session.as_ref().unwrap().child.id(), process);
    }

    #[test]
    fn feasibility_does_not_fetch_a_model_but_refutations_use_the_current_query() {
        let mut solver = persistent();
        let first = query("(= v0 (_ bv17 8))");
        assert!(solver.feasible(&first).unwrap());
        assert_eq!(solver.session.as_ref().unwrap().sequence, 1);
        let second = query("(= v0 (_ bv93 8))");
        assert!(solver.feasible(&second).unwrap());
        match solver.check(&first) {
            Answer::Sat(model) => assert!(model.contains("#x11") || model.contains("bv17")),
            Answer::Unsat => panic!("satisfiable query was called unsatisfiable"),
            Answer::Unknown(error) => panic!("counterexample query failed: {error}"),
        }
        assert_eq!(solver.session.as_ref().unwrap().sequence, 4);
    }

    #[test]
    fn exact_queries_are_cached_without_reusing_a_different_failure_condition() {
        let mut solver = persistent();
        let impossible = query("(distinct v0 v0)");
        assert!(matches!(solver.check(&impossible), Answer::Unsat));
        assert!(!solver.feasible(&impossible).unwrap());
        assert!(matches!(solver.check(&impossible), Answer::Unsat));
        assert_eq!(solver.session.as_ref().unwrap().sequence, 1);
        assert!(solver.feasible(&query("(= v0 v0)")).unwrap());
        assert_eq!(solver.session.as_ref().unwrap().sequence, 2);
    }

    #[test]
    fn malformed_solver_queries_are_unknown_and_restart_the_session() {
        let mut solver = persistent();
        let invalid = "(set-logic ALL)\n(assert missing)\n(check-sat)\n";
        assert!(matches!(solver.check(invalid), Answer::Unknown(_)));
        assert!(!solver.decisions.contains_key(invalid));
        assert!(solver.session.is_none());
        assert!(solver.feasible(&query("(= v0 v0)")).unwrap());
    }

    #[test]
    fn decision_cache_has_byte_and_entry_limits() {
        let mut solver = persistent();
        for index in 0..MAX_CACHE_ENTRIES + 3 {
            solver.remember(&format!("query{index}"), Decision::Unsat);
        }
        assert!(solver.decisions.len() <= MAX_CACHE_ENTRIES);
        assert!(solver.cache_bytes <= MAX_CACHE_BYTES);
        solver.remember(&"x".repeat(MAX_CACHE_BYTES), Decision::Sat);
        assert_eq!(solver.decisions.len(), 1);
        solver.remember(&"x".repeat(MAX_CACHE_BYTES + 1), Decision::Unsat);
        assert_eq!(solver.decisions.len(), 1);
    }

    #[cfg(unix)]
    struct Script(std::path::PathBuf);

    #[cfg(unix)]
    impl Script {
        fn new(body: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("mir-check-solver-test-{}-{id}", std::process::id()));
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self(path)
        }
    }

    #[cfg(unix)]
    impl Drop for Script {
        fn drop(&mut self) {
            std::fs::remove_file(&self.0).unwrap();
        }
    }

    #[test]
    #[cfg(unix)]
    fn eof_and_undecided_responses_never_enter_the_decision_cache() {
        for response in ["unsat", "unknown\nmir_check_response_1"] {
            let script = Script::new(&format!("printf '%s\\n' '{response}'"));
            let mut solver = Solver {
                executable: script.0.clone(),
                ..persistent()
            };
            assert!(matches!(solver.check(&query("true")), Answer::Unknown(_)));
            assert!(solver.decisions.is_empty());
            assert!(solver.session.is_none());
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_stalled_writer_is_killed_by_the_host_deadline() {
        let script = Script::new("exec sleep 20");
        let mut solver = Solver {
            executable: script.0.clone(),
            ..persistent()
        };
        let started = Instant::now();
        assert!(solver.request(&"x".repeat(1_000_000)).is_err());
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(solver.session.is_none());
    }
}
