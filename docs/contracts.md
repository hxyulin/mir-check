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

## Contracts without a source dependency

A JSON sidecar can attach the same checked contracts to unchanged application or dependency code.
For example, this checks the final state of the fleet controller's reset method:

```json
{
  "schema_version": 1,
  "functions": [
    {
      "function": "controller::Pid::reset",
      "arguments": ["state"],
      "no_panic": true,
      "ensures": ["final_state.integral == 0.0"]
    }
  ]
}
```

```sh
cargo mir-check --verify --summary --contracts contracts.json \
  --entry controller::Pid::reset -p controller --lib --target thumbv7em-none-eabihf
```

Function paths must be exact and crate-qualified. Optional argument names are positional aliases;
otherwise available MIR parameter names are used. Aliases cannot conflict with another parameter's
name. Checked preconditions restrict root inputs and become obligations at callers. Checked
postconditions are verified against actual returns and final state. The analyzer executes the
body, including when the sidecar names a function normally handled by a library model.
Nothing is added to the application binary.

Unknown fields, duplicate selectors, malformed predicates and unmatched selectors fail the run.
In Cargo mode matching is checked across the collected reports. A local definition can match
without being selected, but its contracts remain pending until an independent proof succeeds.
A dependency selector must match an analyzed call if its definition has no local inventory.

## Explicitly trusted call boundaries

An opaque formatter or another unsupported boundary can have a user-supplied summary. The tool
checks its caller's preconditions, then assumes the declared panic freedom, effects and return
constraints. This is an explicit exception to body verification. The following illustrates the
configuration shape; its claims need a separate review of the application's write_float function:

```json
{
  "schema_version": 1,
  "functions": [
    {
      "function": "application::write_float",
      "arguments": ["writer", "value", "precision"],
      "requires": ["precision <= 6"],
      "trusted": true,
      "no_panic": true,
      "modifies": ["writer"],
      "ensures": ["final_writer.len <= 32"],
      "reason": "Application review reference for this formatter and writer boundary"
    }
  ]
}
```

This example does not establish that precision <= 6 makes arbitrary float formatting panic-free,
or that the writer's length stays bounded. Those are user claims. Prefer a small application
wrapper whose exact behavior can be reviewed, rather than a blanket core::fmt exception.

Every root that uses a trusted summary is labeled **PROVED_WITH_ASSUMPTIONS**, counted separately
from PROVED, and fails strict verification. Accept these conditional results explicitly with:

```sh
cargo mir-check --verify --summary --contracts contracts.json --allow-assumptions --lib
```

The flag never accepts REFUTED or UNKNOWN. Selecting the trusted function itself as a root still
executes its actual body. An assumption about a callee cannot verify the callee's implementation.
Reports retain the full configuration and each used summary's clauses, reason, concrete compiler
instance, available crate hash and call-site source. The hash is recorded provenance, not a
configured version pin. A trusted function requires no_panic=true and a nonempty reason.
Some local binary builds do not compute a compiler crate hash. Reports omit that hash and the
verbose output explicitly says it is unavailable; the assumption is still recorded.

The return value is fresh within its supported Rust type. A fmt::Result can still be Err: assuming
no panic does not assume success, and a caller that panics on Err can be refuted. Trusted ensures
restrict the fresh return/state; ill-typed or inconsistent clauses yield UNKNOWN. Reference,
pointer, callable, destructor-bearing and alias-containing return shapes require further models.

A trusted boundary returning its mutable-reference argument can declare `returns_alias`, naming
one of its positional argument aliases. The input and output must both be mutable references to
the same pointee type; only the outer lifetime can differ. The returned value keeps the original
tracked allocation and field/index projection. For example, `"arguments": ["storage"]`,
`"returns_alias": "storage"` and `"modifies": []` claim that the function returns that reference
without modifying modeled storage. This is an explicit alias claim, not a fresh symbolic result.
An ensures predicate's `result` reads the returned pointee after the claimed effects.

Missing effects still invalidate storage, so such an alias cannot be used to retain stale facts.
Dead storage, escaping local mutable borrows, different pointee types, shared references, raw
pointers and aggregate alias returns remain unsupported. Returning an alias does not verify
Rust lifetime validity or make a local allocation live forever. It never verifies the assumed
function's implementation; selecting that function as a root still checks its body.

Effects are also explicit claims. Omitting modifies invalidates all modeled storage facts after
the call. Setting modifies to an empty array claims that no modeled storage changes. Listing an
argument permits arbitrary supported writes through that reference, preserves its known aliases,
and claims that other modeled storage is unchanged. A mutable byte slice retains its length;
scalar Cell references can also be listed. `final_<alias>` reads the resulting modeled state.
Unsupported effect shapes yield UNKNOWN. These frames do not verify general concurrent memory
or arbitrary raw-pointer effects.

Concrete synchronous guard destructors execute in the ordinary interpreter. A guard that invokes
a configured restoration leaf therefore checks that leaf's preconditions and records its trusted
contract, just like an explicit call. The callback and destructor body are still checked. Such
leaf assumptions establish only the reported conditional panic-freedom claim; they do not verify
interrupt exclusion or concurrent access. Unsupported drop glue remains UNKNOWN.

Trusted generic calls require an exact instance string, copied from the report's compiler
instance arguments, such as "[u8]". Matching uses the pinned compiler's representation and rejects
overlapping selectors. Trait resolution must still identify an ordinary concrete function;
dynamic dispatch and unsupported shims cannot be covered by a wildcard.
