#![no_std]
#![forbid(unsafe_code)]

use mir_contracts::ensures;

#[ensures(final_values[0] == 7 && final_values[1] == 7 && final_values[2] == 7)]
pub fn set_all(values: &mut [u16; 3]) {
    for value in values.iter_mut() {
        *value = 7;
    }
}

pub fn borrowed_array() {
    let mut values = [1_u16, 2, 3];
    for value in &mut values {
        *value = 7;
    }
    assert!(values[0] == 7 && values[1] == 7 && values[2] == 7);
}

pub fn disjoint() {
    let mut values = [1_u16, 2, 3];
    let mut iter = values.iter_mut();
    let first = iter.next().unwrap();
    let last = iter.next_back().unwrap();
    *first = 7;
    *last = 9;
    assert!(*first == 7 && *last == 9);
    assert!(*iter.next().unwrap() == 2);
    assert!(iter.next().is_none());
    assert!(values[0] == 7 && values[1] == 2 && values[2] == 9);
}

pub fn diagonal() -> [[f32; 6]; 6] {
    let mut matrix = [[0.25; 6]; 6];
    for (i, row) in matrix.iter_mut().enumerate() {
        row[i] = if i < 3 { 10.0 } else { 3.0 };
    }
    assert!(matrix[0][0] == 10.0 && matrix[2][2] == 10.0);
    assert!(matrix[3][3] == 3.0 && matrix[5][5] == 3.0);
    assert!(matrix[0][1] == 0.25 && matrix[5][4] == 0.25);
    matrix
}

pub fn pairs(values: &mut [(u16, bool); 3]) {
    for (i, pair) in values.iter_mut().enumerate() {
        pair.0 = i as u16;
        pair.1 = true;
    }
    assert!(values[0] == (0, true) && values[1] == (1, true) && values[2] == (2, true));
}

pub fn prefix_bytes() {
    let mut values = [1_u8, 2, 3, 4];
    for byte in values[..2].iter_mut() {
        *byte = 9;
    }
    assert!(values[0] == 9 && values[1] == 9 && values[2] == 3 && values[3] == 4);
    values.copy_from_slice(&[7_u8; 4]);
    assert!(values[0] == 7 && values[1] == 7 && values[2] == 7 && values[3] == 7);
}

pub fn bounded_bytes(values: &mut [u8]) {
    if values.len() > 3 {
        return;
    }
    for byte in values.iter_mut() {
        *byte = 0;
    }
    for byte in values.iter() {
        assert!(*byte == 0);
    }
}

pub fn mutable_predicate() {
    let mut values = [1_u16, 2, 3];
    let mut iter = values.iter_mut();
    assert!(!iter.all(|value| {
        *value = 9;
        false
    }));
    assert!(iter.len() == 2);
    assert!(*iter.next().unwrap() == 2);
    assert!(values[0] == 9 && values[1] == 2 && values[2] == 3);
}

pub fn bad_alias() {
    let mut values = [1_u16, 2];
    let mut iter = values.iter_mut();
    *iter.next().unwrap() = 7;
    *iter.next().unwrap() = 9;
    assert!(values[0] == 9);
}

pub fn bad_overflow(values: &mut [u8; 3]) {
    for value in values.iter_mut() {
        *value += 1;
    }
}

pub fn ambiguous(values: &mut [(u16, bool); 3], index: usize) {
    if index < 3 {
        values.iter_mut().nth(index).unwrap().0 = 7;
    }
}

pub fn escaping(values: &mut [u16; 3]) -> core::slice::IterMut<'_, u16> {
    values.iter_mut()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iterator_writes_agree_with_direct_array_updates() {
        for a in 0..16_u16 {
            for b in 0..16_u16 {
                for c in 0..16_u16 {
                    let mut values = [a, b, c];
                    set_all(&mut values);
                    assert_eq!(values, [7; 3]);
                    let mut tuples = [(a, false), (b, true), (c, false)];
                    pairs(&mut tuples);
                    assert_eq!(tuples, [(0, true), (1, true), (2, true)]);
                }
            }
        }
        disjoint();
        borrowed_array();
        prefix_bytes();
        mutable_predicate();
        let matrix = diagonal();
        for (row, values) in matrix.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                let expected = if row != column {
                    0.25
                } else if row < 3 {
                    10.0
                } else {
                    3.0
                };
                assert_eq!(*value, expected);
            }
        }
        bounded_bytes(&mut []);
        let mut bytes = [255_u8; 3];
        bounded_bytes(&mut bytes);
        assert_eq!(bytes, [0; 3]);
    }
}
