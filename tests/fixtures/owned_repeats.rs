#![no_std]
#![forbid(unsafe_code)]

#[derive(Clone, Copy)]
pub struct Pair {
    pub number: u16,
    pub flag: bool,
}

#[derive(Clone, Copy)]
pub enum Choice {
    Empty,
    Value(Pair),
}

pub fn matrix(value: f32) {
    let mut cells = [[value; 6]; 6];
    cells[2][3] = 7.0;
    assert!(cells[2][3] == 7.0);
    assert!(cells[1][3] == value || value.is_nan());
    assert!(cells[2][2] == value || value.is_nan());
}

pub fn diagonal() -> [[f32; 6]; 6] {
    let mut matrix = [[0.1; 6]; 6];
    let mut i = 0;
    while i < 6 {
        matrix[i][i] = 2.0;
        i += 1;
    }
    assert!(matrix[0][0] == 2.0 && matrix[5][5] == 2.0);
    assert!(matrix[0][1] == 0.1 && matrix[5][4] == 0.1);
    matrix
}

pub fn structures(value: u16, flag: bool) {
    let mut pairs = [Pair {
        number: value,
        flag,
    }; 4];
    pairs[0].number = 9;
    assert!(pairs[0].number == 9);
    assert!(pairs[1].number == value);
    assert!(pairs[0].flag == flag && pairs[3].flag == flag);
}

pub fn tuples(value: i16) {
    let mut tuples = [(value, [3_u8; 2]); 3];
    tuples[0].1[0] = 9;
    assert!(tuples[0].1[0] == 9);
    assert!(tuples[1].1[0] == 3);
    assert!(tuples[0].1[1] == 3);
    assert!(tuples[2].0 == value);
}

pub fn variants(value: Choice) {
    let mut copies = [value; 3];
    copies[0] = Choice::Empty;
    assert!(matches!(copies[0], Choice::Empty));
    match (value, copies[1]) {
        (Choice::Empty, Choice::Empty) => (),
        (Choice::Value(a), Choice::Value(b)) => {
            assert!(a.number == b.number && a.flag == b.flag);
        }
        _ => panic!("copy changed the variant"),
    }
}

pub fn empty_tuple() {
    let _ = [(); 4];
    let _ = [((), ()); 4];
}

pub fn bad_copy(value: u16) {
    let mut copies = [Pair {
        number: value,
        flag: true,
    }; 2];
    copies[0].number = 9;
    assert!(copies[1].number == 9);
}

pub fn too_many(value: u16) {
    let _ = [Pair {
        number: value,
        flag: true,
    }; 129];
}

pub fn larger(value: u16, choice: Choice) {
    let mut values = [value; 18];
    values[17] = 9;
    assert!(values[16] == value && values[17] == 9);
    let choices = [choice; 18];
    match (choice, choices[17]) {
        (Choice::Empty, Choice::Empty) => (),
        (Choice::Value(a), Choice::Value(b)) => {
            assert!(a.number == b.number && a.flag == b.flag);
        }
        _ => panic!("copy changed the variant"),
    }
}

pub fn maximum(value: u16) {
    let mut values = [value; 128];
    values[127] = 9;
    assert!(values[126] == value && values[127] == 9);
}

pub fn large_input(values: [u16; 18]) -> u16 {
    values[0]
}

pub fn value_budget_limit(value: u16) {
    let pairs = [(value, value); 85];
    assert!(pairs[84].1 == value);
}

pub fn over_value_budget(value: u16) {
    let _ = [(value, value); 86];
}

pub fn too_large(value: u16) {
    let _ = [[[value; 8]; 8]; 8];
}

pub fn ambiguous(value: u16, index: usize) -> Pair {
    let copies = [Pair {
        number: value,
        flag: true,
    }; 2];
    if index < copies.len() {
        copies[index]
    } else {
        copies[0]
    }
}

pub fn references(value: &core::cell::Cell<u16>) {
    let _ = [(value,); 2];
}

pub fn interior() {
    let cells = [const { core::cell::Cell::new(1_u16) }; 2];
    cells[0].set(9);
    assert!(cells[1].get() == 1);
}
