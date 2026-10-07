#![no_std]
#![forbid(unsafe_code)]
#![feature(custom_mir, core_intrinsics)]

use core::intrinsics::mir::*;

pub fn raw_reborrows_retain_identity(value: u32) {
    let reference = &value;
    let first = &raw const *reference;
    let second = &raw const *reference;
    assert!(first == second);
    assert!(!first.is_null());
    assert!(first as usize % core::mem::align_of::<u32>() == 0);
    assert!(first.cast::<()>() as usize == second as usize);
}

pub fn mutable_reborrows_retain_identity(mut value: u32) {
    let reference = &mut value;
    let first = &raw mut *reference;
    let second = &raw mut *reference;
    assert!(first == second);
    assert!(first.cast_const() == second.cast_const());
}

pub fn address_creation_does_not_hide_a_bounds_failure(index: u8) -> u16 {
    raw_reborrows_retain_identity(7);
    [8, 9][usize::from(index)]
}

pub fn pointer_roundtrip_does_not_authorize_a_read(value: u32) -> u32 {
    let pointer = &raw const value;
    let integer = pointer as usize;
    synthetic::read(integer as *const u32)
}

pub fn pointer_offsets_remain_unknown(value: u32) -> usize {
    (&raw const value).wrapping_add(1) as usize
}

pub fn snapshots_cannot_supply_allocation_identity(value: &u32) -> usize {
    (&raw const *value) as usize
}

pub fn a_frame_pointer_cannot_escape() -> *const u32 {
    let value = 3;
    &raw const value
}

pub mod synthetic {
    use super::*;

    #[custom_mir(dialect = "runtime", phase = "optimized")]
    pub fn read(pointer: *const u32) -> u32 {
        mir! { { RET = *pointer; Return() } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracked_address_operations_preserve_native_identity() {
        for value in [0, 1, u32::MAX] {
            raw_reborrows_retain_identity(value);
            mutable_reborrows_retain_identity(value);
        }
    }

    #[test]
    #[should_panic]
    fn the_independent_bounds_failure_replays() {
        address_creation_does_not_hide_a_bounds_failure(2);
    }
}
