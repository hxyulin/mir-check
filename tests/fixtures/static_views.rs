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

#[repr(C, align(4))]
struct Lanes {
    low: core::sync::atomic::AtomicU16,
    high: core::sync::atomic::AtomicU16,
}

#[repr(C)]
struct Sample {
    prefix: AtomicU32,
    lanes: Lanes,
    unused: MaybeUninit<[u8; 8]>,
}

static SAMPLE: SyncUnsafeCell<Sample> = SyncUnsafeCell::new(Sample {
    prefix: AtomicU32::new(0),
    lanes: Lanes {
        low: core::sync::atomic::AtomicU16::new(9),
        high: core::sync::atomic::AtomicU16::new(23),
    },
    unused: MaybeUninit::uninit(),
});

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_sample(pointer: *mut Sample) -> &'static Sample {
    mir! { { RET = &*pointer; Return() } }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_word(pointer: *const AtomicU32) -> &'static AtomicU32 {
    mir! { { RET = &*pointer; Return() } }
}

fn sample() -> &'static Sample {
    share_sample(SAMPLE.get())
}

pub fn atomic_overlay_load() -> u32 {
    share_word((&raw const sample().lanes).cast()).load(Ordering::Relaxed)
}

pub fn atomic_overlay_at_nonzero_offset() {
    let base = SAMPLE.get() as usize;
    let pointer = (&raw const sample().lanes).cast::<AtomicU32>();
    assert!(pointer as usize == base + 4);
    let value = share_word(pointer).load(Ordering::Acquire);
    if value < 10 {
        assert!(value + 1 <= 10);
    }
}

pub fn atomic_overlay_wrong_bound() {
    let value = atomic_overlay_load();
    assert!(value < 10);
}

pub fn atomic_overlay_is_not_an_initializer_snapshot() {
    assert!(atomic_overlay_load() == 0);
}

#[repr(C, align(4))]
struct Padded {
    first: core::sync::atomic::AtomicU8,
    second: core::sync::atomic::AtomicU16,
}

#[repr(C)]
struct Rejected {
    padded: Padded,
    uninitialized: MaybeUninit<AtomicU32>,
    plain: SyncUnsafeCell<u32>,
    tail_padding: core::sync::atomic::AtomicU8,
}
static REJECTED: SyncUnsafeCell<Rejected> = SyncUnsafeCell::new(Rejected {
    padded: Padded {
        first: core::sync::atomic::AtomicU8::new(0),
        second: core::sync::atomic::AtomicU16::new(0),
    },
    uninitialized: MaybeUninit::uninit(),
    plain: SyncUnsafeCell::new(0),
    tail_padding: core::sync::atomic::AtomicU8::new(0),
});

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_rejected(pointer: *mut Rejected) -> &'static Rejected {
    mir! { { RET = &*pointer; Return() } }
}

pub fn atomic_overlay_padding_is_unknown() {
    let record = share_rejected(REJECTED.get());
    let _ = share_word((&raw const record.padded).cast());
}

pub fn atomic_overlay_uninitialized_is_unknown() {
    let record = share_rejected(REJECTED.get());
    let _ = share_word((&raw const record.uninitialized).cast());
}

pub fn atomic_overlay_plain_storage_is_unknown() {
    let record = share_rejected(REJECTED.get());
    let _ = share_word((&raw const record.plain).cast());
}

pub fn atomic_overlay_cannot_extend_a_field() {
    let record = share_rejected(REJECTED.get());
    let _ = share_word((&raw const record.tail_padding).cast());
}

#[repr(C, align(2))]
struct Flags {
    bits: [core::sync::atomic::AtomicBool; 2],
}
#[repr(C)]
struct FlagSample {
    flags: Flags,
    unused: MaybeUninit<[u8; 2]>,
}
static FLAGS: SyncUnsafeCell<FlagSample> = SyncUnsafeCell::new(FlagSample {
    flags: Flags {
        bits: [const { core::sync::atomic::AtomicBool::new(false) }; 2],
    },
    unused: MaybeUninit::uninit(),
});
#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_flags(pointer: *mut FlagSample) -> &'static FlagSample {
    mir! { { RET = &*pointer; Return() } }
}
#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_halfword(
    pointer: *const core::sync::atomic::AtomicU16,
) -> &'static core::sync::atomic::AtomicU16 {
    mir! { { RET = &*pointer; Return() } }
}

pub fn atomic_bool_array_overlay() -> u16 {
    let flags = share_flags(FLAGS.get());
    share_halfword((&raw const flags.flags).cast()).load(Ordering::Relaxed)
}

#[cfg(test)]
#[test]
fn dense_atomic_overlays_replay_only_without_concurrent_accesses() {
    let _ = atomic_overlay_load();
    atomic_overlay_at_nonzero_offset();
    let _ = atomic_overlay_reborrow();
    let _ = atomic_bool_array_overlay();
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn shared_word_reborrow(pointer: *const AtomicU32) -> &'static AtomicU32 {
    mir! {
        let word: &'static AtomicU32;
        {
            word = &*pointer;
            RET = &*word;
            StorageDead(word);
            Return()
        }
    }
}

pub fn atomic_overlay_reborrow() -> u32 {
    shared_word_reborrow((&raw const sample().lanes).cast()).load(Ordering::Relaxed)
}

#[repr(C, align(4))]
struct MisalignedSample {
    prefix: core::sync::atomic::AtomicU16,
    bytes: [core::sync::atomic::AtomicU8; 4],
    unused: MaybeUninit<[u8; 2]>,
}
static MISALIGNED_SAMPLE: SyncUnsafeCell<MisalignedSample> =
    SyncUnsafeCell::new(MisalignedSample {
        prefix: core::sync::atomic::AtomicU16::new(0),
        bytes: [const { core::sync::atomic::AtomicU8::new(0) }; 4],
        unused: MaybeUninit::uninit(),
    });
#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_misaligned(pointer: *mut MisalignedSample) -> &'static MisalignedSample {
    mir! { { RET = &*pointer; Return() } }
}

pub fn atomic_overlay_misaligned_field_is_unknown() {
    let record = share_misaligned(MISALIGNED_SAMPLE.get());
    let _ = share_word((&raw const record.bytes).cast());
}

#[repr(C)]
union OpaqueUnion {
    pointer: core::mem::ManuallyDrop<core::sync::atomic::AtomicPtr<()>>,
    bytes: [u8; 8],
}
#[repr(C)]
struct PrefixSample {
    word: AtomicU32,
    opaque_union: OpaqueUnion,
    pointer: core::sync::atomic::AtomicPtr<()>,
    unused: MaybeUninit<[u8; 3]>,
}
static PREFIX_SAMPLE: SyncUnsafeCell<PrefixSample> = SyncUnsafeCell::new(PrefixSample {
    word: AtomicU32::new(0),
    opaque_union: OpaqueUnion { bytes: [0; 8] },
    pointer: core::sync::atomic::AtomicPtr::new(core::ptr::null_mut()),
    unused: MaybeUninit::uninit(),
});
#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_prefix(pointer: *mut PrefixSample) -> &'static PrefixSample {
    mir! { { RET = &*pointer; Return() } }
}

pub fn opaque_fields_outside_an_atomic_prefix() -> u32 {
    let source = share_prefix(PREFIX_SAMPLE.get());
    share_word((&raw const *source).cast()).load(Ordering::Relaxed)
}

pub fn atomic_prefix_cannot_read_a_union() {
    let source = share_prefix(PREFIX_SAMPLE.get());
    let _ = share_word((&raw const source.opaque_union).cast()).load(Ordering::Relaxed);
}

pub fn atomic_prefix_cannot_read_a_pointer() {
    let source = share_prefix(PREFIX_SAMPLE.get());
    let _ = share_word((&raw const source.pointer).cast()).load(Ordering::Relaxed);
}

pub fn maybe_uninit_payload_address() {
    let source = share_rejected(REJECTED.get());
    let pointer = source.uninitialized.as_ptr();
    assert!(!pointer.is_null());
    assert!(pointer as usize == (&raw const source.uninitialized) as usize);
}

pub fn maybe_uninit_address_does_not_prove_initialization() {
    let source = share_rejected(REJECTED.get());
    let _ = share_word(source.uninitialized.as_ptr()).load(Ordering::Relaxed);
}

fn capture_reference(value: &'static Record) -> u32 {
    let read = || value.revision.load(Ordering::Relaxed);
    read()
}

pub fn zero_arg_closure_captures_static_reference() -> u32 {
    capture_reference(restored())
}

pub fn zero_arg_then_captures_static_reference() -> Option<&'static Record> {
    let value = restored();
    true.then(|| value)
}

pub fn zero_arg_then_calls_function_item() -> Option<&'static Record> {
    true.then(restored)
}

fn replace_reference(value: &mut &'static Record) {
    *value = direct();
}

pub fn a_mutable_reference_slot_preserves_static_views() -> u32 {
    let mut value = restored();
    replace_reference(&mut value);
    value.revision.load(Ordering::Relaxed)
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_mutable_record(pointer: *mut Record) -> &'static mut Record {
    mir! { { RET = &mut *pointer; Return() } }
}

pub fn mutable_static_addresses_are_supported() {
    let _ = share_mutable_record(DIRECT.get());
}

#[cfg(test)]
#[test]
fn prefix_and_reference_operations_replay_without_payload_reads() {
    let _ = opaque_fields_outside_an_atomic_prefix();
    maybe_uninit_payload_address();
    let _ = zero_arg_closure_captures_static_reference();
    let _ = zero_arg_then_captures_static_reference();
    let _ = zero_arg_then_calls_function_item();
    let _ = a_mutable_reference_slot_preserves_static_views();
}

static INNER_WORD: SyncUnsafeCell<u32> = SyncUnsafeCell::new(3);
#[repr(C, align(4))]
struct ByteWord {
    bytes: [u8; 4],
}
static BYTE_WORD: SyncUnsafeCell<ByteWord> = SyncUnsafeCell::new(ByteWord { bytes: [0; 4] });
static UNINITIALIZED_WORD: SyncUnsafeCell<MaybeUninit<u32>> =
    SyncUnsafeCell::new(MaybeUninit::uninit());

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_plain_word(pointer: *mut u32) -> &'static u32 {
    mir! { { RET = &*pointer; Return() } }
}

pub fn certified_raw_get_preserves_an_address() {
    let pointer = core::cell::UnsafeCell::<u32>::raw_get(raw_word_container(&INNER_WORD).cast());
    assert!(!pointer.is_null());
    let _ = share_plain_word(pointer);
}

pub fn a_certified_raw_get_cannot_return_null() {
    let pointer = core::cell::UnsafeCell::<u32>::raw_get(raw_word_container(&INNER_WORD).cast());
    assert!(pointer.is_null());
}

pub fn raw_get_cannot_certify_an_unrelated_pointee() {
    let pointer = core::cell::UnsafeCell::<u32>::raw_get(BYTE_WORD.get().cast());
    let _ = share_plain_word(pointer);
}

pub fn raw_get_cannot_initialize_a_payload() {
    let pointer = core::cell::UnsafeCell::<u32>::raw_get(UNINITIALIZED_WORD.get().cast());
    let _ = share_plain_word(pointer);
}

#[cfg(test)]
#[test]
fn a_certified_raw_get_preserves_a_native_address() {
    certified_raw_get_preserves_an_address();
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn raw_word_container(reference: &SyncUnsafeCell<u32>) -> *const SyncUnsafeCell<u32> {
    mir! { { RET = &raw const *reference; Return() } }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_mutable_plain_word(pointer: *mut u32) -> &'static mut u32 {
    mir! { { RET = &mut *pointer; Return() } }
}

pub fn a_mutable_static_borrow_does_not_retain_a_payload() -> u32 {
    *share_mutable_plain_word(INNER_WORD.get())
}

pub fn mutable_static_stores_remain_opaque() {
    *share_mutable_plain_word(INNER_WORD.get()) = 7;
}

pub fn a_panic_after_a_mutable_static_borrow_is_reachable() {
    let _ = share_mutable_plain_word(INNER_WORD.get());
    panic!();
}

pub fn uninitialized_static_payloads_cannot_be_borrowed_mutably() {
    let _ = share_mutable_plain_word(UNINITIALIZED_WORD.get().cast());
}

#[cfg(test)]
#[test]
fn mutable_static_address_operations_replay_without_payload_reads() {
    mutable_static_addresses_are_supported();
    mutable_static_stores_remain_opaque();
}

pub fn changing_pointer_spelling_does_not_grant_write_access() {
    let pointer = raw_word_container(&INNER_WORD).cast::<u32>().cast_mut();
    let _ = share_mutable_plain_word(pointer);
}

static WORD_PAIR: SyncUnsafeCell<[u32; 2]> = SyncUnsafeCell::new([1, 2]);

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_mutable_words(pointer: *mut [u32; 2]) -> &'static mut [u32; 2] {
    mir! { { RET = &mut *pointer; Return() } }
}

pub fn a_mutable_static_array_keeps_its_typed_address() {
    let _ = share_mutable_words(WORD_PAIR.get());
}

fn consume_mutable_slice(_value: &mut [u32]) {}

pub fn mutable_static_slice_coercions_remain_unknown() {
    consume_mutable_slice(share_mutable_words(WORD_PAIR.get()));
}

static OPTIONAL_WORD: SyncUnsafeCell<Option<u32>> = SyncUnsafeCell::new(None);

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share_optional_word(pointer: *mut Option<u32>) -> &'static Option<u32> {
    mir! { { RET = &*pointer; Return() } }
}

pub fn opaque_static_variants_need_runtime_storage() -> bool {
    share_optional_word(OPTIONAL_WORD.get()).is_none()
}
