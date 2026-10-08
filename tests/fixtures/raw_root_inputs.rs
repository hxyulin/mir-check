#![no_std]
#![forbid(unsafe_code)]
#![feature(custom_mir, core_intrinsics)]

use core::intrinsics::mir::*;
use core::ptr::NonNull;

pub fn raw_inputs_preserve_observed_nullability(pointer: *const u8) {
    if pointer.is_null() {
        assert!(pointer as usize == 0);
    } else {
        assert!(pointer as usize != 0);
    }
}

pub fn raw_inputs_need_not_be_null(pointer: *mut u32) {
    assert!(pointer.is_null());
}

pub fn nonnull_inputs_have_nonzero_addresses(pointer: NonNull<u32>) {
    assert!(!pointer.as_ptr().is_null());
}

pub struct Address {
    pointer: *mut u32,
    tag: u8,
}

pub fn address_fields_are_symbolic(input: Address) {
    if input.tag == 7 && input.pointer.is_null() {
        assert!(input.pointer as usize == 0);
    }
}

pub fn root_addresses_do_not_grant_pointee_storage(pointer: *mut u32) -> u32 {
    if pointer.is_null() {
        0
    } else {
        read_pointer(pointer)
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn read_pointer(pointer: *mut u32) -> u32 {
    mir! { { RET = *pointer; Return() } }
}

pub fn fat_pointer_inputs_remain_unknown(pointer: *const [u8]) -> bool {
    pointer.is_null()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_address_guards_and_nonnull_inputs_are_valid() {
        raw_inputs_preserve_observed_nullability(core::ptr::null());
        raw_inputs_preserve_observed_nullability(8_usize as *const u8);
        nonnull_inputs_have_nonzero_addresses(NonNull::dangling());
        address_fields_are_symbolic(Address {
            pointer: core::ptr::null_mut(),
            tag: 7,
        });
    }
}
