#![no_std]
#![forbid(unsafe_code)]

use core::cell::Cell;
use core::mem::MaybeUninit;
use core::num::NonZeroU8;
use core::sync::atomic::{AtomicU8, Ordering};

pub fn guarded_as_ref(index: Option<usize>, bytes: [u8; 4]) -> u8 {
    match index.as_ref() {
        Some(index) if *index < bytes.len() => bytes[*index],
        _ => 0,
    }
}

pub fn bad_as_ref(index: Option<usize>, bytes: [u8; 4]) -> u8 {
    match index.as_ref() {
        Some(index) if *index <= bytes.len() => bytes[*index],
        _ => 0,
    }
}

pub fn none() {
    const VALUE: Option<u8> = None;
    assert!(VALUE.is_none());
}

pub fn some() {
    const VALUE: Option<u8> = Some(7);
    match VALUE {
        Some(value) => assert!(value == 7),
        None => panic!("wrong variant"),
    }
}

pub fn wrong_payload() {
    const VALUE: Option<u8> = Some(7);
    match VALUE {
        Some(value) => assert!(value == 8),
        None => (),
    }
}

pub fn niche() {
    const PRESENT: Option<NonZeroU8> = NonZeroU8::new(9);
    const MISSING: Option<NonZeroU8> = None;
    assert!(PRESENT.is_some());
    assert!(MISSING.is_none());
}

pub fn niche_get() {
    const PRESENT: Option<NonZeroU8> = NonZeroU8::new(9);
    if let Some(value) = PRESENT {
        assert!(value.get() == 9);
    }
}

pub fn niche_reference() {
    const PRESENT: Option<&u8> = Some(&13);
    const MISSING: Option<&u8> = None;
    assert!(MISSING.is_none());
    match PRESENT {
        Some(value) => assert!(*value == 13),
        None => panic!("wrong variant"),
    }
}

#[repr(i8)]
enum Mode {
    Missing = -3,
    Reading(u16) = 5,
}

pub fn discriminants() {
    const MISSING: Mode = Mode::Missing;
    const READING: Mode = Mode::Reading(300);
    assert!(matches!(MISSING, Mode::Missing));
    match READING {
        Mode::Reading(value) => assert!(value == 300),
        Mode::Missing => panic!("wrong variant"),
    }
}

struct Packet {
    index: Option<usize>,
    bytes: [u8; 4],
    tuple: (i16, bool, f32),
}

pub fn nested() -> u8 {
    const PACKET: Packet = Packet {
        index: Some(2),
        bytes: [11, 22, 33, 44],
        tuple: (-300, true, 1.5),
    };
    let packet = PACKET;
    assert!(packet.tuple.0 == -300 && packet.tuple.1 && packet.tuple.2 == 1.5);
    match packet.index {
        Some(index) => packet.bytes[index],
        None => 0,
    }
}

pub fn promoted() {
    let packet = &(Some(3_usize), [10_u16, 20, 30, 40]);
    match packet.0 {
        Some(index) => assert!(packet.1[index] == 40),
        None => panic!("wrong variant"),
    }
}

static BYTES: [u8; 5] = [4, 8, 12, 16, 20];

pub fn static_array() {
    assert!(BYTES[2] == 12);
}

pub fn constant_slice() {
    const SLICE: &[u8] = &[2, 4, 6];
    assert!(SLICE[1] == 4);
}

pub fn maximum_bytes() {
    const BYTES: [u8; 128] = [7; 128];
    assert!(BYTES[127] == 7);
}

pub fn uninitialized() {
    const VALUE: MaybeUninit<u32> = MaybeUninit::uninit();
    let _value = VALUE;
}

pub fn inactive_uninitialized() {
    const VALUE: Option<MaybeUninit<u32>> = None;
    assert!(VALUE.is_none());
}

pub fn active_uninitialized() {
    const VALUE: Option<MaybeUninit<u32>> = Some(MaybeUninit::uninit());
    assert!(VALUE.is_some());
}

pub fn interior_mutable() {
    let value = &Cell::new(7_u8);
    assert!(value.get() == 7);
}

static COUNTER: AtomicU8 = AtomicU8::new(7);

pub fn mutable_static_storage() {
    assert!(COUNTER.load(Ordering::Relaxed) == 7);
}

pub fn initialized_union() {
    const VALUE: MaybeUninit<u32> = MaybeUninit::new(7);
    let _value = VALUE;
}

pub fn raw_pointer() {
    const POINTER: *const u8 = core::ptr::null();
    let _value = POINTER;
}

pub fn oversized_array() {
    const ARRAY: [u16; 17] = [1; 17];
    let _value = ARRAY;
}

pub fn oversized_bytes() {
    const ARRAY: [u8; 129] = [1; 129];
    let _value = ARRAY;
}

pub fn oversized_shape() {
    const ARRAY: [[u16; 16]; 16] = [[1; 16]; 16];
    let _value = ARRAY;
}

pub fn deep_shape() {
    const ARRAY: [[[[[[[[[u16; 1]; 1]; 1]; 1]; 1]; 1]; 1]; 1]; 1] =
        [[[[[[[[[1]]]]]]]]];
    let _value = ARRAY;
}
