use super::*;

const MAX_CALL_SIGNATURES: usize = 256;

impl<'tcx> Engine<'tcx> {
    pub(super) fn call_poly_signature(
        &self,
        instance: ty::Instance<'tcx>,
    ) -> Result<ty::PolyFnSig<'tcx>, String> {
        if let Some(signature) = self.call_signatures.borrow().get(&instance) {
            return Ok(*signature);
        }
        let signature = self
            .tcx
            .try_normalize_erasing_regions(
                ty::TypingEnv::fully_monomorphized(),
                self.tcx
                    .fn_sig(instance.def_id())
                    .instantiate(self.tcx, instance.args),
            )
            .map_err(|error| format!("call signature normalization failed: {error:?}"))?;
        let mut signatures = self.call_signatures.borrow_mut();
        if signatures.len() < MAX_CALL_SIGNATURES {
            signatures.insert(instance, signature);
        }
        Ok(signature)
    }

    pub(super) fn call_signature(
        &self,
        instance: ty::Instance<'tcx>,
    ) -> Result<ty::FnSig<'tcx>, String> {
        self.call_poly_signature(instance)
            .map(|sig| sig.skip_binder())
    }
}
