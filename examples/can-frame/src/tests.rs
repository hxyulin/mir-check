use super::{FdFrame, Frame};

#[test]
fn constructors_preserve_valid_ids_lengths_and_payloads() {
    let input: [u8; 65] = core::array::from_fn(|index| index as u8);
    for id in [0, 0x7FF, 0x800, u16::MAX] {
        for len in 0..=65 {
            let data = &input[..len];
            let classic = Frame::new(id, data);
            assert_eq!(classic.is_some(), id <= 0x7FF && len <= 8);
            if let Some(frame) = classic {
                assert_eq!(frame.id(), id);
                assert_eq!(frame.data(), data);
            }
            let fd = FdFrame::new(id, data);
            let valid = len <= 8 || matches!(len, 12 | 16 | 20 | 24 | 32 | 48 | 64);
            assert_eq!(fd.is_some(), id <= 0x7FF && valid);
            if let Some(frame) = fd {
                assert_eq!(frame.id(), id);
                assert_eq!(frame.data(), data);
            }
        }
    }
}
