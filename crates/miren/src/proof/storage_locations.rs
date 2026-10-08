use super::*;

/// Identity stays separate from pointer bits and from the slot holding validity evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AllocationIdentity {
    Tracked(usize),
    Static(DefId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccessCapability {
    Shared,
    Writable,
}

/// Access adapter for retained typed storage and address-only certified static storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StorageLocation {
    identity: AllocationIdentity,
    validity_slot: usize,
    capability: AccessCapability,
}

impl StorageLocation {
    pub(super) fn tracked(allocation: usize, writable: bool) -> Self {
        Self {
            identity: AllocationIdentity::Tracked(allocation),
            validity_slot: allocation,
            capability: Self::capability(writable),
        }
    }

    pub(super) fn static_view(definition: DefId, epoch: usize, writable: bool) -> Self {
        Self {
            identity: AllocationIdentity::Static(definition),
            validity_slot: epoch,
            capability: Self::capability(writable),
        }
    }

    fn capability(writable: bool) -> AccessCapability {
        if writable {
            AccessCapability::Writable
        } else {
            AccessCapability::Shared
        }
    }

    pub(super) fn validate(self, memory: &[Option<Value>]) -> Result<(), String> {
        match self.identity {
            AllocationIdentity::Tracked(allocation) => {
                tracked_storage(memory, allocation).map(|_| ())
            }
            AllocationIdentity::Static(_) => validate_static_epoch(memory, self.validity_slot),
        }
    }

    pub(super) fn read(self, memory: &[Option<Value>]) -> Result<&Value, String> {
        match self.identity {
            AllocationIdentity::Tracked(allocation) => tracked_storage(memory, allocation),
            AllocationIdentity::Static(_) => {
                validate_static_epoch(memory, self.validity_slot)?;
                Err("shared static storage has no retained readable payload".into())
            }
        }
    }

    pub(super) fn require_write(self, memory: &[Option<Value>]) -> Result<(), String> {
        self.validate(memory)?;
        match self.capability {
            AccessCapability::Writable => Ok(()),
            AccessCapability::Shared => Err("write through shared storage is unsupported".into()),
        }
    }
}

fn tracked_storage(memory: &[Option<Value>], allocation: usize) -> Result<&Value, String> {
    memory
        .get(allocation)
        .and_then(Option::as_ref)
        .ok_or_else(|| "reference points to dead or uninitialized storage".into())
}

pub(super) fn validate_static_epoch(memory: &[Option<Value>], epoch: usize) -> Result<(), String> {
    if matches!(memory.get(epoch), Some(Some(Value::Unit))) {
        Ok(())
    } else {
        Err("static storage view invalidated by unknown memory effects".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(index: usize) -> DefId {
        DefId {
            krate: rustc_span::def_id::LOCAL_CRATE,
            index: rustc_span::def_id::DefIndex::from_usize(index),
        }
    }

    #[test]
    fn equal_payloads_and_validity_slots_do_not_merge_allocation_identity() {
        let memory = vec![Some(Value::Unit), Some(Value::Unit)];
        let first = StorageLocation::tracked(0, true);
        let second = StorageLocation::tracked(1, true);
        assert_ne!(first, second);
        assert_ne!(first, StorageLocation::static_view(definition(0), 0, true));
        assert_eq!(
            StorageLocation::static_view(definition(0), 0, false).identity,
            StorageLocation::static_view(definition(0), 1, true).identity,
        );
        assert_ne!(
            StorageLocation::static_view(definition(0), 0, false),
            StorageLocation::static_view(definition(1), 0, false),
        );
        first.validate(&memory).unwrap();
        second.validate(&memory).unwrap();
    }

    #[test]
    fn access_checks_keep_capability_and_retirement_independent() {
        let memory = vec![Some(Value::Unit), None];
        let shared = StorageLocation::tracked(0, false);
        assert!(shared.read(&memory).is_ok());
        assert!(shared.require_write(&memory).is_err());
        assert!(
            StorageLocation::tracked(0, true)
                .require_write(&memory)
                .is_ok()
        );
        assert!(
            StorageLocation::tracked(1, true)
                .require_write(&memory)
                .is_err()
        );
        assert!(StorageLocation::tracked(2, true).read(&memory).is_err());
    }

    #[test]
    fn live_static_addresses_do_not_supply_readable_payloads() {
        let mut memory = vec![Some(Value::Unit)];
        let location = StorageLocation::static_view(definition(0), 0, true);
        location.validate(&memory).unwrap();
        location.require_write(&memory).unwrap();
        assert!(location.read(&memory).is_err());
        memory[0] = None;
        assert!(location.validate(&memory).is_err());
        assert!(location.require_write(&memory).is_err());
        memory[0] = Some(Value::Uninitialized);
        assert!(location.validate(&memory).is_err());
    }
}
