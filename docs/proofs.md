# Proof execution and boundaries

The current engine uses path-sensitive symbolic execution and SMT solving, not an interval
analysis or a general verifier for arbitrary Rust. The compiler adapter reads typed runtime MIR
from the pinned rustc with mir-opt-level=0. MIR supplies explicit types, branches, assertions and
calls; rustc does not supply the proof itself.

## Inputs and paths

Each selected root gets symbolic inputs. Integers are bit-vectors with the target's exact widths
and signedness. f32/f64 use SMT floating-point sorts with nearest-even arithmetic and numeric
NaN/infinity/signed-zero semantics. Float-to-integer casts truncate and saturate as Rust does,
including NaN-to-zero. Float remainder and raw bit observation remain unknown.
Booleans are SMT booleans. Byte contents are SMT arrays; slice lengths satisfy
valid-reference bounds. Struct fields are independent inputs, including private fields. No
constructor invariant is inferred for an arbitrary struct parameter.

Tuples and nested local/dependency structs and enums recursively carry modeled fields and shared
references to supported values. Input bindings retain names such as packet.header.index and
value.1.0. Reference snapshots do not track pointer identity or alias relationships; general
mutation and mutable fields remain unsupported. Input construction is limited to eight levels and
128 values across arguments, so recursive reference shapes and large aggregate trees fail as
unknown.

Input enums have a symbolic discriminant restricted to actual compiler tags and separate modeled
payloads for each variant. The engine proves the tag before reading a downcast payload. At most
16 variants are supported; every variant payload must fit the input model. In reports,
`value.variantN.field` bindings describe a payload only when that variant is active.

The root's requires predicates restrict the input domain. The engine first checks that the domain
is satisfiable, refusing inconsistent preconditions as unknown. It then interprets each MIR block,
maintaining symbolic local values and path conditions. A branch adds its condition or its negation;
infeasible branches are removed only after Z3 answers unsat.

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
preserving reports. Schema version 7 counts root outcomes separately from unselected bodies and
distinct interpreted instances, and groups unknown obligations by reason. These are analysis
counts, not runtime coverage or whole-crate safety percentages.

## Loops and limits

Loops repeat the interpreter over successive states. This can prove small finite domains, such
as the fixed three-device CAN configurations, without a loop invariant. Every feasible iteration
must finish. The global execution budget is 256 dequeued blocks per root, including local calls
and infeasible queued branches. Truncating unfinished paths would be unsound; reaching the budget
returns unknown. Infinite loops and larger finite loops can therefore remain unknown even when
they do not panic. Recursive calls and depths beyond eight also remain unknown.

Each SMT query is limited to 200,000 bytes, with a five-second solver timeout and six-second
process limit. Reaching these limits is a verification failure. The prototype starts solver
processes for separate queries and does not merge states or cache verified function summaries,
so branches and loop iterations can make it expensive.

## What is trusted and missing

The result trusts rustc's lowering and types, this MIR interpreter and predicate evaluator, the
explicit core models, and Z3. Bit-vector, array and floating-point semantics preserve supported
integer, byte and numeric float operations, but the translator has not been formally verified.
Mutation tests and runtime replays check representative semantics; they do not establish correctness
of the analyzer.

Trusted models implement slice length, byte prefix ranges, lossless integer conversions, endian
decoding, shared byte-slice-to-array conversion, fixed-array map, exact copies into owned byte
arrays, opaque formatting arguments from evaluated static strings, and float abs/min/max.
Min/max ignores one NaN and permits either operand on equal numeric inputs, including signed-zero
ties. Raw NaN payload/sign observation is unsupported. Array map executes each
actual callable body; the model supplies array traversal and storage. Compiler identities and
instantiated types select models. Dependencies and dynamic formatters are not assumed safe.
A solver model is not automatically replayed as a Rust test;
confirmed examples currently have separate runtime replay tests.

Concrete generic arguments are substituted and normalized before execution. Static trait dispatch
resolves to a concrete implementation. Available dependency bodies, read-only closures and
function items are interpreted with actual values. Unsupported shims and missing MIR still fail
as unknown. Reports identify interpreted instances separately from trusted models. Compiler
assume intrinsics become validity obligations, so their predicates must be established on the
current path.

Coverage remains limited by enum/struct slices, general mutation and aliasing, mutable captures,
unresolved generic inputs, float remainder/bit observation, trait objects, function pointers and
general iterator machinery, destructors and several MIR operations/constants, including some
promoted constants. Non-byte arrays are limited to 16 elements; symbolic bounded indices work for
integers, floats and booleans; enum/struct elements need a uniquely determined index. Array/slice
patterns prove their minimum length and index bounds before applying constant start/end offsets.
Generic roots with unresolved type parameters remain unsupported. There are no inductive loop
invariants, automatic type invariants, dedicated termination checks or general effect contracts.

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
