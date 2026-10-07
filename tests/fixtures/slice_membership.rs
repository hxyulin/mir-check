#![no_std]
#![forbid(unsafe_code)]

use core::cell::Cell;

pub fn integer_member(values: [u16; 5]) {
    assert!(values.contains(&values[3]));
}

pub fn signed_member(values: [i128; 2]) {
    assert!(values.contains(&values[0]));
}

pub fn byte_member(values: [u8; 5]) {
    assert!(values.contains(&values[4]));
}

pub fn boolean_member(values: [bool; 3]) {
    assert!(values.contains(&values[1]));
}

pub fn character_member(values: [char; 3]) {
    assert!(values.contains(&values[2]));
}

pub fn empty_member(needle: u16) {
    assert!(![].contains(&needle));
}

pub fn bounded_bytes(values: &[u8]) {
    if values.len() <= 7 && !values.is_empty() {
        assert!(values.contains(&values[0]));
    }
}

pub fn shifted_bytes(values: [u8; 6]) {
    let [_, _, middle @ .., _] = &values;
    assert!(middle.contains(&values[3]));
}

pub fn range_view_is_unsupported(values: [u8; 6]) {
    assert!(values[2..5].contains(&values[3]));
}

pub fn shifted_view_excludes_endpoints() {
    let values = [11_u8, 22, 33, 44, 55];
    let [_, middle @ .., _] = &values;
    assert!(!middle.contains(&11));
    assert!(!middle.contains(&55));
}

pub fn float_member(value: f32) {
    if value == value {
        assert!([value].contains(&value));
    }
}

pub fn signed_zero_member() {
    assert!([0.0_f64].contains(&-0.0));
}

pub fn nan_is_not_a_member() {
    assert!(![f32::NAN].contains(&f32::NAN));
}

pub fn wrong_member(value: u16) {
    assert!([value].contains(&(value ^ 1)));
}

pub fn nan_reflexivity(value: f32) {
    assert!([value].contains(&value));
}

pub fn unbounded_bytes(values: &[u8], needle: u8) -> bool {
    values.contains(&needle)
}

pub fn over_budget_bytes(values: [u8; 129], needle: u8) -> bool {
    values.contains(&needle)
}

pub fn limit_bytes(values: [u8; 128]) {
    assert!(values.contains(&values[127]));
}

struct Dangerous;

impl PartialEq for Dangerous {
    fn eq(&self, _: &Self) -> bool {
        panic!("comparison fails");
    }
}

pub fn custom_equality_panics() {
    let _ = [Dangerous].contains(&Dangerous);
}

pub fn empty_custom_equality_does_not_run() {
    assert!(![].contains(&Dangerous));
}

struct Counted<'a>(&'a Cell<u8>, u8);

impl PartialEq for Counted<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.set(self.0.get() + 1);
        self.1 == other.1
    }
}

pub fn custom_equality_effects() {
    let count = Cell::new(0);
    assert!([Counted(&count, 4), Counted(&count, 4)].contains(&Counted(&count, 4)));
    assert!(count.get() == 1);
}

pub fn custom_equality_absent_effects() {
    let count = Cell::new(0);
    assert!(![Counted(&count, 1), Counted(&count, 2)].contains(&Counted(&count, 3)));
    assert!(count.get() == 2);
}

struct Counterfeit;

struct Asymmetric(u16);

impl PartialEq for Asymmetric {
    fn eq(&self, other: &Self) -> bool {
        self.0 < other.0
    }
}

pub fn custom_receiver_order() {
    assert!([Asymmetric(1)].contains(&Asymmetric(2)));
}

impl Counterfeit {
    fn contains(&self, _: &u16) -> bool {
        false
    }
}

pub fn counterfeit_contains() {
    assert!(Counterfeit.contains(&3));
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
#[test]
fn native_membership_agrees_with_numeric_and_custom_equality() {
    integer_member([2, 4, 6, 8, 10]);
    signed_member([-5, 7]);
    byte_member([1, 3, 5, 7, 9]);
    boolean_member([false, true, false]);
    character_member(['a', 'b', 'c']);
    empty_member(15);
    bounded_bytes(&[]);
    bounded_bytes(&[1, 2, 3]);
    shifted_bytes([1, 2, 3, 4, 5, 6]);
    shifted_view_excludes_endpoints();
    float_member(f32::NAN);
    float_member(1.5);
    signed_zero_member();
    nan_is_not_a_member();
    custom_equality_effects();
    custom_equality_absent_effects();
    empty_custom_equality_does_not_run();
    custom_receiver_order();
    assert!(std::panic::catch_unwind(custom_equality_panics).is_err());
    assert!(std::panic::catch_unwind(|| wrong_member(5)).is_err());
    assert!(std::panic::catch_unwind(|| nan_reflexivity(f32::NAN)).is_err());
    assert!(std::panic::catch_unwind(counterfeit_contains).is_err());
}
