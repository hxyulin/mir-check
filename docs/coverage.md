# Coverage and evidence

Coverage means the Rust/MIR behavior the interpreter can analyze and the selected roots tested
against it. These tables do not imply that the interpreter or its explicit library models have
completed a soundness audit. Every result is tied to its compiler, target, flags and root domain.

## Inputs and values

| Feature | Current support | Boundary |
| --- | --- | --- |
| Integers and bool | Symbolic target-width values, signed comparisons and exact bit-vector operations | Char and raw pointers are unsupported inputs |
| Floating point | f32/f64 inputs/constants, IEEE arithmetic/comparisons, abs/min/max and integer/float casts | Remainder, raw bits and wider float formats are unsupported |
| Bytes | Shared byte slices and fixed arrays; symbolic contents and valid-reference length bounds | General mutable slice inputs are unsupported |
| Tuples | Nested values, shared references, field projections and numeric contract fields such as `value.1.0` | Destructured argument names with projected debug bindings are not contract bindings |
| Structs | Nested local/dependency structs, concrete generic fields and supported shared-reference fields | Unions, mutable fields and unresolved generics remain unsupported |
| Fixed non-byte arrays | At most 16 modeled elements, including structs and tuples | Larger arrays fail as UNKNOWN |
| Array indexing | Symbolic bounded integer/bool/float selection, start/end pattern offsets and uniquely determined composite indices | An ambiguous tuple/struct/enum index remains UNKNOWN |
| Enums | Local/dependency inputs with symbolic tags/payloads; constructed variants and core Option/Result/ControlFlow | At most 16 input variants, all payloads modeled; enum/struct slices remain unsupported |
| Shared references | Read-only snapshots of supported values, including nested slice fields | Pointer identity, alias reasoning and writes through shared/interior mutable storage are not modeled |

Root input construction has at most eight recursive levels and 128 values across all arguments.
References, aggregate containers and their children consume the budget. A symbolic byte array or
slice is one modeled value rather than one value per byte. Recursive reference shapes and budget
exhaustion return UNKNOWN before execution. Zero-length arrays do not require modeling an element
value. Struct fields remain arbitrary inputs; privacy and constructors imply no hidden invariant.
Enum selectors are constrained to the actual compiler discriminants, including explicit signed
values. A downcast must prove the active tag before reading its payload. Reports expose
`value.discriminant` and `value.variantN.field`; inactive payload bindings have no runtime meaning.

## Execution and calls

| Feature | Current support | Boundary |
| --- | --- | --- |
| Branches | Path-sensitive states; discard a branch only after an unsat solver response | No state merging or abstract interpretation |
| Loops | Complete finite unrolling through every feasible path | No inductive loop invariants; incomplete exploration is UNKNOWN |
| Generics and static traits | Substitute/normalize concrete arguments and resolve implementations | Unresolved generic roots, trait objects and unsupported shims are UNKNOWN |
| Dependencies | Execute available instantiated MIR | Missing bodies are not assumed safe |
| Closures and function items | Read-only captures and supported generic Fn/FnOnce calls | Mutable captures and function pointers are unsupported |
| Array map | Explicit traversal model, executing each actual callable body in index order | At most 16 elements; general iterators remain gaps |
| Integer operations | Arithmetic, overflow flags, comparisons, casts, boolean casts, bit operations and shifts | Optional overflow checks depend on build settings |
| Float operations | Nearest-even add/subtract/multiply/divide, negation, comparisons and saturating casts | NaN, infinities and signed zero are preserved numerically; raw NaN payloads are not modeled |
| Drop | Skip a concrete value only if rustc says it needs no drop | Destructor execution remains UNKNOWN |
| MIR assume | Prove its predicate as a validity obligation | Never turn it into an unchecked assumption |

The execution budget is 256 dequeued blocks per root, including callees and infeasible queued
branches. Call depth is eight; recursion is unsupported. Queries have at most 200,000 bytes, a
five-second solver timeout and a six-second process limit. Exceeding a limit returns UNKNOWN.

Explicit core models implement byte lengths/ranges/copies, shared slice-to-array conversion,
lossless integer conversion, endian decoding, fixed-array map and opaque formatting arguments from
static strings, plus compiler-identified float absolute value and min/max. They check their
applicable bounds/length conditions and are recorded per root. They are trusted translation code,
not proofs of the modeled library bodies. Dynamic formatting, arbitrary pointer operations and some
promoted constants remain unsupported.

## Contracts and root selection

`requires` predicates constrain selected root inputs and must be proved at every reachable call.
`ensures` predicates are checked at actual returns, with parameter names bound to entry values.
Supported predicates include comparisons, boolean operations, named/numeric fields, array/slice
lengths, constant non-byte array indices, integer/float casts and exhaustive unguarded Option
matches. Symbolic Option arms must be boolean; float literals are checked for type and range.
Arithmetic, dynamic indexing, arbitrary predicate calls and Result contract matches remain gaps.

Cargo can select exact or crate-qualified roots. Unselected callees can still be interpreted
with particular symbolic arguments; that does not independently verify their full input domains.
Schema version 7 separates root outcome counts, unselected bodies, interpreted instance counts
and grouped unknown reasons. An empty inventory or zero selected roots establishes no safety.

## Concrete evidence

| Case | Positive evidence | Negative or incomplete evidence |
| --- | --- | --- |
| Guarded scalar/slice operations | Universal symbolic proofs; exhaustive replay of all u8 addition pairs | Off-by-one, overflow, division, stale-guard and call-bound failures |
| CAN frames | Six unchanged methods and two payload-preservation harnesses on host/ARM | Invalid lengths/IDs, relaxed FD rules and broken byte copies are refuted |
| Bus validator | Two symbolic three-device families through 44-block nested-loop MIR on host/ARM | Five invalid families are refuted/replayed; arbitrary input slices remain UNKNOWN |
| DR16 parser | Unchanged 42-block body, exact length and decoded bounds on host/ARM without entry assumptions | Bad index and channel mask are refuted; 4,608 sample frames use independent formulas |
| Generic/dependency calls | Concrete local/foreign bodies and static trait dispatch preserve values | Generic precondition violation and dependency overflow are refuted |
| Read-only callbacks | Captured closures, function items, map and question-mark payload propagation | Mutable captures remain UNKNOWN |
| Aggregate inputs | Nested/generic structs, tuples, shared byte fields and fixed struct arrays on host/ARM | An off-by-one nested call guard is refuted; mutable/recursive/oversized shapes are UNKNOWN |
| Floats | Host/ARM tests for NaN, zero signs, rounding, saturation and caller bounds | Bad NaN/zero/index/call assertions are refuted; remainder and raw bits remain UNKNOWN |
| Symbolic enums | Signed tags, Option/Result payloads, foreign nested inputs and entry snapshots on host/ARM | Variant/payload/off-by-one call mutations are refuted; unsupported payloads stay UNKNOWN |
| Array patterns | Fixed float arrays and guarded byte slices with start/end projections on host/ARM | An unequal-endpoint assertion is refuted; float slices remain UNKNOWN |
| Cargo selection | Selected roots can prove beside unsupported workspace code | Ambiguous names select all matches; unknown/refuted/missing roots fail |

Mutation regressions change source guards, indices, masks and copies to ensure the corresponding
proof tests reject them. Separate runtime tests cover confirmed failures and formulas. These
checks provide practical evidence, not a formal verification of the translator or whole firmware.
