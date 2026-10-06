#![no_std]
#![forbid(unsafe_code)]

pub struct Tray {
    pub slot: u8,
    pub ticks: u16,
}

fn advance(tray: &mut Tray) {
    if tray.slot == 3 {
        tray.slot = 0;
    } else {
        tray.slot += 1;
    }
    tray.ticks = 0;
}

pub fn persistent_struct_storage() -> ! {
    let mut tray = Tray { slot: 0, ticks: 0 };
    loop {
        advance(&mut tray);
        assert!(tray.slot < 4);
        assert!(tray.ticks == 0);
    }
}

fn broken_advance(tray: &mut Tray) {
    if tray.slot == 4 {
        tray.slot = 0;
    } else {
        tray.slot += 1;
    }
}

pub fn a_bad_write_through_a_callee() -> ! {
    let mut tray = Tray { slot: 0, ticks: 0 };
    loop {
        broken_advance(&mut tray);
        assert!(tray.slot < 4);
    }
}

#[doc = "<!-- mir-check:v1:requires:count < 15 -->"]
#[doc = "<!-- mir-check:v1:ensures:final_count > count -->"]
fn increment(count: &mut u8) {
    *count += 1;
}

pub fn entry_snapshots_are_separate_from_mutable_storage() -> ! {
    let mut count = 0_u8;
    loop {
        increment(&mut count);
        if count == 15 {
            count = 0;
        }
    }
}

#[doc = "<!-- mir-check:v1:ensures:final_count == count -->"]
fn incorrect_snapshot_contract(count: &mut u8) {
    *count = 11;
}

pub fn a_mutation_cannot_change_the_entry_snapshot() -> ! {
    let mut count = 0_u8;
    loop {
        incorrect_snapshot_contract(&mut count);
    }
}

pub fn writes_to_a_root_borrow(bytes: &mut [u8; 4]) {
    let mut index = 0_usize;
    while index < 4 {
        bytes[index] = 0;
        index += 1;
    }
    assert!(bytes[0] == 0);
}

pub fn an_off_by_one_root_write(bytes: &mut [u8; 4]) {
    let mut index = 0_usize;
    while index <= 4 {
        bytes[index] = 0;
        index += 1;
    }
}

pub fn a_shared_root_borrow(bytes: &[u8; 4]) -> ! {
    let mut index = 0_usize;
    loop {
        let _byte = bytes[index];
        if index == 3 {
            index = 0;
        } else {
            index += 1;
        }
    }
}

pub fn interior_storage_is_not_assumed(cell: &core::cell::Cell<u8>) -> ! {
    loop {
        cell.set(0);
    }
}

pub fn a_changing_reference_target(flag: bool) -> ! {
    let mut first = 0_u8;
    let mut second = 0_u8;
    loop {
        let target = if flag { &mut first } else { &mut second };
        *target = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_callee_write_reaches_the_original_storage() {
        let mut tray = Tray { slot: 0, ticks: 0 };
        for _ in 0..4 {
            broken_advance(&mut tray);
        }
        assert!(tray.slot >= 4);
        advance(&mut tray);
        assert_eq!(tray.ticks, 0);
    }

    #[test]
    fn a_mutation_changes_the_value_but_not_its_saved_entry() {
        let mut count = 0_u8;
        let before = count;
        incorrect_snapshot_contract(&mut count);
        assert_ne!(count, before);
    }

    #[test]
    #[should_panic]
    fn the_inclusive_loop_reaches_the_out_of_bounds_write() {
        an_off_by_one_root_write(&mut [0; 4]);
    }
}
