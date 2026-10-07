#![no_std]
#![forbid(unsafe_code)]

fn low_bits(value: u8) -> u8 {
    value & 3
}

fn high_bits(value: u8) -> u8 {
    (value >> 2) & 3
}

fn unchanged(value: u8) -> u8 {
    value
}

fn select<T: Copy>(value: T) -> T {
    value
}

fn return_pointer() -> fn(u8) -> u8 {
    low_bits
}

fn through_adapter<F: Fn(u8) -> u8>(callback: F, value: u8) -> u8 {
    callback(value)
}

fn through_mut_adapter<F: FnMut(u8) -> u8>(mut callback: F, value: u8) -> u8 {
    callback(value)
}

fn through_once_adapter<F: FnOnce(u8) -> u8>(callback: F, value: u8) -> u8 {
    callback(value)
}

pub fn bounded_argument(value: u8) {
    if value < 4 {
        let callback: fn(u8) -> u8 = low_bits;
        assert!(callback(value) == value);
    }
}

pub fn returned_target(value: u8) {
    let callback = return_pointer();
    assert!(callback(value) == value & 3);
}

pub fn selected_target(value: u8, high: bool) {
    let callback: fn(u8) -> u8 = if high { high_bits } else { low_bits };
    let index = callback(value);
    assert!(index < 4);
    let table = [7_u8, 9, 11, 13];
    assert!(table[index as usize] >= 7);
}

pub fn stored_target(value: u8) {
    let callbacks: (Option<fn(u8) -> u8>, fn(u8) -> u8) = (Some(low_bits), high_bits);
    if let Some(callback) = callbacks.0 {
        assert!(callback(value) == value & 3);
    }
    assert!((callbacks.1)(value) < 4);
}

pub fn generic_target(value: u8) {
    let callback: fn(u8) -> u8 = select::<u8>;
    assert!(callback(value) == value);
}

pub fn rust_call_adapters(value: u8) {
    let callback: fn(u8) -> u8 = low_bits;
    assert!(through_adapter(callback, value) < 4);
    assert!(through_mut_adapter(callback, value) < 4);
    assert!(through_once_adapter(callback, value) < 4);
}

fn increment(value: &mut u8) {
    *value = value.wrapping_add(1);
}

pub fn pointer_call_preserves_effects(value: u8) {
    let callback: fn(&mut u8) = increment;
    let mut count = value;
    callback(&mut count);
    assert!(count == value.wrapping_add(1));
}

#[track_caller]
fn caller_sensitive(value: u8) -> u8 {
    value & 3
}

pub fn caller_adapter_is_unknown(value: u8) {
    let callback: fn(u8) -> u8 = caller_sensitive;
    assert!(callback(value) < 4);
}

fn answer() -> u8 {
    6
}

pub fn empty_argument_tuple() {
    let callback: fn() -> u8 = answer;
    assert!(callback() == 6);
    assert!(true.then(callback) == Some(6));
}

fn bad_index(value: u8) {
    let data = [1_u8, 2, 3, 4];
    let _ = data[value as usize];
}

pub fn pointer_call_checks_panics(value: u8) {
    let callback: fn(u8) = bad_index;
    callback(value);
}

pub fn wrong_return_claim(value: u8) {
    let callback: fn(u8) -> u8 = low_bits;
    assert!(callback(value) < 3);
}

pub fn arbitrary_pointer_is_unknown(callback: fn(u8) -> u8, value: u8) {
    assert!(callback(value) < 4);
}

pub fn closure_pointer_is_unknown(value: u8) {
    let callback: fn(u8) -> u8 = |input| input & 3;
    assert!(callback(value) < 4);
}

pub fn numeric_pointer_is_unknown(value: u8) {
    let callback: fn(u8) -> u8 = low_bits;
    assert!(callback as usize != 0);
    assert!(callback(value) < 4);
}

pub fn different_selected_target(value: u8, change: bool) {
    let callback: fn(u8) -> u8 = if change { unchanged } else { low_bits };
    assert!(callback(value) < 4);
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_targets_and_effects_hold_for_every_byte() {
        for value in 0..=u8::MAX {
            super::returned_target(value);
            super::selected_target(value, false);
            super::selected_target(value, true);
            super::stored_target(value);
            super::generic_target(value);
            super::rust_call_adapters(value);
            super::pointer_call_preserves_effects(value);
        }
        super::empty_argument_tuple();
    }
}
