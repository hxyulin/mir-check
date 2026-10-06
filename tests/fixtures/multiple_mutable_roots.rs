#![no_std]
#![forbid(unsafe_code)]

use core::cell::Cell;
use core::sync::atomic::AtomicU8;

pub struct Ledger {
    pub count: u16,
    pub enabled: bool,
}

pub struct Borrowed<'a> {
    pub count: &'a u16,
}

#[doc = "<!-- mir-check:v1:ensures:final_left == 9 && final_right == 12 -->"]
pub fn distinct_scalar_storage(left: &mut u16, right: &mut u16) {
    *left = 9;
    *right = 12;
    assert!(*left == 9 && *right == 12);
}

#[doc = "<!-- mir-check:v1:requires:left < 100 && right < 100 -->"]
#[doc = "<!-- mir-check:v1:ensures:final_left == right && final_right == left -->"]
pub fn entry_snapshots_are_independent(left: &mut u16, right: &mut u16) {
    let original = *left;
    *left = *right;
    *right = original;
}

#[doc = "<!-- mir-check:v1:requires:left == 4 && right == 8 -->"]
#[doc = "<!-- mir-check:v1:ensures:final_left == 5 && final_right == 10 -->"]
fn update_both(left: &mut u16, right: &mut u16) {
    *left += 1;
    *right += 2;
}

pub fn projected_callee_mutations(first: &mut Ledger, second: &mut Ledger) {
    first.count = 4;
    second.count = 8;
    first.enabled = false;
    second.enabled = true;
    update_both(&mut first.count, &mut second.count);
    assert!(first.count == 5 && second.count == 10);
    assert!(!first.enabled && second.enabled);
}

pub fn shared_snapshot_stays_separate(left: &mut u16, right: &mut u16, input: &u16) {
    let original = *input;
    *left = 11;
    *right = 13;
    assert!(*input == original);
    assert!(*left == 11 && *right == 13);
}

pub fn byte_slices_keep_distinct_storage(left: &mut [u8], right: &mut [u8]) {
    if !left.is_empty() && !right.is_empty() {
        left[0] = 31;
        right[0] = 47;
        assert!(left[0] == 31 && right[0] == 47);
    }
}

pub fn interleaved_bytes_and_scalars(left: &mut [u8; 2], right: &mut u16) {
    left[0] = 17;
    *right = 43;
    left[1] = 19;
    assert!(left[0] == 17 && left[1] == 19 && *right == 43);
}

pub fn interleaved_scalars_and_bytes(left: &mut u16, right: &mut [u8; 2]) {
    *left = 43;
    right[0] = 17;
    right[1] = 19;
    assert!(*left == 43 && right[0] == 17 && right[1] == 19);
}

pub fn iterative_writes(left: &mut [u8; 3], right: &mut [u8; 3]) {
    let mut index = 0;
    while index < 3 {
        left[index] = 3;
        right[index] = 7;
        index += 1;
    }
    assert!(left[0] == 3 && right[0] == 7);
}

#[doc = "<!-- mir-check:v1:ensures:final_left == left -->"]
pub fn a_write_does_not_change_its_entry_snapshot(left: &mut u16, right: &mut u16) {
    *left = 9;
    *right = 12;
}

pub fn a_callee_precondition_is_checked(left: &mut u16, right: &mut u16) {
    *left = 100;
    *right = 0;
    update_both(left, right);
}

pub fn a_false_independence_claim(left: &mut u16, right: &mut u16) {
    *left = 9;
    *right = 12;
    assert!(*left == *right);
}

pub fn an_off_by_one_loop(left: &mut [u8; 3], right: &mut [u8; 3]) {
    let mut index = 0;
    while index <= 3 {
        left[index] = 3;
        right[index] = 7;
        index += 1;
    }
}

pub fn a_reference_bearing_pointee(left: &mut Borrowed<'_>, right: &mut u16) {
    *right = *left.count;
}

pub fn an_interior_pointee(left: &mut Cell<u16>, right: &mut u16) {
    left.set(*right);
}

pub fn atomic_pointees_are_not_disjoint_snapshots(left: &mut AtomicU8, right: &mut u16) {
    let _ = left;
    *right = 0;
}

pub fn a_shared_cell_before_mutable_storage(left: &Cell<u16>, right: &mut u16) {
    *right = left.get();
}

pub fn mutable_storage_before_a_shared_cell(left: &mut u16, right: &Cell<u16>) {
    *left = right.get();
}

pub fn unresolved_generic_pointees<T>(left: &mut T, right: &mut u16) {
    let _ = left;
    *right = 0;
}

pub fn non_byte_slices_remain_unsupported(left: &mut [u16], right: &mut u16) {
    if !left.is_empty() {
        left[0] = *right;
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_inputs_keep_each_others_writes_and_entry_values() {
        let mut pair = (0, 0);
        distinct_scalar_storage(&mut pair.0, &mut pair.1);
        assert_eq!(pair, (9, 12));
        entry_snapshots_are_independent(&mut pair.0, &mut pair.1);
        assert_eq!(pair, (12, 9));
        shared_snapshot_stays_separate(&mut pair.0, &mut pair.1, &21);
        assert_eq!(pair, (11, 13));
        let mut first = Ledger {
            count: 0,
            enabled: true,
        };
        let mut second = Ledger {
            count: 0,
            enabled: false,
        };
        projected_callee_mutations(&mut first, &mut second);
        assert_eq!((first.count, second.count), (5, 10));
    }

    #[test]
    fn disjoint_subslices_and_arrays_keep_each_others_writes() {
        let mut bytes = [0; 6];
        let (left, right) = bytes.split_at_mut(3);
        byte_slices_keep_distinct_storage(left, right);
        assert_eq!((bytes[0], bytes[3]), (31, 47));
        let mut first = [0; 3];
        let mut second = [0; 3];
        iterative_writes(&mut first, &mut second);
        assert_eq!((first, second), ([3; 3], [7; 3]));
        let mut word = 0;
        let mut pair = [0; 2];
        interleaved_bytes_and_scalars(&mut pair, &mut word);
        assert_eq!((pair, word), ([17, 19], 43));
        interleaved_scalars_and_bytes(&mut word, &mut pair);
        assert_eq!((word, pair), (43, [17, 19]));
    }

    #[test]
    fn false_storage_claims_and_off_by_one_writes_panic() {
        assert!(
            std::panic::catch_unwind(|| {
                a_false_independence_claim(&mut 0, &mut 0);
            })
            .is_err()
        );
        assert!(
            std::panic::catch_unwind(|| {
                an_off_by_one_loop(&mut [0; 3], &mut [0; 3]);
            })
            .is_err()
        );
    }
}
