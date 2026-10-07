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
