# Analyzer redesign and completion criteria

The analyzer needs common representations for storage, calls and queries before broader models can
reliably share facts. Native counterexample execution supplies independent evidence about translated
failures. It does not replace a proof or make a nonreproduced input safe for every execution.

This design keeps typed rustc MIR, interned SMT terms and Z3 over its text interface. It does not
introduce another solver, unchecked callee summaries or a general byte-level raw-pointer
interpreter.
Each migration must preserve UNKNOWN wherever the new representation lacks required semantics.

## Implementation status

The current stages establish reusable parts without claiming the whole redesign is complete.

| Area | Implemented foundation | Remaining implementation |
| --- | --- | --- |
| Typed locations | Static footprints, initialized-prefix certificates, checked location access and tracked raw addresses | Common typed projections/footprints, subobject initialization and retained static payloads |
| Atomic state | Exact local histories and explicit fresh-startup static histories with conservative invalidation | Verified startup environments, precise overlaps and supported shared interference |
| Aggregate sharing | Interned SMT leaves, lazy input shapes and shared owned ADT fields | Remaining aggregate kinds, branch snapshots and compiler-keyed aggregate shapes |
| Call metadata | Root-local normalized signature cache shared by ordinary and induction models | Common compiler-identified operation descriptions and backend-independent transition semantics |
| Query construction | Dependency reuse for immutable SMT terms | Measured structural query assembly reuse with complete context keys |
| Counterexamples | Explicit native replay for a supported input subset and structured validation outcomes | Additional owned input shapes, source-faithful dependency validation and controlled Miri replay |
| Report UX | Source locations, interpreted call chains, relevant abstraction symbols and replay evidence | Path decisions and additional abstraction sources |

Detailed storage rules are in [the storage model](storage-model.md). Replay boundaries and commands
are in [the usage guide](usage.md#execute-a-counterexample).

## Typed locations and initialization

A location identifies a Rust allocation and a compiler-known subobject within it. Its identity is
independent of numeric pointer bits, SMT value equality and host representation sharing. It carries
an allocation-relative footprint, compiler type, projection path and access capability. Allocation
records retain liveness, initialization and possible interference.

Projection, borrow, load, store, move and drop must consume the same location description. Shared,
exclusive and interior-mutable capabilities permit different operations. Layout certification can
justify an address relationship but cannot establish initialization, exclusivity or a readable
payload. Numeric pointer handles remain separate from provenance-backed locations.

Initialization is tracked per typed subobject. The required states are uninitialized, initialized
with a retained value and initialized with an opaque value. A whole typed store establishes the
payload; a partial store establishes only its addressed field. Moves clear only the moved subobject.
An unknown effect invalidates facts it can change while retaining reference-escape evidence.

MaybeUninit, unions and coroutine saved locals should use these rules. A wrapper or future can be
initialized while a contained payload or saved local is uninitialized. General reads remain UNKNOWN
until the correct location has an initialization fact. Polling a stored future additionally requires
exclusive access and actual available poll MIR; construction does not establish poll safety.

Start by migrating read/write/borrow projection checks without retaining additional values. Compare
complete obligation queries for representation-only changes. Add initialization transitions next,
then payload retention where the visibility policy justifies it. Type invariants follow complete
construction and mutation hooks, including field writes, moves and callee effects.

### Admission rules for the next initialization stage

Intern compiler-backed type and projection shapes in the engine; memory records keep lifetime-free
shape IDs, liveness and bounded subobject initialization. A typed location needs its projection path
as well as its footprint: zero-sized fields and enum variants cannot be identified by offset and
size alone. Invalidated records need a generation or tombstone so a later read cannot resurrect an
initializer.

Initialization and history permission are separate. A valid typed write can establish initialization
without supplying a retained runtime value. Exact mutable static history additionally needs verified
exclusive access or a separately authorized quiescent region. The current startup condition protects
static atomics; it does not authorize retained non-atomic `UnsafeCell` payloads. A mutable reference
or writable capability supplies no interference argument.

Before retaining additional payloads, split effectful moves from copies and give projected drops
explicit initialization transitions, or decline them. A move retires the selected subobject. Drop
keeps its value available to supported destructor MIR, then retires it. Whole and field stores
update
only their addressed initialization; changing an enum variant invalidates its former fields.
Reference-escape evidence survives loss of retained values.

Shape validation is insufficient for restricted types. Retained `NonNull` values need nonzero
address validity evidence, and enums need a valid discriminant and initialized selected payload.
Unsupported union members, conflicting overlaps and unmodeled effects remain UNKNOWN.

A separate coverage stage can model a fresh arbitrary legal discriminant from initialized opaque
static storage, without constructing pointer payloads or assuming the initializer's variant. It must
report the abstraction and preserve no relation across separate storage reads. Such a model may
produce conservative counterexamples; exact startup safety still needs an interference permission.
This snapshot stage and mutable static retention are not implemented yet.

## Atomic histories and interference

An atomic operation uses the same allocation identity and footprint as other typed storage.
Supported fresh owned local integer atomics can retain constructor values and sequential updates
while the analyzer can account for their accesses. Strong compare-exchange relates success to the
actual old value. Weak compare-exchange additionally permits spurious failure.

Shared references and sequential MIR do not establish absence of interference. An unresolved call,
trusted external effect, concurrency boundary or unsupported escape must invalidate precise history
or return UNKNOWN before a dependent proof succeeds. Fence calls validate orderings without
establishing exclusivity or a happens-before relation.

Different views with overlapping footprints cannot keep independent precise histories. The first
extension should decline exact history for these overlaps. Sharing symbolic storage across suitable
supported views is a later step, with explicit initialization and atomic access rules.

Explicit `--startup` now supplies a conditional initializer domain with branch-local static atomic
histories, reported entry assumptions and conservative invalidation. It does not verify the entry
environment or change default arbitrary-root analysis.

Static initializer values are not the runtime atomic state of an arbitrary root. Firmware startup
precision requires a verified execution environment and checks on task, interrupt and hardware
registration. Report any environment assumptions separately. Replaying one ordinary native call
cannot validate an arbitrary interfering atomic history.

## Shared aggregates and branch states

Owned ADT fields now use shared host storage with copy-on-write. Cloning a struct, enum payload,
closure environment or coroutine value retains its field-vector handle. A field write detaches
shared vectors along the changed path, keeping unaffected nested ADT vectors shared. Consuming
shared fields preserves the other owners. Allocation handles and reference validation retain
their existing semantics; host pointer identity is never a modeled Rust address.

Branch allocation vectors now share host storage until mutable access, with allocation identities
and retirement kept branch-local. Startup maps remain independently cloned. Tuple fields, array
elements and enum variant tables still clone their containers, and ADT display names remain owned
strings. Later stages can share those representations and compiler-keyed immutable shapes, with
names at reporting boundaries.

Host sharing must not create Rust aliasing. Copying an owned Rust value produces a separate logical
value; taking an address still identifies its particular allocation. Repeated array elements,
closure environments and independently owned coroutine captures must retain that distinction.
Return continuations copy caller locals and address mappings, then take the returned conditions and
memory directly. They avoid cloning caller memory and startup histories that would be immediately
discarded.

Branches may share their entry representation but must not share later mutations. Contract entry
snapshots remain independent of later writes, including nested referenced storage.

Snapshot and return-value traversal remain recursive: sharing an ADT is not evidence that its
references are live or that its entry values match post-state values. Independent fixtures cover
branch divergence, nested mutation, owned repeats, callback environments, entry snapshots and
frame escape. Measure container copies and compare complete proof results and queries before
expanding the representation to all aggregate kinds.

## Compiler call descriptions and common semantics

A call description must be keyed by the complete concrete compiler Instance, including type and
const arguments and the compiler adapter identity. A display path or item name alone is
insufficient.
Signatures retain their binders when checking function-pointer types. Generic instantiations and
same-named unrelated user items must remain distinct.

The signature cache memoizes successful normalized compiler signatures for up to 256 instances per
root. Calls beyond this bound normalize without caching; the optimization never rejects a supported
call. Ordinary library and iterator models and induction range/slice models use this common lookup.
Function-item coercion retains the source item's binder and separately checks its resolved target.
This cache contains compiler metadata, not proof outcomes or callee summaries.

The next migration classifies a concrete operation once using compiler language items, diagnostic
items, trait identity and normalized signature checks. Classification carries the supported domain
and memory effects rather than choosing success on a matching name. Configured contracts keep their
current precedence, and every actual call checks its preconditions and executes its body or the
existing explicitly trusted boundary. Cached metadata cannot bypass either step.

Separate operation semantics from control-flow encoding. Ordinary execution instantiates a checked
transition; induction emits the same transition relation into its Horn encoding. The encoders retain
separate state and control-flow machinery. Share small integer range and slice cursor transitions
first, with differential fixtures exercising both backends. Unsupported shims, dynamic targets,
missing bodies and unsupported induction values remain UNKNOWN.

## Query dependencies and construction

SMT terms are immutable interned DAGs. Their symbol and floating-point encoding dependencies can be
memoized with the term identity in its owning context. Cache sorted dependency sets and reuse them
across obligations without skipping sort validation or latent encoding collection.

A later query assembly cache must include the structural premises and obligation, every relevant
encoding, symbol declarations and solver configuration. It must not rely on a textual display name
or reuse identities across term contexts. An exact decision cache remains separate. Resource limits,
invalid queries and solver failures keep their existing semantics.

Retain complete standalone SMT scripts in reports even when a cached decision avoids contacting Z3.
Measure dependency walks, rendering time, solver time, allocation volume and query bytes separately.
A faster synthetic query builder is not evidence of faster whole-firmware verification.

## Native counterexample validation

On explicit replay, decode supported concrete root inputs from the retained SAT model, generate a
small caller for the compiled function, then execute it with bounded compilation and execution time.
Only replay panic-safety counterexamples initially. Contract failures need their own validation
procedure because metadata adds no runtime checks.

The initial input domain is integers, Boolean values, unit and supported scalar arrays of at most
128 elements. Symbolic byte-array encodings and floating-point inputs remain unsupported. Accessible
nongeneric free functions on the analyzed native target are eligible. References, shared state,
hardware effects, unknown function pointers and cross-target execution need additional models or
execution environments. Source changes, missing model assignments and unavailable artifacts must
produce an explicit unsupported outcome rather than invented inputs or a success claim.

Native replay preserves the analyzed panic strategy, including code selected by cfg(panic).
A native panic hook records entry; abort replay then exits, while unwind replay catches it.
Preserve relevant target, optimization and overflow settings where supported. Source fingerprints
cover the retained primary Rust sources; dependency artifacts must remain available, and included
data and dependency source are not independently fingerprinted. A compiler error,
timeout or incomplete execution is a tool failure. A normal return means the specific execution did
not reproduce the failure; it does not downgrade the symbolic result to PROVED. A panic at another
site confirms a failing root execution but does not confirm the original obligation.

Report the concrete inputs, panic message and location, whether the location matches the obligation,
and validation status separately from PROVED/REFUTED/UNKNOWN. Store structured evidence in JSON and
JSONL. Do not silently execute analyzed hardware-facing code as part of an ordinary scan.

Replay results also retain the obligation's relevant modeled choices as `uncontrolled_abstractions`.
Concrete root inputs do not impose an interfering shared atomic history, a weak compare-exchange
outcome or an arithmetic NaN encoding. Report this limitation for both observed native panics and
normal returns. The list describes choices present in the query, not a causal explanation or an
inference that the root is safe. Ordinary reports point to execution-environment validation even
when the target cannot be replayed on the host.

Additional input shapes should follow typed replay recipes: tuples, owned structs, enums and owned
byte buffers before borrowed inputs. Preserve compiler layout and valid enum variants. A controlled
Miri hook can independently investigate undefined behavior on supported native fixtures, but Miri
remains an execution validator rather than the analyzer's backend. Nondeterministic or interference
counterexamples require a trace or environment replay, not a single synthesized function call.

## Explain the failure after a scan

Present the root result with the first actionable failure: source location, operation, message,
interpreted call chain and decoded root inputs where available. Distinguish a solver assignment from
an observed native panic. Keep resource-limit and unsupported-operation reasons visible when a root
has both REFUTED and UNKNOWN obligations.

Failure reports identify supported abstraction symbols present in the complete obligation query,
including latent float encoding dependencies. These include arbitrary shared atomic reads, weak
compare-exchange choices and conservative arithmetic NaN encodings. They describe choices in the
query, not which choice caused a runtime failure. Unused choices do not appear in that explanation.
A later stage should retain named branch decisions and additional abstraction sources, including
numeric raw pointers with no provenance.

Keep compact terminal output readable and retain full details in verbose reports. Use explicit
words with color as a secondary cue. Progress updates identify the active root and elapsed time.
Saved report rendering should explain the same evidence without rerunning the compiler or solver.

## Ordered migration and acceptance

1. Establish reusable footprints, projection walks, signature lookup and dependency caches. Record
   baseline results, queries and timings before further representation changes.
2. Implement precise histories for fresh local atomic locations with escape and effect invalidation.
   Keep arbitrary shared storage conservative and test overlap boundaries independently.
3. Add explicit native replay and actionable failure output for the supported owned input domain.
   Validate confirmed panic, nonreproduction, unsupported inputs and tool failures.
4. Migrate typed initialization and capabilities into one location namespace. Add retained payloads
   only after reads, moves, stores, drop and unknown-effect invalidation agree on that
   representation.
5. Share aggregate shapes and values across branches, then share compiler-identified call
   transitions with induction. Use query equivalence and native mutation tests throughout each
   migration.
6. Extend replay shapes and traces, verify type-wide invariants at all mutation hooks, and
   separately evaluate static startup environments and structural query assembly reuse.

Every behavior stage needs host and ARM positive, refuted and UNKNOWN cases, with mutations that
break dependent proofs. Representation-only changes need identical result and query evidence.
Performance claims require representative repeated measurements with fixed compiler, target,
profile, solver and limits. Measure one full Rust build at a time. Complete formatting, lint,
tests, release builds, dependency checks and manual hooks before committing each finished stage.

Verified callee summaries remain a separate user decision. They require a proved contract, complete
frame/modifies information and artifact invalidation tied to compiler, target, build options,
dependencies and configuration. The current design continues executing ordinary callee bodies.
