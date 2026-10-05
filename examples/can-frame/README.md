# Vendored CAN frame slice

Frame and FdFrame are copied from fleet-2027/shared/can-frame/src/lib.rs at commit
b81a247a3295b13553087f5e328f201944a1eb61. The source is MIT OR Apache-2.0, matching this repository.
The first excerpt stops before Use and the bus-ID checker. A second excerpt in bus_upstream.rs
preserves Use, both helpers and check; bus.rs adds annotations and verification harnesses.
src/upstream.rs is the unmodified excerpt. src/lib.rs adds metadata contracts;
the six method bodies remain unchanged and a regression test compares them to the snapshot.

The fixture is a separate no_std crate. Only this vendored crate depends on mir-contracts.
The original firmware repository is neither modified nor required to run these tests.

Constructor contracts request valid IDs and capacity bounds for Some returns, exact stored
payload lengths and invalid-input rejection for None returns. Accessors declare their length
preconditions explicitly: those are conditional proofs, not an automatically inferred invariant
for every value of the type. Callers must establish the bounds from actual constructor behavior.

All six methods prove on aarch64-apple-darwin and thumbv7em-none-eabihf, using the pinned compiler,
panic=abort and overflow checks enabled. Both constructors need no entry preconditions. Their
postconditions establish valid IDs, accepted classic/FD payload lengths, exact stored lengths and
invalid-input rejection. ID getters return the stored ID. Data accessors prove safe slicing and
returned lengths under their explicit capacity preconditions.

classic_payload_round_trip and fd_payload_round_trip are additional verification harnesses. They
restrict inputs to accepted IDs/lengths and a valid index, call the original constructor and
accessor bodies, and assert that the returned byte equals the input byte. The analyzer proves the
accessor precondition at each call and payload equality for all permitted bytes. These harnesses
contain ordinary runtime assertions; the attributes themselves still add no runtime checks.

classic_bad_length and fd_bad_length intentionally construct invalid stored lengths. Verification
rejects both at the accessor call with a failed precondition and solver model. Removing an accessor
precondition also fails: arbitrary struct inputs include invalid lengths despite field privacy.
Mutation tests reject weakened constructor guards, changed stored lengths, invalid FD lengths and
incorrect or missing copies. Runtime tests independently exercise IDs, lengths zero through 65
and the payload harnesses for every accepted byte.

The analysis trusts pinned core models for byte prefix indexing, slice length, u8-to-usize
conversion and exact byte copies into owned local arrays. Reports list those models. General
mutable aliases and some derived implementations remain unsupported. Available dependency bodies
can now be interpreted, subject to the same MIR coverage and execution limits. Finite
loops can prove only when every feasible path finishes within the execution budget.
These are selected-root proofs, not a proof of every function or caller in the original crate.

From the mir-check repository root:

```sh
target/debug/cargo-mir-check --verify --summary --entry Frame::new --entry Frame::data \
  --manifest-path examples/can-frame/Cargo.toml --lib --locked --target thumbv7em-none-eabihf
cargo test --locked -p mir-check --test compiler \
  vendored_constructors_accessors_and_payload_round_trips_prove_on_host_and_arm
cargo test --locked --manifest-path examples/can-frame/Cargo.toml
cargo build --locked --release --manifest-path examples/can-frame/Cargo.toml \
  --target thumbv7em-none-eabihf
```

Compiler integration tests locate the pinned proc-macro artifact and select proof roots directly.
Without --entry, full-crate cargo mir-check --verify is expected to fail because it selects the bad
call-bound harnesses and unsupported derived methods too.

The bus validator's unchanged body contains nested loops, enum matches, two helper calls and
assertions for ID validity, slot validity, collisions and FD compatibility. It has 44 MIR blocks
in the recorded analysis build. shared_bus proves all classic three-device configurations with
one exclusive ID and two distinct slots on another ID. fd_bus proves three distinct IDs containing
a classic frame, a shared slot and an FD frame, with every device FD tolerant. IDs and slots remain
symbolic; the harness requires the stated validity, distinctness and compatibility conditions.

Both configuration families prove on the host and thumbv7em-none-eabihf. Five invalid families
produce solver models: duplicate slots, out-of-range slots, duplicate frame IDs, out-of-range IDs
and an FD-intolerant device sharing a bus with an FD frame. Runtime tests replay each failure in
the original validator and exercise representative accepted IDs and all four slots.

These results do not prove check for an arbitrary &[Use]. Root inputs of that type are still
unsupported. The engine follows constructed local variants and small arrays, with each loop index
uniquely determined on its path. The loop must completely finish; exceeding the 256-block budget
returns UNKNOWN. A narrow trusted model for static formatting arguments permits the validator's
literal panic messages; dynamic formatting and user formatters remain unsupported.

```sh
cargo test --locked -p mir-check --test compiler \
  nested_bus_loops_prove_for_symbolic_ids_and_slots_on_host_and_arm
cargo test --locked -p mir-check --test compiler \
  invalid_bus_ids_slots_collisions_and_fd_compatibility_are_refuted
```
