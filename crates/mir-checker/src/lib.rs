#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
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
    pub functions: Vec<Function>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Function {
    pub name: String,
    pub source: Source,
    pub basic_blocks: usize,
    pub arguments: usize,
    pub contracts: Vec<Contract>,
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
    }
    output
}
