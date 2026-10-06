use super::Value;
use crate::solver::ground::{Constant, constant};

pub(super) fn fold(value: Value) -> Value {
    match value {
        Value::Bool(expression) => match constant(&expression) {
            Some(Constant::Bool(value)) => Value::Bool(value.to_string()),
            Some(Constant::BitVec { .. }) | None => Value::Bool(expression),
        },
        Value::Int {
            expression,
            bits,
            signed,
        } => {
            let expression = match constant(&expression) {
                Some(Constant::BitVec {
                    value,
                    bits: parsed_bits,
                }) if bits == parsed_bits => format!("(_ bv{value} {bits})"),
                Some(Constant::BitVec { .. } | Constant::Bool(_)) | None => expression,
            };
            Value::Int {
                expression,
                bits,
                signed,
            }
        }
        Value::Tuple(fields) => Value::Tuple(fields.into_iter().map(fold).collect()),
        value @ (Value::Float { .. }
        | Value::Bytes { .. }
        | Value::Adt { .. }
        | Value::Enum { .. }
        | Value::Cell { .. }
        | Value::Atomic { .. }
        | Value::Reference { .. }
        | Value::SliceIterator { .. }
        | Value::Elements(_)
        | Value::MetadataPointer(_)
        | Value::StaticText
        | Value::FormatArguments
        | Value::Function
        | Value::Unit) => value,
    }
}

#[cfg(test)]
mod tests {
    use super::fold;
    use crate::symbolic::Value;

    fn integer(expression: &str, bits: u32, signed: bool) -> Value {
        Value::Int {
            expression: expression.to_owned(),
            bits,
            signed,
        }
    }

    #[test]
    fn arithmetic_folding_preserves_wrapping_signedness_and_declared_width() {
        for (expression, bits, signed, expected) in [
            ("(bvadd (_ bv255 8) (_ bv1 8))", 8, false, 0_u128),
            ("(bvsub (_ bv0 8) (_ bv1 8))", 8, true, 255),
            (
                "(bvmul (_ bv18446744073709551615 64) (_ bv3 64))",
                64,
                false,
                18_446_744_073_709_551_613,
            ),
            (
                "(bvadd (_ bv340282366920938463463374607431768211455 128) (_ bv1 128))",
                128,
                false,
                0,
            ),
        ] {
            let Value::Int {
                expression: folded,
                bits: folded_bits,
                signed: folded_signed,
            } = fold(integer(expression, bits, signed))
            else {
                panic!("folding must preserve the integer shape");
            };
            assert_eq!(folded, format!("(_ bv{expected} {bits})"));
            assert_eq!(folded_bits, bits);
            assert_eq!(folded_signed, signed);
        }
        let wrong_width = "(bvadd (_ bv1 8) (_ bv2 8))";
        let Value::Int { expression, .. } = fold(integer(wrong_width, 16, false)) else {
            panic!("folding must preserve the integer shape");
        };
        assert_eq!(expression, wrong_width);
    }

    #[test]
    fn checked_results_fold_the_wrapped_value_and_overflow_independently() {
        for (left, right, bits, signed) in [
            (255, 1, 8, false),
            (127, 1, 8, true),
            (0, 1, 8, true),
            (9_223_372_036_854_775_807, 1, 64, true),
            (18_446_744_073_709_551_615, 1, 64, false),
        ] {
            let extend = if signed { "sign_extend" } else { "zero_extend" };
            let expression = format!("(bvadd (_ bv{left} {bits}) (_ bv{right} {bits}))");
            let overflow = format!(
                "(not (= (bvadd ((_ {extend} 1) (_ bv{left} {bits})) \
                 ((_ {extend} 1) (_ bv{right} {bits}))) ((_ {extend} 1) {expression})))"
            );
            let Value::Tuple(fields) = fold(Value::Tuple(vec![
                integer(&expression, bits, signed),
                Value::Bool(overflow),
            ])) else {
                panic!("folding must preserve checked arithmetic tuples");
            };
            let expected_overflow = if signed {
                let maximum = (1_u128 << (bits - 1)) - 1;
                left + right > maximum
            } else {
                left + right >= 1_u128 << bits
            };
            assert!(matches!(&fields[1], Value::Bool(value)
                if value == &expected_overflow.to_string()));
            let expected = (left + right) & (u128::MAX >> (128 - bits));
            assert!(matches!(&fields[0], Value::Int { expression, .. }
                if expression == &format!("(_ bv{expected} {bits})")));
        }
    }

    #[test]
    fn checked_integer_folding_matches_native_arithmetic_at_signed_boundaries() {
        for bits in [1, 8, 16, 32, 64] {
            let mask = u128::MAX >> (128 - bits);
            let sign = 1_u128 << (bits - 1);
            let signed_minimum = -(sign as i128);
            let signed_maximum = (sign - 1) as i128;
            for left in [0, 1, sign - 1, sign, mask] {
                for right in [0, 1, sign - 1, sign, mask] {
                    for signed in [false, true] {
                        let left_signed = ((left << (128 - bits)) as i128) >> (128 - bits);
                        let right_signed = ((right << (128 - bits)) as i128) >> (128 - bits);
                        for operation in ["checked_add", "checked_sub", "checked_mul"] {
                            let (expected, overflow) = if signed {
                                let wide = match operation {
                                    "checked_add" => left_signed + right_signed,
                                    "checked_sub" => left_signed - right_signed,
                                    "checked_mul" => left_signed * right_signed,
                                    _ => unreachable!(
                                        "only checked arithmetic operations are listed"
                                    ),
                                };
                                (
                                    (wide as u128) & mask,
                                    !(signed_minimum..=signed_maximum).contains(&wide),
                                )
                            } else {
                                let (wide, host_overflow) = match operation {
                                    "checked_add" => left.overflowing_add(right),
                                    "checked_sub" => left.overflowing_sub(right),
                                    "checked_mul" => left.overflowing_mul(right),
                                    _ => unreachable!(
                                        "only checked arithmetic operations are listed"
                                    ),
                                };
                                (wide & mask, host_overflow || wide > mask)
                            };
                            let result = crate::symbolic::binary(
                                operation,
                                crate::symbolic::integer(left, bits, signed),
                                crate::symbolic::integer(right, bits, signed),
                            )
                            .unwrap();
                            let Value::Tuple(fields) = result else {
                                panic!("checked arithmetic must return a tuple");
                            };
                            assert!(
                                matches!(&fields[0], Value::Int { expression, .. }
                                if expression == &format!("(_ bv{expected} {bits})")),
                                "{operation} {left}, {right}, width {bits}, signed {signed}"
                            );
                            assert!(
                                matches!(&fields[1], Value::Bool(value)
                                if value == &overflow.to_string()),
                                "{operation} {left}, {right}, width {bits}, signed {signed}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn symbolic_unknown_and_wide_overflow_terms_are_preserved() {
        for expression in [
            "(bvadd v0 (_ bv1 8))",
            "(unsupported (_ bv1 8))",
            "(_ bv1 8) (_ bv2 8)",
            "((_ zero_extend 1) (_ bv1 128))",
        ] {
            let Value::Int {
                expression: folded, ..
            } = fold(integer(expression, 8, false))
            else {
                panic!("folding must preserve the integer shape");
            };
            assert_eq!(folded, expression);
        }
        let wide = "(= ((_ zero_extend 1) (_ bv1 128)) ((_ zero_extend 1) (_ bv1 128)))";
        assert!(matches!(fold(Value::Bool(wide.to_owned())), Value::Bool(value) if value == wide));
    }
}
