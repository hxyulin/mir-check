#![no_std]
#![forbid(unsafe_code)]

pub fn byte(value: u8) {
    let count = value.count_ones();
    assert!(count <= 8);
    assert!(count + value.count_zeros() == 8);
    assert!((count == 0) == (value == 0));
    assert!((count == 8) == (value == u8::MAX));
}

pub fn widths(a: u16, b: u32, c: u64, d: u128, e: usize) {
    assert!(a.count_ones() <= u16::BITS);
    assert!(b.count_ones() <= u32::BITS);
    assert!(c.count_ones() <= u64::BITS);
    assert!(d.count_ones() <= u128::BITS);
    assert!(e.count_ones() <= usize::BITS);
}

pub fn signed(a: i8, b: i16, c: i32, d: i64, e: i128, f: isize) {
    assert!(a.count_ones() == (a as u8).count_ones());
    assert!(b.count_ones() <= i16::BITS);
    assert!(c.count_ones() <= i32::BITS);
    assert!(d.count_ones() <= i64::BITS);
    assert!(e.count_ones() <= i128::BITS);
    assert!(f.count_ones() <= isize::BITS);
    assert!((-1_i128).count_ones() == 128);
}

pub fn masked(value: u16) {
    let active = (value & 0b10101).count_ones();
    assert!(active <= 3);
}

pub fn bad_bound(value: u8) {
    assert!(value.count_ones() < 8);
}

pub fn bad_signed(value: i8) {
    assert!(value.count_ones() < 8);
}

pub struct Pretender;

impl Pretender {
    pub fn count_ones(&self) -> u32 {
        panic!("user method");
    }
}

pub fn user_method() {
    Pretender.count_ones();
}

pub fn unsupported(value: *const u8) -> u32 {
    (value as usize).count_ones()
}
