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

## Stage 27: ordered array generation and iterator regression coverage

The core::array::from_fn model runs actual concrete callback bodies in ascending index order,
propagates memory effects and conditions, and checks callable contracts. Empty arrays never invoke
callbacks. Generated results must be owned and fit the existing 128-element/256-value budgets;
destructors, storage identities and unsupported mutable captures remain UNKNOWN. Core identity
and the normalized signature gate the model; a same-named application function runs its own MIR.

Synthetic ticket, parcel and label cases exercise 23 roots on host and ARM: 12 prove, four refute
and seven remain unknown. Tests cover callback order, Cell effects, function items, floats,
zero length and exact shape boundaries. Invalid assertions, arithmetic and call bounds refute,
as does a label-offset mutation. Native tests independently replay 512 inputs and callback effects.

Iterator audit regressions cover mixed forward/reverse skips, usize::MAX exhaustion, zero-sized
mutable elements, skipped byte storage, shared Cell callbacks and disjoint projections. False
alias assertions and an exhaustion mutation refute; unsupported views and owned mutable callback
environments remain unknown. Native execution checks 1,792 bounded cases. The review found no
new reproducible false proof in these paths; the interpreter has not completed a soundness audit.
There are 86 compiler integration tests and one metadata integration test.

## Stage 28: exact ground Boolean and bit-vector decisions

A bounded in-process evaluator validates complete generated SMT scripts and decides supported
fully constant assertions. It implements wrapping bit-vector arithmetic, bitwise operations,
signed/unsigned comparisons and exact sign/zero extension through 128 bits. Symbolic expressions,
floats, arrays, malformed scripts and unsupported operations fall back to Z3. False ground
failure conditions can discharge obligations without a solver process; failing obligations still
request real counterexample models. Custom solver overrides retain their compatibility behavior.

Tests compare 1,158 arithmetic, comparison and extension cases with Z3 at widths 1, 8, 16, 32, 64
and 128, including wraparound and signed boundaries. Removing the signed-order transformation
breaks the differential test. Parser tests cover unsupported content after false assertions,
malformed arity/types, declaration collisions, invalid widths and parser budgets. Integration
with the solver verifies that constant decisions need no process while counterexamples still
require a model. This is exact evaluation of a restricted closed-expression subset, not a new
solver for symbolic floating point or memory.

A follow-up direct-analysis benchmark, with the test suite finished, used one warmup and seven
alternating measured runs per release binary under the same host/solver configuration. Median
old-to-final times were 0.323 to 0.047 seconds for units, 0.699 to 0.087 seconds for enumerate and
0.976 to 0.114 seconds for floats. All remained PROVED. Floating-point queries use Z3; their gain
comes from session reuse and decision caching. These are small synthetic roots, not a promise
about branch-heavy or solver-heavy code.

## Stage 29: compiler identities and parsed contract references

The MIR interpreter already operates on typed rustc enums rather than parsing debug output.
This stage removes remaining semantic substring shortcuts: panic language items have an explicit
typed match, atomic Ordering arguments and variants are checked against the normalized compiler
enum, and array from_fn resolves a module child DefId anchored by ArrayIntoIter. Post-state
contract references are detected in parsed expressions, with Option-arm bindings scoped locally;
fields, comments and unrelated names containing final_ do not activate post-state snapshots.
Internal checked arithmetic selection uses an explicit closed operation set.

Host/ARM identity regressions check seven roots: four prove, two refute and one remains unknown.
Application types and functions with library-like names execute their actual bodies. Existing
Cell/atomic regressions preserve their outcomes. AST tests distinguish free post-state paths,
fields, comments and shadowing Option bindings. Ordinary source selectors and report strings
remain text interfaces; they do not determine MIR instruction semantics.

## Stage 30: owned array cursors, folds and evaluated closure constants

Compiler ArrayIntoIter uses the existing typed cursor representation without modeling general
MaybeUninit or transmute. Owned elements have no tracked identities or destructors and fit the
128-element/256-value limits. Models preserve order, skips, count/last, checked predicates,
fold/rfold callbacks and writable reference passthrough. Harmless wrapper drop glue follows typed
fields only when no user destructor can execute. Owned Clone and iterator views remain unknown.

Shared slice fold/rfold execute actual callback bodies and propagate accumulator, conditions and
memory. Ordinary sum MIR proceeds through evaluated noncapturing closure constants, whose typed
upvars, zero fields and zero-sized layout are checked. Captured constant closures remain unknown,
including zero-sized captures. The coverage work caught a cursor-state bug: count through a
mutable shared iterator reference must exhaust the original cursor. Positive and negative
borrowed-consumption regressions now cover count, last, fold and reference passthrough.

The owned fixture checks 29 roots on host/ARM: 13 prove, nine refute and seven remain unknown.
The closure fixture checks 15 roots: eight prove, four refute and three remain unknown. Native
replay, failing callback/call bounds, element-order and fold-seed mutations exercise these models;
the closure suite checks 1,024 inputs and independently replays four panics. Unsupported views,
captured constant environments, destructors and stateful owned callback captures stay unknown.

## Stage 31: integer primitive intrinsic coverage

Exact compiler identities and normalized integer signatures select min/max, saturating add/sub,
leading/trailing zero counts, byte swapping and bit reversal. Signedness and width are checked
against modeled operands. Saturation uses one extra SMT bit; zero counts return the input width
at zero. Bit rearrangements use exact extracts and concatenation. Scoped operand bindings keep
chained 128-bit operations within existing query limits. Unsafe nonzero-only intrinsics remain
unsupported; no resource caps were increased.

Host/ARM tests check 21 roots: 17 prove, three refute and one remains unknown. All 12 scalar
integer types are exercised, and reports confirm all eight intrinsic models ran. Native tests
cover all 65,536 operand pairs for both unsigned and signed eight-bit properties, all 256 byte
inputs for independent zero-count oracles, 128-bit boundaries and direct byte layout checks.
Three failing mutations are independently replayed. The combined suite has 94 compiler
integration tests, 13 solver/contract unit tests and one metadata integration test.

## Stage 32: tracked aggregate borrows and callback environments

Mutable references retain their allocation IDs and projections inside tuples, structs, enums,
closures and supported iterator adapters. Returns preserve incoming storage references; graph
checks reject callee-local/dead borrows, including references hidden in caller storage. General
root aliasing, legacy mutable byte captures and ambiguous non-byte writes remain unknown.
Core Zip/Flatten bodies can now proceed through tracked mutable aggregate storage.

FnMut calls borrow their actual environment. Map/from_fn, predicates and folds allocate one
tracked environment per modeled invocation, carry owned capture updates and external writes
between callbacks, and retire the environment on completion. Callback destructors remain unknown.
Map preserves reference-valued input elements. Existing false assertions that callback state
resets between calls now refute; borrowing external counters can prove.

The synthetic aggregate fixture checks 19 roots on host/ARM: 13 prove, three refute and three remain
unknown. Native tests exercise 700 bounded calls, stateful callback cases and negative panic
catches. A struct-field swap mutation refutes, and four internal graph regressions cover hidden
local escapes, dead storage, nested incoming references and untracked byte views. These tests are
regression evidence rather than a completed alias/lifetime audit.

## Stage 33: floating-point storage encodings

Float values retain IEEE storage bits alongside numeric SMT values. Inputs, evaluated constants,
from_bits, moves, array selection, negation, abs and clamp preserve the selected encoding,
including quiet/signaling NaN signs and payloads. to_bits and same-width integer/float transmutes
observe that encoding. Arithmetic/cast results get one stable encoding constrained to their
numeric IEEE value. NaN arithmetic outputs conservatively allow every encoding, including
signaling NaNs; bit-level counterexamples can therefore fail to replay on the target.

The synthetic storage fixture checks 17 roots on host/ARM: 13 prove, three refute and one remains
unknown. A sign-mask mutation refutes; native cases cover zero signs, subnormals, infinities,
NaN input encodings and 1,024 integer-cast/arithmetic inputs. Checked sidecars also force actual
core to_bits/from_bits bodies through typed transmutes without library summaries. Float remainder
and wider formats remain unknown.

## Stage 34: incremental queries, ground folding and instantiated MIR reuse

The default solver consumes structured declarations/assertions, retains a common assertion prefix
and uses push/pop for branch suffixes. Declarations are installed outside assertion scopes;
incompatible namespaces reset the session. Counterexamples still come from the current context,
and reports retain complete standalone SMT scripts. Exact decision-cache bounds, protocol
failure handling and the custom executable compatibility path remain unchanged.

Exact closed Boolean/bit-vector folding reduces repeated MIR expression growth. A root-local cache
shares at most 128 normalized bodies through Rc, keyed by the full compiler Instance rather than
DefId alone. It caches no function proofs or unchecked summaries and persists no compiler objects
to disk. Regression tests cover assertion-prefix changes, new/incompatible declarations and
current models, alongside differential ground-expression tests and generic-instance separation.

Default exploration increases from 256 to 2,048 steps and from eight to 16 active call frames.
Finite recursion may complete under those limits; unfinished recursive/loop paths remain UNKNOWN.
The 30-second root budget, 200,000-byte query limit and five/six-second solver/request limits remain
unchanged. Speed improvements reduce repeated work; they do not guarantee that larger limits
finish branch-heavy proofs or justify truncating unfinished paths.

On the development host, alternating release builds were measured after a warm-up, using five
measured samples each and the same Z3/compiler configuration. A bounded symbolic counter loop
remained PROVED and improved from 0.258 to 0.060 seconds. A fixed 256-iteration loop went from
UNKNOWN at the old step limit (0.871 seconds) to PROVED (0.048 seconds); its late-panic counterpart
went from UNKNOWN (0.901 seconds) to REFUTED (0.048 seconds). A twelve-helper call chain went from
UNKNOWN (0.035 seconds) to PROVED (0.040 seconds). These small synthetic cases demonstrate lower
expression overhead and completed exploration, rather than a general speed guarantee.

## Stage 35: optimized unit returns and Option panic boundaries

Unit-returning MIR may leave the return local unassigned; execution now returns the unit value
while preserving tracked memory effects. Missing non-unit return locals still remain UNKNOWN.
Option unwrap/expect panic helpers are identified by the actual Option module, exact function
names and never-returning signatures, then checked as panic entry points before operand decoding.
The model is pinned to the compiler; same-named application functions receive no special behavior.

Original host/ARM cases check successful/guarded Options, reachable unwrap failures,
application-name collisions, primitive AddAssign effects and unknown function-pointer calls.
String reborrows before some expect calls remain UNKNOWN. Native cases and a changed increment
verify that unit-returning calls preserve their writes.

## Stage 36: evaluated constant tables and scalar index selection

Evaluated immutable arrays/slices accept up to 128 elements, including non-byte scalars and
composites. The 256-value total budget and eight-level depth limit remain. Root non-byte inputs
still have at most 16 elements. Constants are fully decoded; this does not add lazy storage or
permit uninitialized/interior-mutable data. Exact ground indices select directly; compatible
scalar symbolic reads use one bounds query and conditional selection, avoiding a solver query
for each possible element. Composite reads still require a uniquely determined index.

Original host/ARM regressions cover integer/Boolean tables, exact float payloads and copied
aliases, structs, nested arrays, bounded slices and resource exhaustion. Wrong bounds and payload
assertions refute; a changed table-spacing mutation refutes and fails under native execution.

## Stage 37: defer unused float representation constraints

Computed float values retain fresh stable raw-bit symbols. Their numeric/encoding equalities are
stored within the root and included in a query only when its expressions reference those symbols.
Exact token matching follows dependencies transitively, including bit-preserving transformations
and roundtrips; uncertain lexical forms include every relation. Numeric expressions, ordinary path
conditions, the NaN overapproximation and resource limits remain unchanged. Exported SMT scripts
contain the required equalities and are independently runnable.

Original host/ARM regressions cover array-based gauge encoding, arithmetic/return/transformation/
selection roundtrips, transitive conversions and stable copied bits. An incorrect assertion
refutes; float remainder stays unknown. Unit regressions distinguish exact variable tokens,
unused declarations, cyclic dependencies and conservative lexical fallback. Native IEEE cases
exercise signed zeros, subnormals, infinities and NaNs.

On the development host, one warm-up plus three measured release runs of the original gauge
fixture improved from a 4.887-second median UNKNOWN result to a 0.056-second median PROVED result.
Other analysis processes were active; this is a small-case measurement rather than a general
speed guarantee.

## Stage 38: immutable static string transport

Evaluated literal strings can move through shared dereferences/reborrows, nested arguments and
returns using the existing opaque marker. Option expect can now reach its compiler-identified
panic helper. Strings receive no length, content, equality or pointer semantics; mutable
string-reference storage remains unknown. Original host/ARM regressions check guarded/unbounded
expect paths, application helper identities and rejected unsupported operations. A guard mutation
refutes, and native execution independently checks the shared-reference cases.


## Stage 39: allocation-backed local bytes and finite chunk views

Local mutable byte references use tracked allocations across calls and returns. Prefix views and
copy_from_slice update the original allocation. Compiler-identified byte-slice as_chunks_mut returns
chunk and remainder projections into one allocation, checking nonzero width and finite bounds. Chunk
writes translate directly to parent byte offsets. Symbolic lengths, excessive views and unresolved
root aliases remain UNKNOWN. Fixed byte post-state contracts support checked literal indices.
Original host/ARM cases exercise helper writes, returned views, disjoint copies, parent updates,
impostor methods and failure/unknown cases; native replay and a mutation verify writes.

## Stage 40: resolve dependency function items and encode integer bytes

Function-item callbacks resolve concrete trait dispatch before requesting MIR. Map/from_fn,
predicates, folds and Fn adapters execute actual implementations, including panic paths. Exact
primitive integer endian encoding complements decoding on all signed/unsigned widths. Missing-body
diagnostics distinguish omitted prebuilt core bodies from genuine foreign declarations. The existing
-Zbuild-std=core Cargo path retains core MIR and saved reports can replay it; unsupported pointer
operations remain UNKNOWN after those bodies become available. No foreign bodies are assumed
panic-free. Host/ARM, native boundary cases, mutations and captured replay cover this work.

## Stage 41: retain solver prefixes across declaration growth

Live Z3 sessions use global declarations so newly introduced symbols survive scope pops without
rebuilding shared assertions. Incompatible namespaces and protocol errors still reset the session.
Standalone report query text remains unchanged. Empty deferred-encoding maps skip dependency lexing,
and construction borrows condition strings instead of cloning an intermediate list. Scope-survival,
model-freshness and replay regressions verify answers; removing the global option breaks the new
test. Synthetic optimized workloads improved from 698 to 47.5 ms for 256 growing queries, and from
187 to 99 ms for constructing 5,000 queries, using six alternating samples.

## Stage 42: broader bounded roots and scalar validity

Root inputs allow 512 values, 16 nested levels, 256 non-byte array elements and 64 enum variants.
Constants retain their separate eight-level, 256-value and 128-element budgets. Integer pattern
ranges and alternatives constrain root values to their compiler-described domains. Unicode char
inputs exclude surrogates and values above U+10FFFF. The actual core NonZero getter exposes the
modeled scalar; unrelated get methods execute normal MIR. No constructor invariants are inferred for
other structs. Original host/ARM roots, native valid-domain cases, bound failures and a changed
divisor exercise these extensions. Exhausted or unresolved domains remain UNKNOWN.

## Stage 43: share byte source expressions

Byte region copies and integer endian conversion bind their source expression once with scoped SMT
lets instead of repeating it for each byte. This preserves source snapshots, disjoint writes and
byte order while avoiding expression growth. Resource caps remain enforced. Original numerical
byte-buffer regressions and native cases exercise compact views and subsequent parent reads.

## Stage 44: larger completed paths and leaner loop conditions

The default execution budget increases to 8,192 steps. Call depth remains 16, roots retain a
30-second deadline and queries retain their 200,000-byte bound. Unfinished paths remain UNKNOWN; the
checker never infers completion from budget exhaustion. Literal true path conditions are discarded
before block feasibility checks, avoiding repeated copies and scans on long loops.

Host/ARM regressions complete 1,024-iteration loops and refute a failure after the final iteration.
A longer unfinished loop remains UNKNOWN. Existing bounded-recursion and call-depth failures retain
their outcomes. This increases exploration capacity without adding loop invariants or termination
proofs.


## Stage 45: guarded unchecked arithmetic and compiler check operands

Unchecked MIR AddUnchecked, SubUnchecked and MulUnchecked operations check signed/unsigned
overflow using the existing widened bit-vector arithmetic. Each operation emits a validity
obligation before its result is used; only the non-overflow domain continues. Disabling runtime UB
checks never assumes that unchecked arithmetic is valid. A compiler-identified, signature-checked
cold_path intrinsic models its optimization-only semantics so checked-add overflow branches can
return None.

RuntimeChecks operands delegate to the pinned compiler's RuntimeChecks::value for the active
session, matching code generation for UB, overflow and compiler contract checks. Host/ARM
regressions exercise independent UB/overflow settings, all unsigned widths, signed checked
arithmetic and reachable panic counterexamples. The flattening regression proves core's guarded
unchecked multiplication and remains UNKNOWN at its raw-pointer boundary. Native replay covers all
u8 operand pairs and larger signed/unsigned boundaries; a changed reconstruction refutes. No
unsafe source or foreign-body assumptions are added.


## Stage 46: stop refuted roots after their first counterexample

Verification stops a selected root after its first refuted obligation, retaining that query and
model. Other selected roots still run. The all-failures option continues the former exploration
policy under the same resource limits; it does not promise an exhaustive list of failures.

Reports include a default-compatible stopped_after_counterexample flag, and compact/verbose output
identifies early stops. Cargo forwards the policy to compiler invocations and clears inherited
settings when the option is absent. Original host/ARM cases compare both policies, preserve
positive and unknown roots, replay native failures and expose a late failure after removing its
earlier guard. Caller contracts and postconditions remain checked in either mode.

## Stage 47: avoid unused contract maps and model optimization choices

Ordinary calls build argument name maps only for checked predicates or explicit aliases. Callee
entry snapshots still validate storage, and preconditions, postconditions and alias conflicts
retain their checks. Calls without preconditions avoid a duplicate body/snapshot/name-map pass.
Ambiguous dependency debug names therefore stop blocking ordinary body execution.

The compiler-identified scalar is_val_statically_known intrinsic follows its documented independent
Boolean choice on each call. Both optimization paths must be safe; the checker does not assume
a preferred value or stable repeated results. Pointer operands remain unsupported. Typed
transmute diagnostics identify source and destination types, with unsupported layouts still
UNKNOWN. Original host/ARM, native branch cases, invalid aliases and a guard mutation exercise
these changes. Integer power reaches actual checked_pow MIR but remains unknown at a niche
transmute rather than receiving an unchecked summary.

## Stage 48: typed term DAG and bounded SMT printer

A root-scoped context interns typed Boolean, bit-vector, floating-point and array terms. Node
construction validates sorts, widths, arity and context identity before simplification. Supported
closed Boolean/integer terms fold structurally; wide arithmetic retains its exact solver encoding.
Floating-point numeric equality is separate from structural equality, preserving NaN and signed
zero. The printer visits the DAG iteratively and emits scoped lets when sharing reduces output.
Output budgets fail explicitly instead of emitting incomplete expressions.

Differential tests check all 55 operator encodings and 3,150 closed arithmetic/comparison boundary
cases against Z3, validate array reads/writes and conversions, reject malformed constructions and
check a changed arithmetic formula fails equivalence. Repeated doubling stays compact through 256
levels rather than expanding exponentially. This is the representation foundation; existing MIR
consumers still use strings pending migration. No new language coverage or loop proofs are claimed.

## Stage 49: interpret MIR with typed terms

Symbolic scalar values, byte arrays, path conditions, contract predicates and float encoding
constraints now carry interned terms. Arithmetic, casts, integer intrinsics, population counts,
endian conversion, byte-region copies and atomic ordering use typed operators. Byte offset views
lower to capture-free array lambdas inside the printer. Persistent state snapshots retain immutable
nodes, and copies no longer need manually named source lets. Deferred float encoding constraints
follow structural symbol dependencies, including cycles, rather than lexing SMT text.

Production proofs no longer parse or fold string expressions. Structural constant decisions
validate assertion types and contexts first. Full query budgets apply during printing, including
declarations, assertion wrappers and the prelude. Reports retain standalone text scripts; the Z3
subprocess, assumptions policy and analysis limits remain unchanged.

The operator differential suite covers 57 encodings. Host/ARM byte, float and integer fixtures
retain their outcomes with smaller queries. An original repeated-signal fixture proves a 20-step wrapping
calculation that previously hit the query-size cap; a changed scale refutes and an unresolved
callback remains UNKNOWN. Native replay covers integer boundaries and the changed assertion.
This changes representation and sharing, not language coverage, type invariants or loop induction.
