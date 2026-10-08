use super::*;
use mir_check::{FunctionContract, TrustedCall};

pub(super) fn canonical_name(tcx: TyCtxt<'_>, id: DefId) -> String {
    if id.is_local() {
        format!("{}::{}", tcx.crate_name(id.krate), tcx.def_path_str(id))
    } else {
        tcx.def_path_str(id)
    }
}

impl<'tcx> Engine<'tcx> {
    pub(super) fn core_endian_encoding(
        &mut self,
        callee: DefId,
        args: ty::GenericArgsRef<'tcx>,
        signature: ty::FnSig<'tcx>,
        values: &[Value],
    ) -> Result<Option<Value>, String> {
        let parent = self.tcx.parent(callee);
        let name = self.tcx.item_name(callee);
        if !self
            .tcx
            .lang_items()
            .get(LangItem::SliceLen)
            .is_some_and(|core| core.krate == callee.krate)
            || !matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
            || !matches!(name.as_str(), "to_le_bytes" | "to_be_bytes" | "to_ne_bytes")
            || signature.inputs().len() != 1
        {
            return Ok(None);
        }
        let Some((bits, signed)) = self.integer_type(signature.inputs()[0]) else {
            return Ok(None);
        };
        if self
            .tcx
            .type_of(parent)
            .instantiate(self.tcx, args)
            .skip_norm_wip()
            != signature.inputs()[0]
            || !matches!(signature.output().kind(), ty::Array(element, length)
                if *element == self.tcx.types.u8
                    && length.try_to_target_usize(self.tcx) == Some(u64::from(bits / 8)))
        {
            return Ok(None);
        }
        let [value] = values else {
            return Err("endian encoding requires one primitive integer".to_owned());
        };
        let (expression, actual_bits, actual_signed) = value.integer()?;
        if (bits, signed) != (actual_bits, actual_signed) {
            return Err("endian encoding integer type mismatch".to_owned());
        }
        let little = name.as_str() == "to_le_bytes"
            || (name.as_str() == "to_ne_bytes"
                && self.tcx.data_layout.endian == rustc_abi::Endian::Little);
        let count = bits / 8;
        let pointer_bits = u32::from(self.tcx.sess.target.pointer_width);
        let mut data = self.terms.apply(
            Op::ConstArray {
                index_bits: pointer_bits,
            },
            &[self.terms.bit_vector(0, 8)?],
        )?;
        for index in 0..count {
            let byte = if little { index } else { count - index - 1 };
            let low = byte * 8;
            let high = low + 7;
            let value = self
                .terms
                .apply(Op::Extract { high, low }, std::slice::from_ref(&expression))?;
            data = self.terms.apply(
                Op::Store,
                &[
                    data,
                    self.terms.bit_vector(u128::from(index), pointer_bits)?,
                    value,
                ],
            )?;
        }
        self.record_model(callee, "integer endian encoding; exact byte extraction");
        Ok(Some(Value::Bytes {
            length: Box::new(symbolic::integer(
                &self.terms,
                u128::from(count),
                pointer_bits,
                false,
            )),
            data,
        }))
    }

    pub(super) fn missing_body_reason(&self, id: DefId) -> String {
        let name = self.tcx.def_path_str(id);
        let explanation = if self.tcx.is_foreign_item(id) {
            "; foreign declarations have no Rust MIR body to retain"
        } else if self
            .tcx
            .lang_items()
            .get(LangItem::SliceLen)
            .is_some_and(|core| core.krate == id.krate)
        {
            "; the prebuilt core library omitted this body; cargo -Zbuild-std=core can retain it"
        } else {
            "; rebuild the dependency with MIR retention enabled"
        };
        format!("MIR body unavailable for {name}{explanation}")
    }

    pub(super) fn resolve_function_item(
        &self,
        id: DefId,
        args: ty::GenericArgsRef<'tcx>,
    ) -> Result<ty::Instance<'tcx>, String> {
        ty::Instance::try_resolve(self.tcx, ty::TypingEnv::fully_monomorphized(), id, args)
            .map_err(|error| format!("function-item resolution failed: {error:?}"))?
            .ok_or_else(|| format!("unresolved function item {}", self.tcx.def_path_str(id)))
    }

    pub(super) fn specification(
        &self,
        instance: ty::Instance<'tcx>,
    ) -> Result<Option<FunctionContract>, String> {
        let name = canonical_name(self.tcx, instance.def_id());
        let arguments = format!("{:?}", instance.args);
        let matching = self
            .config
            .functions
            .iter()
            .filter(|spec| {
                spec.function == name
                    && spec
                        .instance
                        .as_ref()
                        .is_none_or(|expected| *expected == arguments)
            })
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err("overlapping function/instance contract selectors".to_owned());
        }
        Ok(matching.first().map(|spec| (*spec).clone()))
    }

    pub(super) fn configured_contracts(
        &mut self,
        instance: ty::Instance<'tcx>,
    ) -> Result<Vec<Contract>, String> {
        let mut contracts = self.contracts(instance.def_id());
        if let Some(spec) = self.specification(instance)? {
            self.record_contract(&spec, instance)?;
            contracts.extend(spec.metadata());
        }
        Ok(contracts)
    }

    fn record_contract(
        &mut self,
        spec: &FunctionContract,
        instance: ty::Instance<'tcx>,
    ) -> Result<(), String> {
        let key = spec.selector();
        if self
            .resolved_contracts
            .insert(key.clone(), instance.def_id())
            .is_some_and(|id| id != instance.def_id())
        {
            return Err("contract selector resolved to ambiguous crate definitions".to_owned());
        }
        if !self.proof.matched_contracts.contains(&key) {
            self.proof.matched_contracts.push(key);
        }
        Ok(())
    }

    pub(super) fn configured_bindings(
        &self,
        body: &Body<'tcx>,
        values: &[Value],
        instance: ty::Instance<'tcx>,
    ) -> Result<BTreeMap<String, Value>, String> {
        let mut bindings = self.bindings(body, values)?;
        if let Some(spec) = self.specification(instance)?
            && !spec.arguments.is_empty()
        {
            if spec.arguments.len() != values.len() {
                return Err("contract argument count does not match the function".to_owned());
            }
            for (index, name) in spec.arguments.iter().enumerate() {
                if body.var_debug_info.iter().any(|debug| {
                    debug.name.as_str() == name
                        && matches!(debug.value, VarDebugInfoContents::Place(place)
                            if place.projection.is_empty()
                                && place.local.as_usize() != index + 1)
                }) {
                    return Err("argument alias conflicts with a MIR parameter name".to_owned());
                }
                bindings.insert(name.clone(), values[index].clone());
            }
        }
        Ok(bindings)
    }

    fn invalidate_local_atomics(value: &Value, memory: &mut [Option<Value>]) -> Result<(), String> {
        match value {
            Value::LocalAtomic {
                allocation,
                bits,
                signed,
            } => {
                let storage = memory
                    .get_mut(*allocation)
                    .ok_or("local atomic effect points outside storage")?;
                // Dead allocations remain dead: interference cannot resurrect their lifetime.
                if storage.is_some() {
                    *storage = Some(Value::Atomic {
                        bits: *bits,
                        signed: *signed,
                    });
                }
            }
            Value::Adt { fields, .. } => {
                for (_, field) in fields {
                    Self::invalidate_local_atomics(field, memory)?;
                }
            }
            Value::Enum { variants, .. } | Value::Tuple(variants) | Value::Elements(variants) => {
                for variant in variants {
                    Self::invalidate_local_atomics(variant, memory)?;
                }
            }
            Value::SliceIterator { source, .. }
            | Value::MetadataPointer(source)
            | Value::DebugReference { source, .. } => {
                Self::invalidate_local_atomics(source, memory)?
            }
            Value::Input(_)
            | Value::Bool(_)
            | Value::Int { .. }
            | Value::Float { .. }
            | Value::Bytes { .. }
            | Value::Cell { .. }
            | Value::Atomic { .. }
            | Value::Reference { .. }
            | Value::RawPointer { .. }
            | Value::TrackedPointer { .. }
            | Value::StaticSlice { .. }
            | Value::StaticView { .. }
            | Value::StaticText
            | Value::FormatArguments
            | Value::Uninitialized
            | Value::FunctionPointer { .. }
            | Value::Function
            | Value::Unit => {}
        }
        Ok(())
    }

    pub(super) fn invalidate_published_atomics(
        state: &State,
        memory: &mut Memory,
    ) -> Result<(), String> {
        memory.invalidate_startup();
        for value in state.locals.iter().chain(state.memory.iter()).flatten() {
            Self::invalidate_local_atomics(value, memory)?;
        }
        Ok(())
    }

    pub(super) fn trusted_call(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        state: &State,
        site: (DefId, Span),
    ) -> Result<Option<Vec<Return>>, String> {
        let Some(spec) = self.specification(instance)? else {
            return Ok(None);
        };
        if !matches!(
            self.tcx.def_kind(instance.def_id()),
            DefKind::Fn | DefKind::AssocFn
        ) {
            return Err("external summaries require ordinary function definitions".to_owned());
        }
        if !spec.trusted {
            return Ok(None);
        }
        self.record_contract(&spec, instance)?;
        if spec.instance.is_none()
            && instance.args.iter().any(|arg| {
                matches!(
                    arg.kind(),
                    ty::GenericArgKind::Type(_) | ty::GenericArgKind::Const(_)
                )
            })
        {
            return Err("trusted generic summaries require an exact instance selector".to_owned());
        }
        let signature = self.call_signature(instance)?;
        if signature.inputs().len() != values.len() {
            return Err("summary call arity mismatch".to_owned());
        }
        let snapshots = self.snapshots(values, &state.memory, &state.conditions)?;
        let body = self.instantiated_body(instance).ok();
        let mut bindings = if let Some(body) = &body {
            self.configured_bindings(body, &snapshots, instance)?
        } else {
            BTreeMap::new()
        };
        let names = if spec.arguments.is_empty() {
            (0..values.len())
                .map(|index| format!("arg{index}"))
                .collect::<Vec<_>>()
        } else {
            spec.arguments.clone()
        };
        if names.len() != values.len() {
            return Err("summary argument count mismatch".to_owned());
        }
        for (name, value) in names.iter().zip(&snapshots) {
            if bindings.contains_key(name) && body.is_some() && spec.arguments.is_empty() {
                return Err(
                    "positional summary name conflicts with MIR metadata; declare argument aliases"
                        .to_owned(),
                );
            }
            bindings.insert(name.clone(), value.clone());
        }
        let mut conditions = state.conditions.clone();
        for contract in self.configured_contracts(instance)? {
            if matches!(contract.kind, ContractKind::Requires) {
                let predicate = contract.predicate.ok_or("missing summary precondition")?;
                let safe = self.predicate(&predicate, &bindings)?;
                self.require(
                    site.0,
                    site.1,
                    &conditions,
                    &safe,
                    ObligationKind::CallPrecondition,
                    format!("{} requires {predicate}", spec.function),
                )?;
                conditions.push(safe);
            }
        }
        if !self.feasible(&conditions)? {
            return Ok(Some(Vec::new()));
        }
        let mut memory = state.memory.clone();
        if let Some(modifies) = &spec.modifies {
            for name in modifies {
                let index = names
                    .iter()
                    .position(|candidate| candidate == name)
                    .ok_or("summary effect argument unavailable")?;
                let ty::Ref(_, element, mutability) = signature.inputs()[index].kind() else {
                    return Err("summary effects need reference arguments".to_owned());
                };
                match &values[index] {
                    Value::Reference {
                        allocation,
                        projection,
                        mutable: true,
                    } if mutability.is_mut() => {
                        if !matches!(element.kind(), ty::Slice(ty) if *ty == self.tcx.types.u8)
                            && !self.summary_shape(*element, 0)
                        {
                            return Err(
                                "summary effect shape contains unsupported aliases".to_owned()
                            );
                        }
                        let mut replacement = if matches!(element.kind(), ty::Slice(_)) {
                            self.byte_input(instance.def_id(), *element, &mut conditions)?
                        } else {
                            self.argument(instance.def_id(), *element, &mut conditions)?
                        };
                        if matches!(element.kind(), ty::Slice(_))
                            && let Value::Bytes { length, .. } = &mut replacement
                        {
                            let Value::Bytes { length: old, .. } =
                                self.reference_value(&values[index], &memory, &conditions)?
                            else {
                                return Err("summary slice effect needs byte storage".to_owned());
                            };
                            *length = old;
                        }
                        let storage = memory
                            .get_mut(*allocation)
                            .and_then(Option::as_mut)
                            .ok_or("summary effect points to unavailable storage")?;
                        self.write_projection(storage, projection, replacement, &conditions)?;
                    }
                    Value::LocalAtomic {
                        allocation,
                        bits,
                        signed,
                    } if self.atomic_shape(*element).is_some() => {
                        let storage = memory
                            .get_mut(*allocation)
                            .and_then(Option::as_mut)
                            .ok_or("summary atomic effect points to unavailable storage")?;
                        *storage = Value::Atomic {
                            bits: *bits,
                            signed: *signed,
                        };
                    }
                    Value::Cell { allocation } if self.cell_element(*element).is_some() => {
                        let inner = self
                            .cell_element(*element)
                            .ok_or("unsupported Cell summary effect")?;
                        memory[*allocation] =
                            Some(self.argument(instance.def_id(), inner, &mut conditions)?);
                    }
                    _ => return Err("summary effect reference is unsupported".to_owned()),
                }
            }
        } else {
            let roots = self
                .static_roots
                .and_then(|allocation| memory[allocation].clone());
            memory.fill(None);
            if let Some(allocation) = self.static_roots {
                memory[allocation] = roots;
            }
        }
        // A trusted call may publish an atomic without changing its value during the call.
        // Even an empty modifies list cannot certify absence of subsequent interference.
        Self::invalidate_published_atomics(state, &mut memory)?;
        let output = signature.output();
        let (result, result_binding) = if let Some(name) = &spec.returns_alias {
            let index = names
                .iter()
                .position(|candidate| candidate == name)
                .ok_or("summary return alias argument unavailable")?;
            let same_pointee = matches!(
                (signature.inputs()[index].kind(), output.kind()),
                (ty::Ref(_, input, input_mut), ty::Ref(_, output, output_mut))
                    if input == output && input_mut.is_mut() && output_mut.is_mut()
            );
            if !same_pointee || !matches!(values[index], Value::Reference { mutable: true, .. }) {
                return Err(
                    "summary return alias needs identical tracked mutable reference types"
                        .to_owned(),
                );
            }
            self.validate_tracked_value(&values[index], state)?;
            let binding = self.reference_value(&values[index], &memory, &conditions)?;
            (values[index].clone(), binding)
        } else {
            if output.needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized())
                || output.is_ref()
                || output.is_raw_ptr()
            {
                return Err(
                    "summary return type needs an explicit ownership/alias model".to_owned(),
                );
            }
            if !self.summary_shape(output, 0) {
                return Err(
                    "summary return shape contains unsupported ownership or aliases".to_owned(),
                );
            }
            let result = self.argument(instance.def_id(), output, &mut conditions)?;
            (result.clone(), result)
        };
        bindings.insert("result".to_owned(), result_binding);
        if spec.ensures.iter().try_fold(false, |found, predicate| {
            Ok::<_, String>(found | contracts::uses_post_state(predicate)?)
        })? {
            let finals = self.snapshots(values, &memory, &conditions)?;
            for (name, value) in names.iter().zip(finals) {
                bindings.insert(format!("final_{name}"), value);
            }
        }
        for predicate in &spec.ensures {
            conditions.push(self.predicate(predicate, &bindings)?);
        }
        if !self.feasible(&conditions)? {
            return Err("trusted summary has inconsistent return/state constraints".to_owned());
        }
        self.proof.trusted_calls.push(TrustedCall {
            contract: spec,
            instance: format!("{:?}", instance.args),
            crate_hash: if instance.def_id().is_local() && !self.tcx.needs_hir_hash() {
                String::new()
            } else {
                format!("{:?}", self.tcx.crate_hash(instance.def_id().krate))
            },
            source: crate::source(self.tcx, site.1),
        });
        Ok(Some(vec![Return {
            value: result,
            conditions,
            memory,
        }]))
    }
    fn summary_shape(&self, ty: Ty<'tcx>, depth: usize) -> bool {
        if depth >= 8 {
            return false;
        }
        if ty.is_bool() || self.integer_type(ty).is_some() || self.float_type(ty).is_some() {
            return true;
        }
        match ty.kind() {
            ty::Tuple(fields) => fields
                .iter()
                .all(|field| self.summary_shape(field, depth + 1)),
            ty::Array(element, _) => self.summary_shape(*element, depth + 1),
            ty::Adt(def, args)
                if (def.is_struct() || def.is_enum())
                    && self.tcx.lang_items().get(LangItem::UnsafeCell) != Some(def.did()) =>
            {
                def.variants().iter().all(|variant| {
                    variant.fields.iter().all(|field| {
                        self.summary_shape(field.ty(self.tcx, args).skip_norm_wip(), depth + 1)
                    })
                })
            }
            _ => false,
        }
    }
}
