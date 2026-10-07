use super::*;
use rustc_abi::VariantIdx;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    pub(super) fn constructed(
        &self,
        ty: Ty<'tcx>,
        variant: usize,
        values: Vec<Value>,
    ) -> Result<Value, String> {
        let ty::Adt(def, _) = ty.kind() else {
            return Err("modeled constructor requires an ADT".to_owned());
        };
        let index = VariantIdx::from_usize(variant);
        let layout = def.variant(index);
        if layout.fields.len() != values.len() || def.is_union() {
            return Err("modeled constructor fields do not match the type".to_owned());
        }
        Ok(Value::Adt {
            name: self.tcx.def_path_str(def.did()),
            variant,
            is_option: self.tcx.lang_items().get(LangItem::Option) == Some(def.did()),
            discriminant: if def.is_enum() {
                def.discriminant_for_variant(self.tcx, index).val
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

    pub(super) fn call_instance(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: Vec<Value>,
        mut conditions: Vec<Term>,
        memory: Vec<Option<Value>>,
        stack: &[DefId],
        site: (DefId, Span),
    ) -> Result<Vec<Return>, String> {
        let callee = instance.def_id();
        let contracts = self.configured_contracts(instance)?;
        if contracts
            .iter()
            .any(|contract| matches!(contract.kind, ContractKind::Requires))
        {
            let body = self.instantiated_body(instance)?;
            let snapshots = self.snapshots(&values, &memory, &conditions)?;
            let bindings = self.configured_bindings(&body, &snapshots, instance)?;
            for contract in contracts
                .iter()
                .filter(|contract| matches!(contract.kind, ContractKind::Requires))
            {
                let text = contract
                    .predicate
                    .as_deref()
                    .ok_or("missing precondition")?;
                let safe = self.predicate(text, &bindings)?;
                self.require(
                    site.0,
                    site.1,
                    &conditions,
                    &safe,
                    ObligationKind::CallPrecondition,
                    format!("{} requires {text}", self.tcx.def_path_str(callee)),
                )?;
                conditions.push(safe);
            }
        }
        self.execute(instance, values, conditions, memory, stack)
    }

    pub(super) fn library_call(
        &mut self,
        instance: ty::Instance<'tcx>,
        values: &[Value],
        raw_values: &[Value],
        state: &State,
        stack: &[DefId],
        site: (DefId, Span),
    ) -> Result<Option<Vec<Return>>, String> {
        let callee = instance.def_id();
        if self.tcx.def_kind(callee) == DefKind::Closure {
            return Ok(None);
        }
        let core = self
            .tcx
            .lang_items()
            .get(LangItem::SliceLen)
            .map(|id| id.krate);
        if Some(callee.krate) != core {
            return Ok(None);
        }
        if let Some(results) =
            self.array_equality(instance, values, raw_values, state, stack, site)?
        {
            return Ok(Some(results));
        }
        if let Some(results) =
            self.slice_equality(instance, values, raw_values, state, stack, site)?
        {
            return Ok(Some(results));
        }
        if let Some(results) =
            self.slice_membership(instance, values, raw_values, state, stack, site)?
        {
            return Ok(Some(results));
        }
        let signature = self.call_signature(instance)?;
        if self.core_array_from_fn() == Some(callee) {
            return self.array_from_fn(instance, signature, raw_values, state, stack, site);
        }
        let inherent_slice = matches!(
            self.tcx.def_kind(self.tcx.parent(callee)),
            DefKind::Impl { of_trait: false }
        ) && matches!(self.tcx.type_of(self.tcx.parent(callee)).instantiate(self.tcx, instance.args)
                .skip_norm_wip().kind(), ty::Slice(element) if *element == self.tcx.types.u8);
        if inherent_slice && self.tcx.item_name(callee) == Symbol::intern("as_chunks_mut") {
            let [input] = signature.inputs() else {
                return Ok(None);
            };
            let ty::Ref(_, slice, mutability) = input.kind() else {
                return Ok(None);
            };
            if !mutability.is_mut()
                || !matches!(slice.kind(), ty::Slice(element) if *element == self.tcx.types.u8)
            {
                return Ok(None);
            }
            let ty::Tuple(outputs) = signature.output().kind() else {
                return Ok(None);
            };
            if outputs.len() != 2 {
                return Ok(None);
            }
            let ty::Ref(_, chunk_slice, mutability) = outputs[0].kind() else {
                return Ok(None);
            };
            let ty::Slice(chunk_array) = chunk_slice.kind() else {
                return Ok(None);
            };
            let ty::Array(element, width) = chunk_array.kind() else {
                return Ok(None);
            };
            let ty::Ref(_, remainder, remainder_mutability) = outputs[1].kind() else {
                return Ok(None);
            };
            if !mutability.is_mut()
                || *element != self.tcx.types.u8
                || !remainder_mutability.is_mut()
                || !matches!(remainder.kind(), ty::Slice(element) if *element == self.tcx.types.u8)
            {
                return Ok(None);
            }
            let width = width
                .try_to_target_usize(self.tcx)
                .ok_or("unknown chunk width")?;
            let safe = self.terms.boolean(width != 0);
            self.require(
                site.0,
                site.1,
                &state.conditions,
                &safe,
                ObligationKind::PanicSafety,
                "as_chunks_mut requires a nonzero chunk width".to_owned(),
            )?;
            if width == 0 {
                return Ok(Some(Vec::new()));
            }
            let [
                Value::Reference {
                    allocation,
                    projection,
                    mutable: true,
                },
            ] = raw_values
            else {
                return Err("as_chunks_mut requires tracked writable byte storage".to_owned());
            };
            let [Value::Bytes { length, .. }] = values else {
                return Err("as_chunks_mut requires modeled byte slice storage".to_owned());
            };
            let Some(symbolic::Constant::BitVec { value: length, .. }) =
                symbolic::constant(&length.integer()?.0)
            else {
                return Err("as_chunks_mut needs a fixed slice length".to_owned());
            };
            if length > 128 {
                return Err("as_chunks_mut exceeds the 128-byte view budget".to_owned());
            }
            let count = length / u128::from(width);
            let covered = count * u128::from(width);
            let bits = u32::from(self.tcx.sess.target.pointer_width);
            let mut chunk_projection = projection.clone();
            chunk_projection.push(symbolic::MemoryProjection::Chunks {
                width: width as usize,
                count: count as usize,
            });
            let mut remainder_projection = projection.clone();
            remainder_projection.push(symbolic::MemoryProjection::Slice {
                offset: Box::new(symbolic::integer(&self.terms, covered, bits, false)),
                length: Box::new(symbolic::integer(
                    &self.terms,
                    length - covered,
                    bits,
                    false,
                )),
            });
            self.record_model(
                callee,
                "fixed mutable byte chunks and remainder; shared allocation regions",
            );
            return Ok(Some(vec![Return {
                value: Value::Tuple(vec![
                    Value::Reference {
                        allocation: *allocation,
                        projection: chunk_projection,
                        mutable: true,
                    },
                    Value::Reference {
                        allocation: *allocation,
                        projection: remainder_projection,
                        mutable: true,
                    },
                ]),
                conditions: state.conditions.clone(),
                memory: state.memory.clone(),
            }]));
        }
        let parent = self.tcx.parent(callee);
        let name = self.tcx.item_name(callee);
        let trait_id = if matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: true }) {
            Some(
                self.tcx
                    .impl_trait_ref(parent)
                    .instantiate(self.tcx, instance.args)
                    .skip_norm_wip()
                    .def_id,
            )
        } else if self.tcx.def_kind(parent) == DefKind::Trait {
            Some(parent)
        } else {
            None
        };
        let conversion = (name == Symbol::intern("try_into")
            && self.tcx.get_diagnostic_item(Symbol::intern("TryInto")) == trait_id)
            || (name == Symbol::intern("try_from")
                && self.tcx.get_diagnostic_item(Symbol::intern("TryFrom")) == trait_id);
        if conversion && trait_id.is_some() && signature.inputs().len() == 1 {
            let ty::Ref(_, source, mutability) = signature.inputs()[0].kind() else {
                return Ok(None);
            };
            let ty::Adt(result, parameters) = signature.output().kind() else {
                return Ok(None);
            };
            if mutability.is_mut()
                || !matches!(source.kind(), ty::Slice(element) if *element == self.tcx.types.u8)
                || !self
                    .tcx
                    .lang_items()
                    .get(LangItem::ResultOk)
                    .is_some_and(|variant| self.tcx.parent(variant) == result.did())
            {
                return Ok(None);
            }
            let ty::Ref(_, target, mutability) = parameters.type_at(0).kind() else {
                return Ok(None);
            };
            let ty::Array(element, capacity) = target.kind() else {
                return Ok(None);
            };
            if mutability.is_mut() || *element != self.tcx.types.u8 {
                return Ok(None);
            }
            let count = capacity
                .try_to_target_usize(self.tcx)
                .ok_or("unknown conversion length")?;
            let [Value::Bytes { length, data }] = values else {
                return Err("slice conversion requires a modeled byte slice".to_owned());
            };
            let target_length = symbolic::integer(
                &self.terms,
                count as u128,
                u32::from(self.tcx.sess.target.pointer_width),
                false,
            );
            let equal =
                symbolic::binary(&self.terms, "eq", (**length).clone(), target_length.clone())?
                    .boolean()?;
            let error_ty = parameters.type_at(1);
            let ty::Adt(error, error_args) = error_ty.kind() else {
                return Err("slice conversion error type is not modeled".to_owned());
            };
            if !error.is_struct() {
                return Err("slice conversion error is not a struct".to_owned());
            }
            let error_fields = error
                .non_enum_variant()
                .fields
                .iter()
                .map(|field| {
                    if matches!(field.ty(self.tcx, error_args).skip_norm_wip().kind(),
                    ty::Tuple(fields) if fields.is_empty())
                    {
                        Ok(Value::Unit)
                    } else {
                        Err("slice conversion error has unsupported fields".to_owned())
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            self.record_model(
                callee,
                "shared byte slice to array; exact length controls Result",
            );
            let mut success = state.conditions.clone();
            success.push(equal.clone());
            let mut failure = state.conditions.clone();
            failure.push(symbolic::not(&equal));
            return Ok(Some(vec![
                Return {
                    value: self.constructed(
                        signature.output(),
                        0,
                        vec![Value::Bytes {
                            length: Box::new(target_length),
                            data: data.clone(),
                        }],
                    )?,
                    conditions: success,
                    memory: state.memory.clone(),
                },
                Return {
                    value: self.constructed(
                        signature.output(),
                        1,
                        vec![self.constructed(error_ty, 0, error_fields)?],
                    )?,
                    conditions: failure,
                    memory: state.memory.clone(),
                },
            ]));
        }
        if name == Symbol::intern("map")
            && matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
            && signature.inputs().len() == 2
            && matches!(signature.inputs()[0].kind(), ty::Array(..))
        {
            let (callable, has_environment) = match signature.inputs()[1].kind() {
                ty::Closure(id, args) => (ty::Instance::new_raw(*id, args), true),
                ty::FnDef(id, args) => {
                    (self.resolve_function_item(*id, args.skip_binder())?, false)
                }
                _ => return Ok(None),
            };
            let [array, closure] = raw_values else {
                return Err("array map arity mismatch".to_owned());
            };
            let elements = self.array_values(array, signature.inputs()[0])?;
            self.record_model(
                callee,
                "fixed array map; callable bodies executed in index order",
            );
            if signature.inputs()[1].needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized()) {
                return Err("array map callback destructors are not modeled".to_owned());
            }
            let (closure, memory) = if has_environment {
                self.callback_environment(closure, state)?
            } else {
                (closure.clone(), state.memory.clone())
            };
            let mut pending = vec![(Vec::new(), state.conditions.clone(), memory)];
            for element in elements {
                let mut next = Vec::new();
                for (collected, conditions, memory) in pending {
                    if !self.feasible(&conditions)? {
                        continue;
                    }
                    let arguments = if has_environment {
                        vec![closure.clone(), element.clone()]
                    } else {
                        vec![element.clone()]
                    };
                    let results =
                        self.call_instance(callable, arguments, conditions, memory, stack, site)?;
                    for result in results {
                        let mut collected = collected.clone();
                        collected.push(result.value);
                        next.push((collected, result.conditions, result.memory));
                    }
                }
                pending = next;
            }
            let mut results = Vec::new();
            for (elements, conditions, memory) in pending {
                let value = if matches!(signature.output().kind(), ty::Array(element, _)
                    if *element == self.tcx.types.u8)
                {
                    let bits = u32::from(self.tcx.sess.target.pointer_width);
                    let length = elements.len() as u128;
                    let mut data = self.terms.apply(
                        Op::ConstArray { index_bits: bits },
                        &[self.terms.bit_vector(0, 8)?],
                    )?;
                    for (index, element) in elements.into_iter().enumerate() {
                        let (expression, width, signed) = element.integer()?;
                        if width != 8 || signed {
                            return Err("mapped byte has the wrong type".to_owned());
                        }
                        data = self.terms.apply(
                            Op::Store,
                            &[
                                data.clone(),
                                self.terms.bit_vector(index as u128, bits)?,
                                expression.clone(),
                            ],
                        )?;
                    }
                    Value::Bytes {
                        length: Box::new(symbolic::integer(&self.terms, length, bits, false)),
                        data,
                    }
                } else {
                    Value::Elements(elements)
                };
                results.push(Return {
                    value,
                    conditions,
                    memory,
                });
            }
            if has_environment {
                self.retire_callback_environment(&closure, &mut results);
            }
            return Ok(Some(results));
        }
        Ok(None)
    }

    fn core_array_from_fn(&self) -> Option<DefId> {
        let iterator = self
            .tcx
            .get_diagnostic_item(Symbol::intern("ArrayIntoIter"))?;
        let module = self.tcx.parent(self.tcx.parent(iterator));
        if self.tcx.def_kind(module) != DefKind::Mod {
            return None;
        }
        self.tcx.module_children(module).iter().find_map(|child| {
            let id = child.res.opt_def_id()?;
            (child.ident.name == Symbol::intern("from_fn")
                && id.krate == iterator.krate
                && self.tcx.def_kind(id) == DefKind::Fn)
                .then_some(id)
        })
    }

    fn array_from_fn(
        &mut self,
        instance: ty::Instance<'tcx>,
        signature: ty::FnSig<'tcx>,
        values: &[Value],
        state: &State,
        stack: &[DefId],
        site: (DefId, Span),
    ) -> Result<Option<Vec<Return>>, String> {
        let ty::Array(element, count) = signature.output().kind() else {
            return Err("array from_fn return type is not an array".to_owned());
        };
        let [callback_ty] = signature.inputs() else {
            return Err("array from_fn callback arity mismatch".to_owned());
        };
        let [callback] = values else {
            return Err("array from_fn value arity mismatch".to_owned());
        };
        let count = count
            .try_to_target_usize(self.tcx)
            .ok_or("unknown generated array length")?;
        if count > 128 {
            return Err("generated array exceeds 128-element limit".to_owned());
        }
        let environment = ty::TypingEnv::fully_monomorphized();
        if element.needs_drop(self.tcx, environment)
            || callback_ty.needs_drop(self.tcx, environment)
        {
            return Err("array from_fn destructors are not modeled".to_owned());
        }
        let (callable, has_environment) = match callback_ty.kind() {
            ty::Closure(id, args) => (ty::Instance::new_raw(*id, args), true),
            ty::FnDef(id, args) => (self.resolve_function_item(*id, args.skip_binder())?, false),
            _ => return Err("array from_fn requires a concrete callback body".to_owned()),
        };
        let (callback, memory) = if has_environment {
            self.callback_environment(callback, state)?
        } else {
            (callback.clone(), state.memory.clone())
        };
        self.record_model(
            instance.def_id(),
            "fixed array from_fn; owned results and callable effects in ascending index order",
        );
        let mut pending = vec![(Vec::new(), 1_usize, state.conditions.clone(), memory)];
        for index in 0..count {
            let mut next = Vec::new();
            for (collected, size, conditions, memory) in pending {
                if !self.feasible(&conditions)? {
                    continue;
                }
                let index = symbolic::integer(
                    &self.terms,
                    u128::from(index),
                    u32::from(self.tcx.sess.target.pointer_width),
                    false,
                );
                let arguments = if has_environment {
                    vec![callback.clone(), index]
                } else {
                    vec![index]
                };
                for result in
                    self.call_instance(callable, arguments, conditions, memory, stack, site)?
                {
                    let size = result
                        .value
                        .owned_repeat_size()
                        .and_then(|element_size| size.checked_add(element_size))
                        .filter(|size| *size <= symbolic::MAX_REPEAT_VALUES)
                        .ok_or("generated array requires owned values within a 256-value budget")?;
                    let mut collected = collected.clone();
                    collected.push(result.value);
                    next.push((collected, size, result.conditions, result.memory));
                }
            }
            pending = next;
        }
        let mut results = Vec::new();
        for (elements, _, conditions, memory) in pending {
            let value = if *element == self.tcx.types.u8 {
                let bits = u32::from(self.tcx.sess.target.pointer_width);
                let mut data = self.terms.apply(
                    Op::ConstArray { index_bits: bits },
                    &[self.terms.bit_vector(0, 8)?],
                )?;
                for (index, element) in elements.into_iter().enumerate() {
                    let (expression, width, signed) = element.integer()?;
                    if width != 8 || signed {
                        return Err("generated byte has the wrong type".to_owned());
                    }
                    data = self.terms.apply(
                        Op::Store,
                        &[
                            data.clone(),
                            self.terms.bit_vector(index as u128, bits)?,
                            expression.clone(),
                        ],
                    )?;
                }
                Value::Bytes {
                    length: Box::new(symbolic::integer(
                        &self.terms,
                        u128::from(count),
                        bits,
                        false,
                    )),
                    data,
                }
            } else {
                Value::Elements(elements)
            };
            results.push(Return {
                value,
                conditions,
                memory,
            });
        }
        if has_environment {
            self.retire_callback_environment(&callback, &mut results);
        }
        Ok(Some(results))
    }

    fn array_values(&self, value: &Value, array_ty: Ty<'tcx>) -> Result<Vec<Value>, String> {
        let ty::Array(element, count) = array_ty.kind() else {
            return Err("expected a fixed array type".to_owned());
        };
        let count = count
            .try_to_target_usize(self.tcx)
            .ok_or("unknown array length")?;
        if count > 16 {
            return Err("array map element limit reached".to_owned());
        }
        match value {
            Value::Elements(elements) if elements.len() as u64 == count => Ok(elements.clone()),
            Value::Bytes { data, .. } if *element == self.tcx.types.u8 => {
                let bits = u32::from(self.tcx.sess.target.pointer_width);
                Ok((0..count)
                    .map(|index| {
                        Ok(Value::Int {
                            expression: self.terms.apply(
                                Op::Select,
                                &[data.clone(), self.terms.bit_vector(index as u128, bits)?],
                            )?,
                            bits: 8,
                            signed: false,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?)
            }
            _ => Err("array storage does not match its instantiated type".to_owned()),
        }
    }
}
