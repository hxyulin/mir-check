use super::*;
use rustc_span::{Symbol, def_id::CrateNum};

pub(super) struct PointerAtomicWrapper<'tcx> {
    pub(super) definition: DefId,
    pub(super) field_name: Symbol,
    pub(super) field_ty: Ty<'tcx>,
}

impl<'tcx> Engine<'tcx> {
    pub(super) fn pointer_atomic_wrapper(
        &self,
        ty: Ty<'tcx>,
        core: CrateNum,
    ) -> Result<PointerAtomicWrapper<'tcx>, String> {
        let ty::Adt(def, args) = ty.kind() else {
            return Err("pointer atomic wrapper is not a core struct".into());
        };
        let env = ty::TypingEnv::fully_monomorphized();
        if def.did().krate != core || !def.is_struct() || ty.needs_drop(self.tcx, env) {
            return Err("pointer atomic wrapper has unsupported identity or drop behavior".into());
        }
        let [field] = def.non_enum_variant().fields.raw.as_slice() else {
            return Err("pointer atomic wrapper must have one storage field".into());
        };
        let layout = self
            .tcx
            .layout_of(env.as_query_input(ty))
            .map_err(|error| format!("pointer atomic wrapper layout failed: {error:?}"))?;
        if layout.size.bits() != u64::from(self.tcx.sess.target.pointer_width)
            || layout.fields.offset(0).bytes() != 0
        {
            return Err("pointer atomic wrapper does not preserve pointer representation".into());
        }
        let field_ty = self
            .tcx
            .try_normalize_erasing_regions(env, field.ty(self.tcx, args))
            .map_err(|error| format!("pointer atomic storage normalization failed: {error:?}"))?;
        Ok(PointerAtomicWrapper {
            definition: def.did(),
            field_name: field.name,
            field_ty,
        })
    }
}
