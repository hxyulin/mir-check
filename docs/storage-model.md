# Typed storage model

The storage model must distinguish a Rust allocation from an address and from a snapshot of a
value. Equal pointer bits, equal layout sizes and equal symbolic values do not establish shared
allocation identity. Compiler layouts establish where typed subobjects live; they do not establish
which bytes are initialized or whether another actor can change them.

## First migration stage: footprints and subobject certificates

The ordinary interpreter now checks static address extents through a common `StorageFootprint`.
It contains an allocation-relative offset, compiler-sized extent and required alignment. Its
constructor rejects overflowing extents and invalid alignments. An access fits only when its extent
is inside the allocation, the allocation has sufficient alignment and the offset is aligned.

A compiler-known initialized aggregate can also certify a reference to an actual subobject at
offset zero. Certification follows normalized struct and tuple field types and nonempty array
element types, requiring compiler-reported offset zero and a valid containing footprint at every
step. Nested prefixes work without inspecting names or inferring relationships from equal sizes.

An enum has no selected runtime variant in this address-only model. Unions, including MaybeUninit,
provide no evidence that their members are initialized, so certification cannot descend into them.
A certificate for a typed field may include that field's own padding; it never treats padding as
another initialized type or supplies readable values for its bytes. Existing dense integer atomic
overlays retain their separate requirement for an initialized atomic footprint without padding.

The certificate proves an address/type relationship. It does not load shared payloads, establish
atomic history or change initialization after a store. Static allocation identity, access
capability, epoch invalidation and retained reference-escape evidence continue to apply.

Compiler integration fixtures cover a nested initialized prefix, a reachable independent panic,
an unrelated equal-layout type, an uninitialized member and a member at a nonzero offset. Moving
the initialized prefix away from offset zero makes the proof UNKNOWN. Native replay checks only
the valid address relationship; it does not execute intentionally invalid reinterpretations.

## Checked access adapters

Tracked allocation references and static views now share a checked `StorageLocation` access layer.
Its identity distinguishes a tracked allocation slot from a static compiler definition. The static
validity epoch is evidence for that allocation's availability, not the allocation's identity: two
statics sharing one epoch remain distinct. Capability checks distinguish shared from writable
access independently of whether the allocation is live.

Tracked reads and projected writes use this layer before following their existing typed projection
paths. Static views use it before their existing compiler type and footprint certificates. Snapshot
and reference-graph checks share the same static epoch validation. A live static location supplies
an address relationship, never a retained readable payload. Writing shared static storage still
retains reference-escape evidence without gaining payload values or atomic history.

Provenance-backed local addresses retain their underlying tracked reference through snapshots and
reference graphs. A dead referent or a frame-owned address escaping its frame remains UNKNOWN. A
stored address keeps that reference as escape evidence. Numeric address equality cannot create an
allocation identity or authorize a write.

This stage leaves compiler types and byte footprints in static view descriptors and symbolic paths
in tracked reference descriptors. It does not attach invented compiler types to existing tracked
values, migrate initialization states, or replace every symbolic storage variant. The next stage
can add typed subobject initialization to this common access layer once construction and mutation
transitions preserve it.

## Common location representation

The next stage should replace parallel static and tracked-reference descriptors with a typed
location. The location identifies one allocation, a compiler type, a subobject path and byte
footprint, plus an access capability. The allocation identity must distinguish caller storage,
frame storage and statics. Allocation records retain liveness and initialization independently of
which references point to them.

Use compiler types and field indices for shape identity. Keep display names at the reporting
boundary. A numeric pointer handle remains a separate value and cannot become a typed location
through an integer cast. Thin provenance-backed raw addresses may retain a location through a
layout-certified cast, but an address cast does not itself certify a dereference.

Projection, borrow, load and store should consume the same location description. A borrow checks
capability, liveness and the necessary initialization state. A load checks that its entire typed
subobject is initialized and available. A store checks capability and type, updates the correct
subobject and retains escape evidence for every stored reference.

## Initialization and effects

Track initialization per typed subobject rather than a single allocation Boolean. The initial
states needed are uninitialized, initialized with a retained typed value, and initialized with an
opaque value. A MaybeUninit wrapper can exist while its payload remains uninitialized. A whole
payload store establishes payload initialization; a partial field store establishes only that
field. A move clears initialization only for the moved subobject.

Retaining a value requires a visibility policy. Owned storage with no possible external mutation
can retain exact values. Shared mutable statics retain at most initialization facts justified by
the environment; their initializer is not the runtime state of an arbitrary selected root.
An unknown call effect must invalidate retained value facts and any initialization facts it may
change, without dropping reference-escape evidence. Overwrites can release old evidence only when
the implementation proves that no copy or alias remains elsewhere.

Coroutine captures and saved locals should use these ordinary initialization rules. Constructing
a future initializes captures and its initial state, not all suspension-specific saved fields.
Polling a stored future additionally needs the relevant exclusive capability and the actual poll
MIR. This should replace special handling of uninitialized saved-field placeholders only after
their construction, suspension, move and drop transitions are covered.

## Atomic identity and interference

Atomic locations need a footprint in the same allocation namespace. The initial implementation
can retain exact history only for freshly owned local atomic storage whose aliases cannot escape
to an external actor. Constructor values, stores and read-modify-write operations then update that
location's typed state. Strong compare-exchange compares against its actual old value; weak
compare-exchange additionally permits spurious failure.

Do not determine exclusivity merely from a shared-reference type or sequential MIR execution.
Passing an atomic to a thread, interrupt registration, a trusted external boundary or unsupported
pointer operation may make its state interfere. Unmodeled escape must conservatively discard
exact history or return UNKNOWN before a proof can rely on it.

Different-sized atomic views can overlap. Exact histories cannot be independent when their
footprints overlap. Start by rejecting precise history for overlapping views, while retaining the
existing conservative atomic abstraction. A later implementation can share suitable symbolic
storage across supported overlaps; a general byte-level raw-pointer interpreter is unnecessary.
Fences check valid orderings but never establish absence of interference by themselves.

Fresh firmware startup requires an explicit verified environment model before static initializer
values become initial runtime atomic values. A root called after other activity has a different
domain. This distinction must remain visible in reports and counterexample validation.

## Completion checks

Each migration stage needs positive, refuted and UNKNOWN fixtures on host and ARM, plus mutations
that invalidate its dependent proof. Include aliases, disjoint subobjects, moves, frame escape,
unknown-effect invalidation, unions, padding, uninitialized payloads and overlapping atomic views.
Compare complete proof queries before and after representation-only migrations. A higher proof
count alone cannot establish that an initialization or interference rule is sound.

Native replay can independently check supported owned inputs. It cannot validate an arbitrary
shared atomic history by executing one ordinary call. Counterexamples whose outcomes depend on
interference need that limitation recorded rather than being presented as observed failures.

## Tracked raw addresses

An address-only tracked pointer retains a reference to its allocation alongside a symbolic thin
address. This is distinct from an integer-derived pointer handle. Whole initialized locals and
tracked reference reborrows can supply these pointers; value snapshots cannot establish provenance.
Same-place reborrows share an address term, ignoring const/mutable pointer spelling. Supported thin
casts retain the reference, while integer exposure supplies only address bits.

Creating a pointer does not authorize a load or store through it. Raw dereferences and arithmetic
remain UNKNOWN. Direct projected raw addresses need compiler layout offsets and remain unsupported;
this avoids assuming that a packed field is aligned merely because its type has alignment.
Snapshots retain pointer identity rather than copying its pointee as an owned value. Liveness and
frame-escape checks traverse the retained reference, including pointers nested in stored aggregates.

## Explicit fresh-startup histories

`--startup` chooses a conditional entry domain for a zero-argument root: Rust statics have their
declared initializer values and no external actor changes their atomics before a publication or
opaque boundary. This premise is retained in `entry_assumptions`, and successful roots report
PROVED_WITH_ASSUMPTIONS. Arbitrary-root analysis remains unchanged.

A memory-state container carries retained allocations and a separate bounded map of static atomic
histories through branches, calls and callbacks. A history key is the compiler static identity plus
its certified byte offset and extent. Initialization reads use rustc's checked scalar access into
the actual initializer allocation; uninitialized storage or pointer provenance cannot become an
integer value. Only compiler-certified integer atomic views receive this history.

Strong and weak compare-exchange use the same transitions as local atomics. Stores, swaps and
supported modular arithmetic update the selected history. Signed and unsigned views of the same
extent share bits. Different overlapping extents invalidate precision rather than establishing
independent histories. Separate statics and disjoint fields retain distinct histories.

Every supported typed static store and trusted call conservatively ends precise startup history,
even a trusted call declaring no tracked writes. The memory state remembers that invalidation so an
unseen static cannot later regain its initializer value. Missing bodies and unsupported operations
remain UNKNOWN. No readable general static payloads or concurrent execution model are introduced.

The current history budget is 128 locations per state. Startup roots with arguments and startup
histories in cyclic induction remain UNKNOWN until those entry domains and transitions are modeled.
Native fixtures check first and repeated claims, initializer mutations, callback updates, branch
histories and invalidation. Initialization of reset-time runtime machinery remains an explicit
entry premise.

## Typed intrinsic stores

Nonvolatile atomic stores use the same static type, extent, capability, liveness and reference
retention checks as ordinary typed stores. The destination must identify a writable certified Rust
static subobject, with the exact compiler integer or thin-pointer type expected by the intrinsic.
Supported orderings come from the compiler intrinsic's const parameter and the real core wrapper.
Pointer stores retain evidence for their referenced allocations, including nested frame escape.
Stores establish no readable shared payload or synchronization facts and end precise startup
history.

An UnsafeCell payload may contain an alignment wrapper around its actual scalar. A raw cast can
reach an initialized offset-zero subobject only through compiler field/layout certificates. The
UnsafeCell itself must retain its transparent layout. Unrelated same-sized types, union payloads,
uninitialized members and packed fields cannot acquire certification through that cast.

Numeric raw destinations, unmodeled pointer loads and volatile atomic operations remain UNKNOWN.
General raw-pointer loads and stores remain outside this model.

## Mutable static address views

Static reference descriptors retain whether the reference is shared or mutable. Forming a mutable
reference requires a live, certified, initialized place and writable capability. It does not prove
exclusive access, absence of interference or a readable payload. Supported stores remain opaque and
invalidate startup histories. Mutable array references preserve their typed descriptor; coercion to
a mutable slice remains UNKNOWN.

`UnsafeCell::get` and `raw_get` derive addresses from certified receivers. The explicit
uninitialized wrapper case retains its initialization limitation. A cast to an unrelated
`UnsafeCell` does not certify that wrapper or its payload, even if its size and alignment match the
original storage.

An enum discriminant is a payload read too. An initialized static address does not provide the
current variant. UNKNOWN reports name that type and point to runtime initialization and update
tracking, rather than suggesting a larger resource budget.

## Branch memory sharing

Cloned branch memories share an immutable allocation vector through host `Rc` storage. Reads keep
that vector shared; mutable indexing, iteration and Vec operations detach it with `Rc::make_mut`.
A detached vector preserves allocation indices, including dead slots, so reference identity does
not change. A write, retirement or appended allocation affects only its branch. This changes host
copying cost without granting any new Rust aliasing or initialization permission.

Startup atomic maps and their invalidation latch remain independently cloned. Invalidating one
branch cannot invalidate another branch or restore an initializer in an already invalidated branch.
The first mutable access still copies the entire vector; this is not persistent per-slot storage.

## Opaque initialized enum tags

An actual MIR discriminant read can observe a certified initialized static enum without reading
its payload. The current admission rule requires an exact enum place, a live epoch, its compiler
initialization certificate, and compiler-confirmed `Copy + Freeze`. It accepts at most 64 variants.
Every read creates a new symbolic tag constrained to rustc's declared discriminants, including
sparse and signed values. Allowing every declared variant is conservative even when some payloads
are uninhabited. The tag neither comes from the initializer nor persists across storage reads.

Saving that tag in an ordinary local preserves one observation. Reading the storage twice supplies
two independent observations, even under `--startup`. An exact typed enum store preserves legal
tag shape but supplies no retained value or noninterference fact. Pointer-bearing tags need no
pointer value or provenance because their payloads remain unreadable. Counterexamples involving
such observations carry an abstraction reason; a solver model alone is not a native panic replay.

The existing type certificate excludes MaybeUninit payloads and unrelated equal-size casts. Epoch
checks reject reads after unknown effects. Initialization validation precedes the compiler's
single-variant shortcut, preventing a constant tag from bypassing those checks. Whole static
moves and copies, static downcasts and payload fields remain unsupported. Static destructors remain
UNKNOWN. A copyable tag field inside a noncopy parent cannot obtain readable initialization evidence
after dropping that parent.

These observations increase coverage without retaining mutable static payloads. Exact histories
still need explicit initialization transitions and an independently justified visibility policy.

## Bounded static retirement transitions

A compiler-confirmed no-destructor drop of a writable, initialized, nonempty static subobject adds
a branch-local retirement marker. Its shape is an interned static descriptor owned by the engine;
comparisons use compiler allocation identity, normalized type and offset rather than descriptor
number. Initialization-consuming operations reject overlapping retired subobjects, including
single-variant tags, borrows, old references, stored references, slices and atomic receivers.
Static atomic constants preserve their descriptors so a fresh reference cannot lose that identity.
Zero-sized accesses cannot establish disjointness by byte offsets alone.

An ordinary typed store validates its value and retains reference-escape evidence before clearing
an exact matching retirement. Writes to a field of a retired parent do not restore the parent.
Stores to a containing object do not clear unmatched child retirements in this bounded model.
Raw addresses remain available to resolve a supported reinitialization destination; initialized
borrowing is a separate operation. Retired array element destinations remain unsupported where
projection requires constructing an initialized array view.

Retirement invalidates precise startup atomic history. Reinitialization supplies neither payload
values nor noninterference facts and cannot revive an epoch lost to unknown effects. Pending
retirements have a 128-subobject limit. Static destructors, static payload moves and zero-sized
retirements remain UNKNOWN until their typed initialization effects are implemented. This stage
tracks missing static initialization, not general partial initialization or owned move semantics.
