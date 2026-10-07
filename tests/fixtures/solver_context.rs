#![no_std]
#![forbid(unsafe_code)]

pub fn a_float_branch_keeps_its_integer_index_guard(index: u8, scale: f32) -> u8 {
    if scale.is_finite() && index < 6 {
        [7_u8; 6][usize::from(index)]
    } else {
        0
    }
}

pub fn a_float_constraint_can_be_needed_for_an_integer_proof(index: u8) {
    if index as f32 == 1.0 {
        assert!(index == 1);
    }
}

pub fn a_full_float_counterexample_must_keep_the_integer_input(index: u8) {
    if index as f32 == 1.0 {
        assert!(index == 0);
    }
}

pub fn unsupported_float_operations_stay_unknown(value: f32) -> f32 {
    value % 3.0
}

pub fn a_signed_index_keeps_both_range_guards(index: i8) -> u8 {
    if index >= 0 && index < 6 {
        [7_u8; 6][index as usize]
    } else {
        0
    }
}

pub fn a_widened_byte_stays_below_its_first_unrepresentable_bound(index: u8) {
    assert!(u32::from(index) < 256);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_indices_and_float_links_match_native_values() {
        for index in 0..=u8::MAX {
            a_float_constraint_can_be_needed_for_an_integer_proof(index);
            a_widened_byte_stays_below_its_first_unrepresentable_bound(index);
            for scale in [f32::NEG_INFINITY, f32::NAN, -0.0, 0.0, 1.0, f32::INFINITY] {
                let expected = if scale.is_finite() && index < 6 { 7 } else { 0 };
                assert_eq!(a_float_branch_keeps_its_integer_index_guard(index, scale), expected);
            }
        }
        for index in i8::MIN..=i8::MAX {
            let expected = if (0..6).contains(&index) { 7 } else { 0 };
            assert_eq!(a_signed_index_keeps_both_range_guards(index), expected);
        }
    }

    #[test]
    #[should_panic]
    fn the_linked_counterexample_replays_at_one() {
        a_full_float_counterexample_must_keep_the_integer_input(1);
    }
}
