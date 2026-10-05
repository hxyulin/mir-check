#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write;

#[derive(Debug, Deserialize, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub compiler: String,
    pub crate_name: String,
    pub target: String,
    pub panic_strategy: String,
    pub overflow_checks: bool,
    pub mir_phase: String,
    pub rustc_arguments: Vec<String>,
    pub functions: Vec<Function>,
    pub traces: Vec<Trace>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Function {
    pub name: String,
    pub source: Source,
    pub basic_blocks: usize,
    pub arguments: usize,
    pub contracts: Vec<Contract>,
    pub sites: Vec<Site>,
    pub local_calls: Vec<LocalCall>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Site {
    pub block: usize,
    pub source: Source,
    pub kind: SiteKind,
    pub detail: String,
    pub failure_condition: Option<String>,
    pub callee: Option<String>,
    pub unwind: Option<String>,
    pub cleanup: bool,
    pub cfg_reachable: bool,
    pub enabled: bool,
    pub status: SiteStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SiteKind {
    BoundsCheck,
    Overflow,
    DivisionByZero,
    RemainderByZero,
    CoroutineState,
    PointerCheck,
    EnumCheck,
    PanicCall,
    ExternalCall,
    UnavailableBody,
    TraitCall,
    IndirectCall,
    Drop,
    InlineAssembly,
}

impl SiteKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::BoundsCheck => "bounds check",
            Self::Overflow => "overflow check",
            Self::DivisionByZero => "division by zero check",
            Self::RemainderByZero => "remainder by zero check",
            Self::CoroutineState => "coroutine state check",
            Self::PointerCheck => "pointer check",
            Self::EnumCheck => "enum validity check",
            Self::PanicCall => "panic entry point",
            Self::ExternalCall => "unknown external call",
            Self::UnavailableBody => "unknown local body",
            Self::TraitCall => "unknown trait dispatch",
            Self::IndirectCall => "unknown indirect call",
            Self::Drop => "unknown destructor",
            Self::InlineAssembly => "unknown inline assembly",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteStatus {
    Unverified,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LocalCall {
    pub block: usize,
    pub callee: String,
    pub source: Source,
    pub cfg_reachable: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Trace {
    pub functions: Vec<String>,
    pub block: usize,
    pub kind: SiteKind,
}

/// One shortest local call path per site and entry, ignoring branch feasibility.
pub fn build_traces(report: &mut Report, entries: &[String]) -> Result<(), String> {
    let functions: BTreeMap<_, _> = report
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect();
    for entry in entries {
        if !functions.contains_key(entry.as_str()) {
            return Err(format!("entry {entry:?} has no inventoried local MIR body"));
        }
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([vec![entry.clone()]]);
        while let Some(path) = queue.pop_front() {
            let name = path.last().expect("paths contain their entry");
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(function) = functions.get(name.as_str()) else {
                continue;
            };
            for site in &function.sites {
                if site.cfg_reachable && site.enabled {
                    report.traces.push(Trace {
                        functions: path.clone(),
                        block: site.block,
                        kind: site.kind,
                    });
                }
            }
            for call in &function.local_calls {
                if call.cfg_reachable && !seen.contains(&call.callee) {
                    let mut next = path.clone();
                    next.push(call.callee.clone());
                    queue.push_back(next);
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Source {
    pub file: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Contract {
    pub kind: ContractKind,
    pub predicate: Option<String>,
    pub status: ContractStatus,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractKind {
    NoPanic,
    Requires,
    Ensures,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractStatus {
    PendingVerification,
}

pub fn render(report: &Report) -> String {
    let mut output = format!(
        concat!(
            "{}: {} MIR bodies (inventory only; no proof)\n  {}\n",
            "  target={} panic={} overflow-checks={}\n"
        ),
        report.crate_name,
        report.functions.len(),
        report.compiler,
        report.target,
        report.panic_strategy,
        report.overflow_checks
    );
    for function in &report.functions {
        let _ = writeln!(
            output,
            "  {} at {}:{}:{} ({} blocks)",
            function.name,
            function.source.file,
            function.source.line,
            function.source.column,
            function.basic_blocks
        );
        for contract in &function.contracts {
            let kind = match contract.kind {
                ContractKind::NoPanic => "no_panic",
                ContractKind::Requires => "requires",
                ContractKind::Ensures => "ensures",
            };
            let predicate = contract.predicate.as_deref().unwrap_or("");
            let _ = writeln!(output, "    {kind}({predicate}): pending verification");
        }
        for site in &function.sites {
            let qualifier = if !site.enabled {
                "disabled by build settings"
            } else if !site.cfg_reachable {
                "structurally unreachable"
            } else {
                "unverified"
            };
            let _ = writeln!(
                output,
                "    bb{} {} at {}:{}:{}: {} ({qualifier})",
                site.block,
                site.kind.label(),
                site.source.file,
                site.source.line,
                site.source.column,
                site.detail
            );
            if let Some(condition) = &site.failure_condition {
                let _ = writeln!(output, "      failure condition: {condition}");
            }
        }
        for call in &function.local_calls {
            let _ = writeln!(output, "    bb{} local call -> {}", call.block, call.callee);
        }
    }
    for trace in &report.traces {
        let _ = writeln!(
            output,
            "  structural path: {} -> bb{} {} (feasibility unverified)",
            trace.functions.join(" -> "),
            trace.block,
            trace.kind.label()
        );
    }
    output
}
