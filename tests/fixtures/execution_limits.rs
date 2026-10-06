#![no_std]
#![forbid(unsafe_code)]

pub fn completed_batches() {
    let mut batches = 0_u16;
    while batches < 256 {
        batches += 1;
    }
    assert!(batches == 256);
}

pub fn wrong_final_batch() {
    let mut batches = 0_u16;
    while batches < 256 {
        batches += 1;
    }
    assert!(batches < 256);
}

pub fn larger_completed_batches() {
    let mut batches = 0_u16;
    while batches < 1024 {
        batches += 1;
    }
    assert!(batches == 1024);
}

pub fn larger_late_failure() {
    let mut batches = 0_u16;
    while batches < 1024 {
        batches += 1;
    }
    assert!(batches < 1024);
}

pub fn unfinished_batches() {
    let mut batches = 0_u16;
    while batches < 4096 {
        batches += 1;
    }
    assert!(batches < 4096);
}

fn visit(level: u8) -> u8 {
    if level == 0 { 0 } else { visit(level - 1) + 1 }
}

pub fn bounded_recursion(level: u8) {
    if level > 10 {
        return;
    }
    assert!(visit(level) == level);
}

pub fn wrong_recursive_result(level: u8) {
    if level > 10 {
        return;
    }
    assert!(visit(level) == level + 1);
}

pub fn unfinished_recursion() {
    unfinished_recursion();
}

fn identity<T: Copy>(value: T) -> T {
    value
}

pub fn different_instances() {
    assert!(identity(11_u8) == 11);
    assert!(identity(513_u16) == 513);
}
