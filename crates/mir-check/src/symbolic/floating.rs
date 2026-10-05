use super::{Value, not};

fn format(bits: u32) -> (u32, u32) {
    match bits {
        32 => (8, 24),
        64 => (11, 53),
        _ => unreachable!("only binary32 and binary64 values are constructed"),
    }
}

pub fn float_sort(bits: u32) -> String {
    let (exponent, significand) = format(bits);
    format!("(_ FloatingPoint {exponent} {significand})")
}

pub fn float(raw: u128, bits: u32) -> Value {
    let (exponent, significand) = format(bits);
    Value::Float {
        expression: format!("((_ to_fp {exponent} {significand}) (_ bv{raw} {bits}))"),
        bits,
    }
}

pub fn binary(operation: &str, left: Value, right: Value) -> Result<Value, String> {
    let (
        Value::Float {
            expression: left,
            bits,
        },
        Value::Float {
            expression: right,
            bits: other,
        },
    ) = (left, right)
    else {
        return Err("expected floating-point operands".to_owned());
    };
    if bits != other {
        return Err("floating-point operand types do not match".to_owned());
    }
    let comparison = match operation {
        "eq" => Some(format!("(fp.eq {left} {right})")),
        "ne" => Some(not(&format!("(fp.eq {left} {right})"))),
        "lt" => Some(format!("(fp.lt {left} {right})")),
        "le" => Some(format!("(fp.leq {left} {right})")),
        "gt" => Some(format!("(fp.gt {left} {right})")),
        "ge" => Some(format!("(fp.geq {left} {right})")),
        _ => None,
    };
    if let Some(expression) = comparison {
        return Ok(Value::Bool(expression));
    }
    let operator = match operation {
        "add" => "fp.add",
        "sub" => "fp.sub",
        "mul" => "fp.mul",
        "div" => "fp.div",
        // SMT fp.rem uses the IEEE nearest-integer quotient, unlike Rust's %.
        _ => return Err(format!("unsupported floating-point operation {operation}")),
    };
    Ok(Value::Float {
        expression: format!("({operator} RNE {left} {right})"),
        bits,
    })
}

pub fn float_cast(value: Value, bits: u32) -> Result<Value, String> {
    let (exponent, significand) = format(bits);
    let (operator, expression) = match value {
        Value::Int {
            expression, signed, ..
        } => (if signed { "to_fp" } else { "to_fp_unsigned" }, expression),
        Value::Float {
            expression,
            bits: old_bits,
        } => {
            if bits == old_bits {
                return Ok(Value::Float { expression, bits });
            }
            ("to_fp", expression)
        }
        _ => return Err("float conversion requires an integer or float".to_owned()),
    };
    Ok(Value::Float {
        expression: format!("((_ {operator} {exponent} {significand}) RNE {expression})"),
        bits,
    })
}

pub fn integer_cast(value: Value, bits: u32, signed: bool) -> Result<Value, String> {
    let Value::Float {
        expression,
        bits: source_bits,
    } = value
    else {
        return Err("expected a floating-point value".to_owned());
    };
    let (exponent, significand) = format(source_bits);
    let magnitude_bits = bits + 1;
    let power = if signed { bits - 1 } else { bits };
    let upper = format!(
        "((_ to_fp_unsigned {exponent} {significand}) RNE \
         (bvshl (_ bv1 {magnitude_bits}) (_ bv{power} {magnitude_bits})))"
    );
    let max = if signed {
        (1_u128 << (bits - 1)) - 1
    } else if bits == 128 {
        u128::MAX
    } else {
        (1_u128 << bits) - 1
    };
    let min = if signed { 1_u128 << (bits - 1) } else { 0 };
    let lower = if signed {
        format!("(fp.neg {upper})")
    } else {
        format!("(_ +zero {exponent} {significand})")
    };
    let convert = if signed { "fp.to_sbv" } else { "fp.to_ubv" };
    Ok(Value::Int {
        expression: format!(
            "(ite (fp.isNaN {expression}) (_ bv0 {bits}) \
             (ite (fp.geq {expression} {upper}) (_ bv{max} {bits}) \
             (ite (fp.leq {expression} {lower}) (_ bv{min} {bits}) \
             ((_ {convert} {bits}) RTZ {expression}))))"
        ),
        bits,
        signed,
    })
}
