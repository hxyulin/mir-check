#![no_std]
#![forbid(unsafe_code)]

pub fn root(bytes: &[u8], index: usize) -> u8 {
    middle(bytes, index)
}

fn middle(bytes: &[u8], index: usize) -> u8 {
    indexed(bytes, index)
}

pub fn indexed(bytes: &[u8], index: usize) -> u8 {
    bytes[index]
}

pub fn sum(left: u8, right: u8) -> u8 {
    left + right
}

pub fn quotient(left: i32, right: i32) -> i32 {
    left / right
}

pub fn remainder(left: i32, right: i32) -> i32 {
    left % right
}

pub fn explicit_panic() {
    panic!("deliberate panic");
}

pub fn assertion(condition: bool) {
    assert!(condition);
}

pub fn unwrap_option(value: Option<u8>) -> u8 {
    value.unwrap()
}

pub fn clamp(value: f32, limit: f32) -> f32 {
    value.clamp(-limit, limit)
}

pub trait Controller {
    fn update(&mut self) -> u8;
}

pub fn dynamic(controller: &mut dyn Controller) -> u8 {
    controller.update()
}

pub fn generic<C: Controller>(controller: &mut C) -> u8 {
    controller.update()
}

pub fn indirect(function: fn() -> u8) -> u8 {
    function()
}

pub struct Bomb;

impl Drop for Bomb {
    fn drop(&mut self) {
        panic!("drop panic");
    }
}

pub fn destructor(value: Bomb) {
    core::mem::drop(value);
}

pub fn implicit_destructor(_value: Bomb) {}

pub fn recursive(bytes: &[u8], count: usize) -> u8 {
    if count == 0 {
        indexed(bytes, 0)
    } else {
        recursive(bytes, count - 1)
    }
}

pub fn panic_fmt(value: u8) -> u8 {
    value
}

pub fn misleading_name(value: u8) -> u8 {
    panic_fmt(value)
}
