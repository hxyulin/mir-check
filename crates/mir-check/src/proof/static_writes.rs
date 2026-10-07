use super::*;

impl<'tcx> Engine<'tcx> {
    pub(super) fn retain_static_store_references(
        &self,
        value: &Value,
        state: &mut State,
    ) -> Result<(), String> {
        let allocation = self
            .static_roots
            .ok_or("static store escape roots unavailable")?;
        let Some(Some(Value::Elements(roots))) = state.memory.get_mut(allocation) else {
            return Err("static store escape roots invalidated by unknown memory effects".into());
        };
        Self::collect_static_store_references(value, roots)
    }

    fn collect_static_store_references(
        value: &Value,
        roots: &mut Vec<Value>,
    ) -> Result<(), String> {
        match value {
            Value::Reference { .. } => {
                if roots.len() >= 512 {
                    return Err("static store exceeds the 512-reference escape budget".into());
                }
                roots.push(value.clone());
            }
            Value::Adt { fields, .. } => {
                for (_, field) in fields {
                    Self::collect_static_store_references(field, roots)?;
                }
            }
            Value::Enum { variants, .. } | Value::Tuple(variants) | Value::Elements(variants) => {
                for variant in variants {
                    Self::collect_static_store_references(variant, roots)?;
                }
            }
            Value::Input(_)
            | Value::Bool(_)
            | Value::Int { .. }
            | Value::Float { .. }
            | Value::Bytes { .. }
            | Value::RawPointer { .. }
            | Value::StaticView { .. }
            | Value::StaticSlice { .. }
            | Value::Cell { .. }
            | Value::Atomic { .. }
            | Value::SliceIterator { .. }
            | Value::MetadataPointer(_)
            | Value::StaticText
            | Value::FormatArguments
            | Value::FunctionPointer { .. }
            | Value::Function
            | Value::Uninitialized
            | Value::Unit => {}
        }
        Ok(())
    }

    fn static_store_coroutine(
        &self,
        id: DefId,
        args: ty::GenericArgsRef<'tcx>,
        value: &Value,
        state: &State,
        depth: usize,
        values: &mut usize,
    ) -> Result<(), String> {
        let Value::Adt {
            name,
            variant: 0,
            discriminant: 0,
            fields,
            ..
        } = value
        else {
            return Err("static future stores require a freshly constructed coroutine".into());
        };
        let captures = args.as_coroutine().upvar_tys();
        let layout = self
            .tcx
            .coroutine_layout(id, args)
            .map_err(|error| format!("static future layout failed: {error:?}"))?;
        if *name != super::coroutines::coroutine_name(id, args)
            || fields.len() != captures.len() + layout.field_tys.len()
        {
            return Err("static future store identity or field shape mismatch".into());
        }
        for (index, (ty, (name, field))) in captures.iter().zip(fields).enumerate() {
            if *name != index.to_string() {
                return Err("static future capture identity mismatch".into());
            }
            self.static_store_value(ty, field, state, depth + 1, values)?;
        }
        for (index, (name, field)) in fields[captures.len()..].iter().enumerate() {
            if *name != format!("saved_{index}") || !matches!(field, Value::Uninitialized) {
                return Err("static future store needs uninitialized saved state".into());
            }
            *values += 1;
            if *values > 512 {
                return Err("static future store exceeds the value budget".into());
            }
        }
        Ok(())
    }

    pub(super) fn static_store_value(
        &self,
        ty: Ty<'tcx>,
        value: &Value,
        state: &State,
        depth: usize,
        values: &mut usize,
    ) -> Result<(), String> {
        if depth >= 16 || *values >= 512 {
            return Err("typed static store exceeds depth or value budget".into());
        }
        *values += 1;
        let env = ty::TypingEnv::fully_monomorphized();
        let value = value.materialize()?;
        if let ty::Coroutine(id, args) = ty.kind() {
            return self.static_store_coroutine(*id, args, &value, state, depth, values);
        }
        if !ty.is_freeze(self.tcx, env) {
            return Err("typed static stores require owned freeze values".into());
        }
        if let Some((bits, signed)) = self.integer_type(ty) {
            let (_, actual_bits, actual_signed) = value.integer()?;
            return if (bits, signed) == (actual_bits, actual_signed) {
                Ok(())
            } else {
                Err("typed static store integer shape mismatch".into())
            };
        }
        if let Some(bits) = self.float_type(ty) {
            return if matches!(value, Value::Float { bits: actual, .. } if bits == actual) {
                Ok(())
            } else {
                Err("typed static store float shape mismatch".into())
            };
        }
        match (ty.kind(), &value) {
            (ty::Bool, Value::Bool(_)) => Ok(()),
            (ty::RawPtr(_, _), Value::RawPointer { bits, .. })
                if self.thin_raw_pointer(ty)
                    && *bits == u32::from(self.tcx.sess.target.pointer_width) =>
            {
                Ok(())
            }
            (ty::Ref(_, _, mutability), Value::Reference { mutable, .. })
                if !mutability.is_mut() || *mutable =>
            {
                self.validate_tracked_value(&value, state)?;
                self.reference_value(&value, &state.memory, &state.conditions)?;
                Ok(())
            }
            (ty::Ref(..) | ty::RawPtr(..), Value::StaticView { .. }) => {
                self.static_stored_address(ty, &value, state)
            }
            (ty::FnPtr(..), Value::FunctionPointer { .. }) => {
                self.known_function_pointer(&value, ty).map(|_| ())
            }
            (ty::Tuple(types), Value::Unit) if types.is_empty() => Ok(()),
            (ty::Tuple(types), Value::Tuple(fields)) if types.len() == fields.len() => {
                for (ty, field) in types.iter().zip(fields) {
                    self.static_store_value(ty, field, state, depth + 1, values)?;
                }
                Ok(())
            }
            (ty::Array(element, count), Value::Elements(fields))
                if count.try_to_target_usize(self.tcx) == Some(fields.len() as u64) =>
            {
                for field in fields {
                    self.static_store_value(*element, field, state, depth + 1, values)?;
                }
                Ok(())
            }
            (ty::Array(element, count), Value::Bytes { length, .. })
                if *element == self.tcx.types.u8 =>
            {
                let expected = count
                    .try_to_target_usize(self.tcx)
                    .ok_or("typed byte store has an unknown array length")?;
                if matches!(symbolic::constant(&length.integer()?.0),
                    Some(symbolic::Constant::BitVec { value, .. }) if value == u128::from(expected))
                {
                    Ok(())
                } else {
                    Err("typed byte store needs an exactly sized owned array".into())
                }
            }
            (
                ty::Adt(def, args),
                Value::Adt {
                    name,
                    variant,
                    discriminant,
                    fields,
                    ..
                },
            ) if !def.is_union() && name == &self.tcx.def_path_str(def.did()) => {
                let index = rustc_abi::VariantIdx::from_usize(*variant);
                let variant = def
                    .variants()
                    .get(index)
                    .ok_or("typed static store has an invalid variant")?;
                if variant.fields.len() != fields.len()
                    || (def.is_struct() && *discriminant != 0)
                    || (def.is_enum()
                        && *discriminant != def.discriminant_for_variant(self.tcx, index).val)
                {
                    return Err("typed static store has an incompatible aggregate shape".into());
                }
                for (field, (name, value)) in variant.fields.iter().zip(fields) {
                    if field.name.as_str() != name {
                        return Err("typed static store field identity mismatch".into());
                    }
                    let ty = self
                        .tcx
                        .try_normalize_erasing_regions(env, field.ty(self.tcx, args))
                        .map_err(|error| {
                            format!("static store field normalization failed: {error:?}")
                        })?;
                    self.static_store_value(ty, value, state, depth + 1, values)?;
                }
                Ok(())
            }
            (ty::Adt(def, _), Value::Enum { variants, .. })
                if def.is_enum() && variants.len() == def.variants().len() =>
            {
                for variant in variants {
                    self.static_store_value(ty, variant, state, depth + 1, values)?;
                }
                Ok(())
            }
            _ => Err(format!("unsupported owned static store shape {ty}")),
        }
    }
}
