mod floating;
pub use floating::{float, float_cast, float_sort};

pub const MAX_REPEAT_VALUES: usize = 256;

#[derive(Clone, Debug)]
pub enum MemoryProjection {
    Field(usize),
    Variant(usize),
    Index(Box<Value>),
}

#[derive(Clone, Debug)]
pub enum Value {
    Bool(String),
    Int {
        expression: String,
        bits: u32,
        signed: bool,
    },
    Float {
        expression: String,
        bits: u32,
    },
    Bytes {
        length: Box<Value>,
        data: String,
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
    MutableBytes {
        owner: usize,
        length: Box<Value>,
    },
    Tuple(Vec<Value>),
    Elements(Vec<Value>),
    MetadataPointer(Box<Value>),
    StaticText,
    FormatArguments,
    Function,
    Unit,
}

impl Value {
    pub fn owned_repeat_size(&self) -> Option<usize> {
        match self {
            Self::Bool(_)
            | Self::Int { .. }
            | Self::Float { .. }
            | Self::Bytes { .. }
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
            | Self::MutableBytes { .. }
            | Self::MetadataPointer(_)
            | Self::StaticText
            | Self::FormatArguments
            | Self::Function => None,
        }
    }

    pub fn contains_mutable(&self) -> bool {
        match self {
            Self::MutableBytes { .. } | Self::Reference { mutable: true, .. } => true,
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
            | Self::Function
            | Self::Unit => false,
        }
    }

    pub fn field(&self, name: &str) -> Result<Value, String> {
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
    pub fn boolean(&self) -> Result<String, String> {
        match self {
            Self::Bool(expression) => Ok(expression.clone()),
            _ => Err("expected a boolean expression".to_owned()),
        }
    }

    pub fn integer(&self) -> Result<(String, u32, bool), String> {
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

pub fn integer(value: u128, bits: u32, signed: bool) -> Value {
    Value::Int {
        expression: format!("(_ bv{value} {bits})"),
        bits,
        signed,
    }
}

pub fn not(expression: &str) -> String {
    format!("(not {expression})")
}

pub fn binary(operation: &str, left: Value, right: Value) -> Result<Value, String> {
    if let (Value::Float { .. }, Value::Float { .. }) = (&left, &right) {
        return floating::binary(operation, left, right);
    }
    if let (Value::Bool(left), Value::Bool(right)) = (&left, &right) {
        let expression = match operation {
            "eq" => format!("(= {left} {right})"),
            "ne" => not(&format!("(= {left} {right})")),
            "and" => format!("(and {left} {right})"),
            "or" => format!("(or {left} {right})"),
            "xor" => format!("(xor {left} {right})"),
            _ => return Err(format!("unsupported boolean operation {operation}")),
        };
        return Ok(Value::Bool(expression));
    }
    let (left, bits, signed) = left.integer()?;
    let (right, right_bits, right_signed) = right.integer()?;
    if bits != right_bits || signed != right_signed {
        return Err("integer operand types do not match".to_owned());
    }
    let comparison = match operation {
        "eq" => Some(format!("(= {left} {right})")),
        "ne" => Some(not(&format!("(= {left} {right})"))),
        "lt" => Some(format!(
            "({} {left} {right})",
            if signed { "bvslt" } else { "bvult" }
        )),
        "le" => Some(format!(
            "({} {left} {right})",
            if signed { "bvsle" } else { "bvule" }
        )),
        "gt" => Some(format!(
            "({} {left} {right})",
            if signed { "bvsgt" } else { "bvugt" }
        )),
        "ge" => Some(format!(
            "({} {left} {right})",
            if signed { "bvsge" } else { "bvuge" }
        )),
        _ => None,
    };
    if let Some(expression) = comparison {
        return Ok(Value::Bool(expression));
    }
    let operator = match operation {
        "add" | "checked_add" => "bvadd",
        "sub" | "checked_sub" => "bvsub",
        "mul" | "checked_mul" => "bvmul",
        "div" => {
            if signed {
                "bvsdiv"
            } else {
                "bvudiv"
            }
        }
        "rem" => {
            if signed {
                "bvsrem"
            } else {
                "bvurem"
            }
        }
        "and" => "bvand",
        "or" => "bvor",
        "xor" => "bvxor",
        _ => return Err(format!("unsupported integer operation {operation}")),
    };
    let expression = format!("({operator} {left} {right})");
    let result = Value::Int {
        expression: expression.clone(),
        bits,
        signed,
    };
    if operation.starts_with("checked_") {
        let extra = if operation == "checked_mul" { bits } else { 1 };
        let extend = if signed { "sign_extend" } else { "zero_extend" };
        let wide =
            format!("({operator} ((_ {extend} {extra}) {left}) ((_ {extend} {extra}) {right}))");
        let overflow = not(&format!("(= {wide} ((_ {extend} {extra}) {expression}))"));
        Ok(Value::Tuple(vec![result, Value::Bool(overflow)]))
    } else {
        Ok(result)
    }
}

pub fn cast(value: Value, bits: u32, signed: bool) -> Result<Value, String> {
    if matches!(value, Value::Float { .. }) {
        return floating::integer_cast(value, bits, signed);
    }
    if let Value::Bool(expression) = value {
        return Ok(Value::Int {
            expression: format!("(ite {expression} (_ bv1 {bits}) (_ bv0 {bits}))"),
            bits,
            signed,
        });
    }
    let (expression, old_bits, old_signed) = value.integer()?;
    let expression = if bits < old_bits {
        format!("((_ extract {} 0) {expression})", bits - 1)
    } else if bits > old_bits {
        let extend = if old_signed {
            "sign_extend"
        } else {
            "zero_extend"
        };
        format!("((_ {extend} {}) {expression})", bits - old_bits)
    } else {
        expression
    };
    Ok(Value::Int {
        expression,
        bits,
        signed,
    })
}

pub fn shift(leftward: bool, left: Value, right: Value) -> Result<Value, String> {
    let (left, bits, signed) = left.integer()?;
    let (right, _, _) = cast(right, bits, false)?.integer()?;
    let amount = format!("(bvand {right} (_ bv{} {bits}))", bits - 1);
    let operation = if leftward {
        "bvshl"
    } else if signed {
        "bvashr"
    } else {
        "bvlshr"
    };
    Ok(Value::Int {
        expression: format!("({operation} {left} {amount})"),
        bits,
        signed,
    })
}

pub fn select_element(elements: &[Value], index: &Value) -> Result<Value, String> {
    let (index, index_bits, _) = index.integer()?;
    let mut result = elements.last().cloned().ok_or("empty array index")?;
    for (position, element) in elements.iter().enumerate().rev().skip(1) {
        let condition = format!("(= {index} (_ bv{position} {index_bits}))");
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
                expression: format!("(ite {condition} {expression} {otherwise})"),
                bits: *bits,
                signed: *signed,
            },
            (Value::Bool(expression), Value::Bool(otherwise)) => {
                Value::Bool(format!("(ite {condition} {expression} {otherwise})"))
            }
            (
                Value::Float { expression, bits },
                Value::Float {
                    expression: otherwise,
                    bits: other_bits,
                },
            ) if *bits == other_bits => Value::Float {
                expression: format!("(ite {condition} {expression} {otherwise})"),
                bits: *bits,
            },
            _ => return Err("array choice only models compatible scalar values".to_owned()),
        };
    }
    Ok(result)
}
