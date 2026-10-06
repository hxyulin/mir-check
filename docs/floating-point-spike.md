# Floating-point induction feasibility

This experiment sends independently written SMT-LIB to Z3 4.15.4. It does not change the checker or
analyze an application. Current `--induction` still rejects floating-point state. The following
binary32 loop initializes a persistent value to positive zero, adds one forever, and asks whether
an invariant can exclude every negative value:

```lisp
(set-logic ALL)
(set-option :fp.xform.bit_blast false)
(declare-fun Reach ((_ FloatingPoint 8 24)) Bool)
(assert (Reach ((_ to_fp 8 24) #x00000000)))
(assert (forall ((x (_ FloatingPoint 8 24)))
  (=> (Reach x) (Reach (fp.add RNE x ((_ to_fp 8 24) #x3f800000))))))
(assert (forall ((x (_ FloatingPoint 8 24)))
  (=> (and (Reach x) (fp.lt x ((_ to_fp 8 24) #x00000000))) false)))
(check-sat-using horn)
```

Run it with `.venv/bin/z3 -T:5 experiment.smt2`. Here SAT means that a closed invariant excludes
failure. UNSAT means that no such invariant satisfies the encoded transition/failure clauses;
it does not provide a native Rust replay. The `HORN` logic parser rejects floating-point sorts
and operations, so this probe uses ALL and explicitly selects the Horn tactic. Enabling Spacer's
bit-blast transformation is deliberately avoided because an earlier integer mutation exposed a
false safety result with that option.

Appending `(get-model)` returns the relation `Reach(x) = not(fp.lt x +zero)`. That invariant
admits NaN, which the displayed safety predicate does not reject. A stronger probe replaces the
failure predicate with `not(fp.geq x +zero)`; Spacer also proves it and returns
`Reach(x) = fp.leq(+zero, x)`, excluding NaN as a Rust `assert!(x >= 0.0)` would.

Single local ARM64 probes produced the following results. These timings are solver-only examples,
not performance benchmarks or coverage claims about real Rust functions.

| Encoding or mutation | Result | Approximate time |
| --- | --- | ---: |
| Native binary32 state, increment, exclude negative values | SAT | 56 ms |
| Same loop, require a nonnegative value and exclude NaN | SAT | 64 ms |
| 32-bit storage state, exact decoding and result-encoding equality | SAT | 52 ms |
| Subtract one instead of adding one | UNSAT | 61 ms |
| Initialize to NaN and require a nonnegative value | UNSAT | 8 ms |
| Reset to zero once the value reaches 17 | SAT | 53 ms |
| Increment and exclude every infinity | SAT | 484 ms |
| Increment and exclude every value at least 17 | Timeout | 5 s |

The infinity result is correct: binary32 addition of one eventually stops changing at 2^24.
Replacing IEEE arithmetic with real arithmetic would miss that behavior. The late-failure probe
does reach 17, but Spacer did not establish that within the budget; timeout must remain UNKNOWN.

For the storage variant, each step uses a fresh 32-bit encoding `y` constrained by
`((_ to_fp 8 24) y) = fp.add(...)`. This fixes numeric values and signed zero while permitting every
NaN payload and sign, matching the ordinary interpreter's conservative result-storage policy.
An implementation must carry these equalities into every relevant transition. Merely allowing
float values in the existing loop layout would omit required storage constraints.

The outcome supports further experiments, with both positive invariants and failing mutations,
before integrating float state into the typed MIR encoding. It does not establish that arbitrary
float-heavy loops will infer useful invariants or finish within the current solver limits. The
[SMT-LIB FloatingPoint theory](https://smt-lib.org/theories-FloatingPoint.shtml) defines the exact
operations, rounding modes and result-encoding equality used here.
