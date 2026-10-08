use super::*;

impl<'tcx> Engine<'tcx> {
    pub(super) fn async_factory_root(
        &mut self,
        factory: ty::Instance<'tcx>,
        arguments: Vec<Value>,
        conditions: Vec<Term>,
        memory: Memory,
    ) -> Result<(), String> {
        let body = self.instantiated_body(factory)?;
        let coroutine_ty = body.return_ty();
        let ty::Coroutine(coroutine, args) = *coroutine_ty.kind() else {
            return Err(
                "async entry factory has no concrete compiler coroutine return type".into(),
            );
        };
        let poll = ty::Instance::new_raw(coroutine, args);
        let poll_body = self.instantiated_body(poll)?;
        if poll_body.arg_count != 2 {
            return Err(
                "async entry requires a lowered Future::poll body with two arguments".into(),
            );
        }
        let pin_ty = poll_body.local_decls[rustc_middle::mir::Local::from_usize(1)].ty;
        let context_ty = poll_body.local_decls[rustc_middle::mir::Local::from_usize(2)].ty;
        let ty::Adt(pin, pin_args) = pin_ty.kind() else {
            return Err("async entry poll receiver is not compiler Pin".into());
        };
        if self.tcx.lang_items().get(LangItem::Pin) != Some(pin.did())
            || !matches!(pin_args.type_at(0).kind(), ty::Ref(_, inner, mutable)
                if *inner == coroutine_ty && mutable.is_mut())
            || !matches!(context_ty.kind(), ty::Ref(_, inner, mutable)
                if mutable.is_mut() && self.is_task_context(*inner))
        {
            return Err("async entry poll signature lacks its typed future or Context".into());
        }
        let ty::Adt(poll_enum, _) = poll_body.return_ty().kind() else {
            return Err("async entry poll result is not compiler Poll".into());
        };
        if self.tcx.lang_items().get(LangItem::Poll) != Some(poll_enum.did()) {
            return Err("async entry poll result is not compiler Poll".into());
        }
        let ready = poll_enum
            .variants()
            .iter_enumerated()
            .find(|(_, variant)| {
                self.tcx.lang_items().get(LangItem::PollReady) == Some(variant.def_id)
            })
            .map(|(index, _)| index.as_usize())
            .ok_or("compiler Poll::Ready variant is unavailable")?;
        let pending = poll_enum
            .variants()
            .iter_enumerated()
            .find(|(_, variant)| {
                self.tcx.lang_items().get(LangItem::PollPending) == Some(variant.def_id)
            })
            .map(|(index, _)| index.as_usize())
            .ok_or("compiler Poll::Pending variant is unavailable")?;
        self.record_model(
            factory.def_id(),
            "async entry: fresh factory inputs; poll Pending states until Ready; valid opaque \
             Context; executor, cancellation and returned-value drops are outside this root",
        );
        let mut queue = std::collections::VecDeque::new();
        for mut constructed in self.execute(factory, arguments, conditions, memory, &[])? {
            if !matches!(&constructed.value, Value::Adt { name, variant: 0, .. }
                if *name == coroutines::coroutine_name(coroutine, args))
            {
                return Err("async entry factory did not return a fresh constructed future".into());
            }
            if constructed.memory.len() + 2 > MAX_INPUT_VALUES {
                return Err("async entry exceeds the tracked allocation budget".into());
            }
            let future_allocation = constructed.memory.len();
            constructed.memory.push(Some(constructed.value));
            let context_allocation = constructed.memory.len();
            constructed.memory.push(Some(Value::Adt {
                name: "opaque task context".into(),
                variant: 0,
                is_option: false,
                discriminant: 0,
                fields: Vec::new().into(),
            }));
            let receiver = self.constructed(
                pin_ty,
                0,
                vec![Value::Reference {
                    allocation: future_allocation,
                    projection: Vec::new(),
                    mutable: true,
                }],
            )?;
            let context = Value::Reference {
                allocation: context_allocation,
                projection: Vec::new(),
                mutable: true,
            };
            queue.push_back((
                receiver,
                context,
                constructed.conditions,
                constructed.memory,
            ));
        }
        while let Some((receiver, context, conditions, memory)) = queue.pop_front() {
            let outcomes = self.call_instance(
                poll,
                vec![receiver.clone(), context.clone()],
                conditions,
                memory,
                &[factory.def_id()],
                (factory.def_id(), self.tcx.def_span(factory.def_id())),
            )?;
            for outcome in outcomes {
                let Value::Adt { name, variant, .. } = outcome.value else {
                    return Err("async entry poll did not return a modeled Poll variant".into());
                };
                if name != self.tcx.def_path_str(poll_enum.did()) {
                    return Err("async entry poll result has a different enum identity".into());
                }
                if variant == pending {
                    queue.push_back((
                        receiver.clone(),
                        context.clone(),
                        outcome.conditions,
                        outcome.memory,
                    ));
                } else if variant != ready {
                    return Err("async entry poll returned an unsupported Poll variant".into());
                }
            }
        }
        Ok(())
    }
}
