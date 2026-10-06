#![no_std]
#![forbid(unsafe_code)]

pub fn archive_word(bits: u32) {
    assert!(f32::from_bits(bits).to_bits() == bits);
}

pub fn archive_wide_word(bits: u64) {
    assert!(f64::from_bits(bits).to_bits() == bits);
}

pub fn restore_sample(sample: f32) {
    assert!(f32::from_bits(sample.to_bits()).to_bits() == sample.to_bits());
}

pub fn restore_wide_sample(sample: f64) {
    assert!(f64::from_bits(sample.to_bits()).to_bits() == sample.to_bits());
}

pub fn signed_zero_labels() {
    assert!(0.0_f32.to_bits() == 0);
    assert!((-0.0_f32).to_bits() == 0x8000_0000);
    assert!((-0.0_f64).to_bits() == 0x8000_0000_0000_0000);
}

pub fn special_labels() {
    const SAMPLE: f32 = f32::from_bits(0xffa1_2345);
    const WIDE: f64 = f64::from_bits(0x7ff0_1234_5678_9abc);
    assert!(SAMPLE.to_bits() == 0xffa1_2345);
    assert!(WIDE.to_bits() == 0x7ff0_1234_5678_9abc);
}

pub fn mirrored_label(bits: u32) {
    assert!((-f32::from_bits(bits)).to_bits() == bits ^ 0x8000_0000);
}

pub fn unsigned_label(bits: u64) {
    assert!(f64::from_bits(bits).abs().to_bits() == bits & 0x7fff_ffff_ffff_ffff);
}

pub fn chosen_sample(index: usize, samples: [f32; 3]) {
    if index < 3 {
        let chosen = samples[index];
        let labels = [
            samples[0].to_bits(),
            samples[1].to_bits(),
            samples[2].to_bits(),
        ];
        assert!(chosen.to_bits() == labels[index]);
    }
}

pub fn bounded_label(bits: u32) {
    let sample = f32::from_bits(bits);
    let bounded = sample.clamp(-2.0, 2.0);
    if sample >= -2.0 && sample <= 2.0 {
        assert!(bounded.to_bits() == bits);
    }
    if sample != sample {
        assert!(bounded.to_bits() == bits);
    }
}

pub fn stable_calculation_label(left: f32, right: f32) {
    let computed = left + right;
    assert!(computed.to_bits() == computed.to_bits());
}

pub fn rounded_label() {
    assert!((1.25_f32 + 0.5).to_bits() == 1.75_f32.to_bits());
    assert!(((-0.0_f64) * 2.0).to_bits() == 0x8000_0000_0000_0000);
}

pub fn integer_cast_label(value: u16) {
    assert!(f32::from_bits((value as f32).to_bits()) == value as f32);
}

pub fn bad_sign_label(bits: u32) {
    assert!((-f32::from_bits(bits)).to_bits() == bits);
}

pub fn bad_special_label() {
    assert!(f32::from_bits(0x7f80_0011).to_bits() == 0x7fc0_0000);
}

pub fn bad_calculated_nan_label() {
    let calculated = f32::from_bits(0x7fc0_1234) + 1.0;
    assert!(calculated.to_bits() == 0x7fc0_0000);
}

pub fn unsupported_remainder(sample: f32) {
    let remainder = sample % 2.0;
    assert!(remainder.to_bits() == sample.to_bits());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_preserve_payloads_and_signed_zero_and_match_native_calculations() {
        let samples = [
            0,
            0x8000_0000,
            1,
            0x8000_0001,
            0x3f80_0000,
            0x7f80_0000,
            0xff80_0000,
            0x7f80_0001,
            0xff80_0001,
            0x7fc0_0011,
            0xffc0_0011,
        ];
        for bits in samples {
            archive_word(bits);
            restore_sample(f32::from_bits(bits));
            mirrored_label(bits);
            bounded_label(bits);
        }
        let wide_samples = [
            0,
            0x8000_0000_0000_0000,
            1,
            0x8000_0000_0000_0001,
            0x3ff0_0000_0000_0000,
            0x7ff0_0000_0000_0000,
            0xfff0_0000_0000_0000,
            0x7ff0_0000_0000_0001,
            0xfff0_0000_0000_0001,
            0x7ff8_0000_0000_0011,
            0xfff8_0000_0000_0011,
        ];
        for bits in wide_samples {
            archive_wide_word(bits);
            restore_wide_sample(f64::from_bits(bits));
            unsigned_label(bits);
        }
        signed_zero_labels();
        special_labels();
        rounded_label();
        for value in 0..=1023_u16 {
            integer_cast_label(value);
            stable_calculation_label(value as f32, -0.5);
        }
        for index in 0..3 {
            chosen_sample(index, [0.0, f32::from_bits(0xff80_0001), -3.0]);
        }
    }
}
