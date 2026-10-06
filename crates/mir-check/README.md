# mir-check

Host compiler adapter and report model. mir-check drives rustc directly; cargo-mir-check
uses it as a Cargo workspace wrapper and collects per-crate reports in an isolated build directory.

Cargo sets mir-check-rustc as the outer compiler wrapper and mir-check as the workspace wrapper. The
outer wrapper appends -Zalways-encode-mir=yes and -Zmir-opt-level=0 without rewriting flags. It
forwards to the workspace analyzer or the pinned compiler, so direct and transitive Cargo library
dependencies retain ordinary non-inline bodies. Only workspace members emit inventories. Use
--no-dependency-mir to disable retention. Prebuilt sysroot libraries are unchanged by default. For a
target with rust-src installed, cargo-mir-check accepts -Zbuild-std=core to rebuild core with
retained MIR. Captured reports can replay against those artifacts while the build directory remains
available. Exposing a body does not add models for its operations; pointer-based library internals
can still fail as unknown. Foreign declarations have no Rust MIR body to retain. Diagnostics
separate these boundaries; retained code is interpreted rather than trusted.

Function-item callbacks resolve static trait dispatch before requesting MIR. Fixed-array map,
from_fn, iterator predicates and folds execute the concrete implementation, including its panic
paths, rather than requesting the bodyless trait method declaration.


Unchecked MIR integer addition, subtraction and multiplication generate validity obligations:
their mathematical result must fit the operand type on the current path before analysis continues.
Guarded core operations such as unsigned checked_add/checked_sub retain their real bodies. The
compiler cold_path intrinsic is an optimization-only marker. RuntimeChecks operands use the pinned
compiler session's exact UB, overflow and contract-check settings; disabled runtime UB checks do
not remove unchecked-arithmetic validity obligations. Unmodeled raw-pointer continuations remain
unknown.

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
Default compact output prints outcomes, prioritized roots and grouped unknown reasons; --summary
is an alias and --verbose shows the full inventory. Available reports are rendered even after
verification failure. Schema version 8 includes root coverage counts separately from distinct
interpreted body instances; neither count claims runtime or whole-crate coverage.

The report model uses std and serde. No compiler types escape the adapter, so JSON consumers
do not need rustc internals. Inventory remains separate from the opt-in proof engine.

The smt module supplies a typed term DAG for the encoding migration. Analysis contexts intern
Boolean, bit-vector, floating-point and array nodes, reject mixed contexts and invalid operand
sorts, and fold supported closed terms structurally. The bounded printer preserves repeated
subexpressions through scoped lets. Z3 remains the solver and the subprocess protocol is unchanged.
The MIR interpreter still uses its existing string representation until its consumers migrate;
the new module alone does not change proof outcomes or analysis limits.

Proof mode interprets a restricted subset of typed MIR, uses exact SMT bit-vectors for integers and
SMT floating-point operations for f32/f64 and follows actual arguments and return values through
concrete local and available dependency calls. Z3 runs as a subprocess. Every reachable panic
condition must be unsatisfiable; unsupported behavior and exhausted resource limits remain unknown
and cause verification failure. Finite loops are unrolled until every feasible path completes;
unfinished paths never become a passing result. Concrete generics, static trait implementations,
function items and supported mutable closures resolve to instantiated MIR bodies. Available
dependency MIR is interpreted; unavailable bodies and unsupported shims remain unknown. Explicit
core models cover slice lengths/ranges, lossless integer conversion, integer endian
encoding/decoding, shared byte-slice-to-array conversion, fixed-array map, owned byte-array copies,
exact integer population counts, floating-point absolute value/min/max/clamp and static formatting
arguments. Array map executes actual callable bodies in order. Reports list interpreted bodies and
trusted models separately. MIR assume becomes a checked validity obligation. Typed allocations
support one mutable root receiver, projected writes, reborrows and call state propagation. Tracked
references in aggregates and captures preserve writes to their allocations. General aliasing and
multiple mutable root references remain unsupported. Structs, symbolic input enums and constructed
variants preserve tags, fields and return facts. Small integer/bool/float arrays support symbolic
bounded indices and pattern projections; other elements require uniquely determined indices. Struct
inputs do not acquire implicit invariants.

Root inputs include tuples, nested local/dependency structs and enums, concrete generic fields,
supported shared references and small arrays of modeled aggregates. Input enums have at most 64
variants; every payload must be modeled, and downcasts require a proven tag check. Input
construction has a 16-level depth limit and a 512-value budget across arguments; recursive
references and larger shapes remain unknown. General mutable-reference fields still fail before
execution. Input bindings retain nested names such as packet.header.index and value.1.0.

Constant decoding reads evaluated values through rustc_const_eval, using compiler layouts,
discriminants and initialized scalar reads. It supports nested structs/tuples, active enum
payloads, immutable promoted/static references and bounded arrays/slices. A reference to interior
mutable storage is rejected. Only active fields are read: None with an unsupported inactive
payload can be modeled, while a reachable union/MaybeUninit remains unknown. Limits are eight
recursive levels, 256 values and 128 elements per evaluated array/slice. The decoder
inspects constants; it does not replace symbolic MIR execution or model arbitrary memory.

The contract evaluator accepts pure comparisons and boolean predicates,
with modeled array/slice lengths, literal fixed-array and known-length byte indices,
named/numeric fields, integer
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
rounding. Float storage encodings support to_bits/from_bits and same-width integer/float
transmutes. Inputs, constants, moves, negation, abs and clamp preserve the selected bits. Numeric
arithmetic results receive one stable storage encoding; all NaN payloads/signs, including signaling
encodings, are allowed conservatively. Such NaN counterexamples may not replay on the target.
Float remainder remains unknown. Min/max permits either operand
for equal numeric inputs, including signed-zero ties. Enum tests cover explicit signed tags,
payload bounds, symbolic Option contracts, entry snapshots and foreign nested types. Array
pattern tests check start/end offsets and minimum lengths, including a refuted payload assertion.
Constant tests cover Option::as_ref, niche layouts, explicit signed tags, nested fields and
immutable storage on host/ARM. Wrong payload assertions, off-by-one guards and a mutated constant
index refute; unions, interior mutation, unsupported transmutes and shape limits remain unknown.

Mutable roots reject reference-containing pointees and multiple mutable root arguments. Each path
carries independent typed storage through calls; dead/uninitialized reads and unsupported writes
fail. Postconditions preserve entry names and expose `final_<parameter>` for updated arguments.
Storage has a 512-allocation limit and each root has a 30-second budget checked before queries.
The PID fixture preserves the fleet bodies and verifies reset, concrete configured updates,
invalid caller limits and a rejected reset-write mutation on host/ARM. Arbitrary symbolic update
paths can still exceed the budget.

Compiler-identified scalar Cell new/get/set/replace models share allocation-backed writes through
supported aliases and calls. Integer atomic new/load/store/fetch_add/fetch_sub/swap models allow
arbitrary current values and interference at each access. RMW operations return the old value and
wrap on overflow. Load/store ordering restrictions are checked, including symbolic Ordering
arguments. Atomic-only static wrappers are represented without freezing mutable initializers.
Reports record these models; atomic history assertions can refute under the conservative
abstraction. RefCell guards, general UnsafeCell operations and raw pointers remain unsupported.

--contracts FILE attaches checked clauses to unchanged code through a schema-1 JSON sidecar.
Exact crate-qualified selectors and positional aliases are validated; stale configuration fails.
Explicit trusted summaries require no_panic=true and a reason, check caller bounds, generate fresh
supported returns and apply declared memory effects. Missing effects invalidate modeled storage;
an explicit modifies list trusts the frame outside listed arguments. Known aliases share writes.
Generic summaries require an exact compiler instance. Unsupported return/effect ownership remains
unknown. Roots using summaries are PROVED_WITH_ASSUMPTIONS, recorded separately from PROVED and
rejected without --allow-assumptions. Selecting a trusted function still checks its actual body.
Reports retain configuration, matched selectors and used summary provenance. See docs/contracts.md.

Progress runs on stderr, with a five-second heartbeat naming the active crate/root or report file.
Final output shows per-crate and aggregate results. Terminal colors distinguish proved, refuted,
unknown and assumed outcomes; --color auto|always|never and NO_COLOR control ANSI output. --quiet
suppresses progress without hiding final results/errors. Compact rows/details are limited for
readability; raw data is complete. --jsonl FILE exports one complete report per line; --jsonl -
reserves stdout for machine data and sends human/Cargo output to stderr. Both binaries provide
a report subcommand for saved schema-7/8 JSON, JSONL or report directories. It recomputes counts
and applies the usual exit policy without claiming a new proof or rerunning the compiler/solver.

Direct --verify without --entry independently checks every inventoried body in one crate.
--entry main checks that selected body's reachable calls instead. --from-report FILE reads a
schema-7/8 report's compiler arguments and recompiles current source using the pinned compiler;
it never reuses saved proofs, selectors or trusted configuration. Run from the original working
directory with dependency artifacts available. Extra rustc arguments and mismatched compilers
are rejected. Missing main selectors list main-related MIR names for expanded macros. Async
construction/coroutine execution remains unsupported, and compiler invocations that bypass MIR
analysis cannot succeed as verification runs.
Reused Cargo diagnostic flags are replaced with human output and the chosen color policy;
analysis flags are preserved and the actual invocation is recorded in the new report.

Population count expands the compiler ctpop intrinsic into an exact bit-vector sum with a u32
result. Primitive core float clamp checks ordered, non-NaN bounds as a panic obligation before
selecting its result; NaN inputs and signed-zero ties follow the pinned implementation. Host/ARM
regressions reject bad bounds, same-named user methods and mutated guards/count limits.

Repeated arrays support small owned tuples, structs, enums and nested arrays, preserving each copy's
fields and independent writes. Generated repeats have at most 128 elements and 256 modeled values.
Non-byte root arrays allow 256 elements within the 512-value input budget; evaluated constants
permit 128 elements. Byte arrays retain their 128-byte limit. Storage identities (Cell, atomics,
tracked references and mutable byte views) are not cloned by the repeat model. Accessing repeated
inline-constant interior mutable storage remains unknown. Composite indices still require a unique
value on each path; this stage does not extend alias or iterator semantics.

Compiler-identified shared slice iterators retain a source and front/back cursors. Models cover
construction, next/next_back, nth/nth_back, len/count/size_hint, clone and all/any. Advancing
updates
typed iterator storage; adapters such as enumerate, copied and rev execute their actual MIR.
Predicate callbacks execute actual bodies in order, preserve memory effects and stop immediately
at the deciding element. Symbolic byte slices work when paths finish within the execution budget;
unbounded loops, unsupported iterator views and raw pointer operations remain unknown.
Compiler layouts supply constant tags for single-variant enum representations, including the
uninhabited residual optimized by core's question-mark implementation.

Mutable slice iterators yield projected references into typed source storage, including local
byte arrays, bounded byte-slice roots and composite array elements. Mutable enumeration preserves
that storage and checks the counter under the active overflow policy. Known Some payloads retain
tracked mutable references when unwrapped. Allocation-backed byte-copy views read and update
addressed
storage, so iteration followed by copy_from_slice preserves the latest data. Mutable references
into incoming storage can return inside aggregates and iterators; reference graphs reject local
or dead storage escaping. Ambiguous composite writes remain unsupported.

Borrowed fixed arrays and slices implement IntoIterator through the same tracked cursor models.
Primitive core f32/f64 finiteness uses exact NaN/infinity classification, reducing the MIR steps
needed by numeric iterator predicates without weakening their conditions.

The default solver keeps one lazily started Z3 process per root. Structured queries retain common
assertion prefixes and use push/pop to replace branch suffixes. Live sessions keep declarations
global, so adding symbols retains shared scopes; incompatible namespaces reset the session. Full
standalone SMT scripts remain in reports. Feasibility checks omit unused counterexample models.
Refutations obtain their model in the same query context. Exact-query decisions are cached within a
root, with at most 1,024 entries or two MiB of query text; unknown responses are never cached. A
six-second host deadline covers pipe writes and reads, and failures discard the session.
MIR_CHECK_Z3 keeps the existing custom one-shot protocol, which relies on the executable's -T:6
timeout option. No proofs are reused across compiler invocations.

A bounded in-process evaluator decides fully constant Boolean/bit-vector queries and validates
the whole script before returning an answer. It supports exact wrapping arithmetic, bitwise
operations, signed/unsigned comparisons and sign/zero extension through 128 bits. Symbolic terms,
floating point, arrays and unsupported syntax fall back to Z3. Constant false failure conditions
can prove an obligation without a solver process; failing obligations still obtain Z3 models.
The evaluator is part of the trusted implementation, with boundary and differential regressions.

Compiler-identified core::array::from_fn executes actual callback bodies in ascending index order,
including checked call bounds and tracked Cell effects. Empty arrays do not invoke the callback.
Generated owned results have at most 128 elements and 256 modeled values, including containers.
Drop-bearing callbacks/elements and storage identities in generated elements remain unknown.
Tracked callback environments preserve mutable captures and owned FnMut state in index order. This
model does not permit general MaybeUninit or partially initialized storage.

Synthetic array-generation and iterator regressions exercise the relevant Rust features with
independent ticket, parcel, score and tally examples. Host/ARM proofs, rejected mutations and
bounded native execution check callback order, skips, exhaustion, zero-sized elements and aliases;
these cases do not constitute a completed soundness audit.

Proof execution reads typed rustc Body, StatementKind, Rvalue, TerminatorKind, Operand and Place
values directly. Printed MIR appears only in diagnostics and inventories; it is not parsed for
proofs. Panic calls use an explicit LangItem match, atomic Ordering uses normalized compiler type
and variant identities, and array from_fn resolves the actual compiler module child DefId.
Post-state contract detection parses syn expressions and distinguishes free final_ identifiers
from field names, comments and local Option bindings. Compiler-provided Symbols still identify
methods or variants when diagnostic items are unavailable; these are exact names within an
already identified compiler type/trait, not substring matches over MIR.

Compiler ArrayIntoIter models share bounded cursor storage with slice iteration. Owned elements
must contain no tracked identities or destructors and fit 128 elements/256 values. Models cover
forward/reverse skips, count/last, all/any and ordered fold/rfold callbacks. Ordinary sum MIR now
uses typed noncapturing closure constants and the cursor fold model. Captured constant closures,
including zero-sized captures, remain unknown. Iterator by_ref/IntoIterator preserve writable
cursor references; consuming methods update the original cursor. Wrapper drop glue is harmless
only when it has no own destructor and all drop-requiring fields are harmless owned iterators.
Owned element Clone, iterator views and user destructors remain unknown.

Exact compiler intrinsic models implement primitive signed/unsigned integer min/max, saturating
add/subtract, defined-zero leading/trailing zero counts, byte swapping and bit reversal. Normalized
signatures and modeled widths/signs gate each model. Saturation uses one extra SMT bit, including
129-bit intermediates for 128-bit inputs. Scoped let bindings preserve operand expressions without
raising query limits. Unsafe nonzero-only count intrinsics remain unsupported.

FnMut callbacks use one allocation-backed environment per modeled invocation. Map/from_fn,
all/any and fold/rfold execute actual bodies, carrying owned capture updates and external writes
between calls. Callback environments are retired after traversal. Ordinary mutable closure calls
also borrow their actual environment. Typed aggregate references allow supported Zip/Flatten MIR
and user structs/tuples to preserve reference identity without new assumed summaries.

Execution defaults are 8,192 steps and 16 active call frames, including bounded recursive calls.
Incomplete loops/recursion return UNKNOWN; the 30-second root and 200,000-byte query limits remain.
Exact Boolean/bit-vector folding simplifies closed MIR expressions before building longer terms. A
root-local cache stores up to 128 normalized instantiated bodies, keyed by the full compiler
Instance and shared with Rc. It avoids repeated cloning/substitution; it caches no proof outcomes or
cross-invocation compiler objects.

Optimized dependency MIR may return unit without assigning the return local; unit returns preserve
tracked effects without requiring that assignment. Core Option unwrap/expect panic helpers use the
actual Option module identity, exact helper names and never-returning signatures. Reachable helper
calls produce panic obligations even when their MIR body is unavailable. Same-named application
helpers execute ordinary MIR. Literal messages can cross shared reborrows; string content
operations remain UNKNOWN.

Computed float encodings keep stable symbols, but their numeric/bit relations are deferred until
a query references those symbols. Query construction follows exact symbol tokens transitively
and includes all relations if tokenization is uncertain. Numeric float expressions and ordinary
path conditions remain unchanged; exported queries include every required relation. This avoids
solving unused representation constraints during numeric-only path exploration.

Evaluated static string literals can pass through shared dereferences, reborrows, arguments and
returns as opaque immutable values. This reaches Option expect panic boundaries without modeling
string contents, lengths, equality or pointer identity. Mutable string-reference storage remains
UNKNOWN.

Local mutable byte borrows cross ordinary calls using allocation-backed references. Byte prefixes,
copy_from_slice and finite as_chunks_mut views preserve parent storage and disjoint offsets.
Zero-width chunks and mismatched copies produce panic obligations; symbolic lengths or views beyond
128 bytes remain UNKNOWN. Contracts can index fixed byte arrays at checked literal indices.

Root construction supports bounded integer pattern ranges and alternatives, valid Unicode char
scalars, and the compiler-identified NonZero getter. It does not infer constructor invariants for
other structs. Non-null pointer patterns and unresolved generics remain UNKNOWN.

Query construction skips encoding dependency scans when there are no deferred encodings and borrows
condition strings rather than cloning an intermediate list. Byte views and endian conversion use
scoped SMT bindings to share source expressions, with the usual size budgets still enforced.

Refuted roots stop after their first counterexample by default; remaining selected roots still
run. --all-failures continues collecting obligations under the existing budgets. Reports include
stopped_after_counterexample, and saved reports without that field still deserialize. CLI and
Cargo modes share this policy. A counterexample retains its full query and model.

Ordinary calls omit unused contract name maps and duplicate precondition setup. Actual predicates
and explicit argument aliases retain validation, while callee snapshots still check storage.
Compiler-identified scalar static-value hints use independent Boolean choices per call, following
the intrinsic's formal contract. Both branches are explored. Pointer hints and unsupported
transmute layouts remain UNKNOWN, with diagnostics naming their source and destination types.
