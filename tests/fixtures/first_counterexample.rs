#![no_std]
#![forbid(unsafe_code)]

pub fn two_failures(label: u8) {
    assert!(label == 0);
    assert!(label == 1);
}

pub fn later_failure(label: u8) {
    assert!(label == 0);
    let mut index = 0;
    while index < 256 {
        assert!(index < 256);
        index += 1;
    }
    assert!(index == 255);
}

pub fn completed_batch() {
    let mut index = 0;
    while index < 256 {
        index += 1;
    }
    assert!(index == 256);
}

pub fn unsupported_callback(callback: fn(u8) -> u8) -> u8 {
    callback(4)
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
#[test]
fn both_early_and_later_failures_panic_for_native_inputs() {
    for label in 0..=u8::MAX {
        assert!(std::panic::catch_unwind(|| two_failures(label)).is_err());
        assert!(std::panic::catch_unwind(|| later_failure(label)).is_err());
    }
    completed_batch();
}
