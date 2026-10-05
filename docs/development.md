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
