use super::{Context, Op, Value, not};
use mir_check::smt::Rounding;

fn format(bits: u32) -> (u32, u32) {
    match bits {
        32 => (8, 24),
        64 => (11, 53),
        _ => unreachable!("only binary32 and binary64 values are constructed"),
    }
}

pub fn float(context: &Context, raw: u128, bits: u32) -> Value {
    float_from_bits(
        context,
        context.bit_vector(raw, bits).expect("float storage width"),
        bits,
    )
}

pub fn float_from_bits(context: &Context, raw_bits: super::Term, bits: u32) -> Value {
    let (exponent, significand) = format(bits);
    Value::Float {
        expression: context
            .apply(
                Op::FloatFromBits {
                    exponent,
                    significand,
                },
                std::slice::from_ref(&raw_bits),
            )
            .expect("matching float storage"),
        bits,
        raw_bits: Some(raw_bits),
    }
}

pub fn float_negate(context: &Context, value: Value) -> Result<Value, String> {
    let Value::Float {
        expression,
        bits,
        raw_bits,
    } = value
    else {
        return Err("floating-point negation requires a float".to_owned());
    };
    Ok(Value::Float {
        expression: context.apply(Op::FpNeg, &[expression])?,
        bits,
        raw_bits: raw_bits
            .map(|raw| {
                context.apply(
                    Op::BvXor,
                    &[raw, context.bit_vector(1_u128 << (bits - 1), bits)?],
                )
            })
            .transpose()?,
    })
}

pub fn binary(
    context: &Context,
    operation: &str,
    left: Value,
    right: Value,
) -> Result<Value, String> {
    let (
        Value::Float {
            expression: a,
            bits,
            ..
        },
        Value::Float {
            expression: b,
            bits: other,
            ..
        },
    ) = (left, right)
    else {
        return Err("expected floating-point operands".to_owned());
    };
    if bits != other {
        return Err("floating-point operand types do not match".to_owned());
    }
    let comparison = match operation {
        "eq" | "ne" => Some(Op::FpEqual),
        "lt" => Some(Op::FpLt),
        "le" => Some(Op::FpLe),
        "gt" => Some(Op::FpGt),
        "ge" => Some(Op::FpGe),
        _ => None,
    };
    if let Some(op) = comparison {
        let term = context.apply(op, &[a, b])?;
        return Ok(Value::Bool(if operation == "ne" {
            not(&term)
        } else {
            term
        }));
    }
    let op = match operation {
        "add" => Op::FpAdd,
        "sub" => Op::FpSub,
        "mul" => Op::FpMul,
        "div" => Op::FpDiv,
        // SMT fp.rem uses the IEEE nearest-integer quotient, unlike Rust's %.
        _ => return Err(format!("unsupported floating-point operation {operation}")),
    };
    Ok(Value::Float {
        expression: context.apply(op, &[context.rounding(Rounding::NearestEven), a, b])?,
        bits,
        raw_bits: None,
    })
}

pub fn float_cast(context: &Context, value: Value, bits: u32) -> Result<Value, String> {
    let (exponent, significand) = format(bits);
    let (op, expression) = match value {
        Value::Int {
            expression, signed, ..
        } => (
            if signed {
                Op::SignedToFloat {
                    exponent,
                    significand,
                }
            } else {
                Op::UnsignedToFloat {
                    exponent,
                    significand,
                }
            },
            expression,
        ),
        Value::Float {
            expression,
            bits: old,
            raw_bits,
        } => {
            if bits == old {
                return Ok(Value::Float {
                    expression,
                    bits,
                    raw_bits,
                });
            }
            (
                Op::FloatToFloat {
                    exponent,
                    significand,
                },
                expression,
            )
        }
        _ => return Err("float conversion requires an integer or float".to_owned()),
    };
    Ok(Value::Float {
        expression: context.apply(op, &[context.rounding(Rounding::NearestEven), expression])?,
        bits,
        raw_bits: None,
    })
}

pub fn integer_cast(
    context: &Context,
    value: Value,
    bits: u32,
    signed: bool,
) -> Result<Value, String> {
    let Value::Float {
        expression,
        bits: source_bits,
        ..
    } = value
    else {
        return Err("expected a floating-point value".to_owned());
    };
    let (exponent, significand) = format(source_bits);
    let magnitude_bits = bits + 1;
    let power = if signed { bits - 1 } else { bits };
    let magnitude = context.apply(
        Op::BvShiftLeft,
        &[
            context.bit_vector(1, magnitude_bits)?,
            context.bit_vector(u128::from(power), magnitude_bits)?,
        ],
    )?;
    let upper = context.apply(
        Op::UnsignedToFloat {
            exponent,
            significand,
        },
        &[context.rounding(Rounding::NearestEven), magnitude],
    )?;
    let max = if signed {
        (1_u128 << (bits - 1)) - 1
    } else if bits == 128 {
        u128::MAX
    } else {
        (1_u128 << bits) - 1
    };
    let min = if signed { 1_u128 << (bits - 1) } else { 0 };
    let lower = if signed {
        context.apply(Op::FpNeg, std::slice::from_ref(&upper))?
    } else {
        context.apply(
            Op::PositiveZero {
                exponent,
                significand,
            },
            &[],
        )?
    };
    let convert = context.apply(
        if signed {
            Op::FloatToSigned(bits)
        } else {
            Op::FloatToUnsigned(bits)
        },
        &[context.rounding(Rounding::TowardZero), expression.clone()],
    )?;
    let below = context.apply(Op::FpLe, &[expression.clone(), lower])?;
    let low_result = context.apply(Op::Ite, &[below, context.bit_vector(min, bits)?, convert])?;
    let above = context.apply(Op::FpGe, &[expression.clone(), upper])?;
    let high_result = context.apply(
        Op::Ite,
        &[above, context.bit_vector(max, bits)?, low_result],
    )?;
    let nan = context.apply(Op::FpIsNaN, &[expression])?;
    Ok(Value::Int {
        expression: context.apply(Op::Ite, &[nan, context.bit_vector(0, bits)?, high_result])?,
        bits,
        signed,
    })
}
