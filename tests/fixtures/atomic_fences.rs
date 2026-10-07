#![no_std]
#![forbid(unsafe_code)]
#![feature(custom_mir, core_intrinsics, adt_const_params)]

use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicU32, Ordering, compiler_fence, fence};

pub fn all_valid_fences() {
    compiler_fence(Ordering::Acquire);
    compiler_fence(Ordering::Release);
    compiler_fence(Ordering::AcqRel);
    compiler_fence(Ordering::SeqCst);
    fence(Ordering::Acquire);
    fence(Ordering::Release);
    fence(Ordering::AcqRel);
    fence(Ordering::SeqCst);
}

pub fn guarded_fences(order: Ordering) {
    match order {
        Ordering::Acquire | Ordering::Release | Ordering::AcqRel | Ordering::SeqCst => {
            compiler_fence(order);
            fence(order);
        }
        _ => {}
    }
}

pub fn dynamic_compiler_fence(order: Ordering) {
    compiler_fence(order);
}
pub fn dynamic_hardware_fence(order: Ordering) {
    fence(order);
}

pub fn relaxed_compiler_fence() {
    dynamic_compiler_fence(Ordering::Relaxed);
}
pub fn relaxed_hardware_fence() {
    dynamic_hardware_fence(Ordering::Relaxed);
}

pub fn fences_preserve_local_storage(bytes: &mut [u8; 4]) {
    bytes[2] = 17;
    let cell = Cell::new(3_u8);
    let alias = &cell;
    compiler_fence(Ordering::Release);
    alias.set(8);
    fence(Ordering::Acquire);
    assert!(bytes[2] == 17);
    assert!(cell.get() == 8);
}

static COUNT: AtomicU32 = AtomicU32::new(0);

pub fn guarded_atomic_value() -> u32 {
    compiler_fence(Ordering::Acquire);
    let value = COUNT.load(Ordering::Relaxed);
    fence(Ordering::SeqCst);
    if value < 9 { value + 1 } else { 0 }
}

pub fn fences_do_not_establish_atomic_history() {
    COUNT.store(7, Ordering::Relaxed);
    compiler_fence(Ordering::SeqCst);
    fence(Ordering::SeqCst);
    assert!(COUNT.load(Ordering::Relaxed) == 7);
}

pub fn unsupported_payload_after_fence() {
    compiler_fence(Ordering::Acquire);
    let value = RefCell::new(0_u8);
    let _reader = value.borrow();
    let _writer = value.borrow_mut();
}

pub fn atomic_singlethreadfence() {
    panic!();
}
pub fn user_function_is_not_an_intrinsic() {
    atomic_singlethreadfence();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_orderings_and_local_storage_replay() {
        all_valid_fences();
        for order in [
            Ordering::Relaxed,
            Ordering::Acquire,
            Ordering::Release,
            Ordering::AcqRel,
            Ordering::SeqCst,
        ] {
            guarded_fences(order);
        }
        fences_preserve_local_storage(&mut [0; 4]);
    }

    #[test]
    #[should_panic]
    fn a_relaxed_compiler_fence_panics() {
        relaxed_compiler_fence();
    }

    #[test]
    #[should_panic]
    fn a_relaxed_hardware_fence_panics() {
        relaxed_hardware_fence();
    }
}

// Synthetic MIR exercises intrinsic boundaries without unsafe Rust implementations.
pub mod synthetic {
    use core::intrinsics::mir::*;
    use core::intrinsics::{AtomicOrdering, atomic_fence, atomic_singlethreadfence};
    #[custom_mir(dialect = "runtime", phase = "optimized")]
    pub fn direct_intrinsic() {
        mir! {
            {
                Call(RET = atomic_fence::<{ AtomicOrdering::Acquire }>(),
                    ReturnTo(done), UnwindUnreachable())
            }
            done = { Return() }
        }
    }
    #[custom_mir(dialect = "runtime", phase = "optimized")]
    pub fn invalid_intrinsic() {
        mir! {
            {
                Call(RET = atomic_singlethreadfence::<{ AtomicOrdering::Relaxed }>(),
                    ReturnTo(done), UnwindUnreachable())
            }
            done = { Return() }
        }
    }
}

pub fn strong_cas(expected: u32, replacement: u32) {
    match COUNT.compare_exchange(expected, replacement, Ordering::AcqRel, Ordering::Acquire) {
        Ok(old) => assert!(old == expected),
        Err(old) => assert!(old != expected),
    }
}

pub fn weak_cas(expected: u32, replacement: u32) {
    if let Ok(old) =
        COUNT.compare_exchange_weak(expected, replacement, Ordering::Relaxed, Ordering::SeqCst)
    {
        assert!(old == expected);
    }
}

pub fn weak_failure_can_match() {
    if let Err(old) = COUNT.compare_exchange_weak(7, 9, Ordering::Relaxed, Ordering::Relaxed) {
        assert!(old != 7);
    }
}

pub fn bad_cas_success_claim() {
    if let Ok(old) = COUNT.compare_exchange(7, 9, Ordering::Relaxed, Ordering::Relaxed) {
        assert!(old == 9);
    }
}

pub fn guarded_cas_ordering(success: Ordering, failure: Ordering) {
    match failure {
        Ordering::Relaxed | Ordering::Acquire | Ordering::SeqCst => {
            let _ = COUNT.compare_exchange(7, 9, success, failure);
        }
        _ => {}
    }
}

pub fn unchecked_cas_ordering(success: Ordering, failure: Ordering) {
    let _ = COUNT.compare_exchange(7, 9, success, failure);
}

pub fn cas_history_remains_arbitrary() {
    if COUNT
        .compare_exchange(7, 9, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        assert!(COUNT.load(Ordering::SeqCst) == 9);
    }
}

pub fn signed_cas() {
    let word = core::sync::atomic::AtomicI16::new(0);
    match word.compare_exchange(-2, 4, Ordering::Release, Ordering::Acquire) {
        Ok(old) => assert!(old == -2),
        Err(old) => assert!(old != -2),
    }
}

pub fn pointer_cas_is_unknown() {
    let word = core::sync::atomic::AtomicPtr::<()>::new(core::ptr::null_mut());
    let _ = word.compare_exchange(
        core::ptr::null_mut(),
        core::ptr::null_mut(),
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
}

#[cfg(test)]
#[test]
fn cas_result_relations_replay_for_every_small_initial_value() {
    for initial in 0..16 {
        COUNT.store(initial, Ordering::Relaxed);
        strong_cas(7, 9);
        COUNT.store(initial, Ordering::Relaxed);
        weak_cas(7, 9);
        guarded_cas_ordering(Ordering::Relaxed, Ordering::SeqCst);
    }
    signed_cas();
}
