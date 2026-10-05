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

## Stage 3 evidence

The first proof engine uses exact path-sensitive symbolic execution and Z3 bit-vectors before
adding abstract interpretation. It proves guarded byte access, checked integer guards and safe
local calls. Off-by-one guards, overflow, division failures, invalid call arguments and stale
guards are refuted with solver models. Loops and missing solvers fail as unknown. Compiler MIR
lint errors cannot emit a success report.

Independent host checks exhaust all 65,536 u8 argument pairs for guarded addition and replay
five rejected panic cases. This validates representative translations; it is not a proof of
the analyzer implementation. Contract predicates and function-level preconditions remain future
work at this stage. The supported subset and resource limits are explicit in README.md.

## Stage 4 evidence

A restricted pure predicate evaluator checks entry domains, every reachable local call's bounds
and postconditions at feasible returns. Callee bodies and return values are still analyzed,
including when annotations claim no panic or a false return bound. Postcondition parameter
bindings snapshot entry values. Unsupported names, expressions, type/range errors and
inconsistent preconditions fail as unknown.

A guarded call to an identity function requiring value < 16 passes. The same caller with a
value <= 16 guard fails with value=16, although the identity body cannot panic. Removing caller
precondition verification makes that regression test fail; the mutation was restored. Changing
MIR's <= translation to < also makes the panic regression test fail; it was restored.

The proof suite passes on the 64-bit host and thumbv7em-none-eabihf's 32-bit usize. Aborting and
unwinding panic modes and enabled/disabled overflow checks are tested. Cargo verification is
covered with both passing contracts and a failed call bound that retains its JSON report. The
example crate demonstrates guarded byte reads and a bounded increment without runtime checks.

This completes the initial contract slice of stage 4. Loop invariants, abstract interpretation,
generic substitutions, dependency summaries, preserved type invariants and effect contracts
remain future work. The analyzer implementation itself has not been formally verified.

## Stage 5: real-code fixture

Frame and FdFrame are vendored from fleet-2027 at b81a247a3295b13553087f5e328f201944a1eb61.
The original method bodies are preserved and compared against an unmodified source excerpt.
The fixture adds constructor and accessor contracts without changing the original workspace.
Runtime checks exercise valid and invalid IDs and all payload lengths from zero through 65.

## Stage 6: real-code proofs

All six unchanged Frame/FdFrame method bodies now prove on aarch64-apple-darwin and
thumbv7em-none-eabihf, with panic=abort and overflow checks enabled. Constructors establish ID,
accepted-length and exact stored-length postconditions without entry assumptions. Data accessors
prove safe slicing and returned lengths under explicit capacity preconditions; private fields do
not imply an invariant for arbitrary struct inputs.

Two payload harnesses prove the constructor-to-accessor path, each callee's required bound and
exact payload equality at every permitted index. Actual constructor bodies supply caller facts;
annotations are not trusted summaries. Struct fields, constructed core Option variants and local
byte arrays extend the modeled MIR subset. Pure contracts gain named fields, integer casts and
restricted exhaustive Option matches.

Pinned core models cover byte prefix ranges, slice length, u8-to-usize conversion and exact copies
into local arrays. Their range and copy-length panic conditions are checked. They are trusted
parts of the translator and are listed per root in JSON schema version 5. Mutable borrows cannot
cross local calls or escape through aggregates/returns. General mutation, input enums, nested
struct inputs, arbitrary dependencies, loops and derived methods remain unsupported.

Regression tests reject invalid stored lengths at accessor calls, weakened constructor guards,
invalid FD lengths and incorrect or missing payload copies. Removing an accessor precondition
also fails. A user method named copy_from_slice receives no trusted model, and mutable call
boundaries remain unknown. Temporarily disabling the translator's destination-byte updates makes
the real-code round-trip test fail as refuted; the mutation was restored.

The compiler suite has 31 passing tests, plus the metadata behavior test and independent vendored
runtime checks covering IDs, lengths zero through 65 and every accepted payload index. Formatting,
warnings-denied Clippy, release builds, dependency checks and manual hooks pass. These selected-root
results are not whole-crate coverage or a formal verification of the translator.
