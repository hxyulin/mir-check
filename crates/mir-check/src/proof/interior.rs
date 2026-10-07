use super::*;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    pub(super) fn check_fence_ordering(
        &mut self,
        instance: ty::Instance<'tcx>,
        signature: ty::FnSig<'tcx>,
        values: &[Value],
        state: &mut State,
        site: (DefId, Span),
    ) -> Result<(), String> {
        let callee = instance.def_id();
        if !["fence", "compiler_fence"]
            .iter()
            .any(|name| self.tcx.get_diagnostic_item(Symbol::intern(name)) == Some(callee))
        {
            return Ok(());
        }
        let ([ordering_ty], [order]) = (signature.inputs(), values) else {
            return Err("atomic fence wrapper needs one ordering argument".into());
        };
        if !signature.output().is_unit() {
            return Err("atomic fence wrapper output type mismatch".into());
        }
        let safe = self.atomic_order(
            order,
            *ordering_ty,
            &["Acquire", "Release", "AcqRel", "SeqCst"],
        )?;
        self.require(
            site.0,
            site.1,
            &state.conditions,
            &safe,
            ObligationKind::PanicSafety,
            "atomic fence requires a non-Relaxed ordering".into(),
        )?;
        state.conditions.push(safe);
        self.record_model(
            callee,
            "fence ordering check; actual wrapper MIR executes on nonpanicking paths",
        );
        Ok(())
    }

    pub(super) fn check_pointer_store_ordering(
        &mut self,
        instance: ty::Instance<'tcx>,
        signature: ty::FnSig<'tcx>,
        values: &[Value],
        state: &mut State,
        site: (DefId, Span),
    ) -> Result<(), String> {
        let callee = instance.def_id();
        let parent = self.tcx.parent(callee);
        let atomic = self.tcx.get_diagnostic_item(Symbol::intern("Atomic"));
        if atomic.is_none_or(|atomic| callee.krate != atomic.krate)
            || self.tcx.item_name(callee) != Symbol::intern("store")
            || !matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
        {
            return Ok(());
        }
        let receiver = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx
                    .type_of(parent)
                    .instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("atomic store receiver normalization failed: {error:?}"))?;
        let ty::Adt(def, args) = receiver.kind() else {
            return Ok(());
        };
        if atomic != Some(def.did()) || !self.thin_raw_pointer(args.type_at(0)) {
            return Ok(());
        }
        let ([reference, value_ty, ordering_ty], [_, _, ordering]) = (signature.inputs(), values)
        else {
            return Err("pointer atomic store wrapper signature mismatch".into());
        };
        if !signature.output().is_unit()
            || *value_ty != args.type_at(0)
            || !matches!(reference.kind(), ty::Ref(_, element, mutability)
                if *element == receiver && !mutability.is_mut())
        {
            return Err(
                "pointer atomic store wrapper requires its exact primitive signature".into(),
            );
        }
        let safe = self.atomic_order(ordering, *ordering_ty, &["Relaxed", "Release", "SeqCst"])?;
        self.require(
            site.0,
            site.1,
            &state.conditions,
            &safe,
            ObligationKind::PanicSafety,
            "pointer atomic store requires a valid non-acquire ordering".into(),
        )?;
        state.conditions.push(safe);
        self.record_model(
            callee,
            "pointer atomic store ordering checked; actual wrapper MIR executes",
        );
        Ok(())
    }

    pub(super) fn atomic_fence_intrinsic(
        &mut self,
        instance: ty::Instance<'tcx>,
        signature: ty::FnSig<'tcx>,
        values: &[Value],
        span: Span,
    ) -> Result<Option<Value>, String> {
        let callee = instance.def_id();
        if !["atomic_fence", "atomic_singlethreadfence"]
            .iter()
            .any(|name| self.tcx.is_intrinsic(callee, Symbol::intern(name)))
        {
            return Ok(None);
        }
        if !signature.inputs().is_empty()
            || !signature.output().is_unit()
            || !values.is_empty()
            || instance.args.len() != 1
        {
            return Err("atomic fence intrinsic signature mismatch".into());
        }
        self.validate_atomic_intrinsic_order(
            instance,
            0,
            &["Acquire", "Release", "AcqRel", "SeqCst"],
            span,
        )?;
        self.record_model(
            callee,
            "atomic fence; validated ordering, no synchronization facts in arbitrary-access model",
        );
        Ok(Some(Value::Unit))
    }

    pub(super) fn atomic_shape(&self, ty: Ty<'tcx>) -> Option<Value> {
        let ty::Adt(def, args) = ty.kind() else {
            return None;
        };
        if self.tcx.get_diagnostic_item(Symbol::intern("Atomic")) != Some(def.did()) {
            return None;
        }
        let (bits, signed) = self.integer_type(args.type_at(0))?;
        Some(Value::Atomic { bits, signed })
    }

    pub(super) fn atomic_container(&self, ty: Ty<'tcx>, depth: usize) -> Option<Value> {
        if depth >= 8 {
            return None;
        }
        if let Some(value) = self.atomic_shape(ty) {
            return Some(value);
        }
        let ty::Adt(def, args) = ty.kind() else {
            return None;
        };
        if !def.is_struct() || def.non_enum_variant().fields.is_empty() {
            return None;
        }
        let fields = def
            .non_enum_variant()
            .fields
            .iter()
            .map(|field| self.atomic_container(field.ty(self.tcx, args).skip_norm_wip(), depth + 1))
            .collect::<Option<Vec<_>>>()?;
        self.constructed(ty, 0, fields).ok()
    }

    pub(super) fn cell_element(&self, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
        let ty::Adt(def, args) = ty.kind() else {
            return None;
        };
        if self.tcx.get_diagnostic_item(Symbol::intern("Cell")) != Some(def.did()) {
            return None;
        }
        let element = args.type_at(0);
        if element.is_bool()
            || self.integer_type(element).is_some()
            || self.float_type(element).is_some()
        {
            Some(element)
        } else {
            None
        }
    }

    pub(super) fn interior_call(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        state: &mut State,
        site: (DefId, Span),
    ) -> Result<Option<Value>, String> {
        let callee = instance.def_id();
        let parent = self.tcx.parent(callee);
        if !matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false }) {
            return Ok(None);
        }
        let receiver_ty = self
            .tcx
            .type_of(parent)
            .instantiate(self.tcx, instance.args)
            .skip_norm_wip();
        let name = self.tcx.item_name(callee);
        if let Some(value) = self.static_cell_get(receiver_ty, name.as_str(), values, state)? {
            self.record_model(callee, "UnsafeCell static storage address; no payload read");
            return Ok(Some(value));
        }
        if let Some(value) =
            self.static_uninit_pointer(receiver_ty, name.as_str(), values, state)?
        {
            self.record_model(
                callee,
                "MaybeUninit static payload address; initialization is not assumed",
            );
            return Ok(Some(value));
        }
        if let Some(element) = self.cell_element(receiver_ty) {
            let result = match (name.as_str(), values) {
                ("new", [value]) => {
                    if state.memory.len() >= 512 {
                        return Err("memory allocation budget reached".to_owned());
                    }
                    let allocation = state.memory.len();
                    state.memory.push(Some(value.clone()));
                    Value::Cell { allocation }
                }
                ("get", [Value::Cell { allocation }]) => state
                    .memory
                    .get(*allocation)
                    .and_then(Option::as_ref)
                    .cloned()
                    .ok_or("Cell storage is unavailable")?,
                ("set" | "replace", [Value::Cell { allocation }, value]) => {
                    let old = state
                        .memory
                        .get(*allocation)
                        .and_then(Option::as_ref)
                        .cloned()
                        .ok_or("Cell storage is unavailable")?;
                    state.memory[*allocation] = Some(value.clone());
                    if name.as_str() == "replace" {
                        old
                    } else {
                        Value::Unit
                    }
                }
                _ => return Ok(None),
            };
            if element.needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized()) {
                return Err("Cell model cannot execute destructors".to_owned());
            }
            self.record_model(callee, "scalar Cell storage; aliases share updates");
            return Ok(Some(result));
        }
        let Some(Value::Atomic { bits, signed }) = self.atomic_shape(receiver_ty) else {
            return Ok(None);
        };
        if name.as_str() == "new" {
            let [value] = values else {
                return Err("integer atomic constructor needs one value".into());
            };
            let (_, actual_bits, actual_signed) = value.integer()?;
            if (actual_bits, actual_signed) != (bits, signed) {
                return Err("atomic constructor value type mismatch".into());
            }
            if state.memory.len() >= 512 {
                return Err("memory allocation budget reached".into());
            }
            let allocation = state.memory.len();
            state.memory.push(Some(value.clone()));
            self.record_model(
                callee,
                "local integer atomic; allocation-backed constructor state",
            );
            return Ok(Some(Value::LocalAtomic {
                allocation,
                bits,
                signed,
            }));
        }
        if !matches!(
            name.as_str(),
            "load"
                | "store"
                | "fetch_add"
                | "fetch_sub"
                | "swap"
                | "compare_exchange"
                | "compare_exchange_weak"
        ) {
            return Ok(None);
        }
        let receiver = values.first().ok_or("atomic receiver is unavailable")?;
        let (old, allocation) = self.atomic_access(receiver, state, bits, signed)?;
        let order = values.last().ok_or("missing atomic ordering")?;
        let signature = self.call_signature(instance)?;
        if matches!(name.as_str(), "compare_exchange" | "compare_exchange_weak") {
            return self
                .atomic_compare_exchange(
                    instance,
                    signature,
                    values,
                    (old, allocation),
                    state,
                    site,
                )
                .map(Some);
        }
        let ordering_ty = signature
            .inputs()
            .last()
            .copied()
            .ok_or("atomic ordering argument type is unavailable")?;
        let allowed = match name.as_str() {
            "load" => &["Relaxed", "Acquire", "SeqCst"][..],
            "store" => &["Relaxed", "Release", "SeqCst"][..],
            _ => &["Relaxed", "Acquire", "Release", "AcqRel", "SeqCst"][..],
        };
        let safe = self.atomic_order(order, ordering_ty, allowed)?;
        self.require(
            site.0,
            site.1,
            &state.conditions,
            &safe,
            ObligationKind::PanicSafety,
            format!("{} requires a valid atomic ordering", name.as_str()),
        )?;
        state.conditions.push(safe);
        self.record_model(
            callee,
            if allocation.is_some() {
                "local integer atomic; exact allocation-backed history and modular RMW"
            } else {
                "integer atomic; arbitrary access state after possible interference"
            },
        );
        let replacement = match name.as_str() {
            "load" => None,
            "store" | "swap" => {
                let [_, value, _] = values else {
                    return Err("atomic write needs receiver, value and ordering".into());
                };
                let (_, actual_bits, actual_signed) = value.integer()?;
                if (actual_bits, actual_signed) != (bits, signed) {
                    return Err("atomic write value type mismatch".into());
                }
                Some(value.clone())
            }
            "fetch_add" | "fetch_sub" => {
                let [_, value, _] = values else {
                    return Err("atomic RMW needs receiver, value and ordering".into());
                };
                let operation = if name.as_str() == "fetch_add" {
                    "add"
                } else {
                    "sub"
                };
                Some(symbolic::binary(
                    &self.terms,
                    operation,
                    old.clone(),
                    value.clone(),
                )?)
            }
            _ => return Err("unsupported integer atomic operation".into()),
        };
        if let (Some(allocation), Some(replacement)) = (allocation, replacement) {
            self.store_atomic(state, allocation, replacement);
        }
        Ok(Some(if name.as_str() == "store" {
            Value::Unit
        } else {
            old
        }))
    }

    fn atomic_access(
        &mut self,
        receiver: &Value,
        state: &mut State,
        bits: u32,
        signed: bool,
    ) -> Result<(Value, Option<AtomicStorage>), String> {
        if self.startup && matches!(receiver, Value::StaticView { .. }) {
            return self.startup_atomic_access(receiver, state, bits, signed);
        }
        let (actual_bits, actual_signed, storage) = match receiver {
            Value::Atomic { bits, signed } => (*bits, *signed, None),
            Value::LocalAtomic {
                allocation,
                bits,
                signed,
            } => (*bits, *signed, Some(*allocation)),
            _ => return Err("atomic receiver is not modeled".into()),
        };
        if (actual_bits, actual_signed) != (bits, signed) {
            return Err("atomic receiver type mismatch".into());
        }
        if let Some(allocation) = storage {
            match state.memory.get(allocation).and_then(Option::as_ref) {
                Some(
                    value @ Value::Int {
                        bits: width,
                        signed: sign,
                        ..
                    },
                ) if (*width, *sign) == (bits, signed) => {
                    return Ok((value.clone(), storage.map(AtomicStorage::Tracked)));
                }
                Some(Value::Atomic {
                    bits: width,
                    signed: sign,
                }) if (*width, *sign) == (bits, signed) => {}
                _ => return Err("local atomic backing is dead or has an invalid shape".into()),
            }
        }
        Ok((self.opaque_atomic_value(bits, signed), None))
    }

    fn atomic_compare_exchange(
        &mut self,
        instance: ty::Instance<'tcx>,
        signature: ty::FnSig<'tcx>,
        values: &[Value],
        access: (Value, Option<AtomicStorage>),
        state: &mut State,
        site: (DefId, Span),
    ) -> Result<Value, String> {
        let (old, allocation) = access;
        let [_, expected, replacement, success, failure] = values else {
            return Err("compare_exchange needs receiver, values and two orderings".into());
        };
        let [_, expected_ty, replacement_ty, success_ty, failure_ty] = signature.inputs() else {
            return Err("compare_exchange signature mismatch".into());
        };
        let (bits, signed) = self
            .integer_type(*expected_ty)
            .ok_or("compare_exchange needs primitive integer values")?;
        if expected_ty != replacement_ty {
            return Err("compare_exchange value types differ".into());
        }
        for value in [expected, replacement] {
            let (_, width, sign) = value.integer()?;
            if (width, sign) != (bits, signed) {
                return Err("compare_exchange modeled value type mismatch".into());
            }
        }
        let success_safe = self.atomic_order(
            success,
            *success_ty,
            &["Relaxed", "Acquire", "Release", "AcqRel", "SeqCst"],
        )?;
        let failure_safe =
            self.atomic_order(failure, *failure_ty, &["Relaxed", "Acquire", "SeqCst"])?;
        let safe = self.terms.apply(Op::And, &[success_safe, failure_safe])?;
        self.require(
            site.0,
            site.1,
            &state.conditions,
            &safe,
            ObligationKind::PanicSafety,
            "compare_exchange requires valid success and non-release failure orderings".into(),
        )?;
        state.conditions.push(safe);
        let result_ty = signature.output();
        let ty::Adt(def, args) = result_ty.kind() else {
            return Err("compare_exchange result is not an enum".into());
        };
        let Some(ok) = self.tcx.lang_items().get(LangItem::ResultOk) else {
            return Err("Result::Ok compiler identity unavailable".into());
        };
        if self.tcx.parent(ok) != def.did()
            || args.type_at(0) != *expected_ty
            || args.type_at(1) != *expected_ty
            || def.variants().len() != 2
        {
            return Err("compare_exchange result type mismatch".into());
        }
        let equal =
            symbolic::binary(&self.terms, "eq", old.clone(), expected.clone())?.boolean()?;
        let weak = self.tcx.item_name(instance.def_id()).as_str() == "compare_exchange_weak";
        let succeeds = if weak {
            let no_spurious_failure = self
                .fresh_abstraction(Sort::Bool, "weak compare-exchange permits spurious failure");
            self.terms.apply(Op::And, &[equal, no_spurious_failure])?
        } else {
            equal
        };
        if let Some(allocation) = allocation {
            let old_term = old.integer()?.0;
            let replacement_term = replacement.integer()?.0;
            self.store_atomic(
                state,
                allocation,
                Value::Int {
                    expression: self
                        .terms
                        .apply(Op::Ite, &[succeeds.clone(), replacement_term, old_term])?,
                    bits,
                    signed,
                },
            );
        }
        let variants = vec![
            self.constructed(result_ty, 0, vec![old.clone()])?,
            self.constructed(result_ty, 1, vec![old])?,
        ];
        let (tag_bits, tag_signed) = self
            .integer_type(result_ty.discriminant_ty(self.tcx))
            .ok_or("compare_exchange result discriminant is not an integer")?;
        let tags = variants
            .iter()
            .map(|value| {
                let Value::Adt { discriminant, .. } = value else {
                    unreachable!();
                };
                self.terms.bit_vector(*discriminant, tag_bits)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let discriminant = Value::Int {
            expression: self
                .terms
                .apply(Op::Ite, &[succeeds, tags[0].clone(), tags[1].clone()])?,
            bits: tag_bits,
            signed: tag_signed,
        };
        self.record_model(
            instance.def_id(),
            match (weak, allocation.is_some()) {
                (true, true) => {
                    "local integer weak CAS; retained history and possible spurious failure"
                }
                (false, true) => "local integer strong CAS; exact comparison and retained update",
                (true, false) => {
                    "integer weak CAS; matching success, spurious failure, arbitrary state"
                }
                (false, false) => "integer strong CAS; exact old-value comparison, arbitrary state",
            },
        );
        Ok(Value::Enum {
            discriminant: Box::new(discriminant),
            variants,
            is_option: false,
        })
    }

    fn atomic_order(
        &self,
        order: &Value,
        ordering_ty: Ty<'tcx>,
        allowed: &[&str],
    ) -> Result<Term, String> {
        let ordering = self
            .tcx
            .get_diagnostic_item(Symbol::intern("Ordering"))
            .ok_or("atomic Ordering identity unavailable")?;
        let ty::Adt(def, _) = ordering_ty.kind() else {
            return Err("atomic ordering argument is not an enum".to_owned());
        };
        if def.did() != ordering || !def.is_enum() {
            return Err("atomic ordering argument type mismatch".to_owned());
        }
        let allowed = allowed
            .iter()
            .map(|name| Symbol::intern(name))
            .collect::<Vec<_>>();
        let allowed_tags = def
            .variants()
            .iter_enumerated()
            .filter(|(_, variant)| allowed.contains(&variant.name))
            .map(|(index, _)| {
                (
                    index.as_usize(),
                    def.discriminant_for_variant(self.tcx, index).val,
                )
            })
            .collect::<Vec<_>>();
        match order {
            Value::Adt {
                variant,
                discriminant,
                fields,
                ..
            } if def.variants().iter_enumerated().any(|(index, definition)| {
                index.as_usize() == *variant
                    && definition.fields.is_empty()
                    && fields.is_empty()
                    && def.discriminant_for_variant(self.tcx, index).val == *discriminant
            }) =>
            {
                Ok(self
                    .terms
                    .boolean(allowed_tags.iter().any(|(index, _)| index == variant)))
            }
            Value::Enum {
                discriminant,
                variants,
                ..
            } if variants.len() == def.variants().len()
                && def.variants().iter_enumerated().all(|(index, definition)| {
                    definition.fields.is_empty()
                        && matches!(&variants[index.as_usize()],
                                Value::Adt { variant, discriminant, fields, .. }
                                    if *variant == index.as_usize()
                                        && fields.is_empty()
                                        && *discriminant == def.discriminant_for_variant(
                                            self.tcx, index,
                                        ).val)
                }) =>
            {
                let (_, bits, signed) = discriminant.integer()?;
                let expressions = allowed_tags
                    .iter()
                    .map(|(_, tag)| {
                        symbolic::binary(
                            &self.terms,
                            "eq",
                            (**discriminant).clone(),
                            symbolic::integer(&self.terms, *tag, bits, signed),
                        )?
                        .boolean()
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                self.terms.apply(Op::Or, &expressions)
            }
            _ => Err("atomic ordering is not modeled".to_owned()),
        }
    }
}
