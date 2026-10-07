# mir-check

Host compiler adapter and report model. mir-check drives rustc directly; cargo-mir-check
uses it as a Cargo workspace wrapper and collects per-crate reports in an isolated build directory.

Human failure reports name the violated condition, source location and symbolic call chain.
UNKNOWN includes a next step for the actual limit or unsupported construct. REFUTED remains a
translated failure; native replay evidence is displayed separately, including a panic at a
different site or a replay that did not panic. A passed replay never converts REFUTED to PROVED.
`--replay` explicitly requests native execution of supported counterexample inputs; the default
scan executes no analyzed functions. Supported concrete assignments can be displayed without
running them. Native evidence includes the observed panic message and location, and records the
native replay's panic strategy, preserving the analyzed build's abort or unwind configuration.
Saved reports without the new evidence fields remain readable and show their runtime outcome as
unconfirmed. JSON and JSONL retain the complete evidence independently of compact display.

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

The smt module supplies the interpreter's typed term DAG. Analysis contexts intern
Boolean, bit-vector, floating-point and array nodes, reject mixed contexts and invalid operand
sorts, and fold supported closed terms structurally. The bounded printer preserves repeated
subexpressions through scoped lets. Z3 remains the solver and the subprocess protocol is unchanged.
Path conditions, contracts, byte storage and float encoding constraints retain nodes until query
printing. Deferred encoding dependencies are selected from symbol identities in the graph. Reports
retain standalone SMT scripts; analysis limits and UNKNOWN handling remain enforced.

Proof mode interprets a restricted subset of typed MIR, uses exact SMT bit-vectors for integers and
SMT floating-point operations for f32/f64 and follows actual arguments and return values through
concrete local and available dependency calls. Z3 runs as a subprocess. Every reachable panic
condition must be unsatisfiable; unsupported behavior and exhausted resource limits remain unknown
and cause verification failure. Finite loops are unrolled until every feasible path completes;
unfinished paths never become a passing result. Concrete generics, static trait implementations,
function items, known function pointers and supported mutable closures resolve to instantiated MIR
bodies. Available
dependency MIR is interpreted; unavailable bodies and unsupported shims remain unknown. Explicit
core models cover slice lengths/ranges, lossless integer conversion, integer endian
encoding/decoding, shared byte-slice-to-array conversion, fixed-array map, owned byte-array copies,
exact integer population counts, floating-point absolute value/min/max/clamp and static formatting
arguments. Primitive slice membership, including shared byte-pattern views, uses exact numeric
equality over at most 128 elements; symbolic byte lengths must be proved within that bound.
Custom equality executes actual MIR.
Ordinary execution models fixed-array equality and inequality for at most 128 elements per array.
Primitive comparisons use numeric equality, preserving NaN non-reflexivity and signed-zero equality.
Nested arrays and custom element types execute ordered, checked comparison bodies with tracked
references and short-circuit effects. The pinned core implementation uses element `ne`, including
user overrides;
the model preserves that behavior. Compiler crate/trait identities and instantiated signatures
select the boundary. Unbounded slice equality and inlined pointer-based comparisons remain gaps.
Array map executes actual callable bodies in order. Reports list interpreted bodies and
trusted models separately. MIR assume becomes a checked validity obligation. Typed allocations
support disjoint mutable root inputs, projected writes, reborrows and call state propagation.
Multiple mutable root pointees must have no interior mutation or reference fields. Scalar Cell
roots mixed with mutable roots remain unsupported. Tracked references in aggregates and captures
preserve writes to their allocations. Tracked allocation loads and projected writes share checked
identity, liveness and access capabilities with static views. Static validity epochs do not merge
static definitions and do not supply readable shared payloads. Provenance-backed local addresses
retain reference-escape evidence; their referents must remain live, and frame-owned addresses cannot
escape. Compiler type and projection certificates keep their existing rules during this migration.
General aliasing remains unsupported. Structs, symbolic input
enums and constructed variants preserve tags, fields and return facts. Small integer/bool/float
arrays support symbolic bounded indices and pattern projections; other elements require uniquely
determined indices. Struct inputs do not acquire implicit invariants.

Root inputs include tuples, nested local/dependency structs and enums, concrete generic fields,
supported shared references and small arrays of modeled aggregates. Input enums have at most 64
variants; every payload must be modeled, and downcasts require a proven tag check. Input
construction has a 16-level depth limit and a 512-value budget across arguments. Large eligible
subtrees use lazy descriptors: reference-free Freeze structs, tuples and fixed arrays with ordinary
scalar leaves. Accepted type shapes are cached across the entire root, including different enum
payloads and arguments. Every occurrence retains independent stable symbols; projected writes
preserve copies and entry snapshots. Descriptors share the 512-node budget and a root reserves at
most 262,144 symbol slots. Each lazy occurrence also consumes one node; cached descriptor fields
are charged only once. Rejected candidates never enter the shared cache. Enums, chars, NonZero and
compiler patterns keep eager validity
constraints, with lazy eligible children. Unsupported leaves, larger reservations and unresolved
lazy induction state remain unknown. General mutable-reference fields still fail before execution.
For inputs whose estimated eager size exceeds the budget, compact subtrees are selected before
other fields consume it. Shape sharing respects each occurrence's nesting depth; fields still have
independent values. Deferred owned records retain the existing 256-value repeat/iterator budget.
Input construction errors include the field, enum variant or array element that blocked it.
Input bindings retain nested names such as packet.header.index and value.1.0.
Bounded membership resolves lazy slice storage one level before primitive comparisons or checked
custom equality calls; it preserves the same element identities as ordinary field access.

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
rounding. Compiler-identified floor, ceil, trunc, round, round_ties_even, sqrt and fused mul_add
intrinsics use exact IEEE terms with explicit rounding modes. This includes the pinned compiler's
experimental no_std core_float_math functions; user functions with similar names retain their MIR.
The rounding models distinguish ties away from zero and ties to even. Square root handles negative
inputs as NaN, and fused multiply-add rounds once. These models do not summarize arbitrary libm
functions or enable floating-point state in induction. Float storage encodings support
to_bits/from_bits and same-width integer/float
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
supported aliases and calls. Integer atomic constructors executed in analyzed MIR create distinct
owned allocations. Local load/store/fetch_add/fetch_sub/swap and strong/weak CAS retain exact
history through aliases, analyzed calls and owned returns. RMW updates wrap without overflow panics.
Weak CAS additionally permits spurious failure. Load/store and CAS ordering restrictions are
checked, including symbolic Ordering arguments.

Root/shared static atomics still permit arbitrary per-access state; their initializers do not prove
startup state. Every trusted boundary discards precise local atomic history, even with modifies=[]:
publishing an alias can allow subsequent interference without modifying the value during the call.
Stores cannot restore precision after that boundary. Dead backing, unsupported pointer publication,
thread execution and borrowed frame escapes remain UNKNOWN. Reports identify the history policy.

Compiler-identified fence/compiler_fence wrappers check non-Relaxed orderings, including symbolic
arguments, then execute available MIR. Valid atomic_fence/atomic_singlethreadfence intrinsics return
unit without adding synchronization facts or changing tracked local storage. Invalid intrinsic
orderings remain UNKNOWN. The models do not prove memory ordering, publication, data-race freedom or
whole-function atomicity. RefCell guards, general UnsafeCell operations and raw pointers remain
gaps.

The fresh-counter fixture proves its first claim, while an occupied counter refutes. Native replay
agrees, and changing the fresh initializer from zero to one fails both proof and replay. A solver
assignment alone is not evidence that the analyzed program reaches a failure, especially when its
outcome depends on conservative shared atomic interference.

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
Non-byte root arrays allow 256 elements with eager values or bounded lazy descriptors; evaluated
constants permit 128 elements. Byte arrays retain their 128-byte limit. Storage identities (Cell,
atomics, tracked references and mutable byte views) are not cloned by the repeat model. Accessing
repeated inline-constant interior mutable storage remains unknown. Composite indices still require a
unique value on each path; this stage does not extend alias or iterator semantics.

Compiler-identified shared slice iterators retain a source and front/back cursors. Models cover
construction, next/next_back, nth/nth_back, len/count/size_hint, clone and all/any. Advancing
updates
typed iterator storage; adapters such as enumerate, copied and rev execute their actual MIR.
Predicate callbacks execute actual bodies in order, preserve memory effects and stop immediately
at the deciding element. Symbolic byte slices work when paths finish within the execution budget;
unbounded iterator loops, unsupported iterator views and raw pointer operations remain unknown.
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
host deadline of the solver timeout plus one second covers pipe writes and reads, and failures
discard the session.
MIR_CHECK_Z3 keeps the existing custom one-shot protocol, which relies on the executable's process
timeout option (-T:6 by default). No proofs are reused across compiler invocations.

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
can include tracked shared/mutable references and nested reference-bearing aggregates, but must
contain no owned Cell/atomic identities or destructors and fit 128 elements/256 values. Yielding a
reference preserves its allocation and projection; callbacks read and write that original storage.
The reference graph must remain live, and frame-local references cannot escape through an iterator.
Identity-bearing array repeats remain unsupported. Composite reference elements require a uniquely
resolved cursor index; symbolic skips selecting different reference identities stay UNKNOWN. Models
cover forward/reverse skips, count/last, all/any and ordered fold/rfold callbacks. Ordinary sum
MIR now
uses typed noncapturing closure constants and the cursor fold model. Captured constant closures,
including zero-sized captures, remain unknown. Iterator by_ref/IntoIterator preserve writable
cursor references; consuming methods update the original cursor. Wrapper drop glue is harmless
only when it has no own destructor and all drop-requiring fields are harmless owned iterators.
Owned element Clone, iterator views and drop-bearing owned iterator elements remain unknown.

Ordinary MIR Drop terminators execute rustc's concrete synchronous DropGlue shim. The compiler
orders the actual Drop::drop call and subsequent field drops, including active enum variants and
partially moved aggregate fields. Normal-return conditions and memory effects reach the caller;
panics inside destructors become ordinary panic obligations. Whole-local storage is retired after
its drop returns. No destructor is replaced by a no-panic summary. Unsupported glue operations,
missing destructor MIR, coroutine drops and unwinding remain UNKNOWN. Pointer-based array/slice
drop glue is currently unsupported, and experimental induction still rejects drop-bearing state.

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
Incomplete loops/recursion return UNKNOWN; the default root/query limits are 30 seconds and 200,000
bytes.
Exact Boolean/bit-vector folding simplifies closed MIR expressions before building longer terms. A
root-local cache stores up to 128 normalized instantiated bodies, keyed by the full compiler
Instance and shared with Rc. It avoids repeated cloning/substitution; it caches no proof outcomes or
cross-invocation compiler objects.

Structural guard simplification also normalizes integer comparisons, narrows zero-extended index
comparisons against representable bounds, and removes duplicate Boolean terms. A condition and its
exact negation make a conjunction infeasible without starting Z3, including when the path contains
unrelated floating-point conditions. Floating-point comparisons retain their NaN semantics. These
are exact identities, not inferred invariants; full query validation and size limits still apply.

Integer/Boolean safety obligations in mixed floating-point paths can first use a stronger query
that omits floating-point assertions. Only UNSAT discharges the obligation; SAT or UNKNOWN falls
back to the full path, and counterexamples always come from the full query. This uses typed term
sorts, retains the target failure condition and adds no assumptions. Custom solver executables and
Horn induction keep their existing strategy. Reports retain the full obligation query, and failed
path-feasibility checks now retain their full query too; SAT for a feasibility query describes a
path rather than a panic. No alternate floating-point tactic or repeated UNKNOWN retry is enabled.

Optimized dependency MIR may return unit without assigning the return local; unit returns preserve
tracked effects without requiring that assignment. Core Option unwrap/expect panic helpers use the
actual Option module identity, exact helper names and never-returning signatures. Reachable helper
calls produce panic obligations even when their MIR body is unavailable. Same-named application
helpers execute ordinary MIR. Literal messages can cross shared reborrows; string content
operations remain UNKNOWN.

Computed float encodings keep stable symbols, but their numeric/bit relations are deferred until
a query references those symbols. Query construction follows typed symbol identities transitively.
Numeric float expressions and ordinary
path conditions remain unchanged; exported queries include every required relation. This avoids
solving unused representation constraints during numeric-only path exploration.

Queries declare only symbols reachable from their assertions and required encoding relations.
Unobserved arithmetic storage therefore does not enlarge the script or change an otherwise
identical query's cache key. Sparse declaration sets still use the checked session reset path when
the existing declaration prefix cannot be reused. Counterexample models may omit unused,
unconstrained inputs; the input mapping records eager fields and summaries of lazy subtrees.

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

Query construction traverses typed assertion dependencies before printing. Byte views and endian
conversion use
scoped SMT bindings to share source expressions, with the usual size budgets still enforced.

Refuted roots stop after their first counterexample by default; remaining selected roots still
run. --all-failures continues collecting obligations under the existing budgets. Reports include
stopped_after_counterexample, and saved reports without that field still deserialize. CLI and
Cargo modes share this policy. A counterexample retains its full query and model.

Ordinary calls omit unused contract name maps and duplicate precondition setup. Actual predicates
and explicit argument aliases retain validation, while callee snapshots still check storage.
Ordinary place reads and static-write discovery share one projection evaluator. Static-write
discovery traverses the path once, preserving its existing storage and escape checks.
Compiler-identified scalar static-value hints use independent Boolean choices per call, following
the intrinsic's formal contract. Both branches are explored. Pointer hints and unsupported
transmute layouts remain UNKNOWN, with diagnostics naming their source and destination types.


Experimental `--induction` translates cyclic root MIR into typed Horn clauses for Z3 Spacer.
Integer/Boolean locals, tuples and fixed byte arrays have per-block state relations; initialization,
branches, assignments and backedges establish inductive panic freedom for any number of iterations.
Byte-array bounds are explicit safety clauses, including writes to persistent local arrays. Loop
exits are checked too. Cycle discovery follows concrete callees, so an acyclic entry can delegate
to a looping helper. Call graphs without a discovered cycle retain ordinary interpretation.

Inductive SAT is a safety model, not a counterexample. Raw JSON/JSONL reports retain those models
in the additive `invariants` field; old reports without it deserialize. UNSAT, malformed output and
solver timeouts remain UNKNOWN pending counterexample replay. Source assertions retain their actual
MIR meaning. Contracts do not inject runtime code. Root requires restrict the initial domain;
callee requires are checked at call sites and ensures are checked at actual returns, including root
returns. Independent entry snapshots preserve original arguments when a callee mutates its owned
values. Caller state is carried through callee block relations and actual return values resume the
caller. A contract never replaces execution of the body.

Concrete local, generic and retained dependency calls can contain loops. Typed allocations support
structs, scalar/byte-array borrows and stable field references. Callee writes update caller storage;
contract entry snapshots remain independent from those writes. Dynamic element indices are relation
parameters; changing allocation targets, slice views, recursion, floats, trusted boundaries and
interior mutation
and coroutines remain UNKNOWN in this mode. Unavailable bodies and unsupported predicates remain
UNKNOWN too.

The encoding has 256 total call-context blocks, 512 parameters per relation, 16 call frames and
200,000-byte script limits, with five-second Z3
and six-second host request deadlines. It proves panic freedom, not termination. Host/ARM original
fixtures include endless scalar/tuple loops, a register parser with byte history, exits and mutated
masks/cursors; native replay exposes mutated panics beyond the old execution budget. A host binary
main is also covered. The Cargo frontend forwards the option without inheriting a previous run's
setting. The default bounded interpreter and its budgets remain unchanged.


Host/ARM call fixtures cover nested generic instances, delegated entries, loops in callees, entry
snapshots of owned bytes, call domains and root/callee postconditions. Changed wraps, violated call
domains, false contracts, recursive calls and unsupported predicates never pass. Existing 256- and
1,024-iteration scalar fixtures also prove through induction. Host/ARM storage fixtures cover
persistent structs, shared array reads, borrowed byte writes and mutable callee snapshots. Mutated
writes and false snapshot claims never pass.

Integer `Range` loops have exact, compiler-identified `into_iter` and `next` models. Supported enums
carry a symbolic tag and every variant's typed payload through block relations; downcasts add safety
clauses before access. Original fixtures cover signed and maximum endpoints, early breaks, tagged
state and a custom iterator whose actual MIR body is translated. Bad ends, false assertions and a
late panic never pass. Optimized MIR locals used only by debug information do not consume state.
Slice iterators over byte slices and fixed integer/Boolean arrays support construction, identity,
next/next_back, nth/nth_back, len, size_hint, count, by_ref and shared cursor cloning. Safety
clauses
check front <= back <= length and every indexed access. Each yielded reference has independent
index parameters, so advancing the cursor preserves earlier references. Mutable byte writes use SMT
array stores; fixed scalar-array writes conditionally update exactly the selected element. Helper
bodies and their contracts remain checked, including entry snapshots of indexed pointees. Local
integer/Boolean arrays have a 16-element shape limit; existing root and relation budgets still
apply.
Arbitrary-length non-byte slices, slice views, changing allocation targets and iterator adapters
remain gaps. Compiler-inlined pointer internals also need models if a supported call is eliminated.

Host/ARM fixtures cover retained shared/mutable references, mixed-end iteration, skips, cloned
cursors, borrowed count exhaustion, scalar writes and indexed helper contracts. Bad bounds, stale
reference claims, false writes and false snapshot contracts never pass. Native replay and complete
finite unrolling cross-check the small cases. A supported 12,000-element range can still time out
during invariant inference; supported syntax alone does not guarantee a proof. Horn models print
without indentation to stay within the unchanged 256 KiB response cap. Solver strategies and time
budgets are unchanged.

A solver preprocessing experiment was rejected after a native-replayed late-panic mutation received
a false safety answer. The checker retains the ordinary Spacer encoding, and scalar relational
queries that crash or time out in Z3 remain UNKNOWN. No bit-level retry strategy is enabled.

Analysis budgets can be configured through both CLIs: --max-steps, --max-call-depth,
--max-query-bytes, --root-timeout-secs and --solver-timeout-ms. Values must be positive;
defaults preserve existing behavior. Ordinary execution and induction use the configured query,
call and time limits; step limits apply to ordinary execution. Reports record analysis_limits,
while older reports retain an unknown historical configuration. Rechecking uses fresh options.
Input, allocation and library-model shape caps remain implementation bounds. Exhausted or
unsupported analysis remains UNKNOWN. See [analysis budgets](../../docs/usage.md#analysis-budgets).

Ordinary slice equality and array/slice comparisons use the same numeric element semantics and
checked custom `ne` calls as fixed arrays. Length mismatches return before any element calls;
empty comparisons invoke no element body. Equal-length paths must establish a length at most
128. Comparisons ignore byte storage outside the selected prefix. Host/ARM debug and optimized
fixtures check numeric guards, prefix views, custom effects and receiver order, reachable and
skipped panics, and mutations with native replay. Arbitrary unbounded equality and general range
indexing still return UNKNOWN. These models are not part of the induction translator.

Constructed async futures now execute their lowered poll MIR. Compiler layouts map variant fields
onto shared saved-local slots, and captures retain tracked references. Polling can suspend and
resume, including nested futures and mutable captures; compiler completion-state assertions and
available cancellation drop glue are checked. Saved locals begin uninitialized and cannot be read
before assignment. Layouts have at most 64 states and 512 capture/saved-local slots.

A proof of an async factory covers future construction. Its deferred body is checked only when
polled by the analyzed root; construction models say this explicitly. Host/ARM debug and optimized
fixtures cover repeated suspensions, later panics, cancellation effects and panics, completed-future
polls, and source mutations with native replay. A standalone binary fixture checks from main and
rejects an out-of-bounds index in the resumed body.

Core task contexts are opaque valid values. The compiler's exact Context/NonNull adapter preserves
tracked mutable references, and core Pin<&mut T> mutable dereference preserves its pointer. Noop
waker and context construction have explicit compiler-identified models. Waker observations and
operations, arbitrary coroutine root inputs, unsupported saved values and unbounded async polling
remain UNKNOWN. Executor infrastructure is not modeled by this support.

Thin raw pointers constructed from integer addresses now retain target-width address terms through
casts, equality, null checks and aggregate storage. Numeric pointer constants without allocation
provenance are accepted. Core pointer-atomic construction checks the actual single-field wrapper
layouts and stores the handle; pointer-atomic loads/stores remain unsupported. This lets available
constructor bodies execute without a raw-pointer memory model. Arbitrary pointer inputs, allocation
provenance, reference-to-pointer conversion, metadata-bearing pointers, pointer arithmetic and
memory access remain UNKNOWN. Host/ARM tests cover signed and truncating casts, copies,
constructors, false claims and source mutations with native replay.

Lifetime-only mutable-reference transmutes preserve the tracked allocation and projection when
their erased MIR types match. Existing dead-storage and frame-escape checks remain enforced; this
does not establish general lifetime validity. An opt-in trusted returns_alias clause can return a
named mutable-reference argument with the same pointee type, preserving its alias under the claimed
memory effects. Its use remains visible as a user assumption, including on UNKNOWN and REFUTED
roots. Missing compiler hashes on local binary builds are reported as unavailable. Host/ARM
debug and optimized synthetic MIR tests cover writes, projected storage, invalid reference shapes
and frame escapes; valid scoped calls and failing mutations also replay natively.

Ordinary execution can recover a known typed static layout from an UnsafeCell byte carrier.
Compiler allocation provenance, the evaluated initializer's typed transmute, and target layouts
establish the storage identity, size, alignment and field offsets. A direct compiler intrinsic or
an available single-move transmute helper can establish the original type. UnsafeCell get/raw_get
and the inlined transparent pointer cast retain that identity. Restoration requires the original
or projected pointee type; matching sizes alone do not authorize unrelated types.

These are opaque storage views. Initializer bytes never become mutable runtime facts, and general
payload reads, arbitrary writes, unions and general fat pointers remain UNKNOWN.
Supported integer atomic fields use the existing arbitrary-per-access model. Exposed addresses are
symbolic, non-null, aligned and non-wrapping; they cannot reconstruct a dereferenceable view from an
integer. An unknown trusted memory effect invalidates existing views. A root reserves two memory
slots for invalidation state and stored-reference escape evidence. At most 512 view descriptors
are interned. Induction over these views remains unsupported. Host/ARM debug and optimized fixtures
check restoration, offsets,
atomic access, rejected layouts, invalidation and mutations, with native replay of valid cases.

Shared static arrays also support slice coercions and iterators over at most 128 opaque element
views. Compiler array strides preserve element addresses and field offsets. Direct indexing needs
a uniquely selected element; ambiguous composite indices and mutable opaque storage remain UNKNOWN.
Slice iterator `find_map` executes concrete callback MIR, retains captured side effects and stops
at the first modeled Some result. Unsupported callback shapes and destructors remain UNKNOWN.
Empty slice descriptors retain effect invalidation. These views do not load array payloads.

Opaque static views can reinterpret a dense prefix of Boolean/integer atomic fields as one
integer atomic. Compiler layouts must cover its footprint without padding or partial atomic leaves,
and the target must fit its certified source region and allocation at an aligned offset. Arrays
and nested structs are supported within the shape limits. Padding, MaybeUninit, unions, pointers
and ordinary fields outside the accessed prefix can be ignored; reading them remains UNKNOWN.
Destructors remain unsupported.
Reads use the existing arbitrary-per-access model without byte-order or initializer assumptions.
Shared atomic reborrows preserve their marker across temporary lifetimes. This does not prove
synchronization protocols or the validity of overlapping accesses and arbitrary overlay writes.

Integer compare_exchange and compare_exchange_weak support typed success/failure results. Strong CAS
succeeds exactly when its old value equals the expected value. Weak CAS may fail spuriously even on
a match. Success orderings accept all five Ordering variants; failure accepts Relaxed, Acquire or
SeqCst, including when stronger than success on the pinned core. Invalid orderings refute.
Replacement values are type-checked. Owned local storage retains the conditional update, while
conservative root/static accesses stay arbitrary. Pointer CAS remains UNKNOWN. Host/ARM debug and
optimized fixtures test signed values, result relations, ordering guards, spurious failure,
mutations and native replay.

MaybeUninit::as_ptr exposes the address of a certified shared static container without certifying
payload initialization. Payload reads remain UNKNOWN; supported typed stores are described below.
Static reference
values can be captured and replaced in tracked local slots while mutable static payload borrows
remain rejected. Concrete zero-argument closures and function items support Rust-call's empty-tuple
unit representation and execute their actual MIR. Host/ARM fixtures test these boundaries and
retain rejected reads, native scoped replay and layout mutations.

Function-item-to-function-pointer coercions preserve a resolved compiler instance and its exact
normalized signature in a root-local registry of at most 512 targets. Branches, local aggregates,
returns and Fn/FnMut/FnOnce adapters retain the selected target. Calls execute actual MIR and check
preconditions/postconditions and memory effects as usual. Missing dependency MIR, arbitrary root
function pointers, closure-to-pointer coercions, compiler reification shims, signature-changing
and numeric casts remain
UNKNOWN. Induction over pointer values and arbitrary opaque static writes remain unsupported.
Independent
host/ARM debug and optimized fixtures cover target selection, generic instances, adapter calls,
mutable effects, panic detection, contracts, missing bodies, target mutations and native replay.

Certified UnsafeCell static places accept supported whole typed stores without reading their old
payload or establishing facts about subsequent shared reads. Scalar, tuple, struct, enum, bounded
array, known callback and tracked address shapes are checked against compiler types. General union
values and unsupported interior-mutable payloads remain UNKNOWN. Address-only dereference/borrowing
of `MaybeUninit<UnsafeCell<T>>` retains its initialization barrier through get and transparent
casts; ordinary payload references and uninitialized atomic reads remain rejected.

A fresh constructed coroutine can be stored after checking its identity, capture types and
uninitialized saved-state slots. Its constructor executes normally; deferred poll/drop bodies are
not proved by the store. Payload reads, future polling through opaque static storage and resumed
future stores remain UNKNOWN. Stored tracked references are retained in a root memory slot for
existing frame/dead-storage escape checks, including across unknown trusted effects. This list is
conservative: overwrite does not remove earlier references. Each store has depth 16 and 512-value
limits, and a root can retain at most 512 reference entries. Unsupported shapes or limits fail.

Compiler-identified thin NonNull wrapping/unwrapping preserves a known static raw address after
pointee, size, alignment and field-offset checks. Numeric handles, unknown pointer inputs and fat
pointers do not gain this model. Independent host/ARM debug and optimized fixtures cover stores,
callback values, constructed futures, uninitialized-address chains, rejected shared reads,
read-only/numeric destinations, reference escape, unknown effects, mutations and native replay.

Shared concrete-reference coercions to compiler-identified core Debug trait objects retain an opaque
reference and its original storage identity. Reborrows, local aggregate storage, argument passing
and caller-storage returns are supported without invoking Debug::fmt. Opaque payload reads,
mutable/other trait coercions, vtable/address operations and induction over these references remain
UNKNOWN. Dead/frame-owned references still fail escape checks; erasure cannot remove their identity.

The pinned core Result unwrap_failed helper is a panic boundary after checking its compiler module,
name, never return type and shared str/Debug signature. It covers unwrap/expect and their reversed
variants through actual Result MIR branches. No user formatter or arbitrary external function is
assumed panic-free. Guarded success/error returns preserve their actual payloads, while feasible
failure-helper calls generate panic obligations. Same-named user traits/helpers do not match.
Independent host/ARM debug and optimized fixtures test positive, refuted and UNKNOWN cases,
formatter counters, reference escape, induction rejection, guard mutations and native replay.

Failure reports identify supported abstraction symbols that occur in the obligation query, including
conservative atomic reads, weak compare-exchange choices and arithmetic NaN encodings. These labels
explain modeling choices; they do not establish which choice caused a real panic.
Native replay records these relevant choices as `uncontrolled_abstractions`, because the generated
caller supplies root inputs without imposing an interfering atomic history, weak compare-exchange
outcome or arithmetic NaN encoding. This limitation remains visible for a confirmed native panic.
A normal return does not prove an abstract counterexample impossible: shared atomic cases need
initialization and interference checked against the selected root's execution environment. Static
startup precision still requires that environment to be verified rather than inferred from a static
initializer.

Tracked raw addresses can be formed from initialized whole locals and tracked reference reborrows.
Thin casts preserve the address and reference evidence; integer exposure preserves only the numeric
address. Repeated addresses of the same tracked place agree. Addresses are symbolic and constrained
to be nonzero and correctly aligned; relative offsets and distinct-allocation address comparisons
remain conservative. Raw dereferences, pointer arithmetic, untracked reference snapshots and
frame-owned pointer escape remain UNKNOWN.

`--verify --startup --entry function` selects an explicit fresh-startup domain for a zero-argument
root. Rust statics start with their declared initializers, with no external atomic interference
before a publication or opaque boundary. Supported integer atomic views retain compiler-decoded
initializer values and branch-local updates through calls and callbacks. Separate allocations and
subobjects keep separate histories; overlapping views discard precise history. Typed static stores
and trusted boundaries invalidate history, including statics not yet accessed. Shared static
payloads otherwise remain opaque. Unsupported initializer reads and cyclic induction stay UNKNOWN.

Startup proofs report `entry_assumptions` and use PROVED_WITH_ASSUMPTIONS, requiring
`--allow-assumptions` for a successful verification exit. Default arbitrary-root analysis keeps its
existing conservative shared atomic state. Cargo forwards the mode, and saved reports retain its
assumptions. The runtime initialization environment is an explicit premise, not a verified reset
handler or a global guarantee about concurrent actors.

Compiler-identified nonvolatile `atomic_store` intrinsics support primitive integers and thin
pointer values at writable certified static destinations. The actual core wrapper MIR executes;
pointer-store ordering checks report invalid acquire orderings before formatting its panic path.
Stores retain tracked pointer reference evidence and invalidate precise startup histories. A
pointer to frame-owned storage cannot escape through a static atomic. Integer-derived destinations,
uncertified type changes, pointer loads and volatile atomic operations remain UNKNOWN.

`UnsafeCell` casts can reach an actual initialized subobject at offset zero inside its payload,
including compiler alignment wrappers. The wrapper must preserve its payload layout, and each
subobject follows compiler fields and alignment checks. Equal sizes, padding, unions and
uninitialized members do not establish a certificate or write capability.
