use super::*;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    pub(super) fn pointer_atomic_compare_exchange(
        &mut self,
        instance: ty::Instance<'tcx>,
        receiver_ty: Ty<'tcx>,
        values: &[Value],
        state: &mut State,
        site: (DefId, Span),
    ) -> Result<Option<Value>, String> {
        let callee = instance.def_id();
        let Some(atomic) = self.tcx.get_diagnostic_item(Symbol::intern("Atomic")) else {
            return Ok(None);
        };
        let ty::Adt(definition, arguments) = receiver_ty.kind() else {
            return Ok(None);
        };
        let name = self.tcx.item_name(callee);
        if definition.did() != atomic
            || callee.krate != atomic.krate
            || !matches!(name.as_str(), "compare_exchange" | "compare_exchange_weak")
        {
            return Ok(None);
        }
        let pointer_ty = arguments.type_at(0);
        if !self.thin_raw_pointer(pointer_ty) {
            return Ok(None);
        }
        let signature = self.call_signature(instance)?;
        let [
            reference_ty,
            expected_ty,
            replacement_ty,
            success_ty,
            failure_ty,
        ] = signature.inputs()
        else {
            return Err("pointer compare-exchange signature mismatch".into());
        };
        if *expected_ty != pointer_ty
            || *replacement_ty != pointer_ty
            || !matches!(reference_ty.kind(), ty::Ref(_, element, mutable)
                if *element == receiver_ty && !mutable.is_mut())
        {
            return Err("pointer compare-exchange requires its exact primitive signature".into());
        }
        let [receiver, expected, replacement, success, failure] = values else {
            return Err("pointer compare-exchange needs receiver, pointers and orderings".into());
        };
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
            "pointer compare-exchange requires valid success and non-release failure orderings"
                .into(),
        )?;
        state.conditions.push(safe);
        self.validate_pointer_atomic_receiver(receiver_ty, pointer_ty, receiver, state)?;
        let bits = u32::from(self.tcx.sess.target.pointer_width);
        for pointer in [expected, replacement] {
            self.validate_tracked_value(pointer, state)?;
            if !matches!(pointer,
                Value::RawPointer { address, bits: width }
                | Value::TrackedPointer { address, bits: width, .. }
                if *width == bits && *address.sort() == Sort::BitVec(bits))
            {
                return Err(
                    "pointer compare-exchange needs initialized thin address handles".into(),
                );
            }
        }
        let result_ty = signature.output();
        let ty::Adt(def, args) = result_ty.kind() else {
            return Err("pointer compare-exchange result is not Result".into());
        };
        let ok = self
            .tcx
            .lang_items()
            .get(LangItem::ResultOk)
            .ok_or("Result::Ok compiler identity unavailable")?;
        if self.tcx.parent(ok) != def.did()
            || args.type_at(0) != pointer_ty
            || args.type_at(1) != pointer_ty
            || def.variants().len() != 2
            || def.variants()[rustc_abi::VariantIdx::from_usize(0)].def_id != ok
        {
            return Err("pointer compare-exchange result type mismatch".into());
        }
        let old = Value::RawPointer {
            address: self.fresh_abstraction(
                Sort::BitVec(bits),
                "pointer compare-exchange observes arbitrary address bits without pointee \
                 provenance",
            ),
            bits,
        };
        let equal =
            symbolic::binary(&self.terms, "eq", old.clone(), expected.clone())?.boolean()?;
        let weak = name.as_str() == "compare_exchange_weak";
        let succeeds = if weak {
            let not_spurious = self
                .fresh_abstraction(Sort::Bool, "weak compare-exchange permits spurious failure");
            self.terms.apply(Op::And, &[equal, not_spurious])?
        } else {
            equal
        };
        let variants = vec![
            self.constructed(result_ty, 0, vec![old.clone()])?,
            self.constructed(result_ty, 1, vec![old])?,
        ];
        let (tag_bits, tag_signed) = self
            .integer_type(result_ty.discriminant_ty(self.tcx))
            .ok_or("pointer compare-exchange Result discriminant is not an integer")?;
        let tags = variants
            .iter()
            .map(|value| {
                let Value::Adt { discriminant, .. } = value else {
                    return Err("pointer compare-exchange variant construction failed".into());
                };
                self.terms.bit_vector(*discriminant, tag_bits)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let discriminant = Value::Int {
            expression: self
                .terms
                .apply(Op::Ite, &[succeeds, tags[0].clone(), tags[1].clone()])?,
            bits: tag_bits,
            signed: tag_signed,
        };
        // A possible pointer publication ends the fresh-startup history premise.
        let mut memory = state.memory.clone();
        Self::invalidate_published_atomics(state, &mut memory)?;
        state.memory = memory;
        // Address equality establishes neither pointee provenance nor exclusive atomic history.
        self.record_model(callee, if weak {
            "pointer weak CAS; matching success, possible spurious failure, arbitrary old address"
        } else {
            "pointer strong CAS; exact old-address comparison without provenance or history"
        });
        Ok(Some(Value::Enum {
            discriminant: Box::new(discriminant),
            variants,
            is_option: false,
        }))
    }
}
