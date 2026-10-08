#![no_std]
#![forbid(unsafe_code)]

use miren_contracts::{ensures, no_panic, requires};

#[no_panic]
#[requires(value > 0)]
#[ensures(result == value)]
pub const fn annotated(value: u8) -> u8 {
    value
}
