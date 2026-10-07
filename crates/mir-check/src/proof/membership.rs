use super::*;
use rustc_span::Symbol;

const MAX_MEMBERSHIP_ELEMENTS: usize = 128;

impl<'tcx> Engine<'tcx> {
    fn core_slice_contains_trait(&self) -> Option<DefId> {
        let iterator = self.tcx.get_diagnostic_item(Symbol::intern("SliceIter"))?;
        let slice = self.tcx.parent(self.tcx.parent(iterator));
        let cmp = self.tcx.module_children(slice).iter().find_map(|child| {
            let id = child.res.opt_def_id()?;
            (child.ident.name == Symbol::intern("cmp")
                && id.krate == iterator.krate
                && self.tcx.def_kind(id) == DefKind::Mod)
                .then_some(id)
        })?;
        self.tcx.module_children(cmp).iter().find_map(|child| {
            let id = child.res.opt_def_id()?;
            (child.ident.name == Symbol::intern("SliceContains")
                && id.krate == iterator.krate
                && self.tcx.def_kind(id) == DefKind::Trait)
                .then_some(id)
        })
    }

    pub(super) fn slice_membership(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        raw_values: &[Value],
        state: &State,
        stack: &[DefId],
        site: (DefId, Span),
    ) -> Result<Option<Vec<Return>>, String> {
        let callee = instance.def_id();
        if !self
            .tcx
            .lang_items()
            .get(LangItem::SliceLen)
            .is_some_and(|id| id.krate == callee.krate)
        {
            return Ok(None);
        }
        let parent = self.tcx.parent(callee);
        let name = self.tcx.item_name(callee);
        let slice_first = name == Symbol::intern("contains")
            && matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
            && matches!(
                self.tcx
                    .type_of(parent)
                    .instantiate(self.tcx, instance.args)
                    .skip_norm_wip()
                    .kind(),
                ty::Slice(_)
            );
        let element_first = name == Symbol::intern("slice_contains")
            && matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: true })
            && self.core_slice_contains_trait().is_some_and(|id| {
                self.tcx
                    .impl_trait_ref(parent)
                    .instantiate(self.tcx, instance.args)
                    .skip_norm_wip()
                    .def_id
                    == id
            });
        if !slice_first && !element_first {
            return Ok(None);
        }
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx.fn_sig(callee).instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("membership signature normalization failed: {error:?}"))?
            .skip_binder();
        if !signature.output().is_bool() {
            return Ok(None);
        }
        let [first, second] = signature.inputs() else {
            return Ok(None);
        };
        let (slice_ty, needle_ty) = if slice_first {
            (*first, *second)
        } else {
            (*second, *first)
        };
        let (ty::Ref(_, slice, slice_mut), ty::Ref(_, element, needle_mut)) =
            (slice_ty.kind(), needle_ty.kind())
        else {
            return Ok(None);
        };
        if slice_mut.is_mut()
            || needle_mut.is_mut()
            || !matches!(slice.kind(), ty::Slice(item) if item == element)
        {
            return Ok(None);
        }
        let [first, second] = values else {
            return Err("slice membership requires two modeled arguments".to_owned());
        };
        let (slice, needle) = if slice_first {
            (first, second)
        } else {
            (second, first)
        };
        let slice = slice.materialize()?;
        if !(element.is_bool()
            || self.integer_type(*element).is_some()
            || self.float_type(*element).is_some())
        {
            let Value::Elements(elements) = &slice else {
                return Err("custom membership needs fixed modeled element storage".to_owned());
            };
            if elements.len() > MAX_MEMBERSHIP_ELEMENTS {
                return Err("slice membership exceeds the 128-element model budget".to_owned());
            }
            let [first, second] = raw_values else {
                return Err("custom membership requires two tracked arguments".to_owned());
            };
            let (slice, needle) = if slice_first {
                (first, second)
            } else {
                (second, first)
            };
            return self
                .custom_membership(
                    instance,
                    (*element, elements.len()),
                    (slice, needle),
                    state,
                    stack,
                    site,
                )
                .map(Some);
        }
        let mut member = self.terms.boolean(false);
        match &slice {
            Value::Elements(elements) => {
                if elements.len() > MAX_MEMBERSHIP_ELEMENTS {
                    return Err("slice membership exceeds the 128-element model budget".to_owned());
                }
                for item in elements {
                    let equal = symbolic::binary(&self.terms, "eq", item.clone(), needle.clone())?
                        .boolean()?;
                    member = self.terms.apply(Op::Or, &[member, equal])?;
                }
            }
            Value::Bytes { length, data } if *element == self.tcx.types.u8 => {
                let (length_term, bits, _) = length.integer()?;
                let limit = self
                    .terms
                    .bit_vector(MAX_MEMBERSHIP_ELEMENTS as u128, bits)?;
                let outside = self
                    .terms
                    .apply(Op::BvUnsignedGt, &[length_term.clone(), limit])?;
                if self.feasible(&[state.conditions.clone(), vec![outside]].concat())? {
                    return Err("byte membership needs a proven length at most 128".to_owned());
                }
                let count = match symbolic::constant(&length_term) {
                    Some(symbolic::Constant::BitVec { value, .. }) => value as usize,
                    _ => MAX_MEMBERSHIP_ELEMENTS,
                };
                for index in 0..count {
                    let index = self.terms.bit_vector(index as u128, bits)?;
                    let item = Value::Int {
                        expression: self
                            .terms
                            .apply(Op::Select, &[data.clone(), index.clone()])?,
                        bits: 8,
                        signed: false,
                    };
                    let equal =
                        symbolic::binary(&self.terms, "eq", item, needle.clone())?.boolean()?;
                    let inside = self
                        .terms
                        .apply(Op::BvUnsignedLt, &[index, length_term.clone()])?;
                    let equal = self.terms.apply(Op::And, &[inside, equal])?;
                    member = self.terms.apply(Op::Or, &[member, equal])?;
                }
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
            | Value::MetadataPointer(_)
            | Value::StaticText
            | Value::FormatArguments
            | Value::RawPointer { .. }
            | Value::StaticSlice { .. }
            | Value::StaticView { .. }
            | Value::Uninitialized
            | Value::Function
            | Value::Unit => {
                return Err("slice membership storage is not modeled".to_owned());
            }
        }
        self.record_model(
            callee,
            "bounded primitive slice membership; exact equality per element",
        );
        Ok(Some(vec![Return {
            value: Value::Bool(member),
            conditions: state.conditions.clone(),
            memory: state.memory.clone(),
        }]))
    }

    fn custom_membership(
        &mut self,
        instance: ty::Instance<'tcx>,
        shape: (Ty<'tcx>, usize),
        sources: (&Value, &Value),
        state: &State,
        stack: &[DefId],
        site: (DefId, Span),
    ) -> Result<Vec<Return>, String> {
        let (element, count) = shape;
        let (slice, needle) = sources;
        let method = self
            .tcx
            .get_diagnostic_item(Symbol::intern("cmp_partialeq_eq"))
            .ok_or("compiler PartialEq method identity is unavailable")?;
        let args = self.tcx.mk_args(&[element.into(), element.into()]);
        let equality = self.resolve_function_item(method, args)?;
        let iterator = Value::SliceIterator {
            source: Box::new(slice.clone()),
            front: Box::new(self.iterator_index(0)),
            back: Box::new(self.iterator_index(count as u128)),
            mutable: false,
        };
        let mut pending = vec![(iterator, state.conditions.clone(), state.memory.clone())];
        let mut returns = Vec::new();
        self.record_model(
            instance.def_id(),
            "fixed custom membership; ordered checked PartialEq calls and short-circuit effects",
        );
        while let Some((iterator, conditions, memory)) = pending.pop() {
            for iteration in
                self.iterator_step(iterator, self.iterator_index(0), false, conditions, memory)?
            {
                let Some(item) = iteration.item else {
                    returns.push(Return {
                        value: Value::Bool(self.terms.boolean(false)),
                        conditions: iteration.conditions,
                        memory: iteration.memory,
                    });
                    continue;
                };
                for result in self.call_instance(
                    equality,
                    vec![item, needle.clone()],
                    iteration.conditions,
                    iteration.memory,
                    stack,
                    site,
                )? {
                    let equal = result.value.boolean()?;
                    let mut found = result.conditions.clone();
                    found.push(equal.clone());
                    if self.feasible(&found)? {
                        returns.push(Return {
                            value: Value::Bool(self.terms.boolean(true)),
                            conditions: found,
                            memory: result.memory.clone(),
                        });
                    }
                    let mut absent = result.conditions;
                    absent.push(symbolic::not(&equal));
                    if self.feasible(&absent)? {
                        pending.push((iteration.iterator.clone(), absent, result.memory));
                    }
                }
            }
        }
        Ok(returns)
    }
}
