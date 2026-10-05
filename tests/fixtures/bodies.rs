#![no_std]
#![forbid(unsafe_code)]

pub fn identity(value: u8) -> u8 {
    value
}

pub fn never_called<T>(value: T) -> T {
    value
}

pub fn guarded(bytes: &[u8], index: usize) -> Option<u8> {
    if index < bytes.len() {
        Some(bytes[index])
    } else {
        None
    }
}

pub struct Reading(pub u8);

impl Reading {
    pub fn value(&self) -> u8 {
        self.0
    }
}
