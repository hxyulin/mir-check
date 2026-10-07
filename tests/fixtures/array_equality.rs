#![no_std]
#![forbid(unsafe_code)]

use core::cell::Cell;

pub fn integer_guard(left: [u16; 3], right: [u16; 3]) {
    if left == right {
        assert!(left[2] == right[2]);
    }
}

pub fn byte_guard(left: &[u8; 128], right: &[u8; 128]) {
    if left == right {
        assert!(left[127] == right[127]);
    }
}

pub fn boolean_guard(left: [bool; 4], right: [bool; 4]) {
    if left == right {
        assert!(left[1] == right[1]);
    }
    assert!((left != right) == !(left == right));
}

pub fn character_guard(left: [char; 2], right: [char; 2]) {
    if left == right {
        assert!(left[0] == right[0]);
    }
}

pub fn nested_guard(left: [[u8; 3]; 2], right: [[u8; 3]; 2]) {
    if left == right {
        assert!(left[1][2] == right[1][2]);
    }
}

#[derive(PartialEq)]
pub struct Label {
    pub code: u16,
    pub enabled: bool,
}

pub fn record_guard(left: &[Label; 3], right: &[Label; 3]) {
    if left == right {
        assert!(left[2].code == right[2].code);
    }
}

struct Threshold(u8);

impl PartialEq<u8> for Threshold {
    fn eq(&self, other: &u8) -> bool {
        self.0 >= *other
    }
}

pub fn different_element_types_keep_their_receiver_order() {
    assert!([Threshold(4)] == [3_u8]);
    assert!([Threshold(3)] != [4_u8]);
}

pub fn float_guard(left: [f64; 2], right: [f64; 2]) {
    if left == right {
        assert!(left[0] == right[0]);
    }
}

pub fn float_edges() {
    assert!([0.0_f32, -0.0] == [-0.0_f32, 0.0]);
    assert!([f32::NAN] != [f32::NAN]);
}

pub fn wrong_float_reflexivity(values: [f32; 2]) {
    assert!(values == values);
}

pub fn wrong_integer_equality(left: [u16; 3], right: [u16; 3]) {
    assert!(left == right);
}

pub fn over_budget(values: &[u8; 129]) -> bool {
    values == values
}

struct Remainder(f32);

impl PartialEq for Remainder {
    fn eq(&self, other: &Self) -> bool {
        self.0 % 2.0 == other.0
    }
}

pub fn unsupported_element_comparisons_stay_unknown(value: f32) -> bool {
    [Remainder(value)] == [Remainder(1.0)]
}

struct Observed<'a> {
    code: u8,
    hits: &'a Cell<u8>,
}

impl PartialEq for Observed<'_> {
    #[inline(never)]
    fn eq(&self, _: &Self) -> bool {
        panic!("array comparisons should use the overridden inequality");
    }

    #[inline(never)]
    fn ne(&self, other: &Self) -> bool {
        self.hits.set(self.hits.get() + 1);
        assert!(self.code != 99);
        self.code != other.code
    }
}

pub fn comparison_effects() {
    let hits = Cell::new(0);
    let left = [
        Observed {
            code: 3,
            hits: &hits,
        },
        Observed {
            code: 4,
            hits: &hits,
        },
    ];
    let right = [
        Observed {
            code: 3,
            hits: &hits,
        },
        Observed {
            code: 4,
            hits: &hits,
        },
    ];
    assert!(left == right);
    assert!(hits.get() == 2);
}

pub fn a_mismatch_skips_later_comparisons() {
    let hits = Cell::new(0);
    let left = [
        Observed {
            code: 1,
            hits: &hits,
        },
        Observed {
            code: 99,
            hits: &hits,
        },
    ];
    let right = [
        Observed {
            code: 2,
            hits: &hits,
        },
        Observed {
            code: 0,
            hits: &hits,
        },
    ];
    assert!(left != right);
    assert!(hits.get() == 1);
}

pub fn comparison_panics(code: u8) {
    let hits = Cell::new(0);
    let left = [Observed { code, hits: &hits }];
    let right = [Observed {
        code: 0,
        hits: &hits,
    }];
    let _ = left == right;
}

struct Inconsistent;

impl PartialEq for Inconsistent {
    fn eq(&self, _: &Self) -> bool {
        false
    }

    fn ne(&self, _: &Self) -> bool {
        false
    }
}

pub fn overridden_inequality_is_not_replaced_with_equality() {
    assert!([Inconsistent] == [Inconsistent]);
    assert!(!([Inconsistent] != [Inconsistent]));
}

pub fn empty_comparisons_do_not_call_elements() {
    let hits = Cell::new(0);
    let left: [Observed<'_>; 0] = [];
    let right: [Observed<'_>; 0] = [];
    assert!(left == right);
    assert!(!(left != right));
    assert!(hits.get() == 0);
}

struct Counterfeit;

impl Counterfeit {
    fn spec_eq(_: &[u8; 2], _: &[u8; 2]) -> bool {
        false
    }
}

pub fn similarly_named_methods_are_actual_calls() {
    assert!(Counterfeit::spec_eq(&[1, 2], &[1, 2]));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_guards_match_native_boundary_values() {
        for a in [0, 1, u16::MAX] {
            for b in [0, 1, u16::MAX] {
                integer_guard([a, 7, a], [b, 7, b]);
            }
        }
        for a in 0_u8..16 {
            for b in 0_u8..16 {
                let flags = |bits| core::array::from_fn(|i| bits & (1 << i) != 0);
                boolean_guard(flags(a), flags(b));
            }
        }
        for a in [0, 1, u8::MAX] {
            for b in [0, 1, u8::MAX] {
                byte_guard(&[a; 128], &[b; 128]);
                nested_guard([[a; 3]; 2], [[b; 3]; 2]);
            }
        }
        character_guard(['a', '\u{10ffff}'], ['b', '\u{10ffff}']);
    }

    #[test]
    fn float_edges_match_native_numeric_equality() {
        float_edges();
        assert!(unsupported_element_comparisons_stay_unknown(3.0));
        for a in [f64::NEG_INFINITY, -0.0, 0.0, 1.0, f64::INFINITY, f64::NAN] {
            for b in [f64::NEG_INFINITY, -0.0, 0.0, 1.0, f64::INFINITY, f64::NAN] {
                float_guard([a; 2], [b; 2]);
            }
        }
    }

    #[test]
    fn custom_comparisons_preserve_inequality_order_and_effects() {
        comparison_effects();
        different_element_types_keep_their_receiver_order();
        a_mismatch_skips_later_comparisons();
        overridden_inequality_is_not_replaced_with_equality();
        empty_comparisons_do_not_call_elements();
    }

    #[test]
    #[should_panic]
    fn a_reachable_element_panic_replays() {
        comparison_panics(99);
    }

    #[test]
    #[should_panic]
    fn nan_reflexivity_is_not_a_numeric_property() {
        wrong_float_reflexivity([f32::NAN, 0.0]);
    }
}
