use super::Raw;

#[test]
fn only_eighteen_byte_inputs_are_accepted() {
    let bytes: [u8; 40] = core::array::from_fn(|index| index as u8);
    for len in 0..=40 {
        assert_eq!(Raw::parse(&bytes[..len]).is_some(), len == 18);
    }
}

#[test]
fn decoded_fields_agree_with_independent_packed_integer_extraction() {
    let mut bytes = [0; 18];
    for position in 0..18 {
        for value in 0..=u8::MAX {
            bytes[position] = value;
            let raw = Raw::parse(&bytes).unwrap();
            let packed = u64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], 0, 0,
            ]);
            for (channel, actual) in raw.channels[..4].iter().enumerate() {
                assert_eq!(*actual, ((packed >> (11 * channel)) & 0x7FF) as i16 - 1024);
            }
            assert_eq!(
                raw.channels[4],
                (u16::from_le_bytes([bytes[16], bytes[17]]) & 0x7FF) as i16 - 1024
            );
            assert_eq!(raw.right_switch, (bytes[5] >> 4) & 3);
            assert_eq!(raw.left_switch, bytes[5] >> 6);
            assert_eq!(
                raw.mouse,
                [
                    i16::from_le_bytes([bytes[6], bytes[7]]),
                    i16::from_le_bytes([bytes[8], bytes[9]]),
                    i16::from_le_bytes([bytes[10], bytes[11]]),
                ]
            );
            assert_eq!(raw.mouse_left, bytes[12] != 0);
            assert_eq!(raw.mouse_right, bytes[13] != 0);
            assert_eq!(raw.keys, u16::from_le_bytes([bytes[14], bytes[15]]));
        }
        bytes[position] = 0;
    }
}
