# Coverage and evidence

Coverage means the Rust/MIR behavior the interpreter can analyze and the selected roots tested
against it. These tables do not imply that the interpreter or its explicit library models have
completed a soundness audit. Every result is tied to its compiler, target, flags and root domain.

## Inputs and values

The interpreter consumes rustc's typed MIR enums and compiler types/definitions directly.
Debug MIR strings are report details, not proof input. Language items identify compiler hooks;
broad Rust support also requires MIR operations, memory/ownership semantics, call resolution
and intrinsics. Complete language support still cannot guarantee a complete proof for every
program: resource limits and undecided queries remain separate sources of UNKNOWN.

| Feature | Current support | Boundary |
| --- | --- | --- |
| Integers and bool | Symbolic target-width values, signed comparisons, exact bit-vector operations and population counts | Raw pointers remain unsupported; Unicode char and integer pattern domains are modeled |
| Floating point | f32/f64 numeric operations, casts and tracked IEEE storage bits | Remainder/wider formats remain unsupported; arithmetic NaN encodings are conservative |
| Bytes | Shared byte slices and fixed arrays; symbolic contents and valid-reference length bounds | Disjoint mutable byte-slice roots support guarded writes; general aliases remain unsupported |
| Tuples | Nested values, shared references, field projections and numeric contract fields such as `value.1.0` | Destructured argument names with projected debug bindings are not contract bindings |
| Structs | Nested local/dependency structs, concrete generic fields and supported shared-reference fields | Unions, reference fields in mutable root pointees and unresolved generics remain unsupported |
| Fixed non-byte arrays | At most 256 input elements; large eligible subtrees use bounded lazy descriptors; evaluated constants and generated owned repeats support up to 128 | Larger arrays fail as UNKNOWN |
| Array indexing | Symbolic bounded integer/bool/float selection, start/end pattern offsets and uniquely determined composite indices | An ambiguous tuple/struct/enum index remains UNKNOWN |
| Fixed-array equality | Primitive numeric equality and actual custom/nested comparisons; ordered short-circuit effects | At most 128 elements per array; general slice equality and inlined pointer-based comparisons remain gaps |
| Enums | Local/dependency inputs with symbolic tags/payloads; constructed variants and core Option/Result/ControlFlow | At most 64 input variants, all payloads modeled; enum/struct slices remain unsupported |
| Mutable storage | Disjoint mutable root references, projected writes, tracked aggregate/capture references and incoming-storage returns | Reference fields in root pointees, general aliasing and partial initialization remain UNKNOWN |
| Interior mutation | Scalar Cell aliases/calls, integer atomic load/store/add/sub/swap and fences with ordering checks | Atomics allow arbitrary per-access state; RefCell, pointer-based access and other operations remain gaps |
| Shared references | Read-only snapshots of supported values, including nested slice fields | Pointer identity, alias reasoning and writes through shared/interior mutable storage are not modeled |
| Constants | Compiler-evaluated structs/tuples, active enum fields, bounded arrays/slices and immutable promoted/static references | Unions/MaybeUninit, interior mutable storage and raw pointers remain UNKNOWN |

Eager root input construction has at most 16 recursive levels and 512 values across all arguments.
References, aggregate containers and their children consume the budget. Large reference-free Freeze
struct/tuple/array subtrees with ordinary scalar leaves can instead use lazy descriptors. Repeated
types share descriptors while retaining independent values. A bounded eager-size estimate selects
compact eligible subtrees before other fields consume the budget. Cached descriptor heights must
fit each occurrence's nesting depth. The descriptor budget
shares the same 512-node total and 16-level limit, counting each descriptor/field edge once and each
lazy occurrence once. Accepted descriptors are shared across eager enum variants and separate
arguments; unaccepted candidates are discarded. Arrays retain their 256-element limit and a root
reserves at most 262,144 symbol slots. One level is materialized on access or mutation, leaving
nested aggregates lazy. Unsupported leaves are checked even when unused. Enums, chars, NonZero and
compiler patterns retain eager validity constraints, with lazy eligible children. Snapshots and
by-value copies retain initial values after writes. Induction does not yet accept lazy state. A
symbolic byte array or slice is one modeled value rather than one value per byte. Recursive
reference shapes and budget exhaustion return UNKNOWN before execution. Zero-length arrays do not
require modeling an element value. Struct fields remain arbitrary inputs; privacy and constructors
imply no hidden invariant. Enum selectors are constrained to the actual compiler discriminants,
including explicit signed values. A downcast must prove the active tag before reading its payload.
Reports expose `value.discriminant` and `value.variantN.field`; inactive payload bindings have no
runtime meaning. Input-construction failures name the specific field, variant or array element.

Ordinary membership can read lazily represented fixed arrays, including nested scalar arrays and
custom element records. Custom equality still executes its actual body; changing that body or an
index guard refutes the original proof. Host/ARM cases and native execution check these
interactions.
Owned repeats and iteration count deferred fields against the same 256-value limit; small no-drop
records can remain deferred, while oversized values and identity-bearing storage remain UNKNOWN.

Constants use rustc_const_eval to read compiler layouts, discriminants and initialized scalars.
The decoder inspects only the active variant; it does not invent values for inactive or
uninitialized fields. Immutable references require a pointee without interior mutation, and the
compiler's inspection context rejects mutable global reads. Constant decoding has eight recursive
levels and 256 values, including containers/references and each element. Evaluated arrays/slices
have at most 128 elements. Exhaustion is UNKNOWN.

## Execution and calls

| Feature | Current support | Boundary |
| --- | --- | --- |
| Branches | Path-sensitive states; discard a branch only after an exact constant UNSAT decision or an unsat solver response | Ordinary mode does not merge states; induction forms per-block relations |
| Loops | Complete finite unrolling; opt-in Spacer induction over cyclic root MIR with integer/Boolean/tuple and fixed byte-array state | Induction includes available concrete callee MIR and checked contracts; typed stable references are supported; integer ranges, tagged enums and typed custom iterators are supported; byte-slice/scalar-array iterators and indexed references are supported; slice views, arbitrary non-byte slices, changing allocations, recursion and coroutines remain gaps; incomplete or undecided proofs are UNKNOWN |
| Generics and static traits | Substitute/normalize concrete arguments and resolve implementations | Unresolved generic roots, trait objects and unsupported shims are UNKNOWN |
| Dependencies | Cargo retains ordinary direct/transitive bodies at MIR level zero and executes concrete instances | Prebuilt sysroot/foreign bodies can remain missing; retained unsupported behavior is UNKNOWN |
| Closures and function items | Tracked captures, owned FnMut state and supported generic Fn/FnMut/FnOnce calls | Function pointers and unsupported call shapes remain UNKNOWN |
| Array map | Actual callback bodies in order, retaining capture state and reference-valued elements | At most 16 elements; callback destructors remain UNKNOWN |
| Array from_fn | Actual callbacks in ascending index order, retaining capture state and effects | At most 128 owned elements and 256 values; drops and identity-bearing results remain UNKNOWN |
| Owned array iteration | Compiler ArrayIntoIter, ordered cursors, count/last, predicates and fold/rfold callbacks | At most 128 owned elements and 256 values; owned Cell/atomic identities, user destructors, clone and views remain UNKNOWN |
| Iterator fold/sum | Actual fold/rfold callbacks preserve accumulator, capture state and effects; sum uses ordinary MIR | Unfinished folds, callback drops and unsupported element/call shapes remain UNKNOWN |
| Evaluated closure constants | Typed noncapturing, zero-field, zero-sized closure values | Captured constants, including zero-sized captures, remain UNKNOWN |
| Integer operations | Arithmetic, overflow flags, min/max, saturating add/subtract, zero counts, byte/bit reversal, comparisons, casts, bit operations and shifts | Optional overflow checks depend on build settings; unsafe nonzero count intrinsics remain unsupported |
| Float operations | Numeric IEEE operations, exact input/from_bits encodings, moves, negation, abs, clamp and to_bits | Arithmetic NaN encodings allow every payload/sign, including signaling NaNs; counterexamples may not replay |
| Drop | Concrete synchronous rustc drop glue executes user destructors and ordered aggregate field drops | Pointer-based array/slice glue, unsupported coroutine drops, unwinding and induction drops remain UNKNOWN |
| MIR assume | Prove its predicate as a validity obligation | Never turn it into an unchecked assumption |

The default execution budget is 8,192 steps per root, including callees, iterator model steps and
infeasible
queued branches. The default call depth is 16 active frames; recursion can finish within the same
limits.
By default, queries have at most 200,000 bytes, a five-second solver timeout and a six-second host
deadline per
default solver request. Exceeding a limit returns UNKNOWN. The default root budget is 30 seconds.
Root-local solver sessions retain common assertions with push/pop and reset incompatible declaration
namespaces. Exact-query caching, Boolean/bit-vector folding, exact complementary integer guards
and a full-Instance MIR cache reduce repeated work. Floating-point ordering keeps its NaN behavior.
None reuses function proofs or unchecked summaries; full standalone queries
remain in reports.

Mixed floating-point paths can discharge an integer/Boolean safety check with a stronger
non-floating query. Only UNSAT is accepted; SAT/UNKNOWN use the original full query, including all
floating constraints, and counterexample models come from that full query. Host/ARM and native
cases cover guarded array accesses, integer facts that require a float constraint, actual linked
counterexamples, unsupported remainder and an off-by-one mutation. Feasibility checks keep the full
path, and their failing queries now appear in UNKNOWN reports. No alternate floating-point tactic
or repeated UNKNOWN retry is enabled.

Explicit core models implement byte lengths/ranges/copies, shared slice-to-array conversion,
lossless integer conversion, endian decoding, fixed-array map and opaque formatting arguments from
static strings, plus compiler-identified float absolute value and min/max. They check their
applicable bounds/length conditions and are recorded per root. They are trusted translation code,
not proofs of the modeled library bodies. Dynamic formatting, arbitrary pointer operations and some
constant shapes remain unsupported.

Primitive slice membership identifies core's inherent slice method or its exact SliceContains
trait implementation and checks the instantiated signature and primitive element type. The model
computes membership over fixed scalar elements or byte storage with a proven length at most 128.
Shared byte-pattern Subslice projections use checked region bounds and shifted array storage.
Byte array selects are guarded by the actual length, including empty and shifted views; longer or
unbounded byte slices remain UNKNOWN. Floating equality preserves NaN non-reflexivity and equality
of signed zeros. Fixed custom element membership calls resolved PartialEq MIR in slice order,
preserving comparison panics, interior effects and short-circuiting. Host/ARM tests include
positive, negative, unknown and source-mutated cases, plus native replay. Inlining that erases the
supported call boundary remains a limitation, including inlined custom comparisons that expose
unsupported interior-storage or panic-formatting internals. Mutable Subslice addresses remain
unsupported.

Fixed-array equality and inequality recognize core's PartialEq implementations and its exact
SpecArrayEq trait, with shared-reference array signatures and matching fixed lengths. Primitive
elements use numeric equality, including char, NaN non-reflexivity and signed-zero equality.
Custom and nested elements call actual resolved `ne` MIR, matching the pinned core slice comparator;
the model preserves receiver order, call checks, panics, interior effects and early termination.
Arrays with no elements call no comparison body. Comparisons have at most 128 elements per array;
unsupported callees and exhausted execution/solver budgets remain UNKNOWN. Host/ARM debug and
optimized cases cover guarded comparisons, byte arrays at the limit, nested and derived records,
cross-type comparisons, overridden inequality, reachable and skipped panics, counterfeit method
names and the budget boundary. Three failing mutations are refuted and replayed natively.
Unsupported float remainder inside an element comparison remains UNKNOWN.
This comparison model currently applies to ordinary execution.

## Contracts and root selection

`requires` predicates constrain selected root inputs and must be proved at every reachable call.
`ensures` predicates are checked at actual returns, with parameter names bound to entry values.
Supported predicates include comparisons, boolean operations, named/numeric fields, array/slice
lengths, constant non-byte array indices, integer/float casts and exhaustive unguarded Option
matches. Symbolic Option arms must be boolean; float literals are checked for type and range.
Arithmetic, dynamic indexing, arbitrary predicate calls and Result contract matches remain gaps.

Cargo can select exact or crate-qualified roots. Unselected callees can still be interpreted
with particular symbolic arguments; that does not independently verify their full input domains.
Schema version 8 separates root outcome counts, unselected bodies, interpreted instance counts
and grouped unknown reasons. An empty inventory or zero selected roots establishes no safety.

## Concrete evidence

| Case | Positive evidence | Negative or incomplete evidence |
| --- | --- | --- |
| Guarded scalar/slice operations | Universal symbolic proofs; exhaustive replay of all u8 addition pairs | Off-by-one, overflow, division, stale-guard and call-bound failures |
| CAN frames | Six unchanged methods and two payload-preservation harnesses on host/ARM | Invalid lengths/IDs, relaxed FD rules and broken byte copies are refuted |
| Bus validator | Two symbolic three-device families through 44-block nested-loop MIR on host/ARM | Five invalid families are refuted/replayed; arbitrary input slices remain UNKNOWN |
| DR16 parser | Unchanged 42-block body, exact length and decoded bounds on host/ARM without entry assumptions | Bad index and channel mask are refuted; 4,608 sample frames use independent formulas |
| Generic/dependency calls | Concrete bodies/static traits; non-inline transitive dependencies on host/ARM, with configuration and encoded flags preserved | Dependency precondition violations and overflow refute; opt-out calls and unsupported retained operations stay UNKNOWN |
| Callbacks and aggregate borrows | Struct/tuple/Option references, returned captures, owned FnMut state, Zip/Flatten and ordered callback effects on host/ARM | Wrong field/state assertions and swapped-field mutations refute; unresolved root aliases remain UNKNOWN |
| Generated arrays | Synthetic ticket/parcel/label cases, function items, zero length, 18/128-element arrays and exact value-budget boundary on host/ARM | Bad callback assertions, overflows, call bounds and label mutations refute; drops, storage identities and larger shapes remain UNKNOWN |
| Iterator audit regressions | Mixed forward/reverse skips, usize::MAX exhaustion, zero-sized elements, skipped byte storage and shared Cell aliases on host/ARM; 1,792 native cases | Wrong alias, false reset-state claims and exhaustion mutations refute; unsupported views remain UNKNOWN |
| Aggregate inputs | Nested/generic structs, tuples, shared byte fields and fixed struct arrays on host/ARM | An off-by-one nested call guard is refuted; mutable/recursive/oversized shapes are UNKNOWN |
| Floats | Host/ARM numeric and storage tests, signaling/quiet NaN inputs, payloads, signed zero, casts and same-width transmutes | Wrong masks/signs and fixed arithmetic-NaN claims refute; remainder stays UNKNOWN and NaN encodings may overapproximate the target |
| Symbolic enums | Signed tags, Option/Result payloads, foreign nested inputs and entry snapshots on host/ARM | Variant/payload/off-by-one call mutations are refuted; unsupported payloads stay UNKNOWN |
| Array patterns | Fixed float arrays and guarded byte slices with start/end projections on host/ARM | An unequal-endpoint assertion is refuted; float slices remain UNKNOWN |
| Aggregate constants | Option::as_ref, niche layouts, signed enum tags, nested fields and immutable storage on host/ARM | Wrong payloads/guards and a mutated constant index refute; unions, mutable storage, unsupported transmutes and oversized shapes remain UNKNOWN |
| Cargo selection | Selected roots can prove beside unsupported workspace code | Ambiguous names select all matches; unknown/refuted/missing roots fail |

Mutation regressions change source guards, indices, masks and copies to ensure the corresponding
proof tests reject them. Separate runtime tests cover confirmed failures and formulas. These
checks provide practical evidence, not a formal verification of the translator or whole firmware.

Mutable-storage tests cover host/ARM writes through callees, branch isolation, byte-slice updates,
entry/final-state postconditions and rejected aliases/returns. A vendored PID excerpt preserves
its original bodies: reset proves with a final-state assertion, a configured update with concrete
inputs proves, invalid limits refute, and a reset-write mutation refutes. Universal symbolic PID
update exploration remains expensive; the root execution budget bounds that work.

Interior-mutation tests prove shared Cell writes through aliases, branches and array-map callbacks.
Atomic tests prove wrapping counters and guarded orderings, refute invalid orderings/unchecked
increments/history assumptions, and keep RefCell conflicts unknown on host/ARM. The fleet's
unchanged validate::Site::fail and total also prove through ordinary ARM Cargo verification.

External sidecars check unchanged body/caller contracts without a source dependency. Host/ARM
tests cover formatter-boundary preconditions, arbitrary Result failures, tracked effects and
strict rejection of conditional proofs. Additional tests cover unavailable dependency bodies
through Cargo, generic instance selection, invalid/stale configuration, inconsistent assumptions
and unsupported reference returns. Trusted summaries remain explicit assumptions, excluded from
ordinary proof counts; they expand caller analysis without establishing the skipped body's safety.
An external checked postcondition also proves final_state.integral == 0.0 for the unchanged fleet
controller::Pid::reset through ARM Cargo verification, without adding a firmware dependency.

## Population counts and floating-point clamp

The compiler-identified ctpop intrinsic counts individual bits exactly and returns u32, including
for signed operands and target-width usize/isize. The model does not recognize user methods by
name. Host/ARM tests prove width bounds, masked counts and byte count/complement relationships;
wrong bounds and a tightened mask bound refute. Unsupported pointer inputs remain UNKNOWN.

Primitive core f32/f64 clamp checks min <= max before producing a value. Reversed bounds and NaN
bounds refute; a NaN input remains NaN and equality preserves the input's signed zero. Tests cover
symbolic ordered bounds, infinities, signed-zero observations and a reversed-guard mutation.
Float storage observations preserve selected input/clamp bits. These models follow the pinned
compiler's core
intrinsic declaration and primitive clamp implementation; they do not trust application summaries.

## Owned aggregate repeats

Small repeated tuples, structs, enum values and nested arrays preserve their contents and variant
tags. Each copy owns its data: changing a field or byte in one copy does not change another copy.
Generated repeats have at most 128 elements and 256 modeled values, counting aggregate containers
and their children. Non-byte root arrays allow 256 elements with eager values or bounded lazy
descriptors; evaluated constants permit 128 elements. Byte arrays retain their 128-byte limit.
Tracked references, Cell/atomic identities and mutable views are excluded from repeat cloning.

Host/ARM tests prove six-by-six float matrix initialization, tuple/struct/enum copies, 18-element
scalar/enum arrays, 128-element generated arrays and byte writes inside nested tuples. An incorrect
copy assertion and a mutation that writes the wrong matrix row refute. Ambiguous composite indices,
excessive shapes and operations on repeated interior mutable storage remain UNKNOWN. Root input
budgets and general aliasing limits are unchanged.

## Shared slice iteration

Shared slice iterator identity comes from the compiler's SliceIter diagnostic item. The model
tracks a source and front/back cursors, advances typed iterator storage and yields original
references where allocation identity is available. Byte snapshots yield their exact symbolic
contents. Forward/reverse next, symbolic nth/nth_back, length/count/size_hint and independent
cloned cursors are modeled. Counts and skips use the cursor directly, without unrolling discarded
items. Generic enumerate/copied/rev adapters execute actual core MIR.

all/any execute concrete callback bodies in element order and preserve memory effects, including
Cell updates. A deciding callback stops traversal immediately and leaves the remaining cursor
intact. Iteration consumes the existing step/time budget; unfinished paths remain UNKNOWN.
Mutable iteration uses the same tracked storage machinery; iterator slice views remain unsupported.

Host/ARM compiler tests cover integers, floats, units, tuples, bounded symbolic byte slices,
clones, mixed-direction traversal, huge skip counts, short-circuiting and shared Cell
identities. Wrong element assertions and reachable callback panics refute. Order/short-circuit
mutations refute, and 4,096 host cases agree with direct array formulas. Unbounded iterator
loops and unsupported views remain UNKNOWN. Compiler-layout-derived singleton tags let optimized
residual enums finish without inventing initialized payloads; ordinary unavailable storage reads
still fail.

## Mutable slice iteration

Mutable iterators retain a writable source reference and yield projections into that allocation.
Forward/reverse traversal and skips share the cursor machinery with immutable iterators. Local
byte-array views are attached to typed storage; subsequent byte copies update that same storage.
Mutable enumerate and IntoIterator adapters preserve the source reference, with checked counter
increments under the active overflow configuration. Unwrapping a known Some mutable payload
preserves its tracked reference. These explicit models are recorded separately from body calls.

Host/ARM tests prove disjoint element writes, final-array postconditions, tuple updates, bounded
byte-slice clearing, byte-prefix writes followed by copies, mutable predicate short-circuiting
and six-by-six diagonal matrix initialization. A wrong-column mutation and incorrect alias or
overflow assertions refute. Two sets of 4,096 host cases compare reads and writes with direct
formulas. Incoming-storage iterator returns and tracked references inside aggregates/captures are
supported. Ambiguous composite writes, general raw pointers and unresolved root aliases remain
UNKNOWN.

Borrowed fixed-array/slice IntoIterator factories also use these cursor models; owned arrays have
a compiler-identified model over the same cursor representation. Compiler-identified primitive
finiteness is exact IEEE NaN/infinity
classification. Host/ARM tests compare f64 classification with abs < infinity and refute an
unconstrained finiteness assertion. This avoids spending MIR steps on the helper implementation
inside each callback while preserving the predicate's meaning.

Owned arrays can move tracked shared/mutable references and aggregates containing them. Cursor
steps preserve allocation/projection identity instead of snapshotting pointee values. Shared Cell
aliases observe writes through earlier yielded references; mutable elements update caller storage.
Reference liveness and frame-escape checks still apply. Owned Cell/atomic elements, element
Clone/views/destructors and symbolic skips between different reference identities stay UNKNOWN.
Repeating identity-bearing values remains unsupported.

Owned array cursors preserve value order, forward/reverse skips, count/last, checked predicate
callbacks and fold/rfold. Callback bodies execute rather than supplying assumed results. Shared
slice folds and ordinary sum bodies use the same callback execution. Consuming count/last/fold
through a mutable iterator reference updates that original cursor; by_ref and IntoIterator
passthrough preserve its reference. Owned iterator clone stays unknown because element Clone
implementations may execute user code.

Harmless owned iterator drop glue is recognized only when elements need no drop and every
drop-requiring field of a wrapper is itself harmless. Other ordinary drops execute rustc's concrete
glue, including a wrapper's actual user destructor. Unsupported glue remains UNKNOWN. Evaluated
noncapturing closures require compiler-confirmed empty upvars, zero fields
and zero-sized layout. Captured constant environments are not fabricated, even when zero-sized.

## Aggregate references and callback state

Tracked references retain allocation IDs and field/index projections inside tuples, structs,
enums and closures. Returned values can refer to caller storage; the reference graph rejects
callee-local or dead allocations escaping directly or through caller storage. Core Zip/Flatten
adapters can execute actual MIR over supported mutable iterators without assumed summaries.
One tracked environment per modeled callback invocation preserves owned FnMut fields and writes
through captures. Array map retains reference-valued input elements. Completed temporary callback
environments are retired; unsupported destructors do not become harmless by entering a model.

The synthetic aggregate fixture checks 19 roots on host/ARM: 14 prove, three refute and two remain
unknown. Native execution checks 700 bounded calls plus stateful callback examples and negative
panic catches. Swapping the mutable struct fields refutes the unchanged assertion. Three memory
regressions reject dead references and a callee borrow hidden in caller storage, while preserving
nested incoming references. These checks do not cover every alias/lifetime rule.

## Floating-point storage

Inputs, evaluated constants and from_bits preserve every IEEE encoding, including quiet/signaling
NaN signs and payloads. Moves, selected array elements, negation, abs and clamp retain the selected
encoding. to_bits and same-width integer/float transmutes expose it. Numeric arithmetic still uses
IEEE nearest-even operations; its stored result is constrained to that numeric value. NaN results
conservatively allow all payloads/signs, including signaling encodings. Repeated reads of one
stored result use the same bits, while a bit-level NaN counterexample may not replay on the target.

The storage fixture checks 17 roots on host/ARM: 13 prove, three refute and one remains unknown.
A changed sign mask refutes. Native checks include signed zero, subnormals, infinities and both
kinds of NaN for f32/f64, plus 1,024 integer-cast/arithmetic inputs. Checked sidecars force actual
core to_bits/from_bits bodies through typed transmutes and retain the same outcomes without
library summaries. Remainder and wider floating-point formats remain unsupported.

Compiler-identified binary32/binary64 floor, ceil, trunc, round, round_ties_even, sqrt and fused
multiply-add intrinsics now have typed IEEE models. Directional rounding uses RTN, RTP and RTZ;
round uses nearest ties away from zero, while round_ties_even, sqrt and mul_add use RNE. Returned
numeric results retain stable storage encodings with the existing conservative NaN payload rules.
This applies to the pinned compiler's experimental no_std core_float_math wrappers. It does not
replace user functions with matching names or summarize a dependency's math implementation.

The intrinsic fixture checks 14 roots on host/ARM: eight prove, five refute and remainder stays
unknown. Cases cover positive/negative rounding ties, signed zeros, infinities, NaNs, subnormals,
invalid square roots and a fused result differing from separate multiply/add. Native tests replay
the positive cases and all five failing mutations. Floating-point induction remains unsupported;
adding ordinary intrinsic models does not extend the loop-state encoding.

## Larger evaluated tables and numeric-only float exploration

Evaluated constants now allow up to 128 elements, with the same 256-value total and eight-level
depth budgets. Symbolic compatible scalar selection needs a bounds proof, while constant indices
select directly. Composite indices still require uniqueness. Host/ARM tests retain unknown results
for oversized, ambiguous, uninitialized and interior-mutable cases.

Deferred float encoding relations avoid translating unused storage observations into numeric-only
queries. Exact symbol dependency closure includes the required relations for computed, returned,
transformed and selected bit roundtrips. Wrong roundtrip assertions refute; native IEEE cases and
copy-correlation regressions check the retained encodings. Arithmetic NaN payload overapproximation
remains unchanged.

Opaque static string transport now supports guarded Option expect and nested shared arguments.
Unguarded expect calls refute; changed guards also refute. String length/content operations and
mutable string-reference storage remain UNKNOWN.

## Mutable byte regions and valid bounded roots

Local mutable byte arrays retain allocation identities through helpers, returned borrows and
closure captures. Prefixes and finite `as_chunks_mut` views share the original allocation; chunk
and remainder writes preserve disjoint regions. The exact core model checks nonzero chunk width
and a fixed length within 128 bytes. Symbolic chunk lengths, larger views and unresolved root
aliasing remain UNKNOWN. Literal byte indices in contracts require a fixed modeled length.

Function-item callbacks resolve concrete trait implementations before requesting their MIR.
Missing-body diagnostics distinguish foreign declarations from omitted prebuilt core bodies.
The existing Cargo `-Zbuild-std=core` path captures rebuilt core MIR, but unsupported operations
inside those bodies still remain UNKNOWN. Primitive integer endian encoding and decoding use
exact compiler identities, signatures and byte order.

Root arrays allow 256 non-byte elements with eager values or bounded lazy descriptors, and enums
allow 64 variants. Integer compiler patterns and Unicode char validity constrain the input domain.
The exact core NonZero getter reads the modeled scalar; other struct invariants are not inferred.
Exhausted budgets and unresolved input types remain UNKNOWN.

Original host/ARM cases check mutable region writes, returned views, caller bounds, callback
dispatch, endian boundaries and valid scalar domains. Native tests and failing mutations check
these effects independently. Byte-source sharing bounds expression growth without increasing
the query-size cap, and standalone SMT scripts remain available in reports.

Guarded unchecked integer MIR arithmetic generates validity obligations before continuing.
Compiler runtime-check operands follow the analyzed session's UB/overflow flags, and the actual
cold-path marker has no runtime effects. Original integer-boundary and host/ARM flag regressions
check these paths. Unsupported pointer continuations remain UNKNOWN.

Scalar static-value optimization hints produce independent Boolean choices, so both branches
must be safe. Unused contract name maps do not block dependency execution; declared predicates
and argument aliases still validate. Integer power currently progresses to a niche-layout
transmute, which remains unsupported.


## Unbounded loop induction

The opt-in induction fixtures prove endless wrapping counters, tuple state, a register parser with
persistent byte history and a loop with an exit on host and ARM no_std. A host binary main proves
without running it. These are actual typed MIR translations, following the earlier handwritten Horn
spike. The packet is an arbitrary fixed root input; renewing hardware reads requires call models.
Changed masks, cursors and a failure after 12,000 iterations never pass. Native tests replay the
mutated panics. Entry requires restrict the initial domain, callee requires are checked at call
sites, and root/callee ensures are checked at actual returns. Inconsistent entry domains and
unsupported predicates remain UNKNOWN.

Host/ARM call tests cover concrete generic instances, retained dependency bodies, an entry
delegating to an endless helper, callee loops, restored caller state and original/final
owned-byte snapshots. Changed wraps, violated call domains, false postconditions, recursion and
unsupported inputs never pass. Existing 256- and 1,024-iteration scalar loops prove inductively.
Typed memory tests cover persistent structs, static field aliases, shared array roots, mutable
byte-array roots and callee writes. Separate contract snapshots retain entry values. Off-by-one
writes, bad wraps and false snapshot contracts never pass; changing aliases and interior mutation
remain UNKNOWN. Integer range loops, tagged enum state and a custom iterator's actual body also
prove on host/ARM.
Tests cover signed endpoints, maximum endpoints, early breaks and borrowed-array writes. Mutated
ends, false custom-iterator claims and a late range panic never pass. Slice tests prove
shared/mutable
byte iteration, retained references, mixed forward/backward steps, skips, cursor cloning, borrowed
count exhaustion, fixed integer/Boolean arrays and indexed helper snapshots. Mutated assertions,
writes and bounds never pass. Native cases and small complete unrollings cross-check the encoding.
Slice views, arbitrary non-byte slices and adapters still remain unsupported. A supported
12,000-element range can exhaust invariant inference; it is not guaranteed faster than unrolling. A
relational decrement/count query remains UNKNOWN after a Z3
crash. A preprocessing experiment that claimed a native-replayed late panic was safe was
rejected and is not enabled.

Typed Horn tests check relation arity/sorts/context, exact printer budgets and a transition
mutation. Solver tests distinguish SAT inductive models from SAT counterexamples and check
resets between Horn and ordinary queries. Raw reports store inductive models separately in
invariants. Horn UNSAT and timeouts remain UNKNOWN pending Rust counterexample replay. See [loop
execution and limits](proofs.md#loops-and-limits) for the supported operations and budgets.

Both CLIs configure steps, call depth, query bytes, root seconds and solver milliseconds.
Reports retain the selected analysis_limits; old reports do not invent historical settings.
Host/ARM tests cover low-budget UNKNOWN results, completed bounded work at larger limits,
late failures and a mutated deep callee. Horn queries carry the selected timeout and byte cap;
a stalled persistent session is killed under its configured deadline. Input and model shape
limits remain independent. See [analysis budgets](usage.md#analysis-budgets).

Ordinary slice equality and array/slice comparisons now model the exact core PartialEq boundary.
Length mismatches return without comparing elements. Equal-length paths need a proven bound of
128; primitive comparisons guard each byte select by the real length, and fixed custom elements
execute ordered resolved `ne` calls. Empty comparisons, unequal unbounded lengths, numeric edge
cases, prefix views and custom effects have host/ARM debug and optimized tests. Mutations refute
and panic during native replay. Arbitrary unbounded equality, general range indexing and induction
comparison translation remain gaps. Increasing execution budgets does not remove model shape caps.

Constructed async futures execute optimized poll MIR with compiler-provided saved-local layouts.
Variant fields that share a saved local use one logical slot. Captures, mutable writes, nested
futures, suspension/resumption and completion-state checks have host/ARM debug and optimized tests.
Available cancellation drop glue is interpreted, including destructor effects and panics. Saved
locals are explicitly uninitialized until assigned; unsupported reads never become symbolic data.
The state model allows at most 64 variants and 512 capture/saved-local slots.

Core Context is an opaque valid argument; its waker/extension fields cannot be observed. The exact
compiler Context/NonNull reference adapter and core Pin<&mut T> mutable dereference preserve tracked
storage. Compiler-identified noop-waker/context constructors let a standalone binary fixture prove
from main through two polls; a bad resumed index refutes and panics natively. Construction-only
proofs cover the async factory, not deferred execution. Unbounded polling, arbitrary coroutine root
states, waker operations and executor internals remain incomplete.

Integer-derived thin pointer handles support address casts, address exposure, equality and null
checks. Numeric constants without allocation provenance preserve their exact target-width address.
Copied handles and aggregate storage retain that address. Core pointer-atomic construction checks
normalized, single-field core wrappers with zero field offset and pointer-sized representation;
operations on pointer-atomic memory remain unsupported. Constructor bodies still execute and their
panics remain checked. This is address-value translation only: arbitrary pointer inputs,
reference-derived pointers, allocation provenance, metadata, arithmetic and dereferences remain
UNKNOWN. Host/ARM debug and optimized fixtures include positive, negative, unknown and mutated
cases, with native replay of address operations and failing claims.

Lifetime-only mutable-reference casts keep their tracked allocation and projection when the erased
MIR reference types match. Typed writes and frame-escape checks still apply; no static allocation
or general lifetime guarantee is introduced. Explicit trusted returns_alias clauses can preserve
one named mutable-reference argument with the same pointee type and the claimed memory effects.
Assumptions are visible on successful, refuted and unknown roots. Host/ARM synthetic MIR fixtures
check scoped writes, rejected casts, dead storage and escaping borrows, with native mutation replay.
General access to interior-mutable byte storage through raw pointers remains unsupported.

Opaque views now support recovering compiler-known typed static storage from UnsafeCell byte
carriers. Whole static provenance, initializer transmute origins, transparent UnsafeCell layouts,
size, alignment and field offsets are checked. Pointer casts and shared reference restoration keep
the storage identity; unrelated same-size types remain UNKNOWN. Initializer bytes are not runtime
state. Supported integer atomic fields use their existing conservative model; general payload
reads/writes, MaybeUninit reads and induction over this storage remain gaps. Shared fixed arrays
support slice coercions and ordered element references for at most 128 elements, with compiler
strides and distinct storage descriptors. Slice iterator `find_map` executes actual callback MIR,
including side effects and short circuiting. Mutable opaque element storage and ambiguous symbolic
composite indices remain UNKNOWN. Address checks use symbolic non-null aligned bases without
enabling arbitrary memory access.

Compiler-identified fence/compiler_fence wrappers validate non-Relaxed Ordering arguments before
executing actual MIR. Their intrinsic boundaries validate constant ordering enums and preserve
tracked local storage. Both compiler and hardware fences add no synchronization facts: atomic
accesses still allow arbitrary per-access values. Host/ARM debug and optimized tests check all valid
orderings, symbolic guarded and invalid orderings, native panic replay, mutations and unsupported
payloads. Invalid direct intrinsic orderings remain UNKNOWN. This does not establish whole-function
atomicity, publication safety or full weak-memory correctness.

Dense Boolean/integer atomic prefixes in certified static storage can form an integer atomic
view. Compiler offsets and strides must cover every accessed byte without padding or cutting a
leaf, and the allocation must satisfy alignment at the projected offset. Opaque fields and padding
outside that footprint can be ignored. Reading padding, uninitialized storage, pointers or
ordinary payload fields, and destructors, remain UNKNOWN. Shared atomic reborrows retain their
opaque
marker. Host/ARM debug and optimized tests cover nonzero offsets, Boolean arrays, arbitrary values,
rejected regions, native scoped replay and layout/offset mutations. This adds no synchronization or
atomic-history facts and does not verify overlapping concurrent accesses.

Integer strong and weak compare_exchange model their old value and success/failure relationship,
with symbolic ordering checks. Weak CAS allows spurious failure. All 15 valid success/failure
ordering pairs in the pinned core are accepted; Release and AcqRel failure orderings refute. The
model adds no atomic history or synchronization facts. Pointer CAS remains UNKNOWN. Host/ARM tests
include positive, refuted and unknown cases, mutations and native replay.

Certified shared static MaybeUninit containers support as_ptr with size/alignment checks. The raw
payload address keeps the container's initialization barrier; it does not authorize reading T.
General union reads and MaybeUninit initialization/writes remain unsupported. Tracked local static
reference slots support shared captures and mutable slot replacement. Actual mutable static
payload borrows remain UNKNOWN. Concrete zero-argument closure/function-item calls now accept the
empty Rust-call tuple's unit representation, with actual callback MIR execution.
