#![no_std]
#![forbid(unsafe_code)]

fn gauge_code(reading: f32) -> u8 {
    ((reading.clamp(-1.5, 3.5) + 1.5) / 5.0 * 180.0) as u8
}

pub fn gauge_codes(readings: [f32; 3]) -> [u8; 3] {
    readings.map(gauge_code)
}

pub fn computed_roundtrip(sample: f32) {
    let computed = sample + 0.25;
    let restored = f32::from_bits(computed.to_bits());
    if computed == computed {
        assert!(restored == computed);
    } else {
        assert!(restored != restored);
    }
}

fn shifted(sample: f32) -> f32 {
    sample + 0.125
}

pub fn returned_roundtrip(sample: f32) {
    let computed = shifted(sample);
    let restored = f32::from_bits(computed.to_bits());
    if computed == computed {
        assert!(restored == computed);
    } else {
        assert!(restored != restored);
    }
}

pub fn transformed_roundtrip(sample: f32) {
    let computed = sample + 0.5;
    let changed = (-computed).abs().clamp(0.25, 4.0);
    let restored = f32::from_bits(changed.to_bits());
    if changed == changed {
        assert!(restored == changed);
    } else {
        assert!(restored != restored);
    }
}

pub fn selected_roundtrip(sample: f32, index: u8) {
    if index > 1 {
        return;
    }
    let readings = [sample + 1.0, sample * 2.0];
    let chosen = readings[index as usize];
    let restored = f32::from_bits(chosen.to_bits());
    if chosen == chosen {
        assert!(restored == chosen);
    } else {
        assert!(restored != restored);
    }
}

pub fn transitive_roundtrip(sample: f32) {
    let first = sample + 0.125;
    let copied = f32::from_bits(first.to_bits());
    let second = copied * 2.0;
    let restored = f32::from_bits(second.to_bits());
    let expected = first * 2.0;
    if expected == expected {
        assert!(restored == expected);
    } else {
        assert!(restored != restored);
    }
}

pub fn stable_copied_encoding(sample: f32) {
    let computed = sample + 0.25;
    let copied = computed;
    assert!(computed.to_bits() == copied.to_bits());
}

pub fn wrong_roundtrip(sample: f32) {
    let computed = sample + 0.25;
    let restored = f32::from_bits(computed.to_bits());
    assert!(restored == 0.0);
}

pub fn unsupported_remainder(reading: f32) -> f32 {
    reading % 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauge_and_encoding_cases_match_native_float_execution() {
        for bits in [
            0,
            1,
            0x8000_0000,
            0x8000_0001,
            0x3f80_0000,
            0xbf80_0000,
            0x7f7f_ffff,
            0xff7f_ffff,
            0x7f80_0000,
            0xff80_0000,
            0x7fc1_2345,
            0xffa1_2345,
        ] {
            let sample = f32::from_bits(bits);
            computed_roundtrip(sample);
            returned_roundtrip(sample);
            transformed_roundtrip(sample);
            transitive_roundtrip(sample);
            stable_copied_encoding(sample);
            for index in 0..2 {
                selected_roundtrip(sample, index);
            }
            let codes = gauge_codes([sample, -sample, 0.0]);
            for (reading, code) in [sample, -sample, 0.0].into_iter().zip(codes) {
                assert_eq!(code, gauge_code(reading));
                assert!(code <= 180);
            }
        }
    }
}
