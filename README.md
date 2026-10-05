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

A Rust MIR analyzer for panic freedom and function contracts. It uses symbolic execution and Z3
to check supported paths, including bounds passed between functions. Contract annotations add
metadata without runtime assertions. The analyzer runs on the host and can check no_std code for
an embedded target.

This is an experimental side project with a pinned nightly compiler and a limited supported
subset. Unsupported behavior returns UNKNOWN and fails verification. The interpreter and its
library models have not completed a soundness audit.

## Try a real parser

Install Rust through rustup and have Python 3 available. The toolchain file pins
nightly-2026-09-22, including rustc-dev, LLVM tools and the ARM target. Cargo installs that
compiler when needed; the analyzer must be built with its exact compiler ABI.

```sh
git clone https://github.com/hxyulin/mir-check.git
cd mir-check
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements-solver.txt
cargo build --workspace --locked

target/debug/cargo-mir-check --verify --summary --entry Raw::parse \
  --manifest-path examples/dr16/Cargo.toml --lib --locked
```

The unchanged DR16 parser proves without entry preconditions: it cannot panic for a valid input
byte slice, accepts exactly 18 bytes, and establishes the decoded switch and channel bounds.
Repeat the command with `--target thumbv7em-none-eabihf` to analyze the ARM build.

`--summary` shows root outcomes, unselected bodies, interpreted instances and grouped reasons for
UNKNOWN. Omit it for the full inventory, obligations, assumptions and solver models. Per-crate
JSON reports are written under `target/mir-check/<run>/reports`, including failed verification.
These counts describe analysis roots, not runtime test coverage or the safety of unselected code.

| Result | What it tells you |
| --- | --- |
| **PROVED** | Every feasible path completed and every obligation passed for the selected root domain |
| **REFUTED** | The solver found a failing assignment for a translated obligation |
| **UNKNOWN** | Unsupported behavior, a missing body or a limit prevented a complete proof |

Only a run where every selected root proves succeeds. A solver assignment is not automatically
a confirmed Rust failure; mutation and replay tests provide independent evidence for fixtures.

## Use it in a project

Install both command-line binaries from this checkout:

```sh
cargo install --path crates/mir-check --locked
```

From the project you want to analyze:

```sh
cargo mir-check --lib
cargo mir-check --verify --summary --entry module::function --lib
cargo mir-check --verify --summary --entry my_crate::module::function --workspace --lib
```

Entries match exact function names from the inventory, optionally prefixed with the Rust crate
name. Repeat `--entry` to check several roots. An unqualified name selects every matching body in
Cargo's selected targets. A name that matches no body fails; unrelated crates retain their
inventories without being verified. Without entries, `--verify` checks every inventoried body.

The checker follows reachable callee bodies and verifies their preconditions even when those
callees are not independently selected as roots. Selecting a root does not verify its callers.
Cargo target, features, profile, panic strategy and overflow settings remain part of the result.
Each run uses a fresh build directory so Cargo caching cannot silently skip analysis.

Z3 is pinned in `requirements-solver.txt`. The development binary finds this checkout's
`.venv/bin/z3`; an installed binary can also use it while the checkout remains present. Otherwise
put Z3 on PATH or set `MIR_CHECK_Z3` to its executable. Inventory mode does not need a solver.

See [the usage guide](docs/usage.md) for workspace selection, ARM examples, failed outcomes and
JSON report interpretation.

## Declare contracts

Panic checking does not require annotations. For preconditions and postconditions, add the
metadata-only crate:

```toml
[dependencies]
mir-contracts = { git = "https://github.com/hxyulin/mir-check" }
```

A local path to `crates/mir-contracts` also works. The macros compile with stable Rust and no_std
consumers; only the analyzer needs the pinned nightly.

```rust
use mir_contracts::{ensures, no_panic, requires};

#[no_panic]
#[requires(value < 15)]
#[ensures(result < 16)]
pub fn increment(value: u8) -> u8 {
    value + 1
}
```

`requires` defines a root's input domain and an obligation at each analyzed call. The checker
executes the callee body with actual arguments; annotations are never trusted as summaries.
`ensures` must hold at every feasible return. Parameter names retain their entry values, and
`result` denotes the actual return value. Attributes do not add runtime protection against a
caller violating the declared domain.

Predicates support comparisons, boolean operations, modeled fields and tuple projections,
array/slice lengths, constant indices into fixed non-byte arrays, integer casts and restricted
exhaustive Option matches. Unsupported predicates and inconsistent entry domains return UNKNOWN.
See [the contract example](examples/contracts/README.md) for guarded calls and a nested packet
containing a shared byte slice.

## Current coverage

| Area | Supported subset |
| --- | --- |
| Inputs | Integers, f32/f64, bool, unit, byte slices/arrays, tuples, nested local/dependency structs and enums, shared references and small arrays |
| Arithmetic | Exact integer operations; IEEE f32/f64 arithmetic, comparisons and saturating casts |
| Calls | Concrete generics, static traits, available dependency MIR, function items and read-only closures |
| Control flow | Feasible branches, symbolic enum tags/payloads, Option/Result propagation and completely unrolled finite loops |
| Library models | Byte ranges/copies/conversions, endian decoding, fixed-array map, float abs/min/max and static formatting arguments |
| Contracts | Caller bounds, entry preconditions and postconditions on actual returns |

Integer/bool/float arrays support symbolic bounded indices and array/slice pattern projections.
Tuple/struct/enum array elements require a uniquely determined index on the current path. Root
struct fields are independent inputs; privacy and constructors do not supply an implicit type
invariant. Nested input construction is limited to eight levels and 128 values; non-byte arrays have
at most 16 elements and input enums have at most 16 variants. All variant payloads must have
supported shapes.

General mutation and aliasing, mutable captures, enum/struct slices, unresolved generic inputs,
float remainder and bit observation, dynamic dispatch, function pointers, destructor execution, some
aggregate/promoted constants and broader iterator machinery remain gaps. Limits and unsupported
operations produce UNKNOWN. A selected-root proof also does not establish absence of undefined
behavior, allocation failure, stack exhaustion, interrupt races or hardware timing failures.

See [the coverage matrix](docs/coverage.md) for evidence and limits, and
[proof execution](docs/proofs.md) for how obligations are generated and what the result trusts.
JSON schema version 7 includes per-root proofs and a per-crate coverage summary. Inventory sites
keep their separate unverified status even when a selected root proves their paths safe.

The [fleet workspace survey](docs/fleet-survey.md) checks unchanged shared crates, board support
and robot firmware. It separates generated MIR roots from function declarations, records the
build configuration and explains why the current tool cannot prove whole control loops.

## Real-code fixtures

The fixtures retain original firmware bodies and test their tokens against source snapshots.
The source firmware repository is independent of this project and needs no added dependency.

| Fixture | What is proved on host and ARM |
| --- | --- |
| [CAN frames](examples/can-frame/README.md) | Six constructors/accessors and two payload-preservation harnesses |
| [Bus validator](examples/can-frame/README.md) | Two symbolic three-device families through nested loops and enum matches |
| [DR16 parser](examples/dr16/README.md) | Panic freedom, exact-length acceptance, switch bounds and five channel bounds |

Negative cases refute invalid call bounds, changed guards, payload-copy errors, invalid bus
configurations and parser-index/mask errors. Runtime checks independently replay failures and
compare decoding formulas. These are proofs of selected roots or configuration families, not of
entire firmware crates. The CAN fixture deliberately includes failing roots.

## Development

Bug reports, small real-code examples and contributions are welcome. See
[CONTRIBUTING.md](CONTRIBUTING.md) for useful reports and implementation expectations.

```sh
cargo fmt --all --check
cargo lint
cargo test --workspace --locked
cargo build --workspace --release --locked
cargo deny check
prek run --all-files --stage manual
```

GitHub Actions runs proofs and runtime tests on Linux and macOS, including ARM proofs and fixture
release builds. See [docs](docs/README.md) for the guides and staged implementation evidence.

The searchable documentation site uses the same Markdown guides. To work on it with Node.js 22
or newer:

```sh
npm ci
npm run docs:dev
npm run docs:build
```

See [documentation development](docs/development.md) for previews and GitHub Pages deployment.

## License

mir-check is available under either the [MIT license](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option.
