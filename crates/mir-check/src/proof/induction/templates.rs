use super::*;
use symbolic::MemoryProjection;

impl<'tcx> Engine<'tcx> {
    pub(super) fn loop_template_projection(
        &self,
        value: Value,
        part: &MemoryProjection,
    ) -> Result<Value, String> {
        match part {
            MemoryProjection::Field(_) | MemoryProjection::Variant(_) => {
                storage::static_projection(value, part)
            }
            MemoryProjection::Index(index) => match value {
                Value::Bytes { data, .. } => Ok(Value::Int {
                    expression: self.terms.apply(Op::Select, &[data, index.integer()?.0])?,
                    bits: 8,
                    signed: false,
                }),
                Value::Elements(fields) => symbolic::select_element(&self.terms, &fields, index),
                Value::Bool(_)
                | Value::Int { .. }
                | Value::Float { .. }
                | Value::Adt { .. }
                | Value::Enum { .. }
                | Value::Cell { .. }
                | Value::Atomic { .. }
                | Value::Reference { .. }
                | Value::SliceIterator { .. }
                | Value::Tuple(_)
                | Value::MetadataPointer(_)
                | Value::StaticText
                | Value::FormatArguments
                | Value::Input(_)
                | Value::RawPointer { .. }
                | Value::StaticSlice { .. }
                | Value::StaticView { .. }
                | Value::Uninitialized
                | Value::FunctionPointer { .. }
                | Value::Function
                | Value::Unit => Err("inductive indexed storage is unsupported".into()),
            },
            MemoryProjection::Slice { .. } | MemoryProjection::Chunks { .. } => {
                Err("inductive slice views need their own reference layout".into())
            }
        }
    }

    pub(super) fn loop_template_place(
        &self,
        state: &State,
        place: Place<'tcx>,
    ) -> Result<Option<Value>, String> {
        let Ok(mut value) = self.local(state, place.local.as_usize()) else {
            return Ok(None);
        };
        for part in place.projection {
            value = match part {
                ProjectionElem::Deref => {
                    let Value::Reference {
                        allocation,
                        projection,
                        ..
                    } = value
                    else {
                        return Err("inductive template dereference needs typed storage".into());
                    };
                    let Some(mut value) = state
                        .memory
                        .get(allocation)
                        .and_then(Option::as_ref)
                        .cloned()
                    else {
                        return Ok(None);
                    };
                    for part in &projection {
                        value = self.loop_template_projection(value, part)?;
                    }
                    value
                }
                ProjectionElem::Field(field, _) => self
                    .loop_template_projection(value, &MemoryProjection::Field(field.as_usize()))?,
                ProjectionElem::Downcast(_, variant) => self.loop_template_projection(
                    value,
                    &MemoryProjection::Variant(variant.as_usize()),
                )?,
                ProjectionElem::Index(index) => self.loop_template_projection(
                    value,
                    &MemoryProjection::Index(Box::new(self.local(state, index.as_usize())?)),
                )?,
                other => return Err(format!("inductive template place unsupported: {other:?}")),
            };
        }
        Ok(Some(value))
    }

    pub(super) fn loop_template_snapshot(
        &self,
        value: &Value,
        state: &State,
    ) -> Result<Value, String> {
        self.loop_snapshot_depth(value, state, 0)
    }

    fn loop_snapshot_depth(
        &self,
        value: &Value,
        state: &State,
        depth: usize,
    ) -> Result<Value, String> {
        if depth >= MAX_INPUT_DEPTH {
            return Err("inductive snapshot depth limit".into());
        }
        match value {
            Value::Reference {
                allocation,
                projection,
                ..
            } => {
                let mut referent = state
                    .memory
                    .get(*allocation)
                    .and_then(Option::as_ref)
                    .cloned()
                    .ok_or("inductive snapshot storage missing")?;
                for part in projection {
                    referent = self.loop_template_projection(referent, part)?;
                }
                self.loop_snapshot_depth(&referent, state, depth + 1)
            }
            Value::Tuple(fields) => Ok(Value::Tuple(
                fields
                    .iter()
                    .map(|field| self.loop_snapshot_depth(field, state, depth + 1))
                    .collect::<Result<_, _>>()?,
            )),
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
                            self.loop_snapshot_depth(value, state, depth + 1)?,
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
                    .map(|value| self.loop_snapshot_depth(value, state, depth + 1))
                    .collect::<Result<_, _>>()?,
            }),
            Value::Elements(fields) => Ok(Value::Elements(
                fields
                    .iter()
                    .map(|field| self.loop_snapshot_depth(field, state, depth + 1))
                    .collect::<Result<_, _>>()?,
            )),
            Value::Bool(_)
            | Value::Int { .. }
            | Value::Float { .. }
            | Value::Bytes { .. }
            | Value::Cell { .. }
            | Value::Atomic { .. }
            | Value::SliceIterator { .. }
            | Value::MetadataPointer(_)
            | Value::StaticText
            | Value::FormatArguments
            | Value::RawPointer { .. }
            | Value::StaticSlice { .. }
            | Value::StaticView { .. }
            | Value::Uninitialized
            | Value::FunctionPointer { .. }
            | Value::Function
            | Value::Unit => Ok(value.clone()),
            Value::Input(_) => Err("lazy input snapshots are not supported by induction".into()),
        }
    }

    pub(super) fn loop_template_operand(
        &self,
        id: DefId,
        body: &Body<'tcx>,
        state: &State,
        operand: &Operand<'tcx>,
    ) -> Result<Option<Value>, String> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => self.loop_template_place(state, *place),
            Operand::Constant(_) | Operand::RuntimeChecks(_) => {
                self.operand(id, body, state, operand).map(Some)
            }
        }
    }

    pub(super) fn loop_call_template(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        state: &State,
    ) -> Result<Option<Value>, String> {
        if let Some(value) = self.loop_slice_template(instance, values, state)? {
            return Ok(Some(value));
        }
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx
                    .fn_sig(instance.def_id())
                    .instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("template call normalization failed: {error:?}"))?
            .skip_binder();
        if let ty::Ref(_, pointee, mutability) = signature.output().kind() {
            for (input, value) in signature.inputs().iter().zip(values) {
                if matches!(input.kind(), ty::Ref(_, input, writable)
                    if input == pointee && (!mutability.is_mut() || writable.is_mut()))
                    && let Value::Reference {
                        allocation,
                        projection,
                        ..
                    } = value
                {
                    // This chooses a layout only. Actual callee transitions and returned
                    // allocation identities must still match it before any proof is accepted.
                    return Ok(Some(Value::Reference {
                        allocation: *allocation,
                        projection: projection.clone(),
                        mutable: mutability.is_mut(),
                    }));
                }
            }
            if let [
                Value::Enum {
                    variants,
                    is_option: true,
                    ..
                },
            ] = values
                && let Some(Value::Adt { fields, .. }) = variants.get(1)
                && let [(_, value @ Value::Reference { .. })] = fields.as_slice()
            {
                return Ok(Some(value.clone()));
            }
        }
        Ok(None)
    }

    pub(super) fn loop_reference_bounds(
        &self,
        state: &mut State,
        reference: &Value,
        system: &mut System,
        premise: &Atom,
    ) -> Result<(), String> {
        let Value::Reference {
            allocation,
            projection,
            ..
        } = reference
        else {
            return Ok(());
        };
        let mut value = state
            .memory
            .get(*allocation)
            .and_then(Option::as_ref)
            .cloned()
            .ok_or("inductive reference storage missing")?;
        for part in projection {
            let safe = match part {
                MemoryProjection::Variant(index) => {
                    if let Value::Enum {
                        discriminant,
                        variants,
                        ..
                    } = &value
                    {
                        let Value::Adt {
                            discriminant: tag, ..
                        } = variants
                            .get(*index)
                            .ok_or("inductive reference variant missing")?
                        else {
                            return Err("inductive reference payload missing".into());
                        };
                        let (_, bits, signed) = discriminant.integer()?;
                        Some(
                            symbolic::binary(
                                &self.terms,
                                "eq",
                                (**discriminant).clone(),
                                symbolic::integer(&self.terms, *tag, bits, signed),
                            )?
                            .boolean()?,
                        )
                    } else {
                        None
                    }
                }
                MemoryProjection::Index(index) => Some(
                    symbolic::binary(
                        &self.terms,
                        "lt",
                        (**index).clone(),
                        self.loop_slice_length(&value)?,
                    )?
                    .boolean()?,
                ),
                MemoryProjection::Field(_) => None,
                MemoryProjection::Slice { .. } | MemoryProjection::Chunks { .. } => {
                    return Err("inductive reference view unsupported".into());
                }
            };
            if let Some(safe) = safe {
                exclude_failure(system, premise, &state.conditions, &safe);
                state.conditions.push(safe);
            }
            value = self.loop_template_projection(value, part)?;
        }
        Ok(())
    }
}
