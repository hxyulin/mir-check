# mir-check

Host compiler adapter and report model. mir-check drives rustc directly; cargo-mir-check
uses it as a Cargo workspace wrapper and collects per-crate reports in an isolated build directory.

Cargo sets mir-check-rustc as the outer compiler wrapper and mir-check as the workspace wrapper.
The outer wrapper appends -Zalways-encode-mir=yes and -Zmir-opt-level=0 without rewriting flags.
It forwards to the workspace analyzer or the pinned compiler, so direct and transitive Cargo
library dependencies retain ordinary non-inline bodies. Only workspace members emit inventories.
Use --no-dependency-mir to disable retention. Prebuilt sysroot bodies, foreign declarations and
unsupported operations remain unknown; retained code is interpreted rather than trusted.

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

Cargo accepts repeated exact or crate-qualified --entry selectors. Crates without a matching root
retain inventories; the Cargo wrapper rejects any requested root absent from all selected targets.
--summary prints outcomes and grouped unknown reasons. Available reports are rendered even after
verification failure. Schema version 7 includes root coverage counts separately from distinct
interpreted body instances; neither count claims runtime or whole-crate coverage.

The report model uses std and serde. No compiler types escape the adapter, so JSON consumers
do not need rustc internals. Inventory remains separate from the opt-in proof engine.

Proof mode interprets a restricted subset of typed MIR, uses exact SMT bit-vectors for integers and
SMT floating-point operations for f32/f64 and follows actual arguments and return values through
concrete local and available dependency calls. Z3 runs as a subprocess. Every reachable panic
condition must be unsatisfiable; unsupported behavior, recursion and resource limits remain unknown
and cause verification failure. Finite loops are unrolled until every feasible path completes;
unfinished paths never become a passing result. Concrete generics, static trait implementations,
function items and read-only closures resolve to instantiated MIR bodies. Available dependency MIR
is interpreted; unavailable bodies and unsupported shims remain unknown. Explicit core models cover
slice lengths/ranges, lossless integer conversion, integer endian decoding, shared
byte-slice-to-array conversion, fixed-array map, owned byte-array copies, floating-point absolute
value/min/max and static formatting arguments. Array map executes actual callable bodies in order.
Reports list interpreted bodies and trusted models separately. MIR assume becomes a checked validity
obligation. General mutation and writes through captured references remain unsupported. Structs,
symbolic input enums and constructed variants preserve tags, fields and return facts. Small
integer/bool/float arrays support symbolic bounded indices and pattern projections; other elements
require uniquely determined indices. Struct inputs do not acquire implicit invariants.

Root inputs include tuples, nested local/dependency structs and enums, concrete generic fields,
supported shared references and small arrays of modeled aggregates. Input enums have at most 16
variants; every payload must be modeled, and downcasts require a proven tag check. Input
construction has an eight-level depth limit and a 128-value budget across arguments; recursive
references and larger shapes remain unknown. General mutable-reference fields still fail before
execution. Input bindings retain nested names such as packet.header.index and value.1.0.

Constant decoding reads evaluated values through rustc_const_eval, using compiler layouts,
discriminants and initialized scalar reads. It supports nested structs/tuples, active enum
payloads, immutable promoted/static references and bounded arrays/slices. A reference to interior
mutable storage is rejected. Only active fields are read: None with an unsupported inactive
payload can be modeled, while a reachable union/MaybeUninit remains unknown. Limits are eight
recursive levels, 256 values, 128 bytes or 16 non-byte elements per array/slice. The decoder
inspects constants; it does not replace symbolic MIR execution or model arbitrary memory.

The contract evaluator accepts pure comparisons and boolean predicates,
with modeled array/slice lengths, constant non-byte array indices, named/numeric fields, integer
and float casts and restricted Option matches. Symbolic Option arms must return booleans.
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

The contract example adds guarded_packet_read, which checks a nested header's index against a
shared byte slice and proves the read callee's bound. Aggregate-input tests cover the host and ARM,
tuple postconditions, fixed struct arrays and a refuted off-by-one caller guard. Workspace tests
separate qualified roots from same-named unsupported functions and reject missing names.

Floating-point tests cover NaN comparisons, signed zero, rounding, infinities, Rust's saturating
float-to-integer casts and float-dependent caller bounds on host/ARM. Arithmetic uses nearest-even
rounding. Float remainder and raw bit observation stay unknown. Min/max permits either operand
for equal numeric inputs, including signed-zero ties. Enum tests cover explicit signed tags,
payload bounds, symbolic Option contracts, entry snapshots and foreign nested types. Array
pattern tests check start/end offsets and minimum lengths, including a refuted payload assertion.
Constant tests cover Option::as_ref, niche layouts, explicit signed tags, nested fields and
immutable storage on host/ARM. Wrong payload assertions, off-by-one guards and a mutated constant
index refute; unions, interior mutation, unsupported transmutes and shape limits remain unknown.
