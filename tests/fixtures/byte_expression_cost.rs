#![no_std]
#![forbid(unsafe_code)]

fn sample_frame(samples: [u16; 6]) -> [u8; 32] {
    let mut frame = [0_u8; 32];
    let (records, trailer) = frame.as_chunks_mut::<5>();
    let mut position = 0;
    while position < samples.len() {
        let scaled = u32::from(samples[position]) * 19 + 7;
        records[position][..4].copy_from_slice(&scaled.to_be_bytes());
        records[position][4] = position as u8;
        position += 1;
    }
    trailer.copy_from_slice(&[61, 83]);
    frame
}

pub fn scaled_sample_storage(samples: [u16; 6]) {
    let frame = sample_frame(samples);
    let first = (u32::from(samples[0]) * 19 + 7).to_be_bytes();
    let last = (u32::from(samples[5]) * 19 + 7).to_be_bytes();
    assert!(frame[0] == first[0] && frame[3] == first[3]);
    assert!(frame[25] == last[0] && frame[28] == last[3]);
    assert!(frame[4] == 0 && frame[29] == 5);
    assert!(frame[30] == 61 && frame[31] == 83);
}

pub fn wrong_sample_storage(samples: [u16; 6]) {
    let frame = sample_frame(samples);
    assert!(frame[29] == 4);
}

pub fn oversized_sample_storage() {
    let mut frame = [0_u8; 129];
    let (records, _) = frame.as_chunks_mut::<5>();
    records[0].copy_from_slice(&[1, 2, 3, 4, 5]);
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
#[test]
fn scaled_records_and_trailer_match_native_byte_encoding() {
    for seed in 0..256_u16 {
        let samples = [seed, seed * 17, 0, 256, u16::MAX - seed, u16::MAX];
        scaled_sample_storage(samples);
        let frame = sample_frame(samples);
        for (position, sample) in samples.into_iter().enumerate() {
            let encoded = (u32::from(sample) * 19 + 7).to_be_bytes();
            assert_eq!(&frame[position * 5..position * 5 + 4], &encoded);
            assert_eq!(frame[position * 5 + 4], position as u8);
        }
        assert_eq!(&frame[30..], &[61, 83]);
    }
    assert!(std::panic::catch_unwind(|| wrong_sample_storage([0; 6])).is_err());
    oversized_sample_storage();
}
