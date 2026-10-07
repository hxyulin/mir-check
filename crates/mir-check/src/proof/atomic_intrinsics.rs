use super::*;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    pub(super) fn atomic_intrinsic_option(
        &self,
        instance: ty::Instance<'tcx>,
        index: usize,
        span: Span,
    ) -> Result<(Ty<'tcx>, Value), String> {
        let callee = instance.def_id();
        let parameter = self
            .tcx
            .generics_of(callee)
            .own_params
            .get(index)
            .ok_or("atomic intrinsic option parameter unavailable")?;
        if !matches!(parameter.kind, ty::GenericParamDefKind::Const { .. })
            || index >= instance.args.len()
        {
            return Err("atomic intrinsic option must be a compiler const parameter".into());
        }
        let ty = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx
                    .type_of(parameter.def_id)
                    .instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("atomic intrinsic option normalization failed: {error:?}"))?;
        let value = self.constant(
            callee,
            rustc_middle::mir::Const::Ty(ty, instance.args.const_at(index)),
            span,
        )?;
        Ok((ty, value))
    }

    pub(super) fn validate_atomic_intrinsic_order(
        &self,
        instance: ty::Instance<'tcx>,
        index: usize,
        allowed: &[&str],
        span: Span,
    ) -> Result<(), String> {
        let (ordering_ty, order) = self.atomic_intrinsic_option(instance, index, span)?;
        let ty::Adt(def, _) = ordering_ty.kind() else {
            return Err("atomic intrinsic ordering needs an enum".into());
        };
        let Value::Adt {
            variant,
            discriminant,
            fields,
            ..
        } = order
        else {
            return Err("atomic intrinsic ordering is not a concrete enum".into());
        };
        let Some((index, definition)) = def
            .variants()
            .iter_enumerated()
            .find(|(index, _)| index.as_usize() == variant)
        else {
            return Err("atomic intrinsic ordering variant is invalid".into());
        };
        if !def.is_enum()
            || !definition.fields.is_empty()
            || !fields.is_empty()
            || def.discriminant_for_variant(self.tcx, index).val != discriminant
            || !allowed.contains(&definition.name.as_str())
        {
            return Err("atomic intrinsic requires a supported ordering".into());
        }
        Ok(())
    }

    pub(super) fn atomic_store_intrinsic(
        &mut self,
        instance: ty::Instance<'tcx>,
        signature: ty::FnSig<'tcx>,
        values: &[Value],
        state: &mut State,
        span: Span,
    ) -> Result<Option<Value>, String> {
        let callee = instance.def_id();
        if !self
            .tcx
            .is_intrinsic(callee, Symbol::intern("atomic_store"))
        {
            return Ok(None);
        }
        let ([destination_ty, value_ty], [destination, value]) = (signature.inputs(), values)
        else {
            return Err("atomic store intrinsic needs a destination and value".into());
        };
        if !signature.output().is_unit()
            || instance.args.len() != 3
            || instance.args.type_at(0) != *value_ty
            || !matches!(destination_ty.kind(), ty::RawPtr(pointee, mutability)
                if *pointee == *value_ty && mutability.is_mut())
            || !(self.integer_type(*value_ty).is_some() || self.thin_raw_pointer(*value_ty))
        {
            return Err(
                "atomic store intrinsic needs a primitive integer or thin pointer value".into(),
            );
        }
        self.validate_atomic_intrinsic_order(instance, 1, &["Relaxed", "Release", "SeqCst"], span)?;
        let (volatile_ty, volatile) = self.atomic_intrinsic_option(instance, 2, span)?;
        if !volatile_ty.is_bool()
            || volatile.boolean()?.constant() != Some(mir_check::smt::Constant::Bool(false))
        {
            return Err("volatile atomic stores require a hardware memory model".into());
        }
        self.store_through_static_raw(destination, *value_ty, value, state)?;
        self.record_model(
            callee,
            "typed static atomic store; ordering and escape checks; payload stays opaque",
        );
        Ok(Some(Value::Unit))
    }
}
