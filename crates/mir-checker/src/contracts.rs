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
        Expr::Field(expr) => {
            let syn::Member::Named(name) = &expr.member else {
                return Err("only named contract fields are modeled".to_owned());
            };
            evaluate(&expr.base, bindings, pointer_bits, None)?.field(&name.to_string())
        }
        Expr::Cast(expr) => {
            let syn::Type::Path(ty) = expr.ty.as_ref() else {
                return Err("unsupported contract cast type".to_owned());
            };
            let name = ty
                .path
                .get_ident()
                .ok_or("qualified contract cast type")?
                .to_string();
            let target = match name.as_str() {
                "u8" => (8, false),
                "u16" => (16, false),
                "u32" => (32, false),
                "u64" => (64, false),
                "u128" => (128, false),
                "usize" => (pointer_bits, false),
                "i8" => (8, true),
                "i16" => (16, true),
                "i32" => (32, true),
                "i64" => (64, true),
                "i128" => (128, true),
                "isize" => (pointer_bits, true),
                _ => return Err("unsupported contract cast target".to_owned()),
            };
            symbolic::cast(
                evaluate(&expr.expr, bindings, pointer_bits, None)?,
                target.0,
                target.1,
            )
        }
        Expr::Match(expr) => {
            let Value::Adt {
                variant,
                is_option: true,
                fields,
                ..
            } = evaluate(&expr.expr, bindings, pointer_bits, None)?
            else {
                return Err("contract match only models constructed Option values".to_owned());
            };
            let mut selected = None;
            let mut seen = [false; 2];
            for arm in &expr.arms {
                if arm.guard.is_some() {
                    return Err("contract match guards are unsupported".to_owned());
                }
                let (tag, binding) = match &arm.pat {
                    syn::Pat::Path(pattern) if pattern.path.is_ident("None") => (0, None),
                    syn::Pat::Ident(pattern)
                        if pattern.ident == "None"
                            && pattern.subpat.is_none()
                            && pattern.mutability.is_none()
                            && pattern.by_ref.is_none() =>
                    {
                        (0, None)
                    }
                    syn::Pat::TupleStruct(pattern)
                        if pattern.path.is_ident("Some") && pattern.elems.len() == 1 =>
                    {
                        let syn::Pat::Ident(name) = &pattern.elems[0] else {
                            return Err("Some requires a named contract binding".to_owned());
                        };
                        if name.by_ref.is_some()
                            || name.mutability.is_some()
                            || name.subpat.is_some()
                        {
                            return Err("unsupported Option contract binding".to_owned());
                        }
                        (1, Some(name.ident.to_string()))
                    }
                    _ => return Err("contract match requires None and Some(name) arms".to_owned()),
                };
                if seen[tag] {
                    return Err("duplicate Option contract arm".to_owned());
                }
                seen[tag] = true;
                if variant == tag {
                    let mut inner = bindings.clone();
                    if let Some(name) = binding {
                        let [(_, value)] = fields.as_slice() else {
                            return Err("invalid Some payload".to_owned());
                        };
                        inner.insert(name, value.clone());
                    }
                    selected = Some(evaluate(&arm.body, &inner, pointer_bits, expected)?);
                }
            }
            if !seen.iter().all(|arm| *arm) {
                return Err("nonexhaustive Option contract match".to_owned());
            }
            selected.ok_or("unknown Option contract variant".to_owned())
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
                Value::Elements(elements) => Ok(symbolic::integer(
                    elements.len() as u128,
                    pointer_bits,
                    false,
                )),
                _ => Err("contract len requires a modeled slice or array".to_owned()),
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
