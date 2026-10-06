use super::*;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    pub(super) fn builtin(
        &mut self,
        caller_body: &Body<'tcx>,
        callee: DefId,
        args: ty::GenericArgsRef<'tcx>,
        values: &[Value],
        state: &mut State,
        span: Span,
    ) -> Result<Option<Value>, String> {
        let caller = caller_body.source.def_id();
        if self.tcx.def_kind(callee) == DefKind::Closure {
            return Ok(None);
        }
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
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx.fn_sig(callee).instantiate(self.tcx, args),
            )
            .map_err(|error| format!("builtin signature normalization failed: {error:?}"))?
            .skip_binder();
        if self.tcx.is_intrinsic(callee, Symbol::intern("ctpop"))
            && signature.inputs().len() == 1
            && self.integer_type(signature.inputs()[0]).is_some()
            && signature.output() == self.tcx.types.u32
        {
            let [value] = values else {
                return Err("population count requires one modeled integer".to_owned());
            };
            let (expression, bits, _) = value.integer()?;
            let mut terms: Vec<_> = (0..bits)
                .map(|bit| format!("((_ zero_extend 31) ((_ extract {bit} {bit}) {expression}))"))
                .collect();
            while terms.len() > 1 {
                terms = terms
                    .chunks(2)
                    .map(|pair| match pair {
                        [left, right] => format!("(bvadd {left} {right})"),
                        [only] => only.clone(),
                        _ => unreachable!("pairs have one or two elements"),
                    })
                    .collect();
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
            let [Value::Float { expression, bits }] = values else {
                return Err("float absolute value requires a modeled float".to_owned());
            };
            self.record_model(callee, "IEEE floating-point absolute value");
            return Ok(Some(Value::Float {
                expression: format!("(fp.abs {expression})"),
                bits: *bits,
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
                },
                Value::Float {
                    expression: right, ..
                },
            ] = values
            else {
                return Err("float min/max requires modeled floats".to_owned());
            };
            let comparison = if float_min { "fp.lt" } else { "fp.gt" };
            let tie = self.fresh("Bool");
            self.record_model(
                callee,
                "IEEE min/max; numeric NaN fallback and either signed-zero tie",
            );
            return Ok(Some(Value::Float {
                expression: format!(
                    "(ite (fp.isNaN {left}) {right} (ite (fp.isNaN {right}) {left} \
                     (ite (fp.eq {left} {right}) (ite {tie} {left} {right}) \
                     (ite ({comparison} {left} {right}) {left} {right}))))"
                ),
                bits: *bits,
            }));
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
        if self.tcx.crate_name(callee.krate) == Symbol::intern("core")
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
            return Ok(Some(Value::Bool(format!(
                "(and (not (fp.isNaN {expression})) (not (fp.isInfinite {expression})))"
            ))));
        }
        if self.tcx.crate_name(callee.krate) == Symbol::intern("core")
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
                Value::Float { expression, bits },
                Value::Float {
                    expression: min, ..
                },
                Value::Float {
                    expression: max, ..
                },
            ] = values
            else {
                return Err("float clamp requires three modeled floats".to_owned());
            };
            let safe = format!("(fp.leq {min} {max})");
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
            return Ok(Some(Value::Float {
                expression: format!(
                    "(ite (fp.lt {expression} {min}) {min} \
                     (ite (fp.gt {expression} {max}) {max} {expression}))"
                ),
                bits: *bits,
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
            return Ok(Some(symbolic::cast(value.clone(), bits, signed)?));
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
            let mut bytes: Vec<_> = (0..count)
                .map(|index| format!("(select {data} (_ bv{index} {pointer_bits}))"))
                .collect();
            if little {
                bytes.reverse();
            }
            let mut bytes = bytes.into_iter();
            let mut expression = bytes.next().ok_or("empty endian integer")?;
            for byte in bytes {
                expression = format!("(concat {expression} {byte})");
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
                .copy_bytes(caller, caller_body, values, state, span)
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
        } = self.local(state, *owner)?
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
        let value = Value::Bytes {
            length: owner_len,
            data,
        };
        if let Some(allocation) = state.addresses[*owner] {
            state.memory[allocation] = Some(value);
        } else {
            state.locals[*owner] = Some(value);
        }
        Ok(Value::Unit)
    }
}
