#![no_std]
#![forbid(unsafe_code)]

use mir_contracts::requires;

struct Pair<'a> {
    left: &'a mut u16,
    right: &'a mut u16,
}

fn pair<'a>(left: &'a mut u16, right: &'a mut u16) -> Pair<'a> {
    Pair { left, right }
}

fn move_pair(value: Pair<'_>) -> Pair<'_> {
    value
}

#[requires(seed < 100)]
pub fn parcel_pair(seed: u16) {
    let mut left = seed;
    let mut right = seed + 1;
    let pair = move_pair(pair(&mut left, &mut right));
    *pair.left += 2;
    *pair.right += 3;
    assert!(left == seed + 2 && right == seed + 4);
}

#[requires(seed < 100)]
pub fn tuple_reborrow(seed: u16) {
    let mut value = seed;
    let refs = (&mut value,);
    let moved = move_tuple(refs);
    *moved.0 += 1;
    assert!(value == seed + 1);
}

fn move_tuple(value: (&mut u16,)) -> (&mut u16,) {
    value
}

#[requires(seed < 100)]
pub fn captured_counter(seed: u16) {
    let mut total = seed;
    let mut add = |value: u16| total += value;
    add(2);
    add(3);
    assert!(total == seed + 5);
}

#[requires(seed < 100)]
pub fn zipped_labels(seed: u16) {
    let mut labels = [seed; 2];
    for (label, adjustment) in labels.iter_mut().zip([2_u16, 5]) {
        *label += adjustment;
    }
    assert!(labels[0] == seed + 2 && labels[1] == seed + 5);
}

#[requires(seed < 100)]
pub fn flattened_bins(seed: u16) {
    let mut bins = [[seed; 2]; 2];
    for label in bins.iter_mut().flatten() {
        *label += 3;
    }
    assert!(bins[0][0] == seed + 3 && bins[0][1] == seed + 3);
    assert!(bins[1][0] == seed + 3 && bins[1][1] == seed + 3);
}

#[requires(seed < 100)]
pub fn wrong_pair(seed: u16) {
    let mut left = seed;
    let mut right = seed;
    let pair = pair(&mut left, &mut right);
    *pair.left += 2;
    assert!(right == seed + 2);
}

fn incrementer(value: &mut u16) -> impl FnMut() + '_ {
    move || *value += 1
}

#[requires(seed < 100)]
pub fn returned_capture(seed: u16) {
    let mut value = seed;
    {
        let mut increment = incrementer(&mut value);
        increment();
        increment();
    }
    assert!(value == seed + 2);
}

fn optional(value: &mut u16) -> Option<&mut u16> {
    Some(value)
}

#[requires(seed < 100)]
pub fn optional_borrow(seed: u16) {
    let mut value = seed;
    *optional(&mut value).unwrap() += 3;
    assert!(value == seed + 3);
}

pub fn wrong_capture() {
    let mut count = 0_u16;
    let mut tick = || count += 1;
    tick();
    tick();
    assert!(count == 1);
}

pub fn owned_capture_state() {
    let mut serial = 0_u16;
    let mut advance = move || {
        serial += 1;
        serial
    };
    assert!(advance() == 1);
    assert!(advance() == 2);
}

pub fn wrong_owned_capture_state() {
    let mut serial = 0_u16;
    let mut advance = move || {
        serial += 1;
        serial
    };
    assert!(advance() == 1);
    assert!(advance() == 1);
}

pub fn generated_capture_state() {
    let mut serial = 0_u16;
    let labels: [u16; 3] = core::array::from_fn(move |_| {
        serial += 1;
        serial
    });
    assert!(labels[0] == 1 && labels[1] == 2 && labels[2] == 3);
}

pub fn mapped_capture_state() {
    let mut serial = 0_u16;
    let labels = [10_u16, 20, 30].map(move |value| {
        serial += 1;
        value + serial
    });
    assert!(labels[0] == 11 && labels[1] == 22 && labels[2] == 33);
}

pub fn mapped_reference_elements() {
    let mut first = 7_u16;
    let mut second = 11_u16;
    let observations = [&mut first, &mut second].map(|value| {
        *value += 2;
        *value
    });
    assert!(observations[0] == 9 && observations[1] == 13);
    assert!(first == 9 && second == 13);
}

pub fn predicate_capture_state() {
    let mut serial = 0_u16;
    assert!([1_u16, 2, 3].into_iter().all(move |value| {
        serial += 1;
        serial == value
    }));
}

pub fn folded_capture_state() {
    let mut serial = 0_u16;
    let checksum = [10_u16, 20, 30].into_iter().fold(0_u16, move |sum, value| {
        serial += 1;
        sum + value + serial
    });
    assert!(checksum == 66);
}

pub fn byte_capture_boundary() {
    let mut bytes = [1_u8, 2];
    let mut change = || bytes[0] = 3;
    change();
    assert!(bytes[0] == 3);
}

pub fn multiple_mutable_inputs(left: &mut u16, right: &mut u16) {
    *left = *right;
}

pub fn ambiguous_write(index: usize) {
    let mut slots = [1_u16, 2];
    if index < slots.len() {
        let refs = (&mut slots[index],);
        *refs.0 = 4;
        assert!(slots[index] == 4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_through_aggregate_and_adapter_borrows_reach_original_storage() {
        for seed in 0..100 {
            parcel_pair(seed);
            tuple_reborrow(seed);
            captured_counter(seed);
            returned_capture(seed);
            optional_borrow(seed);
            zipped_labels(seed);
            flattened_bins(seed);
        }
        owned_capture_state();
        generated_capture_state();
        mapped_capture_state();
        mapped_reference_elements();
        predicate_capture_state();
        folded_capture_state();
        assert!(std::panic::catch_unwind(|| wrong_pair(7)).is_err());
        assert!(std::panic::catch_unwind(wrong_capture).is_err());
        assert!(std::panic::catch_unwind(wrong_owned_capture_state).is_err());
    }
}

#[cfg(test)]
extern crate std;
