# Proof execution and boundaries

The current engine uses path-sensitive symbolic execution and SMT solving, not an interval
analysis or a general verifier for arbitrary Rust. The compiler adapter reads typed runtime MIR
from the pinned rustc with mir-opt-level=0. MIR supplies explicit types, branches, assertions and
calls; rustc does not supply the proof itself.

## Inputs and paths

Each selected root gets symbolic inputs. Integers are bit-vectors with the target's exact widths
and signedness. Booleans are SMT booleans. Byte contents are SMT arrays; slice lengths satisfy
valid-reference bounds. Struct fields are independent inputs, including private fields. No
constructor invariant is inferred for an arbitrary struct parameter.

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
explicit core models, and Z3. Bit-vector and array semantics preserve the supported integer and
byte operations, but the translator has not been formally verified. Mutation tests and runtime
replays check representative semantics; they do not establish correctness of the analyzer.

Trusted models implement slice length, byte prefix ranges, u8-to-usize conversion, exact copies
into owned local byte arrays, and opaque formatting arguments from evaluated static strings.
Compiler identities and instantiated types select them. Arbitrary dependencies and dynamic
formatters are not assumed safe. A solver model is not automatically replayed as a Rust test;
confirmed examples currently have separate runtime replay tests.

Coverage remains limited by missing generic substitutions, cross-crate body analysis, arbitrary
input enums or enum/struct slices, general mutation and aliasing, floats, closures, iterators,
destructors and several MIR operations/constants. Constructed non-byte arrays are limited to
16 elements and uniquely determined indices. There are no inductive loop invariants, automatic
type invariants, dedicated termination checks or general effect contracts.

A selected-root result is conditional on its recorded preconditions and build configuration. It
does not verify unselected callers, every workspace member, another compiler's binary, undefined
behavior, allocation failure, stack exhaustion, interrupt interactions or hardware timing.

Background references:

- [Rust MIR guide](https://rustc-dev-guide.rust-lang.org/mir/index.html)
- [Z3 bit-vectors](https://microsoft.github.io/z3guide/docs/theories/Bitvectors/)
