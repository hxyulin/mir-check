#![no_std]
#![forbid(unsafe_code)]

macro_rules! unsigned_checks {
    ($name:ident, $ty:ty) => {
        pub fn $name(left: $ty, right: $ty) {
            assert!(left.min(right) <= left && left.min(right) <= right);
            assert!(left.max(right) >= left && left.max(right) >= right);
            assert!(left.saturating_add(right) >= left);
            assert!(left.saturating_add(right) >= right);
            assert!(left.saturating_sub(right) <= left);
            if left == 0 {
                assert!(left.leading_zeros() == <$ty>::BITS);
                assert!(left.trailing_zeros() == <$ty>::BITS);
            } else {
                assert!(left.leading_zeros() < <$ty>::BITS);
                assert!(left.trailing_zeros() < <$ty>::BITS);
            }
            assert!(left.reverse_bits().reverse_bits() == left);
            assert!(left.swap_bytes().swap_bytes() == left);
        }
    };
}

macro_rules! signed_checks {
    ($name:ident, $ty:ty) => {
        pub fn $name(left: $ty, right: $ty) {
            assert!(left.min(right) <= left && left.min(right) <= right);
            assert!(left.max(right) >= left && left.max(right) >= right);
            if right >= 0 {
                assert!(left.saturating_add(right) >= left);
                assert!(left.saturating_sub(right) <= left);
            } else {
                assert!(left.saturating_add(right) <= left);
                assert!(left.saturating_sub(right) >= left);
            }
            if left == 0 {
                assert!(left.leading_zeros() == <$ty>::BITS);
                assert!(left.trailing_zeros() == <$ty>::BITS);
            } else {
                assert!(left.leading_zeros() < <$ty>::BITS);
                assert!(left.trailing_zeros() < <$ty>::BITS);
            }
            assert!(left.reverse_bits().reverse_bits() == left);
            assert!(left.swap_bytes().swap_bytes() == left);
        }
    };
}

unsigned_checks!(ticket_u8, u8);
unsigned_checks!(ticket_u16, u16);
unsigned_checks!(ticket_u32, u32);
unsigned_checks!(ticket_u64, u64);
unsigned_checks!(ticket_u128, u128);
unsigned_checks!(ticket_usize, usize);
signed_checks!(balance_i8, i8);
signed_checks!(balance_i16, i16);
signed_checks!(balance_i32, i32);
signed_checks!(balance_i64, i64);
signed_checks!(balance_i128, i128);
signed_checks!(balance_isize, isize);

pub fn leading_scan_matches_masks(value: u8) {
    let leading = if value & 128 != 0 {
        0
    } else if value & 64 != 0 {
        1
    } else if value & 32 != 0 {
        2
    } else if value & 16 != 0 {
        3
    } else if value & 8 != 0 {
        4
    } else if value & 4 != 0 {
        5
    } else if value & 2 != 0 {
        6
    } else if value & 1 != 0 {
        7
    } else {
        8
    };
    assert!(value.leading_zeros() == leading);
}

pub fn trailing_scan_matches_masks(value: u8) {
    let trailing = if value & 1 != 0 {
        0
    } else if value & 2 != 0 {
        1
    } else if value & 4 != 0 {
        2
    } else if value & 8 != 0 {
        3
    } else if value & 16 != 0 {
        4
    } else if value & 32 != 0 {
        5
    } else if value & 64 != 0 {
        6
    } else if value & 128 != 0 {
        7
    } else {
        8
    };
    assert!(value.trailing_zeros() == trailing);
}

pub fn wide_signed_boundary(left: i128, right: i128) {
    if left == i128::MAX && right == 1 {
        assert!(left.saturating_add(right) == i128::MAX);
    }
    if left == i128::MIN && right == 1 {
        assert!(left.saturating_sub(right) == i128::MIN);
    }
    if left == i128::MIN && right == -1 {
        assert!(left.saturating_add(right) == i128::MIN);
    }
    if left == i128::MAX && right == -1 {
        assert!(left.saturating_sub(right) == i128::MAX);
    }
    if left == -17 && right == 8 {
        assert!(left.saturating_add(right) == -9);
        assert!(left.saturating_sub(right) == -25);
    }
}

pub fn wide_unsigned_boundary(left: u128, right: u128) {
    if left == u128::MAX && right == 1 {
        assert!(left.saturating_add(right) == u128::MAX);
    }
    if left == 0 && right == 1 {
        assert!(left.saturating_sub(right) == 0);
    }
    if left == 23 && right == 5 {
        assert!(left.saturating_add(right) == 28);
        assert!(left.saturating_sub(right) == 18);
    }
}

pub fn transformed_word_matches_byte_layout(value: u32) {
    if value == 0x1234_5678 {
        assert!(value.swap_bytes() == 0x7856_3412);
        assert!(value.reverse_bits() == 0x1e6a_2c48);
    }
}

pub fn changed_saturation_result(left: u16, right: u16) {
    if left == u16::MAX && right == 1 {
        assert!(left.saturating_add(right) == 0);
    }
}

pub fn changed_minimum_direction(left: i32, right: i32) {
    assert!(left.min(right) >= right);
}

pub fn changed_zero_count(value: u64) {
    if value == 0 {
        assert!(value.leading_zeros() == 0);
    }
}

pub fn unsupported_function_pointer(callback: fn(u32) -> u32, value: u32) -> u32 {
    callback(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn changed_saturation_replays_a_failure() {
        changed_saturation_result(u16::MAX, 1);
    }

    #[test]
    #[should_panic]
    fn changed_minimum_replays_a_failure() {
        changed_minimum_direction(0, 1);
    }

    #[test]
    #[should_panic]
    fn changed_zero_count_replays_a_failure() {
        changed_zero_count(0);
    }

    #[test]
    fn byte_inputs_and_signed_boundaries_satisfy_the_fixture_properties() {
        for left in 0..=u8::MAX {
            leading_scan_matches_masks(left);
            trailing_scan_matches_masks(left);
            for right in 0..=u8::MAX {
                ticket_u8(left, right);
                balance_i8(left as i8, right as i8);
            }
        }
        for left in [i128::MIN, -17, -1, 0, 1, 23, i128::MAX] {
            for right in [i128::MIN, -1, 0, 1, 8, i128::MAX] {
                balance_i128(left, right);
                wide_signed_boundary(left, right);
            }
        }
        for left in [0, 1, 23, u128::MAX] {
            for right in [0, 1, 5, u128::MAX] {
                ticket_u128(left, right);
                wide_unsigned_boundary(left, right);
            }
        }
        transformed_word_matches_byte_layout(0x1234_5678);
    }
}
