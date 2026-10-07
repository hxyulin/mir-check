#![no_std]
#![feature(custom_mir, core_intrinsics, sync_unsafe_cell)]
#![forbid(unsafe_code)]

use core::sync::atomic::{AtomicU16, Ordering};

static FIRST: AtomicU16 = AtomicU16::new(0);
static SECOND: AtomicU16 = AtomicU16::new(0);
static OCCUPIED: AtomicU16 = AtomicU16::new(1);

struct Flags {
    left: AtomicU16,
    right: AtomicU16,
}

static FLAGS: Flags = Flags {
    left: AtomicU16::new(0),
    right: AtomicU16::new(0),
};

pub fn first_claim() {
    assert!(
        FIRST
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    );
}

pub fn repeated_claim_panics() {
    first_claim();
    first_claim();
}

pub fn an_occupied_initializer_panics() {
    assert!(
        OCCUPIED
            .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    );
}

pub fn distinct_static_locations_do_not_share_history() {
    first_claim();
    assert!(
        SECOND
            .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    );
    FLAGS.left.store(7, Ordering::Relaxed);
    assert!(FLAGS.right.load(Ordering::Relaxed) == 0);
    assert!(FLAGS.left.fetch_add(2, Ordering::Relaxed) == 7);
    assert!(FLAGS.left.load(Ordering::Relaxed) == 9);
}

pub fn spurious_failure_paths_keep_independent_histories() {
    let result = FIRST.compare_exchange_weak(0, 1, Ordering::Relaxed, Ordering::Relaxed);
    if result.is_err() {
        first_claim();
    }
    assert!(FIRST.load(Ordering::Relaxed) == 1);
}

pub fn weak_claim_can_fail_spuriously() {
    assert!(
        FIRST
            .compare_exchange_weak(0, 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    );
}

pub fn boundary() {}

pub fn a_boundary_invalidates_existing_history() {
    first_claim();
    boundary();
    assert!(FIRST.load(Ordering::Relaxed) == 1);
}

pub fn a_boundary_cannot_restore_an_unseen_initializer() {
    boundary();
    first_claim();
}

pub fn argument_roots_need_a_separate_startup_domain(_value: u16) {}

pub fn unsupported_startup_loop() {
    loop {
        core::hint::spin_loop();
    }
}

pub fn callback_updates_keep_static_history() {
    [1_u16, 2].into_iter().for_each(|value| {
        FIRST.fetch_add(value, Ordering::Relaxed);
    });
    assert!(FIRST.load(Ordering::Relaxed) == 3);
}

#[repr(C, align(2))]
struct ByteFlags {
    bytes: [core::sync::atomic::AtomicU8; 2],
}

static OVERLAP: core::cell::SyncUnsafeCell<ByteFlags> =
    core::cell::SyncUnsafeCell::new(ByteFlags {
        bytes: [
            core::sync::atomic::AtomicU8::new(0),
            core::sync::atomic::AtomicU8::new(0),
        ],
    });
static PUBLISHED: core::cell::SyncUnsafeCell<u16> = core::cell::SyncUnsafeCell::new(0);

pub fn overlapping_views_do_not_keep_independent_histories() {
    let byte = synthetic::share_byte(OVERLAP.get().cast());
    assert!(byte.load(Ordering::Relaxed) == 0);
    let whole = synthetic::share_word(OVERLAP.get().cast());
    assert!(whole.load(Ordering::Relaxed) == 0);
}

pub fn a_static_store_invalidates_startup_history() {
    first_claim();
    synthetic::publish(PUBLISHED.get(), 2);
    assert!(FIRST.load(Ordering::Relaxed) == 1);
}

pub mod synthetic {
    use core::intrinsics::mir::*;
    use core::sync::atomic::{AtomicU8, AtomicU16};

    #[custom_mir(dialect = "runtime", phase = "optimized")]
    pub fn share_word(pointer: *const AtomicU16) -> &'static AtomicU16 {
        mir! { { RET = &*pointer; Return() } }
    }

    #[custom_mir(dialect = "runtime", phase = "optimized")]
    pub fn share_byte(pointer: *const AtomicU8) -> &'static AtomicU8 {
        mir! { { RET = &*pointer; Return() } }
    }

    #[custom_mir(dialect = "runtime", phase = "optimized")]
    pub fn publish(pointer: *mut u16, value: u16) {
        mir! { { *pointer = value; RET = (); Return() } }
    }
}
