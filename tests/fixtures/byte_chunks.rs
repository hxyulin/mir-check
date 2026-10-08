#![no_std]
#![forbid(unsafe_code)]

use miren_contracts::ensures;

fn paint_pixel(pixel: &mut [u8; 3], shade: u8) {
    *pixel = [shade, 17, 23];
}

fn paint_strip(pixels: &mut [u8]) {
    let (pixels, trailer) = pixels.as_chunks_mut::<3>();
    for pixel in pixels {
        paint_pixel(pixel, 41);
    }
    if !trailer.is_empty() {
        trailer[0] = 99;
    }
}

#[ensures(final_palette[0] == shade && final_palette[2] == 23 && final_palette[6] == 99)]
pub fn shade_palette(palette: &mut [u8; 7], shade: u8) {
    let (chunks, trailer) = palette.as_chunks_mut::<3>();
    for chunk in chunks {
        paint_pixel(chunk, shade);
    }
    trailer[0] = 99;
}

#[ensures(final_palette[7] == 99)]
pub fn out_of_bounds_contract(palette: &mut [u8; 7]) {
    palette[6] = 99;
}

pub fn pixel_strip() {
    let mut pixels = [0; 10];
    paint_strip(&mut pixels);
    assert!(pixels[0] == 41 && pixels[4] == 17 && pixels[8] == 23 && pixels[9] == 99);
}

pub fn wrong_pixel_strip() {
    let mut pixels = [0; 10];
    paint_strip(&mut pixels);
    assert!(pixels[9] == 0);
}

pub fn interleaved_regions() {
    let mut pixels = [4_u8; 8];
    let (chunks, trailer) = pixels.as_chunks_mut::<3>();
    chunks[0][1] = 6;
    trailer[1] = 8;
    chunks[1] = [11, 12, 13];
    trailer[0] = 7;
    assert!(pixels[0] == 4 && pixels[1] == 6 && pixels[2] == 4);
    assert!(pixels[3] == 11 && pixels[4] == 12 && pixels[5] == 13);
    assert!(pixels[6] == 7 && pixels[7] == 8);
}

fn borrowed_copy(destination: &mut [u8], source: &[u8]) {
    destination.copy_from_slice(source);
}

pub fn copied_prefix() {
    let mut palette = [1; 7];
    borrowed_copy(&mut palette[..3], &[21, 22, 23]);
    assert!(palette[0] == 21 && palette[2] == 23 && palette[3] == 1 && palette[6] == 1);
}

pub fn copied_chunk() {
    let mut palette = [0_u8; 8];
    let (chunks, remainder) = palette.as_chunks_mut::<3>();
    chunks[1].copy_from_slice(&[31, 32, 33]);
    remainder.copy_from_slice(&[51, 52]);
    assert!(palette[0] == 0 && palette[3] == 31 && palette[5] == 33);
    assert!(palette[6] == 51 && palette[7] == 52);
}

pub fn copy_then_repaint() {
    let mut palette = [2_u8; 8];
    let (chunks, trailer) = palette.as_chunks_mut::<5>();
    trailer.copy_from_slice(&[11, 12, 13]);
    chunks[0][..3].copy_from_slice(trailer);
    trailer[0] = 99;
    assert!(palette[0] == 11 && palette[2] == 13 && palette[3] == 2);
    assert!(palette[5] == 99 && palette[6] == 12 && palette[7] == 13);
}

pub fn mismatched_copy() {
    let mut palette = [0_u8; 5];
    borrowed_copy(&mut palette[..3], &[1, 2]);
}

fn chunks_of_width<const N: usize>(palette: &mut [u8]) {
    let _ = palette.as_chunks_mut::<N>();
}

pub fn zero_chunk_width() {
    let mut palette = [0_u8; 4];
    chunks_of_width::<0>(&mut palette);
}

pub fn oversized_chunk() {
    let mut palette = [0_u8; 4];
    let (chunks, remainder) = palette.as_chunks_mut::<9>();
    assert!(chunks.is_empty() && remainder.len() == 4);
    remainder[2] = 18;
    assert!(palette[2] == 18);
}

pub fn empty_regions() {
    let mut palette = [0_u8; 0];
    let (chunks, remainder) = palette.as_chunks_mut::<3>();
    assert!(chunks.is_empty() && remainder.is_empty());
}

pub fn symbolic_chunks(end: usize) {
    let mut palette = [0_u8; 6];
    if end <= 6 {
        let _ = palette[..end].as_chunks_mut::<3>();
    }
}

pub fn oversized_storage() {
    let mut palette = [0_u8; 129];
    let _ = palette.as_chunks_mut::<3>();
}

pub fn symbolic_prefix(end: usize) {
    let mut palette = [2_u8; 6];
    if end > 0 && end <= 6 {
        let prefix = &mut palette[..end];
        prefix[0] = 77;
        assert!(palette[0] == 77 && palette[5] == 2);
    }
}

fn regions(palette: &mut [u8]) -> (&mut [[u8; 3]], &mut [u8]) {
    palette.as_chunks_mut::<3>()
}

pub fn returned_regions() {
    let mut palette = [0_u8; 7];
    let (chunks, trailer) = regions(&mut palette);
    trailer[0] = 22;
    chunks[1] = [41, 42, 43];
    chunks[0][2] = 23;
    assert!(palette[2] == 23 && palette[3] == 41 && palette[5] == 43 && palette[6] == 22);
}

struct Pretend;

impl Pretend {
    fn as_chunks_mut<const N: usize>(&mut self) {
        assert!(N > 10);
    }
}

pub fn same_named_method() {
    Pretend.as_chunks_mut::<3>();
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
#[test]
fn region_writes_match_the_native_execution() {
    pixel_strip();
    for shade in 0..=u8::MAX {
        let mut palette = [0_u8; 7];
        shade_palette(&mut palette, shade);
        assert!(palette[0] == shade && palette[2] == 23 && palette[6] == 99);
    }
    interleaved_regions();
    copied_prefix();
    copied_chunk();
    copy_then_repaint();
    oversized_chunk();
    empty_regions();
    symbolic_chunks(4);
    symbolic_prefix(4);
    returned_regions();
    out_of_bounds_contract(&mut [0_u8; 7]);
    oversized_storage();
    for action in [
        wrong_pixel_strip,
        mismatched_copy,
        zero_chunk_width,
        same_named_method,
    ] {
        assert!(std::panic::catch_unwind(action).is_err());
    }
}
