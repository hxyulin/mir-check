use super::*;
use rustc_index::IndexVec;

const MAX_ARRAY_BYTES: u64 = 128;

impl<'tcx> Engine<'tcx> {
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
                let ty = field.ty(self.tcx, args).skip_norm_wip();
                if matches!(ty.kind(), ty::Ref(..) | ty::Adt(..)) {
                    return Err("reference and nested ADT input fields are not modeled".to_owned());
                }
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
            AggregateKind::Adt(id, variant, _, _, active) if active.is_none() => {
                let def = self.tcx.adt_def(*id);
                if !def.is_struct() && self.tcx.lang_items().get(LangItem::Option) != Some(*id) {
                    return Err(
                        "only structs and constructed Option variants are modeled".to_owned()
                    );
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

    pub(super) fn repeated_bytes(
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
            return Err("byte array model size limit reached".to_owned());
        }
        let (expression, width, signed) = self.operand(id, body, state, operand)?.integer()?;
        if width != 8 || signed {
            return Err("only byte repeats are modeled".to_owned());
        }
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
