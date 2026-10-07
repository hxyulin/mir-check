# Proof execution and boundaries

The current engine uses path-sensitive symbolic execution and SMT solving, not an interval
analysis or a general verifier for arbitrary Rust. The compiler adapter reads typed runtime MIR
from the pinned rustc with mir-opt-level=0. MIR supplies explicit types, branches, assertions and
calls; rustc does not supply the proof itself.

The interpreter matches typed Body, Rvalue, StatementKind, TerminatorKind, Operand and Place
values. It does not parse printed MIR or inventory details for proof decisions. Library models
use compiler definition identities, diagnostic/language items, normalized signatures and exact
Symbols within identified compiler types or traits. Panic calls use an explicit LangItem match;
atomic Ordering uses its actual compiler enum. Contract expressions use syn syntax trees,
including scoped post-state references. Strings remain appropriate for source selectors,
diagnostics and the separately generated SMT protocol.

A typed, interned term DAG carries symbolic values, path conditions and encoding constraints. Its
nodes carry explicit sorts and operator enums; construction checks arity, widths and
analysis-context identity before folding.
The printer introduces scoped lets when sharing saves bytes and enforces an output byte budget.
Closed Boolean and at-most-128-bit integer operations fold structurally; larger widened arithmetic
and floating-point operations retain exact solver terms. Numeric fp.eq is kept distinct from SMT
equality, including NaN and signed-zero behavior. Z3 differential tests exercise the operators and
constant boundaries. Byte views are typed offset-array nodes, lowered to capture-free lambdas by
the printer. The interpreter does not build or parse SMT expression strings. Query text is emitted
at the solver boundary and for report input descriptions. Execution and query limits below remain
unchanged; compact expressions can complete proofs that previously hit the query-size cap.

Language items identify compiler hooks rather than all language operations. Broad support also
requires complete MIR operations, memory/ownership rules, calls and intrinsics. Even complete
semantic support can leave a proof unfinished because loops, recursion, path growth or solver
queries exceed the analysis limits.

## Inputs and paths

Each selected root gets symbolic inputs. Integers are bit-vectors with the target's exact widths
and signedness. f32/f64 use SMT floating-point sorts with nearest-even arithmetic and numeric
NaN/infinity/signed-zero semantics. Float-to-integer casts truncate and saturate as Rust does,
including NaN-to-zero. Inputs/constants and from_bits values also retain their raw encodings,
including NaN signs/payloads. Moves, negation, abs and clamp preserve the selected encoding;
to_bits and same-width integer/float transmutes expose it. Each numeric arithmetic result gets
one stable encoding constrained to its IEEE value. For NaN, this permits every payload/sign,
including signaling encodings. This overapproximation makes proofs conservative, while bit-level
NaN counterexamples may not replay on the target. Float remainder remains unknown.
Booleans are SMT booleans. Byte contents are SMT arrays; slice lengths satisfy
valid-reference bounds. Struct fields are independent inputs, including private fields. No
constructor invariant is inferred for an arbitrary struct parameter.

Tuples and nested local/dependency structs and enums recursively carry modeled fields and shared
references to supported values. Input bindings retain names such as packet.header.index and
value.1.0. Reference snapshots do not track pointer identity or alias relationships; general mutable
roots support disjoint references to modeled pointees without reference fields. Eager input
construction is limited to 16 levels and 512 values across arguments. Eligible large subtrees use
shared lazy shape descriptors instead of eagerly creating a value for every repeated field.

Lazy inputs are reference-free, Freeze structs, tuples and fixed arrays built from ordinary
integers, Booleans, floats, unit and byte arrays. Each subtree is validated before it is accepted,
including unused fields. Enums, chars, compiler patterns, NonZero and interior mutable types use the
eager builder and its validity constraints; eligible children inside an eager container can still be
lazy. Accepted descriptors are cached by exact compiler type across the whole root, including
separate eager enum payloads and arguments. Each occurrence receives an independent,
pre-reserved symbol
range. Unaccepted candidates do not populate the cache.
Repeated reads keep the same symbols; by-value copies share their initial immutable descriptors, and
writes replace only the changed branch. Entry snapshots therefore retain their original values.
For roots exceeding the eager budget, a bounded size estimate selects useful compact subtrees
before constructing the other fields. Cached descriptors retain their height and must fit the
current nesting depth. Sharing describes types, never shared mutable storage or equality of values.

Lazy descriptors share the 512-node input budget and 16-level limit across arguments. Descriptor
nodes and field edges are charged once; every accepted lazy occurrence also consumes one node.
Non-byte arrays still have at most 256 elements. Materializing one level creates at most 512
immediate
values; nested aggregates stay lazy. A root reserves at most 262,144 symbol slots, and checked range
arithmetic rejects larger shapes as UNKNOWN. Byte arrays keep the existing SMT array encoding.
Reports summarize unmaterialized inputs rather than listing every field. Ambiguous composite
indices, unsupported leaves and unresolved lazy induction state remain UNKNOWN.
Owned iteration has a separate shape check from array repetition: a tracked reference counts as
one moved value, while repeats continue to reject references and other storage identities. Cursor
selection does not dereference element values. This preserves mutations and aliases through yielded
references and nested aggregates; the ordinary reference graph and frame-escape checks apply.
A symbolic composite index must select one provably determined element or the result is UNKNOWN.

Deferred records count all logical fields toward the existing 256-value owned repeat and iteration
budget. Descriptor validation excludes references and interior mutation, while the iterator's
existing compiler type check rejects drop-bearing elements.

Input enums have a symbolic discriminant restricted to actual compiler tags and separate modeled
payloads for each variant. The engine proves the tag before reading a downcast payload. At most 64
variants are supported; every variant payload must fit the input model. In reports,
`value.variantN.field` bindings describe a payload only when that variant is active.

The root's requires predicates restrict the input domain. The engine first checks that the domain
is satisfiable, refusing inconsistent preconditions as unknown. It then interprets each MIR block,
maintaining symbolic local values and path conditions. A branch adds its condition or its negation;
infeasible branches are removed only after an exact constant decision or Z3 answers unsat.

## Obligations

For every panic check, the engine asks whether this formula has a solution:

```text
valid-input constraints AND entry preconditions AND path conditions AND NOT safety condition
```

For bytes[index], the safety condition is index < bytes.len(). For a bounded call it is the
callee's requires predicate. For a feasible return it is the function's ensures predicate.
Unsat discharges that obligation for all inputs on that path. Sat yields a failing assignment.
An undecided solver query, unsupported operation or resource limit yields unknown.

The interpreter analyzes local callee bodies with actual symbolic arguments. Before entering a
callee it checks the callee's preconditions at that call. Return values and resulting conditions
flow back into the caller. An ensures annotation is checked against each actual return, never
assumed as a summary. Postcondition parameter names retain their entry values after reassignment.

The root passes only after all feasible paths finish and all obligations pass. A panic call ends
that path with an obligation that its path conditions are impossible. Failed assertions still
make the overall result refuted even though analysis can continue along their successful edge.
Reports retain queries, models, declared assumptions, input bindings and trusted models used.

Cargo root selectors match exact inventory names or crate-qualified names across selected
targets. Only selected roots acquire independent proofs; callees can still be interpreted with
the caller's particular values. A missing requested root fails after Cargo finishes, while
preserving reports. Schema version 8 counts root outcomes separately from unselected bodies and
distinct interpreted instances, and groups unknown obligations by reason. These are analysis
counts, not runtime coverage or whole-crate safety percentages.

Once an obligation refutes, that root stops by default with its counterexample query and model.
The checker continues every other selected root. `--all-failures` collects further obligations
under the normal budgets; it does not guarantee every failure will be found. A report's
`stopped_after_counterexample` flag distinguishes an early stop from continued exploration.
Saved reports predating the flag read it as false.

## Loops and limits

Loops repeat the interpreter over successive states. This can prove small finite domains, such as
the fixed three-device CAN configurations, without a loop invariant. Every feasible iteration must
finish. The default execution budget is 8,192 steps per root, including dequeued blocks, local calls
and
infeasible queued branches. Truncating unfinished paths would be unsound; reaching the budget
returns unknown. Infinite loops and larger finite loops can therefore remain unknown even when they
do not panic. Calls, including recursion, have a default limit of 16 active frames. Finite recursion
can
complete within those bounds; an unfinished recursive path returns unknown.

The opt-in `--induction` mode translates cyclic concrete call graphs into constrained Horn
clauses for Z3's Spacer engine. Each reachable MIR block has a relation over the function's
modeled locals. The entry clause contains the actual initialized arguments and checked entry
preconditions; other locals start unconstrained. Assignments, branches and backedges become
transition clauses. Assertions, panic calls and byte-array bounds failures become clauses
excluding those paths. Storage markers do not restrict values: safe Rust assigns locals before
reading them, and dead scalar values remain unconstrained in the entry state.

A satisfiable HORN system supplies an inductive model containing all initial states, closed under
every encoded transition and excluding every encoded failure. This proves panic freedom for any
number of iterations; it does not prove termination. Its SAT polarity is the opposite of an
ordinary counterexample query. The checker requests the model only after SAT, retains it in the
raw report's `invariants` array, and keeps counterexample `models` separate. Malformed models or
solver output, UNSAT and timeouts remain UNKNOWN. An UNSAT system does not yet produce a replayed
Rust counterexample, so it is not labeled REFUTED.

This first integration supports integer/Boolean locals, tuples, fixed byte arrays up to 128 bytes,
checked arithmetic, integer casts, branches, direct byte indexing/writes and loop exits. Scalar
MIR statements use the existing typed term operations; byte indexing has an explicit Horn safety
clause before a successful access. The translation is limited to 256 blocks and 512 scalar/array
state parameters. The default 200,000-byte script and five-second Z3/six-second host request limits
still apply. There is no iteration limit inside an inductive proof.

Cycle discovery follows concrete callee MIR as well as root backedges. An acyclic main or entry
can therefore delegate to a helper containing an endless loop. Discovery scans at most 128 concrete
bodies; missing bodies or an incomplete scan retain bounded interpretation and cannot license a
proof. Without a discovered cycle, roots use ordinary interpretation. Without `--induction`, all
roots retain the existing bounded interpreter. Cargo forwards the option explicitly and clears an
inherited induction setting when the option is absent.

A release-mode comparison on the local ARM64 host used the same fixture, flags and JSON output,
with one warm-up and five sequential samples per mode. These medians include compiler and CLI time;
the test suite was not running alongside the measurements.

| Fixture | Unrolling | Induction | Result |
| --- | ---: | ---: | --- |
| 256 completed iterations | 31 ms | 44 ms | Both PROVED |
| 1,024 completed iterations | 35 ms | 45 ms | Both PROVED |
| 20 symbolic doublings | 36 ms | 2,630 ms | Both PROVED |
| Endless wrapping counter | 39 ms | 44 ms | Unrolling UNKNOWN; induction PROVED |

Induction reduces repeated obligations but is not a universal speed improvement. Constant folding
makes these finite unrolled cases inexpensive, while Spacer must discover or reconstruct an
invariant. The endless case demonstrates added proof coverage, not a faster equivalent proof.
The opt-in setting preserves a choice between finite execution and inductive inference.

Each call site receives its own callee block relations. They carry immutable copies of the caller's
state as additional parameters, initialize the callee with the actual argument values, and return
actual results to the caller's destination. Nested callees preserve every ancestor's state. Stable
typed allocations carry borrowed storage as
mutable relation parameters. Addressed locals live in that storage, so aliases see the same writes.
Caller locals and entry snapshots are frozen separately; memory is carried through the actual
callee transitions and restored on return. Every edge checks allocation and reference identities
against the frame layout. Changing allocation targets remain UNKNOWN, and references to
interior-mutable storage are rejected. Struct fields and byte-array storage require no byte-level
raw-pointer model. Callee loops use the same transition encoding as root loops. Available local,
concrete generic and retained
dependency MIR is translated; a callee contract never replaces its body with a summary.

Root requires restrict the initial domain. Callee requires add both a failure clause at the call
site and a success constraint on entry. Ensures add a failure clause on each actual return. Entry
arguments have independent snapshot parameters when ensures are present; original and mutated
arguments therefore remain distinguishable. final_ names refer to the current argument values.
Declared predicates are checked for supported syntax even when a function never returns.
Unsupported predicates and invalid aliases remain UNKNOWN. Recursive call contexts, changing
allocations, interior mutation, slice views, arbitrary non-byte slices, iterator adapters,
coroutines,
float state and trusted boundaries still need inductive models. Unsupported behavior is never
omitted.

A [floating-point solver spike](floating-point-spike.md) found that Spacer can prove small exact
IEEE loops using the ALL logic and an explicit Horn tactic. It also hit the five-second budget on
a small late-failure counter. This is feasibility evidence, not implemented floating-point
induction: the current checker still rejects float loop state and does not emit that second logic
encoding. Ordinary float intrinsics use exact SMT terms independently of this experiment.

Supported enums have separate tag and payload parameters. Construction updates the selected payload
and tag, retaining a typed representation for inactive variants. Reading a downcast payload adds a
failure clause for the wrong tag and a success guard before access, including through static field
references. The shape budget is 512 type nodes and at most 16 variants per enum; payloads need the
same inductive representations as other state. Rustc's MIR visitor identifies executable local uses;
a local retained only for debug information has no proof state or executable operation to omit.

Integer ranges use exact library models identified by core crate, language item, trait and concrete
signature. `into_iter` returns the same range. `next` branches on `start < end`: the successful
branch
returns `Some(start)` and increments start; the exhausted branch returns `None` without changing
storage. The strict comparison guarantees increment cannot overflow for primitive signed or unsigned
integers. Custom iterators use their actual available MIR rather than this range model. Configured
contracts cannot be bypassed by the model.

Slice iterator models identify core types, concrete signatures and their traits before translating
operations. Iterator relations carry the source allocation/projection, front and back. Every step
checks front <= back <= source length with a failure clause. A successful next/nth yields a
reference
with the selected index and advances the front; next_back/nth_back update the back instead. Skipping
past exhaustion closes the remaining range at the appropriate end. Borrowed count exhausts the
original cursor. Shared clones have separate cursor parameters over the same storage.

An indexed reference carries its own bit-vector index rather than reusing the iterator cursor's
canonical variable. Index values can vary across transitions, while allocation, projection shape
and mutability must match the frame layout. Each dereference adds an index < length failure clause
before reading or writing. This supports retained references and indexed helper calls without a
raw-pointer model or an assumed non-aliasing relation. Actual transitions establish separation
between previously yielded mutable elements and the remaining range.

Byte slices use SMT arrays. Fixed integer/Boolean arrays use scalar fields and conditional writes:
element k becomes ite(index == k, new, old). Guards prove the index is valid before the update.
Contract entry snapshots encode the entry pointee value in separate parameters; current/final
bindings read current memory. Reference-layout inference chooses parameter types and identities,
and cannot replace callee execution or inject a safety assumption. Return and edge checks reject
incompatible layouts. Local scalar arrays are limited to 16 elements; root shape and relation limits
also apply. Arbitrary non-byte slices, offset slice views and general iterator adapters remain
UNKNOWN. Formatting and compiler-inlined raw-pointer internals still require supported models.
Ordinary execution has a bounded slice membership model selected by exact compiler identities and
instantiated scalar types. It uses numeric equality, including floating-point NaN and signed-zero
semantics, and never replaces a custom PartialEq body. Symbolic byte lengths must prove a bound of
128 elements before finite membership encoding; exceeding the bound is UNKNOWN, never truncation.
This library model does not extend the separate induction call translator.

The Horn printer sets pp.max_indent to zero. One mixed-end fixture's complete model shrank from
570,463 to 131,593 bytes by removing indentation, fitting the existing 256 KiB response cap. This
changes presentation only; bit-blasting remains disabled and solver strategies/timeouts are
unchanged.

The 256-block budget now includes every translated call context, and each relation's 512-parameter
budget includes captured caller state and contract snapshots. The call-depth default is 16 frames.
Some supported relational bit-vector queries crash or time out in Z3, which produces UNKNOWN. An
experimental bit-level preprocessing strategy returned a false safety answer for the native-replayed
12,000-iteration panic mutation. It was rejected; no such retry is enabled.

The original unbounded-loops fixture includes a wrapping counter, tuple state, a register parser
with persistent byte history, and a loop with an exit. Positive cases prove on host and ARM
`no_std`. Changed masks/cursors and a panic after 12,000 iterations never pass; native tests replay
the mutated panics. A plain binary `main` also proves. The parser's packet is a fixed arbitrary root
input, not a hardware read renewed each iteration. Actual async/executor firmware needs coroutine
storage, call effects and shared/hardware state modeled before a whole-main proof is possible.

By default, each SMT query is limited to 200,000 bytes, with a five-second solver timeout and a
six-second host
deadline per solver request. Reaching these limits is a verification failure. A root lazily starts
one Z3 process. Structured queries retain common assertion prefixes, pop the old branch suffix and
push new assertions. Live declarations are global, so extending the namespace retains shared
assertion scopes. An incompatible or shrinking declaration namespace resets the session. Feasibility
checks request a decision; a refuted obligation requests its model from the same query context.
Reports still contain full standalone SMT scripts. Malformed output, missing response markers,
closed pipes and timeouts discard the session and return UNKNOWN. The host deadline covers writes as
well as reads; responses above 256 KiB also return UNKNOWN.

For a nonconstant failure term without floating-point operations, query construction can also form
a probe by omitting assertions whose typed term trees contain floating-point or rounding-mode
sorts. It includes only assertions already present in the complete query, including relevant
latent encodings, and retains the actual failure term. The complete query implies this weaker
conjunction: UNSAT of the probe therefore proves the complete query UNSAT. SAT and UNKNOWN are
inconclusive and fall back to the complete query. Probe SAT never supplies a counterexample model.
Query validation and the full-query size cap apply before probing. No assertion is strengthened or
fabricated, and custom executables and Horn queries bypass this optimization. Default Z3 tactics,
timeouts and the text subprocess protocol are unchanged.

Reports keep complete safety queries even when a probe proves them. An undecided feasibility check
now also records its complete query in the UNKNOWN obligation. Its SAT answer would establish a
possible path, not a panic; only a REFUTED safety obligation represents a failing assignment.

An exact-query decision cache is local to the root and holds at most 1,024 entries or two MiB of
query text. Undecided responses are never cached. A cached satisfiable decision can answer a
feasibility check, but cannot supply a counterexample model. There is no disk proof cache, state
merging or cache of verified function summaries. A separate root-local cache shares up to 128
instantiated and normalized MIR bodies through Rc, keyed by the full compiler Instance. It does
not cache contracts, state or proof outcomes. Branch growth and complex solver queries can
therefore remain expensive.

Structural simplification folds supported closed Boolean and bit-vector terms after type and
arity validation. Integer operations wrap at their declared width; signed comparisons and
sign/zero extensions preserve that width's semantics. Closed arithmetic wider than 128 bits and
floating-point operations retain solver terms. Query construction validates every relevant
Boolean assertion and its context before recognizing an all-true domain or a false conjunct.
Integer comparisons normalize to less-than and its Boolean negation. Unsigned comparisons of a
zero-extended value against a constant use the original width only when the bound fits; an
out-of-range bound folds to the exact result. Boolean equality with true/false retains the original
guard or its negation. Boolean groups flatten and remove duplicate terms. Query construction
recognizes exact complementary conjuncts across assertions without splitting disjunctions.
Floating-point ordering is not normalized this way: NaN makes its comparisons non-complementary.
These identities can discharge an infeasible symbolic path without a solver request; full query
validation and size limits still apply before that decision.
Failing obligations still request a Z3 counterexample model. Remaining symbolic questions go to Z3
with the existing decision cache and persistent subprocess. No symbolic search runs inside
mir-check.
The folder is part of the trusted implementation and has differential tests against Z3. The old
string evaluator remains test-only for protocol regressions; production proofs do not use it.

Setting MIR_CHECK_Z3 retains the custom executable's one-shot stdin/EOF protocol, including its
process timeout option (-T:6 with default limits). This compatibility path relies on the executable
honoring that
option; the default persistent backend enforces the host deadline independently.

## What is trusted and missing

The result trusts rustc's lowering and types, this MIR interpreter, its predicate and constant-query
evaluators, the explicit core models, and Z3. Bit-vector, array and floating-point semantics
preserve supported integer, byte and numeric float operations, but the translator has not been
formally verified.
Mutation tests and runtime replays check representative semantics; they do not establish correctness
of the analyzer.

Trusted models implement slice length, byte prefix ranges, lossless integer conversions, endian
decoding, shared byte-slice-to-array conversion, fixed-array map/from_fn, exact owned byte-array
copies, opaque formatting arguments from evaluated static strings, and float abs/min/max. Exact
compiler-identified float intrinsics model floor, ceil, trunc, both rounding tie modes, sqrt and
fused multiply-add with explicit IEEE rounding.
Min/max ignores one NaN and permits either operand on equal numeric inputs, including signed-zero
ties. Numeric NaN outputs permit all storage encodings rather than selecting an assumed payload.
Array map executes each actual callable body; the model supplies array traversal and storage.
Fixed-array equality models core's typed comparison boundary for up to 128 elements per array.
Primitive elements use exact numeric equality; float NaNs remain unequal and signed zeros equal.
Custom and nested elements execute their actual resolved inequality body in index order, stopping
at the first mismatch and retaining effects, panics and call obligations. This follows the pinned
[core slice comparator][core-array-eq], which uses `ne` rather than assuming it complements `eq`.
Zero-length comparisons invoke no element body. Model selection checks the core crate, compiler
trait identity, shared-reference signature and fixed lengths. Slice equality and array/slice
comparisons first split on length equality, preserving the no-call length mismatch case. On equal
length paths, the model requires a proven bound of 128 and guards byte selects by the actual length;
contents outside a selected prefix do not affect equality. Fixed custom storage uses the same
ordered, checked inequality calls as arrays. Unbounded equality, oversized arrays and unavailable
or unsupported element bodies remain incomplete. Native tests check custom
overrides, numeric edge cases and failing mutations; this model is trusted translation code.
Compiler identities and
instantiated types select models. Dependencies and dynamic formatters are not assumed safe.
A solver model is not automatically replayed as a Rust test;
confirmed examples currently have separate runtime replay tests.

[core-array-eq]: https://github.com/rust-lang/rust/blob/1303417/library/core/src/slice/cmp.rs

Concrete generic arguments are substituted and normalized before execution. Static trait dispatch
resolves to a concrete implementation. Available dependency bodies, supported mutable closures and
function
items are interpreted with actual values. Unsupported shims and missing MIR still fail as unknown.
Cargo rebuilds direct/transitive dependencies with always-encode-mir and MIR optimization level
zero, retaining ordinary function bodies without trusting them. The outer wrapper preserves other
flags; --no-dependency-mir disables retention. Prebuilt sysroot libraries are not rebuilt. Reports
identify interpreted instances separately from trusted models. Compiler assume intrinsics become
validity obligations, so their predicates must be established on the current path.

Evaluated constants are inspected through rustc_const_eval, using compiler layouts and active
discriminants rather than decoding enum bytes by hand. Initialized scalar reads preserve integer,
boolean and floating-point values. Struct/tuple fields, active enum payloads and bounded
arrays/slices recurse into the same decoder. Immutable promoted/static references are snapshots
only when the pointee has no interior mutation; mutable global reads are rejected. Unions and
MaybeUninit remain unknown, even when a union was initialized. An inactive unsupported payload
does not need decoding. Constant shape limits are eight levels and 256 values, with at most
128 elements per evaluated array/slice. This adds compiler constant inspection to the
trusted translation boundary; it does not execute arbitrary runtime calls in rustc's interpreter.

Coverage remains limited by enum/struct slices, general aliasing, interior-mutable root
combinations, unresolved generic inputs, float remainder, trait objects, function pointers and
general iterator machinery, pointer-based drop glue and several MIR operations/constants,
including some constant shapes. Ordinary synchronous destructors execute through rustc's concrete
DropGlue MIR. Their normal-return effects and ordered field drops are checked; unwinding remains
unsupported.
Non-byte input arrays are limited to 256 elements. Symbolic bounded indices work for integers,
floats and booleans; enum/struct elements need a uniquely determined index. Array/slice patterns
prove their minimum length and index bounds before applying constant start/end offsets. Generic
roots with unresolved type parameters remain unsupported. Experimental loop induction supports
scalar/tuple/enum state, fixed byte arrays, typed storage and integer ranges.
Byte-slice/scalar-array iterators support indexed references. Automatic type invariants, dedicated
termination checks and verified general effects remain unsupported.

The unchanged DR16 Raw::parse fixture exercises concrete core Result/Option bodies, question-mark
propagation, three closures, array map, shifts and endian decoding. It proves panic freedom and
decoded bounds for every valid byte slice on the host and ARM target, without entry assumptions.
Separate tests cover Result question-mark success/error payloads and available external generic
and inline bodies, including an overflowing dependency call that is refuted. This expands coverage;
it is not a completed audit of the interpreter or its models.

A selected-root result is conditional on its recorded preconditions and build configuration. It
does not verify unselected callers, every workspace member, another compiler's binary, undefined
behavior, allocation failure, stack exhaustion, interrupt interactions or hardware timing.

Background references:

- [Rust MIR guide](https://rustc-dev-guide.rust-lang.org/mir/index.html)
- [Z3 bit-vectors](https://microsoft.github.io/z3guide/docs/theories/Bitvectors/)

## Typed storage and mutable receivers

Supported mutable references identify an allocation and field/index projection. Each path has its
own storage; calls receive it and return the updated storage with their result. Writes through
reborrows update the original object. Dead or uninitialized storage cannot be read. Projected
writes currently require initialized aggregates; non-byte array writes require a uniquely
established index. Byte writes support symbolic indices with proven bounds.

Root construction accepts multiple safe mutable references as distinct typed allocations.
Simultaneously usable safe mutable borrows have exclusive access to their pointee storage; this
does not require an SMT alias assumption. Reference fields inside mutable root pointees remain
unsupported. When there is more than one mutable root, every pointee must be Freeze (contain no
interior mutation). Shared scalar Cell roots mixed with mutable or other Cell roots remain UNKNOWN
in either argument order. Immutable Freeze roots retain snapshot semantics. Disjoint subslices and
fields may share a backing allocation in native Rust: separate root allocations model their
nonoverlapping contents, without modeling addresses or pointer equality. Multiple references created
from known local storage can cross supported calls. Tuples, structs, enums and closure environments
retain tracked references. Returns can retain references into incoming storage, including inside
iterators or closures. The returned graph and incoming storage are checked for references into the
returning frame or dead allocations. A local mutable borrow cannot escape by being nested.

Ordinary execution accepts mutable-reference transmutes whose erased MIR types are identical.
The reference must already identify live tracked storage, and its allocation, projection and
mutability are preserved. This covers a lifetime-only cast without inventing a static allocation.
Frame escape checks still apply. Shared snapshots, different pointees or mutability, and raw
pointer-to-reference conversions do not acquire this model. This is panic analysis with tracked
storage, not a proof of Rust lifetime validity.

FnMut calls borrow the actual closure environment. Models for map/from_fn, predicates and folds
allocate one environment per invocation and propagate owned field updates and captured writes
between callbacks. The temporary environment is retired after traversal. Local byte borrows use
tracked allocations across ordinary calls. Shared immutable byte values retain their snapshot
semantics; mutable byte regions keep allocation identities and offsets.

Host/ARM fixtures check independent scalar, struct-field and byte roots, shared snapshots, callee
preconditions and postconditions, and two-allocation loop induction. False final-state claims and
changed callee writes are refuted; interior/reference-bearing pointees remain UNKNOWN. Native replay
uses disjoint fields and subslices from the same backing object, plus failing bounds mutations.

Postcondition parameter names refer to entry snapshots. The `final_<parameter>` binding refers to
the argument's state at return, for example final_state.count or final_self.integral. Parameter
names beginning final_ are reserved when a function declares postconditions. Compiler-generated
metadata-only raw pointers support length extraction; they cannot be dereferenced as data pointers.
Storage is limited to 512 allocations per path. A default 30-second root budget is checked before
solver
queries; an in-flight query remains subject to the configured solver/process limits (five/six
seconds by default).
Exhaustion returns UNKNOWN, so expensive floating-point path exploration cannot run indefinitely.

## Cells and atomic counters

Compiler-identified scalar Cell models implement new/get/set/replace with allocation-backed
contents. Supported aliases and calls share updates. One shared scalar Cell root is supported;
multiple root locations with unresolved aliases remain unknown. RefCell guards/destructors and
general UnsafeCell/raw-pointer operations remain gaps.

Compiler-identified integer atomics support new/load/store/fetch_add/fetch_sub/swap. Their state is
conservatively arbitrary at each access, including statics whose initializer is zero. RMW calls
return an arbitrary old value; updates wrap and carry no arithmetic overflow panic. No subsequent
access is correlated with the operation, allowing interference without modeling a full concurrent
execution. Assertions about such relationships may refute in this abstraction; those assignments
are not automatically reachable executions. Load/store ordering restrictions are panic obligations,
including symbolic Ordering arguments. Unsupported operations and targets retain normal unknown
boundaries. The models and their interference policy appear in reports.

Compiler-identified fence/compiler_fence wrappers establish a panic obligation excluding Relaxed,
including when the Ordering is symbolic. The wrapper's available MIR still executes on valid
paths. This catches invalid orderings without requiring a model for the wrapper's panic formatting.
The atomic_fence and atomic_singlethreadfence intrinsic boundaries validate their signatures and
compiler-evaluated constant ordering enums. Acquire, Release, AcqRel and SeqCst return unit without
changing tracked local storage. Invalid or unsupported intrinsic orderings are UNKNOWN, not panic
counterexamples; the safe wrappers are responsible for their defined Relaxed panic behavior.

Fences add no happens-before or atomic-history constraints. Atomic accesses remain arbitrary and
uncorrelated across a fence. The interpreter follows sequential MIR control flow but does not treat
an entire function as indivisible, enumerate concurrent executions, or simulate CPU reordering and
caches. The fence model is a conservative abstraction for supported atomic-value panic checks; it
does not establish publication safety, data-race freedom or full weak-memory correctness. See the
Rust documentation for [fence](https://doc.rust-lang.org/core/sync/atomic/fn.fence.html) and
[compiler_fence](https://doc.rust-lang.org/core/sync/atomic/fn.compiler_fence.html).

An atomic-only wrapper such as validate::Site can be represented without reading its mutable
initializer as immutable data. Other interior-mutable constant references remain unsupported.

## External contracts and conditional proofs

JSON sidecars can supply checked requires/ensures clauses for unchanged code. Their default
execution is the same as source metadata: interpret actual bodies, prove call bounds and check
postconditions. Exact selectors and positional argument aliases are validated; stale or ambiguous
configuration fails instead of silently disappearing.

An explicit trusted summary replaces only calls to its selected concrete function. The engine
proves its preconditions, generates fresh supported return values, applies the claimed memory
effects and assumes its ensures clauses. Missing effects invalidate modeled storage facts;
an explicit effect list includes a trusted frame claim for storage outside that list. Supported
reference writes update known aliases. Unsupported ownership/alias shapes and inconsistent
summary constraints remain UNKNOWN. An explicit returns_alias clause can preserve one tracked
mutable-reference argument with the same pointee type, subject to the claimed memory effects and
existing escape checks. Generic summaries require a concrete instance selector.

The engine records every used summary, reason, call site, instance and available crate hash.
Local binary builds without compiler HIR hashing report an unavailable hash rather than requesting
an unsupported compiler query. Such a root
is PROVED_WITH_ASSUMPTIONS and is excluded from ordinary PROVED counts. Default verification
rejects it; --allow-assumptions explicitly accepts that conditional outcome. REFUTED and UNKNOWN
take precedence and still fail. A trusted function selected as a root executes its real body,
so an assumed call boundary never becomes a body proof. Source annotations alone cannot opt
into trust. See [contracts](contracts.md#explicitly-trusted-call-boundaries) for the policy.

## Deferred float storage relations

Each computed float receives a fresh stable encoding symbol. Copies and bit-preserving
transformations retain that symbol or a derived bit expression. The equality between its numeric
SMT value and the decoded encoding is included only when an obligation/path expression references
the encoding. Query construction follows exact symbol dependencies transitively; unfamiliar
lexical forms conservatively include all encoding equalities. This relies on every binary32/64
numeric SMT value having an IEEE encoding, including the existing NaN overapproximation. Ordinary
path constraints remain mandatory, and query limits apply after required relations are included.

Static string transport keeps an opaque immutable literal value. Shared reborrows and returned
references preserve that marker; length/content/equality/pointer operations and mutable storage
remain unsupported. It does not provide general string reasoning.

## Constructed async futures

The ordinary interpreter accepts coroutine aggregates with a compiler-provided lowered layout.
Captures use initialized fields; each saved local gets an uninitialized slot. A MIR visitor rewrites
coroutine variant-field projections to these logical slots, preserving the compiler's shared-local
mapping across variants. Field types remain the instantiated MIR types. SetDiscriminant changes only
the constructed coroutine's state, after checking its identity and the state's layout bounds.
Ordinary scalar and memory operations then execute the actual lowered poll body. Uninitialized
reads fail as UNKNOWN. A completed future's next poll reaches the compiler's panic-state check.

Opaque Context values represent any valid task context while hiding its fields. The exact core
Context/NonNull transmute pair preserves a tracked mutable reference, without a general pointer
memory model. Core Pin<&mut T> mutable dereference preserves the same tracked pointer; user pointer
implementations execute their own MIR. Core noop-waker and context construction use explicit typed
models. Waker behavior and context extensions are unsupported rather than assumed harmless.

A factory proof covers construction only. The model report states that the deferred body is checked
only when polled. A root that polls once covers that poll and reachable cancellation; code after a
pending await is checked when a later poll reaches it. Available coroutine drop glue executes the
same saved-local mapping and actual destructors. There is no automatic scheduler analysis or
induction over arbitrary coroutine state in this stage. Unsupported operations and exhausted
execution budgets remain UNKNOWN. Fixtures include nested futures, shared slots, mutable captures,
cancellation effects/panics, completion checks, mutations and native replay. A binary main fixture
checks its resumed async body, including a refuted index mutation.

## Thin pointer handles

The ordinary interpreter represents an integer-derived thin pointer as a target-width bit-vector
address, without an allocation or dereference operation. Integer-to-pointer casts truncate or extend
using the source integer's signedness; pointer-to-integer casts preserve the address and apply the
requested integer width. Thin pointer casts preserve the same address. Equality and inequality
compare addresses. Other pointer operations are unsupported. Raw pointer inputs are not introduced
as symbolic addresses, and pointer constants with allocation provenance remain UNKNOWN.

The pinned core uses pointer/word transmutes for address operations and a pointer/atomic transmute
for pointer-atomic construction. Address transmutes require equal widths. The atomic translation
checks the compiler's Atomic identity and pointer parameter, then normalizes the actual storage
fields. Every wrapper must be a core struct with one field at offset zero, pointer-sized layout and
no destructor. The leaf must have exactly the source pointer type. This models construction only;
loads, stores and raw memory operations do not acquire a model. A changed or unsupported layout
returns UNKNOWN. Available constructor bodies and user checks continue to execute normally.

Tracked references are separate from these address handles. Reference-to-pointer casts, general
reference transmutes, metadata-bearing pointers, pointer arithmetic and dereferences remain
incomplete. Tests check host and ARM widths, signed and truncating casts, null constants, copied
handles, aggregate constructors, rejected operations and mutations, with native replay.

## Opaque static storage views

Ordinary execution can retain compiler provenance for a whole reference to an interior-mutable
Rust static. The static's CTFE MIR identifies an original storage type when the initializer returns
a direct transmute or calls an available helper whose body only moves its argument through a
transmute. Multiple incompatible return definitions are rejected. Compiler-normalized types and
layouts must preserve size and sufficient alignment. Foreign statics, promoted mutable storage
and unsupported origins remain UNKNOWN. Constants in executed monomorphized MIR are evaluated in
the fully monomorphized typing environment, including generic alignment-check constants.

A view carries the static identity, original and projected pointee types, byte offset and whether
it denotes a shared reference, raw pointer or place. Field projections use compiler layouts.
Compiler-identified UnsafeCell get/raw_get and inlined transparent UnsafeCell pointer casts expose
its payload address without loading it. A shared reference can restore the original type at
allocation offset zero or retain the certified projected type. An unrelated same-size type does
not acquire a model. Each view must fit its allocation and satisfy its required alignment.

Mutable initializer bytes are never interpreted as current runtime state. General payload loads,
owned copies and writes remain UNKNOWN, including reads of MaybeUninit storage. Supported integer
atomic fields retain the existing conservative arbitrary-per-access behavior. Views do not prove
initialization protocols, alias exclusivity, data-race freedom or general Rust validity of mutable
bytes. Arbitrary pointer arithmetic, general fat pointers, unions and induction over this storage
remain gaps. This is a layout/provenance adapter, not a byte-level memory interpreter.

Shared fixed arrays can coerce to slices of at most 128 opaque element views. Compiler array
strides determine each element offset. Direct indexing requires a uniquely selected element;
ambiguous symbolic composite indices remain UNKNOWN. A separate slice descriptor distinguishes
storage elements from arrays whose values are references. Shared slice iterators retain selected
element references as their cursors advance, including reverse steps. `find_map` executes the
concrete callback MIR with its captured storage and stops at the first modeled Some result;
exhaustion produces None. Unmodeled callback results or destructors remain UNKNOWN. Payload loads
and mutable iterators into opaque storage remain unsupported. Empty slices retain an invalidation
marker even though they have no element views.

Address exposure uses one symbolic base per static, constrained to be non-null, sufficiently
aligned and to fit the allocation without wrapping. Projected offsets preserve that base. Absolute
addresses and disjointness between different statics are not assumed. Casting an exposed integer
back to a pointer does not restore the view. There is no arbitrary-address dereference operation.

One root memory slot tracks whether unknown effects have invalidated the static views. A trusted
summary with omitted modifies invalidates it, while an explicit frame preserving storage retains
it. At most 512 distinct view descriptors are interned per root; exhaustion remains UNKNOWN.
Compiler integration tests check host/ARM debug and optimized builds, rejected representations,
uninitialized reads, raw writes, false address claims, effect invalidation and failing mutations.
Valid scoped reinterpretations and address mutations also replay natively.
