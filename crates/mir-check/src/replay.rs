//! Native counterexample execution is evidence, never a replacement for a proof.

use crate::{ObligationKind, ProofStatus, Report};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const COMPILE_TIMEOUT: Duration = Duration::from_secs(30);
const RUN_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_OUTPUT: u64 = 131_072;
static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize, Serialize)]
pub struct ReplayArguments {
    pub arguments: Vec<ReplayArgument>,
    pub unsupported: Option<String>,
    #[serde(default)]
    pub source_files: BTreeMap<String, String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub build_environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReplayArgument {
    pub name: String,
    pub rust_type: String,
    pub value: ReplayValue,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayValue {
    Bool {
        symbol: String,
    },
    Integer {
        symbol: String,
        bits: u32,
        signed: bool,
    },
    Unit,
    Array {
        elements: Vec<ReplayValue>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplayStatus {
    ConfirmedPanic,
    NotReproduced,
    Unsupported,
    ToolFailure,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReplaySource {
    pub file: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReplayResult {
    pub status: ReplayStatus,
    pub detail: String,
    pub inputs: BTreeMap<String, String>,
    pub panic_source: Option<ReplaySource>,
    #[serde(default)]
    pub panic_message: Option<String>,
    pub matches_obligation: bool,
    pub panic_strategy: String,
    /// Query abstractions not set by the generated caller's concrete root inputs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncontrolled_abstractions: Vec<String>,
}

impl ReplayResult {
    fn new(status: ReplayStatus, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: detail.into(),
            inputs: BTreeMap::new(),
            panic_source: None,
            panic_message: None,
            matches_obligation: false,
            panic_strategy: String::new(),
            uncontrolled_abstractions: Vec::new(),
        }
    }
}

/// Execute the first panic-safety counterexample for each refuted root, on explicit request.
/// Generated code and logs are temporary; only structured evidence is retained in the report.
pub fn replay_report(report: &mut Report) {
    let compiler = Path::new(env!("MIR_CHECK_SYSROOT")).join("bin/rustc");
    for function in &mut report.functions {
        let Some(proof) = &mut function.proof else {
            continue;
        };
        let Some(obligation) = proof.obligations.iter_mut().find(|obligation| {
            obligation.status == ProofStatus::Refuted
                && obligation.kind == ObligationKind::PanicSafety
        }) else {
            continue;
        };
        let result = if env!("MIR_CHECK_COMPILER") != report.compiler {
            ReplayResult::new(
                ReplayStatus::Unsupported,
                "native replay requires the compiler recorded in the analysis",
            )
        } else if env!("MIR_CHECK_HOST") != report.target {
            ReplayResult::new(
                ReplayStatus::Unsupported,
                "execution requires the analyzed native target; cross-target replay is unsupported",
            )
        } else if !proof.trusted_calls.is_empty() {
            ReplayResult::new(
                ReplayStatus::Unsupported,
                "trusted boundaries have no native replay implementation",
            )
        } else {
            replay_one(
                ReplayBuild {
                    compiler: &compiler,
                    arguments: &report.rustc_arguments,
                    crate_name: &report.crate_name,
                    function: &function.name,
                    panic_strategy: &report.panic_strategy,
                },
                proof.replay_inputs.as_ref(),
                obligation.model.as_deref(),
            )
        };
        let mut result = result;
        result.panic_strategy = report.panic_strategy.clone();
        result.uncontrolled_abstractions = obligation.abstraction_reasons.clone();
        if let Some(source) = &result.panic_source {
            result.matches_obligation = source.line == obligation.source.line
                && same_file(&source.file, &obligation.source.file);
        }
        obligation.replay = Some(result);
    }
}

fn same_file(left: &str, right: &str) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

struct ReplayBuild<'a> {
    compiler: &'a Path,
    arguments: &'a [String],
    crate_name: &'a str,
    function: &'a str,
    panic_strategy: &'a str,
}

fn replay_one(
    build: ReplayBuild<'_>,
    recipe: Option<&ReplayArguments>,
    model: Option<&str>,
) -> ReplayResult {
    let unsupported = |detail| ReplayResult::new(ReplayStatus::Unsupported, detail);
    if !["abort", "unwind"].contains(&build.panic_strategy) {
        return unsupported("native replay does not support this panic strategy");
    }
    let Some(recipe) = recipe else {
        return unsupported("this report has no typed replay inputs");
    };
    if let Some(reason) = &recipe.unsupported {
        return unsupported(reason);
    }
    if recipe.source_files.is_empty() {
        return unsupported("the report has no original Rust source fingerprints");
    }
    if recipe
        .working_directory
        .as_ref()
        .is_none_or(|path| !Path::new(path).is_dir())
    {
        return unsupported("the original compiler working directory is unavailable");
    }
    for (file, expected) in &recipe.source_files {
        match fs::read_to_string(file) {
            Ok(source) if source_fingerprint(&source) == *expected => {}
            Ok(_) => return unsupported("original Rust source changed after analysis"),
            Err(_) => return unsupported("original Rust source is no longer available"),
        }
    }
    if syn::parse_str::<syn::Path>(build.function).is_err() || build.function.contains('<') {
        return unsupported("only accessible, nongeneric free-function paths can be replayed");
    }
    let values = match counterexample_inputs(recipe, model.unwrap_or("()")) {
        Ok(values) => recipe
            .arguments
            .iter()
            .map(|argument| (argument.name.clone(), values[&argument.name].clone()))
            .collect::<Vec<_>>(),
        Err(reason) => return unsupported(&reason),
    };
    let directory = match TempDirectory::create() {
        Ok(directory) => directory,
        Err(error) => return ReplayResult::new(ReplayStatus::ToolFailure, error.to_string()),
    };
    let result = execute(&build, &values, recipe, &directory.0);
    let mut result = match result {
        Ok(result) => result,
        Err(error) => ReplayResult::new(ReplayStatus::ToolFailure, error),
    };
    result.inputs = values.into_iter().collect();
    result
}

/// Decode only concrete assignments from the retained model; this never executes analyzed code.
pub fn counterexample_inputs(
    recipe: &ReplayArguments,
    model: &str,
) -> Result<BTreeMap<String, String>, String> {
    if let Some(reason) = &recipe.unsupported {
        return Err(reason.clone());
    }
    let bindings = parse_model(model)?;
    let mut values = BTreeMap::new();
    for argument in &recipe.arguments {
        let value = argument.value.literal(&bindings, &argument.rust_type)?;
        if values.insert(argument.name.clone(), value).is_some() {
            return Err("duplicate replay argument name".to_owned());
        }
    }
    Ok(values)
}

/// Consistency fingerprint for normalized Rust source, not a cryptographic trust boundary.
pub fn source_fingerprint(source: &str) -> String {
    let normalized = source.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let hash = normalized
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    format!("fnv1a:{hash:016x}:{}", normalized.len())
}

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn create() -> std::io::Result<Self> {
        let sequence = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mir-check-replay-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn execute(
    build: &ReplayBuild<'_>,
    values: &[(String, String)],
    recipe: &ReplayArguments,
    directory: &Path,
) -> Result<ReplayResult, String> {
    let library = directory.join("libreplayed.rlib");
    let mut command = Command::new(build.compiler);
    configure_environment(&mut command, recipe);
    command
        .args(library_arguments(build.arguments)?)
        .args(["--crate-type=rlib", "--crate-name", build.crate_name])
        .arg(format!("-Cpanic={}", build.panic_strategy))
        .arg("-o")
        .arg(&library);
    let output = bounded_run(&mut command, directory, "library", COMPILE_TIMEOUT)?;
    if !output.status.success() {
        return Err(format!(
            "original source cannot compile for native replay: {}",
            output.stderr
        ));
    }
    let literals = values
        .iter()
        .map(|(_, literal)| literal.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let source = directory.join("replay.rs");
    let marker = format!(
        "MIR_CHECK_REPLAY_{}_{}",
        std::process::id(),
        NEXT_RUN.fetch_add(1, Ordering::Relaxed)
    );
    let function = build.function;
    let after_hook = if build.panic_strategy == "abort" {
        format!("eprintln!(\"{marker}|ABORT_PANIC\"); std::process::exit(101);")
    } else {
        String::new()
    };
    fs::write(
        &source,
        format!(
            r#"#![forbid(unsafe_code)]
extern crate replayed;
fn main() {{
    std::panic::set_hook(Box::new(|info| {{
        if let Some(location) = info.location() {{
            eprintln!("{marker}|PANIC|{{}}|{{}}|{{}}",
                location.line(), location.column(), location.file());
        }}
        let message = info.payload().downcast_ref::<&str>().copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        let message: String = message.chars().take(512).map(|character| {{
            if character == '\n' || character == '\r' {{ ' ' }} else {{ character }}
        }}).collect();
        eprintln!("{marker}|MESSAGE|{{message}}");
        {after_hook}
    }}));
    let result = std::panic::catch_unwind(|| {{
        let _returned = std::mem::ManuallyDrop::new(replayed::{function}({literals}));
    }});
    if result.is_err() {{
        eprintln!("{marker}|CAUGHT");
        std::process::exit(101);
    }}
    eprintln!("{marker}|RETURNED");
}}
"#
        ),
    )
    .map_err(|error| error.to_string())?;
    let binary = directory.join("replay");
    let mut command = Command::new(build.compiler);
    configure_environment(&mut command, recipe);
    command
        .arg(&source)
        .args([
            "--edition=2024",
            "--crate-name=mir_check_replay",
            "--extern",
        ])
        .arg(format!("replayed={}", library.display()))
        .args(dependency_arguments(build.arguments))
        .arg(format!("-Cpanic={}", build.panic_strategy))
        .arg("-o")
        .arg(&binary);
    // Dependencies are resolved using the original invocation's search paths, below.
    let output = bounded_run(&mut command, directory, "harness", COMPILE_TIMEOUT)?;
    if !output.status.success() {
        return Err(format!(
            "generated harness cannot compile: {}",
            output.stderr
        ));
    }
    let mut command = Command::new(&binary);
    configure_environment(&mut command, recipe);
    let output = bounded_run(&mut command, directory, "execution", RUN_TIMEOUT)?;
    let caught = output
        .stderr
        .lines()
        .any(|line| line == format!("{marker}|CAUGHT"));
    let abort_panic = output
        .stderr
        .lines()
        .any(|line| line == format!("{marker}|ABORT_PANIC"));
    let returned = output
        .stderr
        .lines()
        .any(|line| line == format!("{marker}|RETURNED"));
    let panic_source = output.stderr.lines().find_map(|line| {
        let tail = line.strip_prefix(&format!("{marker}|PANIC|"))?;
        let mut fields = tail.splitn(3, '|');
        Some(ReplaySource {
            line: fields.next()?.parse().ok()?,
            column: fields.next()?.parse().ok()?,
            file: fields.next()?.to_owned(),
        })
    });
    let protocol_panic = if build.panic_strategy == "abort" {
        abort_panic
    } else {
        caught
    };
    if protocol_panic && panic_source.is_some() && output.status.code() == Some(101) {
        let mut result = ReplayResult::new(
            ReplayStatus::ConfirmedPanic,
            "the native function panicked with the retained counterexample inputs",
        );
        result.panic_source = panic_source;
        result.panic_message = output.stderr.lines().find_map(|line| {
            line.strip_prefix(&format!("{marker}|MESSAGE|"))
                .map(str::to_owned)
        });
        Ok(result)
    } else if returned && output.status.success() {
        Ok(ReplayResult::new(
            ReplayStatus::NotReproduced,
            "the native function returned without panic; the abstract failure remains REFUTED",
        ))
    } else {
        Err(format!(
            "execution did not complete the replay protocol ({})",
            output.status
        ))
    }
}

fn configure_environment(command: &mut Command, recipe: &ReplayArguments) {
    if let Some(directory) = &recipe.working_directory {
        command.current_dir(directory);
    }
    command.envs(&recipe.build_environment);
}

fn library_arguments(arguments: &[String]) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut source_count = 0;
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        if [
            "--crate-type",
            "--crate-name",
            "--out-dir",
            "--emit",
            "-o",
            "--target",
            "--error-format",
            "--json",
            "--color",
        ]
        .contains(&argument.as_str())
        {
            arguments.next().ok_or("missing compiler argument value")?;
        } else if [
            "--crate-type=",
            "--crate-name=",
            "--out-dir=",
            "--emit=",
            "--target=",
            "--error-format=",
            "--json=",
            "--color=",
        ]
        .iter()
        .any(|prefix| argument.starts_with(prefix))
            || argument == "--test"
        {
            continue;
        } else if argument == "-C" {
            let option = arguments.next().ok_or("missing codegen argument")?;
            if !option.starts_with("panic=") && !option.starts_with("extra-filename=") {
                result.extend([argument.clone(), option.clone()]);
            }
        } else if argument.starts_with("-Cpanic=") || argument.starts_with("-Cextra-filename=") {
            continue;
        } else {
            source_count += usize::from(!argument.starts_with('-') && argument.ends_with(".rs"));
            result.push(argument.clone());
        }
    }
    if source_count != 1 {
        return Err("replay requires one original Rust source file".to_owned());
    }
    Ok(result)
}

fn dependency_arguments(arguments: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        if argument == "-L" || argument == "--sysroot" {
            result.push(argument.clone());
            if let Some(value) = arguments.next() {
                result.push(value.clone());
            }
        } else if argument.starts_with("-L") || argument.starts_with("--sysroot=") {
            result.push(argument.clone());
        }
    }
    result
}

struct ProcessOutput {
    status: ExitStatus,
    stderr: String,
}

fn bounded_run(
    command: &mut Command,
    directory: &Path,
    name: &str,
    timeout: Duration,
) -> Result<ProcessOutput, String> {
    let stdout_path = directory.join(format!("{name}.stdout"));
    let stderr_path = directory.join(format!("{name}.stderr"));
    command
        .stdin(Stdio::null())
        .stdout(File::create(&stdout_path).map_err(|error| error.to_string())?)
        .stderr(File::create(&stderr_path).map_err(|error| error.to_string())?);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
        let output_bytes = [&stdout_path, &stderr_path]
            .iter()
            .try_fold(0_u64, |sum, path| {
                fs::metadata(path).map(|metadata| sum.saturating_add(metadata.len()))
            });
        let output_bytes = match output_bytes {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        };
        if started.elapsed() >= timeout || output_bytes > MAX_OUTPUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{name} exceeded its time or output limit"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let output_bytes = [&stdout_path, &stderr_path]
        .iter()
        .try_fold(0_u64, |sum, path| {
            fs::metadata(path).map(|metadata| sum.saturating_add(metadata.len()))
        })
        .map_err(|error| error.to_string())?;
    if output_bytes > MAX_OUTPUT {
        return Err(format!("{name} exceeded its output limit"));
    }
    let mut stderr = String::new();
    File::open(stderr_path)
        .map_err(|error| error.to_string())?
        .take(MAX_OUTPUT)
        .read_to_string(&mut stderr)
        .map_err(|error| error.to_string())?;
    Ok(ProcessOutput { status, stderr })
}

#[derive(Debug)]
enum Expression {
    Atom(String),
    List(Vec<Expression>),
}

struct ModelValue {
    sort: Expression,
    value: Expression,
}

fn parse_model(model: &str) -> Result<BTreeMap<String, ModelValue>, String> {
    if model.len() > MAX_OUTPUT as usize {
        return Err("model exceeds replay limit".to_owned());
    }
    let tokens = model.replace('(', " ( ").replace(')', " ) ");
    let tokens: Vec<_> = tokens.split_whitespace().collect();
    let mut cursor = 0;
    let expression = expression(&tokens, &mut cursor, 0)?;
    if cursor != tokens.len() {
        return Err("model has trailing tokens".to_owned());
    }
    let Expression::List(definitions) = expression else {
        return Err("invalid model".to_owned());
    };
    let mut bindings = BTreeMap::new();
    for definition in definitions {
        let Expression::List(mut fields) = definition else {
            continue;
        };
        if fields.len() != 5 || atom(&fields[0]) != Some("define-fun") {
            continue;
        }
        let name = atom(&fields[1]).ok_or("invalid model symbol")?.to_owned();
        if !matches!(&fields[2], Expression::List(arguments) if arguments.is_empty()) {
            continue;
        }
        let value = fields.pop().ok_or("missing model value")?;
        let sort = fields.pop().ok_or("missing model sort")?;
        if bindings.insert(name, ModelValue { sort, value }).is_some() {
            return Err("duplicate model symbol".to_owned());
        }
    }
    Ok(bindings)
}

fn expression(tokens: &[&str], cursor: &mut usize, depth: usize) -> Result<Expression, String> {
    if depth > 128 {
        return Err("model expression depth exceeds replay limit".to_owned());
    }
    let token = *tokens.get(*cursor).ok_or("incomplete model expression")?;
    *cursor += 1;
    match token {
        "(" => {
            let mut fields = Vec::new();
            loop {
                if tokens.get(*cursor) == Some(&")") {
                    *cursor += 1;
                    break;
                }
                fields.push(expression(tokens, cursor, depth + 1)?);
            }
            Ok(Expression::List(fields))
        }
        ")" => Err("unexpected closing model token".to_owned()),
        token => Ok(Expression::Atom(token.to_owned())),
    }
}

fn atom(expression: &Expression) -> Option<&str> {
    match expression {
        Expression::Atom(atom) => Some(atom),
        Expression::List(_) => None,
    }
}

impl ReplayValue {
    fn literal(
        &self,
        model: &BTreeMap<String, ModelValue>,
        rust_type: &str,
    ) -> Result<String, String> {
        match self {
            Self::Bool { symbol } => {
                if rust_type != "bool" {
                    return Err("invalid Boolean replay type".to_owned());
                }
                let value = model.get(symbol).ok_or("missing Boolean model input")?;
                if atom(&value.sort) != Some("Bool") {
                    return Err("model sort differs from Boolean replay input".to_owned());
                }
                match atom(&value.value) {
                    Some("true") => Ok("true".to_owned()),
                    Some("false") => Ok("false".to_owned()),
                    _ => Err(format!("model has no concrete Boolean input {symbol}")),
                }
            }
            Self::Integer {
                symbol,
                bits,
                signed,
            } => {
                let expected = format!("{}{bits}", if *signed { 'i' } else { 'u' });
                let pointer = if *signed { "isize" } else { "usize" };
                if ![8, 16, 32, 64, 128].contains(bits)
                    || (rust_type != expected && rust_type != pointer)
                    || (rust_type == pointer && *bits != usize::BITS)
                {
                    return Err("model integer type differs from replay input".to_owned());
                }
                let binding = model
                    .get(symbol)
                    .ok_or_else(|| format!("model has no concrete input {symbol}"))?;
                if !bitvector_sort(&binding.sort, *bits) {
                    return Err("model sort differs from integer replay input".to_owned());
                }
                let (value, width) = integer(&binding.value)?;
                if width != *bits {
                    return Err("model integer width differs from input".to_owned());
                }
                if *signed && value & (1_u128 << (bits - 1)) != 0 {
                    let mask = if *bits == 128 {
                        u128::MAX
                    } else {
                        (1_u128 << bits) - 1
                    };
                    let magnitude = (!value & mask).wrapping_add(1);
                    Ok(format!("-{magnitude}{rust_type}"))
                } else {
                    Ok(format!("{value}{rust_type}"))
                }
            }
            Self::Unit if rust_type == "()" => Ok("()".to_owned()),
            Self::Unit => Err("invalid unit replay type".to_owned()),
            Self::Array { elements } => {
                let ty =
                    syn::parse_str::<syn::Type>(rust_type).map_err(|error| error.to_string())?;
                let syn::Type::Array(array) = ty else {
                    return Err("invalid array input".to_owned());
                };
                let syn::Expr::Lit(length) = array.len else {
                    return Err("nonconstant replay array length".to_owned());
                };
                let syn::Lit::Int(length) = length.lit else {
                    return Err("invalid replay array length".to_owned());
                };
                if length
                    .base10_parse::<usize>()
                    .map_err(|error| error.to_string())?
                    != elements.len()
                    || elements.len() > 128
                {
                    return Err("replay array shape differs from its type".to_owned());
                }
                let element_type = match *array.elem {
                    syn::Type::Path(path) => path
                        .path
                        .get_ident()
                        .map(ToString::to_string)
                        .ok_or("nonprimitive replay array element")?,
                    _ => return Err("nonprimitive replay array element".to_owned()),
                };
                let elements = elements
                    .iter()
                    .map(|value| value.literal(model, &element_type))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(format!("[{}]", elements.join(", ")))
            }
        }
    }
}

fn bitvector_sort(sort: &Expression, width: u32) -> bool {
    matches!(sort, Expression::List(fields) if fields.len() == 3
        && atom(&fields[0]) == Some("_") && atom(&fields[1]) == Some("BitVec")
        && atom(&fields[2]).and_then(|value| value.parse::<u32>().ok()) == Some(width))
}

fn integer(expression: &Expression) -> Result<(u128, u32), String> {
    if let Some(atom) = atom(expression) {
        if let Some(hex) = atom.strip_prefix("#x") {
            return u128::from_str_radix(hex, 16)
                .map(|value| (value, hex.len() as u32 * 4))
                .map_err(|error| error.to_string());
        }
        if let Some(binary) = atom.strip_prefix("#b") {
            return u128::from_str_radix(binary, 2)
                .map(|value| (value, binary.len() as u32))
                .map_err(|error| error.to_string());
        }
    }
    if let Expression::List(fields) = expression
        && fields.len() == 3
        && atom(&fields[0]) == Some("_")
    {
        let value = atom(&fields[1])
            .and_then(|value| value.strip_prefix("bv"))
            .ok_or("unsupported bit-vector model")?
            .parse::<u128>()
            .map_err(|error| error.to_string())?;
        let width = atom(&fields[2])
            .ok_or("missing bit-vector width")?
            .parse::<u32>()
            .map_err(|error| error.to_string())?;
        if width == 0 || width > 128 || (width < 128 && value >= 1_u128 << width) {
            return Err("invalid bit-vector model value".to_owned());
        }
        return Ok((value, width));
    }
    Err("nonconstant bit-vector model remains unsupported".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_uses_only_concrete_retained_model_values() {
        let model = parse_model(
            "((define-fun v0 () (_ BitVec 8) #xff) \
            (define-fun v1 () Bool true) (define-fun v2 () (_ BitVec 16) (_ bv258 16)))",
        )
        .unwrap();
        let signed = ReplayValue::Integer {
            symbol: "v0".to_owned(),
            bits: 8,
            signed: true,
        };
        assert_eq!(signed.literal(&model, "i8").unwrap(), "-1i8");
        assert!(signed.literal(&model, "u8").is_err());
        let boolean = ReplayValue::Bool {
            symbol: "v1".to_owned(),
        };
        assert_eq!(boolean.literal(&model, "bool").unwrap(), "true");
        let missing = ReplayValue::Integer {
            symbol: "v9".to_owned(),
            bits: 8,
            signed: false,
        };
        assert!(missing.literal(&model, "u8").is_err());
        let wrong_width = ReplayValue::Integer {
            symbol: "v2".to_owned(),
            bits: 8,
            signed: false,
        };
        assert!(wrong_width.literal(&model, "u8").is_err());
        assert!(parse_model("((define-fun v0 () Bool true)) trailing").is_err());
        assert!(parse_model("((define-fun v0 () Bool true)").is_err());
        assert!(
            parse_model("((define-fun v0 () Bool true) (define-fun v0 () Bool false))").is_err()
        );
        let malformed = parse_model("((define-fun v0 () Bool #xff))").unwrap();
        assert!(signed.literal(&malformed, "i8").is_err());
        assert!(boolean.literal(&model, "bool; panic!()").is_err());
    }

    #[test]
    fn compiler_replay_preserves_settings_with_an_explicit_panic_strategy() {
        let arguments = [
            "sample.rs",
            "--crate-type=bin",
            "--target",
            "foreign-target",
            "-Cpanic=abort",
            "-C",
            "overflow-checks=yes",
            "--cfg",
            "feature=\"sample\"",
            "--extern",
            "helper=/tmp/helper.rlib",
            "-L",
            "dependency=/tmp/deps",
            "-o",
            "old",
        ]
        .map(str::to_owned);
        let normalized = library_arguments(&arguments).unwrap();
        assert_eq!(
            normalized,
            [
                "sample.rs",
                "-C",
                "overflow-checks=yes",
                "--cfg",
                "feature=\"sample\"",
                "--extern",
                "helper=/tmp/helper.rlib",
                "-L",
                "dependency=/tmp/deps"
            ]
        );
        assert_eq!(
            dependency_arguments(&arguments),
            ["-L", "dependency=/tmp/deps"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_timeout_and_abnormal_exit_are_tool_failures() {
        let directory = TempDirectory::create().unwrap();
        let error = bounded_run(
            Command::new("/bin/sh").args(["-c", "while :; do :; done"]),
            &directory.0,
            "timeout",
            Duration::from_millis(40),
        )
        .err()
        .unwrap();
        assert!(error.contains("exceeded"));
        let output = bounded_run(
            Command::new("/bin/sh").args(["-c", "exit 101"]),
            &directory.0,
            "abnormal",
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(output.status.code(), Some(101));
        assert!(output.stderr.is_empty());
    }
}
