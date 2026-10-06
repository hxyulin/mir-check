#![no_std]
#![forbid(unsafe_code)]

use core::ops::Add;

fn apply_once<F: FnOnce(bool) -> u8>(callback: F, enabled: bool) -> u8 {
    callback(enabled)
}

pub fn one_label(enabled: bool) {
    assert!(apply_once(u8::from, enabled) == enabled as u8);
}

pub fn mapped_labels(enabled: bool) {
    let labels = [false, enabled, true].map(u8::from);
    assert!(labels[0] == 0);
    assert!(labels[1] == enabled as u8);
    assert!(labels[2] == 1);
}

pub fn wrong_mapped_label(enabled: bool) {
    let labels = [false, enabled, true].map(u8::from);
    assert!(labels[2] == 0);
}

pub fn generated_slots() {
    let slots: [usize; 4] = core::array::from_fn(usize::from);
    assert!(slots[3] == 3);
}

pub fn accumulated_labels() {
    let total = [2_u16, 5, 7].into_iter().fold(0, <u16 as Add<u16>>::add);
    assert!(total == 14);
}

pub fn wrong_accumulated_label() {
    let total = [2_u16, 5, 7].into_iter().fold(0, <u16 as Add<u16>>::add);
    assert!(total == 15);
}

trait ShelfRule {
    fn accepts(label: &u16) -> bool;
}

struct Shelf;

impl ShelfRule for Shelf {
    fn accepts(label: &u16) -> bool {
        *label <= 12
    }
}

pub fn checked_labels() {
    assert!([2_u16, 8, 11].iter().all(<Shelf as ShelfRule>::accepts));
}

pub fn rejected_label() {
    assert!([2_u16, 18, 11].iter().all(<Shelf as ShelfRule>::accepts));
}

pub fn validation_without_a_model(bytes: &[u8]) -> bool {
    core::str::from_utf8(bytes).is_ok()
}

pub fn label_bytes(value: u16) {
    let little = value.to_le_bytes();
    let big = value.to_be_bytes();
    assert!(little[0] == value as u8);
    assert!(little[1] == (value >> 8) as u8);
    assert!(big[0] == little[1]);
    assert!(big[1] == little[0]);
    assert!(u16::from_le_bytes(little) == value);
    assert!(u16::from_be_bytes(big) == value);
    assert!(u16::from_ne_bytes(value.to_ne_bytes()) == value);
}

pub fn signed_label_bytes(value: i32) {
    assert!(i32::from_le_bytes(value.to_le_bytes()) == value);
    assert!(i32::from_be_bytes(value.to_be_bytes()) == value);
    assert!(i32::from_ne_bytes(value.to_ne_bytes()) == value);
}

pub fn wide_label_bytes(value: u128) {
    assert!(u128::from_le_bytes(value.to_le_bytes()) == value);
    assert!(u128::from_be_bytes(value.to_be_bytes()) == value);
    assert!(u128::from_ne_bytes(value.to_ne_bytes()) == value);
}

pub fn wrong_label_bytes() {
    assert!(0x1234_u16.to_be_bytes()[0] == 0x34);
}

struct ApplicationLabel(u16);

impl ApplicationLabel {
    fn to_le_bytes(&self) -> [u8; 2] {
        [0xaa, self.0 as u8]
    }
}

pub fn application_encoding_names_execute_their_body() {
    assert!(ApplicationLabel(0x12).to_le_bytes()[0] == 0xaa);
}

macro_rules! round_trip {
    ($name:ident, $ty:ty) => {
        pub fn $name(value: $ty) {
            assert!(<$ty>::from_le_bytes(value.to_le_bytes()) == value);
            assert!(<$ty>::from_be_bytes(value.to_be_bytes()) == value);
            assert!(<$ty>::from_ne_bytes(value.to_ne_bytes()) == value);
        }
    };
}

round_trip!(round_trip_u8, u8);
round_trip!(round_trip_i8, i8);
round_trip!(round_trip_u16, u16);
round_trip!(round_trip_i16, i16);
round_trip!(round_trip_u32, u32);
round_trip!(round_trip_i32, i32);
round_trip!(round_trip_u64, u64);
round_trip!(round_trip_i64, i64);
round_trip!(round_trip_u128, u128);
round_trip!(round_trip_i128, i128);
round_trip!(round_trip_usize, usize);
round_trip!(round_trip_isize, isize);
