#![no_std]
#![forbid(unsafe_code)]

pub fn a_range_can_fill_borrowed_storage(bytes: &mut [u8; 7]) {
    for index in 0..7 {
        bytes[index] = 3;
    }
    assert!(bytes[6] == 3);
}

pub fn a_wrong_range_end(bytes: &mut [u8; 7]) {
    for index in 0..8 {
        bytes[index] = 3;
    }
}

pub fn a_long_range() {
    for value in 0_u16..12000 {
        assert!(value < 12000);
    }
}

pub fn a_signed_range() {
    for value in -5_i8..7 {
        assert!(value >= -5 && value < 7);
    }
}

pub fn a_maximum_endpoint() {
    for value in 253_u8..255 {
        assert!(value < 255);
    }
}

pub fn an_empty_reversed_range() {
    for _ in 7_u8..3 {
        panic!("a reversed range must be empty");
    }
}

pub fn a_range_body_can_break() {
    for value in 0_u16..60000 {
        if value == 9 {
            break;
        }
        assert!(value < 9);
    }
}

pub fn a_late_range_panic() {
    for value in 0_u16..16000 {
        assert!(value != 12000);
    }
}

pub fn unsupported_slice_iterator_storage(bytes: &[u8]) {
    for value in bytes {
        assert!(*value < 255);
    }
}

pub enum Mode {
    Idle,
    Active(u8),
}

pub fn tagged_loop_state() -> ! {
    let mut mode = Mode::Idle;
    loop {
        mode = match mode {
            Mode::Idle => Mode::Active(0),
            Mode::Active(value) if value < 3 => Mode::Active(value + 1),
            Mode::Active(value) => {
                assert!(value == 3);
                Mode::Idle
            }
        };
    }
}

pub fn an_incorrect_tagged_loop() -> ! {
    let mut mode = Mode::Idle;
    loop {
        mode = match mode {
            Mode::Idle => Mode::Active(0),
            Mode::Active(value) if value < 4 => Mode::Active(value + 1),
            Mode::Active(value) => {
                assert!(value == 3);
                Mode::Idle
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_boundaries_and_breaks_match_native_rust() {
        a_signed_range();
        a_maximum_endpoint();
        an_empty_reversed_range();
        a_range_body_can_break();
        a_long_range();
        a_custom_iterator_uses_its_actual_body();
        let mut bytes = [0; 7];
        a_range_can_fill_borrowed_storage(&mut bytes);
        assert_eq!(bytes, [3; 7]);
    }

    #[test]
    #[should_panic]
    fn a_changed_end_reaches_the_out_of_bounds_store() {
        a_wrong_range_end(&mut [0; 7]);
    }

    #[test]
    #[should_panic]
    fn a_custom_iterator_keeps_its_actual_nonzero_outputs() {
        a_custom_iterator_is_not_the_core_range_model();
    }

    #[test]
    #[should_panic]
    fn a_late_failure_is_reachable_beyond_the_unrolling_budget() {
        a_late_range_panic();
    }
}

pub struct Range {
    next_value: u8,
}

impl Iterator for Range {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        if self.next_value < 3 {
            self.next_value += 1;
            Some(self.next_value)
        } else {
            None
        }
    }
}

pub fn a_custom_iterator_uses_its_actual_body() {
    for value in (Range { next_value: 0 }) {
        assert!(value <= 3);
    }
}

pub fn a_custom_iterator_is_not_the_core_range_model() {
    for value in (Range { next_value: 0 }) {
        assert!(value == 0);
    }
}
