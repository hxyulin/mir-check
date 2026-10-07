#![no_std]
#![forbid(unsafe_code)]
#![feature(custom_mir, core_intrinsics)]

use core::intrinsics::mir::*;

// Synthetic typed MIR tests lifetime casts without adding an unsafe Rust implementation.
#[custom_mir(dialect = "runtime", phase = "optimized")]
pub fn retain<T>(value: &mut T) -> &'static mut T {
    mir! {
        {
            RET = CastTransmute(Move(value));
            Return()
        }
    }
}

pub fn writes_reach_original(value: &mut u8) {
    let alias = retain(value);
    *alias = 23;
    assert!(*value == 23);
}

pub fn projected_storage() {
    let mut samples = [4_u8, 9];
    let alias = retain(&mut samples[1]);
    *alias = 23;
    assert!(samples == [4, 23]);
}

struct Ledger {
    entries: [u8; 3],
    count: u16,
}

pub fn aggregate_storage() {
    let mut ledger = Ledger {
        entries: [4, 6, 8],
        count: 0,
    };
    let alias = retain(&mut ledger);
    alias.entries[2] = 23;
    alias.count = 1;
    assert!(ledger.entries == [4, 6, 23] && ledger.count == 1);
}

pub fn wrong_original_value(value: &mut u8) {
    let alias = retain(value);
    *alias = 23;
    assert!(*value == 22);
}

pub fn local_borrow_cannot_escape() -> &'static mut u8 {
    let mut value = 4;
    retain(&mut value)
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
pub fn dead_storage_is_not_retained() -> &'static mut u8 {
    mir! {
        let storage: u8;
        let alias: &mut u8;
        {
            storage = 4;
            alias = &mut storage;
            StorageDead(storage);
            RET = CastTransmute(Move(alias));
            Return()
        }
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
pub fn different_pointee(value: &mut u8) -> &mut u16 {
    mir! {
        {
            RET = CastTransmute(Move(value));
            Return()
        }
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
pub fn different_mutability(value: &mut u8) -> &u8 {
    mir! {
        {
            RET = CastTransmute(Move(value));
            Return()
        }
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
pub fn raw_pointer_is_not_a_reference(value: *mut u8) -> &'static mut u8 {
    mir! {
        {
            RET = CastTransmute(Move(value));
            Return()
        }
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
pub fn shared_snapshot_is_not_a_tracked_reference(value: &u8) -> &'static u8 {
    mir! {
        {
            RET = CastTransmute(Move(value));
            Return()
        }
    }
}

pub fn selected_alias(left: &mut u8, right: &mut u8) -> &'static mut u8 {
    let _ = right;
    retain(left)
}

pub fn assumed_alias() {
    let mut left = 4;
    let mut right = 9;
    *selected_alias(&mut left, &mut right) = 23;
    assert!(left == 23 && right == 9);
}

pub fn bad_assumed_alias() {
    let mut left = 4;
    let mut right = 9;
    *selected_alias(&mut left, &mut right) = 23;
    assert!(left == 22);
}

pub fn assumed_escape() -> &'static mut u8 {
    let mut left = 4;
    let mut right = 9;
    selected_alias(&mut left, &mut right)
}

pub fn assumed_post_state(value: &mut u8) {
    let mut other = 0;
    let alias = selected_alias(value, &mut other);
    assert!(*alias == 31);
    assert!(*value == 31 && other == 0);
}

pub fn incompatible_alias(value: &mut u8) -> u8 {
    *value
}

pub fn incompatible_caller(value: &mut u8) {
    let _ = incompatible_alias(value);
}

#[cfg(test)]
mod tests {
    #[test]
    fn scoped_alias_writes_preserve_original_storage() {
        let mut value = 0;
        super::writes_reach_original(&mut value);
        super::projected_storage();
        super::aggregate_storage();
        super::assumed_alias();
    }
}
