#![no_std]
#![forbid(unsafe_code)]

struct Counters {
    left: u16,
    right: u16,
}

fn write_pair(pair: (&mut u16, &mut u16)) {
    *pair.0 = 11;
    *pair.1 = 13;
}

fn move_pair<'a, 'b>(pair: (&'a mut u16, &'b mut u16)) -> (&'a mut u16, &'b mut u16) {
    pair
}

pub fn disjoint_field_locations_survive_moves_and_calls(reverse: bool) {
    let mut counters = Counters { left: 5, right: 7 };
    if reverse {
        write_pair(move_pair((&mut counters.right, &mut counters.left)));
        assert!(counters.left == 13 && counters.right == 11);
    } else {
        write_pair(move_pair((&mut counters.left, &mut counters.right)));
        assert!(counters.left == 11 && counters.right == 13);
    }
}

pub fn reborrows_update_one_allocation_without_changing_its_copy() {
    let mut counters = Counters { left: 5, right: 7 };
    let copied = Counters {
        left: counters.left,
        right: counters.right,
    };
    let outer = &mut counters;
    let inner = &mut outer.left;
    *inner = 17;
    assert!(outer.left == 17 && outer.right == 7);
    assert!(copied.left == 5 && copied.right == 7);
}

pub fn equal_initial_values_do_not_merge_distinct_allocations() {
    let mut first = 5_u16;
    let mut second = 5_u16;
    write_pair((&mut first, &mut second));
    assert!(first == 11 && second == 13);
}

pub fn shared_loads_observe_the_selected_subobject() {
    let counters = Counters { left: 5, right: 7 };
    let pair = (&counters.left, &counters.right);
    assert!(*pair.0 == 5 && *pair.1 == 7);
}

pub fn writing_one_field_does_not_write_its_neighbor() {
    let mut counters = Counters { left: 5, right: 7 };
    *&mut counters.left = 11;
    assert!(counters.right == 11);
}

pub fn a_nonunique_array_write_location_remains_unknown(index: usize) -> [u16; 2] {
    let mut values = [5_u16, 7];
    if index < values.len() {
        values[index] = 9;
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_locations_preserve_aliases_and_distinct_subobjects() {
        disjoint_field_locations_survive_moves_and_calls(false);
        disjoint_field_locations_survive_moves_and_calls(true);
        reborrows_update_one_allocation_without_changing_its_copy();
        equal_initial_values_do_not_merge_distinct_allocations();
        shared_loads_observe_the_selected_subobject();
        a_nonunique_array_write_location_remains_unknown(0);
        a_nonunique_array_write_location_remains_unknown(1);
        a_nonunique_array_write_location_remains_unknown(2);
    }

    #[test]
    #[should_panic]
    fn native_field_writes_leave_the_neighbor_unchanged() {
        writing_one_field_does_not_write_its_neighbor();
    }
}
