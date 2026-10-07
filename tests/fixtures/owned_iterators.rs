#![no_std]
#![forbid(unsafe_code)]

use core::cell::Cell;
use mir_contracts::requires;

#[requires(offset < 100)]
pub fn station_labels(offset: u16) {
    let labels = [offset, offset + 2, offset + 4, offset + 6];
    let mut pending = labels.into_iter();
    assert!(pending.len() == 4);
    assert!(pending.next().unwrap() == offset);
    assert!(pending.next_back().unwrap() == offset + 6);
    assert!(pending.nth(1).unwrap() == offset + 4);
    assert!(pending.next().is_none());
    assert!(pending.next_back().is_none());
    assert!(labels[1] == offset + 2);
}

pub fn parcel_ends(skip: usize) {
    let mut parcels = [11_u16, 22, 33, 44, 55].into_iter();
    assert!(parcels.next().unwrap() == 11);
    let end = parcels.nth_back(skip);
    if skip < 4 {
        assert!(end.unwrap() == [55, 44, 33, 22][skip]);
        assert!(parcels.len() == 3 - skip);
    } else {
        assert!(end.is_none());
        assert!(parcels.len() == 0);
    }
}

pub fn byte_count() {
    let mut markers = [7_u8, 19, 31, 43].into_iter();
    assert!(markers.nth(1).unwrap() == 19);
    let (lower, upper) = markers.size_hint();
    assert!(lower == 2 && upper.unwrap() == 2);
    assert!(markers.count() == 2);
}

pub fn empty() {
    let parcels: [u16; 0] = [];
    let mut parcels = parcels.into_iter();
    assert!(parcels.len() == 0);
    assert!(parcels.next().is_none());
    assert!(parcels.nth(usize::MAX).is_none());
    assert!(parcels.next_back().is_none());
    assert!(parcels.count() == 0);
}

pub fn unit_moves() {
    let mut tokens = [(); 3].into_iter();
    assert!(tokens.next().is_some());
    assert!(tokens.next_back().is_some());
    assert!(tokens.nth(0).is_some());
    assert!(tokens.next().is_none());
}

struct Parcel {
    weight_g: u16,
    fragile: bool,
}

pub fn owned_structs() {
    let parcels = [
        Parcel {
            weight_g: 25,
            fragile: false,
        },
        Parcel {
            weight_g: 50,
            fragile: true,
        },
    ];
    let mut pending = parcels.into_iter();
    let front = pending.next().unwrap();
    let back = pending.next_back().unwrap();
    assert!(front.weight_g == 25 && !front.fragile);
    assert!(back.weight_g == 50 && back.fragile);
    assert!(pending.next().is_none());
}

pub fn predicate_effects() {
    let seen = Cell::new(0_u16);
    let mut pending = [2_u16, 4, 6, 8].into_iter();
    assert!(!pending.all(|weight| {
        seen.set(seen.get() + 1);
        weight < 6
    }));
    assert!(seen.get() == 3);
    assert!(pending.next().unwrap() == 8);
    assert!(pending.any(|weight| weight == 9) == false);
}

pub fn predicate_call_bounds() {
    assert!([3_u16, 4, 5].into_iter().all(small));
}

#[requires(weight < 10)]
fn small(weight: u16) -> bool {
    weight < 10
}

pub fn ordered_fold() {
    let seen = Cell::new(0_u16);
    let checksum = [1_u16, 2, 3].into_iter().fold(0_u16, |value, digit| {
        seen.set(seen.get() + 1);
        value * 10 + digit
    });
    assert!(checksum == 123 && seen.get() == 3);
    let reverse = [1_u16, 2, 3]
        .into_iter()
        .rfold(0_u16, |value, digit| value * 10 + digit);
    assert!(reverse == 321);
}

pub fn last_and_adapters() {
    assert!([17_u16, 29, 41].into_iter().last().unwrap() == 41);
    let mut labels = [17_u16, 29, 41].into_iter().rev().enumerate();
    assert!(labels.next().unwrap() == (0, 41));
    assert!(labels.next().unwrap() == (1, 29));
    assert!(labels.next().unwrap() == (2, 17));
    assert!(labels.next().is_none());
}

pub fn borrowed_consuming_methods() {
    let mut parcels = [3_u16, 6, 9].into_iter();
    assert!((&mut parcels).into_iter().next().unwrap() == 3);
    let sum = (&mut parcels).fold(0_u16, |sum, weight| sum + weight);
    assert!(sum == 15 && parcels.len() == 0);
    let mut parcels = [3_u16, 6, 9].into_iter();
    assert!(parcels.by_ref().count() == 3);
    assert!(parcels.next().is_none());
    let mut parcels = [3_u16, 6, 9].into_iter();
    assert!((&mut parcels).last().unwrap() == 9);
    assert!(parcels.next().is_none());
}

pub fn borrowed_shared_count() {
    let labels = [4_u16, 8, 12];
    let mut labels = labels.iter();
    assert!(*labels.next().unwrap() == 4);
    assert!(labels.by_ref().count() == 2);
    let (lower, upper) = labels.size_hint();
    assert!(lower == 0 && upper.unwrap() == 0);
    assert!(labels.next().is_none());
    assert!(labels.next_back().is_none());
    let markers = [5_u8, 10];
    let mut markers = markers.iter();
    assert!((&mut markers).into_iter().count() == 2);
    assert!(markers.next().is_none());
}

pub fn bad_borrowed_shared_count() {
    let labels = [4_u16, 8];
    let mut labels = labels.iter();
    assert!(labels.by_ref().count() == 2);
    assert!(labels.next().is_some());
}

struct ShippingList {
    pending: core::array::IntoIter<u16, 2>,
}

pub fn wrapped_iterator_drop() {
    let mut list = ShippingList {
        pending: [15_u16, 30].into_iter(),
    };
    assert!(list.pending.next().unwrap() == 15);
}

struct DroppingList {
    pending: core::array::IntoIter<u16, 2>,
}

impl Drop for DroppingList {
    fn drop(&mut self) {
        panic!("an enclosing custom destructor must still be checked");
    }
}

pub fn enclosing_destructor() {
    let mut list = DroppingList {
        pending: [15_u16, 30].into_iter(),
    };
    assert!(list.pending.next().unwrap() == 15);
}

pub fn bad_borrowed_count() {
    let mut parcels = [3_u16, 6].into_iter();
    assert!((&mut parcels).count() == 2);
    assert!(parcels.next().is_some());
}

pub fn bad_borrowed_last() {
    let mut parcels = [3_u16, 6].into_iter();
    assert!((&mut parcels).last().unwrap() == 6);
    assert!(parcels.next().is_some());
}

pub fn bad_borrowed_passthrough() {
    let mut parcels = [3_u16, 6].into_iter();
    assert!((&mut parcels).into_iter().next().unwrap() == 3);
    assert!(parcels.len() == 2);
}

pub fn callback_panic() {
    [2_u16, 4, 6].into_iter().all(|weight| {
        assert!(weight < 6);
        true
    });
}

pub fn bad_call_bound() {
    [2_u16, 12].into_iter().all(small);
}

pub fn wrong_order() {
    let mut parcels = [13_u16, 26, 39].into_iter();
    assert!(parcels.next().unwrap() == 39);
}

pub fn bad_fold() {
    let _ = [u16::MAX, 1]
        .into_iter()
        .fold(0_u16, |sum, weight| sum + weight);
}

pub fn identity_elements() {
    let mut counters = [Cell::new(1_u16), Cell::new(2)].into_iter();
    counters.next().unwrap().set(4);
}

struct DroppingParcel;

impl Drop for DroppingParcel {
    fn drop(&mut self) {
        panic!("destructor execution is outside the modeled owned iterator domain");
    }
}

pub fn element_destructor() {
    let mut parcels = [DroppingParcel, DroppingParcel].into_iter();
    let _ = parcels.next();
}

pub fn borrowed_element() {
    let labels = [7_u16, 8];
    let mut pending = [&labels[0], &labels[1]].into_iter();
    assert!(*pending.next().unwrap() == 7);
}

pub fn borrowed_aliases() {
    let label = Cell::new(21_u16);
    let mut pending = [&label, &label].into_iter();
    pending.next().unwrap().set(34);
    assert!(pending.next_back().unwrap().get() == 34);
    assert!(pending.next().is_none());
}

pub fn mutable_elements() {
    let mut left = 12_u16;
    let mut right = 24_u16;
    let mut pending = [&mut left, &mut right].into_iter();
    *pending.next_back().unwrap() = 36;
    *pending.next().unwrap() = 18;
    assert!(pending.next().is_none());
    drop(pending);
    assert!(left == 18 && right == 36);
}

pub fn mutable_predicate() {
    let mut first = 4_u16;
    let mut second = 8_u16;
    let mut third = 16_u16;
    let mut pending = [&mut first, &mut second, &mut third].into_iter();
    assert!(!pending.all(|item| {
        *item += 1;
        *item < 9
    }));
    *pending.next().unwrap() = 32;
    drop(pending);
    assert!(first == 5 && second == 9 && third == 32);
}

pub fn borrowed_fold() {
    let labels = [3_u16, 6, 9];
    let mut pending = [&labels[0], &labels[1], &labels[2]].into_iter();
    let total = (&mut pending).rfold(0_u16, |sum, item| sum + *item);
    assert!(total == 18 && pending.next().is_none());
}

struct BorrowedParcel<'a> {
    weight_g: &'a mut u16,
    label: &'a u16,
}

pub fn nested_borrowed_elements() {
    let mut weight_g = 60_u16;
    let label = 42_u16;
    let mut pending = [Some(BorrowedParcel {
        weight_g: &mut weight_g,
        label: &label,
    })].into_iter();
    let parcel = pending.next().unwrap().unwrap();
    *parcel.weight_g = *parcel.label;
    assert!(pending.next().is_none());
    drop(pending);
    assert!(weight_g == 42);
}

fn move_pending<'a>(left: &'a mut u16, right: &'a mut u16) -> core::array::IntoIter<&'a mut u16, 2> {
    [left, right].into_iter()
}

pub fn returned_borrowed_iterator() {
    let mut left = 5_u16;
    let mut right = 10_u16;
    let mut pending = move_pending(&mut left, &mut right);
    *pending.next().unwrap() = 15;
    *pending.next_back().unwrap() = 20;
    drop(pending);
    assert!(left == 15 && right == 20);
}

pub fn borrowed_cursor_copy() {
    let labels = [17_u16, 29];
    let mut pending = [&labels[0], &labels[1]].into_iter();
    let mut copied = pending.clone();
    assert!(*pending.next().unwrap() == *copied.next().unwrap());
}

pub fn borrowed_symbolic_skip(skip: usize) {
    let labels = [17_u16, 29];
    let mut pending = [&labels[0], &labels[1]].into_iter();
    if let Some(label) = pending.nth(skip) {
        assert!(*label == labels[skip]);
    }
}

pub fn borrowed_callback_panic() {
    let labels = [17_u16, 29];
    [&labels[0], &labels[1]].into_iter().all(|label| {
        assert!(*label < 29);
        true
    });
}

pub fn borrowed_bad_bound() {
    let labels = [3_u16, 12];
    [&labels[0], &labels[1]].into_iter().all(|label| small(*label));
}

pub fn borrowed_wrong_order() {
    let labels = [17_u16, 29];
    let mut pending = [&labels[0], &labels[1]].into_iter();
    assert!(*pending.next_back().unwrap() == 17);
}

pub fn mutable_wrong_effect() {
    let mut left = 12_u16;
    let mut right = 24_u16;
    let mut pending = [&mut left, &mut right].into_iter();
    *pending.next_back().unwrap() = 36;
    drop(pending);
    assert!(left == 36);
}

pub fn borrowed_bad_alias() {
    let label = Cell::new(21_u16);
    let mut pending = [&label, &label].into_iter();
    pending.next().unwrap().set(34);
    assert!(pending.next().unwrap().get() == 21);
}

struct HazardousLabel(u16);

impl Clone for HazardousLabel {
    fn clone(&self) -> Self {
        panic!("element cloning is a callback, not a cursor copy")
    }
}

pub fn unsupported_clone() {
    let labels = [HazardousLabel(1), HazardousLabel(2)].into_iter();
    let mut copied = labels.clone();
    assert!(copied.next().unwrap().0 == 1);
}

pub fn mutable_capture() {
    let mut count = 0_u16;
    [2_u16, 4].into_iter().all(|_| {
        count += 1;
        true
    });
    assert!(count == 2);
}

pub fn unsupported_view() {
    let mut parcels = [5_u16, 10].into_iter();
    let _ = parcels.next();
    assert!(parcels.as_slice()[0] == 10);
}

struct Counterfeit;

impl Counterfeit {
    fn into_iter(self) -> Self {
        self
    }
    fn next(&mut self) -> Option<u16> {
        panic!("same method name is not a compiler identity")
    }
}

pub fn same_named_user_iterator() {
    let mut cursor = Counterfeit.into_iter();
    let _ = cursor.next();
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parcel_cursor_models_match_independent_positions() {
        for offset in 0..100 {
            station_labels(offset);
        }
        for skip in 0..9 {
            parcel_ends(skip);
            let mut pending = [11_u16, 22, 33, 44, 55].into_iter();
            assert_eq!(pending.next(), Some(11));
            let expected = if skip < 4 {
                Some(55 - 11 * skip as u16)
            } else {
                None
            };
            assert_eq!(pending.nth_back(skip), expected);
            assert_eq!(pending.len(), 4_usize.saturating_sub(skip + 1));
        }
        parcel_ends(usize::MAX);
        byte_count();
        empty();
        unit_moves();
        owned_structs();
        predicate_effects();
        predicate_call_bounds();
        ordered_fold();
        last_and_adapters();
        wrapped_iterator_drop();
        borrowed_consuming_methods();
        borrowed_shared_count();
        borrowed_element();
        borrowed_aliases();
        mutable_elements();
        mutable_predicate();
        borrowed_fold();
        nested_borrowed_elements();
        returned_borrowed_iterator();
        borrowed_cursor_copy();
        for skip in [0, 1, 2, usize::MAX] {
            borrowed_symbolic_skip(skip);
        }
    }

    #[test]
    fn borrowed_iterator_failures_replay_as_real_assertion_failures() {
        for check in [
            borrowed_callback_panic,
            borrowed_wrong_order,
            mutable_wrong_effect,
            borrowed_bad_alias,
        ] {
            assert!(std::panic::catch_unwind(check).is_err());
        }
    }

}
