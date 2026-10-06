#![no_std]
#![forbid(unsafe_code)]

macro_rules! checked_ticket {
    ($name:ident, $ty:ty) => {
        pub fn $name(issued: $ty, requested: $ty) {
            if let Some(total) = issued.checked_add(requested) {
                assert!(total >= issued && total >= requested);
            }
            if let Some(remaining) = issued.checked_sub(requested) {
                assert!(remaining <= issued);
                assert!(remaining.checked_add(requested).unwrap() == issued);
            }
        }
    };
}

checked_ticket!(ticket_u8, u8);
checked_ticket!(ticket_u16, u16);
checked_ticket!(ticket_u32, u32);
checked_ticket!(ticket_u64, u64);
checked_ticket!(ticket_u128, u128);
checked_ticket!(ticket_usize, usize);

pub fn signed_balance(balance: i16, adjustment: i16) {
    if let Some(updated) = balance.checked_add(adjustment) {
        assert!(i32::from(updated) == i32::from(balance) + i32::from(adjustment));
    }
    if let Some(updated) = balance.checked_sub(adjustment) {
        assert!(i32::from(updated) == i32::from(balance) - i32::from(adjustment));
    }
    if let Some(updated) = balance.checked_mul(adjustment) {
        assert!(i32::from(updated) == i32::from(balance) * i32::from(adjustment));
    }
}

pub fn rejects_overflowing_ticket(issued: u8, requested: u8) {
    assert!(issued.checked_add(requested).is_some());
}

pub fn rejects_missing_ticket(issued: u8, requested: u8) {
    assert!(issued.checked_sub(requested).is_some());
}

pub fn changed_ticket_result(issued: u16, requested: u16) {
    if let Some(total) = issued.checked_add(requested) {
        assert!(total < issued);
    }
}

pub fn flattened_ticket_rows(rows: [[u8; 2]; 3]) {
    assert!(rows.as_flattened().len() == 6);
}

pub fn indirect_ticket_reader(reader: fn(u8) -> u8) {
    assert!(reader(7) == 7);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_ticket_arithmetic_matches_native_integer_boundaries() {
        for issued in 0..=u8::MAX {
            for requested in 0..=u8::MAX {
                ticket_u8(issued, requested);
            }
        }
        for value in [0, 1, u128::MAX / 2, u128::MAX - 1, u128::MAX] {
            for adjustment in [0, 1, 2, u128::MAX / 2, u128::MAX] {
                ticket_u128(value, adjustment);
            }
        }
        for balance in [i16::MIN, -1000, -1, 0, 1, 1000, i16::MAX] {
            for adjustment in [i16::MIN, -1, 0, 1, i16::MAX] {
                signed_balance(balance, adjustment);
            }
        }
    }
}
