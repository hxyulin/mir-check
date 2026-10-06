# Stages

The original four-step outline was:

1. Compiler adapter: pinned rustc_driver integration, typed local MIR bodies, source locations,
   Cargo integration, JSON reports and a metadata-only contract crate.
2. Panic inventory: enumerate checks and calls, classify unresolved boundaries, and show
   conservative local call paths. Inventory results do not establish feasible execution paths.
3. Proof engine: branch-sensitive abstract interpretation for integer ranges, enum variants and
   slice lengths. Add selective symbolic reasoning, exact arithmetic semantics and loop invariants.
4. Verified contracts and effects: check preconditions at callers, postconditions at returns and
   no_panic claims; add preserved type invariants, dependency summaries and restricted effects.

Each completed stage has its own commit. An unsupported operation, absent dependency body or
analysis timeout remains unknown. An annotation never establishes its own truth. Candidate
counterexamples from approximate models require validation before being reported as confirmed.

The starting examples are guarded byte reads, constant index helpers, frame length invariants,
invalid clamp bounds and unresolved dynamic dispatch. Resource and application-state properties
can follow once the foundational semantics are tested.

## Stage 1 evidence

Completed on macOS aarch64 with the pinned compiler. Four compiler integration tests and one
metadata behavior test pass. The tests cover uncalled generics, compiler errors, metadata
collection, Cargo feature forwarding and repeat analysis. Removing an uncalled generic from the
inventory makes its integration test fail; the mutation was restored.

Formatting, warnings-denied Clippy, release builds, dependency checks and manual hooks pass.

## Stage 2 evidence

The inventory detects bounds, arithmetic and explicit panic sites with panic=abort. Tests cover
optional overflow settings, signed division overflow, unknown external/trait/indirect calls,
destructors, guarded accesses, local call paths and recursion. Sites stay unverified and unknown
entry selection fails. A user function named panic_fmt remains an ordinary local call.

The existing CAN frame source was analyzed read-only on aarch64-apple-darwin and
thumbv7em-none-eabihf, with aborting panics and overflow checks enabled. Both reports contained
18 bodies and 33 sites, including five panic language-item calls. These are inventory counts,
not findings that the guarded operations can panic or proofs that they cannot.

The metadata crate also passes its behavior test under stable Rust 1.98.1. It does not evaluate
even unresolved predicate names or false preconditions at runtime. Verification remains future
work; unsupported contracts do not become trusted assumptions.

Removing bounds checks from the inventory makes the abort-mode integration test fail; the
mutation was restored. Formatting, warnings-denied Clippy, release builds, dependency checks
and manual hooks pass. Compiler and metadata integration tests also cover malformed contracts
and report-write failures.

## Stage 3 evidence

The first proof engine uses exact path-sensitive symbolic execution and Z3 bit-vectors before
adding abstract interpretation. It proves guarded byte access, checked integer guards and safe
local calls. Off-by-one guards, overflow, division failures, invalid call arguments and stale
guards are refuted with solver models. Loops and missing solvers fail as unknown. Compiler MIR
lint errors cannot emit a success report.

Independent host checks exhaust all 65,536 u8 argument pairs for guarded addition and replay
five rejected panic cases. This validates representative translations; it is not a proof of
the analyzer implementation. Contract predicates and function-level preconditions remain future
work at this stage. The supported subset and resource limits are explicit in README.md.

## Stage 4 evidence

A restricted pure predicate evaluator checks entry domains, every reachable local call's bounds
and postconditions at feasible returns. Callee bodies and return values are still analyzed,
including when annotations claim no panic or a false return bound. Postcondition parameter
bindings snapshot entry values. Unsupported names, expressions, type/range errors and
inconsistent preconditions fail as unknown.

A guarded call to an identity function requiring value < 16 passes. The same caller with a
value <= 16 guard fails with value=16, although the identity body cannot panic. Removing caller
precondition verification makes that regression test fail; the mutation was restored. Changing
MIR's <= translation to < also makes the panic regression test fail; it was restored.

The proof suite passes on the 64-bit host and thumbv7em-none-eabihf's 32-bit usize. Aborting and
unwinding panic modes and enabled/disabled overflow checks are tested. Cargo verification is
covered with both passing contracts and a failed call bound that retains its JSON report. The
example crate demonstrates guarded byte reads and a bounded increment without runtime checks.

This completes the initial contract slice of stage 4. Loop invariants, abstract interpretation,
generic substitutions, dependency summaries, preserved type invariants and effect contracts
remain future work. The analyzer implementation itself has not been formally verified.

## Stage 5: real-code fixture

Frame and FdFrame are vendored from fleet-2027 at b81a247a3295b13553087f5e328f201944a1eb61.
The original method bodies are preserved and compared against an unmodified source excerpt.
The fixture adds constructor and accessor contracts without changing the original workspace.
Runtime checks exercise valid and invalid IDs and all payload lengths from zero through 65.

## Stage 6: real-code proofs

All six unchanged Frame/FdFrame method bodies now prove on aarch64-apple-darwin and
thumbv7em-none-eabihf, with panic=abort and overflow checks enabled. Constructors establish ID,
accepted-length and exact stored-length postconditions without entry assumptions. Data accessors
prove safe slicing and returned lengths under explicit capacity preconditions; private fields do
not imply an invariant for arbitrary struct inputs.

Two payload harnesses prove the constructor-to-accessor path, each callee's required bound and
exact payload equality at every permitted index. Actual constructor bodies supply caller facts;
annotations are not trusted summaries. Struct fields, constructed core Option variants and local
byte arrays extend the modeled MIR subset. Pure contracts gain named fields, integer casts and
restricted exhaustive Option matches.

Pinned core models cover byte prefix ranges, slice length, u8-to-usize conversion and exact copies
into local arrays. Their range and copy-length panic conditions are checked. They are trusted
parts of the translator and are listed per root in JSON schema version 5. Mutable borrows cannot
cross local calls or escape through aggregates/returns. General mutation, input enums, nested
struct inputs, arbitrary dependencies, loops and derived methods remain unsupported.

Regression tests reject invalid stored lengths at accessor calls, weakened constructor guards,
invalid FD lengths and incorrect or missing payload copies. Removing an accessor precondition
also fails. A user method named copy_from_slice receives no trusted model, and mutable call
boundaries remain unknown. Temporarily disabling the translator's destination-byte updates makes
the real-code round-trip test fail as refuted; the mutation was restored.

The compiler suite has 31 passing tests, plus the metadata behavior test and independent vendored
runtime checks covering IDs, lengths zero through 65 and every accepted payload index. Formatting,
warnings-denied Clippy, release builds, dependency checks and manual hooks pass. These selected-root
results are not whole-crate coverage or a formal verification of the translator.

## Stage 7: larger bus validator

The vendored Use enum, both helper methods and check retain their original bodies. The validator
has 44 MIR blocks in the recorded build, including nested loops, enum matches, ID and slot checks,
pairwise collisions and FD compatibility. Two three-device configuration families prove with
symbolic IDs and slots on aarch64-apple-darwin and thumbv7em-none-eabihf. Five invalid families are
refuted with solver models and replay as runtime panics in the original validator.

The engine now follows constructed local enums and small constructed non-byte arrays. Indexing
requires a uniquely determined index on the current path. Loops are completely unrolled within
the existing execution budget; arbitrary input slices and larger loop domains remain unknown.
The arbitrary &[Use] validator entry itself is unknown, rather than a whole-domain proof.

Negative assertion paths exposed formatting setup before panic_fmt. A narrow compiler/type-checked
model constructs opaque formatting arguments from evaluated static strings. Dynamic formatting
and similarly named user methods remain unmodeled. Finite-loop tests check exact return values and
panics after later iterations. Infinite or over-budget loops fail as unknown. Temporarily treating
the execution limit as successful completion makes the cutoff regression fail; it was restored.

There are 37 compiler integration tests, one metadata behavior test and seven vendored runtime
tests. Formatting, warnings-denied Clippy, host and ARM release builds, dependency checks and manual
hooks pass. docs/proofs.md records the mechanism, trusted components and remaining coverage gaps.

## Stage 8: broader MIR coverage and DR16 parsing

Coverage takes priority over a broader soundness audit in this stage. Concrete generic arguments
are substituted and normalized; static traits resolve to implementation bodies. Available
dependency MIR, read-only closures and function items are executed with actual values. Reports
in JSON schema version 6 list interpreted body instances separately from explicit library models.
Missing MIR, unsupported shims and mutable captures remain unknown.

Constructed Result and ControlFlow variants support core question-mark propagation. Small
integer/bool arrays accept symbolic bounded indices and fixed-array map executes callable bodies
in order. Integer shifts, boolean casts, lossless integer conversion, endian decoding and exact
shared byte-slice-to-array conversion extend the supported operations. Compiler assume intrinsics
are checked as validity obligations rather than silently added as assumptions.

The unchanged Raw::parse body is vendored from the same firmware snapshot in examples/dr16.
Its 42-block body proves panic freedom without entry preconditions on the host and ARM target.
Postconditions establish exact-length rejection, switch bounds and all five channel bounds.
The proof follows core Result/Option methods and three closures. Incorrect byte-index and channel
mask mutations are refuted. Independent runtime tests compare 4,608 frames against separate
decoding formulas and check input lengths zero through 40.

There are 44 compiler integration tests, one metadata behavior test, seven CAN runtime tests and
two DR16 runtime tests. New cases cover generic call bounds, static dispatch, function items,
read-only closures, scalar arrays, endian values, Result success/error propagation and available
dependency bodies, including a refuted dependency overflow. Formal interpreter/model auditing,
general mutation, arbitrary enum inputs, floating point and broader iterator support remain work
for later stages.

Formatting, warnings-denied Clippy, host tests, host and ARM fixture release builds, dependency
checks and manual hooks pass. A separate ARM parser analysis discharges 45 obligations in about
1.5 seconds on the development machine; this includes compiler/solver execution and excludes
building the checker and proc macro.

## Stage 9: selected Cargo roots, coverage reports and aggregate inputs

Cargo accepts exact or crate-qualified entry names, including repeated selections across
workspace members. Crates without matching roots retain inventories rather than attempting
unrelated proofs. Missing names fail at the wrapper after collected reports have been retained.
Failed verification also renders available reports, so summaries expose failures immediately.

JSON schema version 7 includes per-crate root outcome counts, unselected bodies, distinct
interpreted instance counts and unknown reasons grouped by affected root. --summary displays those
results without the full site/obligation listing. Counts explicitly describe roots, not runtime
coverage or whole-crate safety. Workspace tests cover qualified selection, same-named unknown
roots, repeated selections, refuted roots and missing names.

Input modeling adds tuples, nested local structs, concrete generic fields, supported shared
reference fields and small fixed arrays of modeled values. Numeric contract projections preserve
tuple facts. An eight-level depth limit and 128-value budget prevent recursive/oversized input
construction from hanging; exhaustion and mutable fields remain unknown. Positive aggregate
cases and a refuted off-by-one call guard run on the host and ARM target.

The no_std contract example adds guarded_packet_read with a nested Header and shared byte slice.
The root README now leads with a runnable Cargo parser proof. docs/usage.md explains selectors,
targets, outcomes and schema fields; docs/coverage.md records supported behavior and evidence.

There are 49 compiler integration tests and one metadata test. Cargo tests run the real parser
and nested packet example on host/ARM, with one independently selected root per report. Tests
also preserve the inventory-only distinction and ensure input-budget exhaustion cannot pass.

Formatting, warnings-denied Clippy, host tests, release builds, dependency checks and manual hooks
pass. The documented whole-contract, selected DR16 and selected CAN Cargo commands also prove;
the latter two run for ARM, including an explicitly aborting/overflow-checked parser build.

## Stage 10: fleet measurement and published documentation

The unchanged fleet-2027 workspaces were inventoried and their 32 ARM compilation units checked
independently. The [survey](fleet-survey.md) records compiler/profile scope, generated-root skew,
counterexample interpretation and measured time. The actual DR16 parser proves without entry
preconditions. Whole firmware applications remain outside the current supported subset.

The README adds a theme-aware mark, status badges and direct guide links. VitePress publishes the
existing guides with local search, sidebar navigation and light/dark themes. A dedicated Pages
workflow builds pull requests and deploys main. Node dependencies are locked; the patched Vite
override leaves the dependency audit clear. Contribution and issue templates request minimal
examples, build configuration and evidence without treating solver assignments as runtime bugs.

Validation includes the Rust formatting/lint/test/release/dependency checks, manual file hooks,
the documentation production build and browser checks of page links, search and desktop/mobile
themes. No firmware source or dependency was changed. See [development](development.md) for the
documentation build and publication procedure.

## Stage 11: floating point, enum domains and dependency-defined inputs

The engine models f32/f64 with SMT floating-point sorts, nearest-even arithmetic and Rust's
saturating integer casts. Comparisons preserve NaN and signed-zero behavior. Compiler-identified
absolute value and min/max models preserve numeric NaN fallback and allow either equal operand.
Float remainder and raw bit observation stay unknown. Contracts accept typed finite float
literals, float casts and float-dependent call bounds.

Root enums receive legal symbolic discriminants and per-variant payloads; downcasts must prove
the active tag. Explicit signed discriminants are preserved. Local and dependency-defined
structs/enums use the same recursive shape limits. Symbolic Option contract matches have boolean
arms, and postconditions preserve entry snapshots. Unsupported payloads never become successes.
Array/slice pattern projections check minimum lengths and start/end offsets.

Four new compiler integration tests cover positive, refuted and unknown cases on host and ARM,
including foreign no_std inputs, NaN/zero/rounding/cast edge cases and off-by-one call guards.
There are 53 compiler integration tests and one metadata test. The fleet survey reruns the same
32 ARM units to measure newly supported paths without modifying firmware.

## Stage 12: compare fleet coverage and classify remaining blockers

The expanded fleet survey proves 149 of the same 976 function-declaration roots, up from 25.
All previous proved/refuted roots retain their outcomes; 124 unknowns now prove and 21 refute.
The baseline data is preserved, and a separate JSON summary records the new per-unit counts,
changed roots and first reported unknown reasons. Source paths in the published data are relative
to the firmware repository.

The largest remaining groups are mutable inputs, missing dependency MIR and constants. Removing
input blockers exposes these later boundaries, so their counts can grow without regressions.
The docs distinguish available foreign types from available foreign bodies and explain why
independently proving a helper does not automatically summarize it at an unknown call boundary.
Verification of the 32 already-built ARM units took 35.4 seconds, with more paths executed than
the 11-second baseline. No firmware source, configuration or dependencies changed.

## Stage 13: retain dependency MIR in Cargo analysis

Cargo analysis now retains ordinary non-inline dependency bodies by default. The small
mir-check-rustc outer wrapper appends always-encode-mir and MIR optimization level zero, forwarding
to the workspace analyzer or the pinned compiler. It preserves Cargo configuration, profiles,
features and Rust flags, including encoded arguments containing spaces. The existing fresh target
directory prevents stale metadata from earlier builds. --no-dependency-mir provides a comparison
mode. Prebuilt sysroot libraries are not rebuilt.

Dependency crates are not added to independent root inventories. Their concrete bodies execute
when called, with checked preconditions, actual returns and existing unsupported-operation limits.
A host/ARM regression uses two ordinary no_std dependency crates in a chain. Retention proves a
guarded call, refutes a violated dependency precondition and an overflow in the transitive body,
and preserves an unknown result for unsupported float bit observation. Opt-out calls remain
unknown because their bodies are absent. Tests also preserve configuration and encoded flags.
There are 54 compiler integration tests and one metadata test.

## Stage 14: measure fleet with retained dependency bodies

The same 32 ARM units retain all 149 previously proved and all 46 previously refuted function
outcomes. Dependency retention adds 47 proofs and 25 refutations, leaving 196 proved, 71 refuted
and 709 unknown function-declaration roots. Missing dependency MIR drops from 237 to 16 first
reported blockers. The remainder consists of prebuilt core helpers and a foreign critical-section
function; no missing body is assumed safe.

New passes include engineer arm/head controller constructors, robot PID configuration helpers
and CAN bus constructors/readers. Newly reachable constant and pointer/mutation gaps become more
visible. Independent mutable controller updates remain unsupported. The new survey data preserves
the original/intermediate measurements and records transitions, first-gap reasons and fresh build
costs. Verification of the already-built units takes 63.8 seconds on the development machine.

## Stage 15: inspect initialized aggregate constants

The constant decoder uses rustc_const_eval's inspection context for compiler layouts, active
discriminants, field/index projections and initialized scalar reads. It supports structs, tuples,
active enum fields, bounded arrays/slices and immutable promoted/static references. References to
interior mutable storage, mutable global reads, raw pointers and unions/MaybeUninit remain
unknown. Constants have eight recursive levels and a 256-value budget; byte arrays/slices have
at most 128 bytes and other arrays/slices at most 16 elements. No runtime calls are executed by
the constant inspection context.

Host/ARM integration tests prove guarded Option::as_ref calls, niche variants, explicit signed
tags, nested fields and immutable constants. Off-by-one guards, wrong payload assertions and a
mutated constant index refute. Inactive unsupported payloads need no read; active unions,
interior mutation, unsupported transmutes and shape exhaustion stay unknown. There are 55
compiler integration tests and one metadata test.

## Stage 16: measure fleet after constant decoding

The same 32 ARM units preserve all 196 proved and 71 refuted function outcomes. Constant support
adds 97 proofs and three bounds refutations, leaving 293 proved, 74 refuted and 609 unknown
function-declaration roots. All 54 Option::as_ref first blockers disappear. First constant gaps
fall from 194 to 69; 51 of those remaining reject interior mutable storage, often validation
counters. Mutable inputs still account for 300 first blockers.

New proofs include motor feedback accessors, link decoding, balance-state readers, engineer
commands and board controller/hardware helpers. The three new refutations concern engineer joint
indices under unconstrained root domains. Original and intermediate measurements are preserved;
the new JSON records transitions and remaining gaps. Verification reuses dependency-retaining
metadata and takes 81.3 seconds on the development machine. No firmware source, configuration or
dependencies changed.

## Stage 17: typed storage and mutable writes

References to supported mutable storage carry allocation identities and projections. Per-path
storage crosses calls and preserves writes through reborrows. Entry snapshots remain available in
postconditions, with `final_<parameter>` exposing updated arguments. Root analysis accepts one
mutable reference whose pointee contains no references. General aliases, mutable returns/captures,
partially initialized aggregates and ambiguous non-byte writes remain unknown.

Host/ARM tests cover scalar/aggregate/byte writes, callee updates, branch isolation and post-state
contracts. The vendored PID bodies prove reset and a concrete configured update; invalid limits
and a reset-write mutation refute. Arbitrary floating-point update paths remain expensive.
A root budget of 30 seconds is checked before queries, and storage has at most 512 allocations
per path. There are 57 compiler integration tests and one metadata test.

## Stage 18: scalar cells and conservative atomic counters

Compiler-identified Cell models preserve scalar storage through aliases, branches and callbacks.
Integer atomic models check load/store orderings and support wrapping add/sub/swap operations over
arbitrary current values. Each access permits interference; mutable static initializers are never
frozen. Atomic-only wrappers preserve their structure without reading runtime counter contents.

Host/ARM tests cover positive, refuted and unknown cases. Cell alias/callback updates prove;
unchecked counter arithmetic, invalid ordering and unsupported history assertions refute.
RefCell guards and general raw-pointer operations remain unknown. Ordinary Cargo verification
also proves the unchanged fleet validate::Site::fail and total on ARM. There are 58 compiler
integration tests and one metadata test.

## Stage 19: external checked contracts and explicit trusted summaries

JSON sidecars add contracts to unchanged code without a metadata dependency. Checked clauses
execute actual bodies, constrain root domains and verify call/return obligations. Trusted
summaries require an exact function selector, a no-panic claim and a reason. Generic calls need
an exact compiler instance. Return values remain arbitrary unless constrained, and explicit
memory effects preserve known aliases while claiming a frame for other storage. Omitted effects
invalidate modeled storage; unsupported ownership shapes and inconsistent summaries stay unknown.

Schema version 8 records the configuration, matched selectors and every used summary with its
reason, instance, crate hash and call site. Conditional roots are PROVED_WITH_ASSUMPTIONS,
excluded from ordinary proof counts and rejected unless --allow-assumptions is explicitly set.
Selecting the assumed function itself still checks its actual body.

Host/ARM tests refute out-of-bound calls, erroneous success assumptions and stale state claims.
Cargo tests use unavailable dependency bodies, check configuration forwarding and preserve strict
exits. Malformed/stale selectors, unsupported references and inconsistent summaries fail. There
are 63 compiler integration tests and one metadata test. Report-format and CLI presentation work
is deferred to the next step.

Ordinary ARM Cargo analysis proves the unchanged fleet controller::Pid::reset and its external
final-state postcondition without annotations or a new dependency. The selected check takes
0.25 seconds on the development machine, including compilation. This is selected-root evidence;
the published whole-workspace survey has not been rerun for these memory stages.

## Stage 20: progress, readable results and JSONL

The default CLI view prioritizes failed/incomplete/conditional roots, caps compact rows and long
details, groups the largest gaps, and ends with aggregate outcomes and elapsed time. --verbose
retains the complete inventory/obligations view; --summary remains a compact-view alias. Terminal
colors distinguish all four outcomes without replacing their labels. Auto color honors NO_COLOR;
explicit always/never modes and --quiet support terminals and captured logs.

Builds, active crate/root positions and report processing announce progress on stderr. Five-second
heartbeats keep long solver checks visible without redrawing over compiler diagnostics. JSONL
exports preserve one full raw report per line. Stdout exports redirect all human/Cargo messages
to stderr, including Cargo JSON messages. A report subcommand reads saved schema-7/8 JSON or JSONL,
recomputes cached counts and preserves strict exits; it explicitly states that analysis was not
rerun. Malformed/empty/unsupported inputs and output failures fail the command.

Compiler regressions cover clean machine streams, captured/forced colors, distinct trusted labels,
saved-report exits, legacy input, malformed input, Cargo message forwarding and a deliberately
slow solver that must show active-root progress. There are 67 compiler tests and one metadata test.

## Stage 21: direct compiler-configuration reuse and crate/main selection

--from-report FILE reuses a saved schema-7/8 compiler invocation for fresh source analysis without
calling the Cargo wrapper. --verify without --entry already selects every inventoried crate body;
an explicit main selects only that root and its reachable calls. Saved proof results, selectors
and user-trusted configurations are not reused. Compiler mismatch, missing arguments and mixed
raw/reused arguments fail. Direct compiler probes cannot masquerade as verification, while Cargo
wrapper probes retain their required behavior.

Tests prove a bounded binary main beside an independently refuted helper, then mutate the source
and reject the previously passing main despite loading its saved report. Legacy configurations,
invalid invocations and missing main selectors are covered. Main-related MIR hints help users
select expanded macro names. A coroutine construction case stays unknown; no async execution
semantics were added. There are 70 compiler integration tests and one metadata test.

## Stage 22: integer population counts and checked float clamp

The compiler-identified ctpop model counts every input bit, returns u32 and covers signed,
unsigned and target-width integers. Primitive core f32/f64 clamp has an explicit panic obligation
for ordered, non-NaN bounds, then preserves NaN inputs and signed-zero ties. Compiler identity and
normalized signatures distinguish these models from same-named user functions.

Host/ARM tests prove byte count/complement relationships, masks, integer-width bounds, guarded
float ranges and special values. Invalid bounds and incorrect assertions refute. Mutating a mask
bound or reversing a passing clamp guard also refutes with a counterexample. Unsupported pointer
inputs and raw float bit observations remain unknown. There are 73 compiler integration tests and
one metadata test.

## Stage 23: bounded owned aggregate repeats

The repeat evaluator now accepts nested arrays, tuples, structs and enum values with owned
modeled contents. Copies preserve symbolic fields and variant tags, while independent writes
stay local to the selected copy. The model rejects tracked storage identities and limits each
repeat to 128 elements and 256 modeled values to bound recursive expansion. Input and constant
non-byte array limits remain unchanged.

Host/ARM compiler tests cover six-by-six float matrices, diagonal initialization, tuple/struct/enum
copies, larger generated arrays and nested byte-array writes. Wrong copy assertions and a wrong-row
write mutation refute. Large repeats, ambiguous composite indices and repeated interior-storage
accesses remain unknown. There are 75 compiler integration tests and one metadata test.

## Stage 24: shared slice cursors and checked predicate callbacks

Compiler-identified slice iterators retain their source and front/back positions. Models advance
actual iterator storage, yield source elements and support forward/reverse traversal, symbolic
skips, lengths, count, size hints and independent clones. Generic enumeration, copied and reverse
adapters execute core MIR. all/any execute actual concrete callable bodies, propagate memory
updates and stop at the deciding element. Existing execution budgets bound unfinished loops.

Compiler layouts supply constant discriminants for enum representations with one variant,
including the uninhabited residual used by optimized question-mark MIR. Host/ARM tests cover
ordinary/zero-sized/composite values, bounded byte slices, cursor exhaustion and Cell effects.
Incorrect assertions/callbacks and order/short-circuit mutations refute; 4,096 host cases agree
with direct formulas. Unsupported views and unbounded loops stay unknown. There are 78 compiler
integration tests and one metadata test.

## Stage 25: mutable slice projections and enumeration

Mutable slice cursors yield references into existing typed allocations, preserving writes through
forward/reverse traversal and callbacks. Mutable enumeration and iterator passthrough are explicit
core models with counter-overflow obligations. Known Some mutable payloads preserve reference
identity on unwrap. Legacy byte views attach to typed storage and later copies use that storage.

Host/ARM tests prove array postconditions, independent element writes, tuple and byte updates,
short-circuiting and six-by-six matrix initialization. Wrong writes, alias assertions and unchecked
increments refute. A wrong-column mutation refutes; independent runtime tests cover 4,096 mutable
cases alongside the 4,096 immutable cases. Ambiguous composite writes and mutable returns remain
unknown. There are 80 compiler integration tests and one metadata test.

Borrowed array/slice IntoIterator factories use the same cursor models. Exact primitive float
finiteness classification reduces helper-call steps in numeric predicates, with host/ARM cases
covering NaN, infinities, signed zero and an unconstrained assertion that must refute.

## Stage 26: persistent root-local solver sessions

Feasibility checks request SAT/UNSAT without an unused model. The default backend lazily starts
one Z3 process per root, resets before each query and fetches counterexample models in the same
query context. A root-local exact-query cache holds at most 1,024 entries or two MiB of query
text. Undecided responses never enter the cache. The compatibility executable override retains
its one-shot protocol.

Protocol tests verify declaration/assertion isolation, cache keys and limits, current-context
models, malformed responses, EOF and a stalled writer killed by the six-second host deadline.
The existing compiler, negative, unknown and mutation suites continue to pass.

A local host benchmark used the old and new release binaries, normal Z3 without a profiling
wrapper, one warmup and five alternating measured runs per binary. Median direct analysis times
for the synthetic slice fixture were:

| Root | Before | Persistent session and decision cache |
| --- | ---: | ---: |
| units | 0.359 s | 0.071 s |
| enumerate | 0.957 s | 0.129 s |
| floats | 1.047 s | 0.121 s |

All three roots remained PROVED. These measurements use Apple Silicon, Z3 4.15.4 and the pinned
nightly's host library configuration; they are not a general speed guarantee. Resource limits,
unsupported behavior and the proof domain are unchanged. There is no cross-invocation proof
cache or precomputed standard-library summary store.
