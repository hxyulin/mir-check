# mir-check

Host compiler adapter and report model. mir-check drives rustc directly; cargo-mir-check
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
semantics and follows actual arguments and return values through concrete local and available
dependency calls. Z3 runs as a
subprocess. Every reachable panic condition must be unsatisfiable; unsupported behavior,
recursion and resource limits remain unknown and cause verification failure. Finite loops are
unrolled until every feasible path completes; unfinished paths never become a passing result.
Concrete generics, static trait implementations, function items and read-only closures resolve to
instantiated MIR bodies. Available dependency MIR is interpreted; unavailable bodies and unsupported
shims remain unknown. Explicit core models cover slice lengths/ranges, lossless integer conversion,
integer endian decoding, shared byte-slice-to-array conversion, fixed-array map, owned byte-array
copies and static formatting arguments. Array map executes actual callable bodies in order.
Reports list interpreted bodies and trusted models separately. MIR assume becomes a checked
validity obligation. General mutation and writes through captured references remain unsupported.
Structs, constructed local enums and core Option/Result/ControlFlow values preserve return facts.
Small integer/bool arrays support symbolic bounded indices; other elements require uniquely
determined indices. Struct inputs do not acquire implicit invariants.

The contract evaluator accepts pure comparisons and boolean predicates,
with modeled array/slice lengths, constant non-byte array indices, struct fields, integer casts
and restricted Option matches.
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
configurations. Arbitrary enum slices and ambiguous enum/struct indices remain unknown. Loop tests
cover finite bounded domains, failures after later iterations, nontermination and the step limit.
Static panic payload models do not extend to dynamic formatting or similarly named user methods.

examples/dr16 preserves Raw::parse, including slice conversion, question-mark propagation, captured
closures, fixed-array map, shifts and endian decoding. Its panic freedom, accepted length, switch
bounds and five channel bounds prove without entry preconditions on the host and ARM target.
Regression tests refute incorrect byte indices and channel masks; independent host tests compare
4,608 frames against separate formulas. Other compiler tests cover generic call bounds, static
dispatch, Result question-mark payloads and available dependency bodies with a rejected overflow.
