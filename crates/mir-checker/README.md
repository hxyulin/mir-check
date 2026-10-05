# mir-checker

Host compiler adapter and report model. mir-checker drives rustc directly; cargo-mir-checker
uses it as a Cargo workspace wrapper and collects per-crate reports in an isolated build directory.

The adapter reads local typed runtime MIR using the pinned compiler and disables MIR
optimization. It collects function locations, block counts, pending contracts, MIR checks, panic
language-item calls and unknown call/drop boundaries. Optional entry selection shows structural
paths through local calls; it does not check execution feasibility or substitute generic arguments.

The inventory includes cleanup blocks and marks structural CFG reachability. Assert conditions
and compiler operands are diagnostic strings, not a stable representation for future solvers.
The proof engine consumes typed MIR inside the adapter and does not parse inventory strings.

Compilation continues in Cargo mode and stops after analysis in direct mode. Compiler failures,
unknown entries and report-write failures produce a nonzero exit status. In inventory mode,
success establishes only that the inventory was collected.

The report model uses std and serde. No compiler types escape the adapter, so JSON consumers
do not need rustc internals. Inventory remains separate from the opt-in proof engine.

Proof mode interprets a restricted subset of typed MIR, uses exact SMT bit-vectors for integer
semantics and follows actual arguments and return values through local calls. Z3 runs as a
subprocess. Every reachable panic condition must be unsatisfiable; unsupported behavior, loops,
recursion and resource limits remain unknown and cause verification failure. No dependency
analysis is present. The contract evaluator accepts only pure comparisons and boolean predicates,
with read-only byte lengths. It checks caller preconditions and every feasible return, using
entry values for parameter names in postconditions. It never assumes a callee summary from
annotations. Missing names, type errors, unsupported predicates and inconsistent entry domains
fail verification; passing root metadata is marked verified under preconditions.
