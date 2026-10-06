#![no_std]
#![forbid(unsafe_code)]

pub fn ordered(values: [u16; 3]) {
    let mut iter = values.iter();
    assert!(iter.len() == 3);
    assert!(iter.size_hint() == (3, Some(3)));
    assert!(*iter.next().unwrap() == values[0]);
    assert!(*iter.next_back().unwrap() == values[2]);
    assert!(*iter.next().unwrap() == values[1]);
    assert!(iter.next().is_none());
    assert!(iter.next_back().is_none());
    assert!(iter.len() == 0);
}

pub fn skips(values: [u8; 4], n: usize) {
    let mut iter = values.iter();
    if n < 4 {
        assert!(*iter.nth(n).unwrap() == values[n]);
        assert!(iter.len() == 3 - n);
    } else {
        assert!(iter.nth(n).is_none());
        assert!(iter.next().is_none());
    }
    let mut reverse = values.iter();
    if n < 4 {
        assert!(*reverse.nth_back(n).unwrap() == values[3 - n]);
    } else {
        assert!(reverse.nth_back(n).is_none());
        assert!(reverse.len() == 0);
    }
}

pub fn clone_cursor(values: [i32; 3]) {
    let mut iter = values.iter();
    let mut copy = iter.clone();
    assert!(*iter.next().unwrap() == values[0]);
    assert!(*iter.next().unwrap() == values[1]);
    assert!(*copy.next().unwrap() == values[0]);
    assert!(iter.count() == 1);
    assert!(copy.count() == 2);
}

pub fn enumerate(values: [u16; 3]) {
    let mut count = 0;
    for (i, value) in values.iter().enumerate() {
        assert!(*value == values[i]);
        count += 1;
    }
    assert!(count == 3);
}

pub fn adapters(values: [u8; 3]) {
    let mut reversed = values.iter().copied().rev();
    assert!(reversed.next().unwrap() == values[2]);
    assert!(reversed.next().unwrap() == values[1]);
    assert!(reversed.next().unwrap() == values[0]);
    assert!(reversed.next().is_none());
}

pub fn bounded_bytes(bytes: &[u8]) {
    if bytes.len() > 3 {
        return;
    }
    let mut count = 0;
    for (i, value) in bytes.iter().enumerate() {
        assert!(*value == bytes[i]);
        count += 1;
    }
    assert!(count == bytes.len());
}

pub fn all_any(values: [u8; 3]) {
    let all = values.iter().all(|value| *value < 10);
    assert!(all == (values[0] < 10 && values[1] < 10 && values[2] < 10));
    let any = values.iter().any(|value| *value == 7);
    assert!(any == (values[0] == 7 || values[1] == 7 || values[2] == 7));
}

pub fn short_circuit(values: [u8; 3]) {
    let mut iter = values.iter();
    let result = iter.all(|value| *value != 0);
    if values[0] == 0 {
        assert!(!result && iter.len() == 2);
    }
    assert!(iter.len() == 0 || !result);
}

pub fn no_callback_after_stopping() {
    assert!(![0, 1].iter().all(|value| {
        if *value == 1 {
            panic!("must stop");
        }
        false
    }));
    assert!([0, 1].iter().any(|value| {
        if *value == 1 {
            panic!("must stop");
        }
        true
    }));
}

pub fn predicate_effects() {
    let count = core::cell::Cell::new(0_u8);
    let values = [1_u8, 2, 3];
    let mut iter = values.iter();
    assert!(iter.any(|value| {
        count.set(count.get() + 1);
        *value == 2
    }));
    assert!(count.get() == 2);
    assert!(iter.len() == 1);
}

pub fn shared_cells() {
    let cells = [core::cell::Cell::new(1_u8), core::cell::Cell::new(2)];
    let mut iter = cells.iter();
    iter.next().unwrap().set(9);
    assert!(iter.next().unwrap().get() == 2);
    assert!(cells[0].get() == 9);
}

pub fn floats(values: [f32; 3]) {
    let finite = values.iter().all(|value| value.is_finite());
    assert!(finite == (values[0].is_finite() && values[1].is_finite() && values[2].is_finite()));
}

pub fn composite(values: [(u16, bool); 3]) {
    let mut iter = values.iter();
    let first = iter.next().unwrap();
    assert!(first.0 == values[0].0 && first.1 == values[0].1);
}

pub fn units() {
    let mut count = 0;
    for _ in [(); 3].iter() {
        count += 1;
    }
    assert!(count == 3);
}

pub fn bad_order(values: [u8; 3]) {
    assert!(*values.iter().next().unwrap() == values[1]);
}

pub fn bad_callback(values: [u8; 3]) {
    let _ = values.iter().all(|value| {
        assert!(*value != 0);
        true
    });
}

pub fn exhausted() {
    let values: [u8; 0] = [];
    assert!(values.iter().next().is_some());
}

pub fn unbounded(bytes: &[u8]) {
    for _ in bytes.iter() {}
}

pub fn unsupported_view(values: [u8; 3]) -> usize {
    values.iter().as_slice().len()
}

pub struct Pretender;
impl Pretender {
    pub fn iter(&self) {
        panic!("user method");
    }
}
pub fn user_method() {
    Pretender.iter();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iterator_results_agree_with_direct_array_formulas() {
        for a in 0..16_u8 {
            for b in 0..16_u8 {
                for c in 0..16_u8 {
                    all_any([a, b, c]);
                    short_circuit([a, b, c]);
                    adapters([a, b, c]);
                    bounded_bytes(&[a, b, c]);
                    ordered([u16::from(a), u16::from(b), u16::from(c)]);
                    for n in [0, 1, 2, 3, 4, usize::MAX] {
                        skips([a, b, c, 255], n);
                    }
                }
            }
        }
        bounded_bytes(&[]);
        bounded_bytes(&[255]);
        no_callback_after_stopping();
        predicate_effects();
        shared_cells();
        units();
    }
}
