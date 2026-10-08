use super::*;

impl<'tcx> Engine<'tcx> {
    pub(super) fn thin_raw_pointer(&self, ty: Ty<'tcx>) -> bool {
        matches!(ty.kind(), ty::RawPtr(pointee, _) if
            pointee.is_sized(self.tcx, ty::TypingEnv::fully_monomorphized()))
    }

    pub(super) fn pointer_handle_transmute(
        &self,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
        value: &Value,
    ) -> Result<Option<Value>, String> {
        let width = u32::from(self.tcx.sess.target.pointer_width);
        if !self.thin_raw_pointer(source) {
            return Ok(None);
        }
        if self
            .integer_type(target)
            .is_some_and(|(bits, _)| bits == width)
        {
            return self
                .pointer_handle_cast(
                    CastKind::PointerExposeProvenance,
                    source,
                    target,
                    value.clone(),
                )
                .map(Some);
        }
        let ty::Adt(def, args) = target.kind() else {
            return Ok(None);
        };
        if self
            .tcx
            .get_diagnostic_item(rustc_span::Symbol::intern("Atomic"))
            != Some(def.did())
            || args.type_at(0) != source
        {
            return Ok(None);
        }
        let (Value::RawPointer { bits, .. } | Value::TrackedPointer { bits, .. }) = value else {
            return Err("pointer atomic construction needs an integer-derived handle".into());
        };
        if *bits != width {
            return Err("pointer atomic handle width mismatch".into());
        }
        self.pointer_atomic_storage(target, source, value, def.did().krate, 0)
            .map(Some)
    }

    fn pointer_atomic_storage(
        &self,
        target: Ty<'tcx>,
        pointer: Ty<'tcx>,
        value: &Value,
        core: rustc_span::def_id::CrateNum,
        depth: usize,
    ) -> Result<Value, String> {
        if target == pointer {
            return Ok(value.clone());
        }
        if depth >= 4 {
            return Err("pointer atomic wrapper nesting exceeds the model budget".into());
        }
        let wrapper = self.pointer_atomic_wrapper(target, core)?;
        let inner =
            self.pointer_atomic_storage(wrapper.field_ty, pointer, value, core, depth + 1)?;
        self.constructed(target, 0, vec![inner])
    }

    pub(super) fn pointer_handle_cast(
        &self,
        kind: CastKind,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
        value: Value,
    ) -> Result<Value, String> {
        let width = u32::from(self.tcx.sess.target.pointer_width);
        match kind {
            CastKind::PointerWithExposedProvenance if self.thin_raw_pointer(target) => {
                if self.integer_type(source).is_none() {
                    return Err("pointer handle construction needs an integer address".into());
                }
                let address = symbolic::cast(&self.terms, value, width, false)?
                    .integer()?
                    .0;
                Ok(Value::RawPointer {
                    address,
                    bits: width,
                })
            }
            CastKind::PointerExposeProvenance if self.thin_raw_pointer(source) => {
                let (Value::RawPointer { address, bits }
                | Value::TrackedPointer { address, bits, .. }) = value
                else {
                    return Err("pointer address exposure needs an integer-derived handle".into());
                };
                if bits != width {
                    return Err("pointer handle width does not match the target".into());
                }
                let (bits, signed) = self
                    .integer_type(target)
                    .ok_or("pointer address exposure requires an integer target")?;
                symbolic::cast(
                    &self.terms,
                    Value::Int {
                        expression: address,
                        bits: width,
                        signed: false,
                    },
                    bits,
                    signed,
                )
            }
            CastKind::PtrToPtr
                if self.thin_raw_pointer(source) && self.thin_raw_pointer(target) =>
            {
                if !matches!(
                    value,
                    Value::RawPointer { bits, .. } | Value::TrackedPointer { bits, .. }
                        if bits == width
                ) {
                    return Err("pointer cast needs an integer-derived thin handle".into());
                }
                Ok(value)
            }
            _ => Err("unsupported pointer handle cast or metadata-bearing pointer".into()),
        }
    }
}
