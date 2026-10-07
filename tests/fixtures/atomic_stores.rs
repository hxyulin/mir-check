#![no_std]
#![forbid(unsafe_code)]
#![feature(custom_mir, core_intrinsics, adt_const_params, sync_unsafe_cell)]

use core::sync::atomic::{AtomicPtr, Ordering};

static SLOT: AtomicPtr<u16> = AtomicPtr::new(core::ptr::null_mut());

pub fn numeric_handles_can_be_published() {
    SLOT.store(4_usize as *mut u16, Ordering::Release);
    SLOT.store(core::ptr::null_mut(), Ordering::Relaxed);
}

pub fn caller_storage_keeps_its_reference_evidence(value: &mut u16) {
    SLOT.store(&raw mut *value, Ordering::SeqCst);
}

fn acquire_order() -> Ordering {
    Ordering::Acquire
}

pub fn invalid_ordering_panics() {
    SLOT.store(core::ptr::null_mut(), acquire_order());
}

pub fn guarded_ordering(order: Ordering) {
    if matches!(
        order,
        Ordering::Relaxed | Ordering::Release | Ordering::SeqCst
    ) {
        SLOT.store(core::ptr::null_mut(), order);
    }
}

pub fn a_frame_pointer_cannot_escape_through_an_atomic_store() {
    let mut value = 3;
    SLOT.store(&raw mut value, Ordering::Relaxed);
}

pub fn address_bits_do_not_authorize_an_atomic_destination() {
    synthetic::store(core::ptr::null_mut(), core::ptr::null_mut());
}

pub fn equal_size_does_not_certify_a_destination_type() {
    synthetic::store_other(SLOT.as_ptr().cast(), core::ptr::null_mut());
}

pub fn volatile_stores_remain_unknown() {
    synthetic::volatile_store(SLOT.as_ptr(), core::ptr::null_mut());
}

pub fn pointer_loads_need_their_own_model() -> bool {
    SLOT.load(Ordering::Relaxed).is_null()
}

pub mod synthetic {
    use core::intrinsics::mir::*;
    use core::intrinsics::{AtomicOrdering, atomic_store};

    #[custom_mir(dialect = "runtime", phase = "optimized")]
    pub fn store(destination: *mut *mut u16, value: *mut u16) {
        mir! {
            {
                Call(RET = atomic_store::<*mut u16, {AtomicOrdering::Relaxed}, false>(
                    destination, value), ReturnTo(done), UnwindUnreachable())
            }
            done = { Return() }
        }
    }

    #[custom_mir(dialect = "runtime", phase = "optimized")]
    pub fn store_word(destination: *mut u16, value: u16) {
        mir! {
            {
                Call(RET = atomic_store::<u16, {AtomicOrdering::Relaxed}, false>(
                    destination, value), ReturnTo(done), UnwindUnreachable())
            }
            done = { Return() }
        }
    }

    #[custom_mir(dialect = "runtime", phase = "optimized")]
    pub fn store_other(destination: *mut *mut u32, value: *mut u32) {
        mir! {
            {
                Call(RET = atomic_store::<*mut u32, {AtomicOrdering::Relaxed}, false>(
                    destination, value), ReturnTo(done), UnwindUnreachable())
            }
            done = { Return() }
        }
    }

    #[custom_mir(dialect = "runtime", phase = "optimized")]
    pub fn volatile_store(destination: *mut *mut u16, value: *mut u16) {
        mir! {
            {
                Call(RET = atomic_store::<*mut u16, {AtomicOrdering::Relaxed}, true>(
                    destination, value), ReturnTo(done), UnwindUnreachable())
            }
            done = { Return() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_publications_and_guarded_orderings_do_not_panic() {
        numeric_handles_can_be_published();
        let mut value = 3;
        caller_storage_keeps_its_reference_evidence(&mut value);
        for order in [
            Ordering::Relaxed,
            Ordering::Acquire,
            Ordering::Release,
            Ordering::AcqRel,
            Ordering::SeqCst,
        ] {
            guarded_ordering(order);
        }
    }

    #[test]
    #[should_panic]
    fn an_acquire_store_panics() {
        invalid_ordering_panics();
    }
}

#[repr(C, align(16))]
struct AlignedWord {
    word: u16,
    guard: u8,
}

static WORD: core::cell::SyncUnsafeCell<AlignedWord> =
    core::cell::SyncUnsafeCell::new(AlignedWord { word: 0, guard: 1 });

pub fn an_initialized_integer_prefix_can_receive_an_atomic_store() {
    synthetic::store_word(WORD.get().cast(), 7);
}
