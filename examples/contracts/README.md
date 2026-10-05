# Contract example

A no_std library with metadata-only contracts. The checker proves guarded byte reads and a
bounded increment, including caller preconditions and the increment's return-value bound.

After building the root workspace and installing the solver, run from the repository root:

```sh
target/debug/cargo-mir-checker --verify --manifest-path examples/contracts/Cargo.toml --lib --locked
```

Changing guarded_increment's condition from value < 15 to value <= 15 fails verification:
value=15 violates bounded_increment's precondition. Ordinary Cargo builds still evaluate no
contract predicates and add no runtime checks.
