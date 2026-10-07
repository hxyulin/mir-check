mod floating;
pub mod input;
#[cfg(test)]
mod tests;
pub use floating::{float, float_cast, float_from_bits, float_negate};
pub use mir_check::smt::{Context, Op, Sort, Term};

pub const MAX_REPEAT_VALUES: usize = 256;

#[derive(Clone, Debug)]
pub enum MemoryProjection {
    Field(usize),
    Variant(usize),
    Index(Box<Value>),
    Slice {
        offset: Box<Value>,
        length: Box<Value>,
    },
    Chunks {
        width: usize,
        count: usize,
    },
}

#[derive(Clone, Debug)]
pub enum Value {
    Uninitialized,
    StaticSlice {
        elements: Vec<Value>,
        epoch: usize,
    },
    StaticView {
        id: usize,
        epoch: usize,
    },
    RawPointer {
        address: Term,
        bits: u32,
    },
    Input(input::InputValue),
    Bool(Term),
    Int {
        expression: Term,
        bits: u32,
        signed: bool,
    },
    Float {
        expression: Term,
        bits: u32,
        raw_bits: Option<Term>,
    },
    Bytes {
        length: Box<Value>,
        data: Term,
    },
    Adt {
        name: String,
        variant: usize,
        is_option: bool,
        discriminant: u128,
        fields: Vec<(String, Value)>,
    },
    Enum {
        discriminant: Box<Value>,
        variants: Vec<Value>,
        is_option: bool,
    },
    Cell {
        allocation: usize,
    },
    Atomic {
        bits: u32,
        signed: bool,
    },
    Reference {
        allocation: usize,
        projection: Vec<MemoryProjection>,
        mutable: bool,
    },
    SliceIterator {
        source: Box<Value>,
        front: Box<Value>,
        back: Box<Value>,
        mutable: bool,
    },
    Tuple(Vec<Value>),
    Elements(Vec<Value>),
    MetadataPointer(Box<Value>),
    StaticText,
    FormatArguments,
    FunctionPointer {
        id: usize,
    },
    Function,
    Unit,
}

impl Value {
    pub fn owned_repeat_size(&self) -> Option<usize> {
        match self {
            Self::Input(input) => {
                let size = input.shape.slots as usize;
                (size <= MAX_REPEAT_VALUES).then_some(size)
            }
            Self::Bool(_)
            | Self::Int { .. }
            | Self::Float { .. }
            | Self::Bytes { .. }
            | Self::RawPointer { .. }
            | Self::Unit => Some(1),
            Self::Tuple(fields) | Self::Elements(fields) => {
                fields.iter().try_fold(1_usize, |size, value| {
                    size.checked_add(value.owned_repeat_size()?)
                        .filter(|size| *size <= MAX_REPEAT_VALUES)
                })
            }
            Self::Adt { fields, .. } => fields.iter().try_fold(1_usize, |size, (_, value)| {
                size.checked_add(value.owned_repeat_size()?)
                    .filter(|size| *size <= MAX_REPEAT_VALUES)
            }),
            Self::Enum {
                discriminant,
                variants,
                ..
            } => variants
                .iter()
                .try_fold(1 + discriminant.owned_repeat_size()?, |size, value| {
                    size.checked_add(value.owned_repeat_size()?)
                        .filter(|size| *size <= MAX_REPEAT_VALUES)
                }),
            Self::Cell { .. }
            | Self::Atomic { .. }
            | Self::Reference { .. }
            | Self::SliceIterator { .. }
            | Self::MetadataPointer(_)
            | Self::StaticText
            | Self::FormatArguments
            | Self::StaticSlice { .. }
            | Self::StaticView { .. }
            | Self::Uninitialized
            | Self::FunctionPointer { .. }
            | Self::Function => None,
        }
    }

    pub fn owned_iterator_size(&self) -> Option<usize> {
        match self {
            Self::Reference { .. } => Some(1),
            Self::Tuple(fields) | Self::Elements(fields) => {
                fields.iter().try_fold(1_usize, |size, value| {
                    size.checked_add(value.owned_iterator_size()?)
                        .filter(|size| *size <= MAX_REPEAT_VALUES)
                })
            }
            Self::Adt { fields, .. } => fields.iter().try_fold(1_usize, |size, (_, value)| {
                size.checked_add(value.owned_iterator_size()?)
                    .filter(|size| *size <= MAX_REPEAT_VALUES)
            }),
            Self::Enum {
                discriminant,
                variants,
                ..
            } => {
                variants
                    .iter()
                    .try_fold(1 + discriminant.owned_iterator_size()?, |size, value| {
                        size.checked_add(value.owned_iterator_size()?)
                            .filter(|size| *size <= MAX_REPEAT_VALUES)
                    })
            }
            Self::Input(_)
            | Self::Bool(_)
            | Self::Int { .. }
            | Self::Float { .. }
            | Self::Bytes { .. }
            | Self::RawPointer { .. }
            | Self::Unit => self.owned_repeat_size(),
            Self::Cell { .. }
            | Self::Atomic { .. }
            | Self::SliceIterator { .. }
            | Self::MetadataPointer(_)
            | Self::StaticText
            | Self::FormatArguments
            | Self::StaticSlice { .. }
            | Self::StaticView { .. }
            | Self::Uninitialized
            | Self::FunctionPointer { .. }
            | Self::Function => None,
        }
    }

    pub fn contains_mutable(&self) -> bool {
        match self {
            Self::Input(_) => false,
            Self::Reference { mutable: true, .. } => true,
            Self::SliceIterator {
                mutable, source, ..
            } => *mutable || source.contains_mutable(),
            Self::Reference { mutable: false, .. } | Self::Cell { .. } | Self::Atomic { .. } => {
                false
            }
            Self::Adt { fields, .. } => fields.iter().any(|(_, value)| value.contains_mutable()),
            Self::Enum { variants, .. } => variants.iter().any(Self::contains_mutable),
            Self::Tuple(fields) | Self::Elements(fields) => {
                fields.iter().any(Self::contains_mutable)
            }
            Self::Bool(_)
            | Self::Int { .. }
            | Self::Float { .. }
            | Self::Bytes { .. }
            | Self::MetadataPointer(_)
            | Self::StaticText
            | Self::FormatArguments
            | Self::RawPointer { .. }
            | Self::StaticSlice { .. }
            | Self::StaticView { .. }
            | Self::Uninitialized
            | Self::FunctionPointer { .. }
            | Self::Function
            | Self::Unit => false,
        }
    }

    pub fn field(&self, name: &str) -> Result<Value, String> {
        if let Self::Input(input) = self {
            return input.materialize()?.field(name);
        }
        if let Self::Tuple(fields) = self {
            let index = name.parse::<usize>().map_err(|error| error.to_string())?;
            return fields
                .get(index)
                .cloned()
                .ok_or_else(|| format!("unknown tuple field {index}"));
        }
        let Self::Adt {
            name: ty_name,
            fields,
            ..
        } = self
        else {
            return Err("field access requires a modeled struct".to_owned());
        };
        fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.clone())
            .ok_or_else(|| format!("unknown contract field {name} in {ty_name}"))
    }
    pub fn boolean(&self) -> Result<Term, String> {
        match self {
            Self::Bool(expression) => Ok(expression.clone()),
            _ => Err("expected a boolean expression".to_owned()),
        }
    }

    pub fn integer(&self) -> Result<(Term, u32, bool), String> {
        match self {
            Self::Int {
                expression,
                bits,
                signed,
            } => Ok((expression.clone(), *bits, *signed)),
            _ => Err("expected an integer expression".to_owned()),
        }
    }
}

pub fn integer(context: &Context, value: u128, bits: u32, signed: bool) -> Value {
    Value::Int {
        expression: context
            .bit_vector(value, bits)
            .expect("modeled integer width"),
        bits,
        signed,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constant {
    Bool(bool),
    BitVec { value: u128, bits: u32 },
}

pub fn constant(expression: &Term) -> Option<Constant> {
    use mir_check::smt::Constant as Scalar;
    match expression.constant()? {
        Scalar::Bool(value) => Some(Constant::Bool(value)),
        Scalar::BitVec(value) => {
            let Sort::BitVec(bits) = expression.sort() else {
                return None;
            };
            Some(Constant::BitVec { value, bits: *bits })
        }
        Scalar::Rounding(_) => None,
    }
}

pub fn not(expression: &Term) -> Term {
    expression
        .context()
        .apply(Op::Not, std::slice::from_ref(expression))
        .expect("boolean safety condition")
}

pub fn binary(
    context: &Context,
    operation: &str,
    left: Value,
    right: Value,
) -> Result<Value, String> {
    if let (
        Value::RawPointer {
            address: left,
            bits,
        },
        Value::RawPointer {
            address: right,
            bits: right_bits,
        },
    ) = (&left, &right)
    {
        if bits != right_bits || !matches!(operation, "eq" | "ne") {
            return Err("unsupported raw pointer operation".into());
        }
        let equal = context.apply(Op::Equal, &[left.clone(), right.clone()])?;
        return Ok(Value::Bool(if operation == "ne" {
            not(&equal)
        } else {
            equal
        }));
    }
    if let (Value::Float { .. }, Value::Float { .. }) = (&left, &right) {
        return floating::binary(context, operation, left, right);
    }
    if let (Value::Bool(a), Value::Bool(b)) = (&left, &right) {
        let op = match operation {
            "eq" | "ne" => Op::Equal,
            "and" => Op::And,
            "or" => Op::Or,
            "xor" => Op::Xor,
            _ => return Err(format!("unsupported boolean operation {operation}")),
        };
        let term = context.apply(op, &[a.clone(), b.clone()])?;
        return Ok(Value::Bool(if operation == "ne" {
            not(&term)
        } else {
            term
        }));
    }
    let (a, bits, signed) = left.integer()?;
    let (b, other_bits, other_signed) = right.integer()?;
    if bits != other_bits || signed != other_signed {
        return Err("integer operand types do not match".to_owned());
    }
    let op = match operation {
        "eq" | "ne" => Op::Equal,
        "lt" => {
            if signed {
                Op::BvSignedLt
            } else {
                Op::BvUnsignedLt
            }
        }
        "le" => {
            if signed {
                Op::BvSignedLe
            } else {
                Op::BvUnsignedLe
            }
        }
        "gt" => {
            if signed {
                Op::BvSignedGt
            } else {
                Op::BvUnsignedGt
            }
        }
        "ge" => {
            if signed {
                Op::BvSignedGe
            } else {
                Op::BvUnsignedGe
            }
        }
        "add" | "checked_add" => Op::BvAdd,
        "sub" | "checked_sub" => Op::BvSub,
        "mul" | "checked_mul" => Op::BvMul,
        "div" => {
            if signed {
                Op::BvSignedDiv
            } else {
                Op::BvUnsignedDiv
            }
        }
        "rem" => {
            if signed {
                Op::BvSignedRem
            } else {
                Op::BvUnsignedRem
            }
        }
        "and" => Op::BvAnd,
        "or" => Op::BvOr,
        "xor" => Op::BvXor,
        _ => return Err(format!("unsupported integer operation {operation}")),
    };
    let expression = context.apply(op, &[a.clone(), b.clone()])?;
    if *expression.sort() == Sort::Bool {
        return Ok(Value::Bool(if operation == "ne" {
            not(&expression)
        } else {
            expression
        }));
    }
    let result = Value::Int {
        expression: expression.clone(),
        bits,
        signed,
    };
    if matches!(operation, "checked_add" | "checked_sub" | "checked_mul") {
        let extra = if operation == "checked_mul" { bits } else { 1 };
        let extend = if signed {
            Op::SignExtend(extra)
        } else {
            Op::ZeroExtend(extra)
        };
        let wide = context.apply(
            op,
            &[context.apply(extend, &[a])?, context.apply(extend, &[b])?],
        )?;
        let extended = context.apply(extend, &[expression])?;
        let overflow = not(&context.apply(Op::Equal, &[wide, extended])?);
        Ok(Value::Tuple(vec![result, Value::Bool(overflow)]))
    } else {
        Ok(result)
    }
}

pub fn cast(context: &Context, value: Value, bits: u32, signed: bool) -> Result<Value, String> {
    if matches!(value, Value::Float { .. }) {
        return floating::integer_cast(context, value, bits, signed);
    }
    if let Value::Bool(expression) = value {
        return Ok(Value::Int {
            expression: context.apply(
                Op::Ite,
                &[
                    expression,
                    context.bit_vector(1, bits)?,
                    context.bit_vector(0, bits)?,
                ],
            )?,
            bits,
            signed,
        });
    }
    let (expression, old_bits, old_signed) = value.integer()?;
    let expression = if bits < old_bits {
        context.apply(
            Op::Extract {
                high: bits - 1,
                low: 0,
            },
            &[expression],
        )?
    } else if bits > old_bits {
        context.apply(
            if old_signed {
                Op::SignExtend(bits - old_bits)
            } else {
                Op::ZeroExtend(bits - old_bits)
            },
            &[expression],
        )?
    } else {
        expression
    };
    Ok(Value::Int {
        expression,
        bits,
        signed,
    })
}

pub fn shift(
    context: &Context,
    leftward: bool,
    left: Value,
    right: Value,
) -> Result<Value, String> {
    let (left, bits, signed) = left.integer()?;
    let (right, _, _) = cast(context, right, bits, false)?.integer()?;
    let amount = context.apply(
        Op::BvAnd,
        &[right, context.bit_vector(u128::from(bits - 1), bits)?],
    )?;
    let op = if leftward {
        Op::BvShiftLeft
    } else if signed {
        Op::BvArithmeticShiftRight
    } else {
        Op::BvLogicalShiftRight
    };
    Ok(Value::Int {
        expression: context.apply(op, &[left, amount])?,
        bits,
        signed,
    })
}

pub fn select_element(
    context: &Context,
    elements: &[Value],
    index: &Value,
) -> Result<Value, String> {
    let (index, index_bits, _) = index.integer()?;
    let mut result = elements.last().cloned().ok_or("empty array index")?;
    for (position, element) in elements.iter().enumerate().rev().skip(1) {
        let condition = context.apply(
            Op::Equal,
            &[
                index.clone(),
                context.bit_vector(position as u128, index_bits)?,
            ],
        )?;
        result = match (element, result) {
            (
                Value::Int {
                    expression,
                    bits,
                    signed,
                },
                Value::Int {
                    expression: otherwise,
                    bits: other_bits,
                    signed: other_signed,
                },
            ) if *bits == other_bits && *signed == other_signed => Value::Int {
                expression: context.apply(Op::Ite, &[condition, expression.clone(), otherwise])?,
                bits: *bits,
                signed: *signed,
            },
            (Value::Bool(expression), Value::Bool(otherwise)) => {
                Value::Bool(context.apply(Op::Ite, &[condition, expression.clone(), otherwise])?)
            }
            (
                Value::Float {
                    expression,
                    bits,
                    raw_bits,
                },
                Value::Float {
                    expression: otherwise,
                    bits: other_bits,
                    raw_bits: other_raw_bits,
                },
            ) if *bits == other_bits => {
                let raw_bits = raw_bits
                    .as_ref()
                    .zip(other_raw_bits)
                    .map(|(a, b)| context.apply(Op::Ite, &[condition.clone(), a.clone(), b]))
                    .transpose()?;
                Value::Float {
                    expression: context
                        .apply(Op::Ite, &[condition, expression.clone(), otherwise])?,
                    bits: *bits,
                    raw_bits,
                }
            }
            _ => return Err("array choice only models compatible scalar values".to_owned()),
        };
    }
    Ok(result)
}
