#![no_std]
#![forbid(unsafe_code)]

use core::cell::Cell;

struct Record<'a> {
    log: &'a Cell<u16>,
    digit: u16,
}

impl Drop for Record<'_> {
    fn drop(&mut self) {
        self.log.set(self.log.get() * 10 + self.digit);
    }
}

struct Pair<'a> {
    first: Record<'a>,
    second: Record<'a>,
}

impl Drop for Pair<'_> {
    fn drop(&mut self) {
        self.first.log.set(1);
    }
}

pub fn destructor_then_fields() {
    let log = Cell::new(0);
    {
        let _pair = Pair {
            first: Record { log: &log, digit: 2 },
            second: Record { log: &log, digit: 3 },
        };
    }
    assert!(log.get() == 123);
}

pub fn tuple_fields_in_order() {
    let log = Cell::new(0);
    {
        let _pair = (Record { log: &log, digit: 2 }, Record { log: &log, digit: 3 });
    }
    assert!(log.get() == 23);
}

pub fn branch_and_explicit_drop(flag: bool) {
    let log = Cell::new(0);
    let record = Record { log: &log, digit: 7 };
    if flag {
        core::mem::drop(record);
    } else {
        let _moved = record;
    }
    assert!(log.get() == 7);
}

enum Choice<'a> {
    Empty,
    Full(Record<'a>),
}

pub fn only_the_active_variant_drops(flag: bool) {
    let log = Cell::new(0);
    {
        let _choice = if flag {
            Choice::Full(Record { log: &log, digit: 5 })
        } else {
            Choice::Empty
        };
    }
    assert!(log.get() == if flag { 5 } else { 0 });
}

pub fn moving_a_field_drops_it_once() {
    let log = Cell::new(0);
    let pair = (Record { log: &log, digit: 2 }, Record { log: &log, digit: 3 });
    let first = pair.0;
    core::mem::drop(first);
    core::mem::drop(pair.1);
    assert!(log.get() == 23);
}

struct Guard<'a>(&'a mut u8);

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        *self.0 = 3;
    }
}

pub fn mutable_guard_effects() {
    let mut value = 1;
    {
        let _guard = Guard(&mut value);
    }
    assert!(value == 3);
}

struct AssertOnDrop(u8);

impl Drop for AssertOnDrop {
    fn drop(&mut self) {
        assert!(self.0 < 4);
    }
}

pub fn bounded_destructor(value: u8) {
    if value < 4 {
        let _guard = AssertOnDrop(value);
    }
}

pub fn panicking_destructor(value: u8) {
    if value <= 4 {
        let _guard = AssertOnDrop(value);
    }
}

pub fn wrong_field_order() {
    let log = Cell::new(0);
    {
        let _pair = Pair {
            first: Record { log: &log, digit: 2 },
            second: Record { log: &log, digit: 3 },
        };
    }
    assert!(log.get() == 132);
}

pub fn wrong_mutable_guard_effect() {
    let mut value = 1;
    {
        let _guard = Guard(&mut value);
    }
    assert!(value == 1);
}

pub fn a_closure_drops_its_owned_capture() {
    let log = Cell::new(0);
    {
        let record = Record { log: &log, digit: 8 };
        let _callback = move || core::mem::drop(record);
    }
    assert!(log.get() == 8);
}

pub fn unreachable_unsupported_drop(value: u8) {
    if value < 4 && value >= 4 {
        slice_destructor_is_unsupported();
    }
}

pub fn slice_destructor_is_unsupported() {
    let log = Cell::new(0);
    let _records = [Record { log: &log, digit: 2 }, Record { log: &log, digit: 3 }];
}

pub fn coroutine_drop_is_unsupported() {
    let log = Cell::new(0);
    let _future = async move {
        let _record = Record { log: &log, digit: 2 };
        core::future::pending::<()>().await;
    };
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn actual_destructors_preserve_effects_order_and_moves() {
        destructor_then_fields();
        tuple_fields_in_order();
        mutable_guard_effects();
        moving_a_field_drops_it_once();
        a_closure_drops_its_owned_capture();
        unreachable_unsupported_drop(0);
        for flag in [false, true] {
            branch_and_explicit_drop(flag);
            only_the_active_variant_drops(flag);
        }
        for value in 0..=u8::MAX {
            bounded_destructor(value);
        }
        assert!(std::panic::catch_unwind(|| panicking_destructor(4)).is_err());
        assert!(std::panic::catch_unwind(wrong_field_order).is_err());
        assert!(std::panic::catch_unwind(wrong_mutable_guard_effect).is_err());
    }
}

fn acquire_leaf() -> u8 {
    assert!(false);
    0
}

fn release_leaf(_restore_state: u8) {
    assert!(false);
}

struct BoundaryGuard(u8);

impl Drop for BoundaryGuard {
    fn drop(&mut self) {
        release_leaf(self.0);
    }
}

pub fn guarded_leaf_boundary(value: u8) -> u8 {
    let _guard = BoundaryGuard(acquire_leaf());
    if value < 4 {
        [9, 18, 27, 36][usize::from(value)]
    } else {
        0
    }
}

pub fn broken_leaf_callback(value: u8) -> u8 {
    let _guard = BoundaryGuard(acquire_leaf());
    if value <= 4 {
        [9, 18, 27, 36][usize::from(value)]
    } else {
        0
    }
}
