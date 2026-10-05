# Vendored CAN frame slice

Frame and FdFrame are copied from fleet-2027/shared/can-frame/src/lib.rs at commit
b81a247a3295b13553087f5e328f201944a1eb61. The source is MIT OR Apache-2.0, matching this repository.
The excerpt stops before Use and the bus-ID checker; those loop and enum properties are outside
this experiment. src/upstream.rs is the unmodified excerpt. src/lib.rs adds metadata contracts;
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
mutable aliases, loops, derived implementations and arbitrary dependencies remain unsupported.
These are selected-root proofs, not a proof of every function or caller in the original crate.

From the mir-checker repository root:

```sh
cargo test --locked -p mir-checker --test compiler \
  vendored_constructors_accessors_and_payload_round_trips_prove_on_host_and_arm
cargo test --locked --manifest-path examples/can-frame/Cargo.toml
cargo build --locked --release --manifest-path examples/can-frame/Cargo.toml \
  --target thumbv7em-none-eabihf
```

Compiler integration tests locate the pinned proc-macro artifact and select proof roots directly.
Full-crate cargo mir-checker --verify is expected to fail because it selects the deliberately bad
call-bound harnesses and unsupported derived methods too.
