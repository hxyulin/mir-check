use super::*;
use std::ops::{Deref, DerefMut};

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
    allocations: Vec<Option<Value>>,
    pub(super) startup_atomics: std::collections::HashMap<StaticAtomicLocation, Value>,
    pub(super) startup_invalidated: bool,
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
        &mut self.allocations
    }
}

impl From<Vec<Option<Value>>> for Memory {
    fn from(allocations: Vec<Option<Value>>) -> Self {
        Self {
            allocations,
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
        self.allocations.iter_mut()
    }
}
