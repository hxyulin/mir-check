use super::*;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    pub(super) fn loop_range_call(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        state: &State,
    ) -> Result<Option<Vec<(State, Value)>>, String> {
        let callee = instance.def_id();
        let Some(range_id) = self.tcx.lang_items().get(LangItem::Range) else {
            return Ok(None);
        };
        if callee.krate != range_id.krate
            || self.specification(instance)?.is_some()
            || !self.configured_contracts(instance)?.is_empty()
        {
            return Ok(None);
        }
        let parent = self.tcx.parent(callee);
        if !matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: true }) {
            return Ok(None);
        }
        let trait_id = self
            .tcx
            .impl_trait_ref(parent)
            .instantiate(self.tcx, instance.args)
            .skip_norm_wip()
            .def_id;
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx.fn_sig(callee).instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("range signature normalization failed: {error:?}"))?
            .skip_binder();
        let Some(input) = signature.inputs().first() else {
            return Ok(None);
        };
        let (receiver, borrowed) = match input.kind() {
            ty::Ref(_, pointee, mutability) if mutability.is_mut() => (*pointee, true),
            _ => (*input, false),
        };
        let ty::Adt(def, args) = receiver.kind() else {
            return Ok(None);
        };
        if def.did() != range_id || self.integer_type(args.type_at(0)).is_none() {
            return Ok(None);
        }
        let [value] = values else {
            return Err("inductive range call arity mismatch".into());
        };
        let name = self.tcx.item_name(callee);
        if name == Symbol::intern("into_iter")
            && !borrowed
            && Some(trait_id) == self.tcx.get_diagnostic_item(Symbol::intern("IntoIterator"))
            && signature.output() == receiver
        {
            self.record_model(callee, "integer Range identity into_iter");
            return Ok(Some(vec![(state.clone(), value.clone())]));
        }
        if name != Symbol::intern("next")
            || !borrowed
            || Some(trait_id) != self.tcx.get_diagnostic_item(Symbol::intern("Iterator"))
        {
            return Ok(None);
        }
        let ty::Adt(option, parameters) = signature.output().kind() else {
            return Ok(None);
        };
        if self.tcx.lang_items().get(LangItem::Option) != Some(option.did())
            || parameters.type_at(0) != args.type_at(0)
        {
            return Ok(None);
        }
        let range = self.reference_value(value, &state.memory, &state.conditions)?;
        let start = range.field("start")?;
        let end = range.field("end")?;
        let (_, bits, signed) = start.integer()?;
        let available = symbolic::binary(&self.terms, "lt", start.clone(), end)?.boolean()?;
        let mut some = state.clone();
        some.conditions.push(available.clone());
        let advanced = symbolic::binary(
            &self.terms,
            "add",
            start.clone(),
            symbolic::integer(&self.terms, 1, bits, signed),
        )?;
        let Value::Adt {
            mut fields,
            name,
            variant,
            is_option,
            discriminant,
        } = range
        else {
            return Err("inductive range storage has no fields".into());
        };
        let (field, _) = fields.get_mut(0).ok_or("range start field missing")?;
        if field != "start" {
            return Err("range start layout changed".into());
        }
        fields[0].1 = advanced;
        let Value::Reference {
            allocation,
            projection,
            mutable: true,
        } = value
        else {
            return Err("range next requires tracked mutable storage".into());
        };
        let conditions = some.conditions.clone();
        let storage = some
            .memory
            .get_mut(*allocation)
            .and_then(Option::as_mut)
            .ok_or("range next storage missing")?;
        self.write_projection(
            storage,
            projection,
            Value::Adt {
                fields,
                name,
                variant,
                is_option,
                discriminant,
            },
            &conditions,
        )?;
        let mut none = state.clone();
        none.conditions.push(symbolic::not(&available));
        self.record_model(
            callee,
            "integer Range next: start < end yields Some(start) and advances; otherwise None",
        );
        Ok(Some(vec![
            (some, self.constructed(signature.output(), 1, vec![start])?),
            (none, self.constructed(signature.output(), 0, Vec::new())?),
        ]))
    }
}
