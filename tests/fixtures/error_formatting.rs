#![no_std]
#![forbid(unsafe_code)]
#![feature(custom_mir, core_intrinsics)]

use core::cell::Cell;
use core::fmt::{Debug, Formatter, Result as FormatResult, Write};
use core::intrinsics::mir::*;

#[derive(Debug)]
struct Issue(u8);

fn checked(value: u8) -> Result<u8, Issue> {
    if value < 8 {
        Ok(value)
    } else {
        Err(Issue(value))
    }
}

pub fn guarded_unwrap(value: u8) {
    if value < 8 {
        assert!(checked(value).unwrap() == value);
    }
}

pub fn guarded_expect(value: u8) {
    if value < 8 {
        assert!(checked(value).expect("a bounded sample") == value);
    }
}

pub fn unchecked_unwrap(value: u8) {
    let _ = checked(value).unwrap();
}

pub fn unchecked_expect(value: u8) {
    let _ = checked(value).expect("a bounded sample");
}

pub fn error_payload(value: u8) {
    if value >= 8 {
        assert!(checked(value).unwrap_err().0 == value);
        assert!(checked(value).expect_err("a rejected sample").0 == value);
    }
}

pub fn unwrap_err_on_success(value: u8) {
    let _ = Ok::<u8, Issue>(value).unwrap_err();
}

pub fn expect_err_on_success(value: u8) {
    let _ = Ok::<u8, Issue>(value).expect_err("a rejected sample");
}

struct Counted<'a> {
    calls: &'a Cell<u8>,
    value: u8,
}

impl Debug for Counted<'_> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FormatResult {
        self.calls.set(self.calls.get().wrapping_add(1));
        write!(formatter, "{}", self.value)
    }
}

fn ignore_debug(_value: &dyn Debug) -> bool {
    true
}

pub fn coercing_and_passing_debug_references_does_not_call_the_formatter(value: u8) {
    let calls = Cell::new(0);
    let counted = Counted {
        calls: &calls,
        value,
    };
    let object: &dyn Debug = &counted;
    let stored = (Some(object), value);
    assert!(ignore_debug(stored.0.unwrap()));
    assert!(calls.get() == 0);
}

pub fn reference_return_preserves_caller_storage(value: u8) {
    let calls = Cell::new(0);
    let counted = Counted {
        calls: &calls,
        value,
    };
    assert!(ignore_debug(pass_debug(&counted)));
    assert!(calls.get() == 0);
}

fn pass_debug(value: &dyn Debug) -> &dyn Debug {
    value
}

pub fn debug_constructor_does_not_hide_a_later_panic(value: u8) {
    let object: &dyn Debug = &value;
    assert!(ignore_debug(object));
    let samples = [2_u8, 3, 5, 7];
    let _ = samples[value as usize];
}

struct Sink;

impl Write for Sink {
    fn write_str(&mut self, _text: &str) -> FormatResult {
        Ok(())
    }
}

pub fn dynamic_formatting_remains_unknown(value: u8) {
    let object: &dyn Debug = &value;
    let _ = write!(&mut Sink, "{object:?}");
}

pub fn display_coercions_remain_unknown(value: u8) {
    let object: &dyn core::fmt::Display = &value;
    let _ = object;
}

pub fn mutable_debug_coercions_remain_unknown(value: u8) {
    let mut local = value;
    let object: &mut dyn Debug = &mut local;
    let _ = object;
}

mod user {
    pub trait Debug {}
    impl Debug for u8 {}

    pub fn ignore(_value: &dyn Debug) {}

    pub fn unwrap_failed(_message: &str, _error: &dyn core::fmt::Debug) -> ! {
        loop {}
    }
}

pub fn similarly_named_traits_are_not_core_debug(value: u8) {
    user::ignore(&value);
}

pub fn similarly_named_helpers_are_not_panic_boundaries(value: u8) {
    user::unwrap_failed("a user-defined function", &value);
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn extend(value: &mut u8) -> &'static mut u8 {
    mir! {
        {
            RET = CastTransmute(value);
            Return()
        }
    }
}

pub fn debug_erasure_cannot_hide_frame_owned_references(value: u8) -> &'static dyn Debug {
    let mut local = value;
    let reference = extend(&mut local);
    &*reference
}

pub fn debug_erasure_does_not_restore_numeric_pointer_provenance(value: u8) {
    let object: &dyn Debug = &value;
    let _ = (object as *const dyn Debug) as *const u8 as usize;
}

#[cfg(test)]
mod tests {
    #[test]
    fn guarded_results_and_unexecuted_formatters_match_native_rust() {
        for value in 0..=u8::MAX {
            super::guarded_unwrap(value);
            super::guarded_expect(value);
            super::error_payload(value);
            super::coercing_and_passing_debug_references_does_not_call_the_formatter(value);
            super::reference_return_preserves_caller_storage(value);
        }
    }
}

pub fn indefinite_debug_loop(value: u8) -> ! {
    let object: &dyn Debug = &value;
    loop {
        let _ = ignore_debug(object);
    }
}
