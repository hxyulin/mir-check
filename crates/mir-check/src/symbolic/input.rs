use super::{Sort, Term, Value};
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct InputShape {
    pub kind: InputKind,
    pub slots: u32,
    pub height: usize,
}

#[derive(Clone, Debug)]
pub enum InputKind {
    Bool,
    Integer {
        bits: u32,
        signed: bool,
    },
    Float {
        bits: u32,
    },
    Unit,
    Bytes {
        count: u64,
        pointer_bits: u32,
    },
    Tuple(Vec<Rc<InputShape>>),
    Struct {
        name: String,
        fields: Vec<(String, Rc<InputShape>)>,
    },
    Array {
        element: Rc<InputShape>,
        count: usize,
    },
}

#[derive(Clone, Debug)]
pub struct InputValue {
    pub shape: Rc<InputShape>,
    pub start: u32,
    pub seed: Term,
}

impl InputValue {
    fn child(&self, shape: &Rc<InputShape>, start: u32) -> Result<Value, String> {
        let child = Self {
            shape: shape.clone(),
            start,
            seed: self.seed.clone(),
        };
        match &shape.kind {
            InputKind::Bool
            | InputKind::Integer { .. }
            | InputKind::Float { .. }
            | InputKind::Unit
            | InputKind::Bytes { .. } => child.materialize(),
            InputKind::Tuple(_) | InputKind::Struct { .. } | InputKind::Array { .. } => {
                Ok(Value::Input(child))
            }
        }
    }

    pub fn materialize(&self) -> Result<Value, String> {
        let context = self.seed.context();
        let symbol = |sort| context.symbol(self.start, sort);
        let mut start = self
            .start
            .checked_add(1)
            .ok_or("lazy input symbol range overflow")?;
        match &self.shape.kind {
            InputKind::Bool => Ok(Value::Bool(symbol(Sort::Bool)?)),
            InputKind::Integer { bits, signed } => Ok(Value::Int {
                expression: symbol(Sort::BitVec(*bits))?,
                bits: *bits,
                signed: *signed,
            }),
            InputKind::Float { bits } => Ok(super::float_from_bits(
                context,
                symbol(Sort::BitVec(*bits))?,
                *bits,
            )),
            InputKind::Unit => Ok(Value::Unit),
            InputKind::Bytes {
                count,
                pointer_bits,
            } => Ok(Value::Bytes {
                length: Box::new(super::integer(
                    context,
                    u128::from(*count),
                    *pointer_bits,
                    false,
                )),
                data: symbol(Sort::Array(
                    Box::new(Sort::BitVec(*pointer_bits)),
                    Box::new(Sort::BitVec(8)),
                ))?,
            }),
            InputKind::Tuple(fields) => {
                let fields = fields
                    .iter()
                    .map(|shape| {
                        let value = self.child(shape, start)?;
                        start = start
                            .checked_add(shape.slots)
                            .ok_or("lazy input symbol range overflow")?;
                        Ok(value)
                    })
                    .collect::<Result<_, String>>()?;
                Ok(Value::Tuple(fields))
            }
            InputKind::Struct { name, fields } => {
                let fields = fields
                    .iter()
                    .map(|(name, shape)| {
                        let value = self.child(shape, start)?;
                        start = start
                            .checked_add(shape.slots)
                            .ok_or("lazy input symbol range overflow")?;
                        Ok((name.clone(), value))
                    })
                    .collect::<Result<_, String>>()?;
                Ok(Value::Adt {
                    name: name.clone(),
                    variant: 0,
                    is_option: false,
                    discriminant: 0,
                    fields,
                })
            }
            InputKind::Array { element, count } => {
                let elements = (0..*count)
                    .map(|_| {
                        let value = self.child(element, start)?;
                        start = start
                            .checked_add(element.slots)
                            .ok_or("lazy input symbol range overflow")?;
                        Ok(value)
                    })
                    .collect::<Result<_, String>>()?;
                Ok(Value::Elements(elements))
            }
        }
    }
}

impl Value {
    pub fn materialize(&self) -> Result<Value, String> {
        match self {
            Self::Uninitialized => Err("read of an uninitialized coroutine saved local".to_owned()),
            Self::Input(input) => input.materialize(),
            value => Ok(value.clone()),
        }
    }
}
