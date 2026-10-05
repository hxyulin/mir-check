//! CAN frames with a standard ID, the only kind any bus here carries: `Frame`, a classic frame of
//! up to 8 bytes, and `FdFrame`, a CAN FD frame of up to 64 bytes. `motor` and `link` build and
//! read these, not the HAL's frames, so they build and test on the host; `dm-mc02` converts
//! between the two where it opens a bus. An `FdFrame` is its own type, with no conversion from a
//! `Frame`, so what goes out as FD is always chosen, never converted into.
//!
//! Each device also declares the IDs it puts on a bus as `Use`s, so a board can check each bus
//! when it builds: `check` fails the build when two devices would send or answer on one ID, fill
//! one slot of a shared frame, or when an FD frame would share a bus with a device that cannot
//! tolerate one.

#![no_std]
#![forbid(unsafe_code)]

use mir_contracts::{ensures, no_panic, requires};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Frame {
    id: u16,
    len: u8,
    data: [u8; 8],
}

impl Frame {
    /// `None` when `id` does not fit 11 bits or `data` is longer than 8 bytes.
    #[no_panic]
    #[ensures(match result {
        Some(frame) => frame.id == id && frame.id <= 0x7FF
            && frame.len <= 8 && frame.len as usize == data.len(),
        None => id > 0x7FF || data.len() > 8,
    })]
    pub fn new(id: u16, data: &[u8]) -> Option<Self> {
        if id > 0x7FF || data.len() > 8 {
            return None;
        }
        let mut bytes = [0; 8];
        bytes[..data.len()].copy_from_slice(data);
        Some(Self {
            id,
            len: data.len() as u8,
            data: bytes,
        })
    }

    #[no_panic]
    #[ensures(result == self.id)]
    pub const fn id(&self) -> u16 {
        self.id
    }

    #[no_panic]
    #[requires(self.len <= 8)]
    #[ensures(result.len() == self.len as usize)]
    pub fn data(&self) -> &[u8] {
        &self.data[..usize::from(self.len)]
    }
}

/// A CAN FD frame with a standard ID, sent with bit-rate switching: its data phase runs faster
/// than the bus's arbitration. A device that cannot tolerate FD answers each one with an error
/// frame, destroying it, so only a bus whose every device tolerates FD may carry one: its sender
/// declares it as `Use::FdFrame`, and `check` holds the rest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FdFrame {
    id: u16,
    len: u8,
    data: [u8; 64],
}

impl FdFrame {
    /// `None` when `id` does not fit 11 bits or `data` is not a length an FD frame can have: 0 to
    /// 8, 12, 16, 20, 24, 32, 48 or 64 bytes.
    #[no_panic]
    #[ensures(match result {
        Some(frame) => frame.id == id && frame.id <= 0x7FF
            && frame.len <= 64 && frame.len as usize == data.len(),
        None => id > 0x7FF || !(data.len() <= 8 || data.len() == 12 || data.len() == 16 || data.len() == 20 || data.len() == 24 || data.len() == 32 || data.len() == 48 || data.len() == 64),
    })]
    pub fn new(id: u16, data: &[u8]) -> Option<Self> {
        let len = data.len();
        let valid = len <= 8 || matches!(len, 12 | 16 | 20 | 24 | 32 | 48 | 64);
        if id > 0x7FF || !valid {
            return None;
        }
        let mut bytes = [0; 64];
        bytes[..len].copy_from_slice(data);
        Some(Self {
            id,
            len: len as u8,
            data: bytes,
        })
    }

    #[no_panic]
    #[ensures(result == self.id)]
    pub const fn id(&self) -> u16 {
        self.id
    }

    #[no_panic]
    #[requires(self.len <= 64)]
    #[ensures(result.len() == self.len as usize)]
    pub fn data(&self) -> &[u8] {
        &self.data[..usize::from(self.len)]
    }
}

#[cfg(test)]
mod tests;
