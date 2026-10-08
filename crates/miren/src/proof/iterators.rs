use super::*;
use rustc_span::Symbol;
use symbolic::MemoryProjection;

pub(super) struct Iteration {
    pub(super) iterator: Value,
    pub(super) item: Option<Value>,
    pub(super) conditions: Vec<Term>,
    pub(super) memory: Memory,
}

impl<'tcx> Engine<'tcx> {
    fn slice_iterator_kind(&self, ty: Ty<'tcx>) -> Option<bool> {
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

    pub(super) fn iterator_call(
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
        let Some(shared) = self.tcx.get_diagnostic_item(Symbol::intern("SliceIter")) else {
            return Ok(None);
        };
        if callee.krate != shared.krate {
            return Ok(None);
        }
        let signature = self.call_signature(instance)?;
        let name = self.tcx.item_name(callee);
        let parent = self.tcx.parent(callee);
        let inherent = matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false });
        let borrowed_into_iter = name == Symbol::intern("into_iter")
            && matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: true })
            && self.tcx.get_diagnostic_item(Symbol::intern("IntoIterator"))
                == Some(
                    self.tcx
                        .impl_trait_ref(parent)
                        .instantiate(self.tcx, instance.args)
                        .skip_norm_wip()
                        .def_id,
                );
        if inherent
            && name == Symbol::intern("unwrap")
            && signature.inputs().len() == 1
            && matches!(signature.inputs()[0].kind(), ty::Adt(def, _)
                if self.tcx.lang_items().get(LangItem::Option) == Some(def.did()))
            && let [
                Value::Adt {
                    is_option: true,
                    variant: 1,
                    fields,
                    ..
                },
            ] = values
            && let [(_, value)] = fields.as_slice()
            && value.contains_mutable()
        {
            self.record_model(
                callee,
                "known Some payload; preserves tracked mutable references",
            );
            return Ok(Some(vec![Return {
                value: value.clone(),
                conditions: state.conditions.clone(),
                memory: state.memory.clone(),
            }]));
        }
        if (inherent || borrowed_into_iter)
            && signature.inputs().len() == 1
            && let Some(mutable) = self.slice_iterator_kind(signature.output())
            && let ty::Ref(_, slice, mutability) = signature.inputs()[0].kind()
            && matches!(slice.kind(), ty::Slice(_) | ty::Array(..))
            && mutability.is_mut() == mutable
            && (matches!(name.as_str(), "iter" | "iter_mut" | "new") || borrowed_into_iter)
        {
            let [source] = values else {
                return Err("iterator constructor arity mismatch".to_owned());
            };
            let snapshot = self.snapshot(source, &state.memory, &state.conditions, 0)?;
            let (source, back) = match snapshot {
                Value::Bytes { length, .. } => (source.clone(), *length),
                Value::Elements(elements) => {
                    (source.clone(), self.iterator_index(elements.len() as u128))
                }
                Value::StaticSlice { ref elements, .. } => {
                    let back = self.iterator_index(elements.len() as u128);
                    (snapshot, back)
                }
                Value::StaticView { .. } if !mutable => {
                    let elements = self.static_array_elements(&snapshot, state)?;
                    let Value::StaticSlice {
                        elements: items, ..
                    } = &elements
                    else {
                        unreachable!();
                    };
                    let back = self.iterator_index(items.len() as u128);
                    (elements, back)
                }
                _ => return Err("iterator needs modeled slice storage".to_owned()),
            };
            if mutable && !matches!(source, Value::Reference { mutable: true, .. }) {
                return Err("mutable iterator needs tracked writable slice storage".to_owned());
            }
            self.record_model(
                callee,
                "slice iterator; ordered elements and tracked cursor",
            );
            return Ok(Some(vec![Return {
                value: Value::SliceIterator {
                    source: Box::new(source),
                    front: Box::new(self.iterator_index(0)),
                    back: Box::new(back),
                    mutable,
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
        if let Some(results) = self.mutable_iterator_adapter(
            instance,
            values,
            receiver_ty,
            signature.output(),
            state,
            site,
        )? {
            return Ok(Some(results));
        }
        if self.slice_iterator_kind(receiver_ty).is_none() {
            return Ok(None);
        }
        let receiver = values.first().ok_or("iterator receiver is missing")?;
        let iterator = self.snapshot(receiver, &state.memory, &state.conditions, 0)?;
        let Value::SliceIterator {
            source,
            front,
            back,
            ..
        } = &iterator
        else {
            return Err("slice iterator state is not modeled".to_owned());
        };
        if matches!(source.as_ref(), Value::StaticSlice { .. }) {
            self.snapshot(source, &state.memory, &state.conditions, 0)?;
        }
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
        let iterator_trait = self.tcx.get_diagnostic_item(Symbol::intern("Iterator"));
        let clone_trait = self.tcx.lang_items().get(LangItem::Clone);
        let supported = match name.as_str() {
            "next" | "nth" | "count" | "size_hint" | "all" | "any" | "find_map" | "fold"
            | "by_ref" => trait_id.is_some() && trait_id == iterator_trait,
            "into_iter" => {
                trait_id.is_some()
                    && trait_id == self.tcx.get_diagnostic_item(Symbol::intern("IntoIterator"))
            }
            "next_back" | "nth_back" | "rfold" => trait_id.is_some_and(|id| {
                self.tcx.def_kind(id) == DefKind::Trait
                    && id.krate == shared.krate
                    && self.tcx.item_name(id) == Symbol::intern("DoubleEndedIterator")
            }),
            "len" => trait_id.is_some_and(|id| {
                self.tcx.def_kind(id) == DefKind::Trait
                    && id.krate == shared.krate
                    && self.tcx.item_name(id) == Symbol::intern("ExactSizeIterator")
            }),
            "clone" => trait_id.is_some() && trait_id == clone_trait,
            _ => false,
        };
        if !supported {
            return Ok(None);
        }
        if matches!(name.as_str(), "by_ref" | "into_iter") {
            if signature.inputs().len() != 1
                || signature.output() != signature.inputs()[0]
                || !matches!(signature.output().kind(), ty::Ref(_, _, mutability)
                    if mutability.is_mut())
                || !matches!(receiver, Value::Reference { mutable: true, .. })
            {
                return Ok(None);
            }
            self.record_model(
                callee,
                "slice iterator; preserves the writable cursor reference",
            );
            return Ok(Some(vec![Return {
                value: receiver.clone(),
                conditions: state.conditions.clone(),
                memory: state.memory.clone(),
            }]));
        }
        self.record_model(
            callee,
            "slice iterator; ordered elements and tracked cursor",
        );
        if matches!(name.as_str(), "all" | "any" | "find_map") {
            return self
                .iterator_predicate(instance, values, iterator, state, stack, site)
                .map(Some);
        }
        if matches!(name.as_str(), "fold" | "rfold") {
            return self
                .iterator_fold(instance, values, iterator, state, stack, site)
                .map(Some);
        }
        let remaining = symbolic::binary(&self.terms, "sub", (**back).clone(), (**front).clone())?;
        if name == Symbol::intern("count")
            && matches!(receiver, Value::Reference { mutable: true, .. })
        {
            let Value::SliceIterator {
                source,
                back,
                mutable,
                ..
            } = &iterator
            else {
                return Err("slice iterator state is not modeled".to_owned());
            };
            let exhausted = Value::SliceIterator {
                source: source.clone(),
                front: back.clone(),
                back: back.clone(),
                mutable: *mutable,
            };
            self.store_iterator(receiver, exhausted, &mut state.memory, &state.conditions)?;
        }
        let value = match name.as_str() {
            "len" | "count" => remaining,
            "size_hint" => {
                let ty::Tuple(fields) = signature.output().kind() else {
                    return Err("iterator size hint has unsupported type".to_owned());
                };
                Value::Tuple(vec![
                    remaining.clone(),
                    self.constructed(fields[1], 1, vec![remaining])?,
                ])
            }
            "clone" => iterator,
            "next" | "next_back" | "nth" | "nth_back" => {
                let skip = if matches!(name.as_str(), "nth" | "nth_back") {
                    values
                        .get(1)
                        .ok_or("iterator skip count is missing")?
                        .clone()
                } else {
                    self.iterator_index(0)
                };
                let reverse = matches!(name.as_str(), "next_back" | "nth_back");
                let iterations = self.iterator_step(
                    iterator,
                    skip,
                    reverse,
                    state.conditions.clone(),
                    state.memory.clone(),
                )?;
                let mut results = Vec::new();
                for iteration in iterations {
                    let mut memory = iteration.memory;
                    self.store_iterator(
                        receiver,
                        iteration.iterator,
                        &mut memory,
                        &iteration.conditions,
                    )?;
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
            _ => unreachable!("supported iterator operation"),
        };
        Ok(Some(vec![Return {
            value,
            conditions: state.conditions.clone(),
            memory: state.memory.clone(),
        }]))
    }

    fn mutable_iterator_adapter(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        receiver_ty: Ty<'tcx>,
        output: Ty<'tcx>,
        state: &State,
        site: (DefId, Span),
    ) -> Result<Option<Vec<Return>>, String> {
        let callee = instance.def_id();
        let parent = self.tcx.parent(callee);
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
        let name = self.tcx.item_name(callee);
        let enumerate = self.tcx.get_diagnostic_item(Symbol::intern("Enumerate"));
        let wrapped = match receiver_ty.kind() {
            ty::Adt(def, args) if Some(def.did()) == enumerate => {
                self.slice_iterator_kind(args.type_at(0)) == Some(true)
            }
            _ => false,
        };
        let mutable = self.slice_iterator_kind(receiver_ty) == Some(true);
        if !mutable && !wrapped {
            return Ok(None);
        }
        let receiver = values
            .first()
            .ok_or("mutable iterator receiver is missing")?;
        if name == Symbol::intern("into_iter")
            && trait_id.is_some()
            && trait_id == self.tcx.get_diagnostic_item(Symbol::intern("IntoIterator"))
            && output == receiver_ty
        {
            self.record_model(callee, "mutable slice iterator passthrough");
            return Ok(Some(vec![Return {
                value: receiver.clone(),
                conditions: state.conditions.clone(),
                memory: state.memory.clone(),
            }]));
        }
        if mutable
            && name == Symbol::intern("enumerate")
            && trait_id.is_some()
            && trait_id == self.tcx.get_diagnostic_item(Symbol::intern("Iterator"))
            && matches!(output.kind(), ty::Adt(def, _) if Some(def.did()) == enumerate)
        {
            self.record_model(
                callee,
                "mutable slice enumeration; tracked cursor and checked count",
            );
            return Ok(Some(vec![Return {
                value: self.constructed(
                    output,
                    0,
                    vec![receiver.clone(), self.iterator_index(0)],
                )?,
                conditions: state.conditions.clone(),
                memory: state.memory.clone(),
            }]));
        }
        if !wrapped
            || name != Symbol::intern("next")
            || trait_id.is_none()
            || trait_id != self.tcx.get_diagnostic_item(Symbol::intern("Iterator"))
        {
            return Ok(None);
        }
        let snapshot = self.snapshot(receiver, &state.memory, &state.conditions, 0)?;
        let Value::Adt { fields, .. } = snapshot else {
            return Err("mutable enumerate state is not modeled".to_owned());
        };
        let [(_, iterator), (_, count)] = fields.as_slice() else {
            return Err("mutable enumerate fields are not modeled".to_owned());
        };
        let Value::Reference {
            allocation,
            projection,
            mutable: true,
        } = receiver
        else {
            return Err("mutable enumerate needs writable cursor storage".to_owned());
        };
        self.record_model(
            callee,
            "mutable slice enumeration; tracked cursor and checked count",
        );
        let mut results = Vec::new();
        for iteration in self.iterator_step(
            iterator.clone(),
            self.iterator_index(0),
            false,
            state.conditions.clone(),
            state.memory.clone(),
        )? {
            let mut memory = iteration.memory;
            let mut conditions = iteration.conditions;
            let mut inner_projection = projection.clone();
            inner_projection.push(MemoryProjection::Field(0));
            self.store_iterator(
                &Value::Reference {
                    allocation: *allocation,
                    projection: inner_projection,
                    mutable: true,
                },
                iteration.iterator,
                &mut memory,
                &conditions,
            )?;
            let value = if let Some(item) = iteration.item {
                let Value::Tuple(sum) = symbolic::binary(
                    &self.terms,
                    "checked_add",
                    count.clone(),
                    self.iterator_index(1),
                )?
                else {
                    return Err("enumerate count is not modeled".to_owned());
                };
                if self.tcx.sess.overflow_checks() {
                    let safe = symbolic::not(&sum[1].boolean()?);
                    self.require(
                        site.0,
                        site.1,
                        &conditions,
                        &safe,
                        ObligationKind::PanicSafety,
                        "enumerate counter must not overflow".to_owned(),
                    )?;
                    conditions.push(safe);
                }
                let mut count_projection = projection.clone();
                count_projection.push(MemoryProjection::Field(1));
                let storage = memory
                    .get_mut(*allocation)
                    .and_then(Option::as_mut)
                    .ok_or("enumerate storage is dead")?;
                self.write_projection(storage, &count_projection, sum[0].clone(), &conditions)?;
                self.constructed(output, 1, vec![Value::Tuple(vec![count.clone(), item])])?
            } else {
                self.constructed(output, 0, vec![])?
            };
            results.push(Return {
                value,
                conditions,
                memory,
            });
        }
        Ok(Some(results))
    }

    pub(super) fn iterator_index(&self, index: u128) -> Value {
        symbolic::integer(
            &self.terms,
            index,
            u32::from(self.tcx.sess.target.pointer_width),
            false,
        )
    }

    pub(super) fn store_iterator(
        &self,
        receiver: &Value,
        iterator: Value,
        memory: &mut [Option<Value>],
        conditions: &[Term],
    ) -> Result<(), String> {
        let Value::Reference {
            allocation,
            projection,
            mutable: true,
        } = receiver
        else {
            return Err("iterator advance needs writable iterator storage".to_owned());
        };
        let storage = memory
            .get_mut(*allocation)
            .and_then(Option::as_mut)
            .ok_or("iterator storage is dead")?;
        self.write_projection(storage, projection, iterator, conditions)
    }

    pub(super) fn iterator_step(
        &mut self,
        iterator: Value,
        skip: Value,
        reverse: bool,
        conditions: Vec<Term>,
        memory: Memory,
    ) -> Result<Vec<Iteration>, String> {
        self.steps += 1;
        if self.steps > self.limits.max_steps {
            return Err("symbolic execution step limit reached".to_owned());
        }
        let Value::SliceIterator {
            source,
            front,
            back,
            mutable,
        } = iterator
        else {
            return Err("expected modeled slice iterator".to_owned());
        };
        if matches!(source.as_ref(), Value::StaticSlice { .. }) {
            self.snapshot(&source, &memory, &conditions, 0)?;
        }
        let remaining = symbolic::binary(&self.terms, "sub", (*back).clone(), (*front).clone())?;
        let inside = symbolic::binary(&self.terms, "lt", skip.clone(), remaining)?.boolean()?;
        let mut results = Vec::new();
        let end = [conditions.clone(), vec![symbolic::not(&inside)]].concat();
        if self.feasible(&end)? {
            results.push(Iteration {
                iterator: Value::SliceIterator {
                    source: source.clone(),
                    front: back.clone(),
                    back: back.clone(),
                    mutable,
                },
                item: None,
                conditions: end,
                memory: memory.clone(),
            });
        }
        let conditions = [conditions, vec![inside]].concat();
        if self.feasible(&conditions)? {
            let index = if reverse {
                symbolic::binary(
                    &self.terms,
                    "sub",
                    symbolic::binary(&self.terms, "sub", (*back).clone(), skip)?,
                    self.iterator_index(1),
                )?
            } else {
                symbolic::binary(&self.terms, "add", (*front).clone(), skip)?
            };
            let item = if let Value::Reference {
                allocation,
                projection,
                ..
            } = source.as_ref()
            {
                let mut projection = projection.clone();
                projection.push(MemoryProjection::Index(Box::new(index.clone())));
                Value::Reference {
                    allocation: *allocation,
                    projection,
                    mutable,
                }
            } else {
                let storage = match *source.clone() {
                    Value::Input(input) => input.materialize()?,
                    value => value,
                };
                match storage {
                    Value::Bytes { data, .. } => Value::Int {
                        expression: self
                            .terms
                            .apply(Op::Select, &[data.clone(), index.integer()?.0])?,
                        bits: 8,
                        signed: false,
                    },
                    Value::Elements(elements) | Value::StaticSlice { elements, .. } if !mutable => {
                        self.fixed_element(&elements, &index, &conditions)?
                    }
                    _ => return Err("iterator element storage is not modeled".to_owned()),
                }
            };
            let (front, back) = if reverse {
                (front, Box::new(index))
            } else {
                (
                    Box::new(symbolic::binary(
                        &self.terms,
                        "add",
                        index,
                        self.iterator_index(1),
                    )?),
                    back,
                )
            };
            results.push(Iteration {
                iterator: Value::SliceIterator {
                    source,
                    front,
                    back,
                    mutable,
                },
                item: Some(item),
                conditions,
                memory,
            });
        }
        Ok(results)
    }

    pub(super) fn iterator_predicate(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        iterator: Value,
        state: &State,
        stack: &[DefId],
        site: (DefId, Span),
    ) -> Result<Vec<Return>, String> {
        let signature = self.call_signature(instance)?;
        let (callable, environment) = match signature.inputs()[1].kind() {
            ty::Closure(id, args) => (ty::Instance::new_raw(*id, args), true),
            ty::FnDef(id, args) => (self.resolve_function_item(*id, args.skip_binder())?, false),
            _ => return Err("iterator predicate requires a concrete callable body".to_owned()),
        };
        let predicate = values.get(1).ok_or("iterator predicate is missing")?;
        if signature.inputs()[1].needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized()) {
            return Err("iterator predicate callback destructors are not modeled".to_owned());
        }
        let (predicate, memory) = if environment {
            self.callback_environment(predicate, state)?
        } else {
            (predicate.clone(), state.memory.clone())
        };
        let name = self.tcx.item_name(instance.def_id());
        let all = name == Symbol::intern("all");
        let find_map = name == Symbol::intern("find_map");
        let mut pending = vec![(iterator, state.conditions.clone(), memory)];
        let mut returns = Vec::new();
        while let Some((iterator, conditions, memory)) = pending.pop() {
            for iteration in
                self.iterator_step(iterator, self.iterator_index(0), false, conditions, memory)?
            {
                let mut memory = iteration.memory;
                self.store_iterator(
                    &values[0],
                    iteration.iterator.clone(),
                    &mut memory,
                    &iteration.conditions,
                )?;
                let Some(item) = iteration.item else {
                    returns.push(Return {
                        value: if find_map {
                            self.constructed(signature.output(), 0, vec![])?
                        } else {
                            Value::Bool(self.terms.boolean(all))
                        },
                        conditions: iteration.conditions,
                        memory,
                    });
                    continue;
                };
                let arguments = if environment {
                    vec![predicate.clone(), item]
                } else {
                    vec![item]
                };
                for result in self.call_instance(
                    callable,
                    arguments,
                    iteration.conditions,
                    memory,
                    stack,
                    site,
                )? {
                    if find_map {
                        let Value::Adt {
                            is_option: true,
                            variant,
                            ..
                        } = &result.value
                        else {
                            return Err("find_map callback needs a modeled Option result".into());
                        };
                        match variant {
                            0 => pending.push((
                                iteration.iterator.clone(),
                                result.conditions,
                                result.memory,
                            )),
                            1 => returns.push(result),
                            _ => return Err("find_map callback returned an invalid variant".into()),
                        }
                        continue;
                    }
                    let keep = result.value.boolean()?;
                    let keep = if all { keep } else { symbolic::not(&keep) };
                    let stopped = [result.conditions.clone(), vec![symbolic::not(&keep)]].concat();
                    if self.feasible(&stopped)? {
                        returns.push(Return {
                            value: Value::Bool(self.terms.boolean(!all)),
                            conditions: stopped,
                            memory: result.memory.clone(),
                        });
                    }
                    let continued = [result.conditions, vec![keep]].concat();
                    if self.feasible(&continued)? {
                        pending.push((iteration.iterator.clone(), continued, result.memory));
                    }
                }
            }
        }
        if environment {
            self.retire_callback_environment(&predicate, &mut returns);
        }
        Ok(returns)
    }
}
