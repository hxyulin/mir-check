<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/public/mark-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="docs/public/mark.svg">
    <img src="docs/public/mark.svg" alt="mir-check logo" width="128" height="128">
  </picture>
</p>
<h1 align="center">mir-check</h1>
<p align="center">Panic freedom and function contracts, checked from Rust MIR.</p>
<p align="center">
  <a href="https://github.com/hxyulin/mir-check/actions/workflows/ci.yml">
    <img alt="CI status"
      src="https://github.com/hxyulin/mir-check/actions/workflows/ci.yml/badge.svg?branch=main">
  </a>
  <a href="https://github.com/hxyulin/mir-check/actions/workflows/docs.yml">
    <img alt="Documentation status"
      src="https://github.com/hxyulin/mir-check/actions/workflows/docs.yml/badge.svg?branch=main">
  </a>
  <a href="rust-toolchain.toml">
    <img alt="Rust nightly 2026-09-22"
      src="https://img.shields.io/badge/Rust-nightly--2026--09--22-dea584?logo=rust">
  </a>
  <a href="docs/proofs.md">
    <img alt="Status: experimental"
      src="https://img.shields.io/badge/status-experimental-d4a24a">
  </a>
  <a href="#license">
    <img alt="MIT or Apache-2.0"
      src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue">
  </a>
</p>
<p align="center">
  <a href="https://hxyulin.github.io/mir-check/">Documentation</a> ·
  <a href="https://hxyulin.github.io/mir-check/usage.html">Getting started</a> ·
  <a href="https://hxyulin.github.io/mir-check/examples.html">Examples</a> ·
  <a href="https://hxyulin.github.io/mir-check/fleet-survey.html">Fleet survey</a>
</p>

A Rust MIR analyzer for panic freedom and function contracts. It follows actual function calls
with symbolic inputs and asks Z3 to check the resulting obligations. Contracts add metadata
without runtime assertions. Analysis runs on the host against your Cargo target and profile,
including embedded no_std builds.

This is an experimental side project. It requires a pinned nightly compiler, supports part of
Rust, and has not completed a soundness audit. Unsupported operations and unfinished proofs return
UNKNOWN. A solver counterexample is not automatically a confirmed runtime panic.

## Install

The toolchain file installs the matching nightly, compiler development components and ARM target.

```sh
git clone https://github.com/hxyulin/mir-check.git
cd mir-check
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements-solver.txt
cargo install --path crates/mir-check --locked
```

Z3 must remain available in the checkout's `.venv`, on PATH, or through `MIR_CHECK_Z3`.
The installed binaries also require their pinned rustc sysroot.

## Check a project

Run from the Cargo project you want to analyze:

```sh
mir-check --entry module::function --lib
mir-check --async-entry task --bin firmware
mir-check report
```

Verification is the default. Cargo supplies dependency metadata, environment variables, target,
features and profile; there is no compiler invocation or saved report to prepare. `cargo mir-check`
uses the same workflow. Without an entry, every inventoried local body is selected, including
generated bodies that may require unsupported inputs. Use `mir-check inventory --verbose` to find
exact names before selecting a smaller root.

An optional `mir-check.json` stores settings you want to repeat:

```json
{
  "schema_version": 1,
  "entries": ["module::function"],
  "cargo_args": ["--lib"],
  "limits": {"max_call_depth": 32}
}
```

Then run `mir-check`. Explicit entry options replace configured roots; CLI limit options override
configured limits. There are no built-in presets or named profiles. See the
[configuration guide](docs/usage.md#project-configuration) for precedence and supported fields.

Analysis reuses dependency builds but refreshes workspace artifacts and proof results on every
run. It keeps its own build cache and run reports beneath the Cargo workspace's
`target/mir-check`. `mir-check report` reads the latest attempted run without rebuilding or
rerunning Z3; interrupted and failed runs cannot fall back to an older successful scan.

## Try the parser example

From this checkout:

```sh
cargo build --workspace --locked
target/debug/mir-check --manifest-path examples/dr16/Cargo.toml
```

The example's project configuration selects `Raw::parse`. Its arbitrary byte-slice inputs prove
without entry preconditions. Add `--target thumbv7em-none-eabihf` to check the ARM build.

## Read the result

| Result | Meaning |
| --- | --- |
| **PROVED** | Every reached path completed and every obligation passed for the recorded root domain |
| **PROVED_WITH_ASSUMPTIONS** | Completed under listed assumptions; acceptance requires `--allow-assumptions` |
| **REFUTED** | A translated obligation has a solver counterexample; runtime evidence is reported separately |
| **UNKNOWN** | A missing model, unavailable body or resource limit prevented a complete proof |

The CLI names failing conditions, locations and call chains, and explains the next step for UNKNOWN.
Slow checks report their active root and elapsed time. Colors supplement labels; `--quiet`,
`--verbose` and `--color auto|always|never` control display. Counts describe selected MIR roots,
not line coverage or the safety of unselected code.

```sh
mir-check --entry decode --lib --jsonl results.jsonl
mir-check report results.jsonl --verbose
```

`--jsonl -` emits machine-readable reports on stdout and keeps human messages on stderr.
Native counterexample execution remains opt-in through `--replay`. Startup assumptions,
experimental loop induction and trusted boundaries require explicit settings. Raw rustc and
`--from-report` workflows remain available for compiler debugging.

## Documentation and development

- [Usage and configuration](docs/usage.md): installation, project checks and advanced commands.
- [Coverage](docs/coverage.md): supported language features, limitations and mutation evidence.
- [Contracts](docs/contracts.md): metadata and explicit trusted boundaries.
- [Proofs](docs/proofs.md): symbolic execution, induction and counterexample interpretation.
- [Examples](docs/examples.md): independently testable parser, frame and contract cases.
- [Development](docs/development.md): formatting, linting, tests, builds and documentation checks.
- [Analyzer design](docs/analyzer-redesign.md): remaining storage, call and proof work.

## License

Available under either the [MIT license](LICENSE-MIT) or [Apache License 2.0](LICENSE-APACHE).
