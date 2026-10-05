#![no_std]
#![forbid(unsafe_code)]

use mir_contracts::{ensures, requires};

#[repr(i8)]
pub enum Mode {
    Idle = -3,
    Running = 5,
    Emergency = 7,
}

pub fn discriminants(mode: Mode) {
    let code = mode as i8;
    assert!(code == -3 || code == 5 || code == 7);
}

pub fn bad_variant(mode: Mode) {
    if let Mode::Emergency = mode {
        panic!("emergency");
    }
}

pub enum Reading {
    Missing,
    Sample { index: usize, value: f32 },
    Error(u8),
}

pub fn guarded(reading: &Reading, bytes: [u8; 4]) -> u8 {
    match reading {
        Reading::Sample { index, value } if *value > 0.0 && *index < bytes.len() => bytes[*index],
        Reading::Error(error) => *error,
        _ => 0,
    }
}

pub fn bad_payload(reading: Reading, bytes: [u8; 4]) -> u8 {
    match reading {
        Reading::Sample { index, .. } if index <= bytes.len() => bytes[index],
        _ => 0,
    }
}

#[requires(match index { None => true, Some(value) => value < 4 })]
pub fn bounded_option(index: Option<usize>, bytes: [u8; 4]) -> u8 {
    match index {
        None => 0,
        Some(index) => bytes[index],
    }
}

pub fn guarded_option(index: Option<usize>, bytes: [u8; 4]) -> u8 {
    if index.is_none() || index.is_some_and(|index| index < 4) {
        bounded_option(index, bytes)
    } else {
        0
    }
}

pub fn bad_option(index: Option<usize>, bytes: [u8; 4]) -> u8 {
    if index.is_none() || index.is_some_and(|index| index <= 4) {
        bounded_option(index, bytes)
    } else {
        0
    }
}

#[ensures(match value { None => result == 0, Some(input) => result == input })]
pub fn snapshot(value: Option<u8>) -> u8 {
    value.unwrap_or(0)
}

pub fn result_payload(value: Result<u8, u16>) {
    match value {
        Ok(value) => assert!((value as u16) <= 255),
        Err(value) => assert!((value as u32) <= 65_535),
    }
}

pub fn generic_enum<T>(value: Option<T>) -> bool {
    value.is_some()
}

pub fn mutable_enum(value: &mut Reading) {
    *value = Reading::Missing;
}

pub fn enum_slice(values: &[Reading]) -> bool {
    values.is_empty()
}

pub enum Large {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
}

pub fn large(value: Large) -> bool {
    matches!(value, Large::A)
}

pub enum Empty {}

pub fn empty(value: Empty) -> bool {
    match value {}
}
