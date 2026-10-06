use rustc_attr_ir::LangItem;
use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::DefId;

pub fn is_panic_call(tcx: TyCtxt<'_>, id: DefId) -> bool {
    tcx.lang_items().from_def_id(id).is_some_and(|item| {
        matches!(
            item,
            LangItem::Panic
                | LangItem::PanicNounwind
                | LangItem::PanicFmt
                | LangItem::PanicDisplay
                | LangItem::ConstPanicFmt
                | LangItem::PanicBoundsCheck
                | LangItem::PanicMisalignedPointerDereference
                | LangItem::PanicImpl
                | LangItem::PanicCannotUnwind
                | LangItem::PanicInCleanup
                | LangItem::PanicAddOverflow
                | LangItem::PanicSubOverflow
                | LangItem::PanicMulOverflow
                | LangItem::PanicDivOverflow
                | LangItem::PanicRemOverflow
                | LangItem::PanicNegOverflow
                | LangItem::PanicShrOverflow
                | LangItem::PanicShlOverflow
                | LangItem::PanicDivZero
                | LangItem::PanicRemZero
                | LangItem::PanicCoroutineResumed
                | LangItem::PanicAsyncFnResumed
                | LangItem::PanicAsyncGenFnResumed
                | LangItem::PanicGenFnNone
                | LangItem::PanicCoroutineResumedPanic
                | LangItem::PanicAsyncFnResumedPanic
                | LangItem::PanicAsyncGenFnResumedPanic
                | LangItem::PanicGenFnNonePanic
                | LangItem::PanicNullPointerDereference
                | LangItem::PanicNullReferenceConstructed
                | LangItem::PanicInvalidEnumConstruction
                | LangItem::PanicCoroutineResumedDrop
                | LangItem::PanicAsyncFnResumedDrop
                | LangItem::PanicAsyncGenFnResumedDrop
                | LangItem::PanicGenFnNoneDrop
                | LangItem::BeginPanic
        )
    })
}
