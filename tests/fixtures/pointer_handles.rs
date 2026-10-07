#![no_std]
#![forbid(unsafe_code)]

use core::sync::atomic::{AtomicPtr, Ordering};

pub fn address_roundtrip(address: usize) {
    let pointer = address as *mut u16;
    assert!(pointer as usize == address);
    assert!(pointer.is_null() == (address == 0));
}

pub fn narrow_address(address: u64) {
    let pointer = address as *const u8;
    assert!(pointer as usize == address as usize);
    assert!(pointer as u8 == address as u8);
}

pub fn signed_address(address: i8) {
    let pointer = address as *mut ();
    assert!(pointer as usize == address as isize as usize);
}

pub fn recast(address: usize) {
    let pointer = address as *const u8;
    let other = pointer as *mut u16;
    assert!(pointer as usize == other as usize);
    assert!(other == address as *mut u16);
}

pub fn pointer_guards(left: usize, right: usize) {
    let left_pointer = left as *const ();
    let right_pointer = right as *const ();
    if left_pointer == right_pointer {
        assert!(left == right);
    }
    if left_pointer != right_pointer {
        assert!(left != right);
    }
}

pub fn null_constants() {
    const NIL: *mut u16 = core::ptr::null_mut();
    assert!(NIL as usize == 0);
    assert!(NIL.is_null());
}

struct Handle {
    cookie: *mut (),
    head: AtomicPtr<u16>,
}

impl Handle {
    fn new(cookie: usize) -> Self {
        Self {
            cookie: cookie as *mut (),
            head: AtomicPtr::new(core::ptr::null_mut()),
        }
    }
}

pub fn constructor_storage(cookie: usize) {
    let handle = Handle::new(cookie);
    assert!(handle.cookie as usize == cookie);
}

pub fn repeated_handles(address: usize) {
    let pointer = address as *mut ();
    let copies = [pointer; 3];
    assert!(copies[2] == pointer);
}

pub fn wrong_roundtrip(address: usize) {
    let pointer = address as *mut ();
    assert!(pointer as usize != address);
}

pub fn wrong_comparison(address: usize) {
    assert!(address as *mut () != address as *mut ());
}

pub fn a_constructor_panic_is_checked(cookie: usize) {
    assert!(cookie != 7);
    let _ = Handle::new(cookie);
}

pub fn arbitrary_pointer_inputs_are_unknown(pointer: *const u8) -> bool {
    pointer.is_null()
}

pub fn pointer_arithmetic_is_unknown(address: usize) -> usize {
    (address as *const u8).wrapping_add(1) as usize
}

pub fn atomic_memory_is_unknown(cookie: usize) -> bool {
    Handle::new(cookie).head.load(Ordering::Relaxed).is_null()
}

pub fn local_reference_addresses(value: u8) -> usize {
    &value as *const u8 as usize
}

pub fn allocation_provenance_is_unknown() -> usize {
    const POINTER: *const u8 = &11;
    POINTER as usize
}

pub fn pointer_metadata_is_unknown(bytes: &[u8]) -> usize {
    bytes as *const [u8] as *const u8 as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_operations_and_storage_match_native_casts() {
        null_constants();
        for address in [0, 1, 7, usize::MAX] {
            address_roundtrip(address);
            recast(address);
            constructor_storage(address);
            repeated_handles(address);
            for other in [0, 1, 7, usize::MAX] {
                pointer_guards(address, other);
            }
        }
        for address in [0, 1, u32::MAX as u64, u64::MAX] {
            narrow_address(address);
        }
        for address in [i8::MIN, -1, 0, 1, i8::MAX] {
            signed_address(address);
        }
    }

    #[test]
    #[should_panic]
    fn a_false_roundtrip_replays() {
        wrong_roundtrip(0);
    }

    #[test]
    #[should_panic]
    fn a_constructor_panic_replays() {
        a_constructor_panic_is_checked(7);
    }
}
