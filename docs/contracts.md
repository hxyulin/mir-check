<!--@include: ../crates/mir-contracts/README.md-->

## A bounded call

Add the metadata crate to the code you want to annotate:

```toml
[dependencies]
mir-contracts = { git = "https://github.com/hxyulin/mir-check" }
```

```rust
use mir_contracts::{ensures, no_panic, requires};

#[no_panic]
#[requires(value < 15)]
#[ensures(result < 16)]
pub fn increment(value: u8) -> u8 {
    value + 1
}
```

`requires` constrains the root's input domain and must be proved at every analyzed call.
`ensures` is checked at each feasible return. Neither attribute provides runtime protection
against a caller violating the domain. Panic checking itself does not require annotations.

See the [contract example](examples.md#contracts-and-nested-inputs) and
[proof execution](proofs.md) for caller obligations and the trusted components.

Postconditions can inspect supported updated arguments through `final_<parameter>`, while ordinary
parameter names retain entry snapshots:

```rust
#[requires(state.count < 255)]
#[ensures(final_state.count > state.count)]
fn increment(state: &mut State) {
    state.count += 1;
}
```

The final_ prefix is reserved for these post-state bindings in functions with postconditions.
