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

Build the fixture with Cargo from this directory. Compiler integration tests select methods and
proof harnesses directly; full-crate verification also includes unsupported derived methods.
