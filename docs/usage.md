# Running mir-check

## Install

The repository's toolchain file installs the exact nightly compiler and components needed by
rustc_driver. Build the host analyzer with that toolchain even when analyzing another target.

```sh
git clone https://github.com/hxyulin/mir-check.git
cd mir-check
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements-solver.txt
cargo build --workspace --locked
cargo install --path crates/mir-check --locked
```

The solver is an external executable. The development build locates `.venv/bin/z3` in its source
checkout. An installed build retains that lookup; otherwise use Z3 on PATH or `MIR_CHECK_Z3`.
Removing or moving the source checkout can require updating the solver path or reinstalling.
The binaries also depend on their pinned rustc sysroot being installed.

## Select roots

This command checks every inventoried body in the selected Cargo library:

```sh
cargo mir-check --verify --summary --lib
```

To check the DR16 parser while leaving its derived methods and closure roots unselected, run from
the mir-check checkout:

```sh
target/debug/cargo-mir-check --verify --summary --entry Raw::parse \
  --manifest-path examples/dr16/Cargo.toml --lib --locked
```

Entries are exact inventory names, not patterns or suffix matches. A Rust crate prefix can
disambiguate functions with the same name in different workspace members:

```sh
target/debug/cargo-mir-check --verify --summary --entry mir_check_dr16::Raw::parse \
  --manifest-path examples/dr16/Cargo.toml --lib --locked
```

Use the Rust crate name printed by the inventory; Cargo package hyphens normally become
underscores. Repeat `--entry NAME`, or use `--entry=NAME`, to select several roots. An unqualified
name matches all bodies with that name in the selected Cargo targets. A qualified name can still
match multiple targets sharing a crate name; use Cargo's `-p`, `--lib` or `--bin` to narrow them.

Crates without a matching root keep their inventories and report zero selected roots. Every
requested name must match at least one inventoried body across the run. A typo, a body behind a
disabled feature, or a root excluded by Cargo target selection causes a nonzero exit.

Checker options may appear alongside Cargo options. `--` ends checker-option parsing and passes
the remaining arguments to `cargo check`. Cargo options such as `--features`, `--workspace`, `-p`,
`--profile` and `--target` retain their usual meaning. `--target-dir` is reserved by mir-check.

Selected roots still execute reachable callees and check their call bounds. Unselected functions
do not acquire a universal proof just because one caller interpreted them with particular values.
Metadata is marked verified only for independently selected roots that pass all obligations.

## Dependency MIR

Cargo analysis retains dependency MIR by default. The `mir-check-rustc` wrapper appends
`-Zalways-encode-mir=yes` and `-Zmir-opt-level=0` to compiler invocations. This makes ordinary
non-generic/non-inline dependency functions available, including transitive and shared path
crates. Workspace members still produce the independently selected root reports; dependency
bodies execute only when called by an analyzed root.

The wrapper passes Cargo's arguments unchanged before appending the MIR options. It preserves
`RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, target configuration, features, profiles, panic strategy
and overflow settings. As before, mir-check owns the compiler and wrapper settings for its
isolated analysis build. Dependency MIR optimization is deliberately fixed at level zero, even
if a profile or user flag requests another MIR level. Code-generation optimization settings are
preserved. Fresh target directories prevent reuse of dependencies built without these options.

For a baseline or an inventory that does not need retained dependency bodies:

```sh
cargo mir-check --summary --no-dependency-mir --lib
```

Install all three binaries with the Cargo install command above, or build the whole workspace;
`cargo-mir-check` expects both wrappers beside it. A checkout installed before this feature
needs rebuilding/reinstalling. Direct driver analysis does not rebuild dependencies; its
`--extern` inputs must already contain the required MIR.

This mode does not rebuild precompiled core/std libraries or provide bodies for foreign
functions. Having a body also does not imply that its operations are supported: raw pointers,
mutable state, intrinsics and execution limits can still yield UNKNOWN. Dependency annotations
continue to produce checked call/return obligations, not trusted summaries.

## Embedded builds

The pinned toolchain includes thumbv7em-none-eabihf. This analyzes the fixture's ARM build:

```sh
target/debug/cargo-mir-check --verify --summary --entry Raw::parse \
  --manifest-path examples/dr16/Cargo.toml --lib --locked --target thumbv7em-none-eabihf
```

The report records the compiler, target, panic strategy, overflow checks and compiler arguments.
Target pointer width affects usize/isize proofs. Cargo configuration and build profiles determine
which optional overflow checks exist. A release proof with those checks disabled does not prove
that overflow is impossible. To analyze a build with checks enabled and aborting panics:

```sh
RUSTFLAGS='-Cpanic=abort -Coverflow-checks=yes' \
  target/debug/cargo-mir-check --verify --summary --entry Raw::parse \
  --manifest-path examples/dr16/Cargo.toml --lib --locked --target thumbv7em-none-eabihf
```

Other targets need their Rust target libraries installed for the same pinned compiler and must
stay within the supported MIR subset. Host/ARM coverage in CI is evidence for those builds;
the tool does not claim that every architecture has been tested.

## Read the outcome

| Outcome | Meaning | Verification exit |
| --- | --- | --- |
| PROVED | Every explored feasible path completed and all body/contract obligations passed under the declared root domain | Success if every selected root proves |
| PROVED_WITH_ASSUMPTIONS | Obligations passed using explicit user-trusted call summaries | Nonzero unless --allow-assumptions is set |
| REFUTED | A supported translated obligation has a satisfying failing assignment | Nonzero |
| UNKNOWN | Unsupported behavior, missing MIR, solver failure or an exploration/input/query limit prevented completion | Nonzero |
| Unselected | The body has an inventory, without an independent root proof | Does not affect selected-root verification |

An inventory run without `--verify` reports sites as unverified. Its successful exit establishes
that compilation and inventory completed. Inventory call paths are structural and ignore branch
feasibility. `--entry` works in inventory mode too, to request those paths.

The default compact report prints counts and up to 20 roots per crate, prioritizing refutations,
unknowns and trusted assumptions before ordinary proofs. It includes source locations and the
first matching failed obligation, plus the five largest UNKNOWN gap groups. `--summary` remains
an alias for this view. A refuted root can also have unknown obligations, so gap groups may include
it. Distinct interpreted
instances are deduplicated within one crate report; they can include callees and dependency
bodies. The counts are not statement/branch coverage percentages or whole-crate safety claims.

Use `--verbose` to inspect every root, sites, assumptions, input bindings, interpreted instances,
trusted models and individual obligations. Schema version 8 JSON includes:

- `coverage`: inventoried/selected/unselected counts, root outcomes, interpreted instance count
  and gap groups.
- `functions[].proof`: root status, assumptions, input bindings, interpreted bodies, explicit
  library models, used user-trusted summaries and obligations with SMT queries and optional models.
- `contract_config`, `matched_contracts`: the optional external configuration and matched selectors.
- `functions[].sites`: the independent unverified MIR inventory.
- `rustc_arguments`, `compiler`, `target`, `panic_strategy`, `overflow_checks`: analysis build data.

Cargo writes reports to a fresh `target/mir-check/<run>/reports` directory and prints its path.
It renders available reports even when verification fails. Compiler failures can leave reports
for other completed crates; they cannot produce a successful run. Missing requested roots also
fail while retaining the collected reports. Build directories can be removed after use.

Direct mode accepts `--json`, `--jsonl FILE|-`, `--verbose`, `--summary`, `--verify`, repeated
`--entry`, `--contracts FILE`, `--allow-assumptions`, `--color` and `--quiet` before rustc
arguments:

```sh
target/debug/mir-check --json --verify --entry next_byte -- \
  --crate-type=lib --edition=2024 tests/fixtures/proofs.rs
```

`--json` emits one pretty-printed raw report in direct mode and is mutually exclusive with
`--jsonl`. Cargo always retains per-crate JSON reports. Direct mode rejects missing roots before
emitting a report.

`--contracts FILE` reads a JSON sidecar for checked source-independent contracts or explicitly
trusted summaries. Paths are resolved from the invoking directory and forwarded to compiler
workers. Unmatched selectors fail the run. `--allow-assumptions` accepts conditional proofs while
retaining their distinct status; UNKNOWN and REFUTED still fail. See [contracts](contracts.md)
for examples, effect claims and provenance fields. Schema 7 reports can still be read; new fields
default to empty values when absent.

## Progress and colors

Builds announce that Cargo is compiling and analyzing roots. Each compiler worker announces its
crate and target, then updates its current root and completed-root position. When work lasts more
than five seconds, a heartbeat prints that root and the elapsed time. Report collection announces
how many files it is reading and updates the active file. These are line-based messages on stderr;
they also work in captured logs and leave compiler diagnostics visible. Concurrent Cargo workers
can report progress in interleaved lines.

The final report separates each crate's counts from an aggregate run result, elapsed time and
report locations. Compact rows and long details are limited for readability; raw reports and
--verbose retain everything. Compiler errors and missing selectors still fail even if some
collected roots prove. Inventory-only runs explicitly say that no root proof was performed.

| Display | Meaning |
| --- | --- |
| Green PROVED | Completed proof under the recorded root domain and configuration |
| Red REFUTED | Failing translated obligation |
| Amber UNKNOWN | Incomplete analysis |
| Magenta PROVED_WITH_ASSUMPTIONS | Explicitly conditional on user-trusted summaries |
| Cyan progress | Current build, root or report-processing phase |

Colors are automatic only on a terminal. `--color always` forces ANSI colors, `--color never`
disables them, and NO_COLOR disables automatic color. Labels remain present in every mode.
`--quiet` suppresses progress and Cargo's ordinary build messages while retaining final results
and errors. It does not suppress compiler diagnostics or change verification policy.

## JSONL and saved reports

Export one complete schema-8 crate report per line, including on failed verification when reports
were produced:

```sh
cargo mir-check --verify --entry my_crate::function --jsonl results.jsonl --lib
```

The file contains ordinary JSON objects with the same fields as the per-crate files: build data,
inventory, proof status, SMT queries/models, contracts and trusted-summary provenance. There are
no progress events or ANSI codes in it. An existing output file is overwritten; its parent
directory must exist. A write failure makes the command fail.

For a pipeline, use `--jsonl -`. Human results, progress and Cargo output then go to stderr so
stdout contains only JSONL, even when Cargo uses --message-format=json:

```sh
cargo mir-check --verify --entry my_crate::function --jsonl - --lib > results.jsonl
```

Inspect a saved run without recompiling or rerunning the solver:

```sh
cargo mir-check report results.jsonl
mir-check report target/mir-check/<run>/reports --verbose --color never
mir-check report first.json second.json --jsonl combined.jsonl
```

Both binaries accept the report subcommand. Inputs can be schema-7/8 JSON files, JSONL files or
directories containing those files. Directory entries are sorted, with no recursive scan. Counts
are recomputed from the stored root statuses; supplying the same report twice counts it twice.
Empty input sets, malformed records and unsupported schemas fail without a success report.
The display clearly labels these as saved results; it does not validate the proof's provenance
or establish a new proof. Stored REFUTED/UNKNOWN results produce a nonzero exit, and conditional
results require --allow-assumptions, just as in analysis mode.

## Work through UNKNOWN

Start with an inventory or a small root, then inspect the summary's gap reasons. An unsupported
root input requires new input modeling; selecting a smaller concrete caller may let it construct
supported values. Missing dependency MIR cannot be solved by an annotation that merely claims
the call is safe. Larger loops may need invariants rather than more unrolling. Read-only nested
structs, tuples and enums are supported when every field/payload fits the model. Recursive
reference shapes, mutable fields, unresolved generics and enum/struct slices remain unsupported.

The checker stops a root at its first unsupported operation, so its reported reason need not
enumerate every gap in that function. A solver assignment is a counterexample to the translated
obligation; runtime replay tests provide separate evidence for the confirmed fixture failures.
