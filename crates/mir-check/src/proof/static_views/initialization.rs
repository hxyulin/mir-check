use super::*;

impl<'tcx> Engine<'tcx> {
    pub(super) fn require_static_initialization(
        &self,
        view: StaticView<'tcx>,
        state: &State,
    ) -> Result<(), String> {
        if state.memory.static_uninitialized.is_empty() {
            return Ok(());
        }
        let accessed = self.typed_storage_footprint(view.ty, view.offset)?;
        let views = self.static_views.borrow();
        for id in &state.memory.static_uninitialized {
            let retired = views.get(*id).ok_or("retired static shape unavailable")?;
            if retired.static_id != view.static_id {
                continue;
            }
            let footprint = self.typed_storage_footprint(retired.ty, retired.offset)?;
            if accessed.may_overlap(footprint) {
                return Err(format!(
                    "static payload {} overlaps retired uninitialized storage {}; \
                    a matching typed store must reinitialize it",
                    view.ty, retired.ty
                ));
            }
        }
        Ok(())
    }

    pub(in crate::proof) fn drop_static_place(
        &mut self,
        value: &Value,
        ty: Ty<'tcx>,
        state: &mut State,
    ) -> Result<bool, String> {
        let Value::StaticView { id, .. } = value else {
            return Ok(false);
        };
        let view = self.static_view(value, state)?;
        if view.kind != ViewKind::Place {
            return Ok(false);
        }
        if view.ty != ty || !view.writable || !self.certified_static_type(view)? {
            return Err("static drop requires a writable certified typed place".into());
        }
        if self.uninit_static_address(view)? {
            return Err("static drop cannot consume an uninitialized payload".into());
        }
        self.require_static_initialization(view, state)?;
        if ty.needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized()) {
            return Err(format!(
                "static destructor for {ty} needs subobject initialization effects"
            ));
        }
        if self.static_layout(ty)?.size.bytes() == 0 {
            return Err("zero-sized static drops need typed path identity".into());
        }
        if state.memory.static_uninitialized.len() >= 128 {
            return Err("static initialization exceeds the 128-retired-subobject budget".into());
        }
        state.memory.static_uninitialized.push(*id);
        state.memory.invalidate_startup();
        Ok(true)
    }

    pub(super) fn reinitialize_static_place(&self, view: StaticView<'tcx>, state: &mut State) {
        let views = self.static_views.borrow();
        state.memory.static_uninitialized.retain(|id| {
            let Some(retired) = views.get(*id) else {
                return true;
            };
            (retired.static_id, retired.offset, retired.ty)
                != (view.static_id, view.offset, view.ty)
        });
    }
}

impl Engine<'_> {
    pub(in crate::proof) fn validate_static_initialization_graph(
        &self,
        value: &Value,
        state: &State,
        visited: &mut Vec<usize>,
    ) -> Result<(), String> {
        if state.memory.static_uninitialized.is_empty() {
            return Ok(());
        }
        match value {
            Value::StaticView { .. } => {
                let view = self.static_view(value, state)?;
                if view.kind != ViewKind::Raw {
                    self.require_static_initialization(view, state)?;
                }
                Ok(())
            }
            Value::Reference { allocation, .. }
            | Value::Cell { allocation }
            | Value::LocalAtomic { allocation, .. } => {
                if visited.contains(allocation) {
                    return Ok(());
                }
                visited.push(*allocation);
                if let Some(Some(stored)) = state.memory.get(*allocation) {
                    self.validate_static_initialization_graph(stored, state, visited)?;
                }
                Ok(())
            }
            Value::Adt { fields, .. } => {
                for (_, field) in fields {
                    self.validate_static_initialization_graph(field, state, visited)?;
                }
                Ok(())
            }
            Value::Enum { variants, .. }
            | Value::Tuple(variants)
            | Value::Elements(variants)
            | Value::StaticSlice {
                elements: variants, ..
            } => {
                for variant in variants {
                    self.validate_static_initialization_graph(variant, state, visited)?;
                }
                Ok(())
            }
            Value::TrackedPointer { reference, .. }
            | Value::SliceIterator {
                source: reference, ..
            }
            | Value::MetadataPointer(reference)
            | Value::DebugReference {
                source: reference, ..
            } => self.validate_static_initialization_graph(reference, state, visited),
            Value::Input(_)
            | Value::Bool(_)
            | Value::Int { .. }
            | Value::Float { .. }
            | Value::Bytes { .. }
            | Value::RawPointer { .. }
            | Value::Atomic { .. }
            | Value::StaticText
            | Value::FormatArguments
            | Value::FunctionPointer { .. }
            | Value::Function
            | Value::Uninitialized
            | Value::Unit => Ok(()),
        }
    }
}
