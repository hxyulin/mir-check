#![forbid(unsafe_code)]

pub mod cli;
mod config;
pub mod limits;
pub mod smt;
pub use config::{ContractConfig, FunctionContract};
pub use limits::AnalysisLimits;

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
    pub coverage: Coverage,
    #[serde(default)]
    pub contract_config: Option<ContractConfig>,
    #[serde(default)]
    pub matched_contracts: Vec<String>,
    #[serde(default)]
    pub analysis_limits: Option<AnalysisLimits>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Coverage {
    pub inventoried_bodies: usize,
    pub selected_roots: usize,
    pub proved: usize,
    #[serde(default)]
    pub proved_with_assumptions: usize,
    pub refuted: usize,
    pub unknown: usize,
    pub unselected_bodies: usize,
    pub interpreted_instances: usize,
    pub gaps: Vec<CoverageGap>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CoverageGap {
    pub reason: String,
    pub roots: Vec<String>,
}

pub fn entry_matches(crate_name: &str, function_name: &str, entry: &str) -> bool {
    entry == function_name || entry == format!("{crate_name}::{function_name}")
}

pub fn coverage(report: &Report) -> Coverage {
    let mut result = Coverage {
        inventoried_bodies: report.functions.len(),
        ..Coverage::default()
    };
    let mut instances = BTreeSet::new();
    let mut gaps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for function in &report.functions {
        let Some(proof) = &function.proof else {
            result.unselected_bodies += 1;
            continue;
        };
        result.selected_roots += 1;
        match proof.status {
            ProofStatus::Proved => result.proved += 1,
            ProofStatus::ProvedWithAssumptions => result.proved_with_assumptions += 1,
            ProofStatus::Refuted => result.refuted += 1,
            ProofStatus::Unknown => result.unknown += 1,
        }
        instances.extend(proof.analyzed_bodies.iter());
        for obligation in &proof.obligations {
            if obligation.status == ProofStatus::Unknown {
                gaps.entry(obligation.detail.clone())
                    .or_default()
                    .insert(function.name.clone());
            }
        }
    }
    result.interpreted_instances = instances.len();
    result.gaps = gaps
        .into_iter()
        .map(|(reason, roots)| CoverageGap {
            reason,
            roots: roots.into_iter().collect(),
        })
        .collect();
    result
}

pub fn render_coverage(report: &Report) -> String {
    let coverage = &report.coverage;
    let mut output = format!(
        concat!(
            "{} [{}]: inventoried bodies: {}, selected roots: {}\n",
            "  PROVED {} | REFUTED {} | UNKNOWN {} | unselected {}\n",
            "  {} distinct interpreted instances; counts describe roots, not runtime coverage\n"
        ),
        report.crate_name,
        report.target,
        coverage.inventoried_bodies,
        coverage.selected_roots,
        coverage.proved,
        coverage.refuted,
        coverage.unknown,
        coverage.unselected_bodies,
        coverage.interpreted_instances
    );
    if coverage.proved_with_assumptions > 0 {
        let _ = writeln!(
            output,
            "  PROVED_WITH_ASSUMPTIONS {} (excluded from PROVED)",
            coverage.proved_with_assumptions
        );
    }
    for function in &report.functions {
        if let Some(proof) = &function.proof {
            let _ = writeln!(output, "  {} {}", proof.status.label(), function.name);
        }
    }
    for gap in &coverage.gaps {
        let _ = writeln!(
            output,
            "  gap: {} (roots: {})",
            gap.reason,
            gap.roots.join(", ")
        );
    }
    output
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
    pub proof: Option<Proof>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Proof {
    pub status: ProofStatus,
    #[serde(default)]
    pub stopped_after_counterexample: bool,
    pub assumptions: Vec<String>,
    pub inputs: std::collections::BTreeMap<String, String>,
    pub models: Vec<String>,
    #[serde(default)]
    pub invariants: Vec<String>,
    pub analyzed_bodies: Vec<String>,
    pub obligations: Vec<Obligation>,
    #[serde(default)]
    pub trusted_calls: Vec<TrustedCall>,
    #[serde(default)]
    pub matched_contracts: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TrustedCall {
    pub contract: FunctionContract,
    pub instance: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub crate_hash: String,
    pub source: Source,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProofStatus {
    Proved,
    ProvedWithAssumptions,
    Refuted,
    Unknown,
}

impl ProofStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Proved => "PROVED",
            Self::ProvedWithAssumptions => "PROVED_WITH_ASSUMPTIONS",
            Self::Refuted => "REFUTED",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Obligation {
    pub function: String,
    pub source: Source,
    pub kind: ObligationKind,
    pub detail: String,
    pub status: ProofStatus,
    pub query: Option<String>,
    pub model: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObligationKind {
    PanicSafety,
    Validity,
    CallPrecondition,
    Postcondition,
    Unsupported,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    VerifiedUnderPreconditions,
    VerifiedWithTrustedAssumptions,
}

pub fn render(report: &Report) -> String {
    let mode = if report
        .functions
        .iter()
        .any(|function| function.proof.is_some())
    {
        "verification of selected bodies"
    } else {
        "inventory only; no proof"
    };
    let mut output = format!(
        concat!(
            "{}: {} MIR bodies ({})\n  {}\n",
            "  target={} panic={} overflow-checks={}\n"
        ),
        report.crate_name,
        report.functions.len(),
        mode,
        report.compiler,
        report.target,
        report.panic_strategy,
        report.overflow_checks
    );
    output.push_str(&render_coverage(report));
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
            let status = match contract.status {
                ContractStatus::PendingVerification => "pending verification",
                ContractStatus::VerifiedUnderPreconditions => "verified under preconditions",
                ContractStatus::VerifiedWithTrustedAssumptions => {
                    "verified with trusted external assumptions"
                }
            };
            let _ = writeln!(output, "    {kind}({predicate}): {status}");
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
        if let Some(proof) = &function.proof {
            let _ = writeln!(output, "    verification: {}", proof.status.label());
            if proof.stopped_after_counterexample {
                let _ = writeln!(output, "      stopped after first counterexample");
            }
            for (name, expression) in &proof.inputs {
                let _ = writeln!(output, "      input {name}: {expression}");
            }
            for assumption in &proof.assumptions {
                let _ = writeln!(output, "      assumes: {assumption}");
            }
            for trusted in &proof.trusted_calls {
                let _ = writeln!(
                    output,
                    concat!(
                        "      USER TRUSTED {} {} crate {} at {}:{}; reason: {}; ",
                        "requires {:?}; ensures {:?}; modifies {:?}; returns_alias {:?}"
                    ),
                    trusted.contract.function,
                    trusted.instance,
                    if trusted.crate_hash.is_empty() {
                        "hash unavailable for this build"
                    } else {
                        &trusted.crate_hash
                    },
                    trusted.source.file,
                    trusted.source.line,
                    trusted.contract.reason.as_deref().unwrap_or(""),
                    trusted.contract.requires,
                    trusted.contract.ensures,
                    trusted.contract.modifies,
                    trusted.contract.returns_alias
                );
            }
            for body in &proof.analyzed_bodies {
                let _ = writeln!(output, "      interpreted body: {body}");
            }
            for model in &proof.models {
                let _ = writeln!(output, "      trusted core model: {model}");
            }
            for obligation in &proof.obligations {
                let _ = writeln!(
                    output,
                    "      {} {:?} in {} at {}:{}: {}",
                    obligation.status.label(),
                    obligation.kind,
                    obligation.function,
                    obligation.source.file,
                    obligation.source.line,
                    obligation.detail
                );
                if let Some(model) = &obligation.model {
                    let _ = writeln!(output, "        model: {model}");
                }
            }
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
