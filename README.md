# mir-checker

A standalone Rust static-analysis experiment using typed MIR from rustc. Panic analysis is the
first goal; verified contracts and restricted effects are intended extensions. This repository
is independent of fleet-2027 and installs no firmware dependencies.

Stages 1 and 2 provide a compiler adapter, a Cargo command, metadata-only contract attributes
and a panic inventory. The tool enumerates checks, panic entry points and unresolved calls in
local function bodies, including uncalled generics. Contracts remain pending verification.
It does not prove panic freedom or any contract.

## Build and use

The toolchain file pins nightly-2026-09-22, including rustc-dev and LLVM tooling. The compiler
adapter uses unstable APIs and must be rebuilt with that exact compiler.

```sh
cargo build --workspace --locked
target/debug/mir-checker --json -- --crate-type=lib tests/fixtures/bodies.rs
target/debug/mir-checker --entry root -- --crate-type=lib tests/fixtures/panics.rs
cargo install --path crates/mir-checker --locked
```

From another Rust project:

```sh
cargo mir-checker --lib
cargo mir-checker --manifest-path path/to/Cargo.toml --all-targets
```

The Cargo command forwards arguments to cargo check, uses the checker's pinned compiler and
analyzes workspace members. Each run gets a fresh directory under target/mir-checker in the
current directory, containing build artifacts and per-crate JSON reports. This deliberately
rebuilds dependencies so Cargo caching cannot silently skip analysis. These directories can be
deleted after use. --target-dir is reserved by the tool.

MIR is obtained through optimized_mir with mir-opt-level=0. This is runtime MIR after lowering
and mandatory transformations, rather than source MIR or LLVM IR. The inventory describes this
analysis build, not an artifact produced by another compiler. Cargo's target, features, panic
strategy and overflow settings still matter. Const and static initializer bodies are excluded;
runtime-capable const functions are included.

## Panic inventory

The report identifies MIR bounds, overflow, division, remainder and compiler-generated validity
checks. Calls to rustc's panic language items are recognized by compiler identity, including in
panic=abort builds. A user function with a panic-like name is not treated as a panic entry point.
Optional overflow checks are marked disabled when the analysis build disables them; signed
division overflow and division by zero remain active.

External calls, unresolved trait dispatch, function pointers, unavailable local bodies, drop glue
and inline assembly remain explicit unknown boundaries. Dependency bodies and destructor
implementations are not followed. Checks under guards are still unverified; their presence does
not establish a panicking input. An empty inventory does not establish safety either.

In direct mode, --entry FUNCTION can be repeated to request one shortest structural local call
path to each active site. Names must exactly match the names in the report. Recursive call graphs
terminate. Paths ignore branch feasibility and generic substitutions, so they are explanations of
call-graph connectivity, not counterexamples. This option is not yet available in Cargo mode.

JSON schema version 2 includes source locations, block numbers, conditions, unwind actions,
cleanup flags, structural CFG reachability, local call edges and the compiler arguments. All
sites have unverified status. A successful exit means compilation and inventory completed; it
does not mean contracts passed. Compilation, unknown-entry and report-write errors fail the run.

## Contracts

Add crates/mir-contracts as a path dependency. Its attributes also compile on stable Rust and
work with no_std consumers; the procedural macro executes on the host.

```rust
use mir_contracts::{ensures, no_panic, requires};

#[no_panic]
#[requires(index < bytes.len())]
#[ensures(result == bytes[index])]
pub fn read(bytes: &[u8], index: usize) -> u8 {
    bytes[index]
}
```

The attributes attach versioned, hidden HTML comments in doc metadata. They emit the original
function with no wrappers, predicate evaluation, runtime assertions or runtime library. The
checker reads this metadata from typed compiler attributes. The metadata format is experimental;
it is a request to verify, never evidence that a contract holds.

Currently predicates receive syntax validation only. Names, types, purity and truth are not
checked. result is reserved for the eventual return-value binding in postconditions. Attribute
targets are functions and methods with bodies, including const functions. Trait declarations
without bodies are not supported yet.

## Development

```sh
cargo fmt --all --check
cargo lint
cargo test --workspace --locked
cargo build --workspace --release --locked
cargo deny check
prek run --all-files --stage manual
```

See docs/stages.md for the staged plan and crate READMEs for implementation boundaries.
