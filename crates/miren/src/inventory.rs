use miren::{LocalCall, Site, SiteKind, SiteStatus};
use rustc_hir::def::DefKind;
use rustc_middle::mir::{AssertKind, Body, Operand, START_BLOCK, TerminatorKind};
use rustc_middle::ty::{self, TyCtxt};
use std::collections::VecDeque;

pub fn collect<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> (Vec<Site>, Vec<LocalCall>) {
    let reachable = reachable_blocks(body);
    let mut sites = Vec::new();
    let mut calls = Vec::new();
    for (block, data) in body.basic_blocks.iter_enumerated() {
        let terminator = data.terminator();
        let mut site = Site {
            block: block.as_usize(),
            source: super::source(tcx, terminator.source_info.span.source_callsite()),
            kind: SiteKind::ExternalCall,
            detail: String::new(),
            failure_condition: None,
            callee: None,
            unwind: None,
            cleanup: data.is_cleanup,
            cfg_reachable: reachable[block.as_usize()],
            enabled: true,
            status: SiteStatus::Unverified,
        };
        match &terminator.kind {
            TerminatorKind::Assert {
                cond,
                expected,
                msg,
                unwind,
                ..
            } => {
                site.kind = assertion_kind(msg);
                site.detail = format!("{msg:?}");
                site.failure_condition = Some(format!("{cond:?} != {expected}"));
                site.unwind = Some(format!("{unwind:?}"));
                site.enabled = !msg.is_optional_overflow_check() || tcx.sess.overflow_checks();
            }
            TerminatorKind::Call { func, unwind, .. } => {
                site.unwind = Some(format!("{unwind:?}"));
                if local_call(tcx, body, func, &mut site) {
                    calls.push(LocalCall {
                        block: site.block,
                        callee: site.callee.take().expect("local calls have a named target"),
                        source: site.source,
                        cfg_reachable: site.cfg_reachable,
                    });
                    continue;
                }
            }
            TerminatorKind::TailCall { func, .. } => {
                if local_call(tcx, body, func, &mut site) {
                    calls.push(LocalCall {
                        block: site.block,
                        callee: site.callee.take().expect("local calls have a named target"),
                        source: site.source,
                        cfg_reachable: site.cfg_reachable,
                    });
                    continue;
                }
            }
            TerminatorKind::Drop { place, unwind, .. } => {
                site.kind = SiteKind::Drop;
                site.detail = format!("drop {place:?}: {:?}", place.ty(&body.local_decls, tcx).ty);
                site.unwind = Some(format!("{unwind:?}"));
            }
            TerminatorKind::InlineAsm { .. } => {
                site.kind = SiteKind::InlineAssembly;
                site.detail = "assembly semantics are not modeled".to_owned();
            }
            TerminatorKind::Goto { .. }
            | TerminatorKind::SwitchInt { .. }
            | TerminatorKind::UnwindResume
            | TerminatorKind::UnwindTerminate(_)
            | TerminatorKind::Return
            | TerminatorKind::Unreachable
            | TerminatorKind::Yield { .. }
            | TerminatorKind::CoroutineDrop
            | TerminatorKind::FalseEdge { .. }
            | TerminatorKind::FalseUnwind { .. } => continue,
        }
        sites.push(site);
    }
    (sites, calls)
}

fn assertion_kind(message: &AssertKind<Operand<'_>>) -> SiteKind {
    match message {
        AssertKind::BoundsCheck { .. } => SiteKind::BoundsCheck,
        AssertKind::Overflow(..) | AssertKind::OverflowNeg(_) => SiteKind::Overflow,
        AssertKind::DivisionByZero(_) => SiteKind::DivisionByZero,
        AssertKind::RemainderByZero(_) => SiteKind::RemainderByZero,
        AssertKind::ResumedAfterReturn(_)
        | AssertKind::ResumedAfterPanic(_)
        | AssertKind::ResumedAfterDrop(_) => SiteKind::CoroutineState,
        AssertKind::MisalignedPointerDereference { .. }
        | AssertKind::NullPointerDereference
        | AssertKind::NullReferenceConstructed => SiteKind::PointerCheck,
        AssertKind::InvalidEnumConstruction(_) => SiteKind::EnumCheck,
    }
}

fn local_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    function: &Operand<'tcx>,
    site: &mut Site,
) -> bool {
    let ty::FnDef(id, _) = *function.ty(&body.local_decls, tcx).kind() else {
        site.kind = SiteKind::IndirectCall;
        site.detail = format!("{function:?}; target is not statically resolved");
        return false;
    };
    let name = tcx.def_path_str(id);
    site.callee = Some(name.clone());
    site.detail = name;
    if super::identity::is_panic_call(tcx, id) {
        site.kind = SiteKind::PanicCall;
    } else if tcx.def_kind(tcx.parent(id)) == DefKind::Trait {
        site.kind = SiteKind::TraitCall;
    } else if id.is_local()
        && matches!(
            tcx.def_kind(id),
            DefKind::Fn | DefKind::AssocFn | DefKind::Closure
        )
        && tcx.is_mir_available(id)
    {
        return true;
    } else if id.is_local() {
        site.kind = SiteKind::UnavailableBody;
    } else {
        site.kind = SiteKind::ExternalCall;
    }
    false
}

fn reachable_blocks(body: &Body<'_>) -> Vec<bool> {
    let mut reachable = vec![false; body.basic_blocks.len()];
    let mut queue = VecDeque::from([START_BLOCK]);
    while let Some(block) = queue.pop_front() {
        if reachable[block.as_usize()] {
            continue;
        }
        reachable[block.as_usize()] = true;
        queue.extend(body.basic_blocks[block].terminator().successors());
    }
    reachable
}
