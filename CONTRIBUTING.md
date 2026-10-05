# Contributing to mir-check

Bug reports, small real-code examples, documentation and implementation changes are welcome.
Start with the [usage guide](docs/usage.md), [coverage matrix](docs/coverage.md) and
[proof explanation](docs/proofs.md). This is an experimental side project with a pinned compiler.

For an analysis issue, include a minimal Rust example, the exact command, target, compiler flags
and selected root. Include the outcome and relevant JSON obligations or gap reasons. Distinguish
a solver assignment from a failure reproduced by executing Rust. A PROVED result for a function
that actually panics within the recorded domain is a correctness bug, not a coverage request.

For a coverage request, show the first unsupported operation and the behavior you need checked.
Available dependency MIR, concrete callers and input invariants can affect whether an example
is analyzable. An annotation that claims a function is safe is not a substitute for its proof.

Implementation changes need meaningful positive, negative and unknown compiler cases. Change a
guard, bound or supported operation to confirm that the relevant proof fails when broken. Add
runtime replay evidence when practical. Never turn an unsupported path or missing body into
success. Update the guides and crate READMEs with behavior changes.

See [development](docs/development.md) for setup, required checks and documentation previews,
and [AGENTS.md](AGENTS.md) for code conventions. Keep completed stages in separate commits.
