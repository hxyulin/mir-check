use miren_contracts::{no_panic, requires};

/// One ID a device puts on a bus, for `check`.
#[derive(Clone, Copy)]
pub enum Use {
    /// A classic frame with its own ID: no other device may use the ID, whichever way it goes.
    /// `fd_tolerant`: its device keeps working when an FD frame crosses the bus.
    Frame { id: u16, fd_tolerant: bool },
    /// Slot `slot` of an 8-byte classic frame shared by several devices: whatever sends it fills
    /// every device's slot in one frame, since each device keeps the last frame it saw and a frame
    /// with only one slot filled zeroes the others. No other device may use the slot, nor the ID as
    /// a frame of its own. `fd_tolerant` as for `Frame`.
    Slot {
        id: u16,
        slot: u8,
        fd_tolerant: bool,
    },
    /// An `FdFrame` with its own ID, which every other device on the bus must tolerate.
    FdFrame { id: u16 },
}

impl Use {
    const fn id(&self) -> u16 {
        match *self {
            Use::Frame { id, .. } | Use::Slot { id, .. } | Use::FdFrame { id } => id,
        }
    }

    /// Whether this use's device keeps working when an FD frame crosses the bus.
    const fn fd_tolerant(&self) -> bool {
        match *self {
            Use::Frame { fd_tolerant, .. } | Use::Slot { fd_tolerant, .. } => fd_tolerant,
            Use::FdFrame { .. } => true,
        }
    }
}

/// Panics, which in a `const` fails the build, when two of `uses` clash: one ID used as two
/// frames, or as a frame and a shared frame, or one slot used twice. Also when an ID does not fit
/// 11 bits, a slot is past the frame's 8 bytes of four 2-byte slots, or an FD frame shares the bus
/// with a device that cannot tolerate one.
#[no_panic]
pub const fn check(uses: &[Use]) {
    let mut fd = false;
    let mut intolerant = false;
    let mut i = 0;
    while i < uses.len() {
        let a = uses[i];
        fd |= matches!(a, Use::FdFrame { .. });
        intolerant |= !a.fd_tolerant();
        assert!(a.id() <= 0x7FF, "a CAN ID does not fit 11 bits");
        if let Use::Slot { slot, .. } = a {
            assert!(slot < 4, "a shared frame has four slots, 0 to 3");
        }
        let mut j = i + 1;
        while j < uses.len() {
            let b = uses[j];
            if a.id() == b.id() {
                match (a, b) {
                    (Use::Slot { slot: sa, .. }, Use::Slot { slot: sb, .. }) => {
                        assert!(sa != sb, "two devices on one bus fill the same slot");
                    }
                    _ => panic!("two devices on one bus use the same CAN ID"),
                }
            }
            j += 1;
        }
        i += 1;
    }
    assert!(
        !(fd && intolerant),
        "an FD frame shares a bus with a device that cannot tolerate FD"
    );
}

#[no_panic]
#[requires(frame_id <= 0x7FF && shared_id <= 0x7FF && frame_id != shared_id
    && first_slot < 4 && second_slot < 4 && first_slot != second_slot)]
pub fn shared_bus(frame_id: u16, shared_id: u16, first_slot: u8, second_slot: u8) {
    check(&[
        Use::Frame {
            id: frame_id,
            fd_tolerant: true,
        },
        Use::Slot {
            id: shared_id,
            slot: first_slot,
            fd_tolerant: false,
        },
        Use::Slot {
            id: shared_id,
            slot: second_slot,
            fd_tolerant: false,
        },
    ]);
}

#[no_panic]
#[requires(frame_id <= 0x7FF && shared_id <= 0x7FF && fd_id <= 0x7FF && slot < 4
    && frame_id != shared_id && frame_id != fd_id && shared_id != fd_id)]
pub fn fd_bus(frame_id: u16, shared_id: u16, fd_id: u16, slot: u8) {
    check(&[
        Use::Frame {
            id: frame_id,
            fd_tolerant: true,
        },
        Use::Slot {
            id: shared_id,
            slot,
            fd_tolerant: true,
        },
        Use::FdFrame { id: fd_id },
    ]);
}

#[no_panic]
#[requires(id <= 0x7FF && slot < 4)]
pub fn duplicate_slot(id: u16, slot: u8) {
    check(&[
        Use::Slot {
            id,
            slot,
            fd_tolerant: true,
        },
        Use::Slot {
            id,
            slot,
            fd_tolerant: true,
        },
    ]);
}

#[no_panic]
#[requires(id <= 0x7FF && slot == 4)]
pub fn invalid_slot(id: u16, slot: u8) {
    check(&[Use::Slot {
        id,
        slot,
        fd_tolerant: true,
    }]);
}

#[no_panic]
#[requires(classic_id <= 0x7FF && fd_id <= 0x7FF && classic_id != fd_id)]
pub fn incompatible_fd(classic_id: u16, fd_id: u16) {
    check(&[
        Use::Frame {
            id: classic_id,
            fd_tolerant: false,
        },
        Use::FdFrame { id: fd_id },
    ]);
}

#[no_panic]
#[requires(id <= 0x7FF)]
pub fn duplicate_id(id: u16) {
    check(&[
        Use::Frame {
            id,
            fd_tolerant: true,
        },
        Use::FdFrame { id },
    ]);
}

#[no_panic]
#[requires(id == 0x800)]
pub fn invalid_id(id: u16) {
    check(&[Use::Frame {
        id,
        fd_tolerant: true,
    }]);
}
