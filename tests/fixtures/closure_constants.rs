#![no_std]
#![forbid(unsafe_code)]

struct Labels {
    items: [u16; 3],
    position: usize,
}

impl Iterator for Labels {
    type Item = u16;

    fn next(&mut self) -> Option<u16> {
        if self.position >= self.items.len() {
            return None;
        }
        let value = self.items[self.position];
        self.position += 1;
        Some(value)
    }
}

pub fn default_sum(label: u16) -> u16 {
    let labels = Labels {
        items: [label & 31, 12, 19],
        position: 0,
    };
    let total: u16 = labels.sum();
    assert!(total == (label & 31) + 31);
    total
}

pub fn default_fold(label: u16) -> u16 {
    let labels = Labels {
        items: [label & 31, 12, 19],
        position: 0,
    };
    let total = labels.fold(7_u16, |total, item| total + item);
    assert!(total == (label & 31) + 38);
    total
}

pub fn folded_labels(label: u16) -> u16 {
    let items = [label & 31, 12, 19];
    let total = items.iter().fold(7_u16, |total, item| total + item);
    assert!(total == (label & 31) + 38);
    total
}

pub fn summed_labels(label: u16) -> u16 {
    let items = [label & 31, 12, 19];
    let total: u16 = items.iter().copied().sum();
    assert!(total == (label & 31) + 31);
    total
}

pub fn summed_fractions() -> f32 {
    let portions = [0.125_f32, 0.375];
    let total: f32 = portions.iter().copied().sum();
    assert!(total == 0.5);
    total
}

pub fn folded_empty() -> u16 {
    let items: [u16; 0] = [];
    let total = items.iter().fold(17, |_, _| panic!("empty fold callback"));
    assert!(total == 17);
    total
}

pub fn explicit_constant(value: u16) -> u16 {
    let transform = const { |label: u16| label ^ 42 };
    let output = transform(value);
    assert!(output == value ^ 42);
    output
}

pub fn nested_constant(value: u16) -> u16 {
    let (key, transform) = const { (9_u16, |label: u16| label ^ 42) };
    let output = transform(value ^ key);
    assert!(output == (value ^ 9) ^ 42);
    output
}

pub fn rejected_default_sum() -> u16 {
    Labels {
        items: [65000, 1000, 0],
        position: 0,
    }
    .sum()
}

pub fn rejected_sum() -> u16 {
    [65000_u16, 1000].iter().copied().sum()
}

pub fn rejected_fold() -> u16 {
    let items = [12_u16, 19];
    let total = items.iter().fold(7_u16, |total, item| total + item);
    assert!(total == 39);
    total
}

pub fn rejected_constant_callback() -> u16 {
    let transform = const { |label: u16| label + 1 };
    transform(u16::MAX)
}

pub fn captured_constant(value: u16) -> u16 {
    let transform = const {
        let offset = 13_u16;
        move |label: u16| label + offset
    };
    transform(value & 31)
}

pub fn captured_zero_sized_constant(value: u16) -> u16 {
    let transform = const {
        let marker = ();
        move |label: u16| {
            label + core::mem::size_of_val(&marker) as u16
        }
    };
    transform(value)
}

pub fn mutable_environment() -> u16 {
    let mut seen = 0;
    let items = [12_u16, 19];
    items.iter().fold(7, |total, item| {
        seen += 1;
        total + item
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn labels_match_the_formula_and_failing_callbacks_panic() {
        for label in 0..1024 {
            assert_eq!(super::default_fold(label), (label & 31) + 38);
            assert_eq!(super::default_sum(label), (label & 31) + 31);
            assert_eq!(super::folded_labels(label), (label & 31) + 38);
            assert_eq!(super::summed_labels(label), (label & 31) + 31);
            assert_eq!(super::explicit_constant(label), label ^ 42);
            assert_eq!(super::nested_constant(label), (label ^ 9) ^ 42);
            assert_eq!(super::captured_constant(label), (label & 31) + 13);
            assert_eq!(super::captured_zero_sized_constant(label), label);
        }
        assert_eq!(super::summed_fractions(), 0.5);
        assert_eq!(super::folded_empty(), 17);
        assert_eq!(super::mutable_environment(), 38);
    }

    #[test]
    #[should_panic]
    fn default_sum_overflow_panics() {
        super::rejected_default_sum();
    }

    #[test]
    #[should_panic]
    fn sum_overflow_panics() {
        super::rejected_sum();
    }

    #[test]
    #[should_panic]
    fn a_wrong_fold_postcondition_panics() {
        super::rejected_fold();
    }

    #[test]
    #[should_panic]
    fn a_constant_callback_overflow_panics() {
        super::rejected_constant_callback();
    }
}
