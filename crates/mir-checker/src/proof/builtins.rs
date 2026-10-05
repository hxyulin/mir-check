use super::*;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    pub(super) fn builtin(
        &mut self,
        caller: DefId,
        callee: DefId,
        args: ty::GenericArgsRef<'tcx>,
        values: &[Value],
        state: &mut State,
        span: Span,
    ) -> Result<Option<Value>, String> {
        if self.tcx.lang_items().get(LangItem::SliceLen) == Some(callee) {
            let [receiver] = values else {
                return Err("slice len receiver is not modeled".to_owned());
            };
            self.record_model(callee, "slice length");
            return match receiver {
                Value::Bytes { length, .. } | Value::MutableBytes { length, .. } => {
                    Ok(Some((**length).clone()))
                }
                Value::Elements(elements) => Ok(Some(symbolic::integer(
                    elements.len() as u128,
                    u32::from(self.tcx.sess.target.pointer_width),
                    false,
                ))),
                _ => Err("slice len receiver is not modeled".to_owned()),
            };
        }
        let signature = self
            .tcx
            .fn_sig(callee)
            .instantiate(self.tcx, args)
            .skip_binder();
        let parent = self.tcx.parent(callee);
        let trait_id = if self.tcx.def_kind(parent) == DefKind::Trait {
            Some(parent)
        } else if matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: true }) {
            Some(
                self.tcx
                    .impl_trait_ref(parent)
                    .instantiate(self.tcx, args)
                    .skip_norm_wip()
                    .def_id,
            )
        } else {
            None
        };
        let name = self.tcx.item_name(callee);
        if matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
            && matches!(
                self.tcx.type_of(parent).instantiate(self.tcx, args).skip_norm_wip().kind(),
                ty::Adt(def, _) if self.tcx.lang_items().get(LangItem::FormatArguments)
                    == Some(def.did())
            )
            && (name == Symbol::intern("from_str") || name == Symbol::intern("from_str_nonconst"))
            && signature.inputs().len() == 1
            && matches!(signature.inputs()[0].kind(), ty::Ref(_, element, mutability)
                if element.is_str() && !mutability.is_mut())
            && matches!(signature.output().kind(), ty::Adt(def, _)
                if self.tcx.lang_items().get(LangItem::FormatArguments) == Some(def.did()))
        {
            let [Value::StaticText] = values else {
                return Err("format model requires an evaluated static string".to_owned());
            };
            self.record_model(callee, "static formatting arguments; opaque panic payload");
            return Ok(Some(Value::FormatArguments));
        }
        let index = self
            .tcx
            .lang_items()
            .get(LangItem::Index)
            .is_some_and(|id| trait_id == Some(id))
            && name == Symbol::intern("index");
        let index_mut = self
            .tcx
            .lang_items()
            .get(LangItem::IndexMut)
            .is_some_and(|id| trait_id == Some(id))
            && name == Symbol::intern("index_mut");
        if (index || index_mut) && signature.inputs().len() == 2 {
            let ty::Ref(_, element, mutability) = signature.inputs()[0].kind() else {
                return Ok(None);
            };
            let byte_receiver = match element.kind() {
                ty::Slice(element) | ty::Array(element, _) => *element == self.tcx.types.u8,
                _ => false,
            };
            let ty::Adt(range, parameters) = signature.inputs()[1].kind() else {
                return Ok(None);
            };
            if !byte_receiver
                || mutability.is_mut() != index_mut
                || self.tcx.lang_items().get(LangItem::RangeTo) != Some(range.did())
                || parameters.type_at(0) != self.tcx.types.usize
            {
                return Ok(None);
            }
            self.record_model(callee, "byte prefix range; end <= receiver length");
            let [receiver, range] = values else {
                return Err("range call arity mismatch".to_owned());
            };
            let end = range.field("end")?;
            let length = match receiver {
                Value::Bytes { length, .. } | Value::MutableBytes { length, .. } => &**length,
                _ => return Err("range receiver is not modeled".to_owned()),
            };
            let safe = symbolic::binary("le", end.clone(), length.clone())?.boolean()?;
            self.require(
                caller,
                span,
                &state.conditions,
                &safe,
                ObligationKind::PanicSafety,
                "byte prefix end must not exceed its receiver length".to_owned(),
            )?;
            state.conditions.push(safe);
            let result = match (receiver, index_mut) {
                (Value::Bytes { data, .. }, false) => Value::Bytes {
                    length: Box::new(end),
                    data: data.clone(),
                },
                (Value::MutableBytes { owner, .. }, true) => Value::MutableBytes {
                    owner: *owner,
                    length: Box::new(end),
                },
                _ => return Err("range mutability mismatch".to_owned()),
            };
            return Ok(Some(result));
        }
        if self
            .tcx
            .lang_items()
            .get(LangItem::From)
            .is_some_and(|id| trait_id == Some(id))
            && name == Symbol::intern("from")
            && signature.inputs() == [self.tcx.types.u8]
            && signature.output() == self.tcx.types.usize
        {
            let [value] = values else {
                return Err("conversion arity mismatch".to_owned());
            };
            self.record_model(callee, "lossless u8 to usize");
            return Ok(Some(symbolic::cast(
                value.clone(),
                u32::from(self.tcx.sess.target.pointer_width),
                false,
            )?));
        }
        let core = self
            .tcx
            .lang_items()
            .get(LangItem::SliceLen)
            .map(|id| id.krate);
        let inherent_byte_slice =
            matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
                && matches!(
                    self.tcx.type_of(parent).instantiate(self.tcx, args).skip_norm_wip().kind(),
                    ty::Slice(element) if *element == self.tcx.types.u8
                );
        if Some(callee.krate) == core
            && name == Symbol::intern("copy_from_slice")
            && inherent_byte_slice
            && signature.inputs().len() == 2
        {
            self.record_model(
                callee,
                "byte copy; equal lengths and exact local-array updates",
            );
            return self
                .copy_bytes(caller, self.tcx.optimized_mir(caller), values, state, span)
                .map(Some);
        }
        Ok(None)
    }

    fn record_model(&mut self, callee: DefId, detail: &str) {
        let model = format!("{}: {detail}", self.tcx.def_path_str(callee));
        if !self.proof.models.contains(&model) {
            self.proof.models.push(model);
        }
    }

    fn copy_bytes(
        &mut self,
        caller: DefId,
        body: &Body<'tcx>,
        values: &[Value],
        state: &mut State,
        span: Span,
    ) -> Result<Value, String> {
        let [
            Value::MutableBytes { owner, length },
            Value::Bytes {
                length: source_len,
                data: source,
            },
        ] = values
        else {
            return Err(
                "copy model requires a local mutable byte array and an immutable byte source"
                    .to_owned(),
            );
        };
        let ty::Array(element, capacity) = body.local_decls
            [rustc_middle::mir::Local::from_usize(*owner)]
        .ty
        .kind() else {
            return Err("copy destination owner is not a local array".to_owned());
        };
        if *element != self.tcx.types.u8 {
            return Err("non-byte copy destination".to_owned());
        }
        let capacity = capacity
            .try_to_target_usize(self.tcx)
            .ok_or("unknown copy capacity")?;
        if capacity > 128 {
            return Err("byte array model size limit reached".to_owned());
        }
        let safe = symbolic::binary("eq", (**length).clone(), (**source_len).clone())?.boolean()?;
        self.require(
            caller,
            span,
            &state.conditions,
            &safe,
            ObligationKind::PanicSafety,
            "copy_from_slice requires equal source and destination lengths".to_owned(),
        )?;
        state.conditions.push(safe);
        let Value::Bytes {
            length: owner_len,
            data: old,
        } = state.locals[*owner].as_ref().ok_or("copy owner is dead")?
        else {
            return Err("copy destination storage is not modeled".to_owned());
        };
        let (len, bits, signed) = length.integer()?;
        if signed {
            return Err("signed copy length".to_owned());
        }
        let mut data = old.clone();
        for index in 0..capacity {
            let cell = format!("(_ bv{index} {bits})");
            let value = format!(
                "(ite (bvult {cell} {len}) (select {source} {cell}) (select {old} {cell}))"
            );
            data = format!("(store {data} {cell} {value})");
        }
        state.locals[*owner] = Some(Value::Bytes {
            length: owner_len.clone(),
            data,
        });
        Ok(Value::Unit)
    }
}
