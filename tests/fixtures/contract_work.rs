#![no_std]
#![forbid(unsafe_code)]
#![feature(core_intrinsics)]
// The pinned compiler regression calls its safe optimization hint directly.
#![allow(internal_features)]

fn choose_offset(value: u8, fast: bool) -> u8 {
    if fast { value + 1 } else { value }
}

pub fn bounded_hint(value: u8) {
    if value < u8::MAX {
        let result = choose_offset(value, core::intrinsics::is_val_statically_known(value));
        assert!(result >= value);
    }
}

pub fn unchecked_hint(value: u8) -> u8 {
    choose_offset(value, core::intrinsics::is_val_statically_known(value))
}

pub fn independent_hints(value: u8) {
    assert!(
        core::intrinsics::is_val_statically_known(value)
            == core::intrinsics::is_val_statically_known(value)
    );
}

fn is_val_statically_known(value: u8) -> bool {
    value + 1 == 0
}

pub fn unrelated_hint(value: u8) -> bool {
    is_val_statically_known(value)
}

pub fn hint_pointer(value: &u8) -> bool {
    core::intrinsics::is_val_statically_known(value)
}

pub fn guarded_power(value: u8) {
    if value <= 15 {
        assert!(value.pow(2) >= value);
    }
}

pub fn aliased_arguments(value: u8) -> u8 {
    value
}

pub fn alias_pair(first: u8, second: u8) -> (u8, u8) {
    (first, second)
}

pub fn calls_alias_pair(value: u8) -> (u8, u8) {
    alias_pair(value, 0)
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
#[test]
fn both_optimization_paths_preserve_bounds_and_the_overflow_path_panics() {
    for value in 0..u8::MAX {
        for fast in [false, true] {
            let result = choose_offset(value, fast);
            assert!(result >= value);
        }
        bounded_hint(value);
    }
    for value in 0..=u8::MAX {
        guarded_power(value);
    }
    assert!(std::panic::catch_unwind(|| choose_offset(u8::MAX, true)).is_err());
    assert!(std::panic::catch_unwind(|| unrelated_hint(u8::MAX)).is_err());
}
