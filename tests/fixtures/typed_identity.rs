#![no_std]
#![forbid(unsafe_code)]

use core::sync::atomic::{AtomicU32 as CoreAtomic, Ordering as CoreOrdering};

pub enum Ordering {
    Relaxed,
    Release,
    Acquire,
    AcqRel,
    SeqCst,
}

struct AtomicU32 {
    value: u32,
}

impl AtomicU32 {
    fn new(value: u32) -> Self {
        Self { value }
    }

    fn load(&self, order: Ordering) -> u32 {
        match order {
            Ordering::AcqRel => self.value.wrapping_add(2),
            _ => self.value,
        }
    }
}

struct Cell {
    value: u8,
}

impl Cell {
    fn new(value: u8) -> Self {
        Self { value }
    }

    fn get(&self) -> u8 {
        self.value.wrapping_add(1)
    }
}

pub fn application_atomic_names_execute_their_bodies() {
    let parcel_count = AtomicU32::new(10);
    assert!(parcel_count.load(Ordering::Release) == 10);
    assert!(parcel_count.load(Ordering::AcqRel) == 12);
}

pub fn application_cell_names_execute_their_bodies() {
    let shelf = Cell::new(8);
    assert!(shelf.get() == 9);
}

pub fn application_cell_result_mutation() {
    let shelf = Cell::new(8);
    assert!(shelf.get() == 8);
}

fn panic_gate() -> u8 {
    9
}

pub fn application_panic_names_execute_their_bodies() {
    assert!(panic_gate() == 9);
}

pub fn core_orderings_use_the_compiler_enum(order: CoreOrdering) {
    let inventory = CoreAtomic::new(0);
    match order {
        CoreOrdering::Relaxed | CoreOrdering::Release | CoreOrdering::SeqCst => {
            inventory.store(1, order);
        }
        _ => {}
    }
}

pub fn invalid_core_ordering_is_refuted() {
    let inventory = CoreAtomic::new(0);
    store(&inventory, CoreOrdering::Acquire);
}

fn store(inventory: &CoreAtomic, order: CoreOrdering) {
    inventory.store(1, order);
}

pub fn interior_storage_outside_the_model_remains_unknown() {
    let inventory = core::cell::RefCell::new(0);
    let _reader = inventory.borrow();
    let _writer = inventory.borrow_mut();
}
