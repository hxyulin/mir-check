# Stages

1. Compiler adapter: pinned rustc_driver integration, typed local MIR bodies, source locations,
   Cargo integration, JSON reports and a metadata-only contract crate.
2. Panic inventory: enumerate checks and calls, classify unresolved boundaries, and show
   conservative local call paths. Inventory results do not establish feasible execution paths.
3. Proof engine: branch-sensitive abstract interpretation for integer ranges, enum variants and
   slice lengths. Add selective symbolic reasoning, exact arithmetic semantics and loop invariants.
4. Verified contracts and effects: check preconditions at callers, postconditions at returns and
   no_panic claims; add preserved type invariants, dependency summaries and restricted effects.

Each completed stage has its own commit. An unsupported operation, absent dependency body or
analysis timeout remains unknown. An annotation never establishes its own truth. Candidate
counterexamples from approximate models require validation before being reported as confirmed.

The starting examples are guarded byte reads, constant index helpers, frame length invariants,
invalid clamp bounds and unresolved dynamic dispatch. Resource and application-state properties
can follow once the foundational semantics are tested.

## Stage 1 evidence

Completed on macOS aarch64 with the pinned compiler. Four compiler integration tests and one
metadata behavior test pass. The tests cover uncalled generics, compiler errors, metadata
collection, Cargo feature forwarding and repeat analysis. Removing an uncalled generic from the
inventory makes its integration test fail; the mutation was restored.

Formatting, warnings-denied Clippy, release builds, dependency checks and manual hooks pass.

## Stage 2 evidence

The inventory detects bounds, arithmetic and explicit panic sites with panic=abort. Tests cover
optional overflow settings, signed division overflow, unknown external/trait/indirect calls,
destructors, guarded accesses, local call paths and recursion. Sites stay unverified and unknown
entry selection fails. A user function named panic_fmt remains an ordinary local call.

The existing CAN frame source was analyzed read-only on aarch64-apple-darwin and
thumbv7em-none-eabihf, with aborting panics and overflow checks enabled. Both reports contained
18 bodies and 33 sites, including five panic language-item calls. These are inventory counts,
not findings that the guarded operations can panic or proofs that they cannot.

The metadata crate also passes its behavior test under stable Rust 1.98.1. It does not evaluate
even unresolved predicate names or false preconditions at runtime. Verification remains future
work; unsupported contracts do not become trusted assumptions.

Removing bounds checks from the inventory makes the abort-mode integration test fail; the
mutation was restored. Formatting, warnings-denied Clippy, release builds, dependency checks
and manual hooks pass. Compiler and metadata integration tests also cover malformed contracts
and report-write failures.
