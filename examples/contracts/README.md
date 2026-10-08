# Contract example

A no_std library with metadata-only contracts. The checker proves guarded byte reads and a
bounded increment, including caller preconditions and the increment's return-value bound.
guarded_packet_read accepts a shared Packet containing a nested Header and shared byte slice. It
returns Some exactly when the header is enabled and the index is valid, proves the read callee's
precondition and otherwise returns None. Nested fields are arbitrary root inputs; the proof needs
no implicit constructor invariant.

After building the root workspace and installing the solver, run from the repository root:

```sh
target/debug/cargo-miren --verify --manifest-path examples/contracts/Cargo.toml --lib --locked
target/debug/cargo-miren --verify --summary --entry guarded_packet_read \
  --manifest-path examples/contracts/Cargo.toml --lib --locked --target thumbv7em-none-eabihf
```

Changing guarded_increment's condition from value < 15 to value <= 15 fails verification:
value=15 violates bounded_increment's precondition. Ordinary Cargo builds still evaluate no
contract predicates and add no runtime checks.

Changing the packet guard from index < bytes.len() to index <= bytes.len() permits an invalid call
when the index equals the length. Aggregate-input compiler tests check this failure on host and ARM.
See [usage](../../docs/usage.md) for selected-root scope and
[coverage](../../docs/coverage.md) for the supported input shapes.
