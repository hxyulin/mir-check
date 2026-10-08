use super::*;
use symbolic::MemoryProjection;

const MAX_TRACKED_ADDRESSES: usize = 512;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct AddressLocation {
    allocation: usize,
    projection: Vec<AddressProjection>,
}

#[derive(Clone, PartialEq, Eq)]
enum AddressProjection {
    Field(usize),
    Variant(usize),
    Index(Term),
}

impl<'tcx> Engine<'tcx> {
    pub(super) fn raw_tracked_pointer(
        &mut self,
        body: &Body<'tcx>,
        state: &mut State,
        place: Place<'tcx>,
        mutable: bool,
    ) -> Result<Value, String> {
        let ty = place.ty(&body.local_decls, self.tcx).ty;
        if !ty.is_sized(self.tcx, ty::TypingEnv::fully_monomorphized()) {
            return Err("tracked raw addresses require a sized pointee".into());
        }
        let reference = if place.projection.is_empty() {
            let local = place.local.as_usize();
            let allocation = if let Some(allocation) = state.addresses[local] {
                self.local(state, local)?;
                allocation
            } else {
                if state.memory.len() >= MAX_TRACKED_ADDRESSES {
                    return Err("memory allocation budget reached".into());
                }
                let value = self.local(state, local)?;
                let allocation = state.memory.len();
                state.memory.push(Some(value));
                state.addresses[local] = Some(allocation);
                allocation
            };
            Value::Reference {
                allocation,
                projection: Vec::new(),
                mutable,
            }
        } else {
            let (last, prefix) = place.projection.split_last().expect("nonempty projection");
            if *last != ProjectionElem::Deref {
                return Err("raw subobject addresses need tracked typed layout offsets".into());
            }
            let base = Place {
                local: place.local,
                projection: self.tcx.mk_place_elems(prefix),
            };
            if !matches!(self.place(state, base)?, Value::Reference { .. }) {
                return Err("raw reborrowing needs a tracked allocation reference".into());
            }
            self.borrow(state, place, mutable)?
        };
        self.tracked_reference_address(reference, ty, state)
    }

    pub(super) fn reference_raw_pointer(
        &mut self,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
        value: Value,
        state: &mut State,
    ) -> Result<Value, String> {
        let ty::Ref(_, pointee, _) = source.kind() else {
            return Err("reference address cast requires a reference source".into());
        };
        if !self.thin_raw_pointer(target)
            || !pointee.is_sized(self.tcx, ty::TypingEnv::fully_monomorphized())
        {
            return Err("reference address casts require sized thin pointers".into());
        }
        self.tracked_reference_address(value, *pointee, state)
    }

    fn tracked_reference_address(
        &mut self,
        reference: Value,
        pointee: Ty<'tcx>,
        state: &mut State,
    ) -> Result<Value, String> {
        self.validate_tracked_value(&reference, state)?;
        let Value::Reference {
            allocation,
            projection,
            ..
        } = &reference
        else {
            return Err("raw address requires allocation provenance, not a value snapshot".into());
        };
        let projection = projection
            .iter()
            .map(|projection| match projection {
                MemoryProjection::Field(index) => Ok(AddressProjection::Field(*index)),
                MemoryProjection::Variant(index) => Ok(AddressProjection::Variant(*index)),
                MemoryProjection::Index(index) => Ok(AddressProjection::Index(index.integer()?.0)),
                MemoryProjection::Slice { .. } | MemoryProjection::Chunks { .. } => {
                    Err("raw slice projection addresses need a typed offset model".to_owned())
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let location = AddressLocation {
            allocation: *allocation,
            projection,
        };
        let bits = u32::from(self.tcx.sess.target.pointer_width);
        let address = if let Some((_, address)) = self
            .tracked_addresses
            .iter()
            .find(|(existing, _)| *existing == location)
        {
            address.clone()
        } else {
            if self.tracked_addresses.len() >= MAX_TRACKED_ADDRESSES {
                return Err("tracked raw address identity budget reached".into());
            }
            let address = self.fresh_abstraction(
                Sort::BitVec(bits),
                "tracked allocation addresses are symbolic; relative offsets are not modeled",
            );
            self.tracked_addresses.push((location, address.clone()));
            address
        };
        let layout = self.static_layout(pointee)?;
        let zero = self.terms.bit_vector(0, bits)?;
        state.conditions.push(symbolic::not(
            &self
                .terms
                .apply(Op::Equal, &[address.clone(), zero.clone()])?,
        ));
        let mask = self
            .terms
            .bit_vector(u128::from(layout.align.abi.bytes() - 1), bits)?;
        let low = self.terms.apply(Op::BvAnd, &[address.clone(), mask])?;
        state
            .conditions
            .push(self.terms.apply(Op::Equal, &[low, zero])?);
        Ok(Value::TrackedPointer {
            reference: Box::new(reference),
            address,
            bits,
        })
    }
}
