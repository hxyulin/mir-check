use super::*;
use rustc_span::Symbol;
use symbolic::MemoryProjection;

#[derive(Clone, Copy)]
enum SliceOperation {
    New { mutable: bool },
    Identity,
    Next { reverse: bool, skip: bool },
    Length,
    SizeHint,
    Count,
    Clone,
    ByRef,
}

impl<'tcx> Engine<'tcx> {
    pub(super) fn loop_iterator_type(&self, ty: Ty<'tcx>) -> Option<bool> {
        let ty::Adt(def, _) = ty.kind() else {
            return None;
        };
        let shared = self.tcx.get_diagnostic_item(Symbol::intern("SliceIter"))?;
        if def.did() == shared {
            Some(false)
        } else if self.tcx.parent(def.did()) == self.tcx.parent(shared)
            && self.tcx.item_name(def.did()) == Symbol::intern("IterMut")
        {
            Some(true)
        } else {
            None
        }
    }

    pub(super) fn loop_deferred_type(&self, ty: Ty<'tcx>) -> bool {
        ty.is_ref()
            || self.loop_iterator_type(ty).is_some()
            || matches!(ty.kind(), ty::Adt(def, args)
                if self.tcx.lang_items().get(LangItem::Option) == Some(def.did())
                && args.type_at(0).is_ref())
    }

    fn loop_slice_operation(
        &mut self,
        instance: ty::Instance<'tcx>,
    ) -> Result<Option<SliceOperation>, String> {
        let callee = instance.def_id();
        let Some(shared) = self.tcx.get_diagnostic_item(Symbol::intern("SliceIter")) else {
            return Ok(None);
        };
        if callee.krate != shared.krate
            || self.specification(instance)?.is_some()
            || !self.configured_contracts(instance)?.is_empty()
        {
            return Ok(None);
        }
        let parent = self.tcx.parent(callee);
        let inherent = matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false });
        let trait_id = if matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: true }) {
            Some(
                self.tcx
                    .impl_trait_ref(parent)
                    .instantiate(self.tcx, instance.args)
                    .skip_norm_wip()
                    .def_id,
            )
        } else if self.tcx.def_kind(parent) == DefKind::Trait {
            Some(parent)
        } else {
            None
        };
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx.fn_sig(callee).instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("slice signature normalization failed: {error:?}"))?
            .skip_binder();
        let Some(first) = signature.inputs().first() else {
            return Ok(None);
        };
        let name = self.tcx.item_name(callee);
        let into_iter = trait_id.is_some()
            && trait_id == self.tcx.get_diagnostic_item(Symbol::intern("IntoIterator"))
            && name == Symbol::intern("into_iter");
        if let Some(mutable) = self.loop_iterator_type(signature.output())
            && let ty::Ref(_, pointee, mutability) = first.kind()
            && matches!(pointee.kind(), ty::Slice(_) | ty::Array(..))
            && mutable == mutability.is_mut()
            && signature.inputs().len() == 1
            && ((inherent && matches!(name.as_str(), "iter" | "iter_mut" | "new")) || into_iter)
        {
            return Ok(Some(SliceOperation::New { mutable }));
        }
        let receiver = match first.kind() {
            ty::Ref(_, pointee, _) => *pointee,
            _ => *first,
        };
        let Some(mutable) = self.loop_iterator_type(receiver) else {
            return Ok(None);
        };
        if into_iter && signature.output() == *first {
            return Ok(Some(SliceOperation::Identity));
        }
        let iterator = trait_id.is_some()
            && trait_id == self.tcx.get_diagnostic_item(Symbol::intern("Iterator"));
        let reverse = trait_id.is_some_and(|id| {
            id.krate == shared.krate
                && self.tcx.item_name(id) == Symbol::intern("DoubleEndedIterator")
        });
        let length = trait_id.is_some_and(|id| {
            id.krate == shared.krate
                && self.tcx.item_name(id) == Symbol::intern("ExactSizeIterator")
        });
        let operation = match name.as_str() {
            "next" | "nth" if iterator => Some(SliceOperation::Next {
                reverse: false,
                skip: name == Symbol::intern("nth"),
            }),
            "next_back" | "nth_back" if reverse => Some(SliceOperation::Next {
                reverse: true,
                skip: name == Symbol::intern("nth_back"),
            }),
            "len" if length => Some(SliceOperation::Length),
            "size_hint" if iterator => Some(SliceOperation::SizeHint),
            "count" if iterator => Some(SliceOperation::Count),
            "by_ref" if iterator && signature.output() == *first => Some(SliceOperation::ByRef),
            "clone"
                if !mutable
                    && trait_id.is_some()
                    && trait_id == self.tcx.lang_items().get(LangItem::Clone)
                    && signature.output() == receiver =>
            {
                Some(SliceOperation::Clone)
            }
            _ => None,
        };
        Ok(operation)
    }

    pub(super) fn loop_slice_template(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        state: &State,
    ) -> Result<Option<Value>, String> {
        let Some(operation) = self.loop_slice_operation(instance)? else {
            return Ok(None);
        };
        let receiver = values.first().ok_or("slice receiver missing")?;
        let output = self
            .tcx
            .fn_sig(instance.def_id())
            .instantiate(self.tcx, instance.args)
            .skip_binder()
            .output();
        let value = match operation {
            SliceOperation::New { mutable } => {
                let source = self.loop_template_snapshot(receiver, state)?;
                let length = self.loop_slice_length(&source)?;
                if !matches!(receiver, Value::Reference { mutable: writable, .. }
                    if !mutable || *writable)
                {
                    return Err("slice iterator needs typed storage".into());
                }
                Value::SliceIterator {
                    source: Box::new(receiver.clone()),
                    front: Box::new(self.iterator_index(0)),
                    back: Box::new(length),
                    mutable,
                }
            }
            SliceOperation::Identity | SliceOperation::ByRef => receiver.clone(),
            SliceOperation::Clone => self.loop_template_snapshot(receiver, state)?,
            SliceOperation::Next { .. } => {
                let iterator = self.loop_template_snapshot(receiver, state)?;
                let Value::SliceIterator {
                    source,
                    front,
                    mutable,
                    ..
                } = iterator
                else {
                    return Err("slice template has no iterator cursor".into());
                };
                let item = self.loop_slice_item(&source, *front, mutable)?;
                self.loop_reference_option(output, item)?
            }
            SliceOperation::Length | SliceOperation::Count => self.iterator_index(0),
            SliceOperation::SizeHint => {
                let ty::Tuple(fields) = output.kind() else {
                    return Err("slice size hint type".into());
                };
                Value::Tuple(vec![
                    self.iterator_index(0),
                    self.constructed(fields[1], 1, vec![self.iterator_index(0)])?,
                ])
            }
        };
        Ok(Some(value))
    }

    fn loop_reference_option(&mut self, output: Ty<'tcx>, item: Value) -> Result<Value, String> {
        let ty::Adt(def, _) = output.kind() else {
            return Err("iterator result needs Option".into());
        };
        if self.tcx.lang_items().get(LangItem::Option) != Some(def.did()) {
            return Err("iterator result is not core Option".into());
        }
        let (bits, signed) = self
            .integer_type(output.discriminant_ty(self.tcx))
            .ok_or("iterator Option tag type")?;
        Ok(Value::Enum {
            discriminant: Box::new(Value::Int {
                expression: self.fresh(Sort::BitVec(bits)),
                bits,
                signed,
            }),
            variants: vec![
                self.constructed(output, 0, Vec::new())?,
                self.constructed(output, 1, vec![item])?,
            ],
            is_option: true,
        })
    }

    fn loop_slice_item(
        &self,
        source: &Value,
        index: Value,
        mutable: bool,
    ) -> Result<Value, String> {
        let Value::Reference {
            allocation,
            projection,
            mutable: writable,
        } = source
        else {
            return Err("slice iterator source is not typed storage".into());
        };
        if mutable && !writable {
            return Err("mutable slice iterator has shared storage".into());
        }
        let mut projection = projection.clone();
        projection.push(MemoryProjection::Index(Box::new(index)));
        Ok(Value::Reference {
            allocation: *allocation,
            projection,
            mutable,
        })
    }

    pub(super) fn loop_slice_length(&self, source: &Value) -> Result<Value, String> {
        match source {
            Value::Bytes { length, .. } => Ok((**length).clone()),
            Value::Elements(fields) => Ok(self.iterator_index(fields.len() as u128)),
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
            | Value::Function
            | Value::Unit => {
                Err("inductive slice iterator needs byte or scalar element storage".into())
            }
        }
    }

    pub(super) fn loop_slice_call(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        state: &mut State,
        system: &mut System,
        premise: &Atom,
    ) -> Result<Option<Vec<(State, Value)>>, String> {
        let Some(operation) = self.loop_slice_operation(instance)? else {
            return Ok(None);
        };
        let receiver = values.first().ok_or("slice receiver missing")?;
        if matches!(
            operation,
            SliceOperation::New { .. } | SliceOperation::Identity | SliceOperation::ByRef
        ) {
            let value = self
                .loop_slice_template(instance, values, state)?
                .ok_or("slice operation has no result template")?;
            self.record_model(
                instance.def_id(),
                "typed slice iterator construction or identity",
            );
            return Ok(Some(vec![(state.clone(), value)]));
        }
        let iterator = self.loop_template_snapshot(receiver, state)?;
        let Value::SliceIterator {
            source,
            front,
            back,
            mutable,
        } = &iterator
        else {
            return Err("inductive slice iterator cursor missing".into());
        };
        self.loop_reference_bounds(state, source, system, premise)?;
        let storage = self.loop_template_snapshot(source, state)?;
        let length = self.loop_slice_length(&storage)?;
        let ordered =
            symbolic::binary(&self.terms, "le", (**front).clone(), (**back).clone())?.boolean()?;
        let in_storage =
            symbolic::binary(&self.terms, "le", (**back).clone(), length)?.boolean()?;
        let safe = self.terms.apply(Op::And, &[ordered, in_storage])?;
        exclude_failure(system, premise, &state.conditions, &safe);
        state.conditions.push(safe);
        let remaining = symbolic::binary(&self.terms, "sub", (**back).clone(), (**front).clone())?;
        let output = self
            .tcx
            .fn_sig(instance.def_id())
            .instantiate(self.tcx, instance.args)
            .skip_binder()
            .output();
        self.record_model(
            instance.def_id(),
            "typed slice cursor: front <= back <= length; indexed items",
        );
        let result = match operation {
            SliceOperation::Next { reverse, skip } => {
                let skip = if skip {
                    values.get(1).ok_or("slice skip missing")?.clone()
                } else {
                    self.iterator_index(0)
                };
                let inside =
                    symbolic::binary(&self.terms, "lt", skip.clone(), remaining)?.boolean()?;
                let index = if reverse {
                    symbolic::binary(
                        &self.terms,
                        "sub",
                        symbolic::binary(&self.terms, "sub", (**back).clone(), skip)?,
                        self.iterator_index(1),
                    )?
                } else {
                    symbolic::binary(&self.terms, "add", (**front).clone(), skip)?
                };
                let item = self.loop_slice_item(source, index.clone(), *mutable)?;
                let mut some = state.clone();
                some.conditions.push(inside.clone());
                let next = Value::SliceIterator {
                    source: source.clone(),
                    mutable: *mutable,
                    front: if reverse {
                        front.clone()
                    } else {
                        Box::new(symbolic::binary(
                            &self.terms,
                            "add",
                            index.clone(),
                            self.iterator_index(1),
                        )?)
                    },
                    back: if reverse {
                        Box::new(index)
                    } else {
                        back.clone()
                    },
                };
                self.store_iterator(receiver, next, &mut some.memory, &some.conditions)?;
                let mut none = state.clone();
                none.conditions.push(symbolic::not(&inside));
                let end = if reverse { front.clone() } else { back.clone() };
                self.store_iterator(
                    receiver,
                    Value::SliceIterator {
                        source: source.clone(),
                        front: end.clone(),
                        back: end,
                        mutable: *mutable,
                    },
                    &mut none.memory,
                    &none.conditions,
                )?;
                return Ok(Some(vec![
                    (some, self.constructed(output, 1, vec![item])?),
                    (none, self.constructed(output, 0, Vec::new())?),
                ]));
            }
            SliceOperation::Length => remaining,
            SliceOperation::Count => {
                if matches!(receiver, Value::Reference { mutable: true, .. }) {
                    self.store_iterator(
                        receiver,
                        Value::SliceIterator {
                            source: source.clone(),
                            front: back.clone(),
                            back: back.clone(),
                            mutable: *mutable,
                        },
                        &mut state.memory,
                        &state.conditions,
                    )?;
                }
                remaining
            }
            SliceOperation::Clone => iterator,
            SliceOperation::SizeHint => {
                let ty::Tuple(fields) = output.kind() else {
                    return Err("slice size hint type".into());
                };
                Value::Tuple(vec![
                    remaining.clone(),
                    self.constructed(fields[1], 1, vec![remaining])?,
                ])
            }
            SliceOperation::New { .. } | SliceOperation::Identity | SliceOperation::ByRef => {
                return Err("unexpected slice constructor branch".into());
            }
        };
        Ok(Some(vec![(state.clone(), result)]))
    }
}
