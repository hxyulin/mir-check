use super::{Constant, Kind, Op, Rounding, Sort, Term};
use std::collections::{HashMap, HashSet};

pub(super) fn print(root: &Term, max_bytes: usize) -> Result<String, String> {
    let mut nodes = HashMap::new();
    let mut references = HashMap::<usize, usize>::new();
    let mut pending = vec![root.clone()];
    while let Some(term) = pending.pop() {
        if nodes.contains_key(&term.node.id) {
            continue;
        }
        if let Kind::Apply(_, children) = &term.node.kind {
            for child in children {
                *references.entry(child.node.id).or_default() += 1;
                pending.push(child.clone());
            }
        }
        nodes.insert(term.node.id, term);
    }
    let mut ordered: Vec<_> = nodes.values().collect();
    ordered.sort_by_key(|term| term.node.id);
    let mut costs = HashMap::<usize, usize>::new();
    let mut shared = HashSet::new();
    for term in &ordered {
        let cost = match &term.node.kind {
            Kind::Constant(value) => literal(*value, term.sort()).len(),
            Kind::Symbol(index) => format!("v{index}").len(),
            Kind::Apply(op, children) => children
                .iter()
                .fold(operator(*op, term.sort()).len() + 1, |cost, child| {
                    cost.saturating_add(1 + costs[&child.node.id])
                }),
        };
        let count = references.get(&term.node.id).copied().unwrap_or_default();
        let name_bytes = format!("t{}", term.node.id).len();
        let binding_bytes = 12 + name_bytes;
        if matches!(term.node.kind, Kind::Apply(_, _))
            && count > 1
            && cost.saturating_sub(name_bytes).saturating_mul(count - 1)
                > binding_bytes + name_bytes
        {
            shared.insert(term.node.id);
            costs.insert(term.node.id, name_bytes);
        } else {
            costs.insert(term.node.id, cost.min(usize::MAX / 4));
        }
    }
    let mut output = Output {
        text: String::new(),
        max_bytes,
    };
    let mut available = HashSet::new();
    for term in ordered {
        if shared.contains(&term.node.id) {
            output.push(&format!("(let ((t{} ", term.node.id))?;
            expression(term, &available, &mut output)?;
            output.push(")) ")?;
            available.insert(term.node.id);
        }
    }
    expression(root, &available, &mut output)?;
    for _ in &available {
        output.push(")")?;
    }
    Ok(output.text)
}

struct Output {
    text: String,
    max_bytes: usize,
}

impl Output {
    fn push(&mut self, value: &str) -> Result<(), String> {
        if value.len() > self.max_bytes.saturating_sub(self.text.len()) {
            return Err("printed term exceeds the SMT byte budget".to_owned());
        }
        self.text.push_str(value);
        Ok(())
    }
}

enum Action<'a> {
    Term(&'a Term),
    Space,
    Close,
    Text(String),
}

fn expression(root: &Term, shared: &HashSet<usize>, output: &mut Output) -> Result<(), String> {
    let mut pending = vec![Action::Term(root)];
    while let Some(action) = pending.pop() {
        let term = match action {
            Action::Space => {
                output.push(" ")?;
                continue;
            }
            Action::Close => {
                output.push(")")?;
                continue;
            }
            Action::Text(text) => {
                output.push(&text)?;
                continue;
            }
            Action::Term(term) => term,
        };
        if shared.contains(&term.node.id) {
            output.push(&format!("t{}", term.node.id))?;
            continue;
        }
        match &term.node.kind {
            Kind::Constant(value) => output.push(&literal(*value, term.sort()))?,
            Kind::Symbol(index) => output.push(&format!("v{index}"))?,
            Kind::Apply(Op::ArrayOffset, children) => {
                let name = format!("i{}", term.node.id);
                output.push(&format!(
                    "(lambda (({name} {})) (select ",
                    sort(children[1].sort())
                ))?;
                pending.push(Action::Text(format!(" {name})))")));
                pending.push(Action::Term(&children[1]));
                pending.push(Action::Text(" (bvadd ".to_owned()));
                pending.push(Action::Term(&children[0]));
            }
            Kind::Apply(op, children) => {
                output.push(&operator(*op, term.sort()))?;
                pending.push(Action::Close);
                for child in children.iter().rev() {
                    pending.push(Action::Term(child));
                    pending.push(Action::Space);
                }
            }
        }
    }
    Ok(())
}

fn literal(value: Constant, sort: &Sort) -> String {
    match value {
        Constant::Bool(value) => value.to_string(),
        Constant::BitVec(value) => {
            let Sort::BitVec(bits) = sort else {
                unreachable!("bit-vector constants are constructed with a bit-vector sort");
            };
            format!("(_ bv{value} {bits})")
        }
        Constant::Rounding(Rounding::NearestEven) => "RNE".to_owned(),
        Constant::Rounding(Rounding::NearestAway) => "RNA".to_owned(),
        Constant::Rounding(Rounding::TowardPositive) => "RTP".to_owned(),
        Constant::Rounding(Rounding::TowardNegative) => "RTN".to_owned(),
        Constant::Rounding(Rounding::TowardZero) => "RTZ".to_owned(),
    }
}

pub(super) fn sort(sort: &Sort) -> String {
    match sort {
        Sort::Bool => "Bool".to_owned(),
        Sort::BitVec(bits) => format!("(_ BitVec {bits})"),
        Sort::Float {
            exponent,
            significand,
        } => format!("(_ FloatingPoint {exponent} {significand})"),
        Sort::RoundingMode => "RoundingMode".to_owned(),
        Sort::Array(index, element) => {
            format!("(Array {} {})", self::sort(index), self::sort(element))
        }
    }
}

pub(super) fn declaration(index: u32, value_sort: &Sort) -> String {
    format!("(declare-const v{index} {})", sort(value_sort))
}

fn operator(op: Op, result_sort: &Sort) -> String {
    let name = match op {
        Op::Not => "not",
        Op::And => "and",
        Op::Or => "or",
        Op::Xor => "xor",
        Op::Equal => "=",
        Op::Ite => "ite",
        Op::BvNot => "bvnot",
        Op::BvNeg => "bvneg",
        Op::BvAdd => "bvadd",
        Op::BvSub => "bvsub",
        Op::BvMul => "bvmul",
        Op::BvUnsignedDiv => "bvudiv",
        Op::BvSignedDiv => "bvsdiv",
        Op::BvUnsignedRem => "bvurem",
        Op::BvSignedRem => "bvsrem",
        Op::BvAnd => "bvand",
        Op::BvOr => "bvor",
        Op::BvXor => "bvxor",
        Op::BvShiftLeft => "bvshl",
        Op::BvLogicalShiftRight => "bvlshr",
        Op::BvArithmeticShiftRight => "bvashr",
        Op::BvUnsignedLt => "bvult",
        Op::BvUnsignedLe => "bvule",
        Op::BvUnsignedGt => "bvugt",
        Op::BvUnsignedGe => "bvuge",
        Op::BvSignedLt => "bvslt",
        Op::BvSignedLe => "bvsle",
        Op::BvSignedGt => "bvsgt",
        Op::BvSignedGe => "bvsge",
        Op::Extract { high, low } => return format!("((_ extract {high} {low})"),
        Op::ZeroExtend(extra) => return format!("((_ zero_extend {extra})"),
        Op::SignExtend(extra) => return format!("((_ sign_extend {extra})"),
        Op::Concat => "concat",
        Op::Select => "select",
        Op::Store => "store",
        Op::ArrayOffset => "lambda",
        Op::ConstArray { .. } => return format!("((as const {})", sort(result_sort)),
        Op::FpNeg => "fp.neg",
        Op::FpAbs => "fp.abs",
        Op::FpIsNaN => "fp.isNaN",
        Op::FpIsInfinite => "fp.isInfinite",
        Op::FpEqual => "fp.eq",
        Op::FpLt => "fp.lt",
        Op::FpLe => "fp.leq",
        Op::FpGt => "fp.gt",
        Op::FpGe => "fp.geq",
        Op::FpAdd => "fp.add",
        Op::FpSub => "fp.sub",
        Op::FpMul => "fp.mul",
        Op::FpDiv => "fp.div",
        Op::FpRoundToIntegral => "fp.roundToIntegral",
        Op::FpSqrt => "fp.sqrt",
        Op::FpFma => "fp.fma",
        Op::FloatFromBits {
            exponent,
            significand,
        }
        | Op::SignedToFloat {
            exponent,
            significand,
        }
        | Op::FloatToFloat {
            exponent,
            significand,
        } => return format!("((_ to_fp {exponent} {significand})"),
        Op::UnsignedToFloat {
            exponent,
            significand,
        } => return format!("((_ to_fp_unsigned {exponent} {significand})"),
        Op::FloatToSigned(bits) => return format!("((_ fp.to_sbv {bits})"),
        Op::FloatToUnsigned(bits) => return format!("((_ fp.to_ubv {bits})"),
        Op::PositiveZero {
            exponent,
            significand,
        } => return format!("(_ +zero {exponent} {significand}"),
    };
    format!("({name}")
}
