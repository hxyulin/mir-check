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
        mut conditions: Vec<String>,
        memory: Vec<Option<Value>>,
        stack: &[DefId],
        site: (DefId, Span),
    ) -> Result<Vec<Return>, String> {
        let callee = instance.def_id();
        let body = self.instantiated_body(instance)?;
        let snapshots = self.snapshots(&values, &memory, &conditions)?;
        let bindings = self.configured_bindings(&body, &snapshots, instance)?;
        for contract in self.configured_contracts(instance)? {
            if matches!(contract.kind, ContractKind::Requires) {
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
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx.fn_sig(callee).instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("call signature normalization failed: {error:?}"))?
            .skip_binder();
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
                count as u128,
                u32::from(self.tcx.sess.target.pointer_width),
                false,
            );
            let equal =
                symbolic::binary("eq", (**length).clone(), target_length.clone())?.boolean()?;
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
                ty::FnDef(id, args) => (ty::Instance::new_raw(*id, args.skip_binder()), false),
                _ => return Ok(None),
            };
            let [array, closure] = values else {
                return Err("array map arity mismatch".to_owned());
            };
            let elements = self.array_values(array, signature.inputs()[0])?;
            self.record_model(
                callee,
                "fixed array map; callable bodies executed in index order",
            );
            let mut pending = vec![(Vec::new(), state.conditions.clone(), state.memory.clone())];
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
                    let bits = self.tcx.sess.target.pointer_width;
                    let length = elements.len() as u128;
                    let mut data =
                        format!("((as const (Array (_ BitVec {bits}) (_ BitVec 8))) (_ bv0 8))");
                    for (index, element) in elements.into_iter().enumerate() {
                        let (expression, width, signed) = element.integer()?;
                        if width != 8 || signed {
                            return Err("mapped byte has the wrong type".to_owned());
                        }
                        data = format!("(store {data} (_ bv{index} {bits}) {expression})");
                    }
                    Value::Bytes {
                        length: Box::new(symbolic::integer(length, u32::from(bits), false)),
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
            return Ok(Some(results));
        }
        Ok(None)
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
                let bits = self.tcx.sess.target.pointer_width;
                Ok((0..count)
                    .map(|index| Value::Int {
                        expression: format!("(select {data} (_ bv{index} {bits}))"),
                        bits: 8,
                        signed: false,
                    })
                    .collect())
            }
            _ => Err("array storage does not match its instantiated type".to_owned()),
        }
    }
}
