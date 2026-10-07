use super::*;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    pub(super) fn is_core_panic_helper(&self, callee: DefId) -> bool {
        let Some(option) = self.tcx.lang_items().get(LangItem::Option) else {
            return false;
        };
        if self.tcx.def_kind(callee) != DefKind::Fn
            || self.tcx.parent(callee) != self.tcx.parent(option)
        {
            return false;
        }
        let signature = self.tcx.fn_sig(callee).instantiate_identity().skip_binder();
        if !signature.output().is_never() {
            return false;
        }
        let name = self.tcx.item_name(callee);
        if name == Symbol::intern("unwrap_failed") {
            return signature.inputs().is_empty();
        }
        name == Symbol::intern("expect_failed")
            && matches!(signature.inputs(), [input]
                if matches!(input.kind(), ty::Ref(_, element, mutability)
                    if element.is_str() && !mutability.is_mut()))
    }

    pub(super) fn materialize_float(&mut self, value: Value) -> Value {
        let Value::Float {
            expression,
            bits,
            raw_bits: None,
        } = value
        else {
            return value;
        };
        let raw_bits = self.fresh(Sort::BitVec(bits));
        let Value::Float {
            expression: decoded,
            ..
        } = symbolic::float_from_bits(&self.terms, raw_bits.clone(), bits)
        else {
            unreachable!("bit decoding constructs a float");
        };
        // SMT represents all NaN encodings as one value. This equality allows every
        // NaN payload/sign while fixing the exact encoding of other IEEE values.
        self.float_encodings.insert(
            raw_bits.symbol_index().expect("fresh encoding symbol"),
            self.terms
                .apply(Op::Equal, &[decoded, expression.clone()])
                .expect("matching float encoding sorts"),
        );
        Value::Float {
            expression,
            bits,
            raw_bits: Some(raw_bits),
        }
    }

    pub(super) fn float_to_bits(
        &self,
        value: Value,
        width: u32,
        signed: bool,
    ) -> Result<Value, String> {
        let Value::Float { bits, raw_bits, .. } = value else {
            return Err("float bit observation requires a float".to_owned());
        };
        if bits != width {
            return Err("float bit observation width mismatch".to_owned());
        }
        Ok(Value::Int {
            expression: raw_bits.ok_or("float result has no tracked storage encoding")?,
            bits,
            signed,
        })
    }

    pub(super) fn builtin(
        &mut self,
        caller_body: &Body<'tcx>,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        raw_values: &[Value],
        state: &mut State,
        span: Span,
    ) -> Result<Option<Value>, String> {
        let caller = caller_body.source.def_id();
        let callee = instance.def_id();
        let args = instance.args;
        if self.tcx.def_kind(callee) == DefKind::Closure {
            return Ok(None);
        }
        if let Some(value) = self.nonzero_get(instance, values)? {
            self.record_model(callee, "nonzero integer value");
            return Ok(Some(value));
        }
        if self.tcx.lang_items().get(LangItem::SliceLen) == Some(callee) {
            let [receiver] = values else {
                return Err("slice len receiver is not modeled".to_owned());
            };
            self.record_model(callee, "slice length");
            let receiver = receiver.materialize()?;
            return match &receiver {
                Value::Bytes { length, .. } => Ok(Some((**length).clone())),
                Value::Elements(elements) | Value::StaticSlice { elements, .. } => {
                    Ok(Some(symbolic::integer(
                        &self.terms,
                        elements.len() as u128,
                        u32::from(self.tcx.sess.target.pointer_width),
                        false,
                    )))
                }
                _ => Err("slice len receiver is not modeled".to_owned()),
            };
        }
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx.fn_sig(callee).instantiate(self.tcx, args),
            )
            .map_err(|error| format!("builtin signature normalization failed: {error:?}"))?
            .skip_binder();
        self.check_fence_ordering(instance, signature, values, state, (caller, span))?;
        if let Some(value) = self.atomic_fence_intrinsic(instance, signature, values, span)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.pin_mutable_deref(instance, signature, raw_values, state)? {
            self.record_model(
                callee,
                "core Pin<&mut T> mutable dereference; tracked storage",
            );
            return Ok(Some(value));
        }
        if let Some(value) = self.task_context_constructor(instance, signature, values)? {
            return Ok(Some(value));
        }
        if self.tcx.is_intrinsic(callee, Symbol::intern("cold_path"))
            && signature.inputs().is_empty()
            && signature.output().is_unit()
            && values.is_empty()
        {
            self.record_model(callee, "optimization-only cold path marker");
            return Ok(Some(Value::Unit));
        }
        if self
            .tcx
            .is_intrinsic(callee, Symbol::intern("is_val_statically_known"))
            && signature.inputs().len() == 1
            && signature.output().is_bool()
            && (signature.inputs()[0].is_bool()
                || self.integer_type(signature.inputs()[0]).is_some()
                || self.float_type(signature.inputs()[0]).is_some())
        {
            if values.len() != 1 {
                return Err("static-value hint requires one modeled scalar".to_owned());
            }
            self.record_model(
                callee,
                "static-value optimization hint; independent Boolean per call",
            );
            return Ok(Some(Value::Bool(self.fresh(Sort::Bool))));
        }
        if let Some(value) = self.core_endian_encoding(callee, args, signature, values)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.integer_intrinsic(callee, signature, values)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.floating_intrinsic(callee, signature, values)? {
            return Ok(Some(value));
        }
        if self.tcx.is_intrinsic(callee, Symbol::intern("ctpop"))
            && signature.inputs().len() == 1
            && self.integer_type(signature.inputs()[0]).is_some()
            && signature.output() == self.tcx.types.u32
        {
            let [value] = values else {
                return Err("population count requires one modeled integer".to_owned());
            };
            let (expression, bits, _) = value.integer()?;
            let mut terms = (0..bits)
                .map(|bit| {
                    let value = self.terms.apply(
                        Op::Extract {
                            high: bit,
                            low: bit,
                        },
                        std::slice::from_ref(&expression),
                    )?;
                    self.terms.apply(Op::ZeroExtend(31), &[value])
                })
                .collect::<Result<Vec<_>, String>>()?;
            while terms.len() > 1 {
                terms = terms
                    .chunks(2)
                    .map(|pair| match pair {
                        [left, right] => {
                            self.terms.apply(Op::BvAdd, &[left.clone(), right.clone()])
                        }
                        [only] => Ok(only.clone()),
                        _ => unreachable!("pairs have one or two elements"),
                    })
                    .collect::<Result<Vec<_>, String>>()?;
            }
            self.record_model(callee, "exact integer population count");
            return Ok(Some(Value::Int {
                expression: terms.pop().ok_or("empty population count operand")?,
                bits: 32,
                signed: false,
            }));
        }
        if signature.inputs().len() == 1
            && signature.inputs()[0] == signature.output()
            && self.float_type(signature.output()).is_some()
            && self.tcx.is_intrinsic(callee, Symbol::intern("fabs"))
        {
            let [
                Value::Float {
                    expression,
                    bits,
                    raw_bits,
                },
            ] = values
            else {
                return Err("float absolute value requires a modeled float".to_owned());
            };
            self.record_model(callee, "IEEE floating-point absolute value");
            return Ok(Some(Value::Float {
                expression: self
                    .terms
                    .apply(Op::FpAbs, std::slice::from_ref(expression))?,
                bits: *bits,
                raw_bits: raw_bits
                    .as_ref()
                    .map(|raw| {
                        self.terms.apply(
                            Op::BvAnd,
                            &[
                                raw.clone(),
                                self.terms.bit_vector((1_u128 << (bits - 1)) - 1, *bits)?,
                            ],
                        )
                    })
                    .transpose()?,
            }));
        }
        let float_min = ["minimum_number_nsz_f32", "minimum_number_nsz_f64"]
            .iter()
            .any(|name| self.tcx.is_intrinsic(callee, Symbol::intern(name)));
        let float_max = ["maximum_number_nsz_f32", "maximum_number_nsz_f64"]
            .iter()
            .any(|name| self.tcx.is_intrinsic(callee, Symbol::intern(name)));
        if signature.inputs().len() == 2
            && signature
                .inputs()
                .iter()
                .all(|ty| *ty == signature.output())
            && self.float_type(signature.output()).is_some()
            && (float_min || float_max)
        {
            let [
                Value::Float {
                    expression: left,
                    bits,
                    ..
                },
                Value::Float {
                    expression: right, ..
                },
            ] = values
            else {
                return Err("float min/max requires modeled floats".to_owned());
            };
            let comparison = if float_min { Op::FpLt } else { Op::FpGt };
            let tie = self.fresh(Sort::Bool);
            self.record_model(
                callee,
                "IEEE min/max; numeric NaN fallback and either signed-zero tie",
            );
            let order = self
                .terms
                .apply(comparison, &[left.clone(), right.clone()])?;
            let ordered = self
                .terms
                .apply(Op::Ite, &[order, left.clone(), right.clone()])?;
            let tied = self
                .terms
                .apply(Op::Ite, &[tie, left.clone(), right.clone()])?;
            let equal = self
                .terms
                .apply(Op::FpEqual, &[left.clone(), right.clone()])?;
            let numeric = self.terms.apply(Op::Ite, &[equal, tied, ordered])?;
            let right_nan = self.terms.apply(Op::FpIsNaN, std::slice::from_ref(right))?;
            let right_result = self
                .terms
                .apply(Op::Ite, &[right_nan, left.clone(), numeric])?;
            let left_nan = self.terms.apply(Op::FpIsNaN, std::slice::from_ref(left))?;
            let result = Value::Float {
                expression: self
                    .terms
                    .apply(Op::Ite, &[left_nan, right.clone(), right_result])?,
                bits: *bits,
                raw_bits: None,
            };
            return Ok(Some(self.materialize_float(result)));
        }
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
        if self
            .tcx
            .lang_items()
            .get(LangItem::SliceLen)
            .is_some_and(|id| id.krate == callee.krate)
            && matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
            && self
                .float_type(
                    self.tcx
                        .type_of(parent)
                        .instantiate(self.tcx, args)
                        .skip_norm_wip(),
                )
                .is_some()
            && signature.inputs().len() == 1
        {
            if name == Symbol::intern("to_bits")
                && let Some(width) = self.float_type(signature.inputs()[0])
                && self.integer_type(signature.output()) == Some((width, false))
            {
                let [value] = values else {
                    return Err("float bit observation requires one float".to_owned());
                };
                self.record_model(
                    callee,
                    "exact floating-point storage bits, including NaN payloads",
                );
                return self.float_to_bits(value.clone(), width, false).map(Some);
            }
            if name == Symbol::intern("from_bits")
                && let Some(width) = self.float_type(signature.output())
                && self.integer_type(signature.inputs()[0]) == Some((width, false))
            {
                let [value] = values else {
                    return Err("float bit construction requires one integer".to_owned());
                };
                let (expression, bits, _) = value.integer()?;
                if bits != width {
                    return Err("float bit construction width mismatch".to_owned());
                }
                self.record_model(
                    callee,
                    "exact floating-point construction from storage bits",
                );
                return Ok(Some(symbolic::float_from_bits(
                    &self.terms,
                    expression,
                    bits,
                )));
            }
        }
        if self
            .tcx
            .lang_items()
            .get(LangItem::SliceLen)
            .is_some_and(|id| id.krate == callee.krate)
            && matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
            && self
                .float_type(
                    self.tcx
                        .type_of(parent)
                        .instantiate(self.tcx, args)
                        .skip_norm_wip(),
                )
                .is_some()
            && name == Symbol::intern("is_finite")
            && signature.inputs().len() == 1
            && self.float_type(signature.inputs()[0]).is_some()
            && signature.output().is_bool()
        {
            let [Value::Float { expression, .. }] = values else {
                return Err("float finiteness requires a modeled float".to_owned());
            };
            self.record_model(callee, "IEEE floating-point finiteness classification");
            let nan = self
                .terms
                .apply(Op::FpIsNaN, std::slice::from_ref(expression))?;
            let infinite = self
                .terms
                .apply(Op::FpIsInfinite, std::slice::from_ref(expression))?;
            return Ok(Some(Value::Bool(self.terms.apply(
                Op::And,
                &[symbolic::not(&nan), symbolic::not(&infinite)],
            )?)));
        }
        if self
            .tcx
            .lang_items()
            .get(LangItem::SliceLen)
            .is_some_and(|id| id.krate == callee.krate)
            && matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
            && self
                .float_type(
                    self.tcx
                        .type_of(parent)
                        .instantiate(self.tcx, args)
                        .skip_norm_wip(),
                )
                .is_some()
            && name == Symbol::intern("clamp")
            && signature.inputs().len() == 3
            && signature
                .inputs()
                .iter()
                .all(|ty| *ty == signature.output())
            && self.float_type(signature.output()).is_some()
        {
            let [
                Value::Float {
                    expression,
                    bits,
                    raw_bits,
                },
                Value::Float {
                    expression: min,
                    raw_bits: min_raw,
                    ..
                },
                Value::Float {
                    expression: max,
                    raw_bits: max_raw,
                    ..
                },
            ] = values
            else {
                return Err("float clamp requires three modeled floats".to_owned());
            };
            let safe = self.terms.apply(Op::FpLe, &[min.clone(), max.clone()])?;
            self.record_model(
                callee,
                "IEEE floating-point clamp with checked ordered bounds",
            );
            self.require(
                caller,
                span,
                &state.conditions,
                &safe,
                ObligationKind::PanicSafety,
                "float clamp bounds must be ordered and neither bound may be NaN".to_owned(),
            )?;
            state.conditions.push(safe);
            let below = self
                .terms
                .apply(Op::FpLt, &[expression.clone(), min.clone()])?;
            let above = self
                .terms
                .apply(Op::FpGt, &[expression.clone(), max.clone()])?;
            let upper = self
                .terms
                .apply(Op::Ite, &[above.clone(), max.clone(), expression.clone()])?;
            let result = self
                .terms
                .apply(Op::Ite, &[below.clone(), min.clone(), upper])?;
            let raw_bits = raw_bits
                .as_ref()
                .zip(min_raw.as_ref())
                .zip(max_raw.as_ref())
                .map(|((raw, min_raw), max_raw)| {
                    let upper = self
                        .terms
                        .apply(Op::Ite, &[above, max_raw.clone(), raw.clone()])?;
                    self.terms.apply(Op::Ite, &[below, min_raw.clone(), upper])
                })
                .transpose()?;
            return Ok(Some(Value::Float {
                expression: result,
                bits: *bits,
                raw_bits,
            }));
        }
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
                Value::Bytes { length, .. } => &**length,
                _ => return Err("range receiver is not modeled".to_owned()),
            };
            let safe =
                symbolic::binary(&self.terms, "le", end.clone(), length.clone())?.boolean()?;
            self.require(
                caller,
                span,
                &state.conditions,
                &safe,
                ObligationKind::PanicSafety,
                "byte prefix end must not exceed its receiver length".to_owned(),
            )?;
            state.conditions.push(safe);
            let result = if index_mut {
                let Some(Value::Reference {
                    allocation,
                    projection,
                    mutable: true,
                }) = raw_values.first()
                else {
                    return Err("mutable prefix needs tracked writable byte storage".to_owned());
                };
                let mut projection = projection.clone();
                projection.push(symbolic::MemoryProjection::Slice {
                    offset: Box::new(symbolic::integer(
                        &self.terms,
                        0,
                        u32::from(self.tcx.sess.target.pointer_width),
                        false,
                    )),
                    length: Box::new(end),
                });
                Value::Reference {
                    allocation: *allocation,
                    projection,
                    mutable: true,
                }
            } else {
                let Value::Bytes { data, .. } = receiver else {
                    return Err("shared prefix needs byte storage".to_owned());
                };
                Value::Bytes {
                    length: Box::new(end),
                    data: data.clone(),
                }
            };
            return Ok(Some(result));
        }
        if self
            .tcx
            .lang_items()
            .get(LangItem::From)
            .is_some_and(|id| trait_id == Some(id))
            && name == Symbol::intern("from")
            && signature.inputs().len() == 1
            && let Some((bits, signed)) = self.integer_type(signature.output())
            && let Some((source_bits, source_signed)) = self.integer_type(signature.inputs()[0])
            && ((!source_signed && (!signed || bits > source_bits)) || (source_signed && signed))
            && bits >= source_bits
        {
            let [value] = values else {
                return Err("conversion arity mismatch".to_owned());
            };
            self.record_model(callee, "lossless integer conversion");
            return Ok(Some(symbolic::cast(
                &self.terms,
                value.clone(),
                bits,
                signed,
            )?));
        }
        let core = self
            .tcx
            .lang_items()
            .get(LangItem::SliceLen)
            .map(|id| id.krate);
        if Some(callee.krate) == core
            && matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
            && matches!(
                name.as_str(),
                "from_le_bytes" | "from_be_bytes" | "from_ne_bytes"
            )
            && signature.inputs().len() == 1
            && let Some((bits, signed)) = self.integer_type(signature.output())
            && self
                .tcx
                .type_of(parent)
                .instantiate(self.tcx, args)
                .skip_norm_wip()
                == signature.output()
            && matches!(signature.inputs()[0].kind(), ty::Array(element, length)
                if *element == self.tcx.types.u8
                    && length.try_to_target_usize(self.tcx) == Some(u64::from(bits / 8)))
        {
            let [Value::Bytes { data, .. }] = values else {
                return Err("endian decoding requires a modeled byte array".to_owned());
            };
            let little = name == Symbol::intern("from_le_bytes")
                || (name == Symbol::intern("from_ne_bytes")
                    && self.tcx.data_layout.endian == rustc_abi::Endian::Little);
            let count = bits / 8;
            let pointer_bits = self.tcx.sess.target.pointer_width;
            let mut bytes = (0..count)
                .map(|index| {
                    self.terms.apply(
                        Op::Select,
                        &[
                            data.clone(),
                            self.terms
                                .bit_vector(u128::from(index), u32::from(pointer_bits))?,
                        ],
                    )
                })
                .collect::<Result<Vec<_>, String>>()?;
            if little {
                bytes.reverse();
            }
            let mut bytes = bytes.into_iter();
            let mut expression = bytes.next().ok_or("empty endian integer")?;
            for byte in bytes {
                expression = self.terms.apply(Op::Concat, &[expression, byte])?;
            }
            self.record_model(callee, "integer endian decoding; exact byte concatenation");
            return Ok(Some(Value::Int {
                expression,
                bits,
                signed,
            }));
        }
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
                .copy_bytes(caller, raw_values, values, state, span)
                .map(Some);
        }
        Ok(None)
    }

    pub(super) fn record_model(&mut self, callee: DefId, detail: &str) {
        let model = format!("{}: {detail}", self.tcx.def_path_str(callee));
        if !self.proof.models.contains(&model) {
            self.proof.models.push(model);
        }
    }

    fn copy_bytes(
        &mut self,
        caller: DefId,
        raw_values: &[Value],
        values: &[Value],
        state: &mut State,
        span: Span,
    ) -> Result<Value, String> {
        let [
            Value::Bytes { length, .. },
            Value::Bytes {
                length: source_length,
                data,
            },
        ] = values
        else {
            return Err("copy model requires byte source and destination storage".to_owned());
        };
        let Some(Value::Reference {
            allocation,
            projection,
            mutable: true,
        }) = raw_values.first()
        else {
            return Err("copy destination requires tracked writable byte storage".to_owned());
        };
        let safe = symbolic::binary(
            &self.terms,
            "eq",
            (**length).clone(),
            (**source_length).clone(),
        )?
        .boolean()?;
        self.require(
            caller,
            span,
            &state.conditions,
            &safe,
            ObligationKind::PanicSafety,
            "copy_from_slice requires equal source and destination lengths".to_owned(),
        )?;
        state.conditions.push(safe);
        let storage = state
            .memory
            .get_mut(*allocation)
            .and_then(Option::as_mut)
            .ok_or("copy destination allocation is dead")?;
        self.write_projection(
            storage,
            projection,
            Value::Bytes {
                length: length.clone(),
                data: data.clone(),
            },
            &state.conditions,
        )?;
        Ok(Value::Unit)
    }
}
