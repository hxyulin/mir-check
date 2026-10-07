use super::*;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    pub(super) fn atomic_shape(&self, ty: Ty<'tcx>) -> Option<Value> {
        let ty::Adt(def, args) = ty.kind() else {
            return None;
        };
        if self.tcx.get_diagnostic_item(Symbol::intern("Atomic")) != Some(def.did()) {
            return None;
        }
        let (bits, signed) = self.integer_type(args.type_at(0))?;
        Some(Value::Atomic { bits, signed })
    }

    pub(super) fn atomic_container(&self, ty: Ty<'tcx>, depth: usize) -> Option<Value> {
        if depth >= 8 {
            return None;
        }
        if let Some(value) = self.atomic_shape(ty) {
            return Some(value);
        }
        let ty::Adt(def, args) = ty.kind() else {
            return None;
        };
        if !def.is_struct() || def.non_enum_variant().fields.is_empty() {
            return None;
        }
        let fields = def
            .non_enum_variant()
            .fields
            .iter()
            .map(|field| self.atomic_container(field.ty(self.tcx, args).skip_norm_wip(), depth + 1))
            .collect::<Option<Vec<_>>>()?;
        self.constructed(ty, 0, fields).ok()
    }

    pub(super) fn cell_element(&self, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
        let ty::Adt(def, args) = ty.kind() else {
            return None;
        };
        if self.tcx.get_diagnostic_item(Symbol::intern("Cell")) != Some(def.did()) {
            return None;
        }
        let element = args.type_at(0);
        if element.is_bool()
            || self.integer_type(element).is_some()
            || self.float_type(element).is_some()
        {
            Some(element)
        } else {
            None
        }
    }

    pub(super) fn interior_call(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        state: &mut State,
        site: (DefId, Span),
    ) -> Result<Option<Value>, String> {
        let callee = instance.def_id();
        let parent = self.tcx.parent(callee);
        if !matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false }) {
            return Ok(None);
        }
        let receiver_ty = self
            .tcx
            .type_of(parent)
            .instantiate(self.tcx, instance.args)
            .skip_norm_wip();
        let name = self.tcx.item_name(callee);
        if let Some(value) = self.static_cell_get(receiver_ty, name.as_str(), values, state)? {
            self.record_model(callee, "UnsafeCell static storage address; no payload read");
            return Ok(Some(value));
        }
        if let Some(element) = self.cell_element(receiver_ty) {
            let result = match (name.as_str(), values) {
                ("new", [value]) => {
                    if state.memory.len() >= 512 {
                        return Err("memory allocation budget reached".to_owned());
                    }
                    let allocation = state.memory.len();
                    state.memory.push(Some(value.clone()));
                    Value::Cell { allocation }
                }
                ("get", [Value::Cell { allocation }]) => state
                    .memory
                    .get(*allocation)
                    .and_then(Option::as_ref)
                    .cloned()
                    .ok_or("Cell storage is unavailable")?,
                ("set" | "replace", [Value::Cell { allocation }, value]) => {
                    let old = state
                        .memory
                        .get(*allocation)
                        .and_then(Option::as_ref)
                        .cloned()
                        .ok_or("Cell storage is unavailable")?;
                    state.memory[*allocation] = Some(value.clone());
                    if name.as_str() == "replace" {
                        old
                    } else {
                        Value::Unit
                    }
                }
                _ => return Ok(None),
            };
            if element.needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized()) {
                return Err("Cell model cannot execute destructors".to_owned());
            }
            self.record_model(callee, "scalar Cell storage; aliases share updates");
            return Ok(Some(result));
        }
        let Some(Value::Atomic { bits, signed }) = self.atomic_shape(receiver_ty) else {
            return Ok(None);
        };
        if name.as_str() == "new" {
            self.record_model(
                callee,
                "integer atomic; conservative arbitrary state and interference",
            );
            return Ok(Some(Value::Atomic { bits, signed }));
        }
        if !matches!(
            name.as_str(),
            "load" | "store" | "fetch_add" | "fetch_sub" | "swap"
        ) {
            return Ok(None);
        }
        let Some(Value::Atomic {
            bits: actual_bits,
            signed: actual_signed,
        }) = values.first()
        else {
            return Err("atomic receiver is not modeled".to_owned());
        };
        if (*actual_bits, *actual_signed) != (bits, signed) {
            return Err("atomic receiver type mismatch".to_owned());
        }
        let order = values.last().ok_or("missing atomic ordering")?;
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx.fn_sig(callee).instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("atomic signature normalization failed: {error:?}"))?
            .skip_binder();
        let ordering_ty = signature
            .inputs()
            .last()
            .copied()
            .ok_or("atomic ordering argument type is unavailable")?;
        let allowed = match name.as_str() {
            "load" => &["Relaxed", "Acquire", "SeqCst"][..],
            "store" => &["Relaxed", "Release", "SeqCst"][..],
            _ => &["Relaxed", "Acquire", "Release", "AcqRel", "SeqCst"][..],
        };
        let safe = self.atomic_order(order, ordering_ty, allowed)?;
        self.require(
            site.0,
            site.1,
            &state.conditions,
            &safe,
            ObligationKind::PanicSafety,
            format!("{} requires a valid atomic ordering", name.as_str()),
        )?;
        state.conditions.push(safe);
        self.record_model(
            callee,
            "integer atomic; arbitrary current value, modular RMW and possible interference",
        );
        if name.as_str() == "store" {
            return Ok(Some(Value::Unit));
        }
        let value = Value::Int {
            expression: self.fresh(Sort::BitVec(bits)),
            bits,
            signed,
        };
        // No subsequent access is correlated with this operation: another actor may intervene.
        // fetch_add/sub return the old value; modular updates have no arithmetic panic condition.
        Ok(Some(value))
    }

    fn atomic_order(
        &self,
        order: &Value,
        ordering_ty: Ty<'tcx>,
        allowed: &[&str],
    ) -> Result<Term, String> {
        let ordering = self
            .tcx
            .get_diagnostic_item(Symbol::intern("Ordering"))
            .ok_or("atomic Ordering identity unavailable")?;
        let ty::Adt(def, _) = ordering_ty.kind() else {
            return Err("atomic ordering argument is not an enum".to_owned());
        };
        if def.did() != ordering || !def.is_enum() {
            return Err("atomic ordering argument type mismatch".to_owned());
        }
        let allowed = allowed
            .iter()
            .map(|name| Symbol::intern(name))
            .collect::<Vec<_>>();
        let allowed_tags = def
            .variants()
            .iter_enumerated()
            .filter(|(_, variant)| allowed.contains(&variant.name))
            .map(|(index, _)| {
                (
                    index.as_usize(),
                    def.discriminant_for_variant(self.tcx, index).val,
                )
            })
            .collect::<Vec<_>>();
        match order {
            Value::Adt {
                variant,
                discriminant,
                fields,
                ..
            } if def.variants().iter_enumerated().any(|(index, definition)| {
                index.as_usize() == *variant
                    && definition.fields.is_empty()
                    && fields.is_empty()
                    && def.discriminant_for_variant(self.tcx, index).val == *discriminant
            }) =>
            {
                Ok(self
                    .terms
                    .boolean(allowed_tags.iter().any(|(index, _)| index == variant)))
            }
            Value::Enum {
                discriminant,
                variants,
                ..
            } if variants.len() == def.variants().len()
                && def.variants().iter_enumerated().all(|(index, definition)| {
                    definition.fields.is_empty()
                        && matches!(&variants[index.as_usize()],
                                Value::Adt { variant, discriminant, fields, .. }
                                    if *variant == index.as_usize()
                                        && fields.is_empty()
                                        && *discriminant == def.discriminant_for_variant(
                                            self.tcx, index,
                                        ).val)
                }) =>
            {
                let (_, bits, signed) = discriminant.integer()?;
                let expressions = allowed_tags
                    .iter()
                    .map(|(_, tag)| {
                        symbolic::binary(
                            &self.terms,
                            "eq",
                            (**discriminant).clone(),
                            symbolic::integer(&self.terms, *tag, bits, signed),
                        )?
                        .boolean()
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                self.terms.apply(Op::Or, &expressions)
            }
            _ => Err("atomic ordering is not modeled".to_owned()),
        }
    }
}
