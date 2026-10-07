use crate::{Function, Obligation, ObligationKind, ProofStatus, Report};
use std::fmt::Write as _;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default)]
pub enum Color {
    #[default]
    Auto,
    Always,
    Never,
}

impl Color {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            _ => Err("--color expects auto, always or never".to_owned()),
        }
    }

    pub fn enabled(self, terminal: bool) -> bool {
        match self {
            Self::Auto => terminal && std::env::var_os("NO_COLOR").is_none(),
            Self::Always => true,
            Self::Never => false,
        }
    }

    pub fn argument(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

pub fn paint(text: &str, code: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

pub fn output_path(value: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.starts_with('-') && value != "-" {
        Err("--jsonl needs a file path or - for stdout".to_owned())
    } else {
        Ok(PathBuf::from(value))
    }
}

fn short_detail(detail: &str) -> String {
    if let Some(intrinsic) = detail
        .strip_prefix("unmodeled call adapter Intrinsic(")
        .and_then(|detail| {
            detail
                .split_once(" ~ ")
                .map(|(_, name)| name.trim_end_matches(')'))
        })
    {
        return format!("Unsupported intrinsic: {intrinsic}");
    }
    let detail = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    if detail.chars().count() > 140 {
        format!("{}...", detail.chars().take(137).collect::<String>())
    } else {
        detail
    }
}

fn status_color(status: ProofStatus) -> &'static str {
    match status {
        ProofStatus::Proved => "32",
        ProofStatus::ProvedWithAssumptions => "35",
        ProofStatus::Refuted => "31",
        ProofStatus::Unknown => "33",
    }
}

fn next_step(detail: &str) -> &'static str {
    if detail.contains("call-depth limit") {
        "Try --max-call-depth; recursive calls may need an invariant."
    } else if detail.contains("execution step limit") {
        "Try --induction for supported loops, or increase --max-steps."
    } else if detail.contains("query size limit") {
        "Try --max-query-bytes; this changes a resource budget, not supported operations."
    } else if detail.contains("execution budget") {
        "Try --root-timeout-secs to give this root more analysis time."
    } else if detail.ends_with(": timeout")
        || detail == "timeout"
        || detail.contains("Z3 returned timeout")
        || detail.contains("solver timed out")
    {
        "Try --solver-timeout-ms to give each solver query more time."
    } else if detail.contains("foreign declarations have no Rust MIR") {
        "Provide an explicit --contracts boundary if you can justify its assumptions."
    } else if detail.contains("prebuilt core library omitted") {
        "Try Cargo -Zbuild-std=core to retain core MIR for this target."
    } else if detail.contains("MIR body unavailable") {
        "Rebuild with dependency MIR retention; inspect the named callee and build artifacts."
    } else if detail.contains("mutable static payload reads need a state model") {
        "Add support for this storage's initialization and updates; larger limits \
         cannot supply its runtime value."
    } else if detail.contains("root input") || detail.contains("root parameter") {
        "Select a concrete caller that constructs the input, or add support for its shape."
    } else {
        "Inspect the named construct with --verbose; larger budgets cannot model unsupported code."
    }
}

fn render_call_chain(output: &mut String, function: &Function, obligation: &Obligation) {
    if !obligation.call_chain.is_empty() {
        let chain = obligation.call_chain.join(" -> ");
        if chain.chars().count() < 85 {
            let _ = writeln!(output, "    call chain: {chain}");
        } else {
            output.push_str("    call chain:\n");
            for (index, function) in obligation.call_chain.iter().enumerate() {
                let _ = writeln!(
                    output,
                    "      {}{function}",
                    if index == 0 { "" } else { "-> " }
                );
            }
        }
    } else if obligation.function != function.name {
        let _ = writeln!(
            output,
            "    root: {}; failing function: {} (call chain not recorded)",
            function.name, obligation.function
        );
    }
}

fn render_inputs(output: &mut String, function: &Function, obligation: &Obligation) {
    let (label, inputs) = if let Some(replay) = &obligation.replay
        && !replay.inputs.is_empty()
    {
        ("replay inputs", replay.inputs.clone())
    } else if let Some(recipe) = function
        .proof
        .as_ref()
        .and_then(|proof| proof.replay_inputs.as_ref())
        && let Some(model) = &obligation.model
        && let Ok(inputs) = crate::replay::counterexample_inputs(recipe, model)
    {
        ("counterexample inputs", inputs)
    } else {
        return;
    };
    if !inputs.is_empty() {
        let inputs = inputs
            .iter()
            .map(|(name, value)| format!("{name} = {value}"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(output, "    {label}: {inputs}");
    }
}

fn render_replay(output: &mut String, function: &Function, obligation: &Obligation, colored: bool) {
    use crate::replay::ReplayStatus;

    render_inputs(output, function, obligation);
    let Some(replay) = &obligation.replay else {
        let outcome = if obligation.kind == ObligationKind::PanicSafety {
            "panic"
        } else {
            "failure"
        };
        let _ = writeln!(
            output,
            "    evidence: symbolic counterexample; runtime {outcome} unconfirmed"
        );
        if !obligation.abstraction_reasons.is_empty() {
            output.push_str(
                "    next: Validate these query choices against the root's execution \
                 environment; native replay supplies inputs but cannot force abstract choices.\n",
            );
        }
        return;
    };
    let (label, color) = match replay.status {
        ReplayStatus::ConfirmedPanic => ("native replay panicked", "31"),
        ReplayStatus::NotReproduced => ("native replay did not panic", "33"),
        ReplayStatus::Unsupported => ("native replay unsupported", "33"),
        ReplayStatus::ToolFailure => ("native replay could not complete", "33"),
    };
    let _ = writeln!(output, "    evidence: {}", paint(label, color, colored));
    if replay.status == ReplayStatus::ConfirmedPanic {
        if let Some(message) = &replay.panic_message {
            let _ = writeln!(output, "    native panic message: {message}");
        }
        if let Some(source) = &replay.panic_source {
            let _ = writeln!(
                output,
                "    native panic at {}:{}:{}",
                source.file, source.line, source.column
            );
        }
        if replay.matches_obligation {
            output.push_str("    panic site matches the obligation\n");
        } else {
            output.push_str("    panic site differs or could not be matched to this obligation\n");
        }
    }
    if !replay.detail.is_empty() {
        let _ = writeln!(output, "    replay: {}", short_detail(&replay.detail));
    }
    if !replay.uncontrolled_abstractions.is_empty() {
        output.push_str(
            "    replay scope: root inputs only; query abstraction choices were not forced\n",
        );
        if replay.status == ReplayStatus::NotReproduced {
            output.push_str(
                "    next: Validate these choices against the root's execution environment. \
                 A normal return does not validate every shared-state history.\n",
            );
        }
    }
    let _ = writeln!(
        output,
        "    replay panic strategy: {}",
        replay.panic_strategy
    );
}

fn render_failure(
    output: &mut String,
    function: &Function,
    obligation: &Obligation,
    colored: bool,
) {
    let label = match obligation.kind {
        ObligationKind::PanicSafety => "panic condition",
        ObligationKind::Validity => "validity condition",
        ObligationKind::CallPrecondition => "callee precondition",
        ObligationKind::Postcondition => "postcondition",
        ObligationKind::Unsupported => "blocked by",
    };
    let _ = writeln!(output, "    {label}: {}", short_detail(&obligation.detail));
    let _ = writeln!(
        output,
        "    at {}:{}:{} in {}",
        obligation.source.file,
        obligation.source.line,
        obligation.source.column,
        obligation.function
    );
    render_call_chain(output, function, obligation);
    for reason in &obligation.abstraction_reasons {
        let _ = writeln!(output, "    abstraction in query: {reason}");
    }
    match obligation.status {
        ProofStatus::Refuted => render_replay(output, function, obligation, colored),
        ProofStatus::Unknown => {
            let _ = writeln!(output, "    next: {}", next_step(&obligation.detail));
        }
        ProofStatus::Proved | ProofStatus::ProvedWithAssumptions => {}
    }
}

// Line-based heartbeats do not redraw or hide compiler diagnostics, including in captured logs.
pub struct Progress {
    sender: Option<mpsc::Sender<Option<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    started: Instant,
}

impl Progress {
    pub fn new(enabled: bool, color: Color, message: String) -> Self {
        let started = Instant::now();
        let (sender, thread) = if enabled {
            let (sender, receiver) = mpsc::channel::<Option<String>>();
            let colored = color.enabled(std::io::stderr().is_terminal());
            eprintln!("{} {message}", paint("mir-check:", "36", colored));
            let thread = std::thread::spawn(move || {
                let mut message = message;
                let mut heartbeat = Instant::now();
                loop {
                    let remaining = Duration::from_secs(5).saturating_sub(heartbeat.elapsed());
                    match receiver.recv_timeout(remaining) {
                        Ok(Some(next)) => message = next,
                        Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            eprintln!(
                                "{} {message} ({:.0}s elapsed)",
                                paint("mir-check:", "36", colored),
                                started.elapsed().as_secs_f64()
                            );
                            heartbeat = Instant::now();
                        }
                    }
                }
            });
            (Some(sender), Some(thread))
        } else {
            (None, None)
        };
        Self {
            sender,
            thread,
            started,
        }
    }

    pub fn update(&self, message: String) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(Some(message));
        }
    }

    pub fn elapsed(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(None);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn render_report(report: &Report, verbose: bool, colored: bool) -> String {
    if verbose {
        let mut output = crate::render(report);
        for function in &report.functions {
            let Some(proof) = &function.proof else {
                continue;
            };
            for obligation in &proof.obligations {
                if matches!(
                    obligation.status,
                    ProofStatus::Refuted | ProofStatus::Unknown
                ) {
                    let _ = writeln!(output, "  failure explanation for {}:", function.name);
                    render_failure(&mut output, function, obligation, false);
                }
            }
        }
        return output
            .split_inclusive(char::is_whitespace)
            .map(|word| {
                let label = word.trim_end();
                let status = [
                    ProofStatus::Proved,
                    ProofStatus::ProvedWithAssumptions,
                    ProofStatus::Refuted,
                    ProofStatus::Unknown,
                ]
                .into_iter()
                .find(|status| label == status.label());
                status.map_or_else(
                    || word.to_owned(),
                    |status| {
                        format!(
                            "{}{}",
                            paint(label, status_color(status), colored),
                            &word[label.len()..]
                        )
                    },
                )
            })
            .collect();
    }
    let coverage = &report.coverage;
    let mut output = format!(
        "\n{} [{}]\n",
        paint(&report.crate_name, "1", colored),
        report.target
    );
    for (status, count) in [
        (ProofStatus::Proved, coverage.proved),
        (ProofStatus::Refuted, coverage.refuted),
        (ProofStatus::Unknown, coverage.unknown),
        (
            ProofStatus::ProvedWithAssumptions,
            coverage.proved_with_assumptions,
        ),
    ] {
        let _ = write!(
            output,
            "  {} {}",
            paint(status.label(), status_color(status), colored),
            count
        );
    }
    let _ = writeln!(
        output,
        "\n  {} selected / {} inventoried; {} unselected; {} interpreted instances",
        coverage.selected_roots,
        coverage.inventoried_bodies,
        coverage.unselected_bodies,
        coverage.interpreted_instances
    );
    let mut roots = report
        .functions
        .iter()
        .filter(|function| function.proof.is_some())
        .collect::<Vec<_>>();
    roots.sort_by_key(|function| {
        let status = function.proof.as_ref().expect("selected root").status;
        (
            match status {
                ProofStatus::Refuted => 0,
                ProofStatus::Unknown => 1,
                ProofStatus::ProvedWithAssumptions => 2,
                ProofStatus::Proved => 3,
            },
            &function.name,
        )
    });
    for function in roots.iter().take(20) {
        let proof = function.proof.as_ref().expect("selected root");
        let _ = writeln!(
            output,
            "  {} {}",
            paint(proof.status.label(), status_color(proof.status), colored),
            function.name
        );
        if proof.status != ProofStatus::Proved {
            for assumption in &proof.entry_assumptions {
                let _ = writeln!(output, "    entry assumption: {assumption}");
            }
            let _ = writeln!(
                output,
                "    {}:{}",
                function.source.file, function.source.line
            );
            if let Some(obligation) = proof
                .obligations
                .iter()
                .find(|obligation| obligation.status == proof.status)
            {
                render_failure(&mut output, function, obligation, colored);
            }
            if proof.status == ProofStatus::Refuted
                && let Some(blocker) = proof
                    .obligations
                    .iter()
                    .find(|obligation| obligation.status == ProofStatus::Unknown)
            {
                output.push_str("    analysis also incomplete:\n");
                render_failure(&mut output, function, blocker, colored);
            }
            if proof.stopped_after_counterexample {
                let _ = writeln!(output, "    stopped after first counterexample");
            }
            for trusted in &proof.trusted_calls {
                let _ = writeln!(
                    output,
                    "    assumes {}: {}",
                    trusted.contract.function,
                    trusted
                        .contract
                        .reason
                        .as_deref()
                        .unwrap_or("unspecified reason")
                );
            }
        }
    }
    if roots.len() > 20 {
        let _ = writeln!(
            output,
            "  ... {} more roots; use --verbose or the JSON reports",
            roots.len() - 20
        );
    }
    if !coverage.gaps.is_empty() {
        let mut gaps = coverage.gaps.iter().collect::<Vec<_>>();
        gaps.sort_by_key(|gap| std::cmp::Reverse(gap.roots.len()));
        output.push_str("  Main analysis gaps:\n");
        for gap in gaps.iter().take(5) {
            let _ = writeln!(
                output,
                "    {} root{}: {}",
                gap.roots.len(),
                if gap.roots.len() == 1 { "" } else { "s" },
                short_detail(&gap.reason)
            );
        }
        if gaps.len() > 5 {
            let _ = writeln!(
                output,
                "    ... {} other gap reasons in the JSON reports",
                gaps.len() - 5
            );
        }
    }
    output
}

pub fn accepted(report: &Report, allow_assumptions: bool) -> bool {
    report
        .functions
        .iter()
        .filter_map(|function| function.proof.as_ref())
        .all(|proof| match proof.status {
            ProofStatus::Proved => true,
            ProofStatus::ProvedWithAssumptions => allow_assumptions,
            ProofStatus::Refuted | ProofStatus::Unknown => false,
        })
}

pub fn render_totals(reports: &[Report], success: bool, colored: bool, elapsed_s: f64) -> String {
    let selected: usize = reports
        .iter()
        .map(|report| report.coverage.selected_roots)
        .sum();
    let proved: usize = reports.iter().map(|report| report.coverage.proved).sum();
    let assumed: usize = reports
        .iter()
        .map(|report| report.coverage.proved_with_assumptions)
        .sum();
    let refuted: usize = reports.iter().map(|report| report.coverage.refuted).sum();
    let unknown: usize = reports.iter().map(|report| report.coverage.unknown).sum();
    let label = if !success {
        "Run failed"
    } else if selected == 0 {
        "Inventory complete"
    } else if assumed == 0 {
        "Verification passed"
    } else {
        "Verification passed with assumptions"
    };
    let code = if !success {
        "31"
    } else if assumed > 0 {
        "35"
    } else if selected == 0 {
        "36"
    } else {
        "32"
    };
    let mut output = format!(
        "\n{} in {elapsed_s:.1}s | {} crate report{}\n",
        paint(label, code, colored),
        reports.len(),
        if reports.len() == 1 { "" } else { "s" }
    );
    let _ = writeln!(
        output,
        "  {selected} selected root{}: {proved} proved, {assumed} with assumptions, \
         {refuted} refuted, {unknown} unknown",
        if selected == 1 { "" } else { "s" }
    );
    output.push_str(
        "  Counts describe selected MIR roots, not line coverage or whole-crate safety.\n",
    );
    if selected == 0 {
        output
            .push_str("  Inventory only; no selected-root proof. Use --verify to analyze roots.\n");
    }
    if unknown > 0 {
        output.push_str("  UNKNOWN means analysis could not finish a proof.\n");
    }
    if refuted > 0 {
        output.push_str(
            "  REFUTED is a failing translated obligation; runtime confirmation is separate.\n",
        );
    }
    let mut replay_counts = [0_usize; 5];
    for replay in reports
        .iter()
        .flat_map(|report| &report.functions)
        .filter_map(|function| function.proof.as_ref())
        .flat_map(|proof| &proof.obligations)
        .filter_map(|obligation| obligation.replay.as_ref())
    {
        use crate::replay::ReplayStatus;

        let index = match replay.status {
            ReplayStatus::ConfirmedPanic => 0,
            ReplayStatus::NotReproduced => 1,
            ReplayStatus::Unsupported => 2,
            ReplayStatus::ToolFailure => 3,
        };
        replay_counts[index] += 1;
        if replay.status == ReplayStatus::ConfirmedPanic && replay.matches_obligation {
            replay_counts[4] += 1;
        }
    }
    if replay_counts.iter().any(|count| *count > 0) {
        let [panics, returned, unsupported, incomplete, matched] = replay_counts;
        let _ = writeln!(
            output,
            "  Native replay: {panics} panicked ({matched} at matching sites), \
             {returned} did not panic, {unsupported} unsupported, {incomplete} incomplete."
        );
        output.push_str("  Replay evidence does not change proof status or verification exit.\n");
    }
    let stopped = reports
        .iter()
        .flat_map(|report| &report.functions)
        .filter_map(|function| function.proof.as_ref())
        .filter(|proof| proof.stopped_after_counterexample)
        .count();
    if stopped > 0 {
        let _ = writeln!(
            output,
            "  {stopped} root{} stopped after its first counterexample. \
             Re-run verification with --all-failures to continue analysis.",
            if stopped == 1 { "" } else { "s" }
        );
    }
    if assumed > 0 {
        output.push_str(
            "  Assumed proofs are separate from PROVED; strict verification rejects them.\n",
        );
    }
    if !success {
        output.push_str(
            "  Use --verbose for obligations; raw reports retain SMT queries and models.\n",
        );
    }
    output
}

pub fn write_jsonl(reports: &[Report], path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut writer: Box<dyn Write> = if path == Path::new("-") {
        Box::new(std::io::BufWriter::new(std::io::stdout()))
    } else {
        Box::new(std::io::BufWriter::new(std::fs::File::create(path)?))
    };
    for report in reports {
        serde_json::to_writer(&mut writer, report)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

pub fn report_command(args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    let mut color = Color::Auto;
    let mut verbose = false;
    let mut quiet = false;
    let mut allow_assumptions = false;
    let mut jsonl = None;
    let mut paths = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--color" => color = Color::parse(args.next().ok_or("--color needs a value")?)?,
            "--verbose" => verbose = true,
            "--summary" => verbose = false,
            "--quiet" => quiet = true,
            "--allow-assumptions" => allow_assumptions = true,
            "--jsonl" => jsonl = Some(output_path(args.next().ok_or("--jsonl needs a path")?)?),
            "--help" | "-h" => {
                println!(
                    "Usage: mir-check report [--verbose] [--color auto|always|never] \
                    [--quiet] [--allow-assumptions] [--jsonl FILE|-] \
                    <JSON file, JSONL file or directory>..."
                );
                return Ok(true);
            }
            value if value.starts_with("--color=") => color = Color::parse(&value[8..])?,
            value if value.starts_with('-') => {
                return Err(format!("unknown report option {value}").into());
            }
            path => paths.push(PathBuf::from(path)),
        }
    }
    if paths.is_empty() {
        return Err("report needs at least one input file or directory".into());
    }
    let mut inputs = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut files = std::fs::read_dir(path)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<Result<Vec<_>, _>>()?;
            files.sort();
            inputs.extend(files.into_iter().filter(|path| {
                matches!(
                    path.extension().and_then(|value| value.to_str()),
                    Some("json" | "jsonl")
                )
            }));
        } else {
            inputs.push(path);
        }
    }
    let progress = Progress::new(
        !quiet,
        color,
        format!("Reading {} report files", inputs.len()),
    );
    let mut reports = Vec::new();
    for (index, path) in inputs.iter().enumerate() {
        progress.update(format!(
            "Reading [{}/{}] {}",
            index + 1,
            inputs.len(),
            path.display()
        ));
        if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            let reader = std::io::BufReader::new(std::fs::File::open(path)?);
            for (line_number, line) in reader.lines().enumerate() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                reports.push(
                    serde_json::from_str::<Report>(&line).map_err(|error| {
                        format!("{}:{}: {error}", path.display(), line_number + 1)
                    })?,
                );
            }
        } else {
            reports.push(
                serde_json::from_slice::<Report>(&std::fs::read(path)?)
                    .map_err(|error| format!("{}: {error}", path.display()))?,
            );
        }
    }
    if reports.is_empty() {
        return Err("no crate reports found".into());
    }
    for report in &mut reports {
        if !(7..=9).contains(&report.schema_version) {
            return Err(format!("unsupported report schema {}", report.schema_version).into());
        }
        report.coverage = crate::coverage(report);
    }
    let elapsed_s = progress.elapsed();
    drop(progress);
    let success = reports
        .iter()
        .all(|report| accepted(report, allow_assumptions));
    present(
        &reports,
        verbose,
        color,
        jsonl.as_deref(),
        success,
        elapsed_s,
        true,
    )?;
    Ok(success)
}

pub fn present(
    reports: &[Report],
    verbose: bool,
    color: Color,
    jsonl: Option<&Path>,
    success: bool,
    elapsed_s: f64,
    recorded: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let raw_stdout = jsonl == Some(Path::new("-"));
    let terminal = if raw_stdout {
        std::io::stderr().is_terminal()
    } else {
        std::io::stdout().is_terminal()
    };
    let colored = color.enabled(terminal);
    let mut output: Box<dyn Write> = if raw_stdout {
        Box::new(std::io::stderr())
    } else {
        Box::new(std::io::stdout())
    };
    if recorded {
        output.write_all(b"Saved report results; the compiler and solver were not rerun.\n")?;
    }
    for report in reports {
        output.write_all(render_report(report, verbose, colored).as_bytes())?;
    }
    output.write_all(render_totals(reports, success, colored, elapsed_s).as_bytes())?;
    output.flush()?;
    if let Some(path) = jsonl {
        write_jsonl(reports, path)?;
        if !raw_stdout {
            eprintln!("JSONL report: {}", path.display());
        }
    }
    Ok(())
}
