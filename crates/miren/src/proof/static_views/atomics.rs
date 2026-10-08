use super::*;
use startup_memory::StaticAtomicLocation;

impl<'tcx> Engine<'tcx> {
    pub(in crate::proof) fn static_atomic_storage(
        &self,
        value: &Value,
        receiver_ty: Ty<'tcx>,
        payload_ty: Ty<'tcx>,
        state: &State,
    ) -> Result<StaticAtomicLocation, String> {
        let env = ty::TypingEnv::fully_monomorphized();
        let receiver_ty = self
            .tcx
            .try_normalize_erasing_regions(env, ty::Unnormalized::new_wip(receiver_ty))
            .map_err(|error| format!("static atomic receiver normalization failed: {error:?}"))?;
        let payload_ty = self
            .tcx
            .try_normalize_erasing_regions(env, ty::Unnormalized::new_wip(payload_ty))
            .map_err(|error| format!("static atomic payload normalization failed: {error:?}"))?;
        let ty::Adt(def, args) = receiver_ty.kind() else {
            return Err("static atomic receiver needs a compiler atomic type".into());
        };
        if self
            .tcx
            .get_diagnostic_item(rustc_span::Symbol::intern("Atomic"))
            != Some(def.did())
            || args.type_at(0) != payload_ty
        {
            return Err("static atomic receiver does not match its compiler payload type".into());
        }
        let bits = match payload_ty.kind() {
            ty::Int(_) | ty::Uint(_) => {
                self.integer_type(payload_ty)
                    .ok_or("static atomic integer payload width unavailable")?
                    .0
            }
            ty::RawPtr(..) if self.thin_raw_pointer(payload_ty) => {
                u32::from(self.tcx.sess.target.pointer_width)
            }
            _ => {
                return Err(
                    "static atomic storage supports primitive integers and thin raw pointers"
                        .into(),
                );
            }
        };
        if bits == 0 || bits > 128 || !bits.is_multiple_of(8) {
            return Err("static atomic scalar width is unsupported".into());
        }
        let view = self.static_view(value, state)?;
        self.require_static_initialization(view, state)?;
        if view.kind != (ViewKind::Reference { mutable: false })
            || view.ty != receiver_ty
            || !self.certified_static_type(view)?
        {
            return Err(
                "static atomic access requires a certified shared typed atomic view".into(),
            );
        }
        let receiver_layout = self.static_layout(receiver_ty)?;
        let payload_layout = self.static_layout(payload_ty)?;
        if receiver_layout.size != payload_layout.size
            || payload_layout.size.bits() != u64::from(bits)
        {
            return Err("static atomic access requires exactly sized scalar storage".into());
        }
        Ok(StaticAtomicLocation {
            definition: view.static_id,
            offset: view.offset,
            bytes: receiver_layout.size.bytes(),
        })
    }

    pub(in crate::proof) fn static_atomic_location(
        &self,
        value: &Value,
        state: &State,
        bits: u32,
        signed: bool,
    ) -> Result<StaticAtomicLocation, String> {
        let view = self.static_view(value, state)?;
        if !matches!(self.atomic_shape(view.ty), Some(Value::Atomic { bits: b, signed: s })
            if (bits, signed) == (b, s))
        {
            return Err(
                "static atomic access requires a certified shared integer atomic view".into(),
            );
        }
        let ty::Adt(_, args) = view.ty.kind() else {
            return Err("static integer atomic receiver type unavailable".into());
        };
        self.static_atomic_storage(value, view.ty, args.type_at(0), state)
    }
}
