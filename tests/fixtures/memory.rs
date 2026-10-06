#![no_std]
#![forbid(unsafe_code)]

use mir_contracts::{ensures, requires};

pub struct State {
    pub count: u8,
    pub last: Option<u8>,
    pub pair: (u16, bool),
}

#[requires(state.count < 255)]
#[ensures(final_state.count > state.count)]
pub fn increment(state: &mut State) {
    state.count += 1;
}

pub fn bad_increment(state: &mut State) {
    state.count += 1;
}

pub fn set(value: &mut u8, next: u8) {
    *value = next;
}

#[ensures(final_state.count == 7)]
pub fn calls(state: &mut State) {
    set(&mut state.count, 7);
    state.last = Some(state.count);
    state.pair.0 = 300;
    state.pair.1 = true;
    assert!(state.count == 7 && state.pair.0 == 300 && state.pair.1);
    match state.last { Some(value) => assert!(value == 7), None => panic!("lost write") }
}

pub fn bad_calls(state: &mut State) {
    set(&mut state.count, 7);
    assert!(state.count == 8);
}

pub fn branches(state: &mut State, flag: bool) {
    state.count = if flag { 3 } else { 5 };
    assert!((flag && state.count == 3) || (!flag && state.count == 5));
}

pub fn local_reborrows() {
    let mut state = State { count: 1, last: None, pair: (0, false) };
    calls(&mut state);
    let reference = &mut state;
    set(&mut reference.count, 9);
    let reader = &reference.count;
    assert!(*reader == 9);
    assert!(state.count == 9);
}

pub fn read(value: &u8) -> u8 { *value }

pub fn read_after_write(value: &mut u8) {
    set(value, 17);
    assert!(read(value) == 17);
}

pub fn arrays(values: &mut [u16; 3]) {
    values[1] = 999;
    assert!(values[1] == 999);
}

#[requires(index < bytes.len())]
pub fn bytes(bytes: &mut [u8], index: usize, value: u8) {
    bytes[index] = value;
    assert!(bytes[index] == value);
}

pub fn bad_bytes(bytes: &mut [u8], index: usize) {
    bytes[index] = 7;
}

pub fn two_mutable(left: &mut u8, right: &mut u8) { *left = *right; }

pub struct Borrowed<'a> { value: &'a u8 }
pub fn nested_reference(value: &mut Borrowed<'_>) { let _value = *value.value; }

pub fn escaping_reference(value: &mut u8) -> &mut u8 { value }

pub fn ambiguous_array(values: &mut [(u8, u8); 3], index: usize) {
    if index < 3 { values[index].0 = 7; }
}
