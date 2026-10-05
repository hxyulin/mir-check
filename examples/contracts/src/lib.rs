#![no_std]
#![forbid(unsafe_code)]

use mir_contracts::{ensures, no_panic, requires};

#[no_panic]
#[requires(index < bytes.len())]
pub fn read(bytes: &[u8], index: usize) -> u8 {
    bytes[index]
}

#[no_panic]
pub fn guarded_read(bytes: &[u8], index: usize) -> u8 {
    if index < bytes.len() {
        read(bytes, index)
    } else {
        0
    }
}

#[no_panic]
#[requires(value < 15)]
#[ensures(result < 16)]
pub fn bounded_increment(value: u8) -> u8 {
    value + 1
}

#[no_panic]
pub fn guarded_increment(value: u8) -> u8 {
    if value < 15 {
        bounded_increment(value)
    } else {
        0
    }
}
