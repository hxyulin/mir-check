#![no_std]
#![forbid(unsafe_code)]

use mir_contracts::{ensures, requires};

pub fn arithmetic(value: f32, divisor: f32) -> f32 {
    -((value + 2.0) * 3.0 - value / divisor)
}

pub fn float_methods(value: f32) {
    let magnitude = value.abs();
    assert!(magnitude >= 0.0 || value.is_nan());
    assert!(value.is_finite() == (magnitude < f32::INFINITY));
    assert!(value.min(f32::NAN) == value || value.is_nan());
    assert!(value.max(f32::NAN) == value || value.is_nan());
    let bounded = value.clamp(-2.0, 2.0);
    assert!((bounded >= -2.0 && bounded <= 2.0) || value.is_nan());
}

pub fn bad_min_zero() {
    assert!(1.0 / (-0.0_f32).min(0.0) == f32::INFINITY);
}

pub fn guarded_index(bytes: [u8; 4], index: f32) -> u8 {
    if index >= 0.0 && index < 4.0 {
        bytes[index as usize]
    } else {
        0
    }
}

pub fn bad_index(bytes: [u8; 4], index: f32) -> u8 {
    if index >= 0.0 && index <= 4.0 {
        bytes[index as usize]
    } else {
        0
    }
}

pub fn bad_nan(value: f64) {
    assert!(value == value);
}

pub fn bad_zero(value: f32) {
    if value == 0.0 {
        assert!(1.0 / value == f32::INFINITY);
    }
}

pub fn cast_edges(value: f64) {
    if value != value {
        assert!(value as i32 == 0);
        assert!(value as u128 == 0);
    }
    if value >= 2_147_483_648.0 {
        assert!(value as i32 == i32::MAX);
    }
    if value <= -2_147_483_648.0 {
        assert!(value as i32 == i32::MIN);
    }
    if value <= 0.0 {
        assert!(value as u32 == 0);
    }
}

pub fn casts_and_rounding(value: u8, other: f32) {
    assert!((value as f32) as u8 == value);
    assert!((value as f64) as u8 == value);
    assert!((other as f64) as f32 == other || other != other);
    assert!((16_777_217_u32 as f32) as u32 == 16_777_216);
    assert!((-0.75_f32) as i8 == 0);
    assert!((300.0_f32) as u8 == 255);
    assert!((f32::INFINITY) as i128 == i128::MAX);
    assert!((f32::NEG_INFINITY) as i128 == i128::MIN);
    assert!((f32::MAX) as u128 != u128::MAX);
}

#[requires(value >= -2.0 && 2.0 > value)]
#[ensures(result == value)]
pub fn bounded(value: f32) -> f32 {
    value
}

pub fn guarded_call(value: f32) -> f32 {
    if value >= -2.0 && value < 2.0 {
        bounded(value)
    } else {
        0.0
    }
}

pub fn bad_call(value: f32) -> f32 {
    if value >= -2.0 && value <= 2.0 {
        bounded(value)
    } else {
        0.0
    }
}

pub fn unsupported_remainder(value: f32) -> f32 {
    value % 3.0
}

pub fn unsupported_bits(value: f32) -> u32 {
    value.to_bits()
}
