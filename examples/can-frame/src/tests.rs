use super::{FdFrame, Frame, classic_payload_round_trip, fd_payload_round_trip};

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
                for (index, byte) in data.iter().enumerate() {
                    assert_eq!(classic_payload_round_trip(id, data, index), *byte);
                }
            }
            let fd = FdFrame::new(id, data);
            let valid = len <= 8 || matches!(len, 12 | 16 | 20 | 24 | 32 | 48 | 64);
            assert_eq!(fd.is_some(), id <= 0x7FF && valid);
            if let Some(frame) = fd {
                assert_eq!(frame.id(), id);
                assert_eq!(frame.data(), data);
                for (index, byte) in data.iter().enumerate() {
                    assert_eq!(fd_payload_round_trip(id, data, index), *byte);
                }
            }
        }
    }
}

#[test]
fn distinct_classic_ids_and_slots_and_tolerant_fd_buses_are_accepted() {
    for frame_id in [0, 1, 0x7FF] {
        for shared_id in [0, 1, 0x7FF] {
            if frame_id == shared_id {
                continue;
            }
            for first_slot in 0..4 {
                for second_slot in 0..4 {
                    if first_slot != second_slot {
                        super::bus::shared_bus(frame_id, shared_id, first_slot, second_slot);
                    }
                }
                for fd_id in [0, 1, 0x7FF] {
                    if fd_id != frame_id && fd_id != shared_id {
                        super::bus::fd_bus(frame_id, shared_id, fd_id, first_slot);
                    }
                }
            }
        }
    }
}

#[test]
#[should_panic(expected = "same slot")]
fn a_duplicate_slot_panics_in_the_original_validator() {
    super::bus::duplicate_slot(0x200, 2);
}

#[test]
#[should_panic(expected = "four slots")]
fn a_slot_past_three_panics_in_the_original_validator() {
    super::bus::invalid_slot(0x200, 4);
}

#[test]
#[should_panic(expected = "cannot tolerate FD")]
fn an_incompatible_fd_bus_panics_in_the_original_validator() {
    super::bus::incompatible_fd(0x200, 0x100);
}

#[test]
#[should_panic(expected = "same CAN ID")]
fn a_duplicate_frame_id_panics_in_the_original_validator() {
    super::bus::duplicate_id(0x200);
}

#[test]
#[should_panic(expected = "11 bits")]
fn an_id_past_eleven_bits_panics_in_the_original_validator() {
    super::bus::invalid_id(0x800);
}
