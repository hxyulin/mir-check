#![no_std]
#![forbid(unsafe_code)]

pub fn repeated_numeric_work(mut sample: f32) -> f32 {
    let mut step = 0_u16;
    while step < 768 {
        sample = sample * 1.125 + 0.0625;
        sample = sample * 0.875 - 0.03125;
        sample = sample * 1.0625 + 0.015625;
        sample = sample * 0.9375 - 0.0078125;
        step += 1;
    }
    sample
}

pub fn numeric_work_keeps_its_guard(mut sample: f32, index: u8) -> u8 {
    let values = [11_u8, 23, 47, 89];
    let mut step = 0_u8;
    while step < 128 {
        sample = sample * 1.125 + 0.0625;
        step += 1;
    }
    if index < 4 {
        values[index as usize]
    } else {
        0
    }
}

pub fn numeric_work_with_bad_index(mut sample: f32, index: u8) -> u8 {
    let values = [11_u8, 23, 47, 89];
    let mut step = 0_u8;
    while step < 128 {
        sample = sample * 1.125 + 0.0625;
        step += 1;
    }
    values[index as usize]
}

pub fn unsupported_numeric_work(sample: f32) -> f32 {
    sample % 1.125
}
