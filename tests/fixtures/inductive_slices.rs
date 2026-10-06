#![no_std]
#![forbid(unsafe_code)]

pub fn shared_slice_reads(bytes: &[u8]) {
    for byte in bytes {
        let limited = *byte & 31;
        assert!(limited < 32);
    }
}

pub fn shared_slice_bad_bound(bytes: &[u8]) {
    for byte in bytes {
        assert!(*byte < 255);
    }
}

pub fn mutable_slice_writes(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        *byte = 0;
        assert!(*byte == 0);
    }
}

pub fn mutable_slice_bad_write(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        *byte = 1;
        assert!(*byte == 0);
    }
}

pub fn earlier_references_keep_their_element(bytes: &[u8; 3]) {
    let mut iterator = bytes.iter();
    let Some(first) = iterator.next() else {
        return;
    };
    let saved = *first;
    while let Some(_next) = iterator.next() {
        assert!(*first == saved);
    }
}

pub fn retained_mutable_references_are_disjoint(bytes: &mut [u8; 3]) {
    let mut iterator = bytes.iter_mut();
    let Some(first) = iterator.next() else {
        return;
    };
    *first = 17;
    while let Some(next) = iterator.next() {
        *next = 0;
        assert!(*first == 17);
    }
}

pub fn a_false_retained_reference_claim(bytes: &mut [u8; 3]) {
    let mut iterator = bytes.iter_mut();
    let Some(first) = iterator.next() else {
        return;
    };
    *first = 17;
    while let Some(next) = iterator.next() {
        *next = 0;
        assert!(*first == 0);
    }
}

pub fn advancing_from_both_ends(bytes: &mut [u8; 4]) {
    let mut iterator = bytes.iter_mut();
    while let Some(front) = iterator.next() {
        *front = 7;
        if let Some(back) = iterator.next_back() {
            *back = 9;
            assert!(*front == 7);
        }
    }
}

pub fn skipping_elements(bytes: &[u8]) {
    let mut iterator = bytes.iter();
    while let Some(byte) = iterator.nth(2) {
        assert!((*byte & 7) <= 7);
    }
}

pub fn backward_skips(bytes: &[u8]) {
    let mut iterator = bytes.iter();
    while let Some(byte) = iterator.nth_back(2) {
        assert!((*byte & 7) <= 7);
    }
}

pub fn indexed_local_borrows(bytes: &mut [u8; 4]) {
    let mut index = 0;
    while index < 4 {
        let byte = &mut bytes[index];
        *byte = 13;
        assert!(*byte == 13);
        index += 1;
    }
}

pub fn an_off_by_one_indexed_borrow(bytes: &mut [u8; 4]) {
    let mut index = 0;
    while index <= 4 {
        let byte = &mut bytes[index];
        *byte = 13;
        index += 1;
    }
}

fn clear(byte: &mut u8) {
    *byte = 0;
}

pub fn indexed_references_reach_helper_storage(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        clear(byte);
        assert!(*byte == 0);
    }
}

#[doc = "<!-- mir-check:v1:requires:byte < 32 -->"]
#[doc = "<!-- mir-check:v1:ensures:final_byte > byte -->"]
fn increase(byte: &mut u8) {
    *byte += 1;
}

pub fn indexed_helper_snapshots(bytes: &mut [u8; 3]) {
    for byte in bytes.iter_mut() {
        *byte &= 31;
        increase(byte);
        assert!(*byte <= 32);
    }
}

#[doc = "<!-- mir-check:v1:ensures:final_byte == byte -->"]
fn a_false_indexed_contract(byte: &mut u8) {
    *byte = 17;
}

pub fn indexed_entry_values_are_not_current_values(bytes: &mut [u8; 3]) {
    for byte in bytes.iter_mut() {
        *byte = 0;
        a_false_indexed_contract(byte);
    }
}

pub fn cloned_cursors_advance_independently(bytes: &[u8; 3]) {
    let mut iterator = bytes.iter();
    let mut cloned = iterator.clone();
    while let Some(first) = iterator.next() {
        let Some(second) = cloned.next() else {
            assert!(false);
            return;
        };
        assert!(*first == *second);
    }
}

pub fn borrowed_count_exhausts_the_original_cursor(bytes: &[u8]) {
    let mut iterator = bytes.iter();
    while iterator.len() > 0 {
        let count = iterator.by_ref().count();
        assert!(count > 0);
        assert!(iterator.len() == 0);
    }
}

pub fn count_cannot_leave_a_borrowed_cursor_nonempty(bytes: &[u8]) {
    let mut iterator = bytes.iter();
    while iterator.len() > 0 {
        let _count = iterator.by_ref().count();
        assert!(iterator.len() > 0);
    }
}

pub fn skipping_past_the_end_cannot_leave_items(bytes: &[u8; 3]) {
    let mut iterator = bytes.iter();
    while iterator.len() > 0 {
        let result = iterator.nth(usize::MAX);
        assert!(result.is_none());
        assert!(iterator.len() == 0);
    }
}

pub fn word_array_iterator_reads(words: &[u16; 4]) {
    for word in words.iter() {
        assert!((*word & 255) < 256);
    }
}

pub fn word_array_iterator_writes(words: &mut [u16; 4]) {
    for word in words.iter_mut() {
        *word = 42;
        assert!(*word == 42);
    }
}

pub fn an_incorrect_word_array_write(words: &mut [u16; 4]) {
    for word in words.iter_mut() {
        *word = 43;
        assert!(*word == 42);
    }
}

pub fn boolean_array_iterator_writes(flags: &mut [bool; 3]) {
    for flag in flags.iter_mut() {
        *flag = true;
        assert!(*flag);
    }
}

pub fn a_nonbyte_unbounded_slice_is_still_unknown(words: &[u16]) {
    for word in words.iter() {
        assert!((*word & 255) < 256);
    }
}

pub fn indexed_slice_views_are_still_unknown(bytes: &[u8]) {
    if bytes.len() > 1 {
        for byte in bytes[1..].iter() {
            assert!((*byte & 31) < 32);
        }
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_lengths_skips_counts_and_cursor_clones_match_native_rust() {
        for length in 0..=6 {
            let mut bytes = [255; 6];
            shared_slice_reads(&bytes[..length]);
            skipping_elements(&bytes[..length]);
            backward_skips(&bytes[..length]);
            borrowed_count_exhausts_the_original_cursor(&bytes[..length]);
            mutable_slice_writes(&mut bytes[..length]);
            assert!(bytes[..length].iter().all(|value| *value == 0));
            indexed_references_reach_helper_storage(&mut bytes[..length]);
        }
        initialized_local_scalar_storage();
        initialized_local_byte_storage();
        cloned_cursors_advance_independently(&[1, 5, 9]);
        skipping_past_the_end_cannot_leave_items(&[1, 5, 9]);
        indexed_local_borrows(&mut [0; 4]);
        let mut bytes = [0; 4];
        advancing_from_both_ends(&mut bytes);
        assert_eq!(bytes, [7, 7, 9, 9]);
    }

    #[test]
    fn retained_references_keep_their_address_for_every_small_input() {
        for a in 0..4 {
            for b in 0..4 {
                for c in 0..4 {
                    earlier_references_keep_their_element(&[a, b, c]);
                    let mut bytes = [a, b, c];
                    retained_mutable_references_are_disjoint(&mut bytes);
                    assert_eq!(bytes, [17, 0, 0]);
                }
            }
        }
    }

    #[test]
    fn changed_assertions_and_bounds_replay_as_real_panics() {
        assert!(std::panic::catch_unwind(|| shared_slice_bad_bound(&[255])).is_err());
        assert!(std::panic::catch_unwind(|| mutable_slice_bad_write(&mut [0; 3])).is_err());
        assert!(
            std::panic::catch_unwind(|| a_false_retained_reference_claim(&mut [0; 3])).is_err()
        );
        assert!(std::panic::catch_unwind(|| an_off_by_one_indexed_borrow(&mut [0; 4])).is_err());
        assert!(
            std::panic::catch_unwind(|| count_cannot_leave_a_borrowed_cursor_nonempty(&[1]))
                .is_err()
        );
        assert!(std::panic::catch_unwind(|| an_incorrect_word_array_write(&mut [0; 4])).is_err());
    }

    #[test]
    fn indexed_mutation_preserves_contract_entry_values_and_updates_only_the_selected_element() {
        let mut bytes = [0, 255, 16];
        indexed_helper_snapshots(&mut bytes);
        assert_eq!(bytes, [1, 32, 17]);
        let mut byte = 0;
        let original = byte;
        a_false_indexed_contract(&mut byte);
        assert_ne!(byte, original);
        indexed_entry_values_are_not_current_values(&mut [0; 3]);
        returned_references_keep_their_index_and_current_value(&mut [0, 255, 16]);
        assert!(
            std::panic::catch_unwind(return_layout_inference_cannot_assume_the_first_element)
                .is_err()
        );
        let mut words = [0, 65535, 17, 29];
        word_array_iterator_reads(&words);
        word_array_iterator_writes(&mut words);
        assert_eq!(words, [42; 4]);
        let mut flags = [false, true, false];
        boolean_array_iterator_writes(&mut flags);
        assert_eq!(flags, [true; 3]);
    }
}

pub fn initialized_local_scalar_storage() {
    let mut words = [0_u16; 4];
    for word in words.iter_mut() {
        *word = 42;
        assert!(*word == 42);
    }
    assert!(words[0] == 42);
}

pub fn initialized_local_byte_storage() {
    let mut bytes = [0_u8; 4];
    for byte in bytes.iter_mut() {
        *byte = 9;
        assert!(*byte == 9);
    }
    assert!(bytes[0] == 9);
}

#[doc = "<!-- mir-check:v1:requires:byte < 32 -->"]
#[doc = "<!-- mir-check:v1:ensures:result == final_byte && final_byte > byte -->"]
fn increase_and_return(byte: &mut u8) -> &mut u8 {
    *byte += 1;
    byte
}

pub fn returned_references_keep_their_index_and_current_value(bytes: &mut [u8; 3]) {
    for byte in bytes.iter_mut() {
        *byte &= 31;
        let returned = increase_and_return(byte);
        assert!(*returned <= 32);
    }
}

fn return_the_other_element<'a>(_current: &'a u8, other: &'a u8) -> &'a u8 {
    other
}

pub fn return_layout_inference_cannot_assume_the_first_element() {
    let bytes = [17_u8, 0, 2];
    for current in bytes.iter() {
        let other = &bytes[1];
        let returned = return_the_other_element(current, other);
        assert!(*returned == *current);
    }
}
