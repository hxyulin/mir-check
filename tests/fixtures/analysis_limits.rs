#![no_std]
#![forbid(unsafe_code)]

pub fn completed_batch() {
    let mut count = 0;
    while count < 128 {
        count += 1;
    }
    assert!(count == 128);
}

pub fn later_failure() {
    let mut count = 0;
    while count < 128 {
        count += 1;
    }
    assert!(count == 127);
}

pub fn deep_checked(value: u8) {
    if value < 4 {
        rung0(value);
    }
}

#[inline(never)]
fn rung0(value: u8) {
    rung1(value);
}

#[inline(never)]
fn rung1(value: u8) {
    rung2(value);
}

#[inline(never)]
fn rung2(value: u8) {
    rung3(value);
}

#[inline(never)]
fn rung3(value: u8) {
    rung4(value);
}

#[inline(never)]
fn rung4(value: u8) {
    rung5(value);
}

#[inline(never)]
fn rung5(value: u8) {
    rung6(value);
}

#[inline(never)]
fn rung6(value: u8) {
    rung7(value);
}

#[inline(never)]
fn rung7(value: u8) {
    rung8(value);
}

#[inline(never)]
fn rung8(value: u8) {
    rung9(value);
}

#[inline(never)]
fn rung9(value: u8) {
    rung10(value);
}

#[inline(never)]
fn rung10(value: u8) {
    rung11(value);
}

#[inline(never)]
fn rung11(value: u8) {
    rung12(value);
}

#[inline(never)]
fn rung12(value: u8) {
    rung13(value);
}

#[inline(never)]
fn rung13(value: u8) {
    rung14(value);
}

#[inline(never)]
fn rung14(value: u8) {
    rung15(value);
}

#[inline(never)]
fn rung15(value: u8) {
    rung16(value);
}

#[inline(never)]
fn rung16(value: u8) {
    rung17(value);
}

#[inline(never)]
fn rung17(value: u8) {
    rung18(value);
}

#[inline(never)]
fn rung18(value: u8) {
    rung19(value);
}

#[inline(never)]
fn rung19(value: u8) {
    assert!(value < 4);
}

pub fn never_finishes() {
    loop {}
}

pub fn unsupported_callback(callback: fn(u8) -> u8) -> u8 {
    callback(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_work_and_the_guarded_call_chain_complete_natively() {
        completed_batch();
        for value in 0..=u8::MAX {
            deep_checked(value);
        }
    }

    #[test]
    #[should_panic]
    fn the_failure_after_bounded_work_replays() {
        later_failure();
    }
}
