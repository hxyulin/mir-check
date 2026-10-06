use mir_check::smt::{Constant, Context, Op, Rounding, Sort, Term};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn solve(queries: &[String]) -> Vec<String> {
    let solver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.venv/bin/z3");
    let mut child = Command::new(solver)
        .args(["-in", "-smt2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("term differential tests require Z3");
    let mut input = String::from("(set-logic ALL)\n(set-option :timeout 5000)\n");
    for query in queries {
        input.push_str("(push 1)\n");
        input.push_str(query);
        input.push_str("\n(check-sat)\n(pop 1)\n");
    }
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn equivalent(term: &Term, expected: &str) -> String {
    format!(
        "(assert (not (= {} {expected})))",
        term.smt(200_000).unwrap()
    )
}

#[test]
fn equal_terms_share_nodes_and_roots_keep_distinct_namespaces() {
    let context = Context::default();
    let x = context.symbol(0, Sort::BitVec(32)).unwrap();
    let y = context.symbol(1, Sort::BitVec(32)).unwrap();
    let left = context.apply(Op::BvAdd, &[x.clone(), y.clone()]).unwrap();
    let right = context.apply(Op::BvAdd, &[x.clone(), y]).unwrap();
    assert_eq!(left, right);
    assert_eq!(left.smt(100).unwrap(), "(bvadd v0 v1)");
    let other = Context::default().symbol(0, Sort::BitVec(32)).unwrap();
    assert_ne!(x, other);
    assert!(context.apply(Op::BvAdd, &[x, other]).is_err());
    assert!(context.symbol(0, Sort::Bool).is_err());
}

#[test]
fn invalid_sorts_and_arity_are_rejected_before_simplification() {
    let context = Context::default();
    let false_value = context.boolean(false);
    let word = context.bit_vector(1, 8).unwrap();
    for op in [Op::BvAdd, Op::Equal, Op::And, Op::FpEqual, Op::Select] {
        assert!(
            context
                .apply(op, &[false_value.clone(), word.clone()])
                .is_err()
        );
    }
    assert!(context.apply(Op::Not, &[]).is_err());
    assert!(
        context
            .apply(Op::Ite, &[false_value.clone(), word])
            .is_err()
    );
    assert!(context.bit_vector(0, 0).is_err());
    assert!(context.bit_vector(0, 257).is_err());
    assert!(context.symbol(4, Sort::BitVec(0)).is_err());
    assert!(
        context
            .apply(
                Op::Extract { high: 8, low: 0 },
                &[context.bit_vector(0, 8).unwrap()]
            )
            .is_err()
    );
    assert!(
        context
            .apply(Op::ConstArray { index_bits: 0 }, &[false_value])
            .is_err()
    );
}

#[test]
fn structural_boolean_folding_preserves_symbolic_obligations() {
    let context = Context::default();
    let x = context.symbol(0, Sort::Bool).unwrap();
    let negated = context.apply(Op::Not, std::slice::from_ref(&x)).unwrap();
    assert_eq!(context.apply(Op::Not, &[negated]).unwrap(), x);
    assert_eq!(context.apply(Op::And, &[]).unwrap(), context.boolean(true));
    assert_eq!(context.apply(Op::Or, &[]).unwrap(), context.boolean(false));
    assert_eq!(
        context
            .apply(Op::And, &[x.clone(), context.boolean(true)])
            .unwrap(),
        x
    );
    assert_eq!(
        context
            .apply(
                Op::Ite,
                &[context.boolean(false), context.boolean(true), x.clone()]
            )
            .unwrap(),
        x
    );
    let queries = [
        "(declare-const v0 Bool)\n(assert (not v0))".to_owned(),
        "(declare-const v0 Bool)\n(assert v0)\n(assert (not v0))".to_owned(),
    ];
    assert_eq!(solve(&queries), ["sat", "unsat"]);
}

#[test]
fn closed_bit_vector_operations_match_z3_at_signed_and_width_boundaries() {
    let context = Context::default();
    let operations = [
        (Op::BvAdd, "bvadd"),
        (Op::BvSub, "bvsub"),
        (Op::BvMul, "bvmul"),
        (Op::BvUnsignedDiv, "bvudiv"),
        (Op::BvSignedDiv, "bvsdiv"),
        (Op::BvUnsignedRem, "bvurem"),
        (Op::BvSignedRem, "bvsrem"),
        (Op::BvAnd, "bvand"),
        (Op::BvOr, "bvor"),
        (Op::BvXor, "bvxor"),
        (Op::BvShiftLeft, "bvshl"),
        (Op::BvLogicalShiftRight, "bvlshr"),
        (Op::BvArithmeticShiftRight, "bvashr"),
        (Op::BvUnsignedLt, "bvult"),
        (Op::BvUnsignedLe, "bvule"),
        (Op::BvUnsignedGt, "bvugt"),
        (Op::BvUnsignedGe, "bvuge"),
        (Op::BvSignedLt, "bvslt"),
        (Op::BvSignedLe, "bvsle"),
        (Op::BvSignedGt, "bvsgt"),
        (Op::BvSignedGe, "bvsge"),
    ];
    let mut queries = Vec::new();
    for bits in [1, 8, 16, 32, 64, 128] {
        let maximum = u128::MAX >> (128 - bits);
        let sign = 1_u128 << (bits - 1);
        for left in [0, 1, maximum, sign, sign - 1] {
            for right in [0, 1, maximum, sign, sign - 1] {
                let a = context.bit_vector(left, bits).unwrap();
                let b = context.bit_vector(right, bits).unwrap();
                for (op, name) in operations {
                    let term = context.apply(op, &[a.clone(), b.clone()]).unwrap();
                    assert!(term.constant().is_some());
                    queries.push(equivalent(
                        &term,
                        &format!("({name} (_ bv{left} {bits}) (_ bv{right} {bits}))"),
                    ));
                }
            }
        }
    }
    let answers = solve(&queries);
    assert_eq!(answers.len(), queries.len());
    for (answer, query) in answers.iter().zip(queries) {
        assert_eq!(answer, "unsat", "{query}");
    }
}

#[test]
fn widened_arithmetic_keeps_exact_terms_when_native_folding_cannot_represent_them() {
    let context = Context::default();
    let max = context.bit_vector(u128::MAX, 128).unwrap();
    let wide = context.apply(Op::ZeroExtend(128), &[max]).unwrap();
    let product = context.apply(Op::BvMul, &[wide.clone(), wide]).unwrap();
    assert_eq!(product.sort(), &Sort::BitVec(256));
    assert_eq!(product.constant(), None);
    let expected = concat!(
        "(bvmul ((_ zero_extend 128) (_ bv340282366920938463463374607431768211455 128)) ",
        "((_ zero_extend 128) (_ bv340282366920938463463374607431768211455 128)))"
    );
    assert_eq!(solve(&[equivalent(&product, expected)]), ["unsat"]);
}

#[test]
fn floating_point_equality_preserves_nan_and_signed_zero_semantics() {
    let context = Context::default();
    let decode = Op::FloatFromBits {
        exponent: 8,
        significand: 24,
    };
    let nan = context
        .apply(decode, &[context.bit_vector(0x7fc0_0001, 32).unwrap()])
        .unwrap();
    let positive = context
        .apply(decode, &[context.bit_vector(0, 32).unwrap()])
        .unwrap();
    let negative = context
        .apply(decode, &[context.bit_vector(0x8000_0000, 32).unwrap()])
        .unwrap();
    let nan_equal = context.apply(Op::FpEqual, &[nan.clone(), nan]).unwrap();
    assert_eq!(nan_equal.constant(), None);
    let numeric_zero_equal = context
        .apply(Op::FpEqual, &[positive.clone(), negative.clone()])
        .unwrap();
    let structural_zero_equal = context.apply(Op::Equal, &[positive, negative]).unwrap();
    let queries = [
        equivalent(&nan_equal, "false"),
        equivalent(&numeric_zero_equal, "true"),
        equivalent(&structural_zero_equal, "false"),
    ];
    assert_eq!(solve(&queries), ["unsat", "unsat", "unsat"]);
}

#[test]
fn byte_array_updates_and_float_conversions_print_well_typed_terms() {
    let context = Context::default();
    let array = context
        .symbol(
            0,
            Sort::Array(Box::new(Sort::BitVec(32)), Box::new(Sort::BitVec(8))),
        )
        .unwrap();
    let index = context.bit_vector(2, 32).unwrap();
    let byte = context.bit_vector(91, 8).unwrap();
    let stored = context
        .apply(Op::Store, &[array, index.clone(), byte.clone()])
        .unwrap();
    let read = context.apply(Op::Select, &[stored, index]).unwrap();
    assert_eq!(read, byte);
    let constant = context
        .apply(Op::ConstArray { index_bits: 32 }, &[byte])
        .unwrap();
    let word = context.symbol(1, Sort::BitVec(32)).unwrap();
    let converted = context
        .apply(
            Op::UnsignedToFloat {
                exponent: 8,
                significand: 24,
            },
            &[context.rounding(Rounding::NearestEven), word],
        )
        .unwrap();
    let integer = context
        .apply(
            Op::FloatToUnsigned(32),
            &[context.rounding(Rounding::TowardZero), converted],
        )
        .unwrap();
    let queries = [
        equivalent(
            &constant,
            "((as const (Array (_ BitVec 32) (_ BitVec 8))) (_ bv91 8))",
        ),
        format!(
            "(declare-const v1 (_ BitVec 32))\n{}",
            equivalent(
                &integer,
                "((_ fp.to_ubv 32) RTZ ((_ to_fp_unsigned 8 24) RNE v1))"
            )
        ),
    ];
    assert_eq!(solve(&queries), ["unsat", "unsat"]);
}

#[test]
fn the_printer_shares_repeated_terms_without_capturing_symbols_or_growing_exponentially() {
    let context = Context::default();
    let mut term = context.symbol(0, Sort::BitVec(32)).unwrap();
    let mut expanded = "v0".to_owned();
    for _ in 0..12 {
        term = context.apply(Op::BvAdd, &[term.clone(), term]).unwrap();
        expanded = format!("(bvadd {expanded} {expanded})");
        assert!(term.smt(200_000).unwrap().len() <= expanded.len());
    }
    let text = term.smt(200_000).unwrap();
    assert!(text.contains("(let ((t"));
    assert!(text.len() < 600, "{text}");
    assert_eq!(
        solve(&[format!(
            "(declare-const v0 (_ BitVec 32))\n{}",
            equivalent(&term, &expanded)
        )]),
        ["unsat"]
    );
    let changed = format!("(bvadd {expanded} (_ bv1 32))");
    assert_eq!(
        solve(&[format!(
            "(declare-const v0 (_ BitVec 32))\n{}",
            equivalent(&term, &changed)
        )]),
        ["sat"]
    );
    for _ in 0..244 {
        term = context.apply(Op::BvAdd, &[term.clone(), term]).unwrap();
    }
    let text = term.smt(20_000).unwrap();
    assert!(text.len() < 12_000);
    assert!(term.smt(100).is_err());
    assert_eq!(
        context.bit_vector(256, 8).unwrap().constant(),
        Some(Constant::BitVec(0))
    );
}
