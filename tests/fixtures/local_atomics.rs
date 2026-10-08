#![no_std]
#![forbid(unsafe_code)]
#![feature(custom_mir, core_intrinsics)]

use core::sync::atomic::{AtomicU8, AtomicU16, Ordering};

fn owned_counter(value: u8) -> AtomicU8 {
    AtomicU8::new(value)
}

fn update(counter: &AtomicU8, value: u8) {
    counter.store(value, Ordering::Release);
}

pub fn constructed_counter_is_fresh() {
    let counter = AtomicU8::new(0);
    assert!(
        counter
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    );
    assert!(counter.load(Ordering::Acquire) == 1);
}

pub fn aliases_calls_and_owned_returns_share_history(value: u8) {
    let counter = owned_counter(value);
    let alias = &counter;
    assert!(alias.load(Ordering::Relaxed) == value);
    update(alias, 17);
    assert!(counter.swap(255, Ordering::SeqCst) == 17);
    assert!(alias.fetch_add(2, Ordering::Relaxed) == 255);
    assert!(counter.load(Ordering::Relaxed) == 1);
    assert!(counter.fetch_sub(2, Ordering::Relaxed) == 1);
    assert!(alias.load(Ordering::Relaxed) == 255);
}

fn retain_alias(counter: &AtomicU8) -> &AtomicU8 {
    counter
}

pub fn separate_aggregate_fields_keep_distinct_atomic_locations(value: u8) {
    struct Counters {
        left: AtomicU8,
        right: AtomicU8,
    }
    let counters = Counters {
        left: AtomicU8::new(value),
        right: AtomicU8::new(value),
    };
    let alias = retain_alias(&counters.left);
    update(alias, 17);
    assert!(counters.left.load(Ordering::Relaxed) == 17);
    assert!(counters.right.load(Ordering::Relaxed) == value);
}

pub fn strong_cas_preserves_failure_and_updates_success(value: u8) {
    let counter = AtomicU8::new(value);
    match counter.compare_exchange(7, 3, Ordering::SeqCst, Ordering::Acquire) {
        Ok(old) => {
            assert!(old == 7);
            assert!(counter.load(Ordering::Relaxed) == 3);
        }
        Err(old) => {
            assert!(old == value);
            assert!(old != 7);
            assert!(counter.load(Ordering::Relaxed) == value);
        }
    }
}

pub fn weak_cas_keeps_spurious_failure_history(value: u8) {
    let counter = AtomicU8::new(value);
    match counter.compare_exchange_weak(value, 9, Ordering::SeqCst, Ordering::Acquire) {
        Ok(old) => {
            assert!(old == value);
            assert!(counter.load(Ordering::Relaxed) == 9);
        }
        Err(old) => {
            assert!(old == value);
            assert!(counter.load(Ordering::Relaxed) == value);
        }
    }
}

pub fn signed_modular_updates() {
    let counter = core::sync::atomic::AtomicI8::new(127);
    assert!(counter.fetch_add(1, Ordering::Relaxed) == 127);
    assert!(counter.load(Ordering::Relaxed) == -128);
}

pub fn occupied_counter_is_not_fresh() {
    let counter = AtomicU8::new(1);
    assert!(
        counter
            .compare_exchange(0, 2, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    );
}

pub fn incorrect_retained_value_is_refuted() {
    let counter = AtomicU16::new(11);
    counter.store(13, Ordering::Relaxed);
    assert!(counter.load(Ordering::Relaxed) == 11);
}

pub fn weak_cas_can_fail_spuriously() {
    let counter = AtomicU8::new(0);
    assert!(
        counter
            .compare_exchange_weak(0, 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    );
}

fn load_with_order(counter: &AtomicU8, order: Ordering) {
    let _ = counter.load(order);
}

pub fn release_load_panics() {
    let counter = AtomicU8::new(0);
    load_with_order(&counter, Ordering::Release);
}

pub fn unsupported_pointer_escape() {
    let counter = AtomicU8::new(0);
    let _ = counter.as_ptr();
}

pub fn pointer_atomic_loads_are_conservative() {
    let counter = core::sync::atomic::AtomicPtr::<u8>::new(core::ptr::null_mut());
    let _ = counter.load(Ordering::Relaxed);
}

pub fn boundary(_counter: &AtomicU8) {}

pub fn trusted_boundary_loses_history() {
    let counter = AtomicU8::new(0);
    boundary(&counter);
    assert!(counter.load(Ordering::Relaxed) == 0);
}

pub fn a_new_owned_allocation_after_a_boundary_is_fresh() {
    let published = AtomicU8::new(0);
    boundary(&published);
    let fresh = AtomicU8::new(7);
    assert!(fresh.load(Ordering::Relaxed) == 7);
}

pub fn a_store_cannot_restore_exclusivity() {
    let counter = AtomicU8::new(0);
    boundary(&counter);
    counter.store(7, Ordering::Relaxed);
    assert!(counter.load(Ordering::Relaxed) == 7);
}

#[cfg(not(target_os = "none"))]
extern crate std;

#[cfg(not(target_os = "none"))]
pub fn unsupported_thread_publication() {
    let counter = AtomicU8::new(0);
    std::thread::scope(|scope| {
        scope.spawn(|| counter.store(1, Ordering::Relaxed));
        assert!(counter.load(Ordering::Relaxed) == 0);
    });
}

// An invalid lifetime shape is rejected, without executing it as native Rust.
pub mod synthetic {
    use super::*;
    use core::intrinsics::mir::*;

    #[custom_mir(dialect = "runtime", phase = "optimized")]
    fn escaped_local() -> &'static AtomicU8 {
        mir! {
            let counter: AtomicU8;
            {
                Call(counter = AtomicU8::new(0), ReturnTo(done), UnwindUnreachable())
            }
            done = {
                RET = &counter;
                Return()
            }
        }
    }

    pub fn local_borrow_cannot_escape() {
        let counter = escaped_local();
        assert!(counter.load(Ordering::Relaxed) == 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_histories_match_native_execution() {
        constructed_counter_is_fresh();
        signed_modular_updates();
        for value in u8::MIN..=u8::MAX {
            aliases_calls_and_owned_returns_share_history(value);
            separate_aggregate_fields_keep_distinct_atomic_locations(value);
            strong_cas_preserves_failure_and_updates_success(value);
            weak_cas_keeps_spurious_failure_history(value);
        }
    }

    #[test]
    #[should_panic]
    fn an_occupied_counter_cannot_be_claimed() {
        occupied_counter_is_not_fresh();
    }

    #[test]
    #[should_panic]
    fn the_previous_value_is_not_the_stored_value() {
        incorrect_retained_value_is_refuted();
    }
}

pub fn an_unused_weak_choice_does_not_explain_a_bounds_failure(index: u8) -> u8 {
    let counter = AtomicU8::new(0);
    let _result = counter.compare_exchange_weak(0, 1, Ordering::Relaxed, Ordering::Relaxed);
    [1, 2][usize::from(index)]
}
