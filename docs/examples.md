# Real-code examples

The fixtures are independent no_std crates. Vendored method bodies retain their original source
tokens, checked by regression tests. Only these fixtures add a metadata dependency; the fleet
firmware stays unchanged. Positive proofs, deliberate mutations and runtime replays provide
separate evidence. They do not establish a whole-crate proof or complete the interpreter audit.

## DR16 parsing

`Raw::parse` accepts an arbitrary valid byte slice without entry preconditions. The fixture
checks panic freedom, exact 18-byte acceptance, switch bounds and five decoded channel bounds.
The analyzer follows the actual closures, core Result/Option bodies and question-mark paths.

```sh
target/debug/miren --manifest-path examples/dr16/Cargo.toml
```

Bad byte-index and channel-mask mutations are refuted. Independent decoding formulas cover
4,608 runtime frames. Read the
[fixture and provenance](https://github.com/hxyulin/miren/tree/main/examples/dr16).

## CAN frames and bus configuration

The frame constructors establish accepted IDs, capacities and stored payload lengths. Accessor
proofs use explicit capacity preconditions. Two payload round-trip harnesses check those bounds
at the actual call and preserve every accepted input byte.

```sh
target/debug/miren --entry Frame::new --entry Frame::data \
  --manifest-path examples/can-frame/Cargo.toml --lib --locked --target thumbv7em-none-eabihf
```

The bus validator's nested loops prove two symbolic three-device configuration families. Invalid
IDs, slots, collisions and FD compatibility produce refutations and confirmed runtime failures.
The arbitrary `&[Use]` entry remains unsupported. Full-crate verification intentionally fails
because the fixture also contains negative harnesses and unsupported derived methods.

Read the
[fixture and provenance](https://github.com/hxyulin/miren/tree/main/examples/can-frame).

## Contracts and nested inputs

`guarded_packet_read` takes a shared packet with a nested header and byte slice. It proves the
read callee's index precondition and returns Some exactly when the header is enabled and the
index is valid. Fields remain arbitrary symbolic inputs; there is no constructor invariant.

```sh
target/debug/miren --entry guarded_packet_read \
  --manifest-path examples/contracts/Cargo.toml --lib --locked --target thumbv7em-none-eabihf
```

Changing `<` to `<=` in the guard permits an invalid call and fails verification. Read the
[fixture](https://github.com/hxyulin/miren/tree/main/examples/contracts),
[contract guide](contracts.md) and [coverage matrix](coverage.md).
