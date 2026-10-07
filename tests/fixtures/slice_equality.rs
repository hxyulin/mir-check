#![no_std]
#![forbid(unsafe_code)]

use core::cell::Cell;

pub fn byte_guard(left: &[u8], right: &[u8], index: usize) {
    if left.len() <= 8 && left == right && index < left.len() {
        assert!(left[index] == right[index]);
    }
}

pub fn byte_inequality(left: &[u8], right: &[u8]) {
    if left.len() <= 8 && right.len() <= 8 {
        assert!((left != right) == !(left == right));
    }
}

pub fn unequal_lengths_skip_elements(left: &[u8], right: &[u8]) {
    if left.len() != right.len() {
        assert!(left != right);
        assert!(!(left == right));
    }
}

pub fn empty_slices_are_equal(left: &[u8], right: &[u8]) {
    if left.is_empty() && right.is_empty() {
        assert!(left == right);
    }
}

pub fn array_and_slice(left: &[u8], right: [u8; 3]) {
    if left == right {
        assert!(left.len() == 3);
        assert!(left[2] == right[2]);
    }
    if right == left {
        assert!(right[0] == left[0]);
    }
}

pub fn prefixes(left: &[u8; 5], right: &[u8; 5]) {
    if left[..3] == right[..3] {
        assert!(left[2] == right[2]);
    }
}

pub fn different_tails() {
    assert!([1_u8, 2, 3, 4][..2] == [1_u8, 2, 0, 0][..2]);
    assert!([1_u8, 2, 3, 4][..2] != [1_u8, 3, 3, 4][..2]);
}

pub fn subviews(left: &[u8; 5], right: &[u8; 5]) {
    if left[1..4] == right[1..4] {
        assert!(left[3] == right[3]);
    }
}

pub fn floats(left: [f32; 2], right: [f32; 2]) {
    if left[..] == right[..] {
        assert!(left[0] == right[0]);
    }
    assert!([0.0_f32][..] == [-0.0_f32][..]);
    assert!([f32::NAN][..] != [f32::NAN][..]);
}

pub fn characters(left: [char; 2], right: [char; 2]) {
    if left[..] == right[..] {
        assert!(left[1] == right[1]);
    }
}

pub fn boolean_slices(left: [bool; 3], right: [bool; 3]) {
    if left[..] == right[..] {
        assert!(left[2] == right[2]);
    }
}

pub fn wrong_byte_guard(left: &[u8], right: &[u8]) {
    if left.len() == 1 && left == right {
        assert!(left[0] != right[0]);
    }
}

pub fn unbounded(left: &[u8], right: &[u8]) -> bool {
    left == right
}

pub fn over_budget(left: &[u8; 129], right: &[u8; 129]) -> bool {
    left[..] == right[..]
}

struct Seen<'a> {
    digit: u8,
    count: &'a Cell<u8>,
}

impl PartialEq for Seen<'_> {
    #[inline(never)]
    fn eq(&self, _: &Self) -> bool {
        panic!("the generic slice comparator calls ne");
    }

    #[inline(never)]
    fn ne(&self, other: &Self) -> bool {
        self.count.set(self.count.get() + 1);
        assert!(self.digit < 10);
        self.digit != other.digit
    }
}

pub fn custom_effects() {
    let count = Cell::new(0);
    let left = [
        Seen {
            digit: 1,
            count: &count,
        },
        Seen {
            digit: 2,
            count: &count,
        },
    ];
    let right = [
        Seen {
            digit: 1,
            count: &count,
        },
        Seen {
            digit: 2,
            count: &count,
        },
    ];
    assert!(left[..] == right[..]);
    assert!(count.get() == 2);
}

pub fn custom_mismatch() {
    let count = Cell::new(0);
    let left = [
        Seen {
            digit: 0,
            count: &count,
        },
        Seen {
            digit: 10,
            count: &count,
        },
    ];
    let right = [
        Seen {
            digit: 1,
            count: &count,
        },
        Seen {
            digit: 0,
            count: &count,
        },
    ];
    assert!(left[..] != right[..]);
    assert!(count.get() == 1);
}

pub fn custom_unequal_lengths() {
    let count = Cell::new(0);
    let left = [Seen {
        digit: 10,
        count: &count,
    }];
    let right: [Seen<'_>; 0] = [];
    assert!(left[..] != right[..]);
    assert!(count.get() == 0);
}

pub fn custom_panics(digit: u8) {
    let count = Cell::new(0);
    let left = [Seen {
        digit,
        count: &count,
    }];
    let right = [Seen {
        digit: 0,
        count: &count,
    }];
    let _ = left[..] == right[..];
}

struct Remainder(f32);
impl PartialEq for Remainder {
    fn eq(&self, other: &Self) -> bool {
        self.0 % 2.0 == other.0
    }
}

pub fn unsupported_comparison(value: f32) -> bool {
    [Remainder(value)][..] == [Remainder(1.0)][..]
}

struct LowerBound(u8);
impl PartialEq<u8> for LowerBound {
    fn eq(&self, other: &u8) -> bool {
        self.0 <= *other
    }
}

pub fn cross_type_order() {
    assert!([LowerBound(2)][..] == [3_u8][..]);
    assert!([LowerBound(4)][..] != [3_u8][..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guards_and_effects_match_native_comparisons() {
        for left in [0, 1, u8::MAX] {
            for right in [0, 1, u8::MAX] {
                byte_guard(&[left; 3], &[right; 3], 2);
                byte_inequality(&[left; 3], &[right; 3]);
                array_and_slice(&[left; 3], [right; 3]);
                subviews(&[left; 5], &[right; 5]);
            }
        }
        unequal_lengths_skip_elements(&[0; 256], &[1; 257]);
        empty_slices_are_equal(&[], &[]);
        different_tails();
        floats([f32::NAN, 0.0], [f32::NAN, -0.0]);
        characters(['x', 'y'], ['x', 'y']);
        boolean_slices([false; 3], [false; 3]);
        custom_effects();
        custom_mismatch();
        custom_unequal_lengths();
        cross_type_order();
    }

    #[test]
    #[should_panic]
    fn a_bad_guard_panics() {
        wrong_byte_guard(&[0], &[0]);
    }

    #[test]
    #[should_panic]
    fn a_reachable_custom_panic_replays() {
        custom_panics(10);
    }
}
