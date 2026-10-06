# Fleet workspace survey

On 2026-10-06, mir-check checked the unchanged fleet-2027 shared libraries, DM-MC02 board/robot
workspace and RM-C board workspace for `thumbv7em-none-eabihf`. After adding floating point,
symbolic enum inputs, dependency-defined inputs, array patterns, dependency MIR retention and
aggregate constants, it proves 293 of 976 function roots, up from 25 in the original baseline.
It cannot yet prove entire mutable control loops or firmware applications.

## Coverage change

All four scans use the same 32 ARM compilation units, firmware, pinned compiler, root profiles
and source-span classification. The first expansion adds input/operation support. Dependency
retention then rebuilds Cargo dependencies with ordinary bodies encoded at MIR optimization level
zero. Their metadata paths and retention flags differ; root MIR optimization remains level zero.
The constant expansion reuses the dependency-retaining inventory and compiler arguments.

| Function declarations | Original (`07f1412`) | Input expansion (`af829e4`) | Dependency retention (`0dc7551`) | Constants (`55d9dd3`) |
| --- | ---: | ---: | ---: | ---: |
| PROVED | 25 | 149 | 196 | 293 |
| REFUTED | 25 | 46 | 71 | 74 |
| UNKNOWN | 926 | 781 | 709 | 609 |
| Total | 976 | 976 | 976 | 976 |
| Proved fraction | 2.6% | 15.3% | 20.1% | 30.0% |

Dependency retention turns 47 previous unknowns into proofs and 25 into refutations. All 149
previously proved and all 46 previously refuted function roots retain their outcomes. Aggregate
constants then add 97 proofs and three refutations, preserving all 196 proved and 71 refuted
outcomes. Across the expansions, 268 original unknowns now prove and 49 now refute.

These are root-domain proofs, not line coverage or the fraction of firmware safety established.
The original and intermediate measurements remain available:

- [Original baseline data](https://hxyulin.github.io/mir-check/data/fleet-survey.json).
- [Input expansion data](https://hxyulin.github.io/mir-check/data/fleet-survey-expanded.json).
- [Retained MIR data](https://hxyulin.github.io/mir-check/data/fleet-survey-dependency-mir.json).
- [Aggregate constant data](https://hxyulin.github.io/mir-check/data/fleet-survey-constants.json).

The new data records per-unit counts, outcome transitions and the first reported unknown reasons.

## Scope and configuration

- Firmware revision: `b81a247a3295b13553087f5e328f201944a1eb61`.
- Analyzer implementation: `55d9dd3`, JSON schema 7; previous `0dc7551`, `af829e4`,
  original `07f1412`.
- Compiler: nightly-2026-09-22, rustc commit `1303417c416e1595173d9689e7394c31e136ae95`.
- Target: `thumbv7em-none-eabihf`, panic strategy abort, overflow checks enabled.
- Profiles: shared workspace dev; board workspaces dev with their configured optimization level 1.
- Cargo defaults and locked dependencies; shared workspace excludes the host-only xtask.
- Cargo dependencies rebuilt with always-encode-mir enabled and MIR optimization level zero.
- Prebuilt sysroot libraries are not rebuilt; dependency roots are not independently inventoried.
- No firmware annotations, new dependencies, source changes or unchecked contract assumptions.

These are development-profile analysis results under the checker's pinned compiler, not proofs
of the stable compiler's release binaries. Inventory ran from each workspace root with the target
explicitly supplied. Firmware-directory runner/linker/CPU flags were not selected; those flags
and release overflow settings require separate measurements. MIR optimization remains at the
checker's level 0.

## All inventoried roots

| Workspace scope | ARM compilation units | MIR roots | PROVED | REFUTED | UNKNOWN |
| --- | ---: | ---: | ---: | ---: | ---: |
| Shared libraries | 7 | 1,087 | 522 | 47 | 518 |
| DM-MC02 board, bring-up and ten robot firmwares | 23 | 1,192 | 291 | 27 | 874 |
| RM-C board and bring-up | 2 | 57 | 2 | 0 | 55 |
| Total | 32 | 2,336 | 815 | 74 | 1,447 |

These are independently checked roots, including closures, derives and macro-generated methods.
Host build scripts and third-party dependency roots are excluded. Shared path dependencies are
inventoried in their own workspace rather than counted again for each board. Their independently
selected root results therefore use the shared workspace's default features; board-specific
dependency features can differ. An available dependency body is still executed with actual
arguments when a board root reaches it.

The raw success rate is 34.9%, but that overstates useful application coverage. For example,
zerocopy generates many empty marker methods that prove trivially.

## Function declarations without closures or derives

To expose that skew, this table counts roots whose reported source line contains a function
declaration, excluding names containing `{closure#`. This is a source-span filter, not a full
AST classification. It includes handwritten trait methods and instantiated methods emitted from
local function-declaring macros. Separate compilation units remain separate roots.

| Scope | Function roots | PROVED | REFUTED | UNKNOWN |
| --- | ---: | ---: | ---: | ---: |
| Shared libraries | 436 | 108 | 47 | 281 |
| DM-MC02 workspace | 523 | 185 | 27 | 311 |
| RM-C workspace | 17 | 0 | 0 | 17 |
| Total | 976 | 293 | 74 | 609 |

The current success rate for this subset is 30.0%. It is a count of function roots under arbitrary
supported inputs, not line coverage, runtime branch coverage or a fraction of firmware safety.

| Shared crate | Function roots | PROVED | REFUTED | UNKNOWN |
| --- | ---: | ---: | ---: | ---: |
| attitude | 19 | 5 | 0 | 14 |
| can-frame | 9 | 6 | 2 | 1 |
| controller | 20 | 5 | 0 | 15 |
| devices | 216 | 60 | 44 | 112 |
| link | 11 | 4 | 1 | 6 |
| subsystems | 156 | 28 | 0 | 128 |
| validate | 5 | 0 | 0 | 5 |

## Useful positive results

The actual `devices::dr16::Raw::parse` proves with 43 obligations, seven interpreted bodies and
no declared entry assumptions. Every valid byte slice is in its domain. This establishes panic
freedom; the original function has no annotations asking for decoded-value postconditions.
The separate [vendored fixture](examples.md#dr16-parsing) adds and verifies those postconditions.

Other passes include both CAN constructors and ID getters, `dr16::Switch::from_wire`,
`link::EngineerMode::from_wire`, `link::Flags::from_bits`, the link bit helper, several integer-only
motor constructors and small state/default helpers. The expanded scan adds PID/filter
constructors, IMU temperature conversion, RPM/radian conversions, motor feedback state readers,
attitude getters and subsystem validation/state helpers.

Board and robot passes now include the four gimbal boards' `shooter::update` functions, the
engineer lower board's `chassis::mecanum`, hardware velocity readers, gimbal deadbands and several
leg/mode readers. The shooter policy accepts arbitrary supported `Option` target/remote values,
checks the remote switch, maps two RPM values and returns spin or stop. The result establishes
panic freedom; the unannotated function has no policy postcondition to verify. These passes
require floating point, enum payloads and types defined in shared dependency crates. They do not
prove the mutable PID updates that consume the returned commands. Existing `Gripper::new`, its
handwritten default implementation and `Text::is_empty` passes remain.

Retained dependency bodies add 47 function passes, all in board/robot workspace units. These include
`arm::pid`, `head::cascade` and `head::pid` on the engineer upper board, chassis/gimbal/shooter PID
configuration helpers across the robots, CAN bus constructors and `wired`/supercap readers,
`UserIo::open` and flash instruction construction. They execute actual shared constructor bodies; no
helper annotation or previous independent proof is used as a trusted call summary. The PID
configuration helpers construct controllers; they do not execute mutable controller updates.

Aggregate constants add 24 shared and 73 board/robot proofs. New shared passes include 20 motor
feedback accessors, `link::from_raw` and three balance-state `done` helpers. Board passes include
engineer upper `arm::command` and `Head::new`, gimbal/shooter controller constructors, shooter
target selection, CAN bus readers and hardware joint/wheel helpers. They execute actual retained
bodies with decoded constant fields; the engine does not infer a library summary.

A fresh Cargo verification of hero chassis `gimbal::controllers` and `shooter::target` also proves
both selected roots on ARM, interpreting eight distinct instances across their reports. This
checks the ordinary Cargo path as well as the survey's direct-driver replay.

The constructed CAN payload and bus-configuration fixture proofs remain stronger targeted
examples than whole-crate scanning: a caller supplies useful relationships that an arbitrary
independent struct input does not have.

## What refutations mean here

The 74 refuted roots are failures over their unconstrained root domains, not 74 confirmed
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

Dependency retention exposes 25 additional refutations: seven shared roots and eighteen
board/robot roots. Most board cases reach motor-ID arithmetic through CAN bus `uses` methods with
arbitrary stored IDs. Shared decoder cases reach byte-prefix bounds with arbitrary frame lengths.
These are additional caller/type-invariant assumptions to check, not confirmed runtime failures.

Constants expose three additional bounds refutations in engineer lower `hardware::joint`,
engineer upper `hardware::joint` and `hardware::joint_current_a`. Their root domains include
arbitrary array indices; useful caller guarantees need explicit accepted-index bounds.

Solver assignments concern translated obligations. Runtime replay or caller analysis is needed
before treating one as a reachable defect. The survey did not modify firmware or add replay tests.

## Remaining blockers

For each unknown function root, the following table classifies its first unknown obligation in
report order. This gives mutually exclusive counts, rather than adding every gap reported for a
root. It is not an exhaustive list of a function's needs.

| First reported blocker | Original roots | Input expansion | Dependency retention | Constants |
| --- | ---: | ---: | ---: | ---: |
| Mutable inputs | 263 | 300 | 300 | 300 |
| Floating-point inputs | 159 | 0 | 0 | 0 |
| Other input shapes | 378 | 25 | 25 | 25 |
| Input shape limits | 4 | 44 | 44 | 44 |
| Missing dependency MIR | 34 | 237 | 16 | 17 |
| Constants | 61 | 108 | 194 | 69 |
| Other MIR/library/solver behavior | 26 | 66 | 129 | 147 |
| Execution limits | 1 | 1 | 1 | 7 |
| Total UNKNOWN | 926 | 781 | 709 | 609 |

Counts can rise when an earlier blocker disappears. There are no previous proved/refuted roots
that regressed to unknown. The main remaining work is:

- **Mutable state:** inputs such as `&mut Pid`, balance components, operator state and hardware
  buses need writes and alias relationships to be modeled. A useful first step is owned state or
  a single nonescaping mutable receiver with explicit alias limits. Treating it as a read-only
  snapshot would not prove the real update.
- **Remaining missing bodies:** eleven roots first stop at prebuilt core's `option::unwrap_failed`,
  five at the platform critical-section foreign acquire function, and one at `core::convert::From`.
  Cargo retention cannot supply foreign implementations or bodies omitted from prebuilt sysroot
  metadata. A missing body remains unknown, even when its name suggests a panic helper.
- **Constants and mutable storage:** the 54 `Option::as_ref` first blockers are resolved. Of 69
  remaining constant first blockers, 51 reject references to interior mutable storage, often
  `validate::Site` atomic counters. Nine reach MaybeUninit, six exceed array limits, two involve
  raw pointers and one is a closure constant. These require additional semantics or explicit
  budgets; uninitialized storage cannot be replaced with initialized values.
- **Input limits and pointer/effect operations:** larger arrays or nested hardware shapes exceed
  current budgets. Raw pointers, slice iterators, projected writes, atomics and async machinery
  need additional models. Raising a budget alone does not solve missing semantics.

Floating-point remainder, bit observation and some math functions remain unsupported despite
float inputs now being modeled. Type invariants and verified caller bounds would also turn
several unconstrained root refutations into meaningful conditional checks. Loop invariants and
hardware/async effects remain separate work.

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

The dependency-retaining inventory builds took 13.5, 17.5 and 14.7 seconds, run concurrently.
Verification of the 32 already-built units took about 11 seconds in the original baseline,
35.4 seconds after input expansion, 63.8 seconds with retained dependency bodies, and 81.3 seconds
with aggregate constants on an Apple M3 Pro. The constant scan reuses the retained inventory.
The later runs execute more paths that previously stopped at input construction or calls.
Many roots still return UNKNOWN early, so this is not an estimate for full-firmware proof time.
Larger supported symbolic roots can cost much more.

The [latest JSON summary](https://hxyulin.github.io/mir-check/data/fleet-survey-constants.json)
records per-unit counts, source classification, changed roots and first-gap reasons. Raw solver
reports and build outputs remain local measurement artifacts. No entire workspace passed
verification; every scope still contains unsupported or refuted roots.
