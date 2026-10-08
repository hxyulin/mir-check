# Guides

Read the [published documentation](https://hxyulin.github.io/miren/) or the same guides here.

- [Usage](usage.md): installation, Cargo root selection, embedded targets and report outcomes.
- [Contracts](contracts.md): metadata semantics and a bounded function example.
- [Examples](examples.md): DR16 parsing, CAN frames, bus configuration and nested inputs.
- [Coverage](coverage.md): supported Rust features, concrete evidence and remaining gaps.
- [Fleet survey](fleet-survey.md): measured outcomes for unchanged embedded workspaces.
- [Proof execution](proofs.md): symbolic states, obligations, contracts and trusted components.
- [Development](development.md): local checks, documentation previews and Pages publishing.
- [Analyzer redesign](analyzer-redesign.md): storage, calls, performance and counterexample
  validation.
- [Typed storage](storage-model.md): allocation identity, footprints, initialization and
  interference.
- [Stages](stages.md): implementation history and validation evidence.

Start with the repository README for a runnable parser proof. The compiler crate README describes
the adapter boundary; the miren-contracts README describes metadata behavior. Each example README
records its scope and, for vendored code, its source provenance.
