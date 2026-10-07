use super::*;
use rustc_middle::mir::visit::{PlaceContext, Visitor};
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
            Value::Enum {
                discriminant,
                variants,
                is_option,
            } => Value::Enum {
                discriminant: Box::new(self.loop_fresh_value(discriminant)?),
                variants: variants
                    .iter()
                    .map(|value| self.loop_fresh_value(value))
                    .collect::<Result<_, _>>()?,
                is_option: *is_option,
            },
            Value::Reference {
                allocation,
                projection,
                mutable,
            } => Value::Reference {
                allocation: *allocation,
                mutable: *mutable,
                projection: projection
                    .iter()
                    .map(|part| match part {
                        MemoryProjection::Field(_) | MemoryProjection::Variant(_) => {
                            Ok(part.clone())
                        }
                        MemoryProjection::Index(index) => Ok(MemoryProjection::Index(Box::new(
                            self.loop_fresh_value(index)?,
                        ))),
                        MemoryProjection::Slice { .. } | MemoryProjection::Chunks { .. } => {
                            Err("unsupported inductive reference view".into())
                        }
                    })
                    .collect::<Result<_, String>>()?,
            },
            Value::SliceIterator {
                source,
                front,
                back,
                mutable,
            } => Value::SliceIterator {
                source: Box::new(self.loop_fresh_value(source)?),
                front: Box::new(self.loop_fresh_value(front)?),
                back: Box::new(self.loop_fresh_value(back)?),
                mutable: *mutable,
            },
            Value::MetadataPointer(length) => {
                Value::MetadataPointer(Box::new(self.loop_fresh_value(length)?))
            }
            Value::Unit => Value::Unit,
            Value::Input(_) => {
                return Err("unmaterialized lazy input is unsupported by induction".into());
            }
            other @ (Value::Float { .. }
            | Value::Cell { .. }
            | Value::Atomic { .. }
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
        let mut used = UsedLocals(BTreeSet::new());
        used.visit_body(body);
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
            } else if local.as_usize() != 0 && !used.0.contains(&local.as_usize()) {
                Some(Value::Unit)
            } else if self.loop_deferred_type(declaration.ty) {
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
                        || !self.loop_deferred_type(body.local_decls[destination.local].ty)
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
                            self.loop_template_place(&state, *place)?
                        }
                        Rvalue::Cast(CastKind::PointerCoercion(..), operand, _) => match operand {
                            Operand::Copy(place) | Operand::Move(place) => {
                                self.loop_template_place(&state, *place)?
                            }
                            Operand::Constant(_) | Operand::RuntimeChecks(_) => None,
                        },
                        _ => continue,
                    };
                    if let Some(value) = value {
                        let value = self.loop_fresh_value(&value)?;
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
                if let TerminatorKind::Call {
                    func,
                    args,
                    destination,
                    ..
                } = &block.terminator().kind
                    && destination.projection.is_empty()
                    && self.loop_deferred_type(body.local_decls[destination.local].ty)
                    && let ty::FnDef(id, generics) = *func.ty(&body.local_decls, self.tcx).kind()
                    && let Some(instance) = ty::Instance::try_resolve(
                        self.tcx,
                        ty::TypingEnv::fully_monomorphized(),
                        id,
                        generics.skip_binder(),
                    )
                    .map_err(|error| format!("iterator template resolution failed: {error:?}"))?
                {
                    let values = args
                        .iter()
                        .map(|arg| {
                            self.loop_template_operand(
                                body.source.def_id(),
                                body,
                                &state,
                                &arg.node,
                            )
                        })
                        .collect::<Result<Option<Vec<_>>, _>>()?;
                    if let Some(values) = values
                        && !values.iter().any(|value| {
                            matches!(value, Value::Reference { allocation, .. }
                            if state.memory.get(*allocation).and_then(Option::as_ref).is_none())
                        })
                        && let Some(value) = self.loop_call_template(instance, &values, &state)?
                    {
                        let value = self.loop_fresh_value(&value)?;
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
        if state.memory.len() > 512 {
            return Err("inductive memory exceeds its 512-allocation budget".into());
        }
        if state.locals.iter().any(Option::is_none) || state.memory.iter().any(Option::is_none) {
            let missing = state
                .locals
                .iter()
                .enumerate()
                .filter(|(_, value)| value.is_none())
                .map(|(index, _)| format!("_{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "inductive reference target could not be determined: {missing}"
            ));
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
                    let Some(stored) = state.memory.get(*target).and_then(Option::as_ref).cloned()
                    else {
                        return Ok(None);
                    };
                    value = stored;
                    for part in &projection {
                        value = self.loop_template_projection(value, part)?;
                    }
                }
                ProjectionElem::Field(field, _) => {
                    let index = field.as_usize();
                    value = match value {
                        Value::Tuple(fields) => fields.get(index).cloned(),
                        Value::Adt { fields, .. } => {
                            fields.get(index).map(|(_, value)| value.clone())
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
                        | Value::Function
                        | Value::Unit => {
                            return Err("inductive borrow field has unsupported storage".into());
                        }
                    }
                    .ok_or("inductive borrowed field missing")?;
                    projection.push(MemoryProjection::Field(index));
                }
                ProjectionElem::Downcast(_, variant) => {
                    let index = variant.as_usize();
                    value = static_projection(value, &MemoryProjection::Variant(index))?;
                    projection.push(MemoryProjection::Variant(index));
                }
                ProjectionElem::Index(index) => {
                    let index = self.local(state, index.as_usize())?;
                    value = self.loop_template_projection(
                        value,
                        &MemoryProjection::Index(Box::new(index.clone())),
                    )?;
                    projection.push(MemoryProjection::Index(Box::new(index)));
                }
                other => {
                    return Err(format!(
                        "inductive borrow needs typed projections: {other:?}"
                    ));
                }
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
            for snapshot in arguments
                .iter()
                .map(|value| self.loop_template_snapshot(value, &state))
                .collect::<Result<Vec<_>, _>>()?
            {
                flatten_value(&snapshot, &mut captured)?;
            }
        }
        self.loop_normalize(frame, &mut state)?;
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
        ) if a == d && c == f && b.len() == e.len() => {
            for (a, b) in b.iter().zip(e) {
                projection_shape(a, b)?;
            }
            Ok(())
        }
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
        (
            Value::Enum {
                discriminant: a,
                variants: b,
                is_option: c,
            },
            Value::Enum {
                discriminant: d,
                variants: e,
                is_option: f,
            },
        ) if c == f && b.len() == e.len() => {
            same_shape(Some(a), Some(d))?;
            for (a, b) in b.iter().zip(e) {
                same_shape(Some(a), Some(b))?;
            }
            Ok(())
        }
        (
            Value::SliceIterator {
                source: a,
                front: b,
                back: c,
                mutable: d,
            },
            Value::SliceIterator {
                source: e,
                front: f,
                back: g,
                mutable: h,
            },
        ) if d == h => {
            same_shape(Some(a), Some(e))?;
            same_shape(Some(b), Some(f))?;
            same_shape(Some(c), Some(g))
        }
        (Value::MetadataPointer(a), Value::MetadataPointer(b)) => same_shape(Some(a), Some(b)),
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
            | Value::Function
            | Value::Unit,
            _,
        ) => Err("inductive value or reference identity changed".into()),
    }
}

pub(super) fn static_projection(value: Value, part: &MemoryProjection) -> Result<Value, String> {
    match (part, value) {
        (MemoryProjection::Field(index), Value::Tuple(fields)) => fields
            .get(*index)
            .cloned()
            .ok_or("inductive tuple field missing".into()),
        (MemoryProjection::Field(index), Value::Adt { fields, .. }) => fields
            .get(*index)
            .map(|(_, value)| value.clone())
            .ok_or("inductive field missing".into()),
        (MemoryProjection::Variant(index), Value::Enum { variants, .. }) => variants
            .get(*index)
            .cloned()
            .ok_or("inductive variant missing".into()),
        (MemoryProjection::Variant(index), value @ Value::Adt { .. }) => {
            if matches!(&value, Value::Adt { variant, .. } if variant == index) {
                Ok(value)
            } else {
                Err("inductive variant identity changed".into())
            }
        }
        (
            MemoryProjection::Field(_)
            | MemoryProjection::Variant(_)
            | MemoryProjection::Index(_)
            | MemoryProjection::Slice { .. }
            | MemoryProjection::Chunks { .. },
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
            | Value::Function
            | Value::Unit,
        ) => Err("unsupported inductive reference projection".into()),
    }
}

struct UsedLocals(BTreeSet<usize>);

impl<'tcx> Visitor<'tcx> for UsedLocals {
    fn visit_local(
        &mut self,
        local: rustc_middle::mir::Local,
        context: PlaceContext,
        _location: rustc_middle::mir::Location,
    ) {
        if context.is_use() {
            self.0.insert(local.as_usize());
        }
    }
}

fn projection_shape(a: &MemoryProjection, b: &MemoryProjection) -> Result<(), String> {
    match (a, b) {
        (MemoryProjection::Field(a), MemoryProjection::Field(b))
        | (MemoryProjection::Variant(a), MemoryProjection::Variant(b))
            if a == b =>
        {
            Ok(())
        }
        (MemoryProjection::Index(a), MemoryProjection::Index(b)) => same_shape(Some(a), Some(b)),
        (
            MemoryProjection::Field(_)
            | MemoryProjection::Variant(_)
            | MemoryProjection::Index(_)
            | MemoryProjection::Slice { .. }
            | MemoryProjection::Chunks { .. },
            _,
        ) => Err("inductive reference projection identity changed".into()),
    }
}
