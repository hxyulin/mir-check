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
| Integers and bool | Symbolic target-width values, signed comparisons, exact bit-vector operations and population counts | Char and raw pointers are unsupported inputs |
| Floating point | f32/f64 inputs/constants, IEEE arithmetic/comparisons, abs/min/max/clamp and integer/float casts | Remainder, raw bits and wider float formats are unsupported |
| Bytes | Shared byte slices and fixed arrays; symbolic contents and valid-reference length bounds | One mutable byte-slice root supports guarded writes; general aliases remain unsupported |
| Tuples | Nested values, shared references, field projections and numeric contract fields such as `value.1.0` | Destructured argument names with projected debug bindings are not contract bindings |
| Structs | Nested local/dependency structs, concrete generic fields and supported shared-reference fields | Unions, reference fields in mutable root pointees and unresolved generics remain unsupported |
| Fixed non-byte arrays | At most 16 input/constant elements; generated owned repeats support up to 128 | Larger arrays fail as UNKNOWN |
| Array indexing | Symbolic bounded integer/bool/float selection, start/end pattern offsets and uniquely determined composite indices | An ambiguous tuple/struct/enum index remains UNKNOWN |
| Enums | Local/dependency inputs with symbolic tags/payloads; constructed variants and core Option/Result/ControlFlow | At most 16 input variants, all payloads modeled; enum/struct slices remain unsupported |
| Mutable storage | One mutable root reference, field/byte writes, reborrows and call state propagation | Root pointees with reference fields, general aliasing, mutable returns/captures and partially initialized aggregates remain UNKNOWN |
| Interior mutation | Scalar Cell aliases/calls and integer atomic load/store/add/sub/swap with ordering checks | Atomics allow arbitrary per-access state; RefCell, pointer-based access and other operations remain gaps |
| Shared references | Read-only snapshots of supported values, including nested slice fields | Pointer identity, alias reasoning and writes through shared/interior mutable storage are not modeled |
| Constants | Compiler-evaluated structs/tuples, active enum fields, bounded arrays/slices and immutable promoted/static references | Unions/MaybeUninit, interior mutable storage and raw pointers remain UNKNOWN |

Root input construction has at most eight recursive levels and 128 values across all arguments.
References, aggregate containers and their children consume the budget. A symbolic byte array or
slice is one modeled value rather than one value per byte. Recursive reference shapes and budget
exhaustion return UNKNOWN before execution. Zero-length arrays do not require modeling an element
value. Struct fields remain arbitrary inputs; privacy and constructors imply no hidden invariant.
Enum selectors are constrained to the actual compiler discriminants, including explicit signed
values. A downcast must prove the active tag before reading its payload. Reports expose
`value.discriminant` and `value.variantN.field`; inactive payload bindings have no runtime meaning.

Constants use rustc_const_eval to read compiler layouts, discriminants and initialized scalars.
The decoder inspects only the active variant; it does not invent values for inactive or
uninitialized fields. Immutable references require a pointee without interior mutation, and the
compiler's inspection context rejects mutable global reads. Constant decoding has eight recursive
levels and 256 values, including containers/references and each element. Byte arrays/slices have
at most 128 bytes; other constant arrays/slices have at most 16 elements. Exhaustion is UNKNOWN.

## Execution and calls

| Feature | Current support | Boundary |
| --- | --- | --- |
| Branches | Path-sensitive states; discard a branch only after an exact constant UNSAT decision or an unsat solver response | No state merging or abstract interpretation |
| Loops | Complete finite unrolling through every feasible path | No inductive loop invariants; incomplete exploration is UNKNOWN |
| Generics and static traits | Substitute/normalize concrete arguments and resolve implementations | Unresolved generic roots, trait objects and unsupported shims are UNKNOWN |
| Dependencies | Cargo retains ordinary direct/transitive bodies at MIR level zero and executes concrete instances | Prebuilt sysroot/foreign bodies can remain missing; retained unsupported behavior is UNKNOWN |
| Closures and function items | Read-only captures and supported generic Fn/FnOnce calls | Mutable captures and function pointers are unsupported |
| Array map | Explicit traversal model, executing each actual callable body in index order | At most 16 elements; general iterators remain gaps |
| Array from_fn | Execute actual callbacks in ascending index order, with conditions and tracked effects | At most 128 owned elements and 256 values; drops, identity-bearing results and mutable captures remain UNKNOWN |
| Owned array iteration | Compiler ArrayIntoIter, ordered cursors, count/last, predicates and fold/rfold callbacks | At most 128 owned elements and 256 values; identities, user destructors, clone and views remain UNKNOWN |
| Iterator fold/sum | Execute actual fold/rfold callbacks with accumulator, order and memory effects; sum uses ordinary MIR | Unfinished folds, mutable callback environments and unsupported element/call shapes remain UNKNOWN |
| Evaluated closure constants | Typed noncapturing, zero-field, zero-sized closure values | Captured constants, including zero-sized captures, remain UNKNOWN |
| Integer operations | Arithmetic, overflow flags, comparisons, casts, boolean casts, bit operations and shifts | Optional overflow checks depend on build settings |
| Float operations | Nearest-even add/subtract/multiply/divide, negation, comparisons and saturating casts | NaN, infinities and signed zero are preserved numerically; raw NaN payloads are not modeled |
| Drop | Skip a concrete value only if rustc says it needs no drop | Destructor execution remains UNKNOWN |
| MIR assume | Prove its predicate as a validity obligation | Never turn it into an unchecked assumption |

The execution budget is 256 dequeued blocks per root, including callees and infeasible queued
branches. Call depth is eight; recursion is unsupported. Queries have at most 200,000 bytes, a
five-second solver timeout and a six-second host deadline per default solver request. Exceeding a
limit returns UNKNOWN. Root-local solver sessions reset before each query; bounded exact-query
caching avoids repeated decisions without reusing function proofs or unchecked summaries.

Explicit core models implement byte lengths/ranges/copies, shared slice-to-array conversion,
lossless integer conversion, endian decoding, fixed-array map and opaque formatting arguments from
static strings, plus compiler-identified float absolute value and min/max. They check their
applicable bounds/length conditions and are recorded per root. They are trusted translation code,
not proofs of the modeled library bodies. Dynamic formatting, arbitrary pointer operations and some
constant shapes remain unsupported.

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
| Read-only callbacks | Captured closures, function items, map and question-mark payload propagation | Mutable captures remain UNKNOWN |
| Generated arrays | Synthetic ticket/parcel/label cases, function items, zero length, 18/128-element arrays and exact value-budget boundary on host/ARM | Bad callback assertions, overflows, call bounds and label mutations refute; drops, storage identities and larger shapes remain UNKNOWN |
| Iterator audit regressions | Mixed forward/reverse skips, usize::MAX exhaustion, zero-sized elements, skipped byte storage and shared Cell aliases on host/ARM; 1,792 native cases | Wrong alias and exhaustion mutation refute; unsupported views and owned mutable callback environments remain UNKNOWN |
| Aggregate inputs | Nested/generic structs, tuples, shared byte fields and fixed struct arrays on host/ARM | An off-by-one nested call guard is refuted; mutable/recursive/oversized shapes are UNKNOWN |
| Floats | Host/ARM tests for NaN, zero signs, rounding, saturation and caller bounds | Bad NaN/zero/index/call assertions are refuted; remainder and raw bits remain UNKNOWN |
| Symbolic enums | Signed tags, Option/Result payloads, foreign nested inputs and entry snapshots on host/ARM | Variant/payload/off-by-one call mutations are refuted; unsupported payloads stay UNKNOWN |
| Array patterns | Fixed float arrays and guarded byte slices with start/end projections on host/ARM | An unequal-endpoint assertion is refuted; float slices remain UNKNOWN |
| Aggregate constants | Option::as_ref, niche layouts, signed enum tags, nested fields and immutable storage on host/ARM | Wrong payloads/guards and a mutated constant index refute; unions, mutable storage, transmutes and oversized shapes remain UNKNOWN |
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
Raw float bit observation remains UNKNOWN. These models follow the pinned compiler's core
intrinsic declaration and primitive clamp implementation; they do not trust application summaries.

## Owned aggregate repeats

Small repeated tuples, structs, enum values and nested arrays preserve their contents and variant
tags. Each copy owns its data: changing a field or byte in one copy does not change another copy.
Generated repeats have at most 128 elements and 256 modeled values, counting aggregate containers
and their children. Input and constant non-byte arrays retain their 16-element limit; byte arrays
retain their 128-byte limit.
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
Mutable iteration and iterator slice views are not part of this stage.

Host/ARM compiler tests cover integers, floats, units, tuples, bounded symbolic byte slices,
clones, mixed-direction traversal, huge skip counts, short-circuiting and shared Cell identities.
Wrong element assertions and reachable callback panics refute. Order/short-circuit mutations
refute, and 4,096 host cases agree with direct array formulas. Unbounded loops and unsupported
views remain UNKNOWN. Compiler-layout-derived singleton tags let optimized residual enums finish
without inventing initialized payloads; ordinary unavailable storage reads still fail.

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
formulas. Ambiguous composite writes and general mutable iterator returns remain UNKNOWN;
no broad raw-pointer, captured-reference or root-alias model was added.

Borrowed fixed-array/slice IntoIterator factories also use these cursor models; owned arrays have
a compiler-identified model over the same cursor representation. Compiler-identified primitive
finiteness is exact IEEE NaN/infinity
classification. Host/ARM tests compare f64 classification with abs < infinity and refute an
unconstrained finiteness assertion. This avoids spending MIR steps on the helper implementation
inside each callback while preserving the predicate's meaning.

Owned array cursors preserve value order, forward/reverse skips, count/last, checked predicate
callbacks and fold/rfold. Callback bodies execute rather than supplying assumed results. Shared
slice folds and ordinary sum bodies use the same callback execution. Consuming count/last/fold
through a mutable iterator reference updates that original cursor; by_ref and IntoIterator
passthrough preserve its reference. Owned iterator clone stays unknown because element Clone
implementations may execute user code.

Harmless owned iterator drop glue is recognized only when elements need no drop and every
drop-requiring field of a wrapper is itself harmless. A wrapper with its own destructor remains
unknown. Evaluated noncapturing closures require compiler-confirmed empty upvars, zero fields
and zero-sized layout. Captured constant environments are not fabricated, even when zero-sized.
