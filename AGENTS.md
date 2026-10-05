# Development

This is a standalone host tool, independent of fleet-2027. Start with README.md.

- Every crate forbids unsafe code. Host tooling may use std and allocation.
- Follow standard Rust naming and cargo fmt. Keep code and prose within 100 columns.
- Fix Clippy warnings rather than suppressing them.
- Match project enums exhaustively. Prefer small concrete functions over generic abstractions.
- Update crate READMEs with behavior changes.
- An inventory is not a proof. Never turn an unsupported operation or missing body into success.
- Contracts are metadata. Do not inject runtime checks or treat an annotation as verified.
- Test compiler integration with positive, negative and unknown cases; check that mutations fail.
- Before committing, run formatting, cargo lint, tests, release builds, cargo deny and manual hooks.
- Commit each completed stage separately. No unsafe assumptions or application-code rewrites.
