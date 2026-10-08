use super::{Constant, Context, Op, Sort, Value, binary, constant, integer};

#[test]
fn arithmetic_folding_preserves_wrapping_signedness_and_declared_width() {
    let context = Context::default();
    for (operation, left, right, bits, signed, expected) in [
        ("add", 255, 1, 8, false, 0_u128),
        ("sub", 0, 1, 8, true, 255),
        (
            "mul",
            u64::MAX as u128,
            3,
            64,
            false,
            18_446_744_073_709_551_613,
        ),
        ("add", u128::MAX, 1, 128, false, 0),
    ] {
        let Value::Int {
            expression,
            bits: actual,
            signed: actual_signed,
        } = binary(
            &context,
            operation,
            integer(&context, left, bits, signed),
            integer(&context, right, bits, signed),
        )
        .unwrap()
        else {
            panic!("arithmetic must preserve the integer shape");
        };
        assert_eq!(
            constant(&expression),
            Some(Constant::BitVec {
                value: expected,
                bits
            })
        );
        assert_eq!((actual, actual_signed), (bits, signed));
    }
    assert!(
        binary(
            &context,
            "add",
            integer(&context, 1, 8, false),
            integer(&context, 2, 16, false)
        )
        .is_err()
    );
}

#[test]
fn checked_results_fold_the_wrapped_value_and_overflow_independently() {
    let context = Context::default();
    for (left, right, bits, signed) in [
        (255, 1, 8, false),
        (127, 1, 8, true),
        (0, 1, 8, true),
        (i64::MAX as u128, 1, 64, true),
        (u64::MAX as u128, 1, 64, false),
    ] {
        let Value::Tuple(fields) = binary(
            &context,
            "checked_add",
            integer(&context, left, bits, signed),
            integer(&context, right, bits, signed),
        )
        .unwrap() else {
            panic!("checked arithmetic must return a tuple");
        };
        let overflow = if signed {
            (left + right) > (1_u128 << (bits - 1)) - 1
        } else {
            left + right >= 1_u128 << bits
        };
        assert_eq!(
            constant(&fields[1].boolean().unwrap()),
            Some(Constant::Bool(overflow))
        );
        let expected = (left + right) & (u128::MAX >> (128 - bits));
        assert_eq!(
            constant(&fields[0].integer().unwrap().0),
            Some(Constant::BitVec {
                value: expected,
                bits
            })
        );
    }
}

#[test]
fn checked_integer_folding_matches_native_arithmetic_at_signed_boundaries() {
    let context = Context::default();
    for bits in [1, 8, 16, 32, 64] {
        let mask = u128::MAX >> (128 - bits);
        let sign = 1_u128 << (bits - 1);
        for left in [0, 1, sign - 1, sign, mask] {
            for right in [0, 1, sign - 1, sign, mask] {
                for signed in [false, true] {
                    let a = ((left << (128 - bits)) as i128) >> (128 - bits);
                    let b = ((right << (128 - bits)) as i128) >> (128 - bits);
                    for operation in ["checked_add", "checked_sub", "checked_mul"] {
                        let (expected, overflow) = if signed {
                            let wide = match operation {
                                "checked_add" => a + b,
                                "checked_sub" => a - b,
                                "checked_mul" => a * b,
                                _ => unreachable!("listed checked arithmetic operations"),
                            };
                            (
                                (wide as u128) & mask,
                                !(-(sign as i128)..=(sign - 1) as i128).contains(&wide),
                            )
                        } else {
                            let (wide, overflow) = match operation {
                                "checked_add" => left.overflowing_add(right),
                                "checked_sub" => left.overflowing_sub(right),
                                "checked_mul" => left.overflowing_mul(right),
                                _ => unreachable!("listed checked arithmetic operations"),
                            };
                            (wide & mask, overflow || wide > mask)
                        };
                        let Value::Tuple(fields) = binary(
                            &context,
                            operation,
                            integer(&context, left, bits, signed),
                            integer(&context, right, bits, signed),
                        )
                        .unwrap() else {
                            panic!("checked arithmetic must return a tuple");
                        };
                        assert_eq!(
                            constant(&fields[0].integer().unwrap().0),
                            Some(Constant::BitVec {
                                value: expected,
                                bits
                            }),
                            "{operation} {left}, {right}, width {bits}, signed {signed}"
                        );
                        assert_eq!(
                            constant(&fields[1].boolean().unwrap()),
                            Some(Constant::Bool(overflow)),
                            "{operation} {left}, {right}, width {bits}, signed {signed}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn symbolic_and_wide_arithmetic_remain_terms_and_unsupported_operations_fail() {
    let context = Context::default();
    let input = Value::Int {
        expression: context.symbol(0, Sort::BitVec(8)).unwrap(),
        bits: 8,
        signed: false,
    };
    let result = binary(
        &context,
        "add",
        input.clone(),
        integer(&context, 1, 8, false),
    )
    .unwrap();
    assert_eq!(constant(&result.integer().unwrap().0), None);
    assert!(
        binary(
            &context,
            "unsupported",
            input,
            integer(&context, 1, 8, false)
        )
        .is_err()
    );
    let wide = context
        .apply(Op::ZeroExtend(1), &[context.bit_vector(1, 128).unwrap()])
        .unwrap();
    assert_eq!(constant(&wide), None);
    assert!(
        context
            .apply(
                Op::BvAdd,
                &[
                    context.bit_vector(1, 8).unwrap(),
                    context.bit_vector(1, 16).unwrap()
                ]
            )
            .is_err()
    );
}

#[test]
fn moving_references_through_owned_iterators_does_not_enable_identity_repeats() {
    for mutable in [false, true] {
        let reference = Value::Reference {
            allocation: 0,
            projection: Vec::new(),
            mutable,
        };
        let values = Value::Elements(vec![Value::Tuple(vec![reference])]);
        assert_eq!(values.owned_iterator_size(), Some(3));
        assert_eq!(values.owned_repeat_size(), None);
    }
    let cells = Value::Elements(vec![Value::Cell { allocation: 0 }]);
    assert_eq!(cells.owned_iterator_size(), None);
}
