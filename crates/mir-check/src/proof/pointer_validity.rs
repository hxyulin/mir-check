use super::*;

impl<'tcx> Engine<'tcx> {
    pub(super) fn pointer_non_null_transmute(
        &mut self,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
        value: &Value,
        state: &mut State,
        site: (DefId, Span),
    ) -> Result<Option<Value>, String> {
        let non_null = |ty: Ty<'tcx>| match ty.kind() {
            ty::Adt(def, args)
                if self.tcx.lang_items().get(LangItem::NonNull) == Some(def.did()) =>
            {
                Some((*def, *args))
            }
            _ => None,
        };
        let (wrapper, pointer, wrapped) = if non_null(target).is_some() {
            (target, source, false)
        } else if non_null(source).is_some() {
            (source, target, true)
        } else {
            return Ok(None);
        };
        let Some((def, args)) = non_null(wrapper) else {
            return Ok(None);
        };
        let ty::RawPtr(pointee, _) = pointer.kind() else {
            return Ok(None);
        };
        if *pointee != args.type_at(0) || !self.thin_raw_pointer(pointer) {
            return Err("NonNull pointer handle requires the same sized pointee type".into());
        }
        let handle = if wrapped {
            let Value::Adt {
                name,
                variant: 0,
                discriminant: 0,
                is_option: false,
                fields,
            } = value
            else {
                return Ok(None);
            };
            let [(_, handle)] = fields.as_slice() else {
                return Err("NonNull pointer handle wrapper must have one field".into());
            };
            if *name != self.tcx.def_path_str(def.did()) {
                return Err("NonNull pointer handle wrapper identity mismatch".into());
            }
            handle
        } else {
            value
        };
        let (Value::RawPointer { address, bits } | Value::TrackedPointer { address, bits, .. }) =
            handle
        else {
            return Ok(None);
        };
        let width = u32::from(self.tcx.sess.target.pointer_width);
        if *bits != width || *address.sort() != Sort::BitVec(width) {
            return Err("NonNull pointer handle width or SMT sort mismatch".into());
        }
        let env = ty::TypingEnv::fully_monomorphized();
        let [field] = def.non_enum_variant().fields.raw.as_slice() else {
            return Err("NonNull compiler wrapper must have one pointer field".into());
        };
        let field_ty = self
            .tcx
            .try_normalize_erasing_regions(env, field.ty(self.tcx, args))
            .map_err(|error| format!("NonNull pointer field normalization failed: {error:?}"))?;
        let stored_pointer = match field_ty.kind() {
            ty::Pat(base, pattern) if matches!(**pattern, ty::PatternKind::NotNull) => *base,
            ty::RawPtr(..) => field_ty,
            _ => return Err("NonNull compiler field has an unsupported validity pattern".into()),
        };
        let wrapper_layout = self.static_layout(wrapper)?;
        let pointer_layout = self.static_layout(pointer)?;
        if !matches!(stored_pointer.kind(), ty::RawPtr(element, _) if *element == *pointee)
            || wrapper_layout.size != pointer_layout.size
            || wrapper_layout.align.abi != pointer_layout.align.abi
            || wrapper_layout.fields.offset(0).bytes() != 0
        {
            return Err("NonNull compiler wrapper does not preserve its pointer layout".into());
        }
        if wrapped
            && let Value::Adt { fields, .. } = value
            && fields[0].0 != field.name.as_str()
        {
            return Err("NonNull pointer handle field identity mismatch".into());
        }
        self.validate_tracked_value(handle, state)?;
        let zero = self.terms.bit_vector(0, width)?;
        let nonzero = symbolic::not(&self.terms.apply(Op::Equal, &[address.clone(), zero])?);
        self.require(
            site.0,
            site.1,
            &state.conditions,
            &nonzero,
            ObligationKind::Validity,
            "NonNull pointer handle must have a nonzero address".into(),
        )?;
        state.conditions.push(nonzero);
        if wrapped {
            Ok(Some(handle.clone()))
        } else {
            self.constructed(wrapper, 0, vec![handle.clone()]).map(Some)
        }
    }
}
