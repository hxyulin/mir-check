#![no_std]
#![forbid(unsafe_code)]
#![feature(custom_mir, core_intrinsics, sync_unsafe_cell)]

use core::cell::SyncUnsafeCell;
use core::intrinsics::mir::*;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU32, Ordering};

#[repr(C, align(8))]
pub struct Record {
    revision: AtomicU32,
    payload: MaybeUninit<[u8; 12]>,
}

#[repr(C, align(8))]
struct Carrier {
    bytes: SyncUnsafeCell<[MaybeUninit<u8>; 16]>,
}

// Synthetic MIR exercises representation changes without adding unsafe Rust implementations.
#[custom_mir(dialect = "runtime", phase = "optimized")]
const fn erase(value: Record) -> Carrier {
    mir! {
        {
            RET = CastTransmute(Move(value));
            Return()
        }
    }
}

static ERASED: Carrier = erase(Record {
    revision: AtomicU32::new(0),
    payload: MaybeUninit::uninit(),
});
static DIRECT: SyncUnsafeCell<Record> = SyncUnsafeCell::new(Record {
    revision: AtomicU32::new(0),
    payload: MaybeUninit::uninit(),
});
static UNALIGNED: SyncUnsafeCell<[MaybeUninit<u8>; 16]> =
    SyncUnsafeCell::new([MaybeUninit::uninit(); 16]);

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share(pointer: *mut Record) -> &'static Record {
    mir! {
        {
            RET = &*pointer;
            Return()
        }
    }
}

pub fn restored() -> &'static Record {
    share(ERASED.bytes.get().cast::<u8>().cast::<Record>())
}

pub fn direct() -> &'static Record {
    share(DIRECT.get().cast::<u8>().cast::<Record>())
}

pub fn address_checks() {
    let pointer = ERASED.bytes.get().cast::<Record>();
    assert!(!pointer.is_null());
    assert!(pointer as usize & 7 == 0);
    assert!(pointer as usize == pointer.cast::<u8>() as usize);
}

pub fn field_addresses_preserve_offsets() {
    let base = ERASED.bytes.get() as usize;
    let field = (&raw const restored().payload) as usize;
    assert!(field == base + 4);
}

pub fn wrong_address_claim() {
    let pointer = ERASED.bytes.get();
    assert!(pointer.is_null());
}

pub fn restored_then_panics() {
    let _ = restored();
    panic!();
}

pub fn reads_are_not_initializer_snapshots() -> u32 {
    restored().revision.load(Ordering::Relaxed)
}

pub fn unaligned_is_unknown() -> &'static Record {
    share(UNALIGNED.get().cast::<Record>())
}

pub fn oversized_is_unknown() {
    let _ = ERASED.bytes.get().cast::<[u64; 32]>();
}

#[repr(C, align(8))]
struct Other {
    revision: AtomicU32,
    payload: MaybeUninit<[u8; 12]>,
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_other(pointer: *mut Other) -> &'static Other {
    mir! {
        {
            RET = &*pointer;
            Return()
        }
    }
}

pub fn unrelated_layout_is_unknown() {
    let _ = share_other(ERASED.bytes.get().cast::<Other>());
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn read_payload(value: &Record) -> [u8; 12] {
    mir! {
        {
            RET = CastTransmute((*value).payload);
            Return()
        }
    }
}

pub fn uninitialized_payload_is_unknown() {
    let _ = read_payload(restored());
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn write_record(pointer: *mut Record, revision: AtomicU32) {
    mir! {
        {
            (*pointer).revision = Move(revision);
            Return()
        }
    }
}

pub fn raw_writes_are_unknown() {
    write_record(ERASED.bytes.get().cast(), AtomicU32::new(1));
}

pub fn numeric_pointer_cannot_restore_storage() -> &'static Record {
    share(8_usize as *mut Record)
}

pub fn boundary() {}

pub fn unbounded_static_state() {
    let value = restored();
    loop {
        boundary();
        let _ = &raw const value.payload;
    }
}

pub fn invalidated_view_is_unknown() -> &'static Record {
    let value = restored();
    boundary();
    value
}

pub fn preserves_view() -> &'static Record {
    let value = restored();
    boundary();
    value
}

#[cfg(test)]
mod tests {
    #[test]
    fn static_views_and_addresses_replay_without_reading_uninitialized_storage() {
        let _ = super::restored();
        let _ = super::direct();
        super::address_checks();
        super::field_addresses_preserve_offsets();
    }
}

static ROWS: SyncUnsafeCell<[Record; 3]> = SyncUnsafeCell::new([
    Record {
        revision: AtomicU32::new(0),
        payload: MaybeUninit::uninit(),
    },
    Record {
        revision: AtomicU32::new(0),
        payload: MaybeUninit::uninit(),
    },
    Record {
        revision: AtomicU32::new(0),
        payload: MaybeUninit::uninit(),
    },
]);
static EMPTY: SyncUnsafeCell<[Record; 0]> = SyncUnsafeCell::new([]);
static LARGE: SyncUnsafeCell<[Record; 129]> = SyncUnsafeCell::new(
    [const {
        Record {
            revision: AtomicU32::new(0),
            payload: MaybeUninit::uninit(),
        }
    }; 129],
);

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_rows<const N: usize>(pointer: *mut [Record; N]) -> &'static [Record; N] {
    mir! { { RET = &*pointer; Return() } }
}

pub fn rows() -> &'static [Record] {
    share_rows(ROWS.get())
}

pub fn static_slice_cursor_offsets() {
    let slice = rows();
    assert!(slice.len() == 3);
    let mut cursor = slice.iter();
    let first = cursor.next().unwrap();
    let last = cursor.next_back().unwrap();
    let middle = cursor.nth(0).unwrap();
    assert!((&raw const last.payload) as usize == (&raw const first.payload) as usize + 32);
    assert!((&raw const middle.payload) as usize == (&raw const first.payload) as usize + 16);
    assert!(cursor.next().is_none());
    let _ = first.revision.load(Ordering::Relaxed);
}

pub fn static_array_index() {
    let array = share_rows(ROWS.get());
    let first = &array[0];
    let last = &array[2];
    assert!((&raw const last.payload) as usize == (&raw const first.payload) as usize + 32);
}

pub fn array_of_references_preserves_reference_values() {
    let references = [restored(), direct()];
    let value = references[0];
    let _ = value.revision.load(Ordering::Relaxed);
}

pub fn static_find_map_skips_later_panic() {
    let mut cursor = rows().iter();
    let mut calls = 0;
    let found = cursor.find_map(|row| {
        calls += 1;
        if calls == 1 { Some(row) } else { panic!() }
    });
    assert!(found.is_some());
    assert!(calls == 1);
    assert!(cursor.len() == 2);
}

pub fn static_find_map_reaches_later_panic() {
    let mut calls = 0;
    let _ = rows().iter().find_map(|_| {
        calls += 1;
        if calls == 1 { None::<u32> } else { panic!() }
    });
}

pub fn static_find_map_exhausts() {
    let mut calls = 0;
    let result = rows().iter().find_map(|row| {
        calls += 1;
        let _ = row.revision.load(Ordering::Relaxed);
        None::<u32>
    });
    assert!(result.is_none());
    assert!(calls == 3);
}

pub fn static_slice_payload_read_is_unknown() {
    let _ = read_payload(rows().iter().next().unwrap());
}

pub fn static_slice_budget_is_unknown() -> &'static [Record] {
    share_rows(LARGE.get())
}

pub fn empty_static_slice() -> &'static [Record] {
    share_rows(EMPTY.get())
}

pub fn empty_static_slice_is_invalidated() -> &'static [Record] {
    let slice = empty_static_slice();
    boundary();
    slice
}

#[cfg(test)]
#[test]
fn static_slice_references_replay_without_payload_reads() {
    static_slice_cursor_offsets();
    static_array_index();
    array_of_references_preserves_reference_values();
    static_find_map_skips_later_panic();
    static_find_map_exhausts();
    static_find_map_returns_later_reference();
    assert!(empty_static_slice().is_empty());
}

pub fn static_find_map_returns_later_reference() {
    let slice = rows();
    let first = slice.iter().next().unwrap();
    let mut cursor = slice.iter();
    let mut calls = 0;
    let last = cursor
        .find_map(|row| {
            calls += 1;
            if calls == 3 { Some(row) } else { None }
        })
        .unwrap();
    assert!((&raw const last.payload) as usize == (&raw const first.payload) as usize + 32);
    assert!(cursor.next().is_none());
}

pub fn static_find_map_callback_read_is_unknown() {
    let _ = rows().iter().find_map(|row| Some(read_payload(row)));
}

pub fn empty_static_iterator_is_invalidated() {
    let mut cursor = empty_static_slice().iter();
    boundary();
    assert!(cursor.next().is_none());
}
