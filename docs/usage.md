# Running mir-check

## Install

The repository's toolchain file installs the exact nightly compiler and components needed by
rustc_driver. Build the host analyzer with that toolchain even when analyzing another target.

```sh
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
| REFUTED | A supported translated obligation has a satisfying failing assignment | Nonzero |
| UNKNOWN | Unsupported behavior, missing MIR, solver failure or an exploration/input/query limit prevented completion | Nonzero |
| Unselected | The body has an inventory, without an independent root proof | Does not affect selected-root verification |

An inventory run without `--verify` reports sites as unverified. Its successful exit establishes
that compilation and inventory completed. Inventory call paths are structural and ignore branch
feasibility. `--entry` works in inventory mode too, to request those paths.

`--summary` prints root results and groups UNKNOWN obligation details by affected root. A refuted
root can also have unknown obligations, so gap groups may include it. Distinct interpreted
instances are deduplicated within one crate report; they can include callees and dependency
bodies. The counts are not statement/branch coverage percentages or whole-crate safety claims.

Omit `--summary` to inspect sites, assumptions, named input bindings, interpreted instances,
trusted models and individual obligations. Schema version 7 JSON includes:

- `coverage`: inventoried/selected/unselected counts, root outcomes, interpreted instance count
  and gap groups.
- `functions[].proof`: root status, assumptions, input bindings, interpreted bodies, explicit
  library models and obligations with SMT queries and optional solver models.
- `functions[].sites`: the independent unverified MIR inventory.
- `rustc_arguments`, `compiler`, `target`, `panic_strategy`, `overflow_checks`: analysis build data.

Cargo writes reports to a fresh `target/mir-check/<run>/reports` directory and prints its path.
It renders available reports even when verification fails. Compiler failures can leave reports
for other completed crates; they cannot produce a successful run. Missing requested roots also
fail while retaining the collected reports. Build directories can be removed after use.

Direct mode accepts `--json`, `--summary`, `--verify` and repeated `--entry` before rustc arguments:

```sh
target/debug/mir-check --json --verify --entry next_byte -- \
  --crate-type=lib --edition=2024 tests/fixtures/proofs.rs
```

`--json` takes precedence over `--summary` in direct mode. Cargo always retains JSON reports;
its console summary is optional. Direct mode rejects missing roots before emitting a report.

## Work through UNKNOWN

Start with an inventory or a small root, then inspect the summary's gap reasons. An unsupported
root input requires new input modeling; selecting a smaller concrete caller may let it construct
supported values. Missing dependency MIR cannot be solved by an annotation that merely claims
the call is safe. Larger loops may need invariants rather than more unrolling. Read-only nested
structs and tuples are supported, but recursive reference shapes, mutable fields and arbitrary
enum inputs remain unsupported.

The checker stops a root at its first unsupported operation, so its reported reason need not
enumerate every gap in that function. A solver assignment is a counterexample to the translated
obligation; runtime replay tests provide separate evidence for the confirmed fixture failures.
