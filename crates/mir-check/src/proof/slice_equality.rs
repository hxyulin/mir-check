use super::*;
use rustc_span::Symbol;

const MAX_SLICE_EQUALITY_ELEMENTS: usize = 128;

impl<'tcx> Engine<'tcx> {
    pub(super) fn slice_equality(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        raw_values: &[Value],
        state: &State,
        stack: &[DefId],
        site: (DefId, Span),
    ) -> Result<Option<Vec<Return>>, String> {
        let callee = instance.def_id();
        let parent = self.tcx.parent(callee);
        let name = self.tcx.item_name(callee);
        if !matches!(name.as_str(), "eq" | "ne")
            || !matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: true })
            || !self
                .tcx
                .get_diagnostic_item(Symbol::intern("cmp_partialeq_eq"))
                .is_some_and(|method| {
                    self.tcx.parent(method)
                        == self
                            .tcx
                            .impl_trait_ref(parent)
                            .instantiate(self.tcx, instance.args)
                            .skip_norm_wip()
                            .def_id
                })
        {
            return Ok(None);
        }
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx.fn_sig(callee).instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("slice equality signature normalization failed: {error:?}"))?
            .skip_binder();
        let [left, right] = signature.inputs() else {
            return Ok(None);
        };
        let (ty::Ref(_, left, left_mut), ty::Ref(_, right, right_mut)) =
            (left.kind(), right.kind())
        else {
            return Ok(None);
        };
        let (left_element, right_element) = match (left.kind(), right.kind()) {
            (ty::Slice(left), ty::Slice(right))
            | (ty::Array(left, _), ty::Slice(right))
            | (ty::Slice(left), ty::Array(right, _)) => (*left, *right),
            _ => return Ok(None),
        };
        if left_mut.is_mut() || right_mut.is_mut() || !signature.output().is_bool() {
            return Ok(None);
        }
        let [left, right] = values else {
            return Err("slice equality requires two modeled arguments".into());
        };
        let left = left.materialize()?;
        let right = right.materialize()?;
        let left_length = self.equality_slice_length(&left)?;
        let right_length = self.equality_slice_length(&right)?;
        let same_length =
            symbolic::binary(&self.terms, "eq", left_length.clone(), right_length)?.boolean()?;
        let negate = name == Symbol::intern("ne");
        let mut returns = Vec::new();
        let mismatch = [state.conditions.clone(), vec![symbolic::not(&same_length)]].concat();
        if self.feasible(&mismatch)? {
            returns.push(Return {
                value: Value::Bool(self.terms.boolean(negate)),
                conditions: mismatch,
                memory: state.memory.clone(),
            });
        }
        let conditions = [state.conditions.clone(), vec![same_length]].concat();
        self.record_model(
            callee,
            "slice equality; length check before element comparisons",
        );
        if !self.feasible(&conditions)? {
            return Ok(Some(returns));
        }
        let (length, bits, _) = left_length.integer()?;
        let limit = self
            .terms
            .bit_vector(MAX_SLICE_EQUALITY_ELEMENTS as u128, bits)?;
        let outside = self
            .terms
            .apply(Op::BvUnsignedGt, &[length.clone(), limit])?;
        if self.feasible(&[conditions.clone(), vec![outside]].concat())? {
            return Err("slice equality needs a proven length at most 128".into());
        }
        let primitive = left_element == right_element
            && (left_element.is_bool()
                || left_element.is_char()
                || self.integer_type(left_element).is_some()
                || self.float_type(left_element).is_some());
        if !primitive {
            let Value::Elements(elements) = &left else {
                return Err("custom slice equality needs fixed modeled element storage".into());
            };
            let state = State {
                conditions,
                ..state.clone()
            };
            returns.extend(self.custom_sequence_equality(
                instance,
                (left_element, right_element, elements.len(), negate),
                raw_values,
                &state,
                stack,
                site,
            )?);
            return Ok(Some(returns));
        }
        let count = match (&left, &right) {
            (Value::Elements(elements), _) | (_, Value::Elements(elements)) => elements.len(),
            _ => match symbolic::constant(&length) {
                Some(symbolic::Constant::BitVec { value, .. }) => value as usize,
                _ => MAX_SLICE_EQUALITY_ELEMENTS,
            },
        };
        let mut equal = self.terms.boolean(true);
        for index in 0..count {
            let left = self.slice_equality_item(&left, left_element, index, bits)?;
            let right = self.slice_equality_item(&right, right_element, index, bits)?;
            let item = symbolic::binary(&self.terms, "eq", left, right)?.boolean()?;
            let index = self.terms.bit_vector(index as u128, bits)?;
            let inside = self
                .terms
                .apply(Op::BvUnsignedLt, &[index, length.clone()])?;
            let item = self.terms.apply(Op::Or, &[symbolic::not(&inside), item])?;
            equal = self.terms.apply(Op::And, &[equal, item])?;
        }
        self.record_model(
            callee,
            "bounded primitive slice equality; exact numeric comparisons",
        );
        returns.push(Return {
            value: Value::Bool(if negate { symbolic::not(&equal) } else { equal }),
            conditions,
            memory: state.memory.clone(),
        });
        Ok(Some(returns))
    }

    fn equality_slice_length(&self, source: &Value) -> Result<Value, String> {
        match source {
            Value::Bytes { length, .. } => Ok((**length).clone()),
            Value::Elements(fields) => Ok(self.iterator_index(fields.len() as u128)),
            Value::Bool(_)
            | Value::Int { .. }
            | Value::Float { .. }
            | Value::Adt { .. }
            | Value::Enum { .. }
            | Value::Cell { .. }
            | Value::Atomic { .. }
            | Value::Reference { .. }
            | Value::SliceIterator { .. }
            | Value::Tuple(_)
            | Value::MetadataPointer(_)
            | Value::StaticText
            | Value::FormatArguments
            | Value::Input(_)
            | Value::RawPointer { .. }
            | Value::StaticSlice { .. }
            | Value::StaticView { .. }
            | Value::Uninitialized
            | Value::DebugReference { .. }
            | Value::FunctionPointer { .. }
            | Value::Function
            | Value::Unit => Err("slice equality needs byte or element storage".into()),
        }
    }

    fn slice_equality_item(
        &self,
        value: &Value,
        element: Ty<'tcx>,
        index: usize,
        bits: u32,
    ) -> Result<Value, String> {
        match value {
            Value::Elements(elements) => elements
                .get(index)
                .cloned()
                .ok_or_else(|| "slice equality index exceeds modeled element storage".into()),
            Value::Bytes { data, .. } if element == self.tcx.types.u8 => Ok(Value::Int {
                expression: self.terms.apply(
                    Op::Select,
                    &[data.clone(), self.terms.bit_vector(index as u128, bits)?],
                )?,
                bits: 8,
                signed: false,
            }),
            Value::Input(_)
            | Value::Bool(_)
            | Value::Int { .. }
            | Value::Float { .. }
            | Value::Bytes { .. }
            | Value::Adt { .. }
            | Value::Enum { .. }
            | Value::Cell { .. }
            | Value::Atomic { .. }
            | Value::Reference { .. }
            | Value::SliceIterator { .. }
            | Value::Tuple(_)
            | Value::MetadataPointer(_)
            | Value::StaticText
            | Value::FormatArguments
            | Value::RawPointer { .. }
            | Value::StaticSlice { .. }
            | Value::StaticView { .. }
            | Value::Uninitialized
            | Value::DebugReference { .. }
            | Value::FunctionPointer { .. }
            | Value::Function
            | Value::Unit => Err("slice equality needs modeled element storage".into()),
        }
    }
}
