use super::{Constant, Context, Kind, Op, Sort, Term};
use std::collections::HashSet;

pub(super) fn simplify(context: &Context, op: Op, args: &[Term], sort: &Sort) -> Option<Term> {
    match (op, args) {
        (Op::Not, [value]) => match &value.node.kind {
            Kind::Constant(Constant::Bool(value)) => Some(context.boolean(!value)),
            Kind::Apply(Op::Not, inner) => Some(inner[0].clone()),
            Kind::Constant(Constant::BitVec(_) | Constant::Rounding(_))
            | Kind::Symbol(_)
            | Kind::Apply(_, _) => None,
        },
        (Op::Equal, [left, right]) if left == right => Some(context.boolean(true)),
        (Op::Equal, [left, right]) if left.sort() == &Sort::Bool => {
            for (constant, value) in [(left, right), (right, left)] {
                if let Some(Constant::Bool(flag)) = constant.constant() {
                    return if flag {
                        Some(value.clone())
                    } else {
                        context.apply(Op::Not, std::slice::from_ref(value)).ok()
                    };
                }
            }
            if left.id() > right.id() {
                context
                    .apply(Op::Equal, &[right.clone(), left.clone()])
                    .ok()
            } else {
                None
            }
        }
        (Op::Equal, [left, right])
            if matches!(left.sort(), Sort::BitVec(_)) && left.id() > right.id() =>
        {
            context
                .apply(Op::Equal, &[right.clone(), left.clone()])
                .ok()
        }
        (Op::BvUnsignedLt | Op::BvSignedLt, [left, right]) if left == right => {
            Some(context.boolean(false))
        }
        (Op::BvUnsignedLt, [_, right]) if matches!(right.constant(), Some(Constant::BitVec(0))) => {
            Some(context.boolean(false))
        }
        (Op::BvUnsignedLt, [left, right])
            if matches!(left.node.kind, Kind::Apply(Op::ZeroExtend(_), _))
                || matches!(right.node.kind, Kind::Apply(Op::ZeroExtend(_), _)) =>
        {
            unsigned_extended_comparison(context, left, right)
                .or_else(|| bit_vector(context, op, args, sort))
        }
        (Op::BvUnsignedGe | Op::BvSignedGe, [left, right]) => {
            let comparison = if op == Op::BvUnsignedGe {
                Op::BvUnsignedLt
            } else {
                Op::BvSignedLt
            };
            let less = context
                .apply(comparison, &[left.clone(), right.clone()])
                .ok()?;
            context.apply(Op::Not, &[less]).ok()
        }
        (Op::BvUnsignedGt | Op::BvSignedGt, [left, right]) => {
            let comparison = if op == Op::BvUnsignedGt {
                Op::BvUnsignedLt
            } else {
                Op::BvSignedLt
            };
            context
                .apply(comparison, &[right.clone(), left.clone()])
                .ok()
        }
        (Op::BvUnsignedLe | Op::BvSignedLe, [left, right]) => {
            let comparison = if op == Op::BvUnsignedLe {
                Op::BvUnsignedLt
            } else {
                Op::BvSignedLt
            };
            let less = context
                .apply(comparison, &[right.clone(), left.clone()])
                .ok()?;
            context.apply(Op::Not, &[less]).ok()
        }
        (Op::Ite, [condition, left, right]) => match condition.constant() {
            Some(Constant::Bool(value)) => Some(if value { left } else { right }.clone()),
            _ if left == right => Some(left.clone()),
            _ => None,
        },
        (Op::And | Op::Or, values) => {
            let identity = op == Op::And;
            let mut pending: Vec<_> = values.iter().rev().collect();
            let mut positive = HashSet::new();
            let mut negative = HashSet::new();
            let mut retained = Vec::new();
            while let Some(value) = pending.pop() {
                if let Kind::Apply(inner, arguments) = &value.node.kind
                    && *inner == op
                {
                    pending.extend(arguments.iter().rev());
                    continue;
                }
                if value.constant() == Some(Constant::Bool(!identity)) {
                    return Some(context.boolean(!identity));
                }
                if value.constant() == Some(Constant::Bool(identity)) {
                    continue;
                }
                let (same, opposite, id) = if let Some(inner) = value.negated() {
                    (&mut negative, &mut positive, inner.id())
                } else {
                    (&mut positive, &mut negative, value.id())
                };
                if opposite.contains(&id) {
                    return Some(context.boolean(!identity));
                }
                if same.insert(id) {
                    retained.push(value.clone());
                }
            }
            match retained.as_slice() {
                [] => Some(context.boolean(identity)),
                [only] => Some(only.clone()),
                _ if retained.len() != values.len()
                    || retained
                        .iter()
                        .zip(values)
                        .any(|(left, right)| left != right) =>
                {
                    context.apply(op, &retained).ok()
                }
                _ => None,
            }
        }
        (Op::Xor, [left, right]) => match (left.constant(), right.constant()) {
            (Some(Constant::Bool(left)), Some(Constant::Bool(right))) => {
                Some(context.boolean(left ^ right))
            }
            _ if left == right => Some(context.boolean(false)),
            _ => None,
        },
        (Op::Equal, [left, right]) => match (left.constant(), right.constant()) {
            (Some(left), Some(right)) => Some(context.boolean(left == right)),
            _ => None,
        },
        (Op::Select, [array, index]) => match &array.node.kind {
            Kind::Apply(Op::ConstArray { .. }, values) => Some(values[0].clone()),
            Kind::Apply(Op::Store, values) if &values[1] == index => Some(values[2].clone()),
            Kind::Constant(_) | Kind::Symbol(_) | Kind::Apply(_, _) => None,
        },
        _ => bit_vector(context, op, args, sort),
    }
}

fn unsigned_extended_comparison(context: &Context, left: &Term, right: &Term) -> Option<Term> {
    for (extended, constant, reverse) in [(left, right, false), (right, left, true)] {
        let Kind::Apply(Op::ZeroExtend(_), arguments) = &extended.node.kind else {
            continue;
        };
        let Some(Constant::BitVec(value)) = constant.constant() else {
            continue;
        };
        let original = &arguments[0];
        let Sort::BitVec(bits) = original.sort() else {
            return None;
        };
        if *bits < 128 && value >= 1_u128 << bits {
            return Some(context.boolean(!reverse));
        }
        let narrowed = context.bit_vector(value, *bits).ok()?;
        let operands = if reverse {
            [narrowed, original.clone()]
        } else {
            [original.clone(), narrowed]
        };
        return context.apply(Op::BvUnsignedLt, &operands).ok();
    }
    None
}

fn mask(bits: u32) -> Option<u128> {
    (1..=128).contains(&bits).then(|| u128::MAX >> (128 - bits))
}

fn signed(value: u128, bits: u32) -> i128 {
    if value & (1_u128 << (bits - 1)) == 0 {
        value as i128
    } else {
        (value | !mask(bits).unwrap()) as i128
    }
}

fn bit_vector(context: &Context, op: Op, args: &[Term], sort: &Sort) -> Option<Term> {
    let mut values = Vec::new();
    for argument in args {
        let Constant::BitVec(value) = argument.constant()? else {
            return None;
        };
        let Sort::BitVec(bits) = argument.sort() else {
            return None;
        };
        mask(*bits)?;
        values.push((value, *bits));
    }
    let result = match (op, values.as_slice()) {
        (Op::BvNot, [(value, _)]) => !value,
        (Op::BvNeg, [(value, _)]) => value.wrapping_neg(),
        (Op::ZeroExtend(_), [(value, _)]) => *value,
        (Op::SignExtend(_), [(value, bits)]) => signed(*value, *bits) as u128,
        (Op::Extract { low, .. }, [(value, _)]) => value >> low,
        (Op::Concat, [(left, _), (right, right_bits)]) if *right_bits < 128 => {
            (left << right_bits) | right
        }
        (Op::BvAdd, [(left, _), (right, _)]) => left.wrapping_add(*right),
        (Op::BvSub, [(left, _), (right, _)]) => left.wrapping_sub(*right),
        (Op::BvMul, [(left, _), (right, _)]) => left.wrapping_mul(*right),
        (Op::BvAnd, [(left, _), (right, _)]) => left & right,
        (Op::BvOr, [(left, _), (right, _)]) => left | right,
        (Op::BvXor, [(left, _), (right, _)]) => left ^ right,
        (Op::BvUnsignedDiv, [(left, bits), (right, _)]) => {
            if *right == 0 {
                mask(*bits)?
            } else {
                left / right
            }
        }
        (Op::BvUnsignedRem, [(left, _), (right, _)]) => {
            if *right == 0 {
                *left
            } else {
                left % right
            }
        }
        (Op::BvSignedDiv, [(left, bits), (right, _)]) => {
            let left = signed(*left, *bits);
            let right = signed(*right, *bits);
            if right == 0 {
                if left < 0 { 1 } else { mask(*bits)? }
            } else {
                left.wrapping_div(right) as u128
            }
        }
        (Op::BvSignedRem, [(left, bits), (right, _)]) => {
            let left = signed(*left, *bits);
            let right = signed(*right, *bits);
            if right == 0 {
                left as u128
            } else {
                left.wrapping_rem(right) as u128
            }
        }
        (Op::BvShiftLeft, [(left, bits), (right, _)]) => {
            if right >= &u128::from(*bits) {
                0
            } else {
                left << right
            }
        }
        (Op::BvLogicalShiftRight, [(left, bits), (right, _)]) => {
            if right >= &u128::from(*bits) {
                0
            } else {
                left >> right
            }
        }
        (Op::BvArithmeticShiftRight, [(left, bits), (right, _)]) => {
            let left = signed(*left, *bits);
            if right >= &u128::from(*bits) {
                if left < 0 { mask(*bits)? } else { 0 }
            } else {
                (left >> right) as u128
            }
        }
        (
            comparison @ (Op::BvUnsignedLt
            | Op::BvUnsignedLe
            | Op::BvUnsignedGt
            | Op::BvUnsignedGe
            | Op::BvSignedLt
            | Op::BvSignedLe
            | Op::BvSignedGt
            | Op::BvSignedGe),
            [(left, bits), (right, _)],
        ) => {
            let ordering = match comparison {
                Op::BvSignedLt | Op::BvSignedLe | Op::BvSignedGt | Op::BvSignedGe => {
                    signed(*left, *bits).cmp(&signed(*right, *bits))
                }
                _ => left.cmp(right),
            };
            let value = match comparison {
                Op::BvUnsignedLt | Op::BvSignedLt => ordering.is_lt(),
                Op::BvUnsignedLe | Op::BvSignedLe => ordering.is_le(),
                Op::BvUnsignedGt | Op::BvSignedGt => ordering.is_gt(),
                Op::BvUnsignedGe | Op::BvSignedGe => ordering.is_ge(),
                _ => unreachable!("only bit-vector comparisons reach this match"),
            };
            return Some(context.boolean(value));
        }
        _ => return None,
    };
    let Sort::BitVec(bits) = sort else {
        return None;
    };
    mask(*bits)?;
    context.bit_vector(result, *bits).ok()
}
