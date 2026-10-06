#![no_std]
#![forbid(unsafe_code)]

pub fn guarded(value: f32, min: f32, max: f32) {
    if min <= max {
        let result = value.clamp(min, max);
        assert!(value.is_nan() || (result >= min && result <= max));
        if value >= min && value <= max {
            assert!(result == value);
        }
    }
}

pub fn guarded_double(value: f64, min: f64, max: f64) {
    if min <= max {
        let result = value.clamp(min, max);
        assert!(value.is_nan() || (result >= min && result <= max));
    }
}

pub fn special_values(value: f32) {
    assert!(f32::NAN.clamp(-1.0, 1.0).is_nan());
    let result = value.clamp(f32::NEG_INFINITY, f32::INFINITY);
    assert!(result == value || value.is_nan());
    assert!(1.0 / (-0.0_f32).clamp(0.0, 0.0) == f32::NEG_INFINITY);
    assert!(1.0 / 0.0_f32.clamp(-0.0, -0.0) == f32::INFINITY);
    assert!(1.0 / (-1.0_f32).clamp(-0.0, 1.0) == f32::NEG_INFINITY);
}

pub fn unchecked(value: f32, min: f32, max: f32) -> f32 {
    value.clamp(min, max)
}

pub fn reversed(value: f64) -> f64 {
    value.clamp(2.0, -2.0)
}

pub fn nan_min(value: f32) -> f32 {
    value.clamp(f32::NAN, 1.0)
}

pub fn nan_max(value: f64) -> f64 {
    value.clamp(-1.0, f64::NAN)
}

pub fn bad_nan(value: f32) {
    let result = value.clamp(-1.0, 1.0);
    assert!(result >= -1.0 && result <= 1.0);
}

pub fn bad_zero() {
    assert!(1.0 / (-0.0_f64).clamp(0.0, 0.0) == f64::INFINITY);
}

pub struct Pretender;

impl Pretender {
    pub fn clamp(&self, _: f32, _: f32) -> f32 {
        panic!("user method");
    }
}

pub fn user_method() -> f32 {
    Pretender.clamp(-1.0, 1.0)
}

pub fn unsupported(value: f32) -> u32 {
    value.clamp(-1.0, 1.0).to_bits()
}
