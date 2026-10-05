# mir-checker

Host compiler adapter and report model. mir-checker drives rustc directly; cargo-mir-checker
uses it as a Cargo workspace wrapper and collects per-crate reports in an isolated build directory.

The adapter reads local typed runtime MIR using the pinned compiler and disables MIR
optimization. It collects function locations, block counts, pending contracts, MIR checks, panic
language-item calls and unknown call/drop boundaries. Optional entry selection shows structural
paths through local calls; it does not check execution feasibility or substitute generic arguments.

The inventory includes cleanup blocks and marks structural CFG reachability. Assert conditions
and compiler operands are diagnostic strings, not a stable representation for future solvers.
The future proof engine should consume typed MIR inside the adapter, not parse those strings.

Compilation continues in Cargo mode and stops after analysis in direct mode. Compiler failures,
unknown entries and report-write failures produce a nonzero exit status. Success establishes
only that the inventory was collected.

The report model uses std and serde. No compiler types escape the adapter, so JSON consumers
do not need rustc internals. No proof engine or dependency analysis exists in stages 1 and 2.
