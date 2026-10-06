use super::*;
use rustc_index::IndexVec;

const MAX_ARRAY_BYTES: u64 = 128;
const MAX_ARRAY_ELEMENTS: usize = 16;
const MAX_ENUM_VARIANTS: usize = 16;

impl<'tcx> Engine<'tcx> {
    pub(super) fn enum_input(
        &mut self,
        id: DefId,
        ty: Ty<'tcx>,
        conditions: &mut Vec<String>,
    ) -> Result<Value, String> {
        let ty::Adt(def, args) = ty.kind() else {
            return Err("expected an enum type".to_owned());
        };
        if def.variants().is_empty() || def.variants().len() > MAX_ENUM_VARIANTS {
            return Err("enum inputs need between 1 and 16 variants".to_owned());
        }
        let (bits, signed) = self
            .integer_type(ty.discriminant_ty(self.tcx))
            .ok_or("unsupported enum discriminant type")?;
        let discriminant = Value::Int {
            expression: self.fresh(&format!("(_ BitVec {bits})")),
            bits,
            signed,
        };
        let mut variants = Vec::new();
        let mut valid = Vec::new();
        for (index, variant) in def.variants().iter_enumerated() {
            if self.input_values >= MAX_INPUT_VALUES {
                return Err("input shape exceeds the 128-value budget".to_owned());
            }
            self.input_values += 1;
            let tag = def.discriminant_for_variant(self.tcx, index).val;
            valid.push(
                symbolic::binary(
                    "eq",
                    discriminant.clone(),
                    symbolic::integer(tag, bits, signed),
                )?
                .boolean()?,
            );
            let fields = variant
                .fields
                .iter()
                .map(|field| {
                    let ty = self
                        .tcx
                        .try_normalize_erasing_regions(
                            ty::TypingEnv::fully_monomorphized(),
                            field.ty(self.tcx, args),
                        )
                        .map_err(|error| format!("input field normalization failed: {error:?}"))?;
                    self.argument(id, ty, conditions)
                })
                .collect::<Result<Vec<_>, _>>()?;
            variants.push(self.constructed(ty, index.as_usize(), fields)?);
        }
        conditions.push(format!("(or {})", valid.join(" ")));
        Ok(Value::Enum {
            discriminant: Box::new(discriminant),
            variants,
            is_option: self.tcx.lang_items().get(LangItem::Option) == Some(def.did()),
        })
    }

    pub(super) fn struct_input(
        &mut self,
        id: DefId,
        ty: Ty<'tcx>,
        conditions: &mut Vec<String>,
    ) -> Result<Value, String> {
        let ty::Adt(def, args) = ty.kind() else {
            return Err("expected a struct type".to_owned());
        };
        let fields = def
            .non_enum_variant()
            .fields
            .iter()
            .map(|field| {
                let ty = self
                    .tcx
                    .try_normalize_erasing_regions(
                        ty::TypingEnv::fully_monomorphized(),
                        field.ty(self.tcx, args),
                    )
                    .map_err(|error| format!("input field normalization failed: {error:?}"))?;
                Ok((
                    field.name.as_str().to_owned(),
                    self.argument(id, ty, conditions)?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Value::Adt {
            name: self.tcx.def_path_str(def.did()),
            variant: 0,
            is_option: false,
            discriminant: 0,
            fields,
        })
    }

    pub(super) fn element_input(
        &mut self,
        id: DefId,
        ty: Ty<'tcx>,
        conditions: &mut Vec<String>,
    ) -> Result<Value, String> {
        let ty::Array(element, count) = ty.kind() else {
            return Err("expected a fixed array input".to_owned());
        };
        let count = count
            .try_to_target_usize(self.tcx)
            .ok_or("unknown input array length")?;
        if count > MAX_ARRAY_ELEMENTS as u64 {
            return Err("fixed array input exceeds 16 elements".to_owned());
        }
        Ok(Value::Elements(
            (0..count)
                .map(|_| self.argument(id, *element, conditions))
                .collect::<Result<_, _>>()?,
        ))
    }

    pub(super) fn aggregate(
        &self,
        id: DefId,
        body: &Body<'tcx>,
        state: &State,
        kind: &AggregateKind<'tcx>,
        operands: &IndexVec<rustc_abi::FieldIdx, Operand<'tcx>>,
    ) -> Result<Value, String> {
        let values = operands
            .iter()
            .map(|value| self.operand(id, body, state, value))
            .collect::<Result<Vec<_>, _>>()?;
        if values.iter().any(Value::contains_mutable) {
            return Err("mutable borrows cannot be stored in aggregates".to_owned());
        }
        match kind {
            AggregateKind::Array(element) if *element == self.tcx.types.u8 => {
                if values.len() as u64 > MAX_ARRAY_BYTES {
                    return Err("byte array model size limit reached".to_owned());
                }
                let bits = u32::from(self.tcx.sess.target.pointer_width);
                let mut data =
                    format!("((as const (Array (_ BitVec {bits}) (_ BitVec 8))) (_ bv0 8))");
                for (index, value) in values.iter().enumerate() {
                    let (expression, width, signed) = value.integer()?;
                    if width != 8 || signed {
                        return Err("non-byte array element".to_owned());
                    }
                    data = format!("(store {data} (_ bv{index} {bits}) {expression})");
                }
                Ok(Value::Bytes {
                    length: Box::new(symbolic::integer(values.len() as u128, bits, false)),
                    data,
                })
            }
            AggregateKind::Closure(id, _) => Ok(Value::Adt {
                name: self.tcx.def_path_str(*id),
                variant: 0,
                is_option: false,
                discriminant: 0,
                fields: values
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| (index.to_string(), value))
                    .collect(),
            }),
            AggregateKind::Array(_) => {
                if values.len() > MAX_ARRAY_ELEMENTS {
                    return Err("fixed array element model size limit reached".to_owned());
                }
                Ok(Value::Elements(values))
            }
            AggregateKind::Adt(id, variant, _, _, active) if active.is_none() => {
                let def = self.tcx.adt_def(*id);
                if !def.is_struct() && !def.is_enum() {
                    return Err("only struct and enum aggregates are modeled".to_owned());
                }
                let layout = def.variant(*variant);
                if layout.fields.len() != values.len() {
                    return Err("ADT field count mismatch".to_owned());
                }
                Ok(Value::Adt {
                    name: self.tcx.def_path_str(*id),
                    variant: variant.as_usize(),
                    is_option: self.tcx.lang_items().get(LangItem::Option) == Some(*id),
                    discriminant: if def.is_enum() {
                        def.discriminant_for_variant(self.tcx, *variant).val
                    } else {
                        0
                    },
                    fields: layout
                        .fields
                        .iter()
                        .zip(values)
                        .map(|(field, value)| (field.name.as_str().to_owned(), value))
                        .collect(),
                })
            }
            _ => Err("unsupported aggregate kind".to_owned()),
        }
    }

    pub(super) fn fixed_element(
        &self,
        elements: &[Value],
        index: &Value,
        conditions: &[String],
    ) -> Result<Value, String> {
        let (_, bits, signed) = index.integer()?;
        if signed || bits != u32::from(self.tcx.sess.target.pointer_width) {
            return Err("fixed array index type mismatch".to_owned());
        }
        for (position, element) in elements.iter().enumerate() {
            let equal = symbolic::binary(
                "eq",
                index.clone(),
                symbolic::integer(position as u128, bits, false),
            )?
            .boolean()?;
            let mut different = conditions.to_vec();
            different.push(symbolic::not(&equal));
            if !self.feasible(&different)? {
                return Ok(element.clone());
            }
        }
        let bound = symbolic::binary(
            "lt",
            index.clone(),
            symbolic::integer(elements.len() as u128, bits, false),
        )?
        .boolean()?;
        let mut outside = conditions.to_vec();
        outside.push(symbolic::not(&bound));
        if self.feasible(&outside)? {
            return Err("array read lacks a proven bounds check".to_owned());
        }
        symbolic::select_element(elements, index).map_err(|_| {
            "non-scalar array index is not uniquely determined on this path".to_owned()
        })
    }

    pub(super) fn constant_element(
        &self,
        state: &State,
        value: Value,
        offset: u64,
        min_length: u64,
        from_end: bool,
    ) -> Result<Value, String> {
        let bits = u32::from(self.tcx.sess.target.pointer_width);
        let length = match &value {
            Value::Bytes { length, .. } => length.as_ref().clone(),
            Value::Elements(elements) => symbolic::integer(elements.len() as u128, bits, false),
            _ => return Err("constant indexing needs a modeled array or slice".to_owned()),
        };
        let required = if from_end {
            min_length.max(offset)
        } else {
            min_length
        };
        let long_enough = symbolic::binary(
            "ge",
            length.clone(),
            symbolic::integer(u128::from(required), bits, false),
        )?
        .boolean()?;
        let mut too_short = state.conditions.clone();
        too_short.push(symbolic::not(&long_enough));
        if self.feasible(&too_short)? {
            return Err("constant index lacks a proven minimum length".to_owned());
        }
        let offset = symbolic::integer(u128::from(offset), bits, false);
        let index = if from_end {
            symbolic::binary("sub", length.clone(), offset)?
        } else {
            offset
        };
        match value {
            Value::Elements(elements) => self.fixed_element(&elements, &index, &state.conditions),
            Value::Bytes { data, .. } => {
                let inside = symbolic::binary("lt", index.clone(), length)?.boolean()?;
                let mut outside = state.conditions.clone();
                outside.push(symbolic::not(&inside));
                if self.feasible(&outside)? {
                    return Err("constant byte index lacks a proven bounds check".to_owned());
                }
                let (expression, _, _) = index.integer()?;
                Ok(Value::Int {
                    expression: format!("(select {data} {expression})"),
                    bits: 8,
                    signed: false,
                })
            }
            _ => Err("constant indexing needs a modeled array or slice".to_owned()),
        }
    }

    pub(super) fn repeated_array(
        &self,
        id: DefId,
        body: &Body<'tcx>,
        state: &State,
        operand: &Operand<'tcx>,
        length: ty::Const<'tcx>,
    ) -> Result<Value, String> {
        let count = length
            .try_to_target_usize(self.tcx)
            .ok_or("unevaluated repeat length")?;
        if count > MAX_ARRAY_BYTES {
            return Err("array repeat exceeds the 128-element limit".to_owned());
        }
        if !operand
            .ty(body, self.tcx)
            .is_freeze(self.tcx, ty::TypingEnv::fully_monomorphized())
        {
            return Err("repeated interior mutable storage is not modeled".to_owned());
        }
        let value = self.operand(id, body, state, operand)?;
        if !matches!(
            value,
            Value::Int {
                bits: 8,
                signed: false,
                ..
            }
        ) {
            let size = value
                .owned_repeat_size()
                .ok_or("repeat operand needs a small owned value")?;
            if size * count as usize + 1 > symbolic::MAX_REPEAT_VALUES {
                return Err("owned repeat exceeds the 256-value budget".to_owned());
            }
            return Ok(Value::Elements(vec![value; count as usize]));
        }
        let (expression, _, _) = value.integer()?;
        let bits = u32::from(self.tcx.sess.target.pointer_width);
        Ok(Value::Bytes {
            length: Box::new(symbolic::integer(count as u128, bits, false)),
            data: format!("((as const (Array (_ BitVec {bits}) (_ BitVec 8))) {expression})"),
        })
    }

    pub(super) fn mutable_bytes(&self, state: &State, place: Place<'tcx>) -> Result<Value, String> {
        if place.projection.is_empty() {
            let Value::Bytes { length, .. } = self.place(state, place)? else {
                return Err("mutable borrowing only models owned local byte arrays".to_owned());
            };
            return Ok(Value::MutableBytes {
                owner: place.local.as_usize(),
                length,
            });
        }
        let value = self.place(state, place)?;
        if matches!(value, Value::MutableBytes { .. }) {
            Ok(value)
        } else {
            Err("unsupported mutable reborrow".to_owned())
        }
    }
}
