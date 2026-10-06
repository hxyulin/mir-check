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
        let mut projections = projection.iter().peekable();
        while let Some(element) = projections.next() {
            if let MemoryProjection::Chunks { width, count } = element
                && let Some(MemoryProjection::Index(index)) = projections.peek()
            {
                let bits = u32::from(self.tcx.sess.target.pointer_width);
                self.memory_bounds(
                    index,
                    &symbolic::integer(*count as u128, bits, false),
                    conditions,
                )?;
                let offset = symbolic::binary(
                    "mul",
                    (**index).clone(),
                    symbolic::integer(*width as u128, bits, false),
                )?;
                value = self.memory_projection(
                    value,
                    &MemoryProjection::Slice {
                        offset: Box::new(offset),
                        length: Box::new(symbolic::integer(*width as u128, bits, false)),
                    },
                    conditions,
                )?;
                projections.next();
            } else {
                value = self.memory_projection(value, element, conditions)?;
            }
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
            (
                MemoryProjection::Slice { offset, length },
                Value::Bytes {
                    data,
                    length: capacity,
                },
            ) => {
                let valid_offset =
                    symbolic::binary("le", (**offset).clone(), (*capacity).clone())?.boolean()?;
                let available = symbolic::binary("sub", (*capacity).clone(), (**offset).clone())?;
                let valid_length =
                    symbolic::binary("le", (**length).clone(), available)?.boolean()?;
                let safe = format!("(and {valid_offset} {valid_length})");
                if self.feasible(&[conditions.to_vec(), vec![symbolic::not(&safe)]].concat())? {
                    return Err("byte view lacks proven region bounds".to_owned());
                }
                let (offset, bits, signed) = offset.integer()?;
                if signed {
                    return Err("signed byte view offset".to_owned());
                }
                Ok(Value::Bytes {
                    length: length.clone(),
                    data: format!(
                        "(lambda ((view_index (_ BitVec {bits}))) (select {data} (bvadd {offset} view_index)))"
                    ),
                })
            }
            (MemoryProjection::Chunks { width, count }, bytes @ Value::Bytes { .. }) => {
                let bits = u32::from(self.tcx.sess.target.pointer_width);
                let mut chunks = Vec::with_capacity(*count);
                for index in 0..*count {
                    chunks.push(self.memory_projection(
                        bytes.clone(),
                        &MemoryProjection::Slice {
                            offset: Box::new(symbolic::integer(
                                (index * width) as u128,
                                bits,
                                false,
                            )),
                            length: Box::new(symbolic::integer(*width as u128, bits, false)),
                        },
                        conditions,
                    )?);
                }
                Ok(Value::Elements(chunks))
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
        if !mutable && matches!(value, Value::Bytes { .. } | Value::StaticText) {
            return Ok(value);
        }
        if mutable && matches!(value, Value::StaticText) {
            return Err("mutable static string reference storage is not modeled".to_owned());
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
        if let MemoryProjection::Slice { offset, length } = projection {
            if let Some((
                MemoryProjection::Slice {
                    offset: inner_offset,
                    length: inner_length,
                },
                tail,
            )) = rest.split_first()
            {
                self.memory_projection(storage.clone(), projection, conditions)?;
                let valid_offset =
                    symbolic::binary("le", (**inner_offset).clone(), (**length).clone())?
                        .boolean()?;
                let available =
                    symbolic::binary("sub", (**length).clone(), (**inner_offset).clone())?;
                let valid_length =
                    symbolic::binary("le", (**inner_length).clone(), available)?.boolean()?;
                let safe = format!("(and {valid_offset} {valid_length})");
                if self.feasible(&[conditions.to_vec(), vec![symbolic::not(&safe)]].concat())? {
                    return Err("nested byte view lacks proven region bounds".to_owned());
                }
                let absolute =
                    symbolic::binary("add", (**offset).clone(), (**inner_offset).clone())?;
                let mut translated = vec![MemoryProjection::Slice {
                    offset: Box::new(absolute),
                    length: inner_length.clone(),
                }];
                translated.extend_from_slice(tail);
                return self.write_projection(storage, &translated, value, conditions);
            }
            if let Some((MemoryProjection::Index(index), tail)) = rest.split_first() {
                // Prove the parent region before translating its element address.
                self.memory_projection(storage.clone(), projection, conditions)?;
                self.memory_bounds(index, length, conditions)?;
                let absolute = symbolic::binary("add", (**offset).clone(), (**index).clone())?;
                let mut translated = vec![MemoryProjection::Index(Box::new(absolute))];
                translated.extend_from_slice(tail);
                return self.write_projection(storage, &translated, value, conditions);
            }
            let mut view = self.memory_projection(storage.clone(), projection, conditions)?;
            self.write_projection(&mut view, rest, value, conditions)?;
            return self.merge_byte_view(storage, offset, length, view, conditions);
        }
        if let MemoryProjection::Chunks { width, count } = projection {
            let Some((MemoryProjection::Index(index), rest)) = rest.split_first() else {
                return Err("whole chunk view assignment is not modeled".to_owned());
            };
            let bits = u32::from(self.tcx.sess.target.pointer_width);
            self.memory_bounds(
                index,
                &symbolic::integer(*count as u128, bits, false),
                conditions,
            )?;
            let offset = symbolic::binary(
                "mul",
                (**index).clone(),
                symbolic::integer(*width as u128, bits, false),
            )?;
            let mut translated = vec![MemoryProjection::Slice {
                offset: Box::new(offset),
                length: Box::new(symbolic::integer(*width as u128, bits, false)),
            }];
            translated.extend_from_slice(rest);
            return self.write_projection(storage, &translated, value, conditions);
        }
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
                if data.len() > MAX_QUERY_BYTES {
                    return Err("byte storage expression exceeds the query-size budget".to_owned());
                }
                Ok(())
            }
            _ => Err("unsupported projected memory write".to_owned()),
        }
    }

    fn merge_byte_view(
        &self,
        storage: &mut Value,
        offset: &Value,
        length: &Value,
        view: Value,
        conditions: &[String],
    ) -> Result<(), String> {
        let Value::Bytes {
            length: capacity,
            data,
        } = storage
        else {
            return Err("byte view requires byte allocation storage".to_owned());
        };
        let Value::Bytes {
            length: source_length,
            data: source,
        } = view
        else {
            return Err("byte view assignment requires a byte array".to_owned());
        };
        let equal = symbolic::binary("eq", (*source_length).clone(), length.clone())?.boolean()?;
        if self.feasible(&[conditions.to_vec(), vec![symbolic::not(&equal)]].concat())? {
            return Err("byte view assignment changed its length".to_owned());
        }
        let Some(crate::solver::ground::Constant::BitVec { value: count, .. }) =
            crate::solver::ground::constant(&capacity.integer()?.0)
        else {
            return Err("byte view updates need a fixed allocation capacity".to_owned());
        };
        if count > 128 {
            return Err("byte view allocation exceeds the 128-byte budget".to_owned());
        }
        let (offset, bits, signed) = offset.integer()?;
        let (length, length_bits, length_signed) = length.integer()?;
        if signed || length_signed || bits != length_bits {
            return Err("byte view index type mismatch".to_owned());
        }
        let fixed_length = match crate::solver::ground::constant(&length) {
            Some(crate::solver::ground::Constant::BitVec { value, .. }) if value <= count => {
                Some(value)
            }
            _ => None,
        };
        // SMT let bindings evaluate their values outside the binding scope. Nested copies
        // therefore cannot capture these names in an earlier array expression.
        let mut copied = "byte_copy_destination".to_owned();
        for index in 0..fixed_length.unwrap_or(count) {
            let cell = format!("(_ bv{index} {bits})");
            let target = format!("(bvadd byte_copy_offset {cell})");
            let byte = if fixed_length.is_some() {
                format!("(select byte_copy_source {cell})")
            } else {
                format!(
                    "(ite (bvult {cell} byte_copy_length) (select byte_copy_source {cell}) (select byte_copy_destination {target}))"
                )
            };
            copied = format!("(store {copied} {target} {byte})");
        }
        *data = format!(
            "(let ((byte_copy_source {source}) (byte_copy_destination {data}) (byte_copy_offset {offset}) (byte_copy_length {length})) {copied})"
        );
        if data.len() > MAX_QUERY_BYTES {
            return Err("byte view expression exceeds the query-size budget".to_owned());
        }
        Ok(())
    }

    pub(super) fn callback_environment(
        &self,
        callback: &Value,
        state: &State,
    ) -> Result<(Value, Vec<Option<Value>>), String> {
        self.validate_tracked_value(callback, state)?;
        if state.memory.len() >= MAX_ALLOCATIONS {
            return Err("memory allocation budget reached".to_owned());
        }
        let mut memory = state.memory.clone();
        let allocation = memory.len();
        memory.push(Some(callback.clone()));
        Ok((
            Value::Reference {
                allocation,
                projection: Vec::new(),
                mutable: true,
            },
            memory,
        ))
    }

    pub(super) fn retire_callback_environment(&self, callback: &Value, results: &mut [Return]) {
        if let Value::Reference { allocation, .. } = callback {
            for result in results {
                result.memory[*allocation] = None;
            }
        }
    }

    pub(super) fn validate_tracked_value(
        &self,
        value: &Value,
        state: &State,
    ) -> Result<(), String> {
        Self::validate_reference_graph(value, state, None, &mut Vec::new())
    }

    pub(super) fn validate_frame_escape(
        &self,
        value: &Value,
        state: &State,
        incoming: usize,
    ) -> Result<(), String> {
        self.return_value(value.clone(), state, incoming)?;
        for value in state.memory[..incoming].iter().flatten() {
            Self::validate_reference_graph(value, state, Some(incoming), &mut Vec::new())?;
        }
        Ok(())
    }

    fn validate_reference_graph(
        value: &Value,
        state: &State,
        incoming: Option<usize>,
        visited: &mut Vec<usize>,
    ) -> Result<(), String> {
        match value {
            Value::Reference { allocation, .. } | Value::Cell { allocation } => {
                if matches!(value, Value::Reference { .. })
                    && incoming.is_some_and(|limit| *allocation >= limit)
                {
                    return Err(
                        "reference to frame-owned storage cannot escape its frame".to_owned()
                    );
                }
                let stored = state
                    .memory
                    .get(*allocation)
                    .and_then(Option::as_ref)
                    .ok_or("reference points to dead or uninitialized storage")?;
                if !visited.contains(allocation) {
                    visited.push(*allocation);
                    Self::validate_reference_graph(stored, state, incoming, visited)?;
                }
                Ok(())
            }
            Value::Adt { fields, .. } => {
                for (_, value) in fields {
                    Self::validate_reference_graph(value, state, incoming, visited)?;
                }
                Ok(())
            }
            Value::Enum { variants, .. } => {
                for value in variants {
                    Self::validate_reference_graph(value, state, incoming, visited)?;
                }
                Ok(())
            }
            Value::Tuple(fields) | Value::Elements(fields) => {
                for value in fields {
                    Self::validate_reference_graph(value, state, incoming, visited)?;
                }
                Ok(())
            }
            Value::SliceIterator { source, .. } | Value::MetadataPointer(source) => {
                Self::validate_reference_graph(source, state, incoming, visited)
            }
            Value::Bool(_)
            | Value::Int { .. }
            | Value::Float { .. }
            | Value::Bytes { .. }
            | Value::Atomic { .. }
            | Value::StaticText
            | Value::FormatArguments
            | Value::Function
            | Value::Unit => Ok(()),
        }
    }

    pub(super) fn return_value(
        &self,
        value: Value,
        state: &State,
        incoming: usize,
    ) -> Result<Value, String> {
        match value {
            Value::SliceIterator {
                source,
                front,
                back,
                mutable,
            } => Ok(Value::SliceIterator {
                source: Box::new(self.return_value(*source, state, incoming)?),
                front,
                back,
                mutable,
            }),
            Value::Reference {
                allocation,
                mutable,
                ..
            } if allocation >= incoming => {
                if mutable {
                    return Err("mutable local borrows cannot escape their frame".to_owned());
                }
                let referent = self.reference_value(&value, &state.memory, &state.conditions)?;
                self.return_value(referent, state, incoming)
            }
            value @ (Value::Reference { .. } | Value::Cell { .. }) => {
                Self::validate_reference_graph(&value, state, Some(incoming), &mut Vec::new())?;
                Ok(value)
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
            Value::Enum {
                discriminant,
                variants,
                is_option,
            } => Ok(Value::Enum {
                discriminant,
                variants: variants
                    .into_iter()
                    .map(|value| self.return_value(value, state, incoming))
                    .collect::<Result<_, _>>()?,
                is_option,
            }),
            Value::MetadataPointer(value) => Ok(Value::MetadataPointer(Box::new(
                self.return_value(*value, state, incoming)?,
            ))),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn state(memory: Vec<Option<Value>>) -> State {
        State {
            locals: Vec::new(),
            addresses: Vec::new(),
            conditions: Vec::new(),
            memory,
        }
    }

    fn reference(allocation: usize, mutable: bool) -> Value {
        Value::Reference {
            allocation,
            projection: Vec::new(),
            mutable,
        }
    }

    #[test]
    fn a_frame_borrow_hidden_inside_caller_storage_cannot_escape() {
        let caller = Value::Tuple(vec![reference(1, true)]);
        let state = state(vec![Some(caller), Some(symbolic::integer(7, 16, false))]);
        let error = Engine::validate_reference_graph(
            &reference(0, false),
            &state,
            Some(1),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(error.contains("frame-owned"));
    }

    #[test]
    fn dead_borrows_are_rejected_inside_aggregates() {
        let borrowed = Value::Tuple(vec![reference(0, true)]);
        let state = state(vec![None]);
        let error =
            Engine::validate_reference_graph(&borrowed, &state, None, &mut Vec::new()).unwrap_err();
        assert!(error.contains("dead or uninitialized"));
    }

    #[test]
    fn nested_incoming_references_keep_their_storage_identity() {
        let borrowed = Value::Tuple(vec![reference(0, true), reference(1, false)]);
        let state = state(vec![
            Some(symbolic::integer(7, 16, false)),
            Some(Value::Tuple(vec![reference(0, false)])),
        ]);
        Engine::validate_reference_graph(&borrowed, &state, Some(2), &mut Vec::new()).unwrap();
    }
}
