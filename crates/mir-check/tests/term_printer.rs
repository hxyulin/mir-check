use mir_check::smt::{Context, Op, Rounding, Sort};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[test]
fn every_operator_prints_a_formula_equivalent_to_its_independent_smt_encoding() {
    let context = Context::default();
    let a = context.symbol(0, Sort::BitVec(8)).unwrap();
    let b = context.symbol(1, Sort::BitVec(8)).unwrap();
    let x = context.symbol(2, Sort::Bool).unwrap();
    let y = context.symbol(3, Sort::Bool).unwrap();
    let float_sort = Sort::Float {
        exponent: 8,
        significand: 24,
    };
    let f = context.symbol(4, float_sort.clone()).unwrap();
    let g = context.symbol(5, float_sort).unwrap();
    let array = context
        .symbol(
            6,
            Sort::Array(Box::new(Sort::BitVec(8)), Box::new(Sort::BitVec(8))),
        )
        .unwrap();
    let raw = context.symbol(7, Sort::BitVec(32)).unwrap();
    let rne = context.rounding(Rounding::NearestEven);
    let rtz = context.rounding(Rounding::TowardZero);
    let mut cases = Vec::new();
    for (op, name) in [
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
        (Op::Concat, "concat"),
        (Op::Equal, "="),
    ] {
        cases.push((op, vec![a.clone(), b.clone()], format!("({name} v0 v1)")));
    }
    for (op, name) in [(Op::And, "and"), (Op::Or, "or"), (Op::Xor, "xor")] {
        cases.push((op, vec![x.clone(), y.clone()], format!("({name} v2 v3)")));
    }
    cases.extend([
        (Op::Not, vec![x.clone()], "(not v2)".to_owned()),
        (
            Op::Ite,
            vec![x, a.clone(), b.clone()],
            "(ite v2 v0 v1)".to_owned(),
        ),
        (Op::BvNot, vec![a.clone()], "(bvnot v0)".to_owned()),
        (Op::BvNeg, vec![a.clone()], "(bvneg v0)".to_owned()),
        (
            Op::Extract { high: 5, low: 2 },
            vec![a.clone()],
            "((_ extract 5 2) v0)".to_owned(),
        ),
        (
            Op::ZeroExtend(8),
            vec![a.clone()],
            "((_ zero_extend 8) v0)".to_owned(),
        ),
        (
            Op::SignExtend(8),
            vec![a.clone()],
            "((_ sign_extend 8) v0)".to_owned(),
        ),
        (
            Op::Select,
            vec![array.clone(), a.clone()],
            "(select v6 v0)".to_owned(),
        ),
        (
            Op::Store,
            vec![array, a.clone(), b],
            "(store v6 v0 v1)".to_owned(),
        ),
        (
            Op::ConstArray { index_bits: 8 },
            vec![a.clone()],
            "((as const (Array (_ BitVec 8) (_ BitVec 8))) v0)".to_owned(),
        ),
        (
            Op::PositiveZero {
                exponent: 8,
                significand: 24,
            },
            vec![],
            "(_ +zero 8 24)".to_owned(),
        ),
        (
            Op::FloatFromBits {
                exponent: 8,
                significand: 24,
            },
            vec![raw],
            "((_ to_fp 8 24) v7)".to_owned(),
        ),
        (
            Op::SignedToFloat {
                exponent: 8,
                significand: 24,
            },
            vec![rne.clone(), a.clone()],
            "((_ to_fp 8 24) RNE v0)".to_owned(),
        ),
        (
            Op::UnsignedToFloat {
                exponent: 8,
                significand: 24,
            },
            vec![rne.clone(), a],
            "((_ to_fp_unsigned 8 24) RNE v0)".to_owned(),
        ),
        (
            Op::FloatToFloat {
                exponent: 11,
                significand: 53,
            },
            vec![rne.clone(), f.clone()],
            "((_ to_fp 11 53) RNE v4)".to_owned(),
        ),
        (
            Op::FloatToSigned(32),
            vec![rtz.clone(), f.clone()],
            "((_ fp.to_sbv 32) RTZ v4)".to_owned(),
        ),
        (
            Op::FloatToUnsigned(32),
            vec![rtz, f.clone()],
            "((_ fp.to_ubv 32) RTZ v4)".to_owned(),
        ),
    ]);
    for (op, name) in [
        (Op::FpNeg, "fp.neg"),
        (Op::FpAbs, "fp.abs"),
        (Op::FpIsNaN, "fp.isNaN"),
    ] {
        cases.push((op, vec![f.clone()], format!("({name} v4)")));
    }
    for (op, name) in [
        (Op::FpEqual, "fp.eq"),
        (Op::FpLt, "fp.lt"),
        (Op::FpLe, "fp.leq"),
        (Op::FpGt, "fp.gt"),
        (Op::FpGe, "fp.geq"),
    ] {
        cases.push((op, vec![f.clone(), g.clone()], format!("({name} v4 v5)")));
    }
    for (op, name) in [
        (Op::FpAdd, "fp.add"),
        (Op::FpSub, "fp.sub"),
        (Op::FpMul, "fp.mul"),
        (Op::FpDiv, "fp.div"),
    ] {
        cases.push((
            op,
            vec![rne.clone(), f.clone(), g.clone()],
            format!("({name} RNE v4 v5)"),
        ));
    }
    assert_eq!(cases.len(), 55);
    let mut script = String::from("(set-logic ALL)\n(set-option :timeout 5000)\n");
    for (index, sort) in [
        (0, "(_ BitVec 8)"),
        (1, "(_ BitVec 8)"),
        (2, "Bool"),
        (3, "Bool"),
        (4, "(_ FloatingPoint 8 24)"),
        (5, "(_ FloatingPoint 8 24)"),
        (6, "(Array (_ BitVec 8) (_ BitVec 8))"),
        (7, "(_ BitVec 32)"),
    ] {
        script.push_str(&format!("(declare-const v{index} {sort})\n"));
    }
    for (op, operands, expected) in &cases {
        let term = context.apply(*op, operands).unwrap();
        script.push_str(&format!(
            "(push 1)\n(assert (not (= {} {expected})))\n(check-sat)\n(pop 1)\n",
            term.smt(200_000).unwrap()
        ));
    }
    let solver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.venv/bin/z3");
    let mut child = Command::new(solver)
        .args(["-in", "-smt2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    let answers: Vec<_> = text.lines().collect();
    assert_eq!(answers.len(), cases.len(), "{text}");
    for (answer, (op, _, _)) in answers.iter().zip(cases) {
        assert_eq!(*answer, "unsat", "{op:?}");
    }
}
