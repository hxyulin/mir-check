#![no_std]
#![forbid(unsafe_code)]

use core::sync::atomic::{AtomicU16, Ordering};

#[cfg(not(target_os = "none"))]
extern crate std;

static FRESH_STATE: AtomicU16 = AtomicU16::new(0);
static OCCUPIED_STATE: AtomicU16 = AtomicU16::new(1);

pub fn checked_index(index: u8) -> u8 {
    [3, 5, 8, 13][usize::from(index)]
}

pub fn guarded_index(index: u8) -> u8 {
    if index < 4 {
        [3, 5, 8, 13][usize::from(index)]
    } else {
        0
    }
}

pub fn signed_division(divisor: i8) -> i8 {
    42 / divisor
}

pub fn signed_minimum_panic(value: i128) {
    assert!(value != i128::MIN);
}

pub fn unsigned_maximum_panic(value: u128) {
    assert!(value != u128::MAX);
}

pub fn boolean_panic(fail: bool) {
    assert!(!fail);
}

pub fn panic_strategy_changes_behavior(fail: bool) {
    #[cfg(panic = "abort")]
    assert!(!fail);
    #[cfg(panic = "unwind")]
    let _ = fail;
}

pub fn scalar_array(values: [u16; 2]) {
    assert!(values[0] <= 8 || values[1] <= 8);
}

pub fn fresh_atomic_claim() {
    FRESH_STATE
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .unwrap();
}

pub fn occupied_atomic_claim() {
    OCCUPIED_STATE
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .unwrap();
}

pub fn weak_exchange_is_not_guaranteed_to_succeed() {
    let state = AtomicU16::new(0);
    assert!(
        state
            .compare_exchange_weak(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    );
}

pub fn arithmetic_nan_encoding_is_not_guaranteed() {
    let sample = f32::from_bits(0x7fc5_0001);
    assert!((sample * 2.0).to_bits() == 0x7fc5_0001);
}

pub struct PanickingReturn;

impl Drop for PanickingReturn {
    fn drop(&mut self) {
        panic!("dropping the return value is the caller's operation");
    }
}

pub fn fresh_claim_returns_owned_value() -> PanickingReturn {
    fresh_atomic_claim();
    PanickingReturn
}

pub fn unsupported_reference_input(values: &[u8]) -> u8 {
    values[9]
}

fn private_root(index: u8) -> u8 {
    [3, 5][usize::from(index)]
}

pub fn private_caller(index: u8) -> u8 {
    private_root(index)
}

#[cfg(not(target_os = "none"))]
pub fn nonpanic_exit() {
    FRESH_STATE
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .unwrap();
    std::process::exit(101);
}

pub fn nonterminating_after_claim() {
    FRESH_STATE
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .unwrap();
    loop {
        core::hint::spin_loop();
    }
}
