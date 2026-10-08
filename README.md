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

Default output shows root outcomes, unselected bodies, interpreted instances and the main reasons
for UNKNOWN. `--summary` remains an alias for this compact view; use `--verbose` for the full
inventory, obligations, assumptions and solver models. Per-crate
JSON reports are written under `target/mir-check/<run>/reports`, including failed verification.
These counts describe analysis roots, not runtime test coverage or the safety of unselected code.

The CLI reports build/analysis/report-reading phases and prints the active root with elapsed time
every five seconds during slow checks. Terminal results use green for PROVED, red for REFUTED,
amber for UNKNOWN and magenta for PROVED_WITH_ASSUMPTIONS. Color never carries meaning alone.
Piped output is plain by default; `--color auto|always|never`, NO_COLOR and `--quiet` control
display.

```sh
cargo mir-check --verify --entry my_crate::function --jsonl results.jsonl --lib
cargo mir-check report results.jsonl
cargo mir-check report target/mir-check/<run>/reports --verbose --color never
```

JSONL contains one complete crate report per line, preserving queries, models and trusted
assumptions. `--jsonl -` writes clean JSONL to stdout and moves human output to stderr. The report
command reads saved JSON files, report directories or JSONL without running the compiler or solver.

To run fresh direct analysis with a saved compiler configuration, use
`mir-check --verify --from-report invocation.json`. Omitting --entry checks every inventoried
crate body; adding --entry main checks that root and its reachable calls. This reuses compiler
arguments rather than proof results and requires the original working directory and dependency
artifacts. See [direct checks](docs/usage.md#direct-whole-crate-and-main-checks).

| Result | What it tells you |
| --- | --- |
| **PROVED** | Every feasible path completed and every obligation passed for the selected root domain |
| **PROVED_WITH_ASSUMPTIONS** | The root passed using explicit user-trusted call summaries; acceptance requires --allow-assumptions |
| **REFUTED** | The solver found a failing assignment for a translated obligation |
| **UNKNOWN** | Unsupported behavior, a missing body or a limit prevented a complete proof |

Strict verification succeeds only when every selected root is PROVED. A solver assignment is not
automatically a confirmed Rust failure; mutation and replay tests provide independent evidence
for fixtures.

Add `--replay` to `--verify` to compile and execute supported native counterexample inputs. The
default scan executes no analyzed functions. Replay preserves the analyzed panic strategy and
overflow configuration, records concrete inputs and observed panic locations, and distinguishes
confirmed panics, executions that did not panic, unsupported inputs and incomplete runs. A replay
that returns normally does not prove the root safe. Cross-target execution and arbitrary inputs
remain unsupported; see [native replay](docs/usage.md#execute-a-counterexample).

## Use it in a project

Install the analyzer, Cargo command and compiler wrapper from this checkout:

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

Cargo analysis retains MIR for dependencies by default, including ordinary non-inline functions,
and disables their MIR optimization. It appends these compiler options through `mir-check-rustc`
while preserving Cargo configuration, profiles and Rust flags. Dependencies become available
callee bodies, not independently selected roots. Use `--no-dependency-mir` to compare without
retention. Prebuilt sysroot libraries are not rebuilt; remaining missing/unsupported bodies fail
as UNKNOWN.
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
| Constants | Evaluated structs, tuples, active enum variants, bounded arrays/slices and immutable promoted/static references |
| Arithmetic | Exact integer operations, min/max, saturation, bit counts and rearrangement; IEEE f32/f64 arithmetic, comparisons and saturating casts |
| Interior mutation | Scalar Cell aliases/calls and conservative integer atomic counters with checked orderings |
| Mutable storage | Disjoint mutable root inputs, projected writes, reborrows, tracked aggregate references and call effects |
| Calls | Concrete generics, static traits, available dependency MIR, function items, known function pointers, tracked mutable captures and noncapturing evaluated closure constants |
| Control flow | Feasible branches, symbolic enum tags/payloads, Option/Result propagation completely unrolled finite loops and opt-in scalar/byte-array induction |
| Library models | Byte ranges/copies/conversions, endian decoding, fixed-array map/from_fn, float abs/min/max/clamp and static formatting arguments |
| Iteration | Shared/mutable slices and owned arrays; checked predicates, ordered folds and supported Zip/Flatten bodies |
| Contracts | Caller bounds, entry preconditions and postconditions on actual returns |

Integer/bool/float arrays support symbolic bounded indices and array/slice pattern projections.
Owned aggregate repeats preserve independent copies, with at most 128 elements and 256 modeled
values per repeat. Non-byte input arrays allow 256 elements within the root shape budget; evaluated
constants allow 128 elements. Tuple/struct/enum array elements require a uniquely determined index
on the current path. Root struct fields are independent inputs; privacy and constructors do not
supply an implicit type invariant. Nested input construction is limited to 16 levels and 512 values;
non-byte arrays have at most 256 elements and input enums have at most 64 variants. All variant
payloads must have supported shapes.

Constant decoding uses rustc's constant interpreter for layouts, discriminants and initialized
scalar reads. It follows only immutable references to storage without interior mutation and
decodes only the active variant. Constants have an eight-level depth limit and a 256-value budget;
evaluated arrays/slices have at most 128 elements. Unions,
including MaybeUninit, and mutable or raw-pointer storage remain unknown.

Tracked mutable references can be stored in tuples, structs, enums and closure environments. They
keep allocation identity through calls and returns when their storage belongs to the caller; dead
references and references into the returning frame fail verification. FnMut callbacks retain both
owned capture state and writes through captured references between invocations. Local byte borrows
and bounded chunk/remainder views retain their original allocation. Multiple safe mutable root
inputs receive independent storage when their pointees have no interior mutation or references.
General aliasing and ambiguous non-byte writes remain gaps.

Float storage bits are tracked through inputs, constants, from_bits/to_bits, moves, negation, abs
and clamp. Arithmetic has exact numeric IEEE semantics; NaN output bits conservatively allow all
payloads and signs, including signaling encodings. A bit-level counterexample involving an
arithmetic NaN therefore may not replay on the target.

Enum/struct slices, unresolved generic inputs, float remainder, dynamic dispatch, unknown function
pointers, pointer-based drop glue, some constant shapes and broader iterator machinery remain gaps.
Concrete
synchronous destructors execute through rustc's drop glue, preserving effects and field order.
Experimental induction still rejects drop-bearing values. Limits and
unsupported
operations produce UNKNOWN. A selected-root proof also does not establish absence of undefined
behavior, allocation failure, stack exhaustion, interrupt races or hardware timing failures.

Refuted roots stop at their first counterexample while the other selected roots continue. Add
`--all-failures` to collect further obligations under the same resource limits. Raw reports retain
the first failing query and model and indicate when exploration stopped early.

The default execution limits are 8,192 steps and 16 active call frames per root. Finite recursion
can complete within those limits; unfinished paths remain UNKNOWN. Root/query defaults are 30
seconds
and 200,000 bytes. Both CLIs support [configurable budgets](docs/usage.md#analysis-budgets).
Incremental solver scopes, exact constant folding and a
root-local instantiated MIR cache reduce repeated work without assuming function summaries.

Experimental `--induction` uses Z3 Spacer to prove supported cyclic root bodies without unrolling
an iteration bound. It supports integer/Boolean state, tuples, fixed byte arrays, available concrete
callee MIR, checked preconditions/postconditions and typed shared/mutable storage. Caller state and
entry snapshots are carried through actual callee transitions. Stable field references preserve
aliases. Integer ranges, tagged enum state and supported custom iterators can use induction too.
Shared/mutable byte-slice iterators and fixed integer/Boolean arrays carry cursors and indexed
references through loops. Changing allocation targets, slice views, arbitrary non-byte slices,
interior mutation, coroutines and recursion remain UNKNOWN in this mode. Raw reports retain
inferred models separately in `invariants`. A positive Horn result proves panic freedom rather
than termination. Solver failures and detected Horn failures remain UNKNOWN until counterexample
replay is available. See [loop proof details](docs/proofs.md#loops-and-limits).

```sh
target/release/mir-check --verify --induction --entry sampled_registers -- \
  --crate-type=lib --edition=2024 tests/fixtures/unbounded_loops.rs \
  --target thumbv7em-none-eabihf -Cpanic=abort -Coverflow-checks=yes
```

See [the coverage matrix](docs/coverage.md) for evidence and limits, and
[proof execution](docs/proofs.md) for how obligations are generated and what the result trusts.
JSON schema version 9 includes per-root proofs, recorded failure call chains, optional native replay
evidence and a per-crate coverage summary. Saved schema 7 and 8 reports remain readable. Inventory
sites keep their separate unverified status even when a selected root proves their paths safe.

Use `--contracts contracts.json` to attach checked preconditions and postconditions without adding
a crate dependency or editing the analyzed code. The sidecar can also declare explicit trusted
call boundaries with a reason, input bounds, return constraints and memory effects. Those calls
produce PROVED_WITH_ASSUMPTIONS, recorded separately from ordinary proofs and rejected by default.
See [external contracts](docs/contracts.md#contracts-without-a-source-dependency) for configuration
and the acceptance policy. Dynamic formatting itself remains outside the supported body model.

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

Ordinary analysis can also follow constructed async futures through poll, suspension, resumption
and available cancellation drop glue. It checks actual reached poll bodies, including nested
futures and captured mutable storage. Future construction alone does not prove deferred execution.
A standalone binary fixture proves from main through two polls and rejects a bad resumed index.
Unbounded async polling, arbitrary coroutine inputs, waker operations and executor internals remain
UNKNOWN. See [constructed async futures](docs/proofs.md#constructed-async-futures).

Use `--async-entry FACTORY` to construct and repeatedly poll a fresh async task without a polling
harness. It preserves state across Pending and stops at Ready, sharing the root resource limits.
Initialization can be selected independently with `--entry`. Unsupported executor handles,
unbounded suspension and executor behavior remain UNKNOWN; startup history is not inherited by
the async entry. See [entry selection](docs/usage.md#select-roots) for the scope and limits.

Lifetime-only mutable-reference casts now preserve tracked storage. An explicit trusted
`returns_alias` clause can also preserve a named mutable-reference argument, with the claimed
memory effects and existing escape checks. Assumptions remain visible even when a root ends in
UNKNOWN or REFUTED. See [trusted boundaries](docs/contracts.md#explicitly-trusted-call-boundaries).

The ordinary interpreter can also recover a compiler-known typed static layout through an
UnsafeCell byte carrier while keeping its payload opaque. It checks allocation provenance, original
initializer types, size, alignment and field offsets. Shared fixed arrays support bounded slice
views and ordered element references; iterator `find_map` executes callbacks with short circuiting.
Dense atomic prefixes can form integer atomic views after compiler layout and alignment checks,
even with opaque fields or padding beyond the accessed footprint. General mutable payloads remain
opaque. See
[static storage views](docs/proofs.md#opaque-static-storage-views) for the supported boundaries.

Atomic fence coverage checks compiler-identified fence/compiler_fence wrappers and their intrinsic
boundaries. Relaxed wrapper calls refute; valid fences preserve local storage without adding
synchronization or atomic-history facts. See
[cells and atomics](docs/proofs.md#cells-and-atomic-counters) for the abstraction and its limits.

Integer compare_exchange and compare_exchange_weak check ordering arguments and preserve the
old-value relation within each result. Weak CAS admits spurious failure. Later atomic accesses
remain independent, with no synchronization or atomic-history facts.

Certified static MaybeUninit::as_ptr addresses stay opaque until initialization is established.
Local slots can retain and replace static references; concrete zero-argument closure/function-item
callbacks execute their actual MIR. General payload reads, arbitrary writes and unknown dynamic
calls remain UNKNOWN.

Known function-item coercions retain their resolved target and signature through tracked local
storage and returns. Direct pointer calls and Fn/FnMut/FnOnce adapters execute that target's actual
MIR and check its contracts. Unknown targets, closure-to-pointer coercions, compiler reification
shims, pointer address casts, missing bodies and induction over pointer values remain UNKNOWN. See
[known function pointers](docs/proofs.md#known-function-pointers).

Certified UnsafeCell static places now accept supported typed stores, including known callbacks and
freshly constructed futures. MaybeUninit/UnsafeCell address chains retain their initialization
barrier, and a store supplies no shared-read or atomic-history facts. Stored tracked references
remain subject to frame escape checks. Thin NonNull wrappers preserve known static provenance.
See [typed static stores](docs/proofs.md#typed-static-stores).

Result unwrap/expect failure paths can now reach a checked panic obligation through their error
formatting setup. Concrete shared references can coerce to opaque core Debug references, retaining
storage identity through reborrows, local aggregates and returns. Creating the reference does not
execute a formatter. Dynamic formatter calls, general trait-object inputs and vtable operations
remain UNKNOWN. See [error formatting boundaries](docs/proofs.md#error-formatting-boundaries).

Thin raw-pointer inputs and pointer atomic compare-exchange support address-only reasoning without
pointee provenance. Compiler nonnull input patterns exclude zero; dereferencing numeric inputs
remains UNKNOWN. Pointer CAS validates orderings and allows weak spurious failures, with arbitrary
old addresses and no retained pointer update history.
