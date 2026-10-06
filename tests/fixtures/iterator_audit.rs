#![no_std]
#![forbid(unsafe_code)]

pub fn mixed_scores(scores: [u16; 5], skip: usize) {
    let mut cursor = scores.iter();
    if skip < 5 {
        assert!(*cursor.nth(skip).unwrap() == scores[skip]);
        assert!(cursor.len() == 4 - skip);
        if skip < 4 {
            assert!(*cursor.next_back().unwrap() == scores[4]);
            assert!(cursor.len() == 3 - skip);
        }
    } else {
        assert!(cursor.nth(skip).is_none());
        assert!(cursor.next_back().is_none());
    }
}

pub fn oversized_skips() {
    let scores = [11_u16, 29, 41];
    let mut forward = scores.iter();
    assert!(forward.nth(usize::MAX).is_none());
    assert!(forward.next().is_none());
    assert!(forward.next_back().is_none());
    assert!(forward.len() == 0);
    let mut reverse = scores.iter();
    assert!(reverse.nth_back(usize::MAX).is_none());
    assert!(reverse.next().is_none());
    assert!(reverse.len() == 0);
}

pub fn zero_sized_slots() {
    let mut slots = [(); 4];
    let mut cursor = slots.iter_mut();
    let _first = cursor.next().unwrap();
    let _last = cursor.next_back().unwrap();
    assert!(cursor.len() == 2);
    assert!(cursor.nth_back(usize::MAX).is_none());
    assert!(cursor.next().is_none());
}

pub fn skipped_storage(slots: &mut [u8; 4], skip: usize) {
    let before = *slots;
    if skip < 4 {
        let mut cursor = slots.iter_mut();
        *cursor.nth(skip).unwrap() = 19;
        assert!(cursor.len() == 3 - skip);
        assert!(slots[skip] == 19);
        if skip != 0 {
            assert!(slots[0] == before[0]);
        }
        if skip != 1 {
            assert!(slots[1] == before[1]);
        }
        if skip != 2 {
            assert!(slots[2] == before[2]);
        }
        if skip != 3 {
            assert!(slots[3] == before[3]);
        }
    } else {
        assert!(slots.iter_mut().nth(skip).is_none());
        assert!(slots[0] == before[0] && slots[1] == before[1]);
        assert!(slots[2] == before[2] && slots[3] == before[3]);
    }
}

pub fn shared_tally() {
    let tally = core::cell::Cell::new(0_u8);
    let alias = &tally;
    let cards = [2_u16, 5, 8];
    let mut cursor = cards.iter();
    assert!(cursor.any(|card| {
        alias.set(alias.get() + 1);
        *card == 5
    }));
    assert!(tally.get() == 2);
    assert!(cursor.len() == 1);
    assert!(*cursor.next_back().unwrap() == 8);
}

pub fn separated_writes() {
    let mut scores = [11_u16, 29, 41];
    let mut cursor = scores.iter_mut();
    let first = cursor.next().unwrap();
    let last = cursor.next_back().unwrap();
    *first = 3;
    *last = 7;
    assert!(*first == 3 && *last == 7);
    assert!(*cursor.next().unwrap() == 29);
}

pub fn wrong_alias_claim() {
    let mut scores = [11_u16, 29, 41];
    let mut cursor = scores.iter_mut();
    let first = cursor.next().unwrap();
    let last = cursor.next_back().unwrap();
    *first = 3;
    *last = 7;
    assert!(*first == *last);
}

pub fn unavailable_view(scores: [u16; 3]) -> usize {
    scores.iter().as_slice().len()
}

pub fn owned_mutable_environment() {
    let mut visited = 0_u8;
    assert!([11_u16, 29].iter().all(move |_| {
        visited += 1;
        visited == 1
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn native_execution_rejects_the_alias_claim() {
        wrong_alias_claim();
    }

    #[test]
    #[should_panic]
    fn native_execution_retains_owned_callback_state() {
        owned_mutable_environment();
    }

    #[test]
    fn cursor_and_storage_checks_agree_with_bounded_native_execution() {
        for seed in 0..=u8::MAX {
            let scores = [u16::from(seed), 701, 83, 509, 17];
            for skip in [0, 1, 2, 3, 4, 5, usize::MAX] {
                mixed_scores(scores, skip);
                let initial = [seed, 31, 73, 127];
                let mut slots = initial;
                skipped_storage(&mut slots, skip);
                let mut expected = initial;
                if skip < 4 {
                    expected[skip] = 19;
                }
                assert_eq!(slots, expected);
            }
        }
        oversized_skips();
        zero_sized_slots();
        shared_tally();
        separated_writes();
    }
}
