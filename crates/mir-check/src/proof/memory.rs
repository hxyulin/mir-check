use super::*;
use symbolic::MemoryProjection;

const MAX_ALLOCATIONS: usize = 512;

impl<'tcx> Engine<'tcx> {
    pub(super) fn local(&self, state: &State, local: usize) -> Result<Value, String> {
        let value = if let Some(allocation) = state.addresses[local] {
            state.memory.get(allocation).and_then(Option::as_ref)
        } else {
            state.locals[local].as_ref()
        };
        value
            .cloned()
            .ok_or_else(|| format!("uninitialized or unavailable local _{local}"))
    }

    pub(super) fn snapshots(
        &self,
        values: &[Value],
        memory: &[Option<Value>],
        conditions: &[String],
    ) -> Result<Vec<Value>, String> {
        values
            .iter()
            .map(|value| self.snapshot(value, memory, conditions, 0))
            .collect()
    }

    pub(super) fn snapshot(
        &self,
        value: &Value,
        memory: &[Option<Value>],
        conditions: &[String],
        depth: usize,
    ) -> Result<Value, String> {
        if depth >= 16 {
            return Err("memory snapshot nesting limit reached".to_owned());
        }
        match value {
            Value::Reference { .. } => {
                let value = self.reference_value(value, memory, conditions)?;
                self.snapshot(&value, memory, conditions, depth + 1)
            }
            Value::Adt {
                name,
                variant,
                is_option,
                discriminant,
                fields,
            } => Ok(Value::Adt {
                name: name.clone(),
                variant: *variant,
                is_option: *is_option,
                discriminant: *discriminant,
                fields: fields
                    .iter()
                    .map(|(name, value)| {
                        Ok((
                            name.clone(),
                            self.snapshot(value, memory, conditions, depth + 1)?,
                        ))
                    })
                    .collect::<Result<_, String>>()?,
            }),
            Value::Enum {
                discriminant,
                variants,
                is_option,
            } => Ok(Value::Enum {
                discriminant: discriminant.clone(),
                is_option: *is_option,
                variants: variants
                    .iter()
                    .map(|value| self.snapshot(value, memory, conditions, depth + 1))
                    .collect::<Result<_, _>>()?,
            }),
            Value::Tuple(fields) => Ok(Value::Tuple(
                fields
                    .iter()
                    .map(|value| self.snapshot(value, memory, conditions, depth + 1))
                    .collect::<Result<_, _>>()?,
            )),
            Value::Elements(fields) => Ok(Value::Elements(
                fields
                    .iter()
                    .map(|value| self.snapshot(value, memory, conditions, depth + 1))
                    .collect::<Result<_, _>>()?,
            )),
            other => Ok(other.clone()),
        }
    }

    pub(super) fn reference_value(
        &self,
        reference: &Value,
        memory: &[Option<Value>],
        conditions: &[String],
    ) -> Result<Value, String> {
        let Value::Reference {
            allocation,
            projection,
            ..
        } = reference
        else {
            return Err("expected an allocation reference".to_owned());
        };
        let mut value = memory
            .get(*allocation)
            .and_then(Option::as_ref)
            .cloned()
            .ok_or("reference points to dead or uninitialized storage")?;
        for element in projection {
            value = self.memory_projection(value, element, conditions)?;
        }
        Ok(value)
    }

    fn memory_projection(
        &self,
        value: Value,
        projection: &MemoryProjection,
        conditions: &[String],
    ) -> Result<Value, String> {
        match (projection, value) {
            (MemoryProjection::Field(index), Value::Adt { fields, .. }) => fields
                .get(*index)
                .map(|(_, value)| value.clone())
                .ok_or_else(|| "memory field missing".to_owned()),
            (MemoryProjection::Field(index), Value::Tuple(fields)) => fields
                .get(*index)
                .cloned()
                .ok_or_else(|| "memory tuple field missing".to_owned()),
            (MemoryProjection::Variant(expected), value @ Value::Adt { .. }) => {
                let Value::Adt { variant, .. } = value else {
                    unreachable!()
                };
                if variant != *expected {
                    return Err("memory downcast has wrong variant".to_owned());
                }
                Ok(value)
            }
            (
                MemoryProjection::Variant(expected),
                Value::Enum {
                    discriminant,
                    variants,
                    ..
                },
            ) => {
                let value = variants
                    .get(*expected)
                    .ok_or("memory enum variant missing")?;
                let Value::Adt {
                    discriminant: tag, ..
                } = value
                else {
                    return Err("memory enum payload missing".to_owned());
                };
                let (_, bits, signed) = discriminant.integer()?;
                let equal =
                    symbolic::binary("eq", *discriminant, symbolic::integer(*tag, bits, signed))?
                        .boolean()?;
                if self.feasible(&[conditions.to_vec(), vec![symbolic::not(&equal)]].concat())? {
                    return Err("memory downcast lacks a proven variant check".to_owned());
                }
                Ok(value.clone())
            }
            (MemoryProjection::Index(index), Value::Elements(elements)) => {
                self.fixed_element(&elements, index, conditions)
            }
            (MemoryProjection::Index(index), Value::Bytes { data, length }) => {
                self.memory_bounds(index, &length, conditions)?;
                Ok(Value::Int {
                    expression: format!("(select {data} {})", index.integer()?.0),
                    bits: 8,
                    signed: false,
                })
            }
            _ => Err("unsupported memory projection".to_owned()),
        }
    }

    fn memory_bounds(
        &self,
        index: &Value,
        length: &Value,
        conditions: &[String],
    ) -> Result<(), String> {
        let (_, width, signed) = index.integer()?;
        if signed || width != u32::from(self.tcx.sess.target.pointer_width) {
            return Err("memory index type mismatch".to_owned());
        }
        let inside = symbolic::binary("lt", index.clone(), length.clone())?.boolean()?;
        if self.feasible(&[conditions.to_vec(), vec![symbolic::not(&inside)]].concat())? {
            return Err("memory index lacks a proven bounds check".to_owned());
        }
        Ok(())
    }

    fn memory_path(
        &self,
        state: &State,
        place: Place<'tcx>,
    ) -> Result<(Option<usize>, Vec<MemoryProjection>, bool), String> {
        let local = place.local.as_usize();
        let mut allocation = state.addresses[local];
        let mut path = Vec::new();
        let mut value = self.local(state, local)?;
        let mut writable = true;
        for projection in place.projection {
            match projection {
                ProjectionElem::Deref => {
                    if let Value::Reference {
                        allocation: target,
                        projection,
                        mutable,
                    } = value
                    {
                        allocation = Some(target);
                        path = projection;
                        writable = mutable;
                        value = self.reference_value(
                            &Value::Reference {
                                allocation: target,
                                projection: path.clone(),
                                mutable,
                            },
                            &state.memory,
                            &state.conditions,
                        )?;
                    } else {
                        writable = false;
                    }
                }
                ProjectionElem::Field(field, _) => {
                    let element = MemoryProjection::Field(field.as_usize());
                    value = self.memory_projection(value, &element, &state.conditions)?;
                    path.push(element);
                }
                ProjectionElem::Downcast(_, variant) => {
                    let element = MemoryProjection::Variant(variant.as_usize());
                    value = self.memory_projection(value, &element, &state.conditions)?;
                    path.push(element);
                }
                ProjectionElem::Index(index) => {
                    let element =
                        MemoryProjection::Index(Box::new(self.local(state, index.as_usize())?));
                    value = self.memory_projection(value, &element, &state.conditions)?;
                    path.push(element);
                }
                ProjectionElem::ConstantIndex {
                    offset, from_end, ..
                } => {
                    let width = u32::from(self.tcx.sess.target.pointer_width);
                    let offset = symbolic::integer(u128::from(offset), width, false);
                    let index = if from_end {
                        let length = match &value {
                            Value::Bytes { length, .. } => (**length).clone(),
                            Value::Elements(elements) => {
                                symbolic::integer(elements.len() as u128, width, false)
                            }
                            _ => return Err("constant memory index needs array storage".to_owned()),
                        };
                        symbolic::binary("sub", length, offset)?
                    } else {
                        offset
                    };
                    let element = MemoryProjection::Index(Box::new(index));
                    value = self.memory_projection(value, &element, &state.conditions)?;
                    path.push(element);
                }
                _ => return Err("unsupported memory address projection".to_owned()),
            }
        }
        Ok((allocation, path, writable))
    }

    pub(super) fn borrow(
        &self,
        state: &mut State,
        place: Place<'tcx>,
        mutable: bool,
    ) -> Result<Value, String> {
        let value = self.place(state, place)?;
        if !mutable && matches!(value, Value::Bytes { .. }) {
            return Ok(value);
        }
        let (allocation, projection, writable) = self.memory_path(state, place)?;
        if mutable && !writable {
            return Err("mutable borrow needs writable allocation storage".to_owned());
        }
        let allocation = if let Some(allocation) = allocation {
            allocation
        } else {
            if state.memory.len() >= MAX_ALLOCATIONS {
                return Err("memory allocation budget reached".to_owned());
            }
            let allocation = state.memory.len();
            state
                .memory
                .push(Some(self.local(state, place.local.as_usize())?));
            state.addresses[place.local.as_usize()] = Some(allocation);
            allocation
        };
        Ok(Value::Reference {
            allocation,
            projection,
            mutable,
        })
    }

    pub(super) fn write(
        &self,
        state: &mut State,
        place: Place<'tcx>,
        value: Value,
    ) -> Result<(), String> {
        let local = place.local.as_usize();
        if place.projection.is_empty() {
            if let Some(allocation) = state.addresses[local] {
                state.memory[allocation] = Some(value);
            } else {
                state.locals[local] = Some(value);
            }
            return Ok(());
        }
        let (allocation, path, writable) = self.memory_path(state, place)?;
        if !writable {
            return Err("write through a shared snapshot is unsupported".to_owned());
        }
        let mut storage = if let Some(allocation) = allocation {
            state.memory[allocation].clone()
        } else {
            state.locals[local].clone()
        }
        .ok_or("write into uninitialized aggregate storage")?;
        self.write_projection(&mut storage, &path, value, &state.conditions)?;
        if let Some(allocation) = allocation {
            state.memory[allocation] = Some(storage);
        } else {
            state.locals[local] = Some(storage);
        }
        Ok(())
    }

    pub(super) fn write_projection(
        &self,
        storage: &mut Value,
        path: &[MemoryProjection],
        value: Value,
        conditions: &[String],
    ) -> Result<(), String> {
        let Some((projection, rest)) = path.split_first() else {
            *storage = value;
            return Ok(());
        };
        // Establish downcast/bounds conditions before mutating the selected field.
        self.memory_projection(storage.clone(), projection, conditions)?;
        match (projection, storage) {
            (MemoryProjection::Field(index), Value::Adt { fields, .. }) => {
                self.write_projection(&mut fields[*index].1, rest, value, conditions)
            }
            (MemoryProjection::Field(index), Value::Tuple(fields)) => {
                self.write_projection(&mut fields[*index], rest, value, conditions)
            }
            (MemoryProjection::Variant(_), storage @ Value::Adt { .. }) => {
                self.write_projection(storage, rest, value, conditions)
            }
            (MemoryProjection::Variant(index), Value::Enum { variants, .. }) => {
                self.write_projection(&mut variants[*index], rest, value, conditions)
            }
            (MemoryProjection::Index(index), Value::Elements(elements)) => {
                for (position, element) in elements.iter_mut().enumerate() {
                    let equal = symbolic::binary(
                        "eq",
                        (**index).clone(),
                        symbolic::integer(
                            position as u128,
                            u32::from(self.tcx.sess.target.pointer_width),
                            false,
                        ),
                    )?
                    .boolean()?;
                    if !self
                        .feasible(&[conditions.to_vec(), vec![symbolic::not(&equal)]].concat())?
                    {
                        return self.write_projection(element, rest, value, conditions);
                    }
                }
                Err("array write index is not uniquely determined".to_owned())
            }
            (MemoryProjection::Index(index), Value::Bytes { data, .. }) if rest.is_empty() => {
                let (expression, width, signed) = value.integer()?;
                if width != 8 || signed {
                    return Err("byte write has non-byte value".to_owned());
                }
                *data = format!("(store {data} {} {expression})", index.integer()?.0);
                Ok(())
            }
            _ => Err("unsupported projected memory write".to_owned()),
        }
    }

    pub(super) fn return_value(
        &self,
        value: Value,
        state: &State,
        incoming: usize,
    ) -> Result<Value, String> {
        match value {
            Value::Reference { allocation, .. } if allocation >= incoming => {
                self.snapshot(&value, &state.memory, &state.conditions, 0)
            }
            Value::Adt {
                name,
                variant,
                is_option,
                discriminant,
                fields,
            } => Ok(Value::Adt {
                name,
                variant,
                is_option,
                discriminant,
                fields: fields
                    .into_iter()
                    .map(|(name, value)| Ok((name, self.return_value(value, state, incoming)?)))
                    .collect::<Result<_, String>>()?,
            }),
            Value::Tuple(fields) => Ok(Value::Tuple(
                fields
                    .into_iter()
                    .map(|value| self.return_value(value, state, incoming))
                    .collect::<Result<_, _>>()?,
            )),
            Value::Elements(fields) => Ok(Value::Elements(
                fields
                    .into_iter()
                    .map(|value| self.return_value(value, state, incoming))
                    .collect::<Result<_, _>>()?,
            )),
            other => Ok(other),
        }
    }
}
