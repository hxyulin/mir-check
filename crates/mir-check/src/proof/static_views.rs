use super::*;
use rustc_const_eval::const_eval::mk_eval_cx_for_const_val;
use rustc_middle::mir::interpret::GlobalAlloc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ViewKind {
    Shared,
    Raw,
    Place,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StaticView<'tcx> {
    static_id: DefId,
    original: Ty<'tcx>,
    ty: Ty<'tcx>,
    certified: Ty<'tcx>,
    offset: u64,
    kind: ViewKind,
    writable: bool,
}

impl<'tcx> Engine<'tcx> {
    pub(super) fn static_view_constant(
        &self,
        _id: DefId,
        constant: rustc_middle::mir::Const<'tcx>,
        span: Span,
    ) -> Result<Option<Value>, String> {
        let ty::Ref(_, pointee, mutability) = constant.ty().kind() else {
            return Ok(None);
        };
        if mutability.is_mut()
            || pointee.is_freeze(self.tcx, ty::TypingEnv::fully_monomorphized())
            || self.atomic_container(*pointee, 0).is_some()
        {
            return Ok(None);
        }
        let env = ty::TypingEnv::fully_monomorphized();
        let value = constant
            .eval(self.tcx, env, span)
            .map_err(|error| format!("static view constant evaluation failed: {error:?}"))?;
        let (ecx, operand) = mk_eval_cx_for_const_val(self.tcx.at(span), env, value, constant.ty())
            .ok_or("static view constant layout unavailable")?;
        let pointer = ecx
            .read_scalar(&operand)
            .discard_err()
            .ok_or("static view needs a thin compiler pointer")?
            .to_pointer(&ecx);
        let provenance = pointer
            .provenance
            .ok_or("static view needs allocation provenance")?;
        let GlobalAlloc::Static(static_id) = self.tcx.global_alloc(provenance.alloc_id()) else {
            return Err(
                "interior mutable constant view needs an ordinary static allocation".into(),
            );
        };
        if self.tcx.is_foreign_item(static_id) {
            return Err("static views require a Rust initializer".into());
        }
        if pointer.into_raw_parts().1.bytes() != 0
            || self.static_declared_type(static_id)? != *pointee
        {
            return Err("static view needs a whole, exactly typed static allocation".into());
        }
        let original = self.static_original_type(static_id, *pointee)?;
        self.intern_static_view(StaticView {
            static_id,
            original,
            ty: *pointee,
            certified: *pointee,
            offset: 0,
            kind: ViewKind::Shared,
            writable: false,
        })
        .map(Some)
    }

    fn static_original_type(&self, id: DefId, carrier: Ty<'tcx>) -> Result<Ty<'tcx>, String> {
        let body = self.tcx.mir_for_ctfe(id);
        let mut original = None;
        for block in body.basic_blocks.iter() {
            for statement in &block.statements {
                if let StatementKind::Assign(assignment) = &statement.kind {
                    let (destination, value) = assignment.as_ref();
                    if destination.local == rustc_middle::mir::RETURN_PLACE
                        && destination.projection.is_empty()
                    {
                        let source = match value {
                            Rvalue::Cast(CastKind::Transmute, operand, _) => {
                                operand.ty(&body.local_decls, self.tcx)
                            }
                            _ => carrier,
                        };
                        if original.replace(source).is_some_and(|old| old != source) {
                            return Err(
                                "static initializer has incompatible storage origins".into()
                            );
                        }
                    }
                }
            }
            if let TerminatorKind::Call {
                func,
                args,
                destination,
                ..
            } = &block.terminator().kind
                && destination.local == rustc_middle::mir::RETURN_PLACE
                && destination.projection.is_empty()
            {
                let source = if let ty::FnDef(callee, substitutions) =
                    func.ty(&body.local_decls, self.tcx).kind()
                    && args.len() == 1
                    && (self
                        .tcx
                        .is_intrinsic(*callee, rustc_span::Symbol::intern("transmute"))
                        || self.is_transmute_initializer(*callee, substitutions.skip_binder()))
                {
                    args[0].node.ty(&body.local_decls, self.tcx)
                } else {
                    carrier
                };
                let source = self
                    .tcx
                    .try_normalize_erasing_regions(
                        ty::TypingEnv::fully_monomorphized(),
                        ty::Unnormalized::new_wip(source),
                    )
                    .map_err(|error| format!("static origin normalization failed: {error:?}"))?;
                if original.replace(source).is_some_and(|old| old != source) {
                    return Err("static initializer has incompatible storage origins".into());
                }
            }
        }
        let original = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                ty::Unnormalized::new_wip(original.unwrap_or(carrier)),
            )
            .map_err(|error| format!("static origin normalization failed: {error:?}"))?;
        let source = self.static_layout(original)?;
        let target = self.static_layout(carrier)?;
        if source.size != target.size || source.align.abi > target.align.abi {
            return Err("static initializer does not preserve storage size and alignment".into());
        }
        Ok(original)
    }

    fn is_transmute_initializer(&self, id: DefId, args: ty::GenericArgsRef<'tcx>) -> bool {
        if !matches!(self.tcx.def_kind(id), DefKind::Fn) || !self.tcx.is_mir_available(id) {
            return false;
        }
        let Ok(body) = self.instantiated_body(ty::Instance::new_raw(id, args)) else {
            return false;
        };
        if body.arg_count != 1 || body.basic_blocks.len() != 1 {
            return false;
        }
        let block = &body.basic_blocks[rustc_middle::mir::START_BLOCK];
        let [statement] = block.statements.as_slice() else {
            return false;
        };
        let StatementKind::Assign(assignment) = &statement.kind else {
            return false;
        };
        let (destination, value) = assignment.as_ref();
        let Rvalue::Cast(CastKind::Transmute, Operand::Move(source), _) = value else {
            return false;
        };
        destination.local == rustc_middle::mir::RETURN_PLACE
            && destination.projection.is_empty()
            && source.local.as_usize() == 1
            && source.projection.is_empty()
            && matches!(block.terminator().kind, TerminatorKind::Return)
    }

    fn static_layout(
        &self,
        ty: Ty<'tcx>,
    ) -> Result<rustc_middle::ty::layout::TyAndLayout<'tcx>, String> {
        let layout = self
            .tcx
            .layout_of(ty::TypingEnv::fully_monomorphized().as_query_input(ty))
            .map_err(|error| format!("static view layout failed: {error:?}"))?;
        if layout.is_unsized() {
            return Err("static views require sized layouts".into());
        }
        Ok(layout)
    }

    fn static_declared_type(&self, id: DefId) -> Result<Ty<'tcx>, String> {
        self.tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx.type_of(id).instantiate_identity(),
            )
            .map_err(|error| format!("static declared type normalization failed: {error:?}"))
    }

    fn intern_static_view(&self, view: StaticView<'tcx>) -> Result<Value, String> {
        let declared = self.static_declared_type(view.static_id)?;
        let allocation = self.static_layout(declared)?;
        let target = self.static_layout(view.ty)?;
        if view
            .offset
            .checked_add(target.size.bytes())
            .is_none_or(|end| end > allocation.size.bytes())
            || allocation.align.abi < target.align.abi
            || !view.offset.is_multiple_of(target.align.abi.bytes())
        {
            return Err("static view exceeds its allocation or required alignment".into());
        }
        let mut views = self.static_views.borrow_mut();
        let id = if let Some(id) = views.iter().position(|candidate| *candidate == view) {
            id
        } else {
            if views.len() >= 512 {
                return Err("static view shape budget reached".into());
            }
            let id = views.len();
            views.push(view);
            id
        };
        Ok(Value::StaticView {
            id,
            epoch: self
                .static_epoch
                .ok_or("static view storage epoch unavailable")?,
        })
    }

    fn static_view(&self, value: &Value, state: &State) -> Result<StaticView<'tcx>, String> {
        let Value::StaticView { id, epoch } = value else {
            return Err("expected a static storage view".into());
        };
        if !matches!(state.memory.get(*epoch), Some(Some(Value::Unit))) {
            return Err("static storage view invalidated by unknown memory effects".into());
        }
        self.static_views
            .borrow()
            .get(*id)
            .copied()
            .ok_or("static view identity unavailable".into())
    }

    pub(super) fn static_view_operand(&self, value: Value, state: &State) -> Result<Value, String> {
        if matches!(value, Value::StaticSlice { .. }) {
            self.validate_tracked_value(&value, state)?;
        }
        if matches!(value, Value::StaticView { .. }) {
            let view = self.static_view(&value, state)?;
            if view.kind == ViewKind::Place {
                if self.uninit_static_address(view)? {
                    return Err(
                        "uninitialized static payload reads need an initialization model".into(),
                    );
                }
                if let Some(atomic) = self.atomic_shape(view.ty) {
                    return Ok(atomic);
                }
                return Err(concat!(
                    "mutable static payload reads need a state model; ",
                    "initializer is not runtime state"
                )
                .into());
            }
        }
        Ok(value)
    }

    pub(super) fn static_non_null_transmute(
        &self,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
        value: &Value,
        state: &State,
    ) -> Result<Option<Value>, String> {
        let non_null = |ty: Ty<'tcx>| match ty.kind() {
            ty::Adt(def, args)
                if self.tcx.lang_items().get(LangItem::NonNull) == Some(def.did()) =>
            {
                Some((*def, *args))
            }
            _ => None,
        };
        let (wrapper, pointer, wrapped) = if non_null(target).is_some() {
            (target, source, false)
        } else if non_null(source).is_some() {
            (source, target, true)
        } else {
            return Ok(None);
        };
        let (def, args) = non_null(wrapper).ok_or("NonNull wrapper identity missing")?;
        let ty::RawPtr(pointee, _) = pointer.kind() else {
            return Ok(None);
        };
        if *pointee != args.type_at(0) || !self.thin_raw_pointer(pointer) {
            return Err("NonNull static wrapper needs the same sized pointee type".into());
        }
        let storage = if wrapped {
            let Value::Adt {
                name,
                variant: 0,
                discriminant: 0,
                fields,
                ..
            } = value
            else {
                return Err("NonNull address exposure needs its constructed wrapper".into());
            };
            if *name != self.tcx.def_path_str(def.did()) || fields.len() != 1 {
                return Err("NonNull static wrapper shape mismatch".into());
            }
            &fields[0].1
        } else {
            value
        };
        if !matches!(storage, Value::StaticView { .. }) {
            return Ok(None);
        }
        let view = self.static_view(storage, state)?;
        if view.kind != ViewKind::Raw || view.ty != *pointee {
            return Err("NonNull static wrapper needs a provenance-backed raw address".into());
        }
        let layout = self.static_layout(wrapper)?;
        let raw = self.static_layout(pointer)?;
        if layout.size != raw.size
            || layout.align.abi != raw.align.abi
            || layout.fields.count() != 1
            || layout.fields.offset(0).bytes() != 0
        {
            return Err("NonNull wrapper does not preserve the compiler pointer layout".into());
        }
        if wrapped {
            Ok(Some(storage.clone()))
        } else {
            self.constructed(wrapper, 0, vec![storage.clone()])
                .map(Some)
        }
    }

    pub(super) fn static_stored_address(
        &self,
        ty: Ty<'tcx>,
        value: &Value,
        state: &State,
    ) -> Result<(), String> {
        let view = self.static_view(value, state)?;
        let compatible = match ty.kind() {
            ty::Ref(_, pointee, mutability) => {
                !mutability.is_mut()
                    && view.kind == ViewKind::Shared
                    && *pointee == view.ty
                    && view.certified == view.ty
            }
            ty::RawPtr(pointee, _) => view.kind == ViewKind::Raw && *pointee == view.ty,
            _ => false,
        };
        if compatible {
            Ok(())
        } else {
            Err("static store address does not retain its certified type and provenance".into())
        }
    }

    pub(super) fn write_static_place(
        &self,
        state: &mut State,
        place: Place<'tcx>,
        value: &Value,
    ) -> Result<bool, String> {
        let mut base = self.local(state, place.local.as_usize())?;
        for length in 0..=place.projection.len() {
            if matches!(base, Value::StaticView { .. }) {
                for projection in &place.projection[length..] {
                    base = self.static_view_projection(&base, *projection, state)?;
                }
                let view = self.static_view(&base, state)?;
                if view.kind != ViewKind::Place {
                    return Ok(false);
                }
                if !view.writable
                    || (view.ty != view.certified && !self.uninit_static_address(view)?)
                {
                    return Err("static store needs a certified UnsafeCell payload place".into());
                }
                self.static_store_value(view.ty, value, state, 0, &mut 0)?;
                self.retain_static_store_references(value, state)?;
                // Shared payload reads stay opaque; this store supplies no value/history facts.
                return Ok(true);
            }
            if length == place.projection.len() {
                break;
            }
            let prefix = Place {
                local: place.local,
                projection: self.tcx.mk_place_elems(&place.projection[..length + 1]),
            };
            let Ok(next) = self.place(state, prefix) else {
                return Ok(false);
            };
            base = next;
        }
        Ok(false)
    }

    fn certify_atomic_overlay(&self, source: Ty<'tcx>, target: Ty<'tcx>) -> Result<(), String> {
        if self.atomic_shape(target).is_none() {
            return Err("static reinterpretation does not restore a certified storage type".into());
        }
        let source_layout = self.static_layout(source)?;
        let target_layout = self.static_layout(target)?;
        if source_layout.size < target_layout.size
            || !self.atomic_storage_prefix(source, target_layout.size.bytes(), 0, &mut 0)?
        {
            return Err(
                "atomic overlay needs an initialized atomic footprint without padding".into(),
            );
        }
        Ok(())
    }

    fn atomic_storage_prefix(
        &self,
        ty: Ty<'tcx>,
        bytes: u64,
        depth: usize,
        values: &mut usize,
    ) -> Result<bool, String> {
        if depth >= 8 || *values >= 256 {
            return Err("atomic overlay storage exceeds depth or value budget".into());
        }
        *values += 1;
        let layout = self.static_layout(ty)?;
        if bytes == 0 {
            return Ok(true);
        }
        if layout.size.bytes() < bytes
            || ty.needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized())
        {
            return Ok(false);
        }
        match ty.kind() {
            ty::Adt(def, args)
                if self
                    .tcx
                    .get_diagnostic_item(rustc_span::Symbol::intern("Atomic"))
                    == Some(def.did()) =>
            {
                let element = args.type_at(0);
                Ok((element.is_bool() || self.integer_type(element).is_some())
                    && layout.size == self.static_layout(element)?.size
                    && layout.size.bytes() == bytes)
            }
            ty::Adt(def, args) if def.is_struct() => {
                let mut ranges = Vec::new();
                for (index, field) in def.non_enum_variant().fields.iter_enumerated() {
                    let offset = layout.fields.offset(index.as_usize()).bytes();
                    if offset >= bytes {
                        continue;
                    }
                    let field_ty = self
                        .tcx
                        .try_normalize_erasing_regions(
                            ty::TypingEnv::fully_monomorphized(),
                            field.ty(self.tcx, args),
                        )
                        .map_err(|error| {
                            format!("atomic overlay field normalization failed: {error:?}")
                        })?;
                    let size = self
                        .static_layout(field_ty)?
                        .size
                        .bytes()
                        .min(bytes - offset);
                    if size == 0 {
                        continue;
                    }
                    if !self.atomic_storage_prefix(field_ty, size, depth + 1, values)? {
                        return Ok(false);
                    }
                    ranges.push((offset, size));
                }
                ranges.sort_unstable();
                let mut end = 0;
                for (offset, size) in ranges {
                    if offset != end {
                        return Ok(false);
                    }
                    end = end
                        .checked_add(size)
                        .ok_or("atomic overlay field offset overflow")?;
                }
                Ok(end == bytes)
            }
            ty::Array(element, _) => {
                let rustc_abi::FieldsShape::Array { stride, count } = layout.fields else {
                    return Ok(false);
                };
                let stride = stride.bytes();
                if stride == 0 || stride != self.static_layout(*element)?.size.bytes() {
                    return Ok(false);
                }
                let needed = bytes.div_ceil(stride);
                if needed > count || needed > 128 {
                    return Ok(false);
                }
                for index in 0..needed {
                    let size = (bytes - index * stride).min(stride);
                    if !self.atomic_storage_prefix(*element, size, depth + 1, values)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn static_place_view(&self, value: &Value, state: &State) -> Result<bool, String> {
        Ok(self.static_view(value, state)?.kind == ViewKind::Place)
    }

    fn is_static_unsafe_cell(&self, ty: Ty<'tcx>) -> bool {
        matches!(ty.kind(), ty::Adt(def, _) if
            self.tcx.lang_items().get(LangItem::UnsafeCell) == Some(def.did()))
    }

    fn uninit_static_address(&self, view: StaticView<'tcx>) -> Result<bool, String> {
        let ty::Adt(def, args) = view.certified.kind() else {
            return Ok(false);
        };
        if self.tcx.lang_items().get(LangItem::MaybeUninit) != Some(def.did()) {
            return Ok(false);
        }
        let payload = args.type_at(0);
        let compatible = payload == view.ty
            || (self.is_static_unsafe_cell(payload)
                && matches!(payload.kind(), ty::Adt(_, args) if args.type_at(0) == view.ty));
        if !compatible {
            return Ok(false);
        }
        let wrapper = self.static_layout(view.certified)?;
        let target = self.static_layout(view.ty)?;
        if wrapper.size != target.size || wrapper.align.abi != target.align.abi {
            return Err("uninitialized static address changes its payload layout".into());
        }
        Ok(true)
    }

    pub(super) fn static_uninit_pointer(
        &self,
        receiver: Ty<'tcx>,
        name: &str,
        values: &[Value],
        state: &State,
    ) -> Result<Option<Value>, String> {
        let ty::Adt(def, args) = receiver.kind() else {
            return Ok(None);
        };
        if self.tcx.lang_items().get(LangItem::MaybeUninit) != Some(def.did()) || name != "as_ptr" {
            return Ok(None);
        }
        let [value @ Value::StaticView { .. }] = values else {
            return Ok(None);
        };
        let mut view = self.static_view(value, state)?;
        if view.kind != ViewKind::Shared || view.ty != receiver || view.certified != receiver {
            return Err("MaybeUninit address requires a certified shared container view".into());
        }
        let payload = args.type_at(0);
        let wrapper_layout = self.static_layout(receiver)?;
        let payload_layout = self.static_layout(payload)?;
        if wrapper_layout.size != payload_layout.size
            || wrapper_layout.align.abi != payload_layout.align.abi
        {
            return Err("MaybeUninit payload address does not preserve size and alignment".into());
        }
        view.ty = payload;
        view.kind = ViewKind::Raw;
        self.intern_static_view(view).map(Some)
    }

    pub(super) fn static_view_projection(
        &self,
        value: &Value,
        projection: rustc_middle::mir::PlaceElem<'tcx>,
        state: &State,
    ) -> Result<Value, String> {
        let mut view = self.static_view(value, state)?;
        match projection {
            ProjectionElem::Deref if view.kind != ViewKind::Place => {
                if !self.uninit_static_address(view)? {
                    if view.ty != view.certified && !(view.offset == 0 && view.ty == view.original)
                    {
                        self.certify_atomic_overlay(view.certified, view.ty)?;
                    }
                    view.certified = view.ty;
                }
                view.kind = ViewKind::Place;
            }
            ProjectionElem::Field(field, target) if view.kind == ViewKind::Place => {
                let uninit = self.uninit_static_address(view)?;
                if uninit && !self.is_static_unsafe_cell(view.ty) {
                    return Err("uninitialized static fields do not have certified payloads".into());
                }
                let layout = self.static_layout(view.ty)?;
                if field.as_usize() >= layout.fields.count() {
                    return Err("static view field is outside its layout".into());
                }
                let actual = match view.ty.kind() {
                    ty::Adt(def, args) if def.is_struct() => self
                        .tcx
                        .try_normalize_erasing_regions(
                            ty::TypingEnv::fully_monomorphized(),
                            def.non_enum_variant().fields[field].ty(self.tcx, args),
                        )
                        .map_err(|error| format!("static field normalization failed: {error:?}"))?,
                    ty::Tuple(fields) => fields[field.as_usize()],
                    _ => return Err("static view field requires a struct or tuple".into()),
                };
                if actual != target {
                    return Err("static view projected field type mismatch".into());
                }
                view.offset = view
                    .offset
                    .checked_add(layout.fields.offset(field.as_usize()).bytes())
                    .ok_or("static view offset overflow")?;
                view.ty = target;
                if !uninit {
                    view.certified = target;
                }
            }
            ProjectionElem::Index(index) if view.kind == ViewKind::Place => {
                let Value::StaticSlice { elements, .. } =
                    self.static_array_elements(value, state)?
                else {
                    unreachable!();
                };
                let selected = self.fixed_element(
                    &elements,
                    &self.local(state, index.as_usize())?,
                    &state.conditions,
                )?;
                return self.static_view_projection(&selected, ProjectionElem::Deref, state);
            }
            ProjectionElem::ConstantIndex {
                offset,
                min_length,
                from_end,
            } if view.kind == ViewKind::Place => {
                return self.constant_element(
                    state,
                    self.static_array_elements(value, state)?,
                    offset,
                    min_length,
                    from_end,
                );
            }
            _ => return Err("unsupported static storage projection".into()),
        }
        self.intern_static_view(view)
    }

    pub(super) fn borrow_static_view(
        &self,
        value: &Value,
        mutable: bool,
        state: &State,
    ) -> Result<Value, String> {
        let mut view = self.static_view(value, state)?;
        if mutable || view.kind != ViewKind::Place {
            return Err("static view borrowing requires a shared typed place".into());
        }
        if self.uninit_static_address(view)? {
            if !self.is_static_unsafe_cell(view.ty) {
                return Err("uninitialized static payload borrowing remains unsupported".into());
            }
        } else {
            if let Some(atomic) = self.atomic_shape(view.ty) {
                return Ok(atomic);
            }
            if view.ty != view.certified && !(view.offset == 0 && view.ty == view.original) {
                return Err(
                    "static reinterpretation does not restore a certified storage type".into(),
                );
            }
            view.certified = view.ty;
        }
        view.kind = ViewKind::Shared;
        if matches!(view.ty.kind(), ty::Array(..)) {
            return self.static_array_elements(&self.intern_static_view(view)?, state);
        }
        self.intern_static_view(view)
    }

    pub(super) fn static_array_elements(
        &self,
        value: &Value,
        state: &State,
    ) -> Result<Value, String> {
        let view = self.static_view(value, state)?;
        let ty::Array(element, _) = view.ty.kind() else {
            return Err("static slice views require a certified fixed array".into());
        };
        if !matches!(view.kind, ViewKind::Shared | ViewKind::Place) || view.certified != view.ty {
            return Err("static slice views require a shared certified array reference".into());
        }
        let layout = self.static_layout(view.ty)?;
        let rustc_abi::FieldsShape::Array { stride, count } = layout.fields else {
            return Err("static array does not have an array layout".into());
        };
        if count > 128 {
            return Err("static slice view exceeds the 128-element budget".into());
        }
        let mut elements = Vec::new();
        for index in 0..count {
            let offset = stride
                .bytes()
                .checked_mul(index)
                .and_then(|offset| view.offset.checked_add(offset))
                .ok_or("static array element offset overflow")?;
            elements.push(self.intern_static_view(StaticView {
                ty: *element,
                certified: *element,
                offset,
                kind: ViewKind::Shared,
                ..view
            })?);
        }
        let Value::StaticView { epoch, .. } = value else {
            unreachable!();
        };
        Ok(Value::StaticSlice {
            elements,
            epoch: *epoch,
        })
    }

    pub(super) fn coerce_static_slice(
        &self,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
        value: &Value,
        state: &State,
    ) -> Result<Value, String> {
        let (ty::Ref(_, array, source_mut), ty::Ref(_, slice, target_mut)) =
            (source.kind(), target.kind())
        else {
            return Err("static slice coercion requires shared reference types".into());
        };
        let (ty::Array(element, _), ty::Slice(target_element)) = (array.kind(), slice.kind())
        else {
            return Err("static slice coercion requires an array and slice".into());
        };
        if source_mut.is_mut()
            || target_mut.is_mut()
            || element != target_element
            || self.static_view(value, state)?.ty != *array
        {
            return Err("static slice coercion changes element type or mutability".into());
        }
        self.static_array_elements(value, state)
    }

    pub(super) fn cast_static_view(
        &self,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
        value: &Value,
        state: &State,
    ) -> Result<Value, String> {
        let mut view = self.static_view(value, state)?;
        let (ty::RawPtr(source_pointee, _), ty::RawPtr(target_pointee, _)) =
            (source.kind(), target.kind())
        else {
            return Err("static view casts require thin raw pointer types".into());
        };
        if view.kind != ViewKind::Raw || view.ty != *source_pointee {
            return Err("static view pointer source type mismatch".into());
        }
        if let ty::Adt(def, args) = source_pointee.kind()
            && self.tcx.lang_items().get(LangItem::UnsafeCell) == Some(def.did())
            && (view.certified == *source_pointee || self.uninit_static_address(view)?)
            && args.type_at(0) == *target_pointee
        {
            let wrapper = self.static_layout(*source_pointee)?;
            let payload = self.static_layout(*target_pointee)?;
            if wrapper.fields.count() != 1
                || wrapper.fields.offset(0).bytes() != 0
                || wrapper.size != payload.size
                || wrapper.align.abi != payload.align.abi
            {
                return Err(
                    "UnsafeCell pointer cast does not preserve its transparent layout".into(),
                );
            }
            if view.certified == *source_pointee {
                view.certified = *target_pointee;
            }
            view.writable = true;
        }
        view.ty = *target_pointee;
        self.intern_static_view(view)
    }

    pub(super) fn raw_static_view(&self, value: &Value, state: &State) -> Result<Value, String> {
        let mut view = self.static_view(value, state)?;
        if view.kind != ViewKind::Place {
            return Err("static raw address needs a certified place".into());
        }
        view.kind = ViewKind::Raw;
        self.intern_static_view(view)
    }

    pub(super) fn static_cell_get(
        &self,
        receiver: Ty<'tcx>,
        name: &str,
        values: &[Value],
        state: &State,
    ) -> Result<Option<Value>, String> {
        let ty::Adt(def, args) = receiver.kind() else {
            return Ok(None);
        };
        if self.tcx.lang_items().get(LangItem::UnsafeCell) != Some(def.did())
            || !matches!(name, "get" | "raw_get")
        {
            return Ok(None);
        }
        let [value @ Value::StaticView { .. }] = values else {
            return Ok(None);
        };
        let mut view = self.static_view(value, state)?;
        let expected = if name == "get" {
            ViewKind::Shared
        } else {
            ViewKind::Raw
        };
        if view.kind != expected || view.ty != receiver {
            return Err("UnsafeCell static receiver type mismatch".into());
        }
        let layout = self.static_layout(receiver)?;
        if layout.fields.count() != 1 || layout.fields.offset(0).bytes() != 0 {
            return Err("UnsafeCell does not have its transparent storage layout".into());
        }
        let uninit = self.uninit_static_address(view)?;
        view.ty = args.type_at(0);
        if !uninit {
            view.certified = view.ty;
        }
        view.writable = true;
        view.kind = ViewKind::Raw;
        self.intern_static_view(view).map(Some)
    }

    pub(super) fn expose_static_address(
        &mut self,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
        value: &Value,
        state: &mut State,
    ) -> Result<Value, String> {
        let view = self.static_view(value, state)?;
        if view.kind != ViewKind::Raw
            || !matches!(source.kind(), ty::RawPtr(pointee, _) if *pointee == view.ty)
        {
            return Err("static address exposure needs its certified raw pointer view".into());
        }
        let (target_bits, signed) = self
            .integer_type(target)
            .ok_or("static address exposure needs an integer target")?;
        let bits = u32::from(self.tcx.sess.target.pointer_width);
        let layout = self.static_layout(self.static_declared_type(view.static_id)?)?;
        let base = if let Some(base) = self.static_addresses.get(&view.static_id) {
            base.clone()
        } else {
            let base = self.fresh(Sort::BitVec(bits));
            self.static_addresses.insert(view.static_id, base.clone());
            base
        };
        let integer = |expression| Value::Int {
            expression,
            bits,
            signed: false,
        };
        let zero = symbolic::integer(&self.terms, 0, bits, false);
        state.conditions.push(
            symbolic::binary(&self.terms, "ne", integer(base.clone()), zero.clone())?.boolean()?,
        );
        let mask = symbolic::integer(
            &self.terms,
            u128::from(layout.align.abi.bytes() - 1),
            bits,
            false,
        );
        let low = symbolic::binary(&self.terms, "and", integer(base.clone()), mask)?;
        state
            .conditions
            .push(symbolic::binary(&self.terms, "eq", low, zero)?.boolean()?);
        let maximum = match bits {
            1..=127 => (1_u128 << bits) - 1,
            128 => u128::MAX,
            _ => return Err("unsupported target pointer width".into()),
        };
        let limit = maximum
            .checked_sub(u128::from(layout.size.bytes()))
            .ok_or("static allocation exceeds target address space")?;
        state.conditions.push(
            symbolic::binary(
                &self.terms,
                "le",
                integer(base.clone()),
                symbolic::integer(&self.terms, limit, bits, false),
            )?
            .boolean()?,
        );
        let address = symbolic::binary(
            &self.terms,
            "add",
            integer(base),
            symbolic::integer(&self.terms, u128::from(view.offset), bits, false),
        )?;
        symbolic::cast(&self.terms, address, target_bits, signed)
    }
}
