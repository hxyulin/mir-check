use super::*;

impl<'tcx> Engine<'tcx> {
    pub(super) fn loop_enum(&mut self, ty: Ty<'tcx>, depth: usize) -> Result<Value, String> {
        let ty::Adt(def, args) = ty.kind() else {
            return Err("expected an inductive enum".into());
        };
        if def.variants().is_empty() || def.variants().len() > 16 {
            return Err("inductive enums require between 1 and 16 variants".into());
        }
        let (bits, signed) = self
            .integer_type(ty.discriminant_ty(self.tcx))
            .ok_or("unsupported inductive enum tag type")?;
        let mut variants = Vec::new();
        for (index, variant) in def.variants().iter_enumerated() {
            let fields = variant
                .fields
                .iter()
                .map(|field| {
                    let field_ty = self
                        .tcx
                        .try_normalize_erasing_regions(
                            ty::TypingEnv::fully_monomorphized(),
                            field.ty(self.tcx, args),
                        )
                        .map_err(|error| {
                            format!("loop enum field normalization failed: {error:?}")
                        })?;
                    self.loop_input(field_ty, depth + 1)
                })
                .collect::<Result<_, _>>()?;
            variants.push(self.constructed(ty, index.as_usize(), fields)?);
        }
        Ok(Value::Enum {
            discriminant: Box::new(Value::Int {
                expression: self.fresh(Sort::BitVec(bits)),
                bits,
                signed,
            }),
            variants,
            is_option: self.tcx.lang_items().get(LangItem::Option) == Some(def.did()),
        })
    }

    pub(super) fn loop_normalize(
        &self,
        frame: &Frame<'tcx>,
        state: &mut State,
    ) -> Result<(), String> {
        for (template, value) in frame.state.locals.iter().zip(&mut state.locals) {
            normalize(&self.terms, template.as_ref(), value)?;
        }
        for (template, value) in frame.state.memory.iter().zip(&mut state.memory) {
            normalize(&self.terms, template.as_ref(), value)?;
        }
        Ok(())
    }
}

fn normalize(
    context: &Context,
    template: Option<&Value>,
    slot: &mut Option<Value>,
) -> Result<(), String> {
    let template = template.ok_or("missing inductive value template")?;
    let value = slot.as_mut().ok_or("missing inductive value")?;
    match (template, &mut *value) {
        (
            Value::Enum {
                discriminant,
                variants,
                is_option,
            },
            Value::Adt { variant, .. },
        ) => {
            let index = *variant;
            let (_, bits, signed) = discriminant.integer()?;
            let mut payloads = variants.clone();
            let expected = payloads
                .get(index)
                .ok_or("inductive enum variant missing")?;
            same_shape(Some(expected), Some(value))?;
            let Value::Adt {
                discriminant: tag, ..
            } = value
            else {
                unreachable!();
            };
            let tag = *tag;
            payloads[index] = value.clone();
            *value = Value::Enum {
                discriminant: Box::new(symbolic::integer(context, tag, bits, signed)),
                variants: payloads,
                is_option: *is_option,
            };
        }
        (
            Value::Enum {
                variants: expected, ..
            },
            Value::Enum { variants, .. },
        ) => {
            for (template, field) in expected.iter().zip(variants) {
                let mut slot = Some(field.clone());
                normalize(context, Some(template), &mut slot)?;
                *field = slot.ok_or("missing enum payload")?;
            }
        }
        (
            Value::Adt {
                fields: expected, ..
            },
            Value::Adt { fields, .. },
        ) => {
            for ((_, template), (_, field)) in expected.iter().zip(fields) {
                let mut slot = Some(field.clone());
                normalize(context, Some(template), &mut slot)?;
                *field = slot.ok_or("missing inductive field")?;
            }
        }
        (Value::Tuple(expected), Value::Tuple(fields))
        | (Value::Elements(expected), Value::Elements(fields)) => {
            for (template, field) in expected.iter().zip(fields) {
                let mut slot = Some(field.clone());
                normalize(context, Some(template), &mut slot)?;
                *field = slot.ok_or("missing inductive element")?;
            }
        }
        (
            Value::Bool(_)
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
            | Value::Elements(_)
            | Value::MetadataPointer(_)
            | Value::StaticText
            | Value::FormatArguments
            | Value::Input(_)
            | Value::RawPointer { .. }
            | Value::StaticView { .. }
            | Value::Uninitialized
            | Value::Function
            | Value::Unit,
            _,
        ) => {}
    }
    Ok(())
}
