use super::*;
use symbolic::MemoryProjection;

impl<'tcx> Engine<'tcx> {
    pub(super) fn loop_write(
        &self,
        state: &mut State,
        place: Place<'tcx>,
        value: Value,
    ) -> Result<(), String> {
        if place.projection.is_empty() {
            return self.write(state, place, value);
        }
        let (allocation, path, writable) = self.memory_path(state, place)?;
        if !writable {
            return Err("inductive write through shared storage".into());
        }
        let local = place.local.as_usize();
        let mut storage = if let Some(allocation) = allocation {
            state.memory[allocation].clone()
        } else {
            state.locals[local].clone()
        }
        .ok_or("inductive write storage missing")?;
        self.loop_write_projection(&mut storage, &path, value, &state.conditions)?;
        if let Some(allocation) = allocation {
            state.memory[allocation] = Some(storage);
        } else {
            state.locals[local] = Some(storage);
        }
        Ok(())
    }

    fn loop_write_projection(
        &self,
        storage: &mut Value,
        path: &[MemoryProjection],
        value: Value,
        conditions: &[Term],
    ) -> Result<(), String> {
        match path.split_first() {
            Some((MemoryProjection::Field(index), rest)) => match storage {
                Value::Adt { fields, .. } => {
                    self.loop_write_projection(&mut fields[*index].1, rest, value, conditions)
                }
                Value::Tuple(fields) => {
                    self.loop_write_projection(&mut fields[*index], rest, value, conditions)
                }
                Value::Bool(_)
                | Value::Int { .. }
                | Value::Float { .. }
                | Value::Bytes { .. }
                | Value::Enum { .. }
                | Value::Cell { .. }
                | Value::Atomic { .. }
                | Value::Reference { .. }
                | Value::SliceIterator { .. }
                | Value::Elements(_)
                | Value::MetadataPointer(_)
                | Value::StaticText
                | Value::FormatArguments
                | Value::Input(_)
                | Value::Uninitialized
                | Value::Function
                | Value::Unit => Err("unsupported inductive write field".into()),
            },
            Some((MemoryProjection::Variant(index), rest)) => match storage {
                Value::Enum { variants, .. } => {
                    self.loop_write_projection(&mut variants[*index], rest, value, conditions)
                }
                Value::Adt { variant, .. } if *variant == *index => {
                    self.loop_write_projection(storage, rest, value, conditions)
                }
                Value::Adt { .. } => Err("inductive write has the wrong enum variant".into()),
                Value::Bool(_)
                | Value::Int { .. }
                | Value::Float { .. }
                | Value::Bytes { .. }
                | Value::Cell { .. }
                | Value::Atomic { .. }
                | Value::Reference { .. }
                | Value::SliceIterator { .. }
                | Value::Tuple(_)
                | Value::Elements(_)
                | Value::MetadataPointer(_)
                | Value::StaticText
                | Value::FormatArguments
                | Value::Input(_)
                | Value::Uninitialized
                | Value::Function
                | Value::Unit => Err("unsupported inductive write variant".into()),
            },
            Some((MemoryProjection::Index(index), [])) => {
                if let Value::Elements(fields) = storage {
                    let (index, bits, signed) = index.integer()?;
                    if signed {
                        return Err("inductive write index is signed".into());
                    }
                    for (position, field) in fields.iter_mut().enumerate() {
                        let equal = self.terms.apply(
                            Op::Equal,
                            &[
                                index.clone(),
                                self.terms.bit_vector(position as u128, bits)?,
                            ],
                        )?;
                        *field = scalar_update(&self.terms, equal, &value, field)?;
                    }
                    Ok(())
                } else {
                    self.write_projection(storage, path, value, conditions)
                }
            }
            Some((
                MemoryProjection::Index(_)
                | MemoryProjection::Slice { .. }
                | MemoryProjection::Chunks { .. },
                _,
            ))
            | None => self.write_projection(storage, path, value, conditions),
        }
    }
}

fn scalar_update(
    context: &Context,
    condition: Term,
    new: &Value,
    old: &Value,
) -> Result<Value, String> {
    match (new, old) {
        (
            Value::Int {
                expression: a,
                bits,
                signed,
            },
            Value::Int {
                expression: b,
                bits: c,
                signed: d,
            },
        ) if bits == c && signed == d => Ok(Value::Int {
            expression: context.apply(Op::Ite, &[condition, a.clone(), b.clone()])?,
            bits: *bits,
            signed: *signed,
        }),
        (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(
            context.apply(Op::Ite, &[condition, a.clone(), b.clone()])?,
        )),
        (
            Value::Bool(_)
            | Value::Int { .. }
            | Value::Float { .. }
            | Value::Bytes { .. }
            | Value::Adt { .. }
            | Value::Enum { .. }
            | Value::Cell { .. }
            | Value::Atomic { .. }
            | Value::Reference { .. }
            | Value::SliceIterator { .. }
            | Value::Tuple(_)
            | Value::Elements(_)
            | Value::MetadataPointer(_)
            | Value::StaticText
            | Value::FormatArguments
            | Value::Input(_)
            | Value::Uninitialized
            | Value::Function
            | Value::Unit,
            _,
        ) => Err("inductive indexed writes require compatible integer or Boolean elements".into()),
    }
}
