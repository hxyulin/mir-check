# Vendored DR16 parser

Raw and Raw::parse are copied from fleet-2027/shared/devices/src/dr16.rs at commit
b81a247a3295b13553087f5e328f201944a1eb61. The source is MIT OR Apache-2.0, matching this repository.
src/upstream.rs retains the original excerpt. src/lib.rs adds metadata contracts; the parser body
is unchanged, and a compiler integration test compares its tokens with the snapshot.

This is a separate no_std crate with unsafe code forbidden. Only the fixture depends on
mir-contracts; the original firmware workspace has no new dependency or source changes.

Raw::parse proves on aarch64-apple-darwin and thumbv7em-none-eabihf with the pinned compiler,
panic=abort and overflow checks enabled. The recorded body has 42 MIR blocks. No entry precondition
is needed: any valid byte slice is accepted as input, including arbitrary byte contents and length.
The proof establishes panic freedom, Some only for length 18, None only for other lengths, switch
values at most three, and every decoded channel within -1024 through 1023. These wire bounds do
not imply that a remote-control frame meets the later application validity rules.

The interpreter follows Result::ok, Option's Try/FromResidual implementations and all three
closure bodies. Explicit core models implement exact slice-to-array conversion, lossless integer
conversion, integer endian decoding and fixed-array map. Map executes the actual callable MIR for
each element. The proof report distinguishes interpreted bodies from these trusted models.

Regression tests change u(17) to u(18), introducing an out-of-range byte access, and replace the
11-bit channel mask with a 16-bit mask, breaking the channel bounds. Both mutations are refuted
with solver models. Independent host tests check lengths zero through 40 and compare 4,608 frames
against separate packed-channel and field-decoding formulas: every byte position takes all 256
values while the other bytes are zero. Runtime samples supplement the universal symbolic proof.

From the mir-check repository root:

```sh
cargo test --locked -p mir-check --test compiler \
  the_dr16_parser_proves_without_entry_bounds_on_host_and_arm
cargo test --locked -p mir-check --test compiler \
  incorrect_dr16_indices_and_channel_masks_are_rejected
cargo test --locked --manifest-path examples/dr16/Cargo.toml
cargo build --locked --release --manifest-path examples/dr16/Cargo.toml \
  --target thumbv7em-none-eabihf
```

This fixture does not include Dr16::from_raw or its floating-point normalization and validity
checks. Derived implementations are not selected for proof. Arbitrary enum inputs, general
mutation, mutable captures and missing dependency MIR remain coverage gaps. These results apply
to the selected parser and recorded build, not every function in the original devices crate.
