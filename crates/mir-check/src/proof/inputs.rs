use super::*;

const MAX_LAZY_SYMBOL_SLOTS: u32 = 262_144;

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

impl<'tcx> Engine<'tcx> {
    pub(super) fn lazy_argument(&mut self, ty: Ty<'tcx>) -> Result<Option<Value>, String> {
        let (ty, depth) = match ty.kind() {
            ty::Ref(_, element, mutability)
                if !mutability.is_mut() && !self.building_mutable_input =>
            {
                (*element, self.input_depth + 1)
            }
            _ => (ty, self.input_depth),
        };
        if !matches!(ty.kind(), ty::Adt(..) | ty::Tuple(_) | ty::Array(..))
            || !ty.is_freeze(self.tcx, ty::TypingEnv::fully_monomorphized())
        {
            return Ok(None);
        }
        let mut nodes = 0;
        let mut cache = self.input_shapes.clone();
        let Some(shape) = self.lazy_shape(ty, depth, &mut nodes, &mut cache)? else {
            return Ok(None);
        };
        if nodes >= shape.slots as usize
            || (!self.prefer_lazy_inputs
                && shape.slots as usize <= MAX_INPUT_VALUES.saturating_sub(self.input_values))
        {
            return Ok(None);
        }
        let cost = nodes + 1;
        if cost > MAX_INPUT_VALUES.saturating_sub(self.input_values) {
            return Err("lazy input descriptors exceed the 512-node budget".to_owned());
        }
        let start = self.next_symbol;
        let next = start
            .checked_add(shape.slots)
            .ok_or("lazy input symbol range overflow")?;
        if next > MAX_LAZY_SYMBOL_SLOTS {
            return Err("lazy input exceeds the 262144 reserved-symbol-slot budget".to_owned());
        }
        let seed = self.terms.symbol(start, Sort::Bool)?;
        self.next_symbol = next;
        self.input_values += cost;
        self.input_shapes = cache;
        Ok(Some(Value::Input(symbolic::input::InputValue {
            shape,
            start,
            seed,
        })))
    }

    fn lazy_shape(
        &self,
        ty: Ty<'tcx>,
        depth: usize,
        nodes: &mut usize,
        cache: &mut std::collections::HashMap<Ty<'tcx>, std::rc::Rc<symbolic::input::InputShape>>,
    ) -> Result<Option<std::rc::Rc<symbolic::input::InputShape>>, String> {
        use symbolic::input::{InputKind, InputShape};
        if depth >= MAX_INPUT_DEPTH {
            return Ok(None);
        }
        if let Some(shape) = cache.get(&ty) {
            return Ok((depth + shape.height <= MAX_INPUT_DEPTH).then(|| shape.clone()));
        }
        if *nodes >= MAX_INPUT_VALUES {
            return Ok(None);
        }
        *nodes += 1;
        let kind = match ty.kind() {
            ty::Bool => InputKind::Bool,
            ty::Int(_) | ty::Uint(_) => {
                let (bits, signed) = self.integer_type(ty).ok_or("lazy integer type mismatch")?;
                InputKind::Integer { bits, signed }
            }
            ty::Float(_) => {
                let Some(bits) = self.float_type(ty) else {
                    return Ok(None);
                };
                InputKind::Float { bits }
            }
            ty::Tuple(fields) if fields.is_empty() => InputKind::Unit,
            ty::Tuple(fields) => {
                let mut shapes = Vec::new();
                for field in fields.iter() {
                    if *nodes >= MAX_INPUT_VALUES {
                        return Ok(None);
                    }
                    *nodes += 1;
                    let Some(shape) = self.lazy_shape(field, depth + 1, nodes, cache)? else {
                        return Ok(None);
                    };
                    shapes.push(shape);
                }
                InputKind::Tuple(shapes)
            }
            ty::Array(element, count) => {
                let Some(count) = count.try_to_target_usize(self.tcx) else {
                    return Ok(None);
                };
                if *element == self.tcx.types.u8 {
                    InputKind::Bytes {
                        count,
                        pointer_bits: u32::from(self.tcx.sess.target.pointer_width),
                    }
                } else {
                    if count > 256 {
                        return Ok(None);
                    }
                    let Some(element) = self.lazy_shape(*element, depth + 1, nodes, cache)? else {
                        return Ok(None);
                    };
                    InputKind::Array {
                        element,
                        count: count as usize,
                    }
                }
            }
            ty::Adt(def, args) if def.is_struct() => {
                if self
                    .tcx
                    .is_diagnostic_item(rustc_span::Symbol::intern("NonZero"), def.did())
                {
                    return Ok(None);
                }
                let mut fields = Vec::new();
                for field in &def.non_enum_variant().fields {
                    if *nodes >= MAX_INPUT_VALUES {
                        return Ok(None);
                    }
                    *nodes += 1;
                    let ty = self
                        .tcx
                        .try_normalize_erasing_regions(
                            ty::TypingEnv::fully_monomorphized(),
                            field.ty(self.tcx, args),
                        )
                        .map_err(|error| {
                            format!("lazy input field normalization failed: {error:?}")
                        })?;
                    let Some(shape) = self.lazy_shape(ty, depth + 1, nodes, cache)? else {
                        return Ok(None);
                    };
                    fields.push((field.name.as_str().to_owned(), shape));
                }
                InputKind::Struct {
                    name: self.tcx.def_path_str(def.did()),
                    fields,
                }
            }
            // These shapes need eager validity conditions or an alias/representation model.
            _ => return Ok(None),
        };
        let slots = match &kind {
            InputKind::Bool
            | InputKind::Integer { .. }
            | InputKind::Float { .. }
            | InputKind::Unit
            | InputKind::Bytes { .. } => 1,
            InputKind::Tuple(fields) => fields.iter().try_fold(1_u32, |sum, field| {
                sum.checked_add(field.slots)
                    .ok_or("lazy input symbol range overflow")
            })?,
            InputKind::Struct { fields, .. } => {
                fields.iter().try_fold(1_u32, |sum, (_, field)| {
                    sum.checked_add(field.slots)
                        .ok_or("lazy input symbol range overflow")
                })?
            }
            InputKind::Array { element, count } => element
                .slots
                .checked_mul(*count as u32)
                .and_then(|slots| slots.checked_add(1))
                .ok_or("lazy input symbol range overflow")?,
        };
        let height = match &kind {
            InputKind::Bool
            | InputKind::Integer { .. }
            | InputKind::Float { .. }
            | InputKind::Unit
            | InputKind::Bytes { .. } => 1,
            InputKind::Tuple(fields) => {
                1 + fields.iter().map(|field| field.height).max().unwrap_or(0)
            }
            InputKind::Struct { fields, .. } => {
                1 + fields
                    .iter()
                    .map(|(_, field)| field.height)
                    .max()
                    .unwrap_or(0)
            }
            InputKind::Array { element, .. } => 1 + element.height,
        };
        let shape = std::rc::Rc::new(InputShape {
            kind,
            slots,
            height,
        });
        cache.insert(ty, shape.clone());
        Ok(Some(shape))
    }

    pub(super) fn root_input_cost(&self, ty: Ty<'tcx>) -> usize {
        if let ty::Ref(_, element, mutability) = ty.kind() {
            if mutability.is_mut() {
                return self.eager_input_cost(*element, 0);
            }
            if let Some(inner) = self.cell_element(*element) {
                return self.eager_input_cost(inner, 0);
            }
        }
        self.eager_input_cost(ty, 0)
    }

    fn eager_input_cost(&self, ty: Ty<'tcx>, depth: usize) -> usize {
        let limit = MAX_INPUT_VALUES + 1;
        if depth >= MAX_INPUT_DEPTH {
            return limit;
        }
        if self.atomic_shape(ty).is_some() {
            return 1;
        }
        let field_cost = |ty| self.eager_input_cost(ty, depth + 1);
        match ty.kind() {
            ty::Pat(base, _) => self.eager_input_cost(*base, depth),
            ty::Ref(_, element, mutability) if !mutability.is_mut() => {
                if matches!(element.kind(), ty::Slice(item) if *item == self.tcx.types.u8) {
                    1
                } else {
                    1_usize.saturating_add(field_cost(*element)).min(limit)
                }
            }
            ty::Tuple(fields) => {
                let mut cost = 1_usize;
                for field in fields.iter() {
                    cost = cost.saturating_add(field_cost(field));
                    if cost >= limit {
                        return limit;
                    }
                }
                cost
            }
            ty::Array(element, _) if *element == self.tcx.types.u8 => 1,
            ty::Array(element, count) => match count.try_to_target_usize(self.tcx) {
                Some(0) => 1,
                Some(count) if count <= 256 => 1_usize
                    .saturating_add(field_cost(*element).saturating_mul(count as usize))
                    .min(limit),
                Some(_) | None => limit,
            },
            ty::Adt(def, args) if !def.is_union() => {
                let mut cost = 1_usize;
                for variant in def.variants() {
                    if def.is_enum() {
                        cost = cost.saturating_add(1);
                    }
                    for field in &variant.fields {
                        let Ok(ty) = self.tcx.try_normalize_erasing_regions(
                            ty::TypingEnv::fully_monomorphized(),
                            field.ty(self.tcx, args),
                        ) else {
                            return limit;
                        };
                        cost = cost.saturating_add(field_cost(ty));
                        if cost >= limit {
                            return limit;
                        }
                    }
                }
                cost
            }
            _ => 1,
        }
    }
}
