#![no_std]
#![forbid(unsafe_code)]

use core::cell::Cell;
use miren_contracts::{ensures, requires};

#[requires(start < 65000)]
#[ensures(result[0] == start && result[3] > start)]
pub fn ticket_numbers(start: u16) -> [u16; 4] {
    core::array::from_fn(|index| start + index as u16)
}

pub fn byte_labels() -> [u8; 5] {
    let labels = core::array::from_fn(|index| index as u8 + 1);
    assert!(labels[0] == 1 && labels[4] == 5);
    labels
}

pub fn parcel_slots(priority: bool) -> [(u16, bool); 3] {
    let slots: [(u16, bool); 3] = core::array::from_fn(|index| (index as u16 + 20, priority));
    assert!(slots[0].0 == 20 && slots[2].0 == 22);
    assert!(slots[1].1 == priority);
    slots
}

pub fn callback_order() -> [u16; 4] {
    let next = Cell::new(0_usize);
    let tickets = core::array::from_fn(|index| {
        assert!(next.get() == index);
        next.set(index + 1);
        index as u16 + 30
    });
    assert!(next.get() == 4);
    assert!(tickets[0] == 30 && tickets[3] == 33);
    tickets
}

pub fn caller_effects(next: &Cell<u16>) -> [u16; 3] {
    let values = core::array::from_fn(|index| {
        next.set(index as u16 + 10);
        next.get()
    });
    assert!(next.get() == 12);
    assert!(values[0] == 10 && values[2] == 12);
    values
}

pub fn empty() -> [u16; 0] {
    core::array::from_fn(|_| panic!("empty arrays must not call their callback"))
}

fn item(index: usize) -> u16 {
    index as u16
}

pub fn function_item() -> [u16; 4] {
    let values = core::array::from_fn(item);
    assert!(values[0] == 0 && values[3] == 3);
    values
}

pub fn larger_owned_array() -> [u16; 18] {
    let values = core::array::from_fn(item);
    assert!(values[0] == 0 && values[17] == 17);
    values
}

fn byte_item(index: usize) -> u8 {
    index as u8
}

pub fn maximum_owned_array() -> [u8; 128] {
    let values = core::array::from_fn(byte_item);
    assert!(values[0] == 0 && values[127] == 127);
    values
}

pub fn branch_results(express: bool) -> [u16; 2] {
    let values = core::array::from_fn(|index| {
        if express {
            index as u16 + 40
        } else {
            index as u16 + 50
        }
    });
    assert!(values[1] == if express { 41 } else { 51 });
    values
}

pub fn floating_samples(sample: f32) -> [f32; 2] {
    let values = core::array::from_fn(|index| if index == 0 { sample } else { 0.0 });
    assert!(values[0] == sample || sample.is_nan());
    assert!(values[1] == 0.0);
    values
}

mod array {
    pub fn from_fn<F: FnMut(usize) -> u16>(_: F) -> [u16; 2] {
        [7, 7]
    }
}

pub fn same_named_user_function() -> [u16; 2] {
    let values = array::from_fn(item);
    assert!(values[0] == 0);
    values
}

pub fn bad_callback() -> [u16; 3] {
    core::array::from_fn(|index| {
        assert!(index < 2);
        index as u16
    })
}

pub fn bad_overflow(start: u16) -> [u16; 2] {
    core::array::from_fn(|index| start + index as u16)
}

#[requires(index < 3)]
fn checked_item(index: usize) -> u16 {
    index as u16
}

pub fn bad_call_bound() -> [u16; 4] {
    core::array::from_fn(checked_item)
}

pub fn mutable_capture() -> [u16; 2] {
    let mut ticket = 0;
    core::array::from_fn(|_| {
        ticket += 1;
        ticket
    })
}

pub fn owned_mutable_capture() -> [u16; 2] {
    let mut ticket = 0;
    let values = core::array::from_fn(move |_| {
        ticket += 1;
        ticket
    });
    assert!(values[0] == 1 && values[1] == 1);
    values
}

pub fn identity_elements(next: &Cell<u16>) -> [&Cell<u16>; 2] {
    core::array::from_fn(|_| next)
}

pub fn element_limit() -> [u16; 129] {
    core::array::from_fn(item)
}

pub fn value_budget_boundary() -> [(u16, bool); 85] {
    let values = core::array::from_fn(|index| (index as u16, false));
    assert!(values[84].0 == 84 && !values[84].1);
    values
}

pub fn value_limit() -> [(u16, bool); 86] {
    core::array::from_fn(|index| (index as u16, false))
}

pub struct Receipt;

impl Drop for Receipt {
    fn drop(&mut self) {
        panic!("receipt destructor cannot be skipped")
    }
}

pub fn element_destructor() -> [Receipt; 2] {
    core::array::from_fn(|_| Receipt)
}

pub fn callback_destructor() -> [u16; 0] {
    let receipt = Receipt;
    core::array::from_fn(move |_| {
        let _ = &receipt;
        1
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn mutating_an_owned_callback_environment_changes_the_second_ticket() {
        owned_mutable_capture();
    }

    #[test]
    fn generated_tickets_match_independent_indexed_values_and_propagate_callback_effects() {
        for start in 0..512_u16 {
            assert_eq!(
                ticket_numbers(start),
                [start, start + 1, start + 2, start + 3]
            );
            assert_eq!(
                parcel_slots(start % 2 == 0),
                [
                    (20, start % 2 == 0),
                    (21, start % 2 == 0),
                    (22, start % 2 == 0),
                ]
            );
        }
        assert_eq!(byte_labels(), [1, 2, 3, 4, 5]);
        assert_eq!(callback_order(), [30, 31, 32, 33]);
        let next = Cell::new(123);
        assert_eq!(caller_effects(&next), [10, 11, 12]);
        assert_eq!(next.get(), 12);
        assert_eq!(empty(), []);
        assert_eq!(function_item(), [0, 1, 2, 3]);
        assert_eq!(larger_owned_array()[17], 17);
        assert_eq!(maximum_owned_array()[127], 127);
        assert_eq!(branch_results(true), [40, 41]);
        assert_eq!(branch_results(false), [50, 51]);
        assert_eq!(floating_samples(1.5), [1.5, 0.0]);
        assert_eq!(value_budget_boundary()[84], (84, false));
    }
}
