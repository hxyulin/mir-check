# mir-checker

A standalone Rust static-analysis experiment using typed MIR from rustc. Panic analysis is the
first goal; verified contracts and restricted effects are intended extensions. This repository
is independent of fleet-2027 and installs no firmware dependencies.

The compiler adapter and panic inventory now include an opt-in proof engine for a restricted
subset of typed MIR. It can prove panic freedom for guarded integer and byte-slice operations
and trace concrete arguments through local calls. Contracts remain pending verification.

## Build and use

The toolchain file pins nightly-2026-09-22, including rustc-dev and LLVM tooling. The compiler
adapter uses unstable APIs and must be rebuilt with that exact compiler.

```sh
cargo build --workspace --locked
target/debug/mir-checker --json -- --crate-type=lib tests/fixtures/bodies.rs
target/debug/mir-checker --entry root -- --crate-type=lib tests/fixtures/panics.rs
cargo install --path crates/mir-checker --locked
```

Proof mode needs Z3. Install the pinned solver into the development environment:

```sh
uv venv .venv
uv pip install --python .venv/bin/python -r requirements-solver.txt
target/debug/mir-checker --verify --entry next_byte -- \
  --crate-type=lib --edition=2024 tests/fixtures/proofs.rs
```

Alternatively put z3 on PATH or set MIR_CHECKER_Z3 to its executable. The adapter invokes Z3
as a subprocess, without unsafe Rust bindings. Development tests require the solver.

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

## Panic proofs

--verify uses path-sensitive symbolic execution of typed MIR and SMT bit-vectors, preserving
integer widths, signed comparisons, casts, wrapping operations and checked arithmetic. Each
panic condition must be unsatisfiable under the path conditions. The proof is universal over
valid Rust inputs to the selected function; it is not based on test input coverage.

Supported inputs are bool, integers, unit, shared byte slices and byte arrays. Shared slice
contents are modeled as SMT arrays. Local calls are analyzed with the actual symbolic arguments
and return values. The compiler-identified slice length method has a built-in model. Other
external calls, trait dispatch, mutable references, floats and unsupported statements fail as
UNKNOWN. Reachable loops and recursive calls also remain UNKNOWN.

--entry selects roots for verification as well as call traces. Without entries, all inventoried
local bodies must pass. cargo mir-checker --verify applies this mode to Cargo workspace members.
Inventory mode remains available without a solver.

JSON schema version 3 adds a separate proof result per selected root, with PROVED, REFUTED and
UNKNOWN outcomes. Every obligation includes the generated SMT query and, for a satisfiable
failure, its solver model. Inventory sites keep their separate unverified status. Verification
exits nonzero for either REFUTED or UNKNOWN. Compiler errors discovered while fetching MIR
suppress the report entirely.

Proofs describe the recorded analysis compiler, target and build flags. They assume valid Rust
references and trust rustc's MIR semantics, this translator and Z3. The engine limits execution
to 256 blocks, call depth to eight and each query to 200,000 bytes. Z3 queries have a five-second
timeout and a six-second process limit. Reaching any limit fails verification as UNKNOWN.

Arithmetic guards, byte indexing and local calls have positive and negative integration cases.
An independent host replay exhausts all u8 input pairs for guarded addition, tests small slice
inputs and reproduces the rejected off-by-one, overflow, invalid-call and stale-guard panics.

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
