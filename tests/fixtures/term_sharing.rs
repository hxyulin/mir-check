#![no_std]
#![forbid(unsafe_code)]

fn accumulated_signal(value: u32) -> u32 {
    let mut signal = value;
    let mut round = 0;
    while round < 20 {
        signal = signal.wrapping_add(signal);
        round += 1;
    }
    signal
}

pub fn the_accumulated_signal_matches_its_scale(value: u32) {
    assert!(accumulated_signal(value) == value.wrapping_mul(1 << 20));
}

pub fn an_incorrect_scale_is_detected(value: u32) {
    assert!(accumulated_signal(value) == value.wrapping_mul((1 << 20) + 1));
}

pub fn an_unresolved_callback_stays_unknown(callback: fn(u32) -> u32, value: u32) {
    assert!(callback(value) == accumulated_signal(value));
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    #[test]
    fn accumulated_signals_match_wrapping_native_arithmetic() {
        for value in [0, 1, 19, 4095, 1 << 20, u32::MAX / 2, u32::MAX] {
            super::the_accumulated_signal_matches_its_scale(value);
        }
        assert!(std::panic::catch_unwind(|| super::an_incorrect_scale_is_detected(1)).is_err());
    }
}
