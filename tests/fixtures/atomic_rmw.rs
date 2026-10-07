#![no_std]
#![forbid(unsafe_code)]

use core::sync::atomic::{AtomicBool, AtomicI8, AtomicU8, AtomicU16, Ordering};

pub fn bitwise_updates_retain_the_previous_value(value: u8, operand: u8, order: Ordering) {
    let counter = AtomicU8::new(value);
    assert!(counter.fetch_and(operand, order) == value);
    let masked = value & operand;
    assert!(counter.load(Ordering::Relaxed) == masked);
    assert!(counter.fetch_or(operand, order) == masked);
    assert!(counter.load(Ordering::Relaxed) == operand);
    assert!(counter.fetch_xor(value, order) == operand);
    let toggled = value ^ operand;
    assert!(counter.load(Ordering::Relaxed) == toggled);
    assert!(counter.fetch_nand(operand, order) == toggled);
    assert!(counter.load(Ordering::Relaxed) == !(toggled & operand));
}

pub fn signed_extrema_compare_negative_values(value: i8, operand: i8, order: Ordering) {
    let counter = AtomicI8::new(value);
    assert!(counter.fetch_min(operand, order) == value);
    let smaller = if value < operand { value } else { operand };
    assert!(counter.load(Ordering::Relaxed) == smaller);
    assert!(counter.fetch_max(value, order) == smaller);
    assert!(counter.load(Ordering::Relaxed) == value);
}

pub fn unsigned_extrema_keep_the_high_bit(value: u8, operand: u8, order: Ordering) {
    let counter = AtomicU8::new(value);
    assert!(counter.fetch_max(operand, order) == value);
    let larger = if value > operand { value } else { operand };
    assert!(counter.load(Ordering::Relaxed) == larger);
    assert!(counter.fetch_min(value, order) == larger);
    assert!(counter.load(Ordering::Relaxed) == value);
}

pub fn signed_bitwise_updates_keep_the_sign_bit(value: i8, operand: i8) {
    let counter = AtomicI8::new(value);
    assert!(counter.fetch_nand(operand, Ordering::AcqRel) == value);
    assert!(counter.load(Ordering::Relaxed) == !(value & operand));
    assert!(counter.fetch_xor(i8::MIN, Ordering::Release) == !(value & operand));
    assert!(counter.load(Ordering::Relaxed) == (!(value & operand) ^ i8::MIN));
}

macro_rules! wider_extrema {
    ($name:ident, $atomic:ty, $integer:ty) => {
        pub fn $name(value: $integer, operand: $integer) {
            let counter = <$atomic>::new(value);
            assert!(counter.fetch_max(operand, Ordering::Acquire) == value);
            let larger = if value > operand { value } else { operand };
            assert!(counter.load(Ordering::Relaxed) == larger);
            assert!(counter.fetch_min(operand, Ordering::Release) == larger);
            assert!(counter.load(Ordering::Relaxed) == operand);
            assert!(counter.fetch_add(1, Ordering::Relaxed) == operand);
            assert!(counter.load(Ordering::Relaxed) == operand.wrapping_add(1));
            assert!(counter.fetch_sub(1, Ordering::SeqCst) == operand.wrapping_add(1));
            assert!(counter.load(Ordering::Relaxed) == operand);
        }
    };
}

wider_extrema!(unsigned_word, AtomicU16, u16);
wider_extrema!(signed_word, core::sync::atomic::AtomicI16, i16);
wider_extrema!(unsigned_double_word, core::sync::atomic::AtomicU32, u32);
wider_extrema!(signed_double_word, core::sync::atomic::AtomicI32, i32);
wider_extrema!(
    unsigned_pointer_width,
    core::sync::atomic::AtomicUsize,
    usize
);
wider_extrema!(signed_pointer_width, core::sync::atomic::AtomicIsize, isize);
#[cfg(target_has_atomic = "64")]
wider_extrema!(unsigned_quad_word, core::sync::atomic::AtomicU64, u64);
#[cfg(target_has_atomic = "64")]
wider_extrema!(signed_quad_word, core::sync::atomic::AtomicI64, i64);

static FLAGS: AtomicU16 = AtomicU16::new(0x80);

pub fn startup_bitwise_history() {
    assert!(FLAGS.fetch_or(3, Ordering::Release) == 0x80);
    assert!(FLAGS.fetch_and(0xff, Ordering::Acquire) == 0x83);
    assert!(FLAGS.fetch_xor(0x80, Ordering::AcqRel) == 0x83);
    assert!(FLAGS.fetch_nand(3, Ordering::SeqCst) == 3);
    assert!(FLAGS.load(Ordering::Relaxed) == !3);
}

pub fn a_read_modify_write_cannot_establish_exclusivity(counter: &AtomicU8) {
    let _old = counter.fetch_and(0, Ordering::SeqCst);
    assert!(counter.load(Ordering::Relaxed) == 0);
}

pub fn wrong_bitwise_replacement() {
    let counter = AtomicU8::new(0b1010);
    assert!(counter.fetch_nand(0b1100, Ordering::Relaxed) == 0b1010);
    assert!(counter.load(Ordering::Relaxed) == 0b1000);
}

pub fn signed_comparison_cannot_use_unsigned_order() {
    let counter = AtomicI8::new(-120);
    assert!(counter.fetch_min(100, Ordering::Relaxed) == -120);
    assert!(counter.load(Ordering::Relaxed) == -120);
}

pub fn unsupported_boolean_rmw() {
    let flags = AtomicBool::new(false);
    let _old = flags.fetch_or(true, Ordering::Relaxed);
}

pub fn unsupported_pointer_rmw() {
    let flags = core::sync::atomic::AtomicPtr::<u8>::new(core::ptr::null_mut());
    let _old = flags.fetch_or(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_read_modify_write_histories_match_native_execution() {
        for value in u8::MIN..=u8::MAX {
            for operand in u8::MIN..=u8::MAX {
                for order in [
                    Ordering::Relaxed,
                    Ordering::Acquire,
                    Ordering::Release,
                    Ordering::AcqRel,
                    Ordering::SeqCst,
                ] {
                    bitwise_updates_retain_the_previous_value(value, operand, order);
                    signed_extrema_compare_negative_values(value as i8, operand as i8, order);
                    unsigned_extrema_keep_the_high_bit(value, operand, order);
                }
                signed_bitwise_updates_keep_the_sign_bit(value as i8, operand as i8);
            }
        }
        unsigned_word(u16::MAX, 0x8000);
        signed_word(i16::MAX, i16::MIN);
        unsigned_double_word(u32::MAX, 0x80000000);
        signed_double_word(i32::MIN, i32::MAX);
        unsigned_pointer_width(usize::MAX, 0);
        signed_pointer_width(isize::MIN, isize::MAX);
        #[cfg(target_has_atomic = "64")]
        {
            unsigned_quad_word(u64::MAX, 0);
            signed_quad_word(i64::MIN, i64::MAX);
        }
        signed_comparison_cannot_use_unsigned_order();
        startup_bitwise_history();
    }

    #[test]
    #[should_panic]
    fn nand_cannot_retain_the_uncomplemented_value() {
        wrong_bitwise_replacement();
    }
}
