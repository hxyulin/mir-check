#![no_std]
#![forbid(unsafe_code)]

pub fn array(values: [f32; 3]) -> f32 {
    let [first, _, last] = values;
    assert!(first == values[0] || first != first);
    assert!(last == values[2] || last != last);
    first + last
}

pub fn slice(bytes: &[u8]) {
    if let [first, .., last] = bytes {
        assert!(*first == bytes[0]);
        assert!(*last == bytes[bytes.len() - 1]);
    }
}

pub fn bad_slice(bytes: &[u8]) {
    if let [first, .., last] = bytes {
        assert!(*first == *last);
    }
}

pub fn unsupported_slice(values: &[f32]) -> f32 {
    if let [first, ..] = values {
        *first
    } else {
        0.0
    }
}
