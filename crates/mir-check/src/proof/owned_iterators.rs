use super::*;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    fn owned_iterator_element(&self, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
        let ty::Adt(def, args) = ty.kind() else {
            return None;
        };
        let iterator = self
            .tcx
            .get_diagnostic_item(Symbol::intern("ArrayIntoIter"))?;
        (def.did() == iterator).then(|| args.type_at(0))
    }

    pub(super) fn is_owned_no_drop_iterator(&self, ty: Ty<'tcx>) -> bool {
        self.noop_owned_iterator_drop(ty, 0)
    }

    fn noop_owned_iterator_drop(&self, ty: Ty<'tcx>, depth: usize) -> bool {
        let environment = ty::TypingEnv::fully_monomorphized();
        if !ty.needs_drop(self.tcx, environment) {
            return true;
        }
        if depth >= 8 {
            return false;
        }
        if let Some(element) = self.owned_iterator_element(ty) {
            return !element.needs_drop(self.tcx, environment);
        }
        match ty.kind() {
            ty::Adt(def, args) if def.destructor(self.tcx).is_none() => {
                def.all_fields().all(|field| {
                    self.tcx
                        .try_normalize_erasing_regions(environment, field.ty(self.tcx, args))
                        .is_ok_and(|field| self.noop_owned_iterator_drop(field, depth + 1))
                })
            }
            ty::Tuple(fields) => fields
                .iter()
                .all(|field| self.noop_owned_iterator_drop(field, depth + 1)),
            ty::Array(element, _) => self.noop_owned_iterator_drop(*element, depth + 1),
            _ => false,
        }
    }

    pub(super) fn owned_iterator_call(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        state: &mut State,
        stack: &[DefId],
        site: (DefId, Span),
    ) -> Result<Option<Vec<Return>>, String> {
        let callee = instance.def_id();
        if self.tcx.def_kind(callee) == DefKind::Closure {
            return Ok(None);
        }
        let Some(iterator) = self
            .tcx
            .get_diagnostic_item(Symbol::intern("ArrayIntoIter"))
        else {
            return Ok(None);
        };
        if callee.krate != iterator.krate {
            return Ok(None);
        }
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx.fn_sig(callee).instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("owned iterator signature normalization failed: {error:?}"))?
            .skip_binder();
        let parent = self.tcx.parent(callee);
        let name = self.tcx.item_name(callee);
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
        let inherent = matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false });
        let into_iterator = self.tcx.get_diagnostic_item(Symbol::intern("IntoIterator"));
        if signature.inputs().len() == 1
            && self.owned_iterator_element(signature.output()).is_some()
            && let ty::Array(element, count) = signature.inputs()[0].kind()
            && ((name == Symbol::intern("into_iter")
                && trait_id.is_some()
                && trait_id == into_iterator)
                || (inherent && name == Symbol::intern("new")))
        {
            let count = count
                .try_to_target_usize(self.tcx)
                .ok_or("owned array iterator has unknown length")?;
            if count > 128 {
                return Err("owned array iterator exceeds 128-element limit".to_owned());
            }
            if element.needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized()) {
                return Err("owned array iterator element destructors are not modeled".to_owned());
            }
            let [source] = values else {
                return Err("owned array iterator constructor arity mismatch".to_owned());
            };
            source
                .owned_repeat_size()
                .filter(|size| *size <= symbolic::MAX_REPEAT_VALUES)
                .ok_or("owned array iterator requires owned values within a 256-value budget")?;
            let length = match source {
                Value::Bytes { length, .. } => (**length).clone(),
                Value::Elements(elements) if elements.len() as u64 == count => {
                    self.iterator_index(u128::from(count))
                }
                _ => return Err("owned array iterator requires modeled array values".to_owned()),
            };
            self.record_model(
                callee,
                "owned array iterator; no-drop values and ordered cursor",
            );
            return Ok(Some(vec![Return {
                value: Value::SliceIterator {
                    source: Box::new(source.clone()),
                    front: Box::new(self.iterator_index(0)),
                    back: Box::new(length),
                    mutable: false,
                },
                conditions: state.conditions.clone(),
                memory: state.memory.clone(),
            }]));
        }
        let Some(first) = signature.inputs().first() else {
            return Ok(None);
        };
        let receiver_ty = match first.kind() {
            ty::Ref(_, ty, _) => *ty,
            _ => *first,
        };
        let Some(element) = self.owned_iterator_element(receiver_ty) else {
            return Ok(None);
        };
        if element.needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized()) {
            return Err("owned array iterator element destructors are not modeled".to_owned());
        }
        let iterator_trait = self.tcx.get_diagnostic_item(Symbol::intern("Iterator"));
        let double_ended = trait_id.is_some_and(|id| {
            id.krate == iterator.krate
                && self.tcx.def_kind(id) == DefKind::Trait
                && self.tcx.item_name(id) == Symbol::intern("DoubleEndedIterator")
        });
        let exact_size = trait_id.is_some_and(|id| {
            id.krate == iterator.krate
                && self.tcx.def_kind(id) == DefKind::Trait
                && self.tcx.item_name(id) == Symbol::intern("ExactSizeIterator")
        });
        let supported = match name.as_str() {
            "next" | "nth" | "count" | "size_hint" | "all" | "any" | "fold" | "last" | "by_ref" => {
                trait_id.is_some() && trait_id == iterator_trait
            }
            "next_back" | "nth_back" | "rfold" => double_ended,
            "len" | "is_empty" => exact_size,
            "into_iter" => trait_id.is_some() && trait_id == into_iterator,
            _ => false,
        };
        if !supported {
            return Ok(None);
        }
        let receiver = values.first().ok_or("owned iterator receiver is missing")?;
        let cursor = self.snapshot(receiver, &state.memory, &state.conditions, 0)?;
        let Value::SliceIterator {
            source,
            front,
            back,
            mutable: false,
        } = &cursor
        else {
            return Err("owned array iterator cursor is not modeled".to_owned());
        };
        source
            .owned_repeat_size()
            .ok_or("owned array iterator cannot yield storage identities")?;
        self.record_model(
            callee,
            "owned array iterator; no-drop values and ordered cursor",
        );
        if matches!(name.as_str(), "all" | "any" | "fold" | "rfold") {
            let callback_index = if matches!(name.as_str(), "fold" | "rfold") {
                2
            } else {
                1
            };
            let callback_ty = signature.inputs()[callback_index];
            if callback_ty.needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized()) {
                return Err("owned iterator callback destructors are unmodeled".to_owned());
            }
            if matches!(name.as_str(), "all" | "any") {
                return self
                    .iterator_predicate(instance, values, cursor, state, stack, site)
                    .map(Some);
            }
            return self
                .iterator_fold(instance, values, cursor, state, stack, site)
                .map(Some);
        }
        let remaining = symbolic::binary("sub", (**back).clone(), (**front).clone())?;
        let value = match name.as_str() {
            "into_iter" => receiver.clone(),
            "by_ref" => {
                if signature.output() != *first {
                    return Err("iterator by_ref return type differs from receiver".to_owned());
                }
                receiver.clone()
            }
            "len" => remaining,
            "count" => {
                if matches!(receiver, Value::Reference { mutable: true, .. }) {
                    let mut memory = state.memory.clone();
                    self.store_iterator(
                        receiver,
                        Value::SliceIterator {
                            source: source.clone(),
                            front: back.clone(),
                            back: back.clone(),
                            mutable: false,
                        },
                        &mut memory,
                        &state.conditions,
                    )?;
                    return Ok(Some(vec![Return {
                        value: remaining,
                        conditions: state.conditions.clone(),
                        memory,
                    }]));
                }
                remaining
            }
            "is_empty" => symbolic::binary("eq", remaining, self.iterator_index(0))?,
            "size_hint" => {
                let ty::Tuple(fields) = signature.output().kind() else {
                    return Err("owned iterator size hint has unsupported type".to_owned());
                };
                Value::Tuple(vec![
                    remaining.clone(),
                    self.constructed(fields[1], 1, vec![remaining])?,
                ])
            }
            "next" | "next_back" | "nth" | "nth_back" | "last" => {
                let skip = if matches!(name.as_str(), "nth" | "nth_back") {
                    values
                        .get(1)
                        .ok_or("owned iterator skip count is missing")?
                        .clone()
                } else {
                    self.iterator_index(0)
                };
                let reverse = matches!(name.as_str(), "next_back" | "nth_back" | "last");
                let mut results = Vec::new();
                for iteration in self.iterator_step(
                    cursor.clone(),
                    skip,
                    reverse,
                    state.conditions.clone(),
                    state.memory.clone(),
                )? {
                    let mut memory = iteration.memory;
                    if name == Symbol::intern("last") {
                        if matches!(receiver, Value::Reference { mutable: true, .. }) {
                            self.store_iterator(
                                receiver,
                                Value::SliceIterator {
                                    source: source.clone(),
                                    front: back.clone(),
                                    back: back.clone(),
                                    mutable: false,
                                },
                                &mut memory,
                                &iteration.conditions,
                            )?;
                        }
                    } else {
                        self.store_iterator(
                            receiver,
                            iteration.iterator,
                            &mut memory,
                            &iteration.conditions,
                        )?;
                    }
                    results.push(Return {
                        value: self.constructed(
                            signature.output(),
                            usize::from(iteration.item.is_some()),
                            iteration.item.into_iter().collect(),
                        )?,
                        conditions: iteration.conditions,
                        memory,
                    });
                }
                return Ok(Some(results));
            }
            _ => unreachable!("supported owned iterator operation"),
        };
        Ok(Some(vec![Return {
            value,
            conditions: state.conditions.clone(),
            memory: state.memory.clone(),
        }]))
    }

    pub(super) fn iterator_fold(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        iterator: Value,
        state: &State,
        stack: &[DefId],
        site: (DefId, Span),
    ) -> Result<Vec<Return>, String> {
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx
                    .fn_sig(instance.def_id())
                    .instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("iterator fold signature normalization failed: {error:?}"))?
            .skip_binder();
        if values.len() != 3 || signature.inputs().len() != 3 {
            return Err("iterator fold callback arity mismatch".to_owned());
        }
        let callback_ty = signature.inputs()[2];
        if callback_ty.needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized()) {
            return Err("iterator fold callback destructors are unmodeled".to_owned());
        }
        let (callable, has_environment) = match callback_ty.kind() {
            ty::Closure(id, args) => (ty::Instance::new_raw(*id, args), true),
            ty::FnDef(id, args) => (ty::Instance::new_raw(*id, args.skip_binder()), false),
            _ => return Err("iterator fold requires a concrete callback body".to_owned()),
        };
        let (callback, memory) = if has_environment {
            self.callback_environment(&values[2], state)?
        } else {
            (values[2].clone(), state.memory.clone())
        };
        let reverse = self.tcx.item_name(instance.def_id()) == Symbol::intern("rfold");
        let mut pending = vec![(
            iterator,
            values[1].clone(),
            state.conditions.clone(),
            memory,
        )];
        let mut returns = Vec::new();
        while let Some((iterator, accumulator, conditions, memory)) = pending.pop() {
            for iteration in self.iterator_step(
                iterator,
                self.iterator_index(0),
                reverse,
                conditions,
                memory,
            )? {
                let mut memory = iteration.memory;
                if matches!(&values[0], Value::Reference { mutable: true, .. }) {
                    self.store_iterator(
                        &values[0],
                        iteration.iterator.clone(),
                        &mut memory,
                        &iteration.conditions,
                    )?;
                }
                let Some(item) = iteration.item else {
                    returns.push(Return {
                        value: accumulator.clone(),
                        conditions: iteration.conditions,
                        memory,
                    });
                    continue;
                };
                let mut arguments = vec![accumulator.clone(), item];
                if has_environment {
                    arguments.insert(0, callback.clone());
                }
                for result in self.call_instance(
                    callable,
                    arguments,
                    iteration.conditions,
                    memory,
                    stack,
                    site,
                )? {
                    pending.push((
                        iteration.iterator.clone(),
                        result.value,
                        result.conditions,
                        result.memory,
                    ));
                }
            }
        }
        if has_environment {
            self.retire_callback_environment(&callback, &mut returns);
        }
        Ok(returns)
    }
}
