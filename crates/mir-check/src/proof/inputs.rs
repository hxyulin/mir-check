use super::*;

impl<'tcx> Engine<'tcx> {
    pub(super) fn nonzero_get(
        &self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
    ) -> Result<Option<Value>, String> {
        if self.tcx.item_name(instance.def_id()) != rustc_span::Symbol::intern("get") {
            return Ok(None);
        }
        let signature = self
            .tcx
            .fn_sig(instance.def_id())
            .instantiate(self.tcx, instance.args)
            .skip_binder();
        let [receiver] = signature.inputs() else {
            return Ok(None);
        };
        let ty::Adt(def, _) = receiver.kind() else {
            return Ok(None);
        };
        if !self
            .tcx
            .is_diagnostic_item(rustc_span::Symbol::intern("NonZero"), def.did())
        {
            return Ok(None);
        }
        let Some((bits, signed)) = self.integer_type(signature.output()) else {
            return Ok(None);
        };
        let [Value::Adt { fields, .. }] = values else {
            return Err("nonzero getter receiver is not modeled".to_owned());
        };
        let [(_, Value::Adt { fields, .. })] = fields.as_slice() else {
            return Err("nonzero getter inner storage is not modeled".to_owned());
        };
        let [(_, value)] = fields.as_slice() else {
            return Err("nonzero getter scalar is not modeled".to_owned());
        };
        let (_, value_bits, value_signed) = value.integer()?;
        if value_bits != bits || value_signed != signed {
            return Err("nonzero getter scalar type mismatch".to_owned());
        }
        Ok(Some(value.clone()))
    }

    pub(super) fn input_pattern(
        &self,
        pattern: ty::Pattern<'tcx>,
        value: &Value,
        depth: usize,
    ) -> Result<Term, String> {
        if depth >= MAX_INPUT_DEPTH {
            return Err("input pattern nesting limit reached".to_owned());
        }
        let (_, bits, signed) = value.integer()?;
        match *pattern {
            ty::PatternKind::Range { start, end } => {
                let endpoint = |constant: ty::Const<'tcx>| {
                    constant
                        .try_to_leaf()
                        .filter(|scalar| scalar.size().bits() == u64::from(bits))
                        .map(|scalar| {
                            symbolic::integer(
                                &self.terms,
                                scalar.to_bits(scalar.size()),
                                bits,
                                signed,
                            )
                        })
                        .ok_or_else(|| "unevaluated or mismatched input pattern bound".to_owned())
                };
                let lower = symbolic::binary(&self.terms, "ge", value.clone(), endpoint(start)?)?
                    .boolean()?;
                let upper = symbolic::binary(&self.terms, "le", value.clone(), endpoint(end)?)?
                    .boolean()?;
                self.terms.apply(Op::And, &[lower, upper])
            }
            ty::PatternKind::Or(patterns) => {
                if patterns.is_empty() || patterns.len() > 64 {
                    return Err("input pattern needs between 1 and 64 alternatives".to_owned());
                }
                let alternatives = patterns
                    .iter()
                    .map(|pattern| self.input_pattern(pattern, value, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?;
                self.terms.apply(Op::Or, &alternatives)
            }
            ty::PatternKind::NotNull => {
                Err("non-null pointer input patterns remain unsupported".to_owned())
            }
        }
    }
}
