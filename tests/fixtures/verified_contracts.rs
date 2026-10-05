#![no_std]
#![forbid(unsafe_code)]

use mir_contracts::{ensures, no_panic, requires};

#[no_panic]
#[requires(index < bytes.len())]
#[ensures(result <= 255)]
pub fn read(bytes: &[u8], index: usize) -> u8 {
    bytes[index]
}

pub fn guarded_read(bytes: &[u8], index: usize) -> u8 {
    if index < bytes.len() { read(bytes, index) } else { 0 }
}

pub fn bad_read(bytes: &[u8], index: usize) -> u8 {
    if index <= bytes.len() { read(bytes, index) } else { 0 }
}

#[no_panic]
#[requires(value < 16)]
#[ensures(result == value)]
pub fn bounded(value: u8) -> u8 {
    value
}

pub fn guarded_bound(value: u8) -> u8 {
    if value < 16 { bounded(value) } else { 0 }
}

pub fn bad_bound(value: u8) -> u8 {
    if value <= 16 { bounded(value) } else { 0 }
}

#[no_panic]
pub fn lying_no_panic(bytes: &[u8], index: usize) -> u8 {
    bytes[index]
}

#[ensures(result < 16)]
pub fn lying_postcondition(value: u8) -> u8 {
    value
}

pub fn use_lying_postcondition(bytes: &[u8; 16], value: u8) -> u8 {
    let index = lying_postcondition(value);
    bytes[index as usize]
}

#[requires(value < 16)]
#[ensures(result == value)]
pub fn snapshot(mut value: u8) -> u8 {
    let saved = value;
    value = 0;
    let _ = value;
    saved
}

#[requires(value < 16)]
#[ensures(result == value)]
pub fn changed_snapshot(mut value: u8) -> u8 {
    value = 0;
    value
}

#[requires(value < 0)]
pub fn inconsistent(value: u8) -> u8 {
    value
}

#[requires(missing < 16)]
pub fn missing_name(value: u8) -> u8 {
    value
}

#[requires(value < 256)]
pub fn out_of_range(value: u8) -> u8 {
    value
}

#[requires(value < 16u16)]
pub fn wrong_type(value: u8) -> u8 {
    value
}

#[requires(side_effect(value))]
pub fn impure(value: u8) -> u8 {
    value
}

#[requires(value + 1 < 16)]
pub fn arithmetic(value: u8) -> u8 {
    value
}

#[no_panic]
#[requires(-128 <= value && value <= 100)]
#[ensures(result == value)]
pub fn signed_bound(value: i8) -> i8 {
    value
}

pub fn valid_signed_call(value: i8) -> i8 {
    if value <= 100 { signed_bound(value) } else { 0 }
}

pub fn bad_signed_call(value: i8) -> i8 {
    if value <= 101 { signed_bound(value) } else { 0 }
}

#[requires(flag || value < 16)]
#[ensures(!flag || result < 16)]
pub fn boolean_contract(flag: bool, value: u8) -> u8 {
    if flag { 0 } else { value }
}

pub fn array_read(bytes: &[u8; 4]) -> u8 {
    read(bytes, 3)
}

pub fn bad_array_read(bytes: &[u8; 4]) -> u8 {
    read(bytes, 4)
}
