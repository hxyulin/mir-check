use super::*;
use rustc_abi::FieldIdx;
use rustc_index::IndexVec;
use rustc_middle::mir::visit::{MutVisitor, PlaceContext};
use rustc_middle::mir::{Local, LocalDecl, Location};

fn coroutine_name(id: DefId, args: ty::GenericArgsRef<'_>) -> String {
    format!("coroutine {id:?} {args:?}")
}

impl<'tcx> Engine<'tcx> {
    pub(super) fn construct_coroutine(
        &mut self,
        id: DefId,
        args: ty::GenericArgsRef<'tcx>,
        captures: Vec<Value>,
    ) -> Result<Value, String> {
        if captures.len() != args.as_coroutine().upvar_tys().len() {
            return Err("coroutine captures do not match its compiler type".into());
        }
        let layout = self
            .tcx
            .coroutine_layout(id, args)
            .map_err(|error| format!("coroutine layout is unavailable: {error:?}"))?;
        if layout.variant_fields.len() > 64
            || captures.len() + layout.field_tys.len() > MAX_INPUT_VALUES
        {
            return Err("coroutine layout exceeds the state model budget".into());
        }
        let mut fields: Vec<_> = captures
            .into_iter()
            .enumerate()
            .map(|(index, value)| (index.to_string(), value))
            .collect();
        fields.extend(
            (0..layout.field_tys.len())
                .map(|index| (format!("saved_{index}"), Value::Uninitialized)),
        );
        self.record_model(
            id,
            "coroutine construction; execution is checked only when polled",
        );
        Ok(Value::Adt {
            name: coroutine_name(id, args),
            variant: 0,
            is_option: false,
            discriminant: 0,
            fields,
        })
    }

    pub(super) fn flatten_coroutine_places(&self, body: &mut Body<'tcx>) -> Result<(), String> {
        if !body.local_decls.iter().any(|local| {
            local
                .ty
                .walk()
                .any(|arg| arg.as_type().is_some_and(|ty| ty.is_coroutine()))
        }) {
            return Ok(());
        }
        let mut visitor = CoroutinePlaces {
            tcx: self.tcx,
            locals: body.local_decls.clone(),
            error: None,
        };
        visitor.visit_body(body);
        visitor.error.map_or(Ok(()), Err)
    }

    pub(super) fn set_coroutine_state(
        &self,
        body: &Body<'tcx>,
        state: &mut State,
        place: Place<'tcx>,
        index: usize,
    ) -> Result<(), String> {
        let ty::Coroutine(id, args) = place.ty(&body.local_decls, self.tcx).ty.kind() else {
            return Err("setting a non-coroutine discriminant remains unsupported".into());
        };
        let layout = self
            .tcx
            .coroutine_layout(*id, args)
            .map_err(|error| format!("coroutine layout is unavailable: {error:?}"))?;
        if index >= layout.variant_fields.len() {
            return Err("coroutine state index exceeds its compiler layout".into());
        }
        let mut value = self.place(state, place)?;
        let Value::Adt {
            name,
            variant,
            discriminant,
            ..
        } = &mut value
        else {
            return Err("coroutine state needs a constructed tracked future".into());
        };
        if *name != coroutine_name(*id, args) {
            return Err("coroutine state storage identity does not match its type".into());
        }
        *variant = index;
        *discriminant = index as u128;
        self.write(state, place, value)
    }

    pub(super) fn is_task_context(&self, ty: Ty<'tcx>) -> bool {
        matches!(ty.kind(), ty::Adt(def, _) if
            self.tcx.lang_items().get(LangItem::Context) == Some(def.did()))
    }

    pub(super) fn pin_mutable_deref(
        &self,
        instance: ty::Instance<'tcx>,
        signature: ty::FnSig<'tcx>,
        values: &[Value],
        state: &State,
    ) -> Result<Option<Value>, String> {
        let callee = instance.def_id();
        let parent = self.tcx.parent(callee);
        if self.tcx.item_name(callee) != rustc_span::Symbol::intern("deref_mut")
            || !matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: true })
            || self.tcx.lang_items().get(LangItem::DerefMut)
                != Some(
                    self.tcx
                        .impl_trait_ref(parent)
                        .instantiate(self.tcx, instance.args)
                        .skip_norm_wip()
                        .def_id,
                )
        {
            return Ok(None);
        }
        let [receiver] = signature.inputs() else {
            return Ok(None);
        };
        let ty::Ref(_, pin, mutable) = receiver.kind() else {
            return Ok(None);
        };
        let ty::Adt(def, args) = pin.kind() else {
            return Ok(None);
        };
        if !mutable.is_mut()
            || self.tcx.lang_items().get(LangItem::Pin) != Some(def.did())
            || callee.krate != def.did().krate
        {
            return Ok(None);
        }
        let ty::Ref(_, pointee, mutable) = args.type_at(0).kind() else {
            return Ok(None);
        };
        if !mutable.is_mut()
            || !matches!(signature.output().kind(), ty::Ref(_, target, mutable)
                if mutable.is_mut() && target == pointee)
        {
            return Ok(None);
        }
        let [reference] = values else {
            return Err("Pin mutable dereference arity mismatch".into());
        };
        let pin = self.reference_value(reference, &state.memory, &state.conditions)?;
        let pointer = pin.field("pointer")?;
        if !matches!(pointer, Value::Reference { mutable: true, .. }) {
            return Err("Pin mutable dereference needs a tracked mutable pointer".into());
        }
        Ok(Some(pointer))
    }

    pub(super) fn task_context_constructor(
        &mut self,
        instance: ty::Instance<'tcx>,
        signature: ty::FnSig<'tcx>,
        values: &[Value],
    ) -> Result<Option<Value>, String> {
        let callee = instance.def_id();
        let parent = self.tcx.parent(callee);
        if !matches!(self.tcx.def_kind(parent), DefKind::Impl { of_trait: false }) {
            return Ok(None);
        }
        let waker = self
            .tcx
            .get_diagnostic_item(rustc_span::Symbol::intern("Waker"));
        let is_waker =
            |ty: Ty<'tcx>| matches!(ty.kind(), ty::Adt(def, _) if Some(def.did()) == waker);
        let owner = self
            .tcx
            .type_of(parent)
            .instantiate(self.tcx, instance.args)
            .skip_norm_wip();
        if is_waker(owner)
            && Some(callee.krate) == waker.map(|id| id.krate)
            && self.tcx.item_name(callee) == rustc_span::Symbol::intern("noop")
            && signature.inputs().is_empty()
            && matches!(signature.output().kind(), ty::Ref(_, inner, mutable)
                if !mutable.is_mut() && is_waker(*inner))
        {
            if !values.is_empty() {
                return Err("noop waker arity mismatch".into());
            }
            self.record_model(callee, "core noop waker; opaque shared static waker");
            return Ok(Some(Value::Adt {
                name: "opaque noop waker".into(),
                variant: 0,
                is_option: false,
                discriminant: 0,
                fields: Vec::new(),
            }));
        }
        if self.is_task_context(owner)
            && Some(callee.krate)
                == self
                    .tcx
                    .lang_items()
                    .get(LangItem::Context)
                    .map(|id| id.krate)
            && self.tcx.item_name(callee) == rustc_span::Symbol::intern("from_waker")
            && self.is_task_context(signature.output())
            && matches!(signature.inputs(), [input] if matches!(input.kind(),
                ty::Ref(_, inner, mutable) if !mutable.is_mut() && is_waker(*inner)))
        {
            if !matches!(values, [Value::Adt { name, .. }] if name == "opaque noop waker") {
                return Err("task context construction needs a modeled noop waker".into());
            }
            self.record_model(
                callee,
                "core task context from noop waker; opaque valid context",
            );
            return Ok(Some(Value::Adt {
                name: "opaque task context".into(),
                variant: 0,
                is_option: false,
                discriminant: 0,
                fields: Vec::new(),
            }));
        }
        Ok(None)
    }

    pub(super) fn context_transmute(
        &self,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
        value: &Value,
    ) -> Result<Option<Value>, String> {
        let context_ref = |ty: Ty<'tcx>| {
            matches!(ty.kind(), ty::Ref(_, inner, mutable)
            if mutable.is_mut() && self.is_task_context(*inner))
        };
        let context_pointer = |ty: Ty<'tcx>| {
            matches!(ty.kind(), ty::Adt(def, args)
            if self.tcx.lang_items().get(LangItem::NonNull) == Some(def.did())
                && self.is_task_context(args.type_at(0)))
        };
        if context_ref(source) && context_pointer(target) {
            if !matches!(value, Value::Reference { mutable: true, .. }) {
                return Err("coroutine context adapter needs a tracked mutable reference".into());
            }
            return self.constructed(target, 0, vec![value.clone()]).map(Some);
        }
        if context_pointer(source) && context_ref(target) {
            let reference = value.field("pointer")?;
            if !matches!(reference, Value::Reference { mutable: true, .. }) {
                return Err("coroutine context adapter lost its tracked reference".into());
            }
            return Ok(Some(reference));
        }
        Ok(None)
    }
}

struct CoroutinePlaces<'tcx> {
    tcx: TyCtxt<'tcx>,
    locals: IndexVec<Local, LocalDecl<'tcx>>,
    error: Option<String>,
}

impl<'tcx> MutVisitor<'tcx> for CoroutinePlaces<'tcx> {
    fn tcx(&self) -> TyCtxt<'tcx> {
        self.tcx
    }

    fn visit_place(&mut self, place: &mut Place<'tcx>, _: PlaceContext, _: Location) {
        if self.error.is_some() {
            return;
        }
        let original = *place;
        let mut projection = Vec::new();
        let mut position = 0;
        while position < original.projection.len() {
            let element = original.projection[position];
            let prefix = Place {
                local: original.local,
                projection: self.tcx.mk_place_elems(&original.projection[..position]),
            };
            if let ProjectionElem::Downcast(_, variant) = element
                && let ty::Coroutine(id, args) = prefix.ty(&self.locals, self.tcx).ty.kind()
            {
                let Some(ProjectionElem::Field(field, field_ty)) =
                    original.projection.get(position + 1)
                else {
                    self.error = Some("whole coroutine variant projection is unsupported".into());
                    return;
                };
                let Ok(layout) = self.tcx.coroutine_layout(*id, args) else {
                    self.error = Some("coroutine has no lowered state layout".into());
                    return;
                };
                let Some(saved) = layout
                    .variant_fields
                    .get(variant)
                    .and_then(|fields| fields.get(*field))
                else {
                    self.error = Some("coroutine projection exceeds its compiler layout".into());
                    return;
                };
                // Variants can share a saved local; all accesses use its one logical slot.
                let index = args.as_coroutine().upvar_tys().len() + saved.as_usize();
                projection.push(ProjectionElem::Field(
                    FieldIdx::from_usize(index),
                    *field_ty,
                ));
                position += 2;
            } else {
                projection.push(element);
                position += 1;
            }
        }
        place.projection = self.tcx.mk_place_elems(&projection);
    }
}
