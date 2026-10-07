# Development and documentation

mir-check is a standalone host tool. Its compiler adapter requires the exact nightly in
`rust-toolchain.toml`, including rustc-dev, LLVM tools and rust-src. Install the pinned solver
before running compiler integration tests:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements-solver.txt
cargo build --workspace --locked
```

## Check a change

```sh
cargo fmt --all --check
cargo lint
cargo test --workspace --locked
cargo build --workspace --release --locked
cargo deny check
prek run --all-files --stage manual
```

The compiler tests include positive, negative and unknown outcomes. Source mutations must fail
the corresponding proof. Fixture runtime tests independently replay selected failures and compare
decoding formulas. GitHub Actions runs the proof suite on Linux and macOS, including ARM proofs.

When changing fixtures, also run their formatting, Clippy, host tests and ARM release builds.
See the [repository conventions](https://github.com/hxyulin/mir-check/blob/main/AGENTS.md).

## Analyzer design priorities

The [analyzer redesign](analyzer-redesign.md) records implemented foundations, remaining migrations
and completion criteria for storage, calls, query construction and counterexample validation.

The typed MIR adapter, interned SMT terms and persistent Z3 text interface already provide the
intended foundations. Keep unsupported behavior explicit while replacing narrowly scoped models
with common semantics where the current representations lose necessary facts.

- Unify typed storage locations and projections across tracked references, static views and
  provenance-backed pointers. Keep numeric pointer handles separate: a matching address or layout
  must not create allocation provenance. A location needs allocation identity, compiler type and
  layout, offset, access capability, initialization state and effect invalidation. Build supported
  typed operations rather than a general byte-level pointer interpreter.
- Share immutable aggregate shapes and values across branch states, using copies of changed paths
  for writes. Current Value and State clones copy owned field vectors and names recursively even
  though SMT leaves are already shared. Preserve distinct allocation identity for owned copies,
  independent contract entry snapshots and path-local mutation; shared host representation must
  not introduce Rust aliases. Compiler type identities should describe shapes instead of repeated
  display names.
- Give atomic operations storage identity before adding precise history. Preserve constructor
  values and writes only where exclusivity or an explicit environment model justifies it. Shared
  or escaped storage keeps conservative interference. Fence calls alone cannot establish
  exclusivity. Overlapping atomic views need a common footprint model, not independent histories.
- Share compiler-identified call descriptions and operation semantics between ordinary execution
  and induction. The backends still need different control-flow encodings. Keep configured
  contracts ahead of library models and preserve UNKNOWN for unsupported operations or shims.
- Separate reference validation from contract snapshot construction before avoiding unnecessary
  copies. Resolve a concrete call once, cache its derived description and check contracts at every
  actual call. Existing rustc queries already cache compiler metadata; measure additional caches
  rather than duplicating them indiscriminately.
- Cache derived term dependencies and query assembly separately from solver decisions. Current
  exact-query lookup happens after rendering, while symbol and floating-point dependency walks
  revisit immutable DAGs. A structural query key must include the analysis context, all relevant
  latent encodings and configured limits. Keep full query validation and report scripts, including
  when a cached decision avoids contacting Z3.
- Track the abstraction choices that contribute to a failing path and validate counterexamples
  independently. Begin replay with supported scalar and owned-array roots. Shared atomic state,
  hardware, escaped references and arithmetic NaN payloads need their own validation rules; an
  unconfirmed model must not be presented as an observed Rust failure.

Typed initialization tracking should precede general MaybeUninit payload reads and retained static
future polling. Stores must establish facts for the correct location, and unknown effects must
invalidate them while retaining reference-escape evidence. Type invariants likewise need complete
construction and mutation hooks before being advertised as verified type-wide guarantees.

Verified callee summaries remain a separate policy decision. They require a proved contract,
complete frame/effect information and invalidation tied to the compiler, target, build options,
dependencies and configuration. Current contracts continue to execute callee bodies. Z3 remains
the solver, and arbitrary raw-pointer memory, whole weak-memory verification and a new solver
implementation remain outside this design.

## Work on the docs

The site uses VitePress with local search and light/dark themes. Guides live directly under
`docs/`; the contracts page includes the metadata crate's README. There is no second copy of
the coverage or proof guides. Node.js 22 or newer is needed only for the documentation site.
The package override pins patched Vite 6.4.3 beneath stable VitePress 1.6.4; keep the production
build and theme preview checks when updating that combination.

```sh
npm ci
npm run docs:dev
npm run docs:build
npm run docs:preview
```

Open the local server at `/mir-check/`. Production builds check internal Markdown links.
Keep prose within 100 columns, except tables, and review the home page and guides on desktop
and mobile in both themes before publishing layout changes.

## Publish the docs

The Documentation workflow builds pull requests and pushes to main. Pull requests produce a
build artifact; pushes to main deploy that artifact to GitHub Pages. Deployment uses the
`github-pages` environment, with Pages write and OIDC permissions confined to the deploy job.

The repository's Pages source is GitHub Actions. The public address is
[hxyulin.github.io/mir-check](https://hxyulin.github.io/mir-check/). The workflow needs no Rust
compiler or Z3 because the site renders checked-in guides rather than running proofs.

When adding analyzer behavior, update the [coverage matrix](coverage.md) and relevant examples
with concrete evidence. Never label an unsupported operation, missing body or partial path
exploration as a successful proof.
