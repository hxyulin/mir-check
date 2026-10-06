#![no_std]
#![forbid(unsafe_code)]

fn next_bin(bin: u8) -> u8 {
    if bin == 5 { 0 } else { bin + 1 }
}

pub fn a_pure_acyclic_entry(value: u8) {
    if value < 6 {
        assert!(next_bin(value) < 6);
    }
}

pub fn cycling_through_a_helper() -> ! {
    let mut bin = 0_u8;
    loop {
        assert!(bin < 6);
        bin = next_bin(bin);
    }
}

fn bad_next_bin(bin: u8) -> u8 {
    if bin == 6 { 0 } else { bin + 1 }
}

pub fn a_helper_with_a_bad_wrap() -> ! {
    let mut bin = 0_u8;
    loop {
        assert!(bin < 6);
        bin = bad_next_bin(bin);
    }
}

#[doc = "<!-- mir-check:v1:requires:bin < 6 -->"]
#[doc = "<!-- mir-check:v1:ensures:result > bin && final_bin == result -->"]
fn checked_advance(mut bin: u8) -> u8 {
    bin += 1;
    bin
}

pub fn checked_call_domains() -> ! {
    let mut bin = 0_u8;
    loop {
        bin = checked_advance(bin);
        if bin == 6 {
            bin = 0;
        }
    }
}

pub fn a_broken_call_domain() -> ! {
    let mut bin = 0_u8;
    loop {
        bin = checked_advance(bin);
        if bin == 7 {
            bin = 0;
        }
    }
}

#[doc = "<!-- mir-check:v1:ensures:result < 6 -->"]
fn a_misleading_return_contract() -> u8 {
    17
}

pub fn a_false_contract_cannot_hide_the_body() -> ! {
    loop {
        assert!(a_misleading_return_contract() < 6);
    }
}

#[doc = "<!-- mir-check:v1:requires:limit < 32 -->"]
#[doc = "<!-- mir-check:v1:ensures:result == limit && final_limit == 0 -->"]
pub fn count_down(mut limit: u8) -> u8 {
    let mut completed = 0_u8;
    while limit > 0 {
        limit -= 1;
        completed += 1;
    }
    completed
}

#[doc = "<!-- mir-check:v1:requires:limit < 32 -->"]
#[doc = "<!-- mir-check:v1:ensures:result == limit && final_limit == limit -->"]
pub fn count_up(limit: u8) -> u8 {
    let mut completed = 0_u8;
    while completed < limit {
        completed += 1;
    }
    completed
}

pub fn a_loop_inside_a_callee(seed: u8) -> ! {
    loop {
        let limited = seed & 31;
        assert!(count_up(limited) == limited);
    }
}

#[doc = "<!-- mir-check:v1:ensures:result < limit -->"]
pub fn a_false_root_postcondition(limit: u8) -> u8 {
    let mut result = 0_u8;
    while result < limit {
        result += 1;
    }
    result
}

#[doc = "<!-- mir-check:v1:ensures:unmodeled_predicate(result) -->"]
pub fn an_unsupported_postcondition(limit: u8) -> u8 {
    let mut result = 0_u8;
    while result < limit {
        result += 1;
    }
    result
}

fn return_original<T: Copy>(value: T) -> T {
    value
}

fn decode(packet: [u8; 4]) -> (u16, u8) {
    (
        ((packet[0] as u16) | ((packet[1] as u16) << 8)) & 255,
        packet[2] & 31,
    )
}

pub fn caller_state_survives_nested_calls(packet: [u8; 4]) -> ! {
    let mut bin = 0_u8;
    loop {
        let before = bin;
        let first = return_original(before);
        let second = return_original(513_u16);
        let decoded = decode(packet);
        assert!(first == before);
        assert!(second == 513);
        assert!(decoded.0 <= 255);
        assert!(decoded.1 <= 31);
        bin = next_bin(bin);
        assert!(bin < 6);
    }
}

pub fn a_delegating_entry() -> ! {
    cycling_through_a_helper()
}

fn recursive(value: u8) -> u8 {
    if value == 0 { 0 } else { recursive(value - 1) }
}

pub fn a_recursive_callee_remains_unknown() -> ! {
    loop {
        assert!(recursive(3) == 0);
    }
}

fn an_indirect_callee(callback: fn(u8) -> u8, value: u8) -> u8 {
    callback(value)
}

#[doc = "<!-- mir-check:v1:ensures:unmodeled_predicate(result) -->"]
pub fn an_unsupported_contract_on_an_endless_function() -> u8 {
    loop {}
}

pub fn an_unsupported_callee_remains_unknown() -> ! {
    loop {
        assert!(an_indirect_callee(next_bin, 0) == 1);
    }
}

#[doc = "<!-- mir-check:v1:requires:data[0] < 6 -->"]
#[doc = "<!-- mir-check:v1:ensures:result[0] > data[0] && final_data[0] == result[0] -->"]
fn advance_buffer(mut data: [u8; 4]) -> [u8; 4] {
    data[0] += 1;
    data
}

pub fn owned_byte_buffers_keep_their_entry_snapshots() -> ! {
    let mut data = [0_u8; 4];
    loop {
        data = advance_buffer(data);
        if data[0] == 6 {
            data[0] = 0;
        }
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_valid_counter_and_packet_preserves_the_declared_domains() {
        for bin in 0..6 {
            assert!(next_bin(bin) < 6);
            assert_eq!(checked_advance(bin), bin + 1);
            assert_eq!(advance_buffer([bin, 2, 3, 4]), [bin + 1, 2, 3, 4]);
        }
        for limit in 0..32 {
            assert_eq!(count_down(limit), limit);
            assert_eq!(count_up(limit), limit);
        }
        for byte in 0..=u8::MAX {
            let result = decode([byte; 4]);
            assert!(result.0 <= 255);
            assert!(result.1 <= 31);
        }
    }

    #[test]
    fn the_changed_wrap_reaches_a_real_panic() {
        assert!(std::panic::catch_unwind(a_helper_with_a_bad_wrap).is_err());
    }

    #[test]
    fn false_contracts_fail_their_claims_without_injected_runtime_checks() {
        assert_eq!(a_misleading_return_contract(), 17);
        assert!(std::panic::catch_unwind(a_false_contract_cannot_hide_the_body).is_err());
        assert_eq!(a_false_root_postcondition(4), 4);
        assert!(
            std::panic::catch_unwind(|| {
                assert!(a_false_root_postcondition(4) < 4);
            })
            .is_err()
        );
    }
}
