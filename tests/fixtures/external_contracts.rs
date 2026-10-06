#![no_std]
#![forbid(unsafe_code)]

use core::fmt::{self, Write};

pub struct Writer {
    pub len: u8,
}
impl Write for Writer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.len += text.len() as u8;
        Ok(())
    }
}

pub fn write_float(writer: &mut Writer, value: f32, precision: usize) -> fmt::Result {
    write!(writer, "{:.*}", precision, value)
}

pub fn caller(writer: &mut Writer, value: f32) {
    let _result = write_float(writer, value, 3);
    assert!(writer.len <= 32);
}

pub fn bad_bound(writer: &mut Writer, value: f32) {
    let _result = write_float(writer, value, 7);
}

pub fn result_failure(writer: &mut Writer, value: f32) {
    match write_float(writer, value, 3) {
        Ok(()) => (),
        Err(_) => panic!("writer failed"),
    }
}

pub fn ignored(writer: &mut Writer, value: f32) {
    let _result = write_float(writer, value, 3);
}

pub fn stale(writer: &mut Writer, value: f32) {
    writer.len = 7;
    let _result = write_float(writer, value, 3);
    assert!(writer.len == 7);
}

pub fn increment(value: u8) -> u8 {
    value + 1
}
pub fn checked_caller(value: u8) -> u8 {
    if value < 255 { increment(value) } else { 0 }
}
pub fn bad_checked_caller() -> u8 {
    increment(255)
}

pub fn assumed(value: u8) -> u8 {
    if value == 7 {
        panic!("actual body")
    }
    value
}
pub fn assumed_caller(value: u8) -> u8 {
    assumed(value)
}
pub fn unrelated() {}

pub fn generic<T>(value: T) -> T {
    value
}
pub fn generic_call(value: u8) -> u8 {
    generic(value)
}

pub fn reference(value: &u8) -> &u8 {
    value
}
pub fn reference_call(value: &u8) -> u8 {
    *reference(value)
}
