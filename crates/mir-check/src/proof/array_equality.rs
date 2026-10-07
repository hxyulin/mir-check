use super::*;
use rustc_span::Symbol;

const MAX_EQUALITY_ELEMENTS: usize = 128;

impl<'tcx> Engine<'tcx> {
    fn core_array_equality_trait(&self) -> Option<DefId> {
        let iterator = self
            .tcx
            .get_diagnostic_item(Symbol::intern("ArrayIntoIter"))?;
        let array = self.tcx.parent(self.tcx.parent(iterator));
        let equality = self.tcx.module_children(array).iter().find_map(|child| {
            let id = child.res.opt_def_id()?;
            (child.ident.name == Symbol::intern("equality")
                && id.krate == iterator.krate
                && self.tcx.def_kind(id) == DefKind::Mod)
                .then_some(id)
        })?;
        self.tcx.module_children(equality).iter().find_map(|child| {
            let id = child.res.opt_def_id()?;
            (child.ident.name == Symbol::intern("SpecArrayEq")
                && id.krate == iterator.krate
                && self.tcx.def_kind(id) == DefKind::Trait)
                .then_some(id)
        })
    }

    pub(super) fn array_equality(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        raw_values: &[Value],
        state: &State,
        stack: &[DefId],
        site: (DefId, Span),
    ) -> Result<Option<Vec<Return>>, String> {
        let callee = instance.def_id();
        let parent = self.tcx.parent(callee);
        let trait_id = match self.tcx.def_kind(parent) {
            DefKind::Impl { of_trait: true } => {
                self.tcx
                    .impl_trait_ref(parent)
                    .instantiate(self.tcx, instance.args)
                    .skip_norm_wip()
                    .def_id
            }
            DefKind::Trait => parent,
            _ => return Ok(None),
        };
        let name = self.tcx.item_name(callee);
        let partial_eq = self
            .tcx
            .get_diagnostic_item(Symbol::intern("cmp_partialeq_eq"))
            .map(|id| self.tcx.parent(id));
        let ordinary = Some(trait_id) == partial_eq && matches!(name.as_str(), "eq" | "ne");
        let specialized = matches!(name.as_str(), "spec_eq" | "spec_ne")
            && Some(trait_id) == self.core_array_equality_trait();
        if !ordinary && !specialized {
            return Ok(None);
        }
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx.fn_sig(callee).instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("array equality signature normalization failed: {error:?}"))?
            .skip_binder();
        let [left, right] = signature.inputs() else {
            return Ok(None);
        };
        let (ty::Ref(_, left, left_mut), ty::Ref(_, right, right_mut)) =
            (left.kind(), right.kind())
        else {
            return Ok(None);
        };
        let (ty::Array(left_element, left_count), ty::Array(right_element, right_count)) =
            (left.kind(), right.kind())
        else {
            return Ok(None);
        };
        if left_mut.is_mut() || right_mut.is_mut() || !signature.output().is_bool() {
            return Ok(None);
        }
        let count = left_count
            .try_to_target_usize(self.tcx)
            .ok_or("array equality has an unknown length")?;
        if right_count.try_to_target_usize(self.tcx) != Some(count) {
            return Err("array equality requires matching fixed lengths".to_owned());
        }
        if count > MAX_EQUALITY_ELEMENTS as u64 {
            return Err("array equality exceeds the 128-element model budget".to_owned());
        }
        let [left, right] = values else {
            return Err("array equality requires two modeled arguments".to_owned());
        };
        let left_values = self.equality_elements(left, *left_element, count as usize, state)?;
        let right_values = self.equality_elements(right, *right_element, count as usize, state)?;
        let negate = matches!(name.as_str(), "ne" | "spec_ne");
        if left_element == right_element
            && (left_element.is_bool()
                || left_element.is_char()
                || self.integer_type(*left_element).is_some()
                || self.float_type(*left_element).is_some())
        {
            let mut equal = self.terms.boolean(true);
            for (left, right) in left_values.into_iter().zip(right_values) {
                let item = symbolic::binary(&self.terms, "eq", left, right)?.boolean()?;
                equal = self.terms.apply(Op::And, &[equal, item])?;
            }
            self.record_model(
                callee,
                "fixed primitive array equality; exact numeric element comparisons",
            );
            return Ok(Some(vec![Return {
                value: Value::Bool(if negate { symbolic::not(&equal) } else { equal }),
                conditions: state.conditions.clone(),
                memory: state.memory.clone(),
            }]));
        }
        self.custom_sequence_equality(
            instance,
            (*left_element, *right_element, count as usize, negate),
            raw_values,
            state,
            stack,
            site,
        )
        .map(Some)
    }

    fn equality_elements(
        &mut self,
        value: &Value,
        element: Ty<'tcx>,
        count: usize,
        state: &State,
    ) -> Result<Vec<Value>, String> {
        match value.materialize()? {
            Value::Elements(elements) if elements.len() == count => Ok(elements),
            Value::Bytes { length, data } if element == self.tcx.types.u8 => {
                let valid = symbolic::binary(
                    &self.terms,
                    "eq",
                    (*length).clone(),
                    self.iterator_index(count as u128),
                )?
                .boolean()?;
                if self
                    .feasible(&[state.conditions.clone(), vec![symbolic::not(&valid)]].concat())?
                {
                    return Err("array equality byte storage length is not established".to_owned());
                }
                (0..count)
                    .map(|index| {
                        Ok(Value::Int {
                            expression: self.terms.apply(
                                Op::Select,
                                &[
                                    data.clone(),
                                    self.iterator_index(index as u128).integer()?.0,
                                ],
                            )?,
                            bits: 8,
                            signed: false,
                        })
                    })
                    .collect()
            }
            Value::Input(_)
            | Value::Bool(_)
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
            | Value::Uninitialized
            | Value::Function
            | Value::Unit => {
                Err("array equality requires matching fixed modeled storage".to_owned())
            }
        }
    }

    pub(super) fn custom_sequence_equality(
        &mut self,
        instance: ty::Instance<'tcx>,
        shape: (Ty<'tcx>, Ty<'tcx>, usize, bool),
        sources: &[Value],
        state: &State,
        stack: &[DefId],
        site: (DefId, Span),
    ) -> Result<Vec<Return>, String> {
        let (left_element, right_element, count, negate) = shape;
        let [left, right] = sources else {
            return Err("custom sequence equality requires two tracked arguments".to_owned());
        };
        // Pinned core's generic slice comparator invokes ne, including user overrides.
        let method = self
            .tcx
            .get_diagnostic_item(Symbol::intern("cmp_partialeq_ne"))
            .ok_or("compiler PartialEq inequality method identity is unavailable")?;
        let args = self
            .tcx
            .mk_args(&[left_element.into(), right_element.into()]);
        let inequality = self.resolve_function_item(method, args)?;
        let iterator = |source: &Value| Value::SliceIterator {
            source: Box::new(source.clone()),
            front: Box::new(self.iterator_index(0)),
            back: Box::new(self.iterator_index(count as u128)),
            mutable: false,
        };
        let mut pending = vec![(
            iterator(left),
            iterator(right),
            state.conditions.clone(),
            state.memory.clone(),
        )];
        let mut returns = Vec::new();
        self.record_model(
            instance.def_id(),
            "fixed custom sequence equality; ordered checked ne calls and short-circuit effects",
        );
        while let Some((left, right, conditions, memory)) = pending.pop() {
            for left in
                self.iterator_step(left, self.iterator_index(0), false, conditions, memory)?
            {
                for right in self.iterator_step(
                    right.clone(),
                    self.iterator_index(0),
                    false,
                    left.conditions.clone(),
                    left.memory.clone(),
                )? {
                    let (left_item, right_item) = match (&left.item, &right.item) {
                        (None, None) => {
                            returns.push(Return {
                                value: Value::Bool(self.terms.boolean(!negate)),
                                conditions: right.conditions,
                                memory: right.memory,
                            });
                            continue;
                        }
                        (Some(left), Some(right)) => (left.clone(), right.clone()),
                        _ => {
                            return Err("array equality cursors have mismatched lengths".to_owned());
                        }
                    };
                    for result in self.call_instance(
                        inequality,
                        vec![left_item, right_item],
                        right.conditions,
                        right.memory,
                        stack,
                        site,
                    )? {
                        let different = result.value.boolean()?;
                        let mismatch =
                            [result.conditions.clone(), vec![different.clone()]].concat();
                        if self.feasible(&mismatch)? {
                            returns.push(Return {
                                value: Value::Bool(self.terms.boolean(negate)),
                                conditions: mismatch,
                                memory: result.memory.clone(),
                            });
                        }
                        let same = [result.conditions, vec![symbolic::not(&different)]].concat();
                        if self.feasible(&same)? {
                            pending.push((
                                left.iterator.clone(),
                                right.iterator.clone(),
                                same,
                                result.memory,
                            ));
                        }
                    }
                }
            }
        }
        Ok(returns)
    }
}
