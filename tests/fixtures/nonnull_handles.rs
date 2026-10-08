#![no_std]
#![forbid(unsafe_code)]
#![feature(custom_mir, core_intrinsics, sync_unsafe_cell)]

use core::intrinsics::mir::*;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicPtr, Ordering};

static ADDRESS: AtomicPtr<u32> = AtomicPtr::new(core::ptr::null_mut());

pub fn checked_handles_preserve_the_observed_address(address: usize) {
    if let Some(handle) = NonNull::new(address as *mut u32) {
        assert!(handle.as_ptr() as usize == address);
    } else {
        assert!(address == 0);
    }
}

pub fn a_nonnull_handle_does_not_require_pointee_alignment() {
    let handle = NonNull::new(1_usize as *mut u32).unwrap();
    assert!(handle.as_ptr() as usize == 1);
}

pub fn loaded_pointer_handles_keep_only_the_observed_address() {
    let pointer = ADDRESS.load(Ordering::Acquire);
    if let Some(handle) = NonNull::new(pointer) {
        assert!(handle.as_ptr() == pointer);
    } else {
        assert!(pointer.is_null());
    }
}

pub fn handles_of_caller_storage_preserve_their_address(value: &mut u32) {
    let pointer = &raw mut *value;
    let handle = NonNull::new(pointer).unwrap();
    assert!(handle.as_ptr() == pointer);
}

pub fn a_nonnull_handle_cannot_change_its_address(address: usize) {
    if let Some(handle) = NonNull::new(address as *mut u32) {
        assert!(handle.as_ptr() as usize != address);
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn reinterpret(pointer: *mut u32) -> NonNull<u32> {
    mir! { { RET = CastTransmute(pointer); Return() } }
}

pub fn a_guard_establishes_a_nonnull_conversion(address: usize) {
    if address != 0 {
        let handle = reinterpret(address as *mut u32);
        assert!(handle.as_ptr() as usize == address);
    }
}

pub fn zero_is_not_a_valid_nonnull_handle() -> usize {
    reinterpret(core::ptr::null_mut()).as_ptr() as usize
}

pub fn a_numeric_nonnull_handle_does_not_authorize_a_read(address: usize) -> u32 {
    if let Some(handle) = NonNull::new(address as *mut u32) {
        read_handle(handle)
    } else {
        0
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn read_handle(handle: NonNull<u32>) -> u32 {
    mir! {
        let reference: &u32;
        let borrowed: &NonNull<u32>;
        { borrowed = &handle;
          Call(reference = NonNull::<u32>::as_ref(borrowed),
            ReturnTo(read), UnwindUnreachable()) }
        read = { RET = *reference; Return() }
    }
}

#[cfg(test)]
#[test]
fn valid_address_only_operations_replay_natively() {
    for address in [0, 1, 16, 0x4400, usize::MAX] {
        checked_handles_preserve_the_observed_address(address);
        a_guard_establishes_a_nonnull_conversion(address);
    }
    a_nonnull_handle_does_not_require_pointee_alignment();
    for address in [0, 1, 16] {
        ADDRESS.store(address as *mut u32, Ordering::Release);
        loaded_pointer_handles_keep_only_the_observed_address();
        a_nonnull_wrapper_can_return_its_checked_pointer_bits(address);
    }
    let mut value = 8;
    handles_of_caller_storage_preserve_their_address(&mut value);
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn expose_handle(handle: NonNull<u32>) -> *mut u32 {
    mir! { { RET = CastTransmute(handle); Return() } }
}

pub fn a_nonnull_wrapper_can_return_its_checked_pointer_bits(address: usize) {
    if let Some(handle) = NonNull::new(address as *mut u32) {
        assert!(expose_handle(handle) as usize == address);
    }
}

type PointerSlot = Option<NonNull<u32>>;

#[custom_mir(dialect = "runtime", phase = "optimized")]
const fn erase_slot(
    value: PointerSlot,
) -> core::cell::SyncUnsafeCell<[core::mem::MaybeUninit<usize>; 1]> {
    mir! { { RET = CastTransmute(Move(value)); Return() } }
}

static PUBLISHED: core::cell::SyncUnsafeCell<[core::mem::MaybeUninit<usize>; 1]> = erase_slot(None);
static NONZERO_WORD: core::cell::SyncUnsafeCell<core::num::NonZeroU8> =
    core::cell::SyncUnsafeCell::new(core::num::NonZeroU8::MIN);

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn store_slot(pointer: *mut PointerSlot, value: PointerSlot) {
    mir! { { *pointer = value; Return() } }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn read_slot(pointer: *mut PointerSlot) -> PointerSlot {
    mir! { { RET = *pointer; Return() } }
}

pub fn checked_nonnull_values_can_be_published(address: usize) {
    let handle = NonNull::new(address as *mut u32);
    store_slot(PUBLISHED.get().cast(), handle);
}

pub fn published_pointer_payloads_remain_opaque(address: usize) -> PointerSlot {
    checked_nonnull_values_can_be_published(address);
    read_slot(PUBLISHED.get().cast())
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn store_nonzero_word(pointer: *mut core::num::NonZeroU8, value: core::num::NonZeroU8) {
    mir! { { *pointer = value; Return() } }
}

pub fn compiler_range_valid_values_can_be_published(value: core::num::NonZeroU8) {
    store_nonzero_word(NONZERO_WORD.get(), value);
}

#[cfg(test)]
#[test]
fn valid_pattern_constrained_stores_replay_natively() {
    for address in [0, 1, 16, 0x4400] {
        checked_nonnull_values_can_be_published(address);
        let observed = read_slot(PUBLISHED.get().cast());
        assert_eq!(
            observed.map(|handle| handle.as_ptr() as usize).unwrap_or(0),
            address
        );
    }
    compiler_range_valid_values_can_be_published(core::num::NonZeroU8::MIN);
    compiler_range_valid_values_can_be_published(core::num::NonZeroU8::new(40).unwrap());
}
