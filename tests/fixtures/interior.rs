#![no_std]
#![forbid(unsafe_code)]

use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};

pub fn cell_aliases() {
    let cell = Cell::new(1_u8);
    let left = &cell;
    let right = &cell;
    left.set(7);
    assert!(right.get() == 7);
    let old = right.replace(9);
    assert!(old == 7 && left.get() == 9);
}

pub fn cell_callee(cell: &Cell<u8>) { cell.set(11); }

pub fn cell_calls(cell: &Cell<u8>) {
    cell_callee(cell);
    assert!(cell.get() == 11);
}

pub fn bad_cell(cell: &Cell<u8>) {
    cell.set(11);
    assert!(cell.get() == 12);
}

pub fn cell_branches(cell: &Cell<u8>, flag: bool) {
    if flag { cell.set(2); } else { cell.set(4); }
    assert!((flag && cell.get() == 2) || (!flag && cell.get() == 4));
}

pub fn two_cells(left: &Cell<u8>, right: &Cell<u8>) { left.set(right.get()); }

static COUNTER: AtomicU32 = AtomicU32::new(0);
static SMALL: AtomicU8 = AtomicU8::new(0);

pub struct Site(AtomicU32);
static SITE: Site = Site(AtomicU32::new(0));
impl Site {
    pub fn fail(&self) -> u32 {
        COUNTER.fetch_add(1, Ordering::Relaxed);
        self.0.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
    }
}

pub fn site() -> u32 { SITE.fail() }

pub fn counters() {
    let _value = COUNTER.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    let _value = SMALL.fetch_sub(1, Ordering::AcqRel).wrapping_sub(1);
    COUNTER.store(3, Ordering::Release);
    let _value = COUNTER.load(Ordering::Acquire);
    let _value = COUNTER.swap(4, Ordering::SeqCst);
}

pub fn wrong_increment() -> u32 { COUNTER.fetch_add(1, Ordering::Relaxed) + 1 }

pub fn unsupported_history() {
    COUNTER.store(7, Ordering::Relaxed);
    assert!(COUNTER.load(Ordering::Relaxed) == 7);
}

fn load(order: Ordering) -> u32 { COUNTER.load(order) }
pub fn bad_load() -> u32 { load(Ordering::Release) }
fn store(order: Ordering) { COUNTER.store(7, order); }
pub fn bad_store() { store(Ordering::Acquire); }

pub fn guarded_ordering(order: Ordering) -> u32 {
    match order {
        Ordering::Relaxed | Ordering::Acquire | Ordering::SeqCst => COUNTER.load(order),
        _ => 0,
    }
}

pub fn bad_ordering(order: Ordering) -> u32 { COUNTER.load(order) }

pub fn refcell_conflict() {
    let value = RefCell::new(0_u8);
    let _reader = value.borrow();
    let _writer = value.borrow_mut();
}

pub fn cell_callback() {
    let cell = Cell::new(0_u8);
    let _values = [1_u8, 2, 3].map(|value| { cell.set(cell.get() + value); cell.get() });
    assert!(cell.get() == 6);
}
