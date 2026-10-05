#[derive(Clone, Debug)]
pub enum Value {
    Bool(String),
    Int {
        expression: String,
        bits: u32,
        signed: bool,
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
    MutableBytes {
        owner: usize,
        length: Box<Value>,
    },
    Tuple(Vec<Value>),
    Unit,
}

impl Value {
    pub fn contains_mutable(&self) -> bool {
        match self {
            Self::MutableBytes { .. } => true,
            Self::Adt { fields, .. } => fields.iter().any(|(_, value)| value.contains_mutable()),
            Self::Tuple(fields) => fields.iter().any(Self::contains_mutable),
            Self::Bool(_) | Self::Int { .. } | Self::Bytes { .. } | Self::Unit => false,
        }
    }

    pub fn field(&self, name: &str) -> Result<Value, String> {
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
