# mir-contracts

Host procedural macros declaring no_panic, requires and ensures contracts for mir-checker.
Consumers can use stable Rust and no_std. The attributes add doc metadata without evaluating
predicates, modifying function bodies or adding target runtime dependencies.

Predicates must parse as Rust expressions. Their names, types, purity and truth are not yet
verified. The checker records every contract as pending verification. result names the intended
return value in ensures. Functions and methods with bodies are supported, including const fn.

Versioned HTML comments in doc attributes carry the declarations through macro expansion to
the compiler adapter. This format is experimental and is not a trusted proof certificate.
