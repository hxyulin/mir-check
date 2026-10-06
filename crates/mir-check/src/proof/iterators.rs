use super::*;
use rustc_span::Symbol;
use symbolic::MemoryProjection;

struct Iteration {
    iterator: Value,
    item: Option<Value>,
    conditions: Vec<String>,
    memory: Vec<Option<Value>>,
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
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx.fn_sig(callee).instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("iterator signature normalization failed: {error:?}"))?
            .skip_binder();
        let name = self.tcx.item_name(callee);
        let parent = self.tcx.parent(callee);
        let inherent = matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false });
        if inherent
            && signature.inputs().len() == 1
            && let Some(mutable) = self.slice_iterator_kind(signature.output())
            && let ty::Ref(_, slice, mutability) = signature.inputs()[0].kind()
            && matches!(slice.kind(), ty::Slice(_))
            && mutability.is_mut() == mutable
            && matches!(name.as_str(), "iter" | "iter_mut" | "new")
        {
            if mutable {
                return Err("mutable slice iteration is not modeled yet".to_owned());
            }
            let [source] = values else {
                return Err("iterator constructor arity mismatch".to_owned());
            };
            let snapshot = self.snapshot(source, &state.memory, &state.conditions, 0)?;
            let back = match snapshot {
                Value::Bytes { length, .. } => *length,
                Value::Elements(elements) => self.iterator_index(elements.len() as u128),
                _ => return Err("iterator needs modeled slice storage".to_owned()),
            };
            self.record_model(
                callee,
                "slice iterator; ordered elements and tracked cursor",
            );
            return Ok(Some(vec![Return {
                value: Value::SliceIterator {
                    source: Box::new(source.clone()),
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
        if self.slice_iterator_kind(receiver_ty).is_none() {
            return Ok(None);
        }
        let receiver = values.first().ok_or("iterator receiver is missing")?;
        let iterator = self.snapshot(receiver, &state.memory, &state.conditions, 0)?;
        let Value::SliceIterator { front, back, .. } = &iterator else {
            return Err("slice iterator state is not modeled".to_owned());
        };
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
            "next" | "nth" | "count" | "size_hint" | "all" | "any" => {
                trait_id.is_some() && trait_id == iterator_trait
            }
            "next_back" | "nth_back" => trait_id
                .is_some_and(|id| self.tcx.def_path_str(id) == "core::iter::DoubleEndedIterator"),
            "len" => trait_id
                .is_some_and(|id| self.tcx.def_path_str(id) == "core::iter::ExactSizeIterator"),
            "clone" => trait_id.is_some() && trait_id == clone_trait,
            _ => false,
        };
        if !supported {
            return Ok(None);
        }
        self.record_model(
            callee,
            "slice iterator; ordered elements and tracked cursor",
        );
        if matches!(name.as_str(), "all" | "any") {
            return self
                .iterator_predicate(instance, values, iterator, state, stack, site)
                .map(Some);
        }
        let remaining = symbolic::binary("sub", (**back).clone(), (**front).clone())?;
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

    fn iterator_index(&self, index: u128) -> Value {
        symbolic::integer(index, u32::from(self.tcx.sess.target.pointer_width), false)
    }

    fn store_iterator(
        &self,
        receiver: &Value,
        iterator: Value,
        memory: &mut [Option<Value>],
        conditions: &[String],
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

    fn iterator_step(
        &mut self,
        iterator: Value,
        skip: Value,
        reverse: bool,
        conditions: Vec<String>,
        memory: Vec<Option<Value>>,
    ) -> Result<Vec<Iteration>, String> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
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
        let remaining = symbolic::binary("sub", (*back).clone(), (*front).clone())?;
        let inside = symbolic::binary("lt", skip.clone(), remaining)?.boolean()?;
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
                    "sub",
                    symbolic::binary("sub", (*back).clone(), skip)?,
                    self.iterator_index(1),
                )?
            } else {
                symbolic::binary("add", (*front).clone(), skip)?
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
                let snapshot = self.snapshot(&source, &memory, &conditions, 0)?;
                match snapshot {
                    Value::Bytes { data, .. } => Value::Int {
                        expression: format!("(select {data} {})", index.integer()?.0),
                        bits: 8,
                        signed: false,
                    },
                    Value::Elements(elements) if !mutable => {
                        self.fixed_element(&elements, &index, &conditions)?
                    }
                    _ => return Err("iterator element storage is not modeled".to_owned()),
                }
            };
            let (front, back) = if reverse {
                (front, Box::new(index))
            } else {
                (
                    Box::new(symbolic::binary("add", index, self.iterator_index(1))?),
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

    fn iterator_predicate(
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
            .fn_sig(instance.def_id())
            .instantiate(self.tcx, instance.args)
            .skip_binder();
        let (callable, environment) = match signature.inputs()[1].kind() {
            ty::Closure(id, args) => (ty::Instance::new_raw(*id, args), true),
            ty::FnDef(id, args) => (ty::Instance::new_raw(*id, args.skip_binder()), false),
            _ => return Err("iterator predicate requires a concrete callable body".to_owned()),
        };
        let predicate = values.get(1).ok_or("iterator predicate is missing")?;
        let all = self.tcx.item_name(instance.def_id()) == Symbol::intern("all");
        let mut pending = vec![(iterator, state.conditions.clone(), state.memory.clone())];
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
                        value: Value::Bool(all.to_string()),
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
                    let keep = result.value.boolean()?;
                    let keep = if all { keep } else { symbolic::not(&keep) };
                    let stopped = [result.conditions.clone(), vec![symbolic::not(&keep)]].concat();
                    if self.feasible(&stopped)? {
                        returns.push(Return {
                            value: Value::Bool((!all).to_string()),
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
        Ok(returns)
    }
}
