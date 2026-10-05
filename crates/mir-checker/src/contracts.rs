use super::symbolic::{self, Value};
use std::collections::BTreeMap;
use syn::{BinOp, Expr, Lit, UnOp};

type IntegerType = (u32, bool);

pub fn predicate(
    text: &str,
    bindings: &BTreeMap<String, Value>,
    pointer_bits: u32,
) -> Result<String, String> {
    let expression = syn::parse_str::<Expr>(text).map_err(|error| error.to_string())?;
    evaluate(&expression, bindings, pointer_bits, None)?.boolean()
}

fn evaluate(
    expression: &Expr,
    bindings: &BTreeMap<String, Value>,
    pointer_bits: u32,
    expected: Option<IntegerType>,
) -> Result<Value, String> {
    match expression {
        Expr::Paren(expr) => evaluate(&expr.expr, bindings, pointer_bits, expected),
        Expr::Group(expr) => evaluate(&expr.expr, bindings, pointer_bits, expected),
        Expr::Path(expr) if expr.qself.is_none() && expr.path.get_ident().is_some() => {
            let name = expr.path.get_ident().unwrap().to_string();
            bindings
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("unknown contract name {name}"))
        }
        Expr::Lit(expr) => match &expr.lit {
            Lit::Bool(value) => Ok(Value::Bool(value.value.to_string())),
            Lit::Int(value) => literal(value, false, pointer_bits, expected),
            _ => Err("unsupported contract literal".to_owned()),
        },
        Expr::Unary(expr) => match expr.op {
            UnOp::Not(_) => {
                let value = evaluate(&expr.expr, bindings, pointer_bits, None)?.boolean()?;
                Ok(Value::Bool(symbolic::not(&value)))
            }
            UnOp::Neg(_) => {
                // Restrict negation to literals, with mathematical range checking before encoding.
                let Expr::Lit(lit) = expr.expr.as_ref() else {
                    return Err("contract negation only supports integer literals".to_owned());
                };
                let Lit::Int(value) = &lit.lit else {
                    return Err("expected an integer literal".to_owned());
                };
                literal(value, true, pointer_bits, expected)
            }
            _ => Err("unsupported contract unary expression".to_owned()),
        },
        Expr::Binary(expr) => {
            let operation = match expr.op {
                BinOp::Eq(_) => "eq",
                BinOp::Ne(_) => "ne",
                BinOp::Lt(_) => "lt",
                BinOp::Le(_) => "le",
                BinOp::Gt(_) => "gt",
                BinOp::Ge(_) => "ge",
                BinOp::And(_) => "and",
                BinOp::Or(_) => "or",
                _ => return Err("contract arithmetic and mutation are unsupported".to_owned()),
            };
            if matches!(expr.op, BinOp::And(_) | BinOp::Or(_)) {
                let left = evaluate(&expr.left, bindings, pointer_bits, None)?.boolean()?;
                let right = evaluate(&expr.right, bindings, pointer_bits, None)?.boolean()?;
                return symbolic::binary(operation, Value::Bool(left), Value::Bool(right));
            }
            let (left, right) = if untyped_integer(&expr.left) && !untyped_integer(&expr.right) {
                let right = evaluate(&expr.right, bindings, pointer_bits, None)?;
                let left = evaluate(&expr.left, bindings, pointer_bits, integer_type(&right))?;
                (left, right)
            } else {
                let left = evaluate(&expr.left, bindings, pointer_bits, None)?;
                let right = evaluate(&expr.right, bindings, pointer_bits, integer_type(&left))?;
                (left, right)
            };
            symbolic::binary(operation, left, right)
        }
        Expr::MethodCall(expr)
            if expr.method == "len" && expr.args.is_empty() && expr.turbofish.is_none() =>
        {
            match evaluate(&expr.receiver, bindings, pointer_bits, None)? {
                Value::Bytes { length, .. } => Ok(*length),
                _ => Err("contract len requires a byte slice or array".to_owned()),
            }
        }
        _ => Err("unsupported or impure contract expression".to_owned()),
    }
}

fn integer_type(value: &Value) -> Option<IntegerType> {
    match value {
        Value::Int { bits, signed, .. } => Some((*bits, *signed)),
        _ => None,
    }
}

fn untyped_integer(expression: &Expr) -> bool {
    match expression {
        Expr::Paren(expr) => untyped_integer(&expr.expr),
        Expr::Group(expr) => untyped_integer(&expr.expr),
        Expr::Unary(expr) if matches!(expr.op, UnOp::Neg(_)) => untyped_integer(&expr.expr),
        Expr::Lit(expr) => matches!(&expr.lit, Lit::Int(value) if value.suffix().is_empty()),
        _ => false,
    }
}

fn literal(
    literal: &syn::LitInt,
    negative: bool,
    pointer_bits: u32,
    expected: Option<IntegerType>,
) -> Result<Value, String> {
    let explicit = match literal.suffix() {
        "" => None,
        "u8" => Some((8, false)),
        "u16" => Some((16, false)),
        "u32" => Some((32, false)),
        "u64" => Some((64, false)),
        "u128" => Some((128, false)),
        "usize" => Some((pointer_bits, false)),
        "i8" => Some((8, true)),
        "i16" => Some((16, true)),
        "i32" => Some((32, true)),
        "i64" => Some((64, true)),
        "i128" => Some((128, true)),
        "isize" => Some((pointer_bits, true)),
        _ => return Err("unknown integer suffix in contract".to_owned()),
    };
    if let (Some(explicit), Some(expected)) = (explicit, expected)
        && explicit != expected
    {
        return Err("contract integer literal type mismatch".to_owned());
    }
    let (bits, signed) = explicit.or(expected).unwrap_or((32, true));
    if negative && !signed {
        return Err("negative unsigned contract literal".to_owned());
    }
    let magnitude = literal
        .base10_parse::<u128>()
        .map_err(|error| error.to_string())?;
    let max = if signed {
        (1_u128 << (bits - 1)) - u128::from(!negative)
    } else if bits == 128 {
        u128::MAX
    } else {
        (1_u128 << bits) - 1
    };
    if magnitude > max {
        return Err("contract integer literal is out of range".to_owned());
    }
    let value = if negative {
        let mask = if bits == 128 {
            u128::MAX
        } else {
            (1_u128 << bits) - 1
        };
        magnitude.wrapping_neg() & mask
    } else {
        magnitude
    };
    Ok(symbolic::integer(value, bits, signed))
}
