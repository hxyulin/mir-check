use super::symbolic::{self, Context, Op, Term, Value};
use std::collections::BTreeMap;
use syn::{BinOp, Expr, Lit, UnOp};

type IntegerType = (u32, bool);

pub fn uses_post_state(text: &str) -> Result<bool, String> {
    let expression = syn::parse_str::<Expr>(text).map_err(|error| error.to_string())?;
    post_state_reference(&expression, &[])
}

fn post_state_reference(expression: &Expr, bound: &[String]) -> Result<bool, String> {
    match expression {
        Expr::Paren(expr) => post_state_reference(&expr.expr, bound),
        Expr::Group(expr) => post_state_reference(&expr.expr, bound),
        Expr::Path(expr) => Ok(expr.qself.is_none()
            && expr.path.get_ident().is_some_and(|name| {
                let name = name.to_string();
                name.starts_with("final_") && !bound.contains(&name)
            })),
        Expr::Field(expr) => post_state_reference(&expr.base, bound),
        Expr::Index(expr) => Ok(
            post_state_reference(&expr.expr, bound)? | post_state_reference(&expr.index, bound)?
        ),
        Expr::Cast(expr) => post_state_reference(&expr.expr, bound),
        Expr::Unary(expr) => post_state_reference(&expr.expr, bound),
        Expr::Binary(expr) => Ok(
            post_state_reference(&expr.left, bound)? | post_state_reference(&expr.right, bound)?
        ),
        Expr::Lit(_) => Ok(false),
        Expr::MethodCall(expr) => expr.args.iter().try_fold(
            post_state_reference(&expr.receiver, bound)?,
            |found, argument| Ok(found | post_state_reference(argument, bound)?),
        ),
        Expr::Match(expr) => {
            expr.arms
                .iter()
                .try_fold(post_state_reference(&expr.expr, bound)?, |found, arm| {
                    let (_, binding) = option_pattern(arm)?;
                    let mut inner = bound.to_vec();
                    inner.extend(binding);
                    Ok(found | post_state_reference(&arm.body, &inner)?)
                })
        }
        _ => Err("unsupported contract expression".to_owned()),
    }
}

#[derive(Clone, Copy)]
enum ScalarType {
    Integer(u32, bool),
    Float(u32),
}

pub fn predicate(
    context: &Context,
    text: &str,
    bindings: &BTreeMap<String, Value>,
    pointer_bits: u32,
) -> Result<Term, String> {
    let expression = syn::parse_str::<Expr>(text).map_err(|error| error.to_string())?;
    evaluate(context, &expression, bindings, pointer_bits, None)?.boolean()
}

fn evaluate(
    context: &Context,
    expression: &Expr,
    bindings: &BTreeMap<String, Value>,
    pointer_bits: u32,
    expected: Option<ScalarType>,
) -> Result<Value, String> {
    match expression {
        Expr::Paren(expr) => evaluate(context, &expr.expr, bindings, pointer_bits, expected),
        Expr::Group(expr) => evaluate(context, &expr.expr, bindings, pointer_bits, expected),
        Expr::Path(expr) if expr.qself.is_none() && expr.path.get_ident().is_some() => {
            let name = expr.path.get_ident().unwrap().to_string();
            bindings
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("unknown contract name {name}"))
        }
        Expr::Field(expr) => {
            let name = match &expr.member {
                syn::Member::Named(name) => name.to_string(),
                syn::Member::Unnamed(index) => index.index.to_string(),
            };
            evaluate(context, &expr.base, bindings, pointer_bits, None)?.field(&name)
        }
        Expr::Index(expr) => {
            let Expr::Lit(index) = expr.index.as_ref() else {
                return Err("contract array index must be an integer literal".to_owned());
            };
            let Lit::Int(index) = &index.lit else {
                return Err("contract array index must be an integer literal".to_owned());
            };
            literal(
                context,
                index,
                false,
                pointer_bits,
                Some((pointer_bits, false)),
            )?;
            let index = index
                .base10_parse::<usize>()
                .map_err(|error| error.to_string())?;
            match evaluate(context, &expr.expr, bindings, pointer_bits, None)? {
                Value::Elements(elements) => elements
                    .get(index)
                    .cloned()
                    .ok_or("contract array index is out of bounds".to_owned()),
                Value::Bytes { length, data } => {
                    let (length, bits, signed) = length.integer()?;
                    if signed || bits != pointer_bits {
                        return Err("contract byte length must be target usize".to_owned());
                    }
                    let Some(symbolic::Constant::BitVec {
                        value: length,
                        bits,
                    }) = symbolic::constant(&length)
                    else {
                        return Err("contract byte indexing needs a fixed array length".to_owned());
                    };
                    if bits != pointer_bits || index as u128 >= length {
                        return Err("contract byte array index is out of bounds".to_owned());
                    }
                    Ok(Value::Int {
                        expression: context.apply(
                            Op::Select,
                            &[data, context.bit_vector(index as u128, pointer_bits)?],
                        )?,
                        bits: 8,
                        signed: false,
                    })
                }
                _ => Err("contract indexing requires a fixed array".to_owned()),
            }
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
            if matches!(name.as_str(), "f32" | "f64") {
                return symbolic::float_cast(
                    context,
                    evaluate(context, &expr.expr, bindings, pointer_bits, None)?,
                    if name == "f32" { 32 } else { 64 },
                );
            }
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
                context,
                evaluate(context, &expr.expr, bindings, pointer_bits, None)?,
                target.0,
                target.1,
            )
        }
        Expr::Match(expr) => {
            let option = evaluate(context, &expr.expr, bindings, pointer_bits, None)?;
            if let Value::Enum {
                discriminant,
                variants,
                is_option: true,
            } = &option
            {
                let (_, bits, signed) = discriminant.integer()?;
                let mut seen = [false; 2];
                let mut cases = Vec::new();
                for arm in &expr.arms {
                    let (tag, binding) = option_pattern(arm)?;
                    if seen[tag] {
                        return Err("duplicate Option contract arm".to_owned());
                    }
                    seen[tag] = true;
                    let Value::Adt {
                        fields,
                        discriminant: number,
                        ..
                    } = variants.get(tag).ok_or("missing Option variant")?
                    else {
                        return Err("invalid Option variant".to_owned());
                    };
                    let mut inner = bindings.clone();
                    if let Some(name) = binding {
                        let [(_, value)] = fields.as_slice() else {
                            return Err("invalid Some payload".to_owned());
                        };
                        inner.insert(name, value.clone());
                    }
                    let result =
                        evaluate(context, &arm.body, &inner, pointer_bits, expected)?.boolean()?;
                    let active = symbolic::binary(
                        context,
                        "eq",
                        (**discriminant).clone(),
                        symbolic::integer(context, *number, bits, signed),
                    )?
                    .boolean()?;
                    cases.push(context.apply(Op::And, &[active, result])?);
                }
                if !seen.iter().all(|arm| *arm) {
                    return Err("nonexhaustive Option contract match".to_owned());
                }
                return Ok(Value::Bool(context.apply(Op::Or, &cases)?));
            }
            let Value::Adt {
                variant,
                is_option: true,
                fields,
                ..
            } = option
            else {
                return Err("contract match only models supported Option values".to_owned());
            };
            let mut selected = None;
            let mut seen = [false; 2];
            for arm in &expr.arms {
                let (tag, binding) = option_pattern(arm)?;
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
                    selected = Some(evaluate(
                        context,
                        &arm.body,
                        &inner,
                        pointer_bits,
                        expected,
                    )?);
                }
            }
            if !seen.iter().all(|arm| *arm) {
                return Err("nonexhaustive Option contract match".to_owned());
            }
            selected.ok_or("unknown Option contract variant".to_owned())
        }
        Expr::Lit(expr) => match &expr.lit {
            Lit::Bool(value) => Ok(Value::Bool(context.boolean(value.value))),
            Lit::Int(value) => literal(
                context,
                value,
                false,
                pointer_bits,
                integer_expected(expected)?,
            ),
            Lit::Float(value) => float_literal(context, value, false, expected),
            _ => Err("unsupported contract literal".to_owned()),
        },
        Expr::Unary(expr) => match expr.op {
            UnOp::Not(_) => {
                let value =
                    evaluate(context, &expr.expr, bindings, pointer_bits, None)?.boolean()?;
                Ok(Value::Bool(symbolic::not(&value)))
            }
            UnOp::Neg(_) => {
                // Restrict negation to literals, with mathematical range checking before encoding.
                let Expr::Lit(lit) = expr.expr.as_ref() else {
                    return Err("contract negation only supports numeric literals".to_owned());
                };
                match &lit.lit {
                    Lit::Int(value) => literal(
                        context,
                        value,
                        true,
                        pointer_bits,
                        integer_expected(expected)?,
                    ),
                    Lit::Float(value) => float_literal(context, value, true, expected),
                    _ => Err("expected a numeric literal".to_owned()),
                }
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
                let left =
                    evaluate(context, &expr.left, bindings, pointer_bits, None)?.boolean()?;
                let right =
                    evaluate(context, &expr.right, bindings, pointer_bits, None)?.boolean()?;
                return symbolic::binary(context, operation, Value::Bool(left), Value::Bool(right));
            }
            let (left, right) = if untyped_number(&expr.left) && !untyped_number(&expr.right) {
                let right = evaluate(context, &expr.right, bindings, pointer_bits, None)?;
                let left = evaluate(
                    context,
                    &expr.left,
                    bindings,
                    pointer_bits,
                    scalar_type(&right),
                )?;
                (left, right)
            } else {
                let left = evaluate(context, &expr.left, bindings, pointer_bits, None)?;
                let right = evaluate(
                    context,
                    &expr.right,
                    bindings,
                    pointer_bits,
                    scalar_type(&left),
                )?;
                (left, right)
            };
            symbolic::binary(context, operation, left, right)
        }
        Expr::MethodCall(expr)
            if expr.method == "len" && expr.args.is_empty() && expr.turbofish.is_none() =>
        {
            match evaluate(context, &expr.receiver, bindings, pointer_bits, None)? {
                Value::Bytes { length, .. } => Ok(*length),
                Value::Elements(elements) => Ok(symbolic::integer(
                    context,
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

fn option_pattern(arm: &syn::Arm) -> Result<(usize, Option<String>), String> {
    if arm.guard.is_some() {
        return Err("contract match guards are unsupported".to_owned());
    }
    match &arm.pat {
        syn::Pat::Path(pattern) if pattern.path.is_ident("None") => Ok((0, None)),
        syn::Pat::Ident(pattern)
            if pattern.ident == "None"
                && pattern.subpat.is_none()
                && pattern.mutability.is_none()
                && pattern.by_ref.is_none() =>
        {
            Ok((0, None))
        }
        syn::Pat::TupleStruct(pattern)
            if pattern.path.is_ident("Some") && pattern.elems.len() == 1 =>
        {
            let syn::Pat::Ident(name) = &pattern.elems[0] else {
                return Err("Some requires a named contract binding".to_owned());
            };
            if name.by_ref.is_some() || name.mutability.is_some() || name.subpat.is_some() {
                return Err("unsupported Option contract binding".to_owned());
            }
            Ok((1, Some(name.ident.to_string())))
        }
        _ => Err("contract match requires None and Some(name) arms".to_owned()),
    }
}

fn scalar_type(value: &Value) -> Option<ScalarType> {
    match value {
        Value::Int { bits, signed, .. } => Some(ScalarType::Integer(*bits, *signed)),
        Value::Float { bits, .. } => Some(ScalarType::Float(*bits)),
        _ => None,
    }
}

fn integer_expected(expected: Option<ScalarType>) -> Result<Option<IntegerType>, String> {
    match expected {
        Some(ScalarType::Integer(bits, signed)) => Ok(Some((bits, signed))),
        Some(ScalarType::Float(_)) => Err("integer literal compared with a float".to_owned()),
        None => Ok(None),
    }
}

fn float_literal(
    context: &Context,
    literal: &syn::LitFloat,
    negative: bool,
    expected: Option<ScalarType>,
) -> Result<Value, String> {
    let explicit = match literal.suffix() {
        "" => None,
        "f32" => Some(32),
        "f64" => Some(64),
        _ => return Err("unsupported contract float suffix".to_owned()),
    };
    let expected = match expected {
        Some(ScalarType::Float(bits)) => Some(bits),
        Some(ScalarType::Integer(..)) => {
            return Err("float literal compared with an integer".to_owned());
        }
        None => None,
    };
    if explicit.is_some() && expected.is_some() && explicit != expected {
        return Err("contract float literal type mismatch".to_owned());
    }
    let bits = explicit.or(expected).unwrap_or(64);
    let raw = if bits == 32 {
        let value = literal
            .base10_parse::<f32>()
            .map_err(|error| error.to_string())?;
        if !value.is_finite() {
            return Err("contract float literal is out of range".to_owned());
        }
        u128::from(if negative { -value } else { value }.to_bits())
    } else {
        let value = literal
            .base10_parse::<f64>()
            .map_err(|error| error.to_string())?;
        if !value.is_finite() {
            return Err("contract float literal is out of range".to_owned());
        }
        u128::from(if negative { -value } else { value }.to_bits())
    };
    Ok(symbolic::float(context, raw, bits))
}

fn untyped_number(expression: &Expr) -> bool {
    match expression {
        Expr::Paren(expr) => untyped_number(&expr.expr),
        Expr::Group(expr) => untyped_number(&expr.expr),
        Expr::Unary(expr) if matches!(expr.op, UnOp::Neg(_)) => untyped_number(&expr.expr),
        Expr::Lit(expr) => match &expr.lit {
            Lit::Int(value) => value.suffix().is_empty(),
            Lit::Float(value) => value.suffix().is_empty(),
            _ => false,
        },
        _ => false,
    }
}

fn literal(
    context: &Context,
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
    Ok(symbolic::integer(context, value, bits, signed))
}

#[cfg(test)]
mod tests {
    use super::uses_post_state;

    #[test]
    fn post_state_uses_expression_paths_without_confusing_fields_comments_or_bound_names() {
        for predicate in [
            "total_final_count == result",
            "state.final_count == result",
            "result == 1 /* final_state */",
            "match result { Some(final_value) => final_value >= 1, None => true }",
            "match result { Some(value) => value.total_final_count >= 1, None => true }",
        ] {
            assert!(!uses_post_state(predicate).unwrap(), "{predicate}");
        }
        for predicate in [
            "final_state.count == result",
            "final_state.len() == 3",
            "(final_count as u32) == result",
            "match result { Some(final_value) => final_other >= final_value, None => true }",
            "match final_result { Some(value) => value >= 1, None => true }",
        ] {
            assert!(uses_post_state(predicate).unwrap(), "{predicate}");
        }
        assert!(uses_post_state("final_state ==").is_err());
        assert!(uses_post_state("unsupported(final_state)").is_err());
    }
    #[test]
    fn byte_contract_indices_require_fixed_target_lengths_and_valid_literal_bounds() {
        use super::{BTreeMap, Value, predicate, symbolic};
        for bits in [32, 64] {
            let context = &symbolic::Context::default();
            let mut bindings = BTreeMap::new();
            bindings.insert(
                "bytes".to_owned(),
                Value::Bytes {
                    length: Box::new(symbolic::integer(context, 3, bits, false)),
                    data: context
                        .symbol(
                            0,
                            symbolic::Sort::Array(
                                Box::new(symbolic::Sort::BitVec(bits)),
                                Box::new(symbolic::Sort::BitVec(8)),
                            ),
                        )
                        .unwrap(),
                },
            );
            let valid = predicate(context, "bytes[2] == 7_u8", &bindings, bits).unwrap();
            assert!(
                valid
                    .smt(200_000)
                    .unwrap()
                    .contains(&format!("(select v0 (_ bv2 {bits}))"))
            );
            for expression in [
                "bytes[3] == 7",
                "bytes[0_i32] == 7",
                "bytes[-1] == 7",
                "bytes[1 + 1] == 7",
            ] {
                assert!(
                    predicate(context, expression, &bindings, bits).is_err(),
                    "{expression}"
                );
            }
            for length in [
                Value::Int {
                    expression: context.symbol(1, symbolic::Sort::BitVec(bits)).unwrap(),
                    bits,
                    signed: false,
                },
                symbolic::integer(context, 3, bits, true),
                symbolic::integer(context, 3, 8, false),
            ] {
                bindings.insert(
                    "bytes".to_owned(),
                    Value::Bytes {
                        length: Box::new(length),
                        data: context
                            .symbol(
                                0,
                                symbolic::Sort::Array(
                                    Box::new(symbolic::Sort::BitVec(bits)),
                                    Box::new(symbolic::Sort::BitVec(8)),
                                ),
                            )
                            .unwrap(),
                    },
                );
                assert!(predicate(context, "bytes[0] == 7", &bindings, bits).is_err());
            }
        }
    }
}
