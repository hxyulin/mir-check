---
layout: home

hero:
  name: mir-check
  text: Check the paths that can panic.
  tagline: Symbolic execution of Rust MIR, with contracts that add no runtime checks.
  image:
    light: /mark.svg
    dark: /mark-dark.svg
    alt: Branching MIR blocks with a check mark
  actions:
    - theme: brand
      text: Run your first proof
      link: /usage
    - theme: alt
      text: See what is supported
      link: /coverage
    - theme: alt
      text: View on GitHub
      link: https://github.com/hxyulin/mir-check

features:
  - title: Follow actual calls
    details: Execute available callee MIR with symbolic arguments and check the bounds at each call.
  - title: Declare static contracts
    details: Verify preconditions and postconditions without injecting runtime assertions.
  - title: Analyze embedded builds
    details: Run on the host and check no_std code for the ARM target, with recorded compiler flags.
  - title: Keep gaps visible
    details: Unsupported operations and unfinished paths return UNKNOWN and fail verification.
---

## A real parser, arbitrary input

The unchanged DR16 parser fixture proves panic freedom for every valid input byte slice. Its
contracts also check exact-length acceptance and decoded channel bounds. The tests run on the
host and `thumbv7em-none-eabihf`.

```sh
git clone https://github.com/hxyulin/mir-check.git
cd mir-check
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements-solver.txt
cargo build --workspace --locked
target/debug/cargo-mir-check --verify --summary --entry Raw::parse \
  --manifest-path examples/dr16/Cargo.toml --lib --locked
```

Read the [examples](examples.md), [usage guide](usage.md) and [proof explanation](proofs.md).
The [fleet survey](fleet-survey.md) measures the current checker against unchanged firmware.

::: warning Experimental side project
mir-check supports a limited Rust/MIR subset and requires a pinned nightly compiler. The
interpreter and its library models have not completed a soundness audit. A selected-root proof
is conditional on its input domain and build configuration; it is not a whole-firmware guarantee.
:::
