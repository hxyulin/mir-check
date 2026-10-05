# mir-checker

A standalone Rust static-analysis experiment using typed MIR from rustc. Panic analysis is the
first goal; verified contracts and restricted effects are intended extensions. This repository
is independent of fleet-2027 and installs no firmware dependencies.

The compiler adapter and panic inventory now include an opt-in proof engine for a restricted
subset of typed MIR. It can prove panic freedom for guarded integer and byte-slice operations
and trace symbolic arguments through concrete local and available dependency calls. It also checks
a restricted contract language:
callee preconditions, caller bounds and postconditions.

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
cargo mir-checker --verify --lib
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

The inventory includes source locations, block numbers, conditions, unwind actions,
cleanup flags, structural CFG reachability, local call edges and the compiler arguments. All
sites have unverified status. Without --verify, a successful exit means compilation and inventory
completed; it does not mean contracts passed. Compilation, unknown-entry and report-write errors
fail the run.

## Panic proofs

--verify uses path-sensitive symbolic execution of typed MIR and SMT bit-vectors, preserving
integer widths, signed comparisons, casts, wrapping operations and checked arithmetic. Each
panic condition must be unsatisfiable under the path conditions. The proof is universal over
valid Rust inputs satisfying the selected function's declared preconditions; it is not based on
test input coverage.

Supported inputs are bool, integers, unit, shared byte slices, byte arrays, small integer/bool
arrays and local structs with modeled fields. Shared references to integers, booleans, arrays and
those structs are supported. Input fields
are arbitrary: private fields do not imply an invariant. Shared slice contents are SMT arrays.
The engine models constructed structs, local enums, core Option/Result/ControlFlow variants, field
projections and local array initialization. Non-byte arrays have at most 16 elements. Integer/bool
elements support symbolic bounded indexing; enum/struct elements require a uniquely determined
index. Arbitrary enum/struct slices and enum inputs remain unsupported.

Calls resolve concrete generic substitutions and static trait implementations, then interpret
available local or dependency MIR with actual arguments and return values. Read-only closures and
function items can run through generic Fn/FnOnce calls and fixed-array map. Constructed Option and
Result values support the question-mark operator by following their instantiated core bodies.
Reports list interpreted bodies separately from explicit library models. Missing bodies and
unsupported compiler shims remain UNKNOWN; a dependency is not automatically assumed safe.

Pinned core models cover slice length, byte prefix indexing, lossless integer conversions,
integer endian decoding, shared byte-slice-to-array conversion, fixed-array map and
copy_from_slice into an owned local byte array. Array map executes each actual callable body in
index order. Another narrow model constructs opaque formatting
arguments from an evaluated static string, so literal panic messages can reach their panic call.
The range and copy models check their panic conditions and model the exact copied bytes.
Compiler identities and instantiated receiver types identify
these operations; similarly named user methods receive no special treatment. These core models
are trusted parts of the translator, not proofs of dependency implementations. Each root's report
lists the models it used. MIR assume intrinsics become validity obligations rather than unchecked
assumptions.

Mutable borrows are restricted to local byte arrays and their prefixes; closure environments can
be borrowed for read-only execution. Writes through captured references remain unsupported.
Mutable byte-array borrows cannot cross call boundaries, enter aggregates or escape as returns.
Unavailable external calls, unresolved trait dispatch, general mutable references, input enums,
nested struct inputs, floats and
unsupported statements fail as UNKNOWN. Finite loops can prove through complete symbolic
unrolling. No path is silently truncated: an unfinished exploration at the step limit fails as
UNKNOWN. Recursive calls still require an invariant and remain UNKNOWN.

--entry selects roots for verification as well as call traces. Without entries, all inventoried
local bodies must pass. cargo mir-checker --verify applies this mode to Cargo workspace members.
Inventory mode remains available without a solver.

JSON schema version 6 includes a separate proof result per selected root, with PROVED, REFUTED and
UNKNOWN outcomes. Every obligation includes the generated SMT query and, for a satisfiable
failure, its solver model. Named input bindings make models interpretable; entry preconditions
are listed as assumptions. Inventory sites keep their separate unverified status. Verification
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
pub fn read(bytes: &[u8], index: usize) -> u8 {
    bytes[index]
}
```

The attributes attach versioned, hidden HTML comments in doc metadata. They emit the original
function with no wrappers, predicate evaluation, runtime assertions or runtime library. The
checker reads this metadata from typed compiler attributes. The metadata format is experimental;
it is a request to verify, never evidence that a contract holds.

In verification mode, requires predicates define the entry domain. Every reachable local call
must independently prove each callee precondition from its current path conditions. Callee bodies
are analyzed with actual symbolic arguments; annotations are never trusted as summaries. Each
ensures predicate must hold at every feasible return. result denotes the actual return value;
parameter names refer to their entry values, even when the function reassigns its parameters.

The supported predicate language includes named bool/integer parameters, integer and boolean
literals, comparisons, &&, ||, !, parentheses, negative integer literals, read-only modeled array
or byte-slice .len(), constant indices into fixed non-byte arrays, named struct fields, integer
casts and exhaustive unguarded
None/Some(name) matches on constructed core Option values. Literals are checked against the
inferred target integer type and range.
Arithmetic, dynamic indexing, arbitrary calls, mutation and unknown names are unsupported and fail
as UNKNOWN. Inconsistent entry preconditions also fail as UNKNOWN instead of yielding a vacuous
proof. A requires predicate is an obligation for analyzed callers; it adds no runtime protection
against other callers violating that domain.

Only a selected root that passes all body and contract obligations marks its metadata verified
under preconditions. Inventory-only and unselected functions keep pending metadata, even when a
caller has been checked with a particular argument. Attribute targets are functions and methods
with bodies, including const functions; analyzing receivers outside the input subset remains
unsupported. Trait declarations without bodies are not supported.

See examples/contracts for a runnable no_std crate with guarded reads and a bounded increment.
From this repository root, after building and installing the solver:

```sh
target/debug/cargo-mir-checker --verify --manifest-path examples/contracts/Cargo.toml --lib --locked
```

This prototype can establish the listed obligations for its supported subset and recorded build.
The translator itself has not been formally verified, and a selected-root result is not a claim
about an entire crate, all its callers, dependencies, allocation failures, stack exhaustion or
undefined behavior.

## Development

```sh
cargo fmt --all --check
cargo lint
cargo test --workspace --locked
cargo build --workspace --release --locked
cargo deny check
prek run --all-files --stage manual
```

See docs/proofs.md for how obligations are generated and what the proof trusts, docs/stages.md
for the staged plan and crate READMEs for implementation boundaries.

## Real-code fixtures

examples/can-frame vendors the Frame and FdFrame slice from fleet-2027 with metadata contracts.
The original excerpt is retained, and tests check that the six method bodies are unchanged.
Only the vendored crate depends on mir-contracts; the original workspace remains independent.

All six methods prove on the host and thumbv7em-none-eabihf with panic=abort and overflow checks
enabled: both constructors, both ID getters and both data accessors. Constructor postconditions
cover ID validity, accepted payload lengths and exact stored lengths. Accessor proofs assume the
declared capacity bounds. Two additional harnesses prove the constructor-to-accessor call bounds
and byte-for-byte payload preservation for every permitted index.

```sh
cargo test --locked -p mir-checker --test compiler \
  vendored_constructors_accessors_and_payload_round_trips_prove_on_host_and_arm
cargo test --locked --manifest-path examples/can-frame/Cargo.toml
```

Regression tests reject changed guards, invalid stored lengths, incorrect copies and relaxed FD
length rules with solver models. Removing an accessor precondition exposes the possible range
failure. The fixture also contains intentionally failing call-bound harnesses; verifying every
body, including derived methods, is expected to fail. These selected-root results do not establish
whole-crate coverage. See examples/can-frame/README.md for provenance and the exact proof scope.

The larger examples/can-frame/src/bus.rs fixture preserves the original bus validator and both
helpers. The validator has 44 MIR blocks, nested loops, enum matches and collision/FD assertions.
Two three-device configurations prove for symbolic IDs and slots satisfying their declared bounds
on the host and ARM target. Five invalid configurations are refuted with solver models and replay
as runtime panics. The arbitrary-slice validator entry itself remains UNKNOWN; these proofs cover
the selected configuration families.

```sh
cargo test --locked -p mir-checker --test compiler \
  nested_bus_loops_prove_for_symbolic_ids_and_slots_on_host_and_arm
cargo test --locked -p mir-checker --test compiler \
  invalid_bus_ids_slots_collisions_and_fd_compatibility_are_refuted
```

examples/dr16 vendors the unchanged Raw::parse body from the same firmware snapshot. Its 42-block
MIR body proves panic freedom for every valid byte slice on both the host and ARM target, without
entry preconditions. Postconditions establish exact-length rejection, both switch bounds and all
five decoded channel bounds. The analysis follows Result::ok, Option's question-mark machinery
and three closure bodies, with explicit conversion, endian and array-map models.

```sh
cargo test --locked -p mir-checker --test compiler \
  the_dr16_parser_proves_without_entry_bounds_on_host_and_arm
cargo test --locked --manifest-path examples/dr16/Cargo.toml
```

Parser mutations introduce an out-of-range byte index or an incorrect channel mask; both are
refuted. Independent runtime tests compare 4,608 frames against separate decoding formulas.
See examples/dr16/README.md for provenance and scope. The floating-point Dr16::from_raw layer is
not included in this fixture.
