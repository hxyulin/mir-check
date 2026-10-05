# mir-checker

Host compiler adapter and report model. mir-checker drives rustc directly; cargo-mir-checker
uses it as a Cargo workspace wrapper and collects per-crate reports in an isolated build directory.

The adapter reads local typed runtime MIR using the pinned compiler and disables MIR
optimization. It collects function locations, block counts and pending contract metadata. It
continues compilation in Cargo mode and stops after analysis in direct mode. Compiler failures
and report-write failures produce a nonzero exit status.

The report model uses std and serde. No compiler types escape the adapter, so JSON consumers
do not need rustc internals. No proof engine or dependency analysis exists in stage 1.
