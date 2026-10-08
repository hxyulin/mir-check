# miren

Formerly mir-check. The binaries and Cargo package now use `miren`, the metadata crate is
`miren-contracts`, and project settings use `miren.json` and `MIREN_*` environment variables.

Host compiler adapter, project orchestration and report model for checking Rust MIR with Z3.
The adapter requires the exact nightly compiler in the repository's toolchain file.

## Commands

Install all three binaries with `cargo install --path crates/miren --locked`:

- `miren`: checks the current Cargo project, reads saved results, or drives rustc directly.
- `cargo-miren`: the Cargo subcommand; delegates to the same project implementation.
- `miren-rustc`: retains dependency MIR through Cargo's outer compiler wrapper.

```sh
miren --entry module::function --lib
miren --async-entry task --bin firmware
miren report
miren inventory --verbose --lib
```

Project verification is enabled by default. An optional flat `miren.json` stores entry
selectors, Cargo arguments, limits and explicit analysis policies. CLI selectors replace configured
roots and CLI limit options override configured values. Configuration cannot enable native replay.
There are no built-in presets or named check profiles. See the [usage guide](../../docs/usage.md).

## Build and result lifecycle

Project discovery uses the pinned Cargo's workspace metadata. Cargo supplies target, profile,
features, dependency paths and compilation environment. Project analysis owns an isolated cache
under the Cargo workspace's `target/miren`, with separate retained-MIR and plain dependency
builds. Before checking, it cleans only workspace package artifacts in that cache, so current
roots are always recompiled and reanalyzed. Dependency source/configuration changes remain subject
to Cargo's ordinary fingerprints. An OS file lock serializes access to the shared analysis cache;
waiting reports progress, and process exit releases the lock.

Each attempt writes fresh per-crate JSON reports into its own run directory. `latest.json` records
completion and run success; it is published before analysis as incomplete and replaced after
reporting. The default report command checks that record and cannot return success for a failed
or interrupted run. Reading an explicit JSON/JSONL file retains the existing per-report policy.
Saved display never establishes a new proof or validates current source provenance.

Dependency retention appends `-Zalways-encode-mir=yes` and `-Zmir-opt-level=0` without rewriting
Cargo's target/profile flags. It does not rebuild precompiled sysroot libraries or give foreign
functions a Rust body. `-Zbuild-std=core` can expose core MIR on a configured target. Missing bodies
and unsupported operations remain UNKNOWN.

## Compiler integration

The proof engine consumes typed MIR directly. Inventory strings are diagnostics, not parsed
solver input. `miren rustc` preserves the direct compiler interface; existing explicit Rust
source/argument commands and `--from-report` remain supported. Direct mode retains its explicit
`--verify` flag. Saved invocation reuse checks the compiler identity, analyzes current source and
requires the original dependencies and working directory. Project commands rebuild through Cargo
instead of asking users to manage those artifacts.

Ordinary and async entry scopes differ: selecting an async constructor with `--entry` checks
construction; `--async-entry` also polls its fresh future through Pending and Ready states.
Executor scheduling, cancellation and unbounded suspension require additional models or proofs.
Startup entry assumptions are explicit and are not inherited by async entry analysis.

Contracts are metadata. Callee bodies and configured bounds are checked; annotations do not become
unchecked verified summaries. Explicit trusted boundaries and startup domains remain listed as
assumptions. Replay executes supported native inputs only with `--replay`, reports its independent
evidence, and never upgrades a passing execution to PROVED.

The [coverage guide](../../docs/coverage.md) is the maintained feature reference. The
[proof guide](../../docs/proofs.md) explains the encoding and limitations, and the
[development guide](../../docs/development.md) lists required checks.
