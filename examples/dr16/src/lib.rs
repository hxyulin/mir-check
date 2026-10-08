#![no_std]
#![forbid(unsafe_code)]

use miren_contracts::{ensures, no_panic};

/// One DBUS frame's fields as sent, unchecked. Channels have the centre, 1024, taken off, so each
/// is within ±660 in a good frame: right stick x and y, left stick x and y, then the wheel.
/// Switches are the wire values: 1 up, 2 down, 3 middle.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Raw {
    pub channels: [i16; 5],
    pub right_switch: u8,
    pub left_switch: u8,
    /// x, y and z (the scroll wheel), as the receiver sends them.
    pub mouse: [i16; 3],
    pub mouse_left: bool,
    pub mouse_right: bool,
    /// One bit per key: W, S, A, D, Shift, Ctrl, Q, E, R, F, G, Z, X, C, V, B from bit 0.
    pub keys: u16,
}

impl Raw {
    /// `None` unless `bytes` is exactly 18 bytes. Nothing is checked: `Dr16::from_raw` does.
    #[no_panic]
    #[ensures(match result {
        Some(raw) => bytes.len() == 18 && raw.right_switch <= 3 && raw.left_switch <= 3
            && raw.channels[0] >= -1024 && raw.channels[0] <= 1023
            && raw.channels[1] >= -1024 && raw.channels[1] <= 1023
            && raw.channels[2] >= -1024 && raw.channels[2] <= 1023
            && raw.channels[3] >= -1024 && raw.channels[3] <= 1023
            && raw.channels[4] >= -1024 && raw.channels[4] <= 1023,
        None => bytes.len() != 18,
    })]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let b: &[u8; 18] = bytes.try_into().ok()?;
        let u = |i: usize| u16::from(b[i]);
        let i = |i: usize| i16::from_le_bytes([b[i], b[i + 1]]);
        // Five 11-bit channels, little-endian bit order. The first four are packed back to back
        // from byte 0, so they straddle byte boundaries; the wheel is alone in bytes 16 and 17.
        let raw = [
            u(0) | u(1) << 8,
            u(1) >> 3 | u(2) << 5,
            u(2) >> 6 | u(3) << 2 | u(4) << 10,
            u(4) >> 1 | u(5) << 7,
            u(16) | u(17) << 8,
        ];
        Some(Self {
            channels: raw.map(|raw| (raw & 0x7FF) as i16 - 1024),
            right_switch: b[5] >> 4 & 0x3,
            left_switch: b[5] >> 6 & 0x3,
            mouse: [i(6), i(8), i(10)],
            mouse_left: b[12] != 0,
            mouse_right: b[13] != 0,
            keys: u16::from_le_bytes([b[14], b[15]]),
        })
    }
}

#[cfg(test)]
mod tests;
