#![no_std]
#![forbid(unsafe_code)]

pub fn rotating_slots() -> ! {
    let mut slot = 0_u8;
    loop {
        assert!(slot < 8);
        if slot == 7 {
            slot = 0;
        } else {
            slot += 1;
        }
    }
}

pub fn a_late_bad_wrap() -> ! {
    let mut slot = 0_u16;
    loop {
        assert!(slot < 12000);
        if slot == 12000 {
            slot = 0;
        } else {
            slot += 1;
        }
    }
}

pub fn guarded_cycle(seed: u16) -> ! {
    let mut state = (0_u16, seed & 511);
    loop {
        assert!(state.0 < 12);
        assert!(state.1 <= 511);
        state.1 = (state.1 ^ state.0) & 511;
        if state.0 == 11 {
            state.0 = 0;
        } else {
            state.0 += 1;
        }
    }
}

pub fn a_broken_mask(seed: u16) -> ! {
    let mut channel = 0_u16;
    loop {
        assert!(channel <= 511);
        channel = seed;
    }
}

fn adjust(value: u8) -> u8 {
    value & 7
}

pub fn a_loop_with_a_scalar_helper(value: u8) -> ! {
    loop {
        assert!(adjust(value) < 8);
    }
}

#[doc = "<!-- mir-check:v1:ensures:result == 0 -->"]
pub fn a_loop_with_a_postcondition(limit: u16) -> u16 {
    let mut count = 0_u16;
    while count < limit {
        count += 1;
    }
    count
}

#[doc = "<!-- mir-check:v1:requires:seed <= 511 -->"]
pub fn a_constrained_register(seed: u16) -> ! {
    let mut reading = seed;
    loop {
        assert!(reading <= 511);
        reading ^= 255;
    }
}

#[doc = "<!-- mir-check:v1:requires:seed > 511 && seed <= 511 -->"]
pub fn an_inconsistent_loop_domain(seed: u16) -> ! {
    loop {
        assert!(seed <= 511);
    }
}

pub fn an_unresolved_loop(read: fn() -> u8) -> ! {
    loop {
        assert!(read() < 8);
    }
}

pub fn a_loop_that_can_exit(limit: u16) -> u16 {
    let mut count = 0_u16;
    while count < limit {
        count += 1;
    }
    assert!(count == limit);
    count
}

pub fn sampled_registers(packet: [u8; 24]) -> ! {
    let mut history = [0_u8; 8];
    let mut slot = 0_usize;
    let mut reading = 0_u16;
    loop {
        assert!(reading <= 1023);
        reading = ((packet[0] as u16) | ((packet[1] as u16) << 8)) & 1023;
        history[slot] = packet[2] & 127;
        if slot == 7 {
            slot = 0;
        } else {
            slot += 1;
        }
    }
}

pub fn a_bad_history_cursor(packet: [u8; 24]) -> ! {
    let mut history = [0_u8; 8];
    let mut slot = 0_usize;
    loop {
        history[slot] = packet[2] & 127;
        if slot == 8 {
            slot = 0;
        } else {
            slot += 1;
        }
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correct_cycles_preserve_their_ranges_for_every_register_byte() {
        for seed in 0..=u16::MAX {
            let mut slot = 0_u16;
            let mut reading = seed & 511;
            for _ in 0..48 {
                assert!(slot < 12);
                assert!(reading <= 511);
                reading = (reading ^ slot) & 511;
                slot = if slot == 11 { 0 } else { slot + 1 };
            }
        }
    }

    #[test]
    fn removing_a_mask_and_changing_a_wrap_both_reach_panics() {
        assert!(std::panic::catch_unwind(|| a_broken_mask(512)).is_err());
        assert!(std::panic::catch_unwind(|| a_bad_history_cursor([3; 24])).is_err());
    }

    #[test]
    fn the_bad_wrap_panics_after_more_than_the_unrolling_budget() {
        assert!(std::panic::catch_unwind(a_late_bad_wrap).is_err());
    }
}
