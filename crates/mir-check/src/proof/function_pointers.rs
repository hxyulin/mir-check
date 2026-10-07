use super::*;
use rustc_middle::ty::adjustment::PointerCoercion;

impl<'tcx> Engine<'tcx> {
    pub(super) fn reify_function_pointer(
        &mut self,
        kind: PointerCoercion,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
    ) -> Result<Value, String> {
        if !matches!(kind, PointerCoercion::ReifyFnPointer(_)) {
            return Err("only known function-item pointer coercions are modeled".into());
        }
        let ty::FnDef(id, args) = source.kind() else {
            return Err("function-pointer coercion needs a concrete function item".into());
        };
        let instance = ty::Instance::resolve_for_fn_ptr(
            self.tcx,
            ty::TypingEnv::fully_monomorphized(),
            *id,
            args.skip_binder(),
        )
        .ok_or_else(|| {
            format!(
                "unresolved function-pointer target {}",
                self.tcx.def_path_str(*id)
            )
        })?;
        if !matches!(instance.def, ty::InstanceKind::Item(_)) {
            return Err(format!(
                "unsupported function-pointer adapter {:?}",
                instance.def
            ));
        }
        let signature = self.call_poly_signature(ty::Instance::new_raw(*id, args.skip_binder()))?;
        let expected = self
            .tcx
            .erase_and_anonymize_regions(Ty::new_fn_ptr(self.tcx, signature));
        let target = self.tcx.erase_and_anonymize_regions(target);
        if expected != target {
            return Err("function-pointer coercion changes the callable signature".into());
        }
        let entry = (instance, target);
        let id = if let Some(id) = self
            .function_pointers
            .iter()
            .position(|item| *item == entry)
        {
            id
        } else {
            if self.function_pointers.len() >= 512 {
                return Err("known function-pointer registry exceeds 512 targets".into());
            }
            let id = self.function_pointers.len();
            self.function_pointers.push(entry);
            id
        };
        Ok(Value::FunctionPointer { id })
    }

    pub(super) fn known_function_pointer(
        &self,
        value: &Value,
        callable: Ty<'tcx>,
    ) -> Result<ty::Instance<'tcx>, String> {
        let Value::FunctionPointer { id } = value else {
            return Err("indirect call needs a known function-pointer target".into());
        };
        let (instance, signature) = self
            .function_pointers
            .get(*id)
            .ok_or("function-pointer target is missing")?;
        let callable = self.tcx.erase_and_anonymize_regions(callable);
        if callable != *signature {
            return Err("indirect-call signature differs from its known target".into());
        }
        Ok(*instance)
    }
}
