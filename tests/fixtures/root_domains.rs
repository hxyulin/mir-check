#![no_std]
#![forbid(unsafe_code)]
use core::num::{NonZeroI16, NonZeroU32};

pub struct Envelope<T> {
    pub value: T,
}
type Level1 = Envelope<u16>;
type Level2 = Envelope<Level1>;
type Level3 = Envelope<Level2>;
type Level4 = Envelope<Level3>;
type Level5 = Envelope<Level4>;
type Level6 = Envelope<Level5>;
type Level7 = Envelope<Level6>;
type Level8 = Envelope<Level7>;
type Level9 = Envelope<Level8>;
type Level10 = Envelope<Level9>;

pub fn spectrum(values: [u16; 256], index: usize) -> u16 {
    if index < values.len() {
        values[index]
    } else {
        0
    }
}
pub fn wrong_spectrum(values: [u16; 256], index: usize) -> u16 {
    values[index]
}
pub fn packed_records(values: [[u16; 32]; 8]) -> u16 {
    values[7][31]
}
pub fn deep_record(value: Level10) -> u16 {
    value
        .value
        .value
        .value
        .value
        .value
        .value
        .value
        .value
        .value
        .value
}
pub fn too_many_values(values: [[u16; 64]; 8]) -> u16 {
    values[0][0]
}
pub fn too_many_elements(values: [u16; 257]) -> u16 {
    values[0]
}

pub enum Label {
    Label0,
    Label1,
    Label2,
    Label3,
    Label4,
    Label5,
    Label6,
    Label7,
    Label8,
    Label9,
    Label10,
    Label11,
    Label12,
    Label13,
    Label14,
    Label15,
    Label16,
    Label17,
    Label18,
    Label19,
    Label20,
    Label21,
    Label22,
    Label23,
    Label24,
    Label25,
    Label26,
    Label27,
    Label28,
    Label29,
    Label30,
    Label31,
}
pub enum Archive {
    Entry0,
    Entry1,
    Entry2,
    Entry3,
    Entry4,
    Entry5,
    Entry6,
    Entry7,
    Entry8,
    Entry9,
    Entry10,
    Entry11,
    Entry12,
    Entry13,
    Entry14,
    Entry15,
    Entry16,
    Entry17,
    Entry18,
    Entry19,
    Entry20,
    Entry21,
    Entry22,
    Entry23,
    Entry24,
    Entry25,
    Entry26,
    Entry27,
    Entry28,
    Entry29,
    Entry30,
    Entry31,
    Entry32,
    Entry33,
    Entry34,
    Entry35,
    Entry36,
    Entry37,
    Entry38,
    Entry39,
    Entry40,
    Entry41,
    Entry42,
    Entry43,
    Entry44,
    Entry45,
    Entry46,
    Entry47,
    Entry48,
    Entry49,
    Entry50,
    Entry51,
    Entry52,
    Entry53,
    Entry54,
    Entry55,
    Entry56,
    Entry57,
    Entry58,
    Entry59,
    Entry60,
    Entry61,
    Entry62,
    Entry63,
    Entry64,
}
pub fn too_many_variants(entry: Archive) -> bool {
    matches!(entry, Archive::Entry0)
}
pub fn label_code(label: Label) {
    assert!((label as u8) < 32);
}
pub fn wrong_label(label: Label) {
    assert!((label as u8) < 31);
}
pub fn ticket_divisor(divisor: NonZeroU32, count: u32) -> u32 {
    count / divisor.get()
}
pub fn signed_ticket(ticket: NonZeroI16) {
    assert!(ticket.get() != 0);
}
pub fn wrong_signed_ticket(ticket: NonZeroI16) {
    assert!(ticket.get() > 0);
}
pub fn codepoint(letter: char) {
    let code = letter as u32;
    assert!(code <= 0x10ffff);
    assert!(code < 0xd800 || code > 0xdfff);
}
pub fn wrong_codepoint(letter: char) {
    assert!((letter as u32) < 128);
}
pub struct CustomTicket;
impl CustomTicket {
    pub fn get(self) -> u32 {
        panic!("custom ticket")
    }
}
pub fn custom_get(ticket: CustomTicket) -> u32 {
    ticket.get()
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_records_and_typed_scalar_domains_match_native_execution() {
        let values = core::array::from_fn(|index| (index * 5 + 9) as u16);
        for index in 0..300 {
            assert_eq!(
                spectrum(values, index),
                if index < 256 {
                    (index * 5 + 9) as u16
                } else {
                    0
                }
            );
        }
        assert_eq!(packed_records([[11; 32]; 8]), 11);
        for value in 1..1000 {
            assert_eq!(
                ticket_divisor(NonZeroU32::new(value).unwrap(), 4000),
                4000 / value
            );
        }
        for value in [i16::MIN, -3, -1, 1, 7, i16::MAX] {
            signed_ticket(NonZeroI16::new(value).unwrap());
        }
        for letter in ['\0', 'A', 'é', '\u{d7ff}', '\u{e000}', '\u{10ffff}'] {
            codepoint(letter);
        }
        assert!(std::panic::catch_unwind(|| wrong_spectrum(values, 256)).is_err());
        assert!(
            std::panic::catch_unwind(|| wrong_signed_ticket(NonZeroI16::new(-1).unwrap())).is_err()
        );
        assert!(std::panic::catch_unwind(|| wrong_codepoint('é')).is_err());
        assert!(std::panic::catch_unwind(|| wrong_label(Label::Label31)).is_err());
    }
}
