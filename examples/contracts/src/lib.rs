#![no_std]
#![forbid(unsafe_code)]

use miren_contracts::{ensures, no_panic, requires};

#[no_panic]
#[requires(index < bytes.len())]
pub fn read(bytes: &[u8], index: usize) -> u8 {
    bytes[index]
}

#[no_panic]
pub fn guarded_read(bytes: &[u8], index: usize) -> u8 {
    if index < bytes.len() {
        read(bytes, index)
    } else {
        0
    }
}

#[no_panic]
#[requires(value < 15)]
#[ensures(result < 16)]
pub fn bounded_increment(value: u8) -> u8 {
    value + 1
}

#[no_panic]
pub fn guarded_increment(value: u8) -> u8 {
    if value < 15 {
        bounded_increment(value)
    } else {
        0
    }
}

pub struct Header {
    pub index: usize,
    pub enabled: bool,
}

pub struct Packet<'a> {
    pub header: Header,
    pub bytes: &'a [u8],
}

#[no_panic]
#[ensures(match result {
    Some(byte) => packet.header.enabled && packet.header.index < packet.bytes.len(),
    None => !packet.header.enabled || packet.header.index >= packet.bytes.len(),
})]
pub fn guarded_packet_read(packet: &Packet<'_>) -> Option<u8> {
    if packet.header.enabled && packet.header.index < packet.bytes.len() {
        Some(read(packet.bytes, packet.header.index))
    } else {
        None
    }
}
