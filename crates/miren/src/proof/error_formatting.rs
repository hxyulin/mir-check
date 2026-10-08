use super::*;
use rustc_middle::ty::adjustment::PointerCoercion;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    pub(super) fn is_shared_debug_reference(&self, ty: Ty<'tcx>) -> bool {
        let ty::Ref(_, object, mutability) = ty.kind() else {
            return false;
        };
        let ty::Dynamic(predicates, _) = object.kind() else {
            return false;
        };
        !mutability.is_mut()
            && predicates.principal().is_some_and(|principal| {
                self.tcx
                    .is_diagnostic_item(Symbol::intern("Debug"), principal.skip_binder().def_id)
            })
    }

    pub(super) fn debug_reference_coercion(
        &self,
        kind: PointerCoercion,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
        value: Value,
        state: &State,
    ) -> Result<Value, String> {
        if !self.is_shared_debug_reference(target) {
            return Err("Debug coercion target is not a shared Debug trait object".into());
        }
        if kind == PointerCoercion::Unsize
            && self.tcx.erase_and_anonymize_regions(source)
                == self.tcx.erase_and_anonymize_regions(target)
        {
            if !matches!(value, Value::DebugReference { place: false, .. }) {
                return Err("Debug reborrow needs its opaque reference identity".into());
            }
            self.validate_tracked_value(&value, state)?;
            return Ok(value);
        }
        let ty::Ref(_, pointee, mutability) = source.kind() else {
            return Err("Debug coercion needs a shared concrete reference".into());
        };
        if kind != PointerCoercion::Unsize
            || mutability.is_mut()
            || !pointee.is_sized(self.tcx, ty::TypingEnv::fully_monomorphized())
        {
            return Err("only concrete shared-reference Debug unsizing is modeled".into());
        }
        self.validate_tracked_value(&value, state)?;
        Ok(Value::DebugReference {
            source: Box::new(value),
            place: false,
        })
    }

    pub(super) fn is_result_panic_helper(&self, callee: DefId) -> bool {
        let Some(result) = self
            .tcx
            .lang_items()
            .get(LangItem::ResultOk)
            .map(|variant| self.tcx.parent(variant))
        else {
            return false;
        };
        if self.tcx.def_kind(callee) != DefKind::Fn
            || self.tcx.parent(callee) != self.tcx.parent(result)
            || self.tcx.item_name(callee) != Symbol::intern("unwrap_failed")
        {
            return false;
        }
        let signature = self.tcx.fn_sig(callee).instantiate_identity().skip_binder();
        if !signature.output().is_never() {
            return false;
        }
        let [message, error] = signature.inputs() else {
            return false;
        };
        matches!(message.kind(), ty::Ref(_, element, mutability)
            if element.is_str() && !mutability.is_mut())
            && self.is_shared_debug_reference(*error)
    }
}
