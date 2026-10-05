#![no_std]
#![forbid(unsafe_code)]

pub fn guarded(bytes: &[u8], index: usize) -> u8 {
    if index < bytes.len() { bytes[index] } else { 0 }
}

pub fn off_by_one(bytes: &[u8], index: usize) -> u8 {
    if index <= bytes.len() {
        bytes[index]
    } else {
        0
    }
}

pub fn array_guarded(bytes: &[u8; 4], index: usize) -> u8 {
    if index < 4 { bytes[index] } else { 0 }
}

pub fn next_byte(bytes: &[u8], index: usize) -> u8 {
    if index >= bytes.len() || bytes.len() - index < 2 {
        return 0;
    }
    bytes[index + 1]
}

pub fn guarded_sum(left: u8, right: u8) -> u8 {
    if left <= 100 && right <= 100 {
        left + right
    } else {
        0
    }
}

pub fn overflowing_sum(left: u8, right: u8) -> u8 {
    left + right
}

pub fn guarded_division(left: i32, right: i32) -> i32 {
    if right == 0 || (left == i32::MIN && right == -1) {
        return 0;
    }
    left / right
}

pub fn unguarded_division(left: i32, right: i32) -> i32 {
    left / right
}

pub fn inner(bytes: &[u8; 4], index: usize) -> u8 {
    bytes[index]
}

pub fn valid_call(bytes: &[u8; 4]) -> u8 {
    inner(bytes, 3)
}

pub fn invalid_call(bytes: &[u8; 4]) -> u8 {
    inner(bytes, 4)
}

pub fn truncated_index(bytes: &[u8; 4], index: u16) -> u8 {
    let index = index as u8;
    if index < 4 { bytes[index as usize] } else { 0 }
}

pub fn stale_guard(bytes: &[u8], mut index: usize) -> u8 {
    if index < bytes.len() {
        index = bytes.len();
        bytes[index]
    } else {
        0
    }
}

pub fn loop_unknown(mut count: u8) -> u8 {
    while count > 0 {
        count -= 1;
    }
    count
}

pub fn false_assertion() {
    assert!(false);
}
