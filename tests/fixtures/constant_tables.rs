#![no_std]
#![forbid(unsafe_code)]

use mir_contracts::requires;

const fn shelf_codes() -> [u16; 128] {
    let mut codes = [0; 128];
    let mut position = 0;
    while position < codes.len() {
        codes[position] = position as u16 * 3 + 7;
        position += 1;
    }
    codes
}

static SHELF_CODES: [u16; 128] = shelf_codes();

const fn inspection_flags() -> [bool; 128] {
    let mut flags = [false; 128];
    let mut position = 0;
    while position < flags.len() {
        flags[position] = position % 3 == 0;
        position += 1;
    }
    flags
}

const INSPECTION_FLAGS: [bool; 128] = inspection_flags();

const fn sample_labels() -> [f32; 32] {
    let mut samples = [0.0; 32];
    let mut position = 0;
    while position < samples.len() {
        samples[position] = f32::from_bits(0x7f80_0001 + position as u32);
        position += 1;
    }
    samples
}

const SAMPLE_LABELS: [f32; 32] = sample_labels();

struct Exhibit {
    year: u16,
    open: bool,
}

const EXHIBITS: [Exhibit; 32] = [const { Exhibit { year: 2024, open: true } }; 32];
const ROWS: [[u16; 4]; 32] = [[7, 11, 13, 17]; 32];

#[requires(index < 128)]
pub fn code_at(index: usize) {
    assert!(SHELF_CODES[index] == index as u16 * 3 + 7);
}

pub fn edge_codes() {
    assert!(SHELF_CODES[0] == 7);
    assert!(SHELF_CODES[64] == 199);
    assert!(SHELF_CODES[127] == 388);
}

pub fn sliced_codes(index: usize) {
    let codes: &[u16] = &SHELF_CODES;
    if index < codes.len() {
        assert!(codes[index] < 400);
    }
}

#[requires(index < 128)]
pub fn inspection_at(index: usize) {
    assert!(INSPECTION_FLAGS[index] == (index % 3 == 0));
}

#[requires(index < 32)]
pub fn sample_encoding(index: usize) {
    let sample = SAMPLE_LABELS[index];
    assert!(sample.to_bits() == 0x7f80_0001 + index as u32);
    let saved = sample;
    assert!(saved.to_bits() == sample.to_bits());
}

pub fn exhibit_at() {
    assert!(EXHIBITS[31].year == 2024 && EXHIBITS[31].open);
}

pub fn row_at() {
    assert!(ROWS[31][3] == 17);
}

#[requires(index < 128)]
pub fn wrong_code(index: usize) {
    assert!(SHELF_CODES[index] < 300);
}

pub fn unbounded_code(index: usize) -> u16 {
    SHELF_CODES[index]
}

#[requires(index < 32)]
pub fn wrong_sample_encoding(index: usize) {
    assert!(SAMPLE_LABELS[index].to_bits() == 0x7fc0_0000);
}

const OVERSIZED: [u16; 129] = [11; 129];
const TOO_MANY_VALUES: [[u16; 4]; 64] = [[1, 2, 3, 4]; 64];

pub fn element_limit() -> u16 {
    OVERSIZED[0]
}

pub fn value_limit() -> u16 {
    TOO_MANY_VALUES[0][0]
}

pub fn ambiguous_exhibit(index: usize) {
    if index < 32 {
        assert!(EXHIBITS[index].year == 2024);
    }
}

pub fn interior_table() -> u16 {
    const CELLS: [core::cell::Cell<u16>; 32] = [const { core::cell::Cell::new(3) }; 32];
    CELLS[0].get()
}

pub fn root_array_limit(values: [u16; 32]) -> u16 {
    values[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluated_tables_preserve_slots_payloads_and_aliases() {
        for index in 0..128 {
            code_at(index);
            sliced_codes(index);
            inspection_at(index);
        }
        for index in 0..32 {
            sample_encoding(index);
        }
        edge_codes();
        exhibit_at();
        row_at();
        assert!(std::panic::catch_unwind(|| wrong_code(127)).is_err());
        assert!(std::panic::catch_unwind(|| unbounded_code(128)).is_err());
        assert!(std::panic::catch_unwind(|| wrong_sample_encoding(7)).is_err());
    }
}

#[cfg(test)]
extern crate std;
