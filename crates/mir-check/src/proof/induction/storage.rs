use super::*;
use symbolic::MemoryProjection;

impl<'tcx> Engine<'tcx> {
    pub(super) fn loop_root_storage(
        &self,
        instance: ty::Instance<'tcx>,
        mut arguments: Vec<Value>,
        mut memory: Vec<Option<Value>>,
    ) -> Result<(Vec<Value>, Vec<Option<Value>>), String> {
        let body = self.instantiated_body(instance)?;
        for (local, argument) in body.args_iter().zip(&mut arguments) {
            if let ty::Ref(_, pointee, mutable) = body.local_decls[local].ty.kind() {
                if !pointee.is_freeze(self.tcx, ty::TypingEnv::fully_monomorphized()) {
                    return Err(
                        "inductive references require storage without interior mutation".into(),
                    );
                }
                if !matches!(argument, Value::Reference { .. }) {
                    let allocation = memory.len();
                    memory.push(Some(argument.clone()));
                    *argument = Value::Reference {
                        allocation,
                        projection: Vec::new(),
                        mutable: mutable.is_mut(),
                    };
                }
            }
        }
        Ok((arguments, memory))
    }

    pub(super) fn loop_fresh_value(&mut self, value: &Value) -> Result<Value, String> {
        Ok(match value {
            Value::Bool(_) => Value::Bool(self.fresh(Sort::Bool)),
            Value::Int { bits, signed, .. } => Value::Int {
                expression: self.fresh(Sort::BitVec(*bits)),
                bits: *bits,
                signed: *signed,
            },
            Value::Bytes { length, data } => Value::Bytes {
                length: length.clone(),
                data: self.fresh(data.sort().clone()),
            },
            Value::Adt {
                name,
                variant,
                is_option,
                discriminant,
                fields,
            } => Value::Adt {
                name: name.clone(),
                variant: *variant,
                is_option: *is_option,
                discriminant: *discriminant,
                fields: fields
                    .iter()
                    .map(|(name, value)| Ok((name.clone(), self.loop_fresh_value(value)?)))
                    .collect::<Result<_, String>>()?,
            },
            Value::Tuple(fields) => Value::Tuple(
                fields
                    .iter()
                    .map(|value| self.loop_fresh_value(value))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Elements(fields) => Value::Elements(
                fields
                    .iter()
                    .map(|value| self.loop_fresh_value(value))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Reference { .. } => {
                flatten_value(value, &mut Vec::new())?;
                value.clone()
            }
            Value::MetadataPointer(length) => {
                Value::MetadataPointer(Box::new(self.loop_fresh_value(length)?))
            }
            Value::Unit => Value::Unit,
            other @ (Value::Float { .. }
            | Value::Enum { .. }
            | Value::Cell { .. }
            | Value::Atomic { .. }
            | Value::SliceIterator { .. }
            | Value::StaticText
            | Value::FormatArguments
            | Value::Function) => {
                return Err(format!("unsupported inductive storage {other:?}"));
            }
        })
    }

    pub(super) fn loop_storage(
        &mut self,
        body: &Body<'tcx>,
        arguments: &[Value],
        incoming: &[Option<Value>],
    ) -> Result<State, String> {
        if arguments.len() != body.arg_count {
            return Err("inductive arguments do not match MIR".into());
        }
        let mut state = State {
            locals: Vec::new(),
            conditions: Vec::new(),
            addresses: vec![None; body.local_decls.len()],
            memory: incoming
                .iter()
                .map(|cell| {
                    cell.as_ref()
                        .ok_or_else(|| "unavailable incoming storage".to_owned())
                        .and_then(|value| self.loop_fresh_value(value))
                        .map(Some)
                })
                .collect::<Result<_, _>>()?,
        };
        for (local, declaration) in body.local_decls.iter_enumerated() {
            let value = if local.as_usize() > 0 && local.as_usize() <= arguments.len() {
                Some(self.loop_fresh_value(&arguments[local.as_usize() - 1])?)
            } else if declaration.ty.is_ref() {
                None
            } else if matches!(declaration.ty.kind(), ty::RawPtr(..)) {
                let bits = u32::from(self.tcx.sess.target.pointer_width);
                Some(Value::MetadataPointer(Box::new(Value::Int {
                    expression: self.fresh(Sort::BitVec(bits)),
                    bits,
                    signed: false,
                })))
            } else {
                Some(self.loop_input(declaration.ty, 0)?)
            };
            state.locals.push(value);
        }
        for block in body.basic_blocks.iter() {
            for statement in &block.statements {
                if let StatementKind::Assign(assignment) = &statement.kind
                    && let Rvalue::Ref(_, _, place) = &assignment.1
                    && !place.projection.contains(&ProjectionElem::Deref)
                {
                    let local = place.local.as_usize();
                    if state.addresses[local].is_none() {
                        state.addresses[local] = Some(state.memory.len());
                        state.memory.push(state.locals[local].clone());
                    }
                }
            }
        }
        for _ in 0..body.local_decls.len() {
            let mut changed = false;
            for block in body.basic_blocks.iter() {
                for statement in &block.statements {
                    let StatementKind::Assign(assignment) = &statement.kind else {
                        continue;
                    };
                    let (destination, rvalue) = assignment.as_ref();
                    if !destination.projection.is_empty()
                        || !body.local_decls[destination.local].ty.is_ref()
                    {
                        continue;
                    }
                    let value = match rvalue {
                        Rvalue::Ref(_, BorrowKind::Shared | BorrowKind::Mut { .. }, place) => self
                            .loop_alias(
                                &state,
                                *place,
                                matches!(rvalue, Rvalue::Ref(_, BorrowKind::Mut { .. }, _)),
                            )?,
                        Rvalue::Use(Operand::Copy(place) | Operand::Move(place), _) => {
                            self.place(&state, *place).ok()
                        }
                        Rvalue::Cast(CastKind::PointerCoercion(..), operand, _) => match operand {
                            Operand::Copy(place) | Operand::Move(place) => {
                                self.place(&state, *place).ok()
                            }
                            Operand::Constant(_) | Operand::RuntimeChecks(_) => None,
                        },
                        _ => {
                            return Err("inductive reference assignment has no stable model".into());
                        }
                    };
                    if let Some(value) = value {
                        let local = destination.local.as_usize();
                        if let Some(previous) = &state.locals[local] {
                            same_shape(Some(previous), Some(&value))?;
                        } else {
                            state.locals[local] = Some(value.clone());
                            if let Some(allocation) = state.addresses[local] {
                                state.memory[allocation] = Some(value);
                            }
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        if state.locals.iter().any(Option::is_none) || state.memory.iter().any(Option::is_none) {
            return Err("inductive reference target could not be determined".into());
        }
        Ok(state)
    }

    pub(super) fn loop_alias(
        &self,
        state: &State,
        place: Place<'tcx>,
        mutable: bool,
    ) -> Result<Option<Value>, String> {
        let mut allocation = state.addresses[place.local.as_usize()];
        let mut projection = Vec::new();
        let Ok(mut value) = self.local(state, place.local.as_usize()) else {
            return Ok(None);
        };
        for part in place.projection {
            match part {
                ProjectionElem::Deref => {
                    let Value::Reference {
                        allocation: target,
                        projection: path,
                        mutable: writable,
                    } = &value
                    else {
                        return Err("inductive dereference has no typed allocation".into());
                    };
                    if mutable && !writable {
                        return Err("inductive mutable borrow through shared reference".into());
                    }
                    allocation = Some(*target);
                    projection = path.clone();
                    value = self.reference_value(&value, &state.memory, &[])?;
                }
                ProjectionElem::Field(field, _) => {
                    let index = field.as_usize();
                    value = match value {
                        Value::Tuple(fields) => fields.get(index).cloned(),
                        Value::Adt { fields, .. } => {
                            fields.get(index).map(|(_, value)| value.clone())
                        }
                        _ => return Err("inductive borrow field has unsupported storage".into()),
                    }
                    .ok_or("inductive borrowed field missing")?;
                    projection.push(MemoryProjection::Field(index));
                }
                other => return Err(format!("inductive borrow needs static fields: {other:?}")),
            }
        }
        Ok(allocation.map(|allocation| Value::Reference {
            allocation,
            projection,
            mutable,
        }))
    }

    pub(super) fn loop_entry_atom(
        &self,
        frame: &Frame<'tcx>,
        arguments: &[Value],
        incoming: &[Option<Value>],
        mut captured: Vec<Term>,
    ) -> Result<Atom, String> {
        let mut state = frame.state.clone();
        state.memory[..incoming.len()].clone_from_slice(incoming);
        for (local, value) in frame.body.args_iter().zip(arguments) {
            if let Some(allocation) = state.addresses[local.as_usize()] {
                state.memory[allocation] = Some(value.clone());
            } else {
                state.locals[local.as_usize()] = Some(value.clone());
            }
        }
        if !frame.entry.is_empty() {
            for snapshot in self.snapshots(arguments, incoming, &[])? {
                flatten_value(&snapshot, &mut captured)?;
            }
        }
        let mut entry = frame.clone();
        entry.ghosts = captured;
        entry.atom(START_BLOCK, &state)
    }
}

pub(super) fn same_shape(expected: Option<&Value>, actual: Option<&Value>) -> Result<(), String> {
    let (Some(expected), Some(actual)) = (expected, actual) else {
        return Err("inductive storage became unavailable".into());
    };
    let mut left = Vec::new();
    let mut right = Vec::new();
    flatten_value(expected, &mut left)?;
    flatten_value(actual, &mut right)?;
    if left.iter().map(Term::sort).ne(right.iter().map(Term::sort)) {
        return Err("inductive storage sorts changed".into());
    }
    match (expected, actual) {
        (Value::Bool(_), Value::Bool(_)) | (Value::Unit, Value::Unit) => Ok(()),
        (
            Value::Int {
                bits: a, signed: b, ..
            },
            Value::Int {
                bits: c, signed: d, ..
            },
        ) if a == c && b == d => Ok(()),
        (Value::Bytes { length: a, .. }, Value::Bytes { length: b, .. }) => {
            // Fixed byte lengths stay structural, not unconstrained state parameters.
            if a.integer()? == b.integer()? {
                Ok(())
            } else {
                Err("inductive byte storage length changed".into())
            }
        }
        (
            Value::Reference {
                allocation: a,
                projection: b,
                mutable: c,
            },
            Value::Reference {
                allocation: d,
                projection: e,
                mutable: f,
            },
        ) if a == d && c == f && b.len() == e.len() && b.iter().zip(e).all(|(a, b)| matches!((a, b), (MemoryProjection::Field(a), MemoryProjection::Field(b)) if a == b)) => Ok(()),
        (
            Value::Adt {
                name: a,
                variant: b,
                discriminant: c,
                fields: d,
                is_option: e,
            },
            Value::Adt {
                name: f,
                variant: g,
                discriminant: h,
                fields: i,
                is_option: j,
            },
        ) if a == f && b == g && c == h && e == j && d.len() == i.len() => {
            for ((a, b), (c, d)) in d.iter().zip(i) {
                if a != c {
                    return Err("inductive field identity changed".into());
                }
                same_shape(Some(b), Some(d))?;
            }
            Ok(())
        }
        (Value::Tuple(a), Value::Tuple(b)) | (Value::Elements(a), Value::Elements(b))
            if a.len() == b.len() =>
        {
            for (a, b) in a.iter().zip(b) {
                same_shape(Some(a), Some(b))?;
            }
            Ok(())
        }
        (Value::MetadataPointer(a), Value::MetadataPointer(b)) => same_shape(Some(a), Some(b)),
        _ => Err("inductive value or reference identity changed".into()),
    }
}
