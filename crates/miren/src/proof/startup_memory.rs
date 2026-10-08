use super::*;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct StaticAtomicLocation {
    pub(super) definition: DefId,
    pub(super) offset: u64,
    pub(super) bytes: u64,
}

impl StaticAtomicLocation {
    pub(super) fn overlaps(self, other: Self) -> bool {
        self.definition == other.definition
            && self.offset < other.offset + other.bytes
            && other.offset < self.offset + self.bytes
    }
}

#[derive(Clone, Default)]
pub(super) struct Memory {
    allocations: Rc<Vec<Option<Value>>>,
    pub(super) startup_atomics: std::collections::HashMap<StaticAtomicLocation, Value>,
    pub(super) startup_invalidated: bool,
    pub(super) static_uninitialized: Vec<usize>,
}

impl Memory {
    pub(super) fn invalidate_startup(&mut self) {
        self.startup_atomics.clear();
        self.startup_invalidated = true;
    }
}

impl Deref for Memory {
    type Target = Vec<Option<Value>>;

    fn deref(&self) -> &Self::Target {
        &self.allocations
    }
}

impl DerefMut for Memory {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Rc::make_mut(&mut self.allocations)
    }
}

impl From<Vec<Option<Value>>> for Memory {
    fn from(allocations: Vec<Option<Value>>) -> Self {
        Self {
            allocations: Rc::new(allocations),
            ..Self::default()
        }
    }
}

impl FromIterator<Option<Value>> for Memory {
    fn from_iter<T: IntoIterator<Item = Option<Value>>>(values: T) -> Self {
        Vec::from_iter(values).into()
    }
}

impl<'a> IntoIterator for &'a Memory {
    type Item = &'a Option<Value>;
    type IntoIter = std::slice::Iter<'a, Option<Value>>;

    fn into_iter(self) -> Self::IntoIter {
        self.allocations.iter()
    }
}

impl<'a> IntoIterator for &'a mut Memory {
    type Item = &'a mut Option<Value>;
    type IntoIter = std::slice::IterMut<'a, Option<Value>>;

    fn into_iter(self) -> Self::IntoIter {
        Rc::make_mut(&mut self.allocations).iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atomic_location(index: usize) -> StaticAtomicLocation {
        StaticAtomicLocation {
            definition: DefId {
                krate: rustc_span::def_id::LOCAL_CRATE,
                index: rustc_span::def_id::DefIndex::from_usize(index),
            },
            offset: 0,
            bytes: 4,
        }
    }

    #[test]
    fn branch_reads_share_allocations_and_writes_preserve_other_branches() {
        let original: Memory = vec![
            Some(Value::Reference {
                allocation: 1,
                projection: Vec::new(),
                mutable: true,
            }),
            Some(Value::Unit),
        ]
        .into();
        let mut branch = original.clone();
        assert!(Rc::ptr_eq(&original.allocations, &branch.allocations));
        assert_eq!(branch.iter().count(), 2);
        assert!(Rc::ptr_eq(&original.allocations, &branch.allocations));
        branch[1] = None;
        branch.push(Some(Value::Uninitialized));
        assert!(!Rc::ptr_eq(&original.allocations, &branch.allocations));
        assert!(matches!(original[1], Some(Value::Unit)));
        assert!(branch[1].is_none());
        assert_eq!(original.len(), 2);
        assert_eq!(branch.len(), 3);
        for memory in [&original, &branch] {
            assert!(matches!(
                memory[0],
                Some(Value::Reference { allocation: 1, .. })
            ));
        }
    }

    #[test]
    fn mutating_iteration_detaches_shared_allocations() {
        let original: Memory = vec![Some(Value::Uninitialized), None].into();
        let mut branch = original.clone();
        for slot in &mut branch {
            *slot = Some(Value::Unit);
        }
        assert!(matches!(original[0], Some(Value::Uninitialized)));
        assert!(original[1].is_none());
        assert!(branch.iter().all(|slot| matches!(slot, Some(Value::Unit))));
        assert!(!Rc::ptr_eq(&original.allocations, &branch.allocations));
    }

    #[test]
    fn unique_allocation_writes_reuse_the_vector() {
        let mut memory: Memory = vec![Some(Value::Uninitialized)].into();
        let address = Rc::as_ptr(&memory.allocations);
        memory[0] = Some(Value::Unit);
        assert_eq!(address, Rc::as_ptr(&memory.allocations));
        assert!(matches!(memory[0], Some(Value::Unit)));
    }

    #[test]
    fn startup_history_updates_and_invalidation_remain_branch_local() {
        let location = atomic_location(7);
        let other_location = atomic_location(8);
        let mut original: Memory = vec![Some(Value::Unit)].into();
        original.startup_atomics.insert(location, Value::Unit);
        original.static_uninitialized.push(4);
        let mut branch = original.clone();
        branch.static_uninitialized.clear();
        assert_eq!(original.static_uninitialized, vec![4]);
        branch
            .startup_atomics
            .insert(location, Value::Uninitialized);
        branch.startup_atomics.insert(other_location, Value::Unit);
        assert!(matches!(original.startup_atomics[&location], Value::Unit));
        assert!(!original.startup_atomics.contains_key(&other_location));
        branch.invalidate_startup();
        assert!(branch.startup_atomics.is_empty());
        assert!(branch.startup_invalidated);
        assert!(!original.startup_invalidated);
        assert_eq!(original.startup_atomics.len(), 1);
        let invalidated_branch = branch.clone();
        assert!(invalidated_branch.startup_invalidated);
        assert!(invalidated_branch.startup_atomics.is_empty());
        assert!(Rc::ptr_eq(&original.allocations, &branch.allocations));
        assert!(Rc::ptr_eq(
            &branch.allocations,
            &invalidated_branch.allocations
        ));
    }
}
