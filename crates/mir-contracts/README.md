# mir-contracts

Host procedural macros declaring no_panic, requires and ensures contracts for mir-check.
Consumers can use stable Rust and no_std. The attributes add doc metadata without evaluating
predicates, modifying function bodies or adding target runtime dependencies.

The macros validate Rust expression syntax only. With --verify, the checker resolves and checks
its restricted pure predicate language against typed MIR arguments, checks call preconditions and
proves postconditions. Unsupported predicates cannot pass verification. Without the checker,
unresolved names and false predicates still add no runtime behavior.

In ensures, result names the actual return value and parameter names denote entry values.
Functions and methods with bodies are supported, including const fn. Receiver types outside the
checker's modeled input subset remain unsupported for proof.

Versioned HTML comments in doc attributes carry the declarations through macro expansion to
the compiler adapter. This format is experimental and is not a trusted proof certificate.
