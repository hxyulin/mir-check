#![no_std]
#![forbid(unsafe_code)]
#![feature(custom_mir, core_intrinsics, sync_unsafe_cell, type_alias_impl_trait)]

use core::cell::{SyncUnsafeCell, UnsafeCell};
use core::intrinsics::mir::*;
use core::mem::MaybeUninit;

#[derive(Clone, Copy)]
struct Palette {
    tint: (u8, bool),
    levels: [u8; 4],
    filter: Option<fn(u8) -> u8>,
}

static PALETTE: SyncUnsafeCell<Palette> = SyncUnsafeCell::new(Palette {
    tint: (0, false),
    levels: [0; 4],
    filter: None,
});
#[repr(C, align(2))]
struct WordCarrier {
    bytes: SyncUnsafeCell<[MaybeUninit<u8>; 2]>,
}

struct ByteCarrier<const N: usize> {
    bytes: SyncUnsafeCell<[MaybeUninit<u8>; N]>,
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
const fn hide<S, T>(value: S) -> T {
    mir! {
        {
            RET = CastTransmute(Move(value));
            Return()
        }
    }
}

static UNINIT: WordCarrier = hide(SyncUnsafeCell::new(MaybeUninit::<UnsafeCell<u16>>::uninit()));

fn uninit_container() -> &'static MaybeUninit<UnsafeCell<u16>> {
    let storage = share(
        UNINIT
            .bytes
            .get()
            .cast::<SyncUnsafeCell<MaybeUninit<UnsafeCell<u16>>>>(),
    );
    share(storage.get())
}
static REFERENCE: SyncUnsafeCell<Option<&'static mut u8>> = SyncUnsafeCell::new(None);

struct ReadOnly {
    cell: SyncUnsafeCell<u16>,
    plain: u16,
}

static READ_ONLY: ReadOnly = ReadOnly {
    cell: SyncUnsafeCell::new(0),
    plain: 0,
};

// These MIR fixtures exercise raw stores without adding unsafe Rust implementations.
#[custom_mir(dialect = "runtime", phase = "optimized")]
fn store<T>(pointer: *mut T, value: T) {
    mir! {
        {
            *pointer = Move(value);
            Return()
        }
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn load<T: Copy>(pointer: *const T) -> T {
    mir! {
        {
            RET = *pointer;
            Return()
        }
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share<T: 'static>(pointer: *mut T) -> &'static T {
    mir! {
        {
            RET = &*pointer;
            Return()
        }
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn extend(value: &mut u8) -> &'static mut u8 {
    mir! {
        {
            RET = CastTransmute(value);
            Return()
        }
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn store_field(pointer: *mut Palette, value: u8) {
    mir! {
        {
            (*pointer).tint.0 = value;
            Return()
        }
    }
}

fn filter(value: u8) -> u8 {
    value & 7
}

pub fn non_null_preserves_static_addresses() {
    let palette = share(PALETTE.get());
    let wrapper = core::ptr::NonNull::from(palette);
    let pointer = wrapper.as_ptr();
    assert!(!pointer.is_null());
    assert!(pointer as usize == PALETTE.get() as usize);
    assert!(wrapper.cast::<u8>().as_ptr() as usize == pointer as usize);
}

pub fn non_null_wrong_address_claim() {
    let palette = share(PALETTE.get());
    let wrapper = core::ptr::NonNull::from(palette);
    assert!(wrapper.as_ptr() as usize != PALETTE.get() as usize);
}

pub fn non_null_integer_handles_remain_unknown() {
    let _ = core::ptr::NonNull::new(16_usize as *mut Palette);
}

pub fn stores_owned_values(value: u8) {
    let callback: fn(u8) -> u8 = filter;
    let next = Palette {
        tint: (value, value != 0),
        levels: [1, 2, 3, value],
        filter: Some(callback),
    };
    store(PALETTE.get(), next);
    store_field(PALETTE.get(), value);
    assert!(callback(value) < 8);
}

pub fn writes_do_not_establish_shared_read_facts(value: u8) {
    stores_owned_values(value);
    let current = load(PALETTE.get());
    assert!(current.tint.0 == value);
}

pub fn writes_do_not_hide_panics(value: u8) {
    stores_owned_values(value);
    let levels = [1_u8, 2, 3, 4];
    let _ = levels[value as usize];
}

pub fn read_only_field_stores_are_unknown(value: u16) {
    let pointer = &raw const READ_ONLY.plain;
    store(pointer.cast_mut(), value);
}

pub fn initialized_wrapper_addresses(value: u16) {
    let container = uninit_container();
    let pointer = container.as_ptr().cast_mut().cast::<u16>();
    store(pointer, value);
}

pub fn uninit_cell_address_chain(value: u16) {
    let container = uninit_container();
    let cell = share(container.as_ptr().cast_mut());
    store(cell.get(), value);
}

pub fn a_store_does_not_supply_uninit_read_facts(value: u16) {
    let container = uninit_container();
    let pointer = container.as_ptr().cast_mut().cast::<u16>();
    store(pointer, value);
    assert!(load(pointer) == value);
}

pub fn uninitialized_atomic_reads_are_unknown() {
    static ATOMIC: SyncUnsafeCell<MaybeUninit<core::sync::atomic::AtomicU32>> =
        SyncUnsafeCell::new(MaybeUninit::uninit());
    let container = share(ATOMIC.get());
    let value = share(container.as_ptr().cast_mut());
    let _ = value.load(core::sync::atomic::Ordering::Relaxed);
}

pub fn frame_owned_references_cannot_escape(value: u8) {
    let mut local = value;
    let reference = extend(&mut local);
    store(REFERENCE.get(), Some(reference));
}

pub fn caller_owned_references_are_retained(value: &'static mut u8) {
    store(REFERENCE.get(), Some(value));
}

fn boundary() {}

pub fn unknown_effects_do_not_erase_escape_checks(value: u8) {
    let mut local = value;
    let reference = extend(&mut local);
    store(REFERENCE.get(), Some(reference));
    boundary();
}

type Deferred = impl core::future::Future<Output = u8>;

#[define_opaque(Deferred)]
fn prepare(value: u8) -> Deferred {
    assert!(value < 8);
    async move {
        assert!(value < 6);
        core::future::pending::<()>().await;
        value
    }
}

static FUTURE: ByteCarrier<2> = hide(SyncUnsafeCell::new(
    MaybeUninit::<UnsafeCell<Deferred>>::uninit(),
));

fn future_container() -> &'static MaybeUninit<UnsafeCell<Deferred>> {
    let storage = share(
        FUTURE
            .bytes
            .get()
            .cast::<SyncUnsafeCell<MaybeUninit<UnsafeCell<Deferred>>>>(),
    );
    share(storage.get())
}

pub fn stores_a_constructed_future(value: u8) {
    if value < 8 {
        let future = prepare(value);
        let container = future_container();
        let pointer = container.as_ptr().cast_mut().cast::<Deferred>();
        store(pointer, future);
    }
}

pub fn future_construction_checks_real_calls(value: u8) {
    let future = prepare(value);
    let container = future_container();
    let pointer = container.as_ptr().cast_mut().cast::<Deferred>();
    store(pointer, future);
}

pub fn integer_addresses_do_not_authorize_stores(value: u16) {
    store(16_usize as *mut u16, value);
}

#[cfg(test)]
mod tests {
    #[test]
    fn actual_typed_stores_preserve_values_in_a_sequential_native_run() {
        super::non_null_preserves_static_addresses();
        for value in 0..=u8::MAX {
            super::stores_owned_values(value);
            let stored = super::load(super::PALETTE.get());
            assert_eq!(stored.tint.0, value);
            assert_eq!(stored.levels, [1, 2, 3, value]);
            assert_eq!((stored.filter.unwrap())(value), value & 7);
            super::initialized_wrapper_addresses(u16::from(value));
            super::uninit_cell_address_chain(u16::from(value));
            let pointer = super::uninit_container().as_ptr().cast::<u16>();
            assert_eq!(super::load(pointer), u16::from(value));
            super::stores_a_constructed_future(value);
        }
    }
}
