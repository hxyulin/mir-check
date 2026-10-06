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

pub(crate) mod ground;

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

pub struct Query {
    text: String,
    declarations: Vec<String>,
    assertions: Vec<String>,
}

impl Query {
    pub fn new(declarations: &[String], conditions: &[String], failure: &str) -> Self {
        Self::from_assertions(
            declarations,
            conditions.iter().map(String::as_str).chain([failure]),
        )
    }

    fn from_assertions<'a>(
        declarations: &[String],
        assertions: impl Iterator<Item = &'a str>,
    ) -> Self {
        let mut seen = std::collections::HashSet::new();
        let assertions = assertions
            .filter(|assertion| *assertion != "true" && seen.insert(*assertion))
            .map(str::to_owned)
            .collect();
        let mut query = Self {
            text: prelude().to_owned(),
            declarations: declarations.to_vec(),
            assertions,
        };
        for declaration in &query.declarations {
            query.text.push_str(declaration);
            query.text.push('\n');
        }
        for assertion in &query.assertions {
            query.text.push_str(&format!("(assert {assertion})\n"));
        }
        query.text.push_str("(check-sat)\n");
        query
    }

    pub fn with_bindings(
        declarations: &[String],
        conditions: &[String],
        failure: &str,
        bindings: &std::collections::BTreeMap<String, String>,
    ) -> Self {
        if bindings.is_empty() {
            return Self::new(declarations, conditions, failure);
        }
        let mut pending: Vec<&str> = conditions
            .iter()
            .map(String::as_str)
            .chain([failure])
            .collect();
        let mut included = std::collections::BTreeSet::new();
        while let Some(expression) = pending.pop() {
            let Some(symbols) = expression_symbols(expression) else {
                return Self::from_assertions(
                    declarations,
                    conditions
                        .iter()
                        .chain(bindings.values())
                        .map(String::as_str)
                        .chain([failure]),
                );
            };
            for symbol in symbols {
                if let Some(constraint) = bindings.get(symbol)
                    && included.insert(symbol)
                {
                    pending.push(constraint);
                }
            }
        }
        Self::from_assertions(
            declarations,
            conditions
                .iter()
                .map(String::as_str)
                .chain(included.into_iter().map(|symbol| bindings[symbol].as_str()))
                .chain([failure]),
        )
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

fn expression_symbols(expression: &str) -> Option<Vec<&str>> {
    let mut symbols = Vec::new();
    let mut start = None;
    let mut depth = 0_usize;
    for (position, byte) in expression.bytes().enumerate() {
        if !byte.is_ascii()
            || (byte.is_ascii_control() && !matches!(byte, b'\t' | b'\r' | b'\n'))
            || matches!(byte, b';' | b'"' | b'|' | b'\\')
        {
            return None;
        }
        if byte.is_ascii_whitespace() || matches!(byte, b'(' | b')') {
            if let Some(start) = start.take() {
                symbols.push(&expression[start..position]);
            }
            match byte {
                b'(' => depth = depth.checked_add(1)?,
                b')' => depth = depth.checked_sub(1)?,
                _ => {}
            }
        } else {
            start.get_or_insert(position);
        }
    }
    if let Some(start) = start {
        symbols.push(&expression[start..]);
    }
    (depth == 0).then_some(symbols)
}

fn prelude() -> &'static str {
    "(set-logic ALL)\n(set-option :timeout 5000)\n(set-option :pp.bv-literals false)\n"
}

#[derive(Default)]
struct Context {
    declarations: Vec<String>,
    assertions: Vec<String>,
}

pub struct Solver {
    executable: PathBuf,
    custom: bool,
    session: Option<Session>,
    decisions: HashMap<String, Decision>,
    cache_bytes: usize,
    context: Option<Context>,
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
            context: None,
        }
    }
}

impl Solver {
    pub fn feasible_query(&mut self, query: &Query) -> Result<bool, String> {
        self.feasible_inner(query.text(), Some(query))
    }

    pub fn check_query(&mut self, query: &Query) -> Answer {
        self.check_inner(query.text(), Some(query))
    }

    #[cfg(test)]
    pub fn feasible(&mut self, query: &str) -> Result<bool, String> {
        self.feasible_inner(query, None)
    }

    fn feasible_inner(&mut self, query: &str, structured: Option<&Query>) -> Result<bool, String> {
        let decision = if let Some(decision) = self.decisions.get(query) {
            *decision
        } else {
            let decision = if !self.custom
                && let Some(answer) = ground::feasible(query)
            {
                if answer {
                    Decision::Sat
                } else {
                    Decision::Unsat
                }
            } else {
                self.decide_query(query, structured)?
            };
            self.remember(query, decision);
            decision
        };
        Ok(matches!(decision, Decision::Sat))
    }

    #[cfg(test)]
    pub fn check(&mut self, query: &str) -> Answer {
        self.check_inner(query, None)
    }

    fn check_inner(&mut self, query: &str, structured: Option<&Query>) -> Answer {
        if matches!(self.decisions.get(query), Some(Decision::Unsat)) {
            return Answer::Unsat;
        }
        if !self.custom && ground::feasible(query) == Some(false) {
            self.remember(query, Decision::Unsat);
            return Answer::Unsat;
        }
        match self.decide_query(query, structured) {
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
                        self.context = None;
                        Answer::Unknown(error)
                    }
                }
            }
            Err(error) => Answer::Unknown(error),
        }
    }

    fn decide_query(
        &mut self,
        query: &str,
        structured: Option<&Query>,
    ) -> Result<Decision, String> {
        let output = if self.custom {
            run(&self.executable, query)?
        } else if let Some(query) = structured {
            self.incremental(query)?
        } else {
            self.context = None;
            self.request(&format!("(reset)\n{query}"))?
        };
        match output.trim() {
            "unsat" => Ok(Decision::Unsat),
            "sat" => Ok(Decision::Sat),
            _ => {
                self.session = None;
                self.context = None;
                Err(format!("solver did not decide the query: {output}"))
            }
        }
    }

    fn incremental(&mut self, query: &Query) -> Result<String, String> {
        let mut commands = String::new();
        if self
            .context
            .as_ref()
            .is_none_or(|context| !query.declarations.starts_with(&context.declarations))
        {
            commands.push_str("(reset)\n");
            commands.push_str(prelude());
            // Symbols outlive assertion scopes, so adding one does not discard the
            // shared prefix. Standalone queries retain their ordinary declarations.
            commands.push_str("(set-option :global-decls true)\n");
            self.context = Some(Context::default());
        }
        let context = self.context.as_mut().unwrap();
        let common = context
            .assertions
            .iter()
            .zip(&query.assertions)
            .take_while(|(left, right)| left == right)
            .count();
        let pop = context.assertions.len() - common;
        if pop > 0 {
            commands.push_str(&format!("(pop {pop})\n"));
        }
        for declaration in &query.declarations[context.declarations.len()..] {
            commands.push_str(declaration);
            commands.push('\n');
        }
        for assertion in &query.assertions[common..] {
            commands.push_str(&format!("(push 1)\n(assert {assertion})\n"));
        }
        commands.push_str("(check-sat)\n");
        context.declarations = query.declarations.clone();
        context.assertions = query.assertions.clone();
        self.request(&commands)
    }

    fn request(&mut self, commands: &str) -> Result<String, String> {
        if self.session.is_none() {
            match Session::start(&self.executable) {
                Ok(session) => self.session = Some(session),
                Err(error) => {
                    self.context = None;
                    return Err(error);
                }
            }
        }
        let result = self.session.as_mut().unwrap().request(commands);
        if result.is_err() {
            self.session = None;
            self.context = None;
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

    #[test]
    fn constant_decisions_need_no_process_but_counterexamples_still_need_models() {
        let mut solver = Solver {
            executable: std::env::temp_dir().join("mir-check-missing-ground-test-solver"),
            ..persistent()
        };
        let ground = |assertion: &str| {
            format!(
                "(set-logic ALL)\n(set-option :timeout 5000)\n\
                 (set-option :pp.bv-literals false)\n(assert {assertion})\n(check-sat)\n"
            )
        };
        assert!(solver.feasible(&ground("true")).unwrap());
        assert!(!solver.feasible(&ground("false")).unwrap());
        assert!(matches!(solver.check(&ground("false")), Answer::Unsat));
        assert!(solver.session.is_none());
        assert!(matches!(solver.check(&ground("true")), Answer::Unknown(_)));
        assert!(solver.feasible(&query("(= v0 (_ bv17 8))")).is_err());
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

#[cfg(test)]
mod incremental_tests {
    use super::*;

    #[test]
    fn a_failed_process_start_does_not_leave_an_installed_query_context() {
        let executable = Solver::default().executable;
        let mut solver = Solver {
            executable: std::env::temp_dir().join("mir-check-missing-incremental-test-solver"),
            custom: false,
            ..Solver::default()
        };
        let query = Query::new(
            &["(declare-const v0 Bool)".into()],
            &["v0".into()],
            "(not v0)",
        );
        assert!(matches!(solver.check_query(&query), Answer::Unknown(_)));
        assert!(solver.context.is_none());
        assert!(solver.session.is_none());
        assert!(solver.decisions.is_empty());
        solver.executable = executable;
        assert!(matches!(solver.check_query(&query), Answer::Unsat));
        assert!(solver.context.is_some());
    }

    #[test]
    fn incremental_queries_pop_branches_and_install_new_declarations_outside_scopes() {
        let mut solver = Solver {
            custom: false,
            ..Solver::default()
        };
        let mut declarations = vec!["(declare-const v0 (_ BitVec 8))".to_owned()];
        let first = Query::new(&declarations, &["(= v0 (_ bv17 8))".into()], "true");
        assert!(solver.feasible_query(&first).unwrap());
        let process = solver.session.as_ref().unwrap().child.id();
        let second = Query::new(&declarations, &["(= v0 (_ bv93 8))".into()], "true");
        assert!(solver.feasible_query(&second).unwrap());
        declarations.push("(declare-const v1 Bool)".into());
        let third = Query::new(&declarations, &["v1".into()], "(= v0 (_ bv23 8))");
        match solver.check_query(&third) {
            Answer::Sat(model) => assert!(model.contains("bv23") && model.contains("true")),
            Answer::Unsat => panic!("new branch inherited an old assertion"),
            Answer::Unknown(error) => panic!("{error}"),
        }
        assert_eq!(solver.session.as_ref().unwrap().child.id(), process);
        let contradict = Query::new(&declarations, &["v1".into()], "(not v1)");
        assert!(matches!(solver.check_query(&contradict), Answer::Unsat));
        assert!(
            solver
                .feasible_query(&Query::new(&declarations, &[], "(not v1)"))
                .unwrap()
        );
    }

    #[test]
    fn declarations_added_under_a_shared_prefix_survive_popping_that_prefix() {
        let mut solver = Solver {
            custom: false,
            ..Solver::default()
        };
        let mut declarations = vec!["(declare-const v0 Bool)".to_owned()];
        let initial = Query::new(&declarations, &["v0".into()], "true");
        assert!(solver.feasible_query(&initial).unwrap());
        let process = solver.session.as_ref().unwrap().child.id();
        declarations.push("(declare-const v1 (_ BitVec 8))".into());
        let extended = Query::new(&declarations, &["v0".into()], "(= v1 (_ bv29 8))");
        assert!(solver.feasible_query(&extended).unwrap());
        let other_branch = Query::new(&declarations, &["(not v0)".into()], "(= v1 (_ bv73 8))");
        match solver.check_query(&other_branch) {
            Answer::Sat(model) => {
                assert!(model.contains("bv73") && model.contains("false"));
            }
            Answer::Unsat => panic!("new branch inherited the old prefix"),
            Answer::Unknown(error) => panic!("global declaration was lost: {error}"),
        }
        assert_eq!(solver.session.as_ref().unwrap().child.id(), process);
        let impossible = Query::new(&declarations, &["v0".into()], "(not v0)");
        assert!(matches!(solver.check_query(&impossible), Answer::Unsat));
        let malformed = Query::new(&declarations, &[], "missing");
        assert!(matches!(solver.check_query(&malformed), Answer::Unknown(_)));
        assert!(solver.context.is_none());
        assert!(solver.feasible_query(&extended).unwrap());
        for query in [&initial, &extended, &other_branch, &impossible] {
            let standalone = run(&solver.executable, query.text()).unwrap();
            assert_eq!(
                solver.feasible_query(query).unwrap(),
                standalone.trim() == "sat"
            );
            assert!(!query.text().contains("global-decls"));
        }
    }

    #[test]
    fn structured_queries_reset_incompatible_namespaces_and_keep_models_current() {
        let mut solver = Solver {
            custom: false,
            ..Solver::default()
        };
        let first = Query::new(&["(declare-const v0 Bool)".into()], &[], "v0");
        assert!(solver.feasible_query(&first).unwrap());
        let second = Query::new(
            &["(declare-const v0 (_ BitVec 8))".into()],
            &[],
            "(= v0 (_ bv41 8))",
        );
        assert!(solver.feasible_query(&second).unwrap());
        assert!(matches!(solver.check_query(&first), Answer::Sat(_)));
        let invalid = Query::new(&[], &[], "missing");
        assert!(matches!(solver.check_query(&invalid), Answer::Unknown(_)));
        assert!(solver.context.is_none());
        assert!(solver.feasible_query(&second).unwrap());
    }

    #[test]
    fn query_canonicalization_only_removes_true_and_identical_conjuncts() {
        let query = Query::new(
            &[],
            &["true".into(), "false".into(), "false".into()],
            "false",
        );
        assert_eq!(query.assertions, ["false"]);
        assert!(query.text().contains("(assert false)"));
        let query = Query::new(&[], &["p".into(), "(not p)".into()], "p");
        assert_eq!(query.assertions, ["p", "(not p)"]);
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn absent_latent_encodings_preserve_queries_and_do_not_validate_unsupported_syntax() {
        let declarations = ["(declare-const v0 Bool)".to_owned()];
        let conditions = ["true".to_owned(), "v0".to_owned(), "v0".to_owned()];
        for failure in ["(not v0)", "(= |v0| v0)", "missing", "(not v0"] {
            let plain = Query::new(&declarations, &conditions, failure);
            let deferred =
                Query::with_bindings(&declarations, &conditions, failure, &BTreeMap::new());
            assert_eq!(plain.text(), deferred.text());
            assert_eq!(plain.assertions, deferred.assertions);
        }
        let mut solver = Solver {
            custom: false,
            ..Solver::default()
        };
        let invalid = Query::with_bindings(&declarations, &[], "missing", &BTreeMap::new());
        assert!(matches!(solver.check_query(&invalid), Answer::Unknown(_)));
        assert!(solver.decisions.is_empty());
    }

    #[test]
    fn latent_encodings_follow_exact_symbol_dependencies_without_matching_prefixes() {
        let declarations = [
            "(declare-const v1 (_ BitVec 8))".to_owned(),
            "(declare-const v2 (_ BitVec 8))".to_owned(),
            "(declare-const v10 (_ BitVec 8))".to_owned(),
        ];
        let bindings = BTreeMap::from([
            ("v1".to_owned(), "(= v1 (_ bv41 8))".to_owned()),
            ("v2".to_owned(), "(= v2 (bvadd v1 (_ bv1 8)))".to_owned()),
            ("v10".to_owned(), "(= v10 (_ bv77 8))".to_owned()),
        ]);
        let query = Query::with_bindings(&declarations, &[], "(distinct v2 (_ bv42 8))", &bindings);
        assert_eq!(
            query.text(),
            Query::new(
                &declarations,
                &[bindings["v1"].clone(), bindings["v2"].clone()],
                "(distinct v2 (_ bv42 8))",
            )
            .text()
        );
        assert!(
            query
                .assertions
                .iter()
                .any(|assertion| assertion == &bindings["v1"])
        );
        assert!(
            query
                .assertions
                .iter()
                .any(|assertion| assertion == &bindings["v2"])
        );
        assert!(
            !query
                .assertions
                .iter()
                .any(|assertion| assertion == &bindings["v10"])
        );
        let mut solver = Solver {
            custom: false,
            ..Solver::default()
        };
        assert!(matches!(solver.check_query(&query), Answer::Unsat));
        let missing_definition = Query::new(&declarations, &[], "(distinct v2 (_ bv42 8))");
        assert!(matches!(
            solver.check_query(&missing_definition),
            Answer::Sat(_)
        ));
        let query = Query::with_bindings(&declarations, &[], "(= v10 (_ bv77 8))", &bindings);
        assert_eq!(query.assertions.len(), 1);
        assert_eq!(query.assertions[0], bindings["v10"]);
        let unused = Query::with_bindings(&declarations, &[], "true", &bindings);
        assert!(unused.assertions.is_empty());
    }

    #[test]
    fn latent_encoding_cycles_terminate_and_uncertain_lexing_includes_every_definition() {
        let bindings = BTreeMap::from([
            ("v1".to_owned(), "(= v1 v2)".to_owned()),
            ("v2".to_owned(), "(= v2 v1)".to_owned()),
        ]);
        let query = Query::with_bindings(&[], &[], "v1", &bindings);
        assert_eq!(query.assertions.len(), 3);
        for expression in [
            "(= |v1| v1)",
            "(= \"v1\" \"v2\")",
            "true ; v1",
            "(= v1 v2",
            "(= v1 v2))",
            "(= v1 \\v2)",
            "(= v1 café)",
            "true\u{0000}",
            "true\u{000b}",
            "true\u{000c}",
            "true\u{007f}",
        ] {
            let query = Query::with_bindings(&[], &[], expression, &bindings);
            for binding in bindings.values() {
                assert!(
                    query
                        .assertions
                        .iter()
                        .any(|assertion| assertion == binding)
                );
            }
        }
    }
}
