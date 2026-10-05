# Fleet workspace survey

On 2026-10-06, mir-check checked the unchanged fleet-2027 shared libraries, DM-MC02 board/robot
workspace and RM-C board workspace for `thumbv7em-none-eabihf`. The current tool can prove the
actual DR16 decoder and some small integer helpers. It cannot yet prove entire control loops or
firmware applications.

## Scope and configuration

- Firmware revision: `b81a247a3295b13553087f5e328f201944a1eb61`.
- Analyzer implementation: `07f1412`, JSON schema 7.
- Compiler: nightly-2026-09-22, rustc commit `1303417c416e1595173d9689e7394c31e136ae95`.
- Target: `thumbv7em-none-eabihf`, panic strategy abort, overflow checks enabled.
- Profiles: shared workspace dev; board workspaces dev with their configured optimization level 1.
- Cargo defaults and locked dependencies; shared workspace excludes the host-only xtask.
- No firmware annotations, new dependencies, source changes or unchecked contract assumptions.

These are development-profile analysis results under the checker's pinned compiler, not proofs
of the stable compiler's release binaries. Inventory ran from each workspace root with the target
explicitly supplied. Firmware-directory runner/linker/CPU flags were not selected; those flags
and release overflow settings require separate measurements. MIR optimization remains at the
checker's level 0.

## All inventoried roots

| Workspace scope | ARM compilation units | MIR roots | PROVED | REFUTED | UNKNOWN |
| --- | ---: | ---: | ---: | ---: | ---: |
| Shared libraries | 7 | 1,087 | 304 | 20 | 763 |
| DM-MC02 board, bring-up and ten robot firmwares | 23 | 1,192 | 20 | 5 | 1,167 |
| RM-C board and bring-up | 2 | 57 | 2 | 0 | 55 |
| Total | 32 | 2,336 | 326 | 25 | 1,985 |

These are independently checked roots, including closures, derives and macro-generated methods.
Host build scripts and third-party dependency roots are excluded. Shared path dependencies are
inventoried in their own workspace rather than counted again for each board. Their independently
selected root results therefore use the shared workspace's default features; board-specific
dependency features can differ. An available dependency body is still executed with actual
arguments when a board root reaches it.

The raw success rate is 14.0%, but that overstates useful application coverage. For example,
zerocopy generates many empty marker methods that prove trivially.

## Function declarations without closures or derives

To expose that skew, this table counts roots whose reported source line contains a function
declaration, excluding names containing `{closure#`. This is a source-span filter, not a full
AST classification. It includes handwritten trait methods and instantiated methods emitted from
local function-declaring macros. Separate compilation units remain separate roots.

| Scope | Function roots | PROVED | REFUTED | UNKNOWN |
| --- | ---: | ---: | ---: | ---: |
| Shared libraries | 436 | 22 | 20 | 394 |
| DM-MC02 workspace | 523 | 3 | 5 | 515 |
| RM-C workspace | 17 | 0 | 0 | 17 |
| Total | 976 | 25 | 25 | 926 |

The current success rate for this subset is 2.6%. It is a count of function roots under arbitrary
supported inputs, not line coverage, runtime branch coverage or a fraction of firmware safety.

| Shared crate | Function roots | PROVED | REFUTED | UNKNOWN |
| --- | ---: | ---: | ---: | ---: |
| attitude | 19 | 0 | 0 | 19 |
| can-frame | 9 | 4 | 2 | 3 |
| controller | 20 | 0 | 0 | 20 |
| devices | 216 | 10 | 18 | 188 |
| link | 11 | 3 | 0 | 8 |
| subsystems | 156 | 5 | 0 | 151 |
| validate | 5 | 0 | 0 | 5 |

## Useful positive results

The actual `devices::dr16::Raw::parse` proves with 43 obligations, seven interpreted bodies and
no declared entry assumptions. Every valid byte slice is in its domain. This establishes panic
freedom; the original function has no annotations asking for decoded-value postconditions.
The separate [vendored fixture](examples.md#dr16-parsing) adds and verifies those postconditions.

Other passes include both CAN constructors and ID getters, `dr16::Switch::from_wire`,
`link::EngineerMode::from_wire`, `link::Flags::from_bits`, the link bit helper, several integer-only
motor constructors and small state/default helpers. The robot/bring-up function passes are
`Gripper::new`, its handwritten default implementation and `Text::is_empty`.

The constructed CAN payload and bus-configuration fixture proofs remain stronger targeted
examples than whole-crate scanning: a caller supplies useful relationships that an arbitrary
independent struct input does not have.

## What refutations mean here

The 25 refuted roots are failures over their unconstrained root domains, not 25 confirmed
firmware defects. They identify assumptions worth expressing and checking at callers:

- CAN and referee data accessors, console `Line::as_str` and `Text::as_bytes` need stored lengths
  within buffer capacity. Private fields currently acquire arbitrary symbolic values; the
  checker does not infer constructor-preserved type invariants.
- Several motor constructors intentionally panic on invalid IDs. Their useful guarantee is
  conditional on accepted ID bounds or a caller that constructs a valid configuration.
- `motor::received_tick` requires a nonzero `period_us`, as its existing documentation states.
  A root over arbitrary u32 values includes zero and refutes division safety.
- `dji_uses` and referee `frame_len` have arithmetic obligations that require input bounds.
- Three chassis `count` helpers can overflow before their final clamp for arbitrary i32 values.
  Proving their actual callers requires the state/count bounds maintained by the control loop.

Solver assignments concern translated obligations. Runtime replay or caller analysis is needed
before treating one as a reachable defect. The survey did not modify firmware or add replay tests.

## Where coverage should grow next

The first unsupported reason for most roots is an input shape: mutable receivers, floats,
arbitrary enums, foreign structs, generic parameters or hardware handles. Other blockers include
atomics, promoted/static constants, transmute operations, library intrinsics and async machinery.
The first gap is not an exhaustive list of what a function needs.

The next practical work is owned mutable state with explicit alias limits, arbitrary enum inputs
and floating-point support. Float modeling must preserve NaN, infinities and casts; panic checking
may need less precision than numeric postconditions, but unsupported values cannot simply be
treated as safe. Type invariants and caller bounds would then turn several apparent root failures
into meaningful conditional checks. Hardware/async effects and loop invariants are separate,
larger pieces of work.

## Measurement procedure and cost

First inventory each complete workspace, because verification failures can prevent Cargo from
building dependent crates. From the relevant workspace root:

```sh
# Root workspace: all seven embedded shared libraries.
/path/to/mir-check/target/release/cargo-mir-check --summary \
  --workspace --exclude xtask --lib --target thumbv7em-none-eabihf --locked

# bsp/dm-mc02 and bsp/rm-c: library and binary targets of their members.
/path/to/mir-check/target/release/cargo-mir-check --summary \
  --workspace --target thumbv7em-none-eabihf --locked
```

Next invoke the direct driver's `--json --verify --` mode for each ARM report's recorded
`rustc_arguments`, from the same working directory and with Cargo's `CARGO_CRATE_NAME` restored.
The survey used four parallel compiler processes. Every replay reproduced the original
inventory's names, source spans and arguments. The direct driver stops after analysis; it does
not execute the firmware.

The three fresh inventory builds took 14.5, 18.5 and 15.3 seconds, run concurrently. Verification
of the 32 already-built units took about 11 seconds on an Apple M3 Pro. Many roots return UNKNOWN
before executing their bodies, so this is not an estimate for full-firmware proof time. Larger
supported symbolic roots can cost much more.

The [machine-readable summary](https://hxyulin.github.io/mir-check/data/fleet-survey.json)
records per-unit counts and the source classification. Raw solver reports and build outputs
remain local measurement artifacts. No
entire workspace passed verification; every scope still contains unsupported or refuted roots.
