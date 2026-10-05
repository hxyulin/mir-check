# mir-checker

Host compiler adapter and report model. mir-checker drives rustc directly; cargo-mir-checker
uses it as a Cargo workspace wrapper and collects per-crate reports in an isolated build directory.

The adapter reads local typed runtime MIR using the pinned compiler and disables MIR
optimization. It collects function locations, block counts, pending contracts, MIR checks, panic
language-item calls and unknown call/drop boundaries. Optional entry selection shows structural
paths through local calls; it does not check execution feasibility or substitute generic arguments.

The inventory includes cleanup blocks and marks structural CFG reachability. Assert conditions
and compiler operands are diagnostic strings, not a stable representation for future solvers.
The proof engine consumes typed MIR inside the adapter and does not parse inventory strings.

Compilation continues in Cargo mode and stops after analysis in direct mode. Compiler failures,
unknown entries and report-write failures produce a nonzero exit status. In inventory mode,
success establishes only that the inventory was collected.

The report model uses std and serde. No compiler types escape the adapter, so JSON consumers
do not need rustc internals. Inventory remains separate from the opt-in proof engine.

Proof mode interprets a restricted subset of typed MIR, uses exact SMT bit-vectors for integer
semantics and follows actual arguments and return values through local calls. Z3 runs as a
subprocess. Every reachable panic condition must be unsatisfiable; unsupported behavior,
recursion and resource limits remain unknown and cause verification failure. Finite loops are
unrolled until every feasible path completes; unfinished paths never become a passing result.
No dependency analysis is present. Explicit trusted core models implement slice lengths and indexing,
u8-to-usize conversion, copying into owned local byte arrays and constructing opaque formatting
arguments from evaluated static strings. Range and copy panic conditions are checked; model use
is listed in each proof report. General mutation and mutable borrows across calls remain unsupported.
Struct fields, constructed local enums and core Option values carry return facts through local calls.
Small constructed non-byte arrays support uniquely determined indices;
struct inputs do not acquire implicit invariants.

The contract evaluator accepts pure comparisons and boolean predicates,
with read-only byte lengths, modeled struct fields, integer casts and restricted Option matches.
It checks caller preconditions and every feasible return, using
entry values for parameter names in postconditions. It never assumes a callee summary from
annotations. Missing names, type errors, unsupported predicates and inconsistent entry domains
fail verification; passing root metadata is marked verified under preconditions.

Real-code fixtures live in examples/can-frame. Tests compare vendored method bodies to the
unmodified source excerpt before testing analysis coverage. All six methods and two payload
round-trip harnesses prove on the host and thumbv7em-none-eabihf. Mutation tests reject broken
bounds, invalid IDs or FD lengths and lost payload copies. Separate cases reject similarly named
user methods and keep unsupported mutable call boundaries unknown.

The bus fixture adds a 44-block validator with nested loops, helper calls and enum matches. Tests
prove two symbolic three-device configurations on the host and ARM target and refute five invalid
configurations. Arbitrary enum slices and ambiguous non-byte indices remain unknown. Loop tests
cover finite bounded domains, failures after later iterations, nontermination and the step limit.
Static panic payload models do not extend to dynamic formatting or similarly named user methods.
