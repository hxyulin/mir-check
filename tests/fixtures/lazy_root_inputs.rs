#![no_std]
#![forbid(unsafe_code)]

#[derive(Clone, Copy)]
pub struct Sample {
    pub slot: u16,
    pub enabled: bool,
    pub estimate: f32,
    pub coordinates: (u16, u16),
    pub bytes: [u8; 12],
}

#[derive(Clone, Copy)]
pub struct Archive {
    pub records: [Sample; 128],
    pub active: bool,
}

pub struct DescriptorPair {
    pub left: Archive,
    pub right: Archive,
}

pub fn repeated_shapes_keep_distinct_storage(pair: &mut DescriptorPair) {
    pair.left.records[5].slot = 19;
    pair.right.records[5].slot = 23;
    assert!(pair.left.records[5].slot == 19);
    assert!(pair.right.records[5].slot == 23);
}

pub fn shared_descriptors_do_not_make_values_equal(pair: &DescriptorPair) {
    assert!(pair.left.records[5].slot == pair.right.records[5].slot);
}

pub struct Envelope<T> {
    pub value: T,
}

type Nested1 = Envelope<[u16; 8]>;
type Nested2 = Envelope<Nested1>;
type Nested3 = Envelope<Nested2>;
type Nested4 = Envelope<Nested3>;
type Nested5 = Envelope<Nested4>;
type Nested6 = Envelope<Nested5>;
type Nested7 = Envelope<Nested6>;
type Nested8 = Envelope<Nested7>;
type Nested9 = Envelope<Nested8>;
type Nested10 = Envelope<Nested9>;
type Nested11 = Envelope<Nested10>;
type Nested12 = Envelope<Nested11>;
type Nested13 = Envelope<Nested12>;

pub struct DepthProbe {
    pub shallow: [u16; 8],
    pub nested: Nested13,
}

pub fn cached_shapes_still_respect_nesting(input: &DepthProbe) -> u16 {
    input.shallow[0]
}

impl PartialEq for Sample {
    fn eq(&self, rhs: &Self) -> bool {
        self.slot == rhs.slot
    }
}

pub fn lazy_record_membership(archive: &Archive) {
    includes_seed(&archive.records);
}

fn includes_seed(records: &[Sample]) {
    assert!(records.len() == 128);
    assert!(records.contains(&records[5]));
}

pub fn lazy_scalar_subarray_membership(input: &[[u16; 64]; 128]) {
    assert!(input[7].contains(&input[7][3]));
}

pub fn owned_lazy_records_keep_their_fields(archive: &Archive) {
    for sample in [archive.records[5], archive.records[6]] {
        assert!(sample.slot == archive.records[5].slot || sample.slot == archive.records[6].slot);
    }
}

pub fn owned_lazy_records_do_not_satisfy_a_false_claim(archive: &Archive) {
    for sample in [archive.records[5], archive.records[6]] {
        assert!(sample.slot == 3);
    }
}

pub fn oversized_owned_lazy_records_stay_unknown(archive: Archive) -> usize {
    archive.records.into_iter().count()
}

pub fn a_false_lazy_record_member(archive: &Archive) {
    let mut needle = archive.records[5];
    needle.slot ^= 1;
    assert!(archive.records.contains(&needle));
}

pub fn guarded_field_index(archive: &Archive) -> u8 {
    let index = archive.records[5].slot;
    if index < 8 {
        [11_u8; 8][usize::from(index)]
    } else {
        0
    }
}

pub fn unguarded_field_index(archive: &Archive) -> u8 {
    [11_u8; 8][usize::from(archive.records[5].slot)]
}

#[doc = "<!-- mir-check:v1:requires:archive.records[5].slot == 3 -->"]
#[doc = "<!-- mir-check:v1:ensures:final_archive.records[5].slot == 4 -->"]
#[doc = "<!-- mir-check:v1:ensures:archive.records[5].slot == 3 -->"]
#[doc = "<!-- mir-check:v1:ensures:final_archive.records[6].slot == archive.records[6].slot -->"]
pub fn updates_preserve_entry_snapshots(archive: &mut Archive) {
    archive.records[5].slot += 1;
    archive.records[5].enabled = true;
    assert!(archive.records[5].enabled);
}

#[doc = "<!-- mir-check:v1:ensures:final_archive.records[5].slot == archive.records[5].slot -->"]
pub fn a_false_snapshot_claim(archive: &mut Archive) {
    archive.records[5].slot = 7;
}

pub fn copied_storage_keeps_its_value(archive: &mut Archive) {
    let original = archive.records[5].slot;
    let copy = *archive;
    archive.records[5].slot = 9;
    assert!(copy.records[5].slot == original);
    assert!(archive.records[5].slot == 9);
}

pub fn changing_a_copy_does_not_change_the_input(mut archive: Archive) {
    let original = archive.records[5].slot;
    let mut copy = archive;
    copy.records[5].slot = 13;
    archive.records[6].slot = 17;
    assert!(archive.records[5].slot == original);
    assert!(copy.records[5].slot == 13);
    assert!(archive.records[6].slot == 17);
}

pub fn a_false_copy_claim(archive: Archive) {
    let mut copy = archive;
    copy.records[5].slot = 13;
    assert!(copy.records[5].slot == archive.records[5].slot);
}

pub fn independent_roots_have_independent_values(left: &Archive, right: &Archive) {
    assert!(left.records[5].slot == right.records[5].slot);
}

pub fn independent_mutable_storage(left: &mut Archive, right: &mut Archive) {
    left.records[5].slot = 19;
    right.records[5].slot = 23;
    assert!(left.records[5].slot == 19);
    assert!(right.records[5].slot == 23);
}

pub fn projected_bytes_still_use_array_storage(archive: &mut Archive, index: usize) {
    if index < 12 {
        archive.records[5].bytes[index] = 29;
        assert!(archive.records[5].bytes[index] == 29);
    }
}

pub fn tuple_and_nested_arrays(input: &[([u16; 8], bool); 128]) -> u8 {
    if input[7].1 && input[7].0[3] < 8 {
        [31_u8; 8][usize::from(input[7].0[3])]
    } else {
        0
    }
}

pub fn ambiguous_composite_index(archive: &Archive, index: usize) -> u16 {
    if index < 128 {
        archive.records[index].slot
    } else {
        0
    }
}

pub struct HiddenPointer {
    pub records: [Sample; 128],
    pub pointer: *const u16,
}

pub fn an_unused_pointer_is_still_unsupported(input: &HiddenPointer) -> bool {
    input.records[0].enabled
}

pub struct HiddenReference<'a> {
    pub records: [Sample; 128],
    pub reference: &'a u16,
}

pub fn an_unused_reference_is_still_unsupported(input: &mut HiddenReference<'_>) {
    input.records[0].enabled = false;
}

pub struct Constrained {
    pub records: [core::num::NonZeroU16; 256],
    pub other: [u16; 256],
}

pub fn constrained_inputs_keep_the_eager_boundary(input: &Constrained) -> u16 {
    input.records[0].get()
}

pub struct Interior {
    pub records: [Sample; 128],
    pub cell: core::cell::Cell<u16>,
}

pub fn nested_cell_access_is_still_unsupported(input: &Interior) -> u16 {
    input.cell.get()
}

pub fn lazy_loop_state_is_not_silently_omitted(archive: &Archive) {
    let mut index = 0;
    while index < 128 {
        assert!(archive.records[index].slot < 8);
        index += 1;
    }
}

pub enum Mode {
    Idle,
    Active(u8),
}

pub struct Mixed {
    pub mode: Mode,
    pub label: char,
    pub minimum: core::num::NonZeroU16,
    pub archive: Archive,
}

pub fn eager_invariants_and_lazy_subtrees_coexist(input: &Mixed) -> u8 {
    match input.mode {
        Mode::Active(index) if index < 8 && input.label == 'x' => [37_u8; 8][usize::from(index)],
        Mode::Idle | Mode::Active(_) => guarded_field_index(&input.archive),
    }
}

pub fn eager_nonzero_domain_survives_lazy_siblings(input: &Mixed) {
    match input.mode {
        Mode::Active(index) if index < 8 => {
            assert!(index < 8);
            assert!(input.minimum.get() > 0);
            assert!(input.label <= '\u{10ffff}');
        }
        Mode::Idle | Mode::Active(_) => (),
    }
}

pub enum Choice {
    First(Archive),
    Second(Archive),
}

pub fn eager_tags_select_lazy_variant_payloads(input: &Choice) -> u8 {
    match input {
        Choice::First(archive) | Choice::Second(archive) => guarded_field_index(archive),
    }
}

pub fn a_bad_variant_payload_index(input: &Choice) -> u8 {
    match input {
        Choice::First(archive) | Choice::Second(archive) => unguarded_field_index(archive),
    }
}

pub fn oversized_symbol_reservation(input: &[[[u16; 256]; 256]; 256]) -> u16 {
    input[0][0][0]
}

pub enum Shelves {
    Shelf0(Archive),
    Shelf1(Archive),
    Shelf2(Archive),
    Shelf3(Archive),
    Shelf4(Archive),
    Shelf5(Archive),
    Shelf6(Archive),
    Shelf7(Archive),
    Shelf8(Archive),
    Shelf9(Archive),
    Shelf10(Archive),
    Shelf11(Archive),
    Shelf12(Archive),
    Shelf13(Archive),
    Shelf14(Archive),
    Shelf15(Archive),
    Shelf16(Archive),
    Shelf17(Archive),
    Shelf18(Archive),
    Shelf19(Archive),
    Shelf20(Archive),
    Shelf21(Archive),
    Shelf22(Archive),
    Shelf23(Archive),
    Shelf24(Archive),
    Shelf25(Archive),
    Shelf26(Archive),
    Shelf27(Archive),
    Shelf28(Archive),
    Shelf29(Archive),
    Shelf30(Archive),
    Shelf31(Archive),
    Shelf32(Archive),
    Shelf33(Archive),
    Shelf34(Archive),
    Shelf35(Archive),
    Shelf36(Archive),
    Shelf37(Archive),
    Shelf38(Archive),
    Shelf39(Archive),
}

pub fn shared_payload_shapes_keep_guarded_access_safe(input: &Shelves) -> u8 {
    if let Shelves::Shelf0(archive) = input {
        guarded_field_index(archive)
    } else {
        0
    }
}

pub fn shared_payload_shapes_do_not_prove_an_unchecked_index(input: &Shelves) -> u8 {
    if let Shelves::Shelf39(archive) = input {
        unguarded_field_index(archive)
    } else {
        0
    }
}

pub fn reused_shapes_keep_argument_symbols_independent(left: &Shelves, right: &Shelves) {
    if let (Shelves::Shelf0(left), Shelves::Shelf0(right)) = (left, right) {
        assert!(left.records[5].slot == right.records[5].slot);
    }
}

pub fn reused_shapes_preserve_writes(input: &mut Shelves) {
    if let Shelves::Shelf39(archive) = input {
        archive.records[5].slot = 19;
        assert!(archive.records[5].slot == 19);
    }
}

pub struct SharedDepthProbe {
    pub payloads: Shelves,
    pub nested: Nested13,
}

pub fn shapes_cached_by_previous_fields_still_obey_depth(input: &SharedDepthProbe) {
    let _ = &input.payloads;
}

pub struct SharedUnsupportedProbe {
    pub payloads: Shelves,
    pub unused: *const u16,
}

pub fn cached_shapes_do_not_hide_unsupported_types(input: &SharedUnsupportedProbe) {
    let _ = &input.payloads;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive() -> Archive {
        Archive {
            records: [Sample {
                slot: 3, enabled: false, estimate: 0.5, coordinates: (2, 4), bytes: [0; 12],
            }; 128],
            active: false,
        }
    }

    #[test]
    fn projections_copies_and_disjoint_mutation_match_native_rust() {
        let mut left = archive();
        let mut right = archive();
        lazy_record_membership(&left);
        lazy_scalar_subarray_membership(&[[3; 64]; 128]);
        owned_lazy_records_keep_their_fields(&left);
        assert_eq!(guarded_field_index(&left), 11);
        updates_preserve_entry_snapshots(&mut left);
        copied_storage_keeps_its_value(&mut left);
        changing_a_copy_does_not_change_the_input(left);
        independent_mutable_storage(&mut left, &mut right);
        projected_bytes_still_use_array_storage(&mut left, 6);
        assert_eq!(left.records[5].bytes[6], 29);
        assert_eq!(tuple_and_nested_arrays(&[([3; 8], true); 128]), 31);
        let mut pair = DescriptorPair { left: archive(), right: archive() };
        repeated_shapes_keep_distinct_storage(&mut pair);
        let shelves = Shelves::Shelf0(archive());
        assert_eq!(shared_payload_shapes_keep_guarded_access_safe(&shelves), 11);
        let mut shelves = Shelves::Shelf39(archive());
        reused_shapes_preserve_writes(&mut shelves);
    }

    #[test]
    #[should_panic]
    fn a_false_copy_claim_panics() {
        a_false_copy_claim(archive());
    }

    #[test]
    #[should_panic]
    fn an_unguarded_index_panics() {
        let mut input = archive();
        input.records[5].slot = 8;
        unguarded_field_index(&input);
    }

    #[test]
    #[should_panic]
    fn a_false_lazy_record_member_panics() {
        a_false_lazy_record_member(&archive());
    }

    #[test]
    #[should_panic]
    fn shared_descriptors_do_not_make_values_equal_in_native_rust() {
        let mut pair = DescriptorPair { left: archive(), right: archive() };
        pair.right.records[5].slot = 4;
        shared_descriptors_do_not_make_values_equal(&pair);
    }

    #[test]
    #[should_panic]
    fn owned_lazy_records_do_not_satisfy_a_false_claim_in_native_rust() {
        let mut input = archive();
        input.records[5].slot = 4;
        owned_lazy_records_do_not_satisfy_a_false_claim(&input);
    }

    #[test]
    #[should_panic]
    fn separate_cached_arguments_keep_independent_native_values() {
        let left = Shelves::Shelf0(archive());
        let mut right = archive();
        right.records[5].slot = 4;
        reused_shapes_keep_argument_symbols_independent(&left, &Shelves::Shelf0(right));
    }

    #[test]
    #[should_panic]
    fn cached_payloads_do_not_protect_an_unchecked_native_index() {
        let mut archive = archive();
        archive.records[5].slot = 8;
        shared_payload_shapes_do_not_prove_an_unchecked_index(&Shelves::Shelf39(archive));
    }

}
