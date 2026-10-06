#![no_std]
#![forbid(unsafe_code)]
#![feature(core_float_math)]

pub fn unary_binary32(value: f32) -> [f32; 6] {
    [
        core::f32::math::floor(value),
        core::f32::math::ceil(value),
        core::f32::math::trunc(value),
        core::f32::math::round(value),
        core::f32::math::round_ties_even(value),
        core::f32::math::sqrt(value),
    ]
}

pub fn unary_binary64(value: f64) -> [f64; 6] {
    [
        core::f64::math::floor(value),
        core::f64::math::ceil(value),
        core::f64::math::trunc(value),
        core::f64::math::round(value),
        core::f64::math::round_ties_even(value),
        core::f64::math::sqrt(value),
    ]
}

pub fn rounding_ties_and_directions() {
    assert!(core::f32::math::floor(-1.25) == -2.0);
    assert!(core::f64::math::ceil(-1.25) == -1.0);
    assert!(core::f32::math::trunc(-1.75) == -1.0);
    assert!(core::f64::math::trunc(1.75) == 1.0);
    assert!(core::f32::math::round(2.5) == 3.0);
    assert!(core::f64::math::round(-2.5) == -3.0);
    assert!(core::f32::math::round_ties_even(2.5) == 2.0);
    assert!(core::f64::math::round_ties_even(-3.5) == -4.0);
}

pub fn rounding_preserves_zero_and_infinity() {
    assert!(core::f32::math::trunc(-0.25).to_bits() == (-0.0_f32).to_bits());
    assert!(core::f64::math::ceil(-0.25).to_bits() == (-0.0_f64).to_bits());
    assert!(core::f32::math::round_ties_even(-0.5).to_bits() == (-0.0_f32).to_bits());
    assert!(core::f64::math::round(-0.0).to_bits() == (-0.0_f64).to_bits());
    assert!(core::f32::math::floor(f32::INFINITY) == f32::INFINITY);
    assert!(core::f64::math::ceil(f64::NEG_INFINITY) == f64::NEG_INFINITY);
    assert!(core::f32::math::round(f32::NAN).is_nan());
    assert!(core::f64::math::trunc(f64::NAN).is_nan());
    assert!(core::f32::math::floor(f32::from_bits(1)) == 0.0);
    assert!(core::f64::math::ceil(f64::from_bits(1)) == 1.0);
}

pub fn square_root_edges() {
    assert!(core::f32::math::sqrt(4.0) == 2.0);
    assert!(core::f64::math::sqrt(9.0) == 3.0);
    assert!(core::f32::math::sqrt(-1.0).is_nan());
    assert!(core::f64::math::sqrt(f64::NEG_INFINITY).is_nan());
    assert!(core::f32::math::sqrt(f32::INFINITY) == f32::INFINITY);
    assert!(core::f64::math::sqrt(f64::NAN).is_nan());
    assert!(core::f32::math::sqrt(-0.0).to_bits() == (-0.0_f32).to_bits());
    assert!(core::f64::math::sqrt(-0.0).to_bits() == (-0.0_f64).to_bits());
    assert!(core::f32::math::sqrt(f32::from_bits(2)) == f32::from_bits(0x1a80_0000));
    assert!(core::f64::math::sqrt(f64::from_bits(1)) == f64::from_bits(0x1e60_0000_0000_0000));
}

pub fn fused_multiply_add_rounds_once() {
    let result32 = core::f32::math::mul_add(
        f32::from_bits(0x3f80_0001),
        f32::from_bits(0x3f7f_fffe),
        -1.0,
    );
    assert!(result32 == f32::from_bits(0xa880_0000));
    let result64 = core::f64::math::mul_add(
        f64::from_bits(0x3ff0_0000_0000_0001),
        f64::from_bits(0x3fef_ffff_ffff_fffe),
        -1.0,
    );
    assert!(result64 == f64::from_bits(0xb970_0000_0000_0000));
    assert!(core::f32::math::mul_add(f32::INFINITY, 0.0, 1.0).is_nan());
    assert!(core::f64::math::mul_add(f64::INFINITY, 1.0, f64::NEG_INFINITY).is_nan());
    assert!(core::f32::math::mul_add(-0.0, 1.0, -0.0).to_bits() == (-0.0_f32).to_bits());
}

pub fn returned_result_has_stable_storage(value: f64) {
    let rounded = core::f64::math::floor(value);
    let copied = rounded;
    assert!(rounded.to_bits() == copied.to_bits());
    let decoded = f64::from_bits(rounded.to_bits());
    assert!(decoded == rounded || rounded.is_nan());
}

pub fn floor_keeps_guarded_indices_in_range(bytes: [u8; 7], value: f32) -> u8 {
    if value >= 0.0 && value < 7.0 {
        bytes[core::f32::math::floor(value) as usize]
    } else {
        0
    }
}

pub fn a_false_rounding_tie() {
    assert!(core::f32::math::round(2.5) == 2.0);
}

pub fn a_false_square_root_domain() {
    assert!(core::f64::math::sqrt(-1.0) >= 0.0);
}

pub fn a_false_separate_rounding_claim() {
    let product = core::f32::math::mul_add(
        f32::from_bits(0x3f80_0001),
        f32::from_bits(0x3f7f_fffe),
        -1.0,
    );
    assert!(product == 0.0);
}

pub fn a_rounded_index_can_reach_the_end(bytes: [u8; 7], value: f32) -> u8 {
    if value >= 0.0 && value < 7.0 {
        bytes[core::f32::math::ceil(value) as usize]
    } else {
        0
    }
}

fn floorf32(value: f32) -> f32 {
    assert!(value < 0.0);
    value
}

pub fn an_intrinsic_name_does_not_replace_a_user_body() {
    let _ = floorf32(1.0);
}

pub fn remainder_is_still_unsupported(value: f32) -> f32 {
    value % 2.5
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_special_values_and_mutations_match_the_expected_behavior() {
        rounding_ties_and_directions();
        rounding_preserves_zero_and_infinity();
        square_root_edges();
        fused_multiply_add_rounds_once();
        assert!(floor_keeps_guarded_indices_in_range([1, 2, 3, 4, 5, 6, 7], 6.25) == 7);
        assert!(floor_keeps_guarded_indices_in_range([1, 2, 3, 4, 5, 6, 7], f32::NAN) == 0);
        for value in [
            f64::NEG_INFINITY,
            -3.5,
            -0.0,
            0.0,
            f64::from_bits(1),
            f64::MIN_POSITIVE,
            2.5,
            f64::INFINITY,
            f64::NAN,
        ] {
            returned_result_has_stable_storage(value);
        }
        assert!(std::panic::catch_unwind(a_false_rounding_tie).is_err());
        assert!(std::panic::catch_unwind(a_false_square_root_domain).is_err());
        assert!(std::panic::catch_unwind(a_false_separate_rounding_claim).is_err());
        assert!(
            std::panic::catch_unwind(|| a_rounded_index_can_reach_the_end([0; 7], 6.25)).is_err()
        );
        assert!(std::panic::catch_unwind(an_intrinsic_name_does_not_replace_a_user_body).is_err());
    }
}
