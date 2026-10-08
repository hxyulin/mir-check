#![no_std]
#![forbid(unsafe_code)]
#![feature(custom_mir, core_intrinsics)]

use core::intrinsics::mir::*;
use core::sync::atomic::{AtomicPtr, Ordering};

static LINK: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());

pub fn strong_success_returns_the_expected_address(expected: usize, replacement: usize) {
    let expected = expected as *mut u8;
    let result = LINK.compare_exchange(
        expected,
        replacement as *mut u8,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    match result {
        Ok(old) => assert!(old == expected),
        Err(old) => assert!(old != expected),
    }
}

pub fn weak_success_returns_the_expected_address(expected: usize) {
    let expected = expected as *mut u8;
    if let Ok(old) = LINK.compare_exchange_weak(
        expected,
        core::ptr::null_mut(),
        Ordering::Release,
        Ordering::Relaxed,
    ) {
        assert!(old == expected);
    }
}

pub fn weak_failure_may_return_the_expected_address() {
    if let Err(old) = LINK.compare_exchange_weak(
        core::ptr::null_mut(),
        1_usize as *mut u8,
        Ordering::Relaxed,
        Ordering::Relaxed,
    ) {
        assert!(!old.is_null());
    }
}

pub fn a_local_cas_does_not_assume_an_exclusive_history() {
    let link = AtomicPtr::<u8>::new(core::ptr::null_mut());
    assert!(
        link.compare_exchange(
            core::ptr::null_mut(),
            1_usize as *mut u8,
            Ordering::SeqCst,
            Ordering::Relaxed,
        )
        .is_ok()
    );
}

fn release() -> Ordering {
    Ordering::Release
}

pub fn release_failure_ordering_panics() {
    let _ = LINK.compare_exchange(
        core::ptr::null_mut(),
        core::ptr::null_mut(),
        Ordering::Relaxed,
        release(),
    );
}

pub fn guarded_failure_ordering(order: Ordering) {
    if matches!(
        order,
        Ordering::Relaxed | Ordering::Acquire | Ordering::SeqCst
    ) {
        let _ = LINK.compare_exchange_weak(
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            Ordering::Relaxed,
            order,
        );
    }
}

pub fn matching_bits_preserve_only_the_observed_address() {
    let mut byte = 7_u8;
    let expected = &mut byte as *mut u8;
    if let Ok(old) = LINK.compare_exchange(expected, expected, Ordering::Relaxed, Ordering::Relaxed)
    {
        let _reference = core::ptr::NonNull::new(old).unwrap();
        assert!(old as usize == expected as usize);
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn read_pointer(pointer: *mut u8) -> u8 {
    core::intrinsics::mir::mir! { { RET = *pointer; Return() } }
}

pub fn matching_bits_do_not_authorize_a_pointee_read() {
    let mut byte = 7_u8;
    let expected = &mut byte as *mut u8;
    if let Ok(old) = LINK.compare_exchange(expected, expected, Ordering::Relaxed, Ordering::Relaxed)
    {
        let _ = read_pointer(old);
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
pub fn uninitialized_pointer_storage_is_unknown(order: Ordering) {
    mir! {
        let link: AtomicPtr<u8>;
        let reference: &AtomicPtr<u8>;
        let pointer: *mut u8;
        let result: Result<*mut u8, *mut u8>;
        {
            pointer = 0_usize as *mut u8;
            reference = &link;
            Call(result = AtomicPtr::<u8>::compare_exchange(
                reference, pointer, pointer, order, order),
                ReturnTo(done), UnwindUnreachable())
        }
        done = { Return() }
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strong_and_weak_address_relations_hold() {
        strong_success_returns_the_expected_address(0, 1);
        weak_success_returns_the_expected_address(1);
        matching_bits_preserve_only_the_observed_address();
        guarded_failure_ordering(Ordering::Acquire);
        guarded_failure_ordering(Ordering::Release);
    }

    #[test]
    #[should_panic]
    fn release_is_invalid_for_a_failure_ordering() {
        release_failure_ordering_panics();
    }
}

static FLAG: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(5);

pub fn pointer_publication_invalidates_other_startup_histories() {
    let _before = FLAG.load(Ordering::Relaxed);
    let _ = LINK.compare_exchange(
        core::ptr::null_mut(),
        1_usize as *mut u8,
        Ordering::Release,
        Ordering::Relaxed,
    );
    assert!(FLAG.load(Ordering::Relaxed) == 5);
}

static COUNTER_ADDRESS: AtomicPtr<core::sync::atomic::AtomicU8> =
    AtomicPtr::new(core::ptr::null_mut());

pub fn pointer_publication_invalidates_owned_atomic_history() {
    let counter = core::sync::atomic::AtomicU8::new(5);
    let address = &counter as *const core::sync::atomic::AtomicU8;
    let _ = COUNTER_ADDRESS.compare_exchange(
        core::ptr::null_mut(),
        address as *mut core::sync::atomic::AtomicU8,
        Ordering::Release,
        Ordering::Relaxed,
    );
    assert!(counter.load(Ordering::Relaxed) == 5);
}

pub fn newly_created_atomics_after_pointer_publication_are_fresh() {
    let counter = core::sync::atomic::AtomicU8::new(5);
    let address = &counter as *const core::sync::atomic::AtomicU8;
    let _ = COUNTER_ADDRESS.compare_exchange(
        core::ptr::null_mut(),
        address as *mut core::sync::atomic::AtomicU8,
        Ordering::Release,
        Ordering::Relaxed,
    );
    let fresh = core::sync::atomic::AtomicU8::new(9);
    assert!(fresh.load(Ordering::Relaxed) == 9);
}
