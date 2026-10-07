#![no_std]
#![forbid(unsafe_code)]
#![feature(custom_mir, core_intrinsics, sync_unsafe_cell)]

use core::cell::SyncUnsafeCell;
use core::intrinsics::mir::*;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU16, Ordering};

#[repr(C)]
struct Header {
    counter: AtomicU16,
    tag: u16,
}

#[repr(C)]
struct Layer {
    header: Header,
    payload: [u8; 4],
}

#[repr(C)]
struct Envelope {
    layer: Layer,
    trailer: u32,
}

static STORAGE: SyncUnsafeCell<Envelope> = SyncUnsafeCell::new(Envelope {
    layer: Layer {
        header: Header {
            counter: AtomicU16::new(0),
            tag: 2,
        },
        payload: [3; 4],
    },
    trailer: 5,
});

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn share<T: 'static>(pointer: *const T) -> &'static T {
    mir! {
        {
            RET = &*pointer;
            Return()
        }
    }
}

fn header() -> &'static Header {
    let pointer = core::ptr::NonNull::from(share(STORAGE.get())).cast::<Header>();
    share(pointer.as_ptr())
}

pub fn nested_prefix_addresses_preserve_type_and_provenance() {
    let header = header();
    let value = header.counter.load(Ordering::Relaxed);
    assert!(u32::from(value) < 65536);
    assert!(core::ptr::NonNull::from(header).as_ptr() as usize == STORAGE.get() as usize);
}

pub fn recovered_prefixes_do_not_hide_panics(index: u8) {
    nested_prefix_addresses_preserve_type_and_provenance();
    let _ = [1_u8, 2, 3, 4][usize::from(index)];
}

static ARRAY: SyncUnsafeCell<[Header; 2]> = SyncUnsafeCell::new([
    Header {
        counter: AtomicU16::new(0),
        tag: 6,
    },
    Header {
        counter: AtomicU16::new(0),
        tag: 7,
    },
]);

pub fn an_array_prefix_certifies_its_first_element() {
    let first = share(ARRAY.get().cast::<Header>());
    let value = first.counter.load(Ordering::Relaxed);
    assert!(u32::from(value) < 65536);
    assert!(core::ptr::NonNull::from(first).as_ptr() as usize == ARRAY.get() as usize);
}

#[repr(C)]
struct Unrelated {
    counter: AtomicU16,
    tag: u16,
}

pub fn equal_layout_does_not_supply_a_subobject_certificate() {
    let reference = share(STORAGE.get().cast::<Unrelated>());
    let _ = reference.counter.load(Ordering::Relaxed);
}

#[repr(C)]
struct Delayed {
    header: MaybeUninit<Header>,
    trailer: u32,
}

static DELAYED: SyncUnsafeCell<Delayed> = SyncUnsafeCell::new(Delayed {
    header: MaybeUninit::uninit(),
    trailer: 0,
});

pub fn uninitialized_members_do_not_supply_a_subobject_certificate() {
    let reference = share(DELAYED.get().cast::<Header>());
    let _ = reference.counter.load(Ordering::Relaxed);
}

#[repr(C)]
struct Displaced {
    prefix: u16,
    header: Header,
}

static DISPLACED: SyncUnsafeCell<Displaced> = SyncUnsafeCell::new(Displaced {
    prefix: 1,
    header: Header {
        counter: AtomicU16::new(0),
        tag: 2,
    },
});

pub fn nonzero_offset_is_not_a_prefix_certificate() {
    let reference = share(DISPLACED.get().cast::<Header>());
    let _ = reference.counter.load(Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_nested_prefix_references_reach_the_actual_member() {
        super::nested_prefix_addresses_preserve_type_and_provenance();
        super::an_array_prefix_certifies_its_first_element();
        assert_eq!(super::header().tag, 2);
        assert_eq!(
            super::share(super::ARRAY.get().cast::<super::Header>()).tag,
            6
        );
    }
}
