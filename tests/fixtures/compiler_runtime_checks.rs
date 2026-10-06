#![no_std]
#![forbid(unsafe_code)]
#![feature(core_intrinsics, rustc_attrs)]
// Internal features expose compiler operands directly for this pinned-toolchain regression.
#![allow(internal_features)]
#![rustc_preserve_ub_checks]

pub fn ub_checks_enabled() {
    assert!(core::intrinsics::ub_checks());
}

pub fn ub_checks_disabled() {
    assert!(!core::intrinsics::ub_checks());
}

pub fn overflow_checks_enabled() {
    assert!(core::intrinsics::overflow_checks());
}

pub fn overflow_checks_disabled() {
    assert!(!core::intrinsics::overflow_checks());
}
