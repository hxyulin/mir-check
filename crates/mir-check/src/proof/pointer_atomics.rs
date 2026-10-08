use super::*;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    pub(super) fn pointer_atomic_load(
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
        if definition.did() != atomic
            || callee.krate != atomic.krate
            || self.tcx.item_name(callee) != Symbol::intern("load")
        {
            return Ok(None);
        }
        let pointer_ty = arguments.type_at(0);
        if !self.thin_raw_pointer(pointer_ty) {
            return Ok(None);
        }
        let signature = self.call_signature(instance)?;
        let ([reference_ty, ordering_ty], [receiver, ordering]) = (signature.inputs(), values)
        else {
            return Err("pointer atomic load wrapper signature mismatch".into());
        };
        if signature.output() != pointer_ty
            || !matches!(reference_ty.kind(), ty::Ref(_, element, mutability)
                if *element == receiver_ty && !mutability.is_mut())
        {
            return Err("pointer atomic load requires its exact primitive signature".into());
        }
        let safe = self.atomic_order(ordering, *ordering_ty, &["Relaxed", "Acquire", "SeqCst"])?;
        self.require(
            site.0,
            site.1,
            &state.conditions,
            &safe,
            ObligationKind::PanicSafety,
            "pointer atomic load requires a valid non-release ordering".into(),
        )?;
        state.conditions.push(safe);
        if matches!(receiver, Value::StaticView { .. }) {
            self.static_atomic_storage(receiver, receiver_ty, pointer_ty, state)?;
        } else {
            self.validate_tracked_value(receiver, state)?;
            self.validate_pointer_atomic_value(receiver_ty, pointer_ty, receiver, state, 0)?;
        }
        self.record_model(
            callee,
            "pointer atomic load; arbitrary address bits without pointee provenance or history",
        );
        let bits = u32::from(self.tcx.sess.target.pointer_width);
        Ok(Some(Value::RawPointer {
            address: self.fresh_abstraction(
                Sort::BitVec(bits),
                "pointer atomic reads allow arbitrary addresses; pointee provenance and history \
                 are unverified",
            ),
            bits,
        }))
    }

    fn validate_pointer_atomic_value(
        &self,
        ty: Ty<'tcx>,
        pointer_ty: Ty<'tcx>,
        value: &Value,
        state: &State,
        depth: usize,
    ) -> Result<(), String> {
        if depth >= 8 {
            return Err("pointer atomic storage wrapper exceeds the validation budget".into());
        }
        if let Value::Reference { .. } = value {
            let value = self.reference_value(value, &state.memory, &state.conditions)?;
            return self.validate_pointer_atomic_value(ty, pointer_ty, &value, state, depth + 1);
        }
        if ty == pointer_ty {
            let width = u32::from(self.tcx.sess.target.pointer_width);
            return match value {
                Value::RawPointer { address, bits }
                | Value::TrackedPointer { address, bits, .. }
                    if *bits == width && *address.sort() == Sort::BitVec(width) =>
                {
                    Ok(())
                }
                Value::StaticView { .. } => self.static_stored_address(pointer_ty, value, state),
                _ => Err("pointer atomic storage needs an initialized thin pointer handle".into()),
            };
        }
        let Value::Adt {
            name,
            variant: 0,
            discriminant: 0,
            is_option: false,
            fields,
        } = value
        else {
            return Err(
                "pointer atomic storage wrapper is uninitialized or has invalid shape".into(),
            );
        };
        let atomic = self
            .tcx
            .get_diagnostic_item(Symbol::intern("Atomic"))
            .ok_or("pointer atomic storage compiler identity unavailable")?;
        let wrapper = self.pointer_atomic_wrapper(ty, atomic.krate)?;
        let [(field_name, field_value)] = fields.as_slice() else {
            return Err("pointer atomic storage wrapper requires one initialized field".into());
        };
        if *name != self.tcx.def_path_str(wrapper.definition)
            || field_name != wrapper.field_name.as_str()
        {
            return Err("pointer atomic storage wrapper identity or field mismatch".into());
        }
        self.validate_pointer_atomic_value(
            wrapper.field_ty,
            pointer_ty,
            field_value,
            state,
            depth + 1,
        )
    }
}
