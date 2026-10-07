use super::*;

/// A compiler-sized access into one allocation, independent of its address representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StorageFootprint {
    offset: u64,
    size: u64,
    alignment: u64,
}

impl StorageFootprint {
    pub(super) fn new(offset: u64, size: u64, alignment: u64) -> Result<Self, String> {
        if !alignment.is_power_of_two() || offset.checked_add(size).is_none() {
            return Err("typed storage footprint has invalid alignment or extent".into());
        }
        Ok(Self {
            offset,
            size,
            alignment,
        })
    }

    pub(super) fn fits(self, allocation_size: u64, allocation_alignment: u64) -> bool {
        self.offset + self.size <= allocation_size
            && allocation_alignment >= self.alignment
            && self.offset.is_multiple_of(self.alignment)
    }
}

impl<'tcx> Engine<'tcx> {
    pub(super) fn typed_storage_footprint(
        &self,
        ty: Ty<'tcx>,
        offset: u64,
    ) -> Result<StorageFootprint, String> {
        let layout = self.static_layout(ty)?;
        StorageFootprint::new(offset, layout.size.bytes(), layout.align.abi.bytes())
    }

    /// Certify a type through actual compiler subobjects, never through equal byte sizes.
    pub(super) fn initialized_storage_prefix(
        &self,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
    ) -> Result<bool, String> {
        self.initialized_storage_prefix_inner(source, target, 0, &mut 0)
    }

    fn initialized_storage_prefix_inner(
        &self,
        source: Ty<'tcx>,
        target: Ty<'tcx>,
        depth: usize,
        values: &mut usize,
    ) -> Result<bool, String> {
        if source == target {
            return Ok(true);
        }
        if depth >= 8 || *values >= 256 {
            return Err("typed subobject certificate exceeds depth or value budget".into());
        }
        *values += 1;
        let layout = self.static_layout(source)?;
        let fields = match source.kind() {
            ty::Adt(def, args) if def.is_struct() => def
                .non_enum_variant()
                .fields
                .iter()
                .map(|field| {
                    self.tcx
                        .try_normalize_erasing_regions(
                            ty::TypingEnv::fully_monomorphized(),
                            field.ty(self.tcx, args),
                        )
                        .map_err(|error| {
                            format!("storage subobject type normalization failed: {error:?}")
                        })
                })
                .collect::<Result<Vec<_>, _>>()?,
            ty::Tuple(fields) => fields.iter().collect(),
            ty::Array(element, _) => {
                let rustc_abi::FieldsShape::Array { count, .. } = layout.fields else {
                    return Err("typed array storage does not have an array layout".into());
                };
                if count == 0 {
                    return Ok(false);
                }
                vec![*element]
            }
            // An enum has no selected runtime variant here. A union, including MaybeUninit,
            // supplies no initialization evidence for any of its member types.
            _ => return Ok(false),
        };
        for (index, field) in fields.into_iter().enumerate() {
            if layout.fields.offset(index).bytes() != 0 {
                continue;
            }
            let footprint = self.typed_storage_footprint(field, 0)?;
            if !footprint.fits(layout.size.bytes(), layout.align.abi.bytes()) {
                return Err("compiler storage subobject exceeds its containing layout".into());
            }
            if self.initialized_storage_prefix_inner(field, target, depth + 1, values)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::StorageFootprint;

    #[test]
    fn typed_footprints_check_extent_and_alignment_without_wrapping() {
        let footprint = StorageFootprint::new(8, 4, 4).unwrap();
        assert!(footprint.fits(12, 8));
        assert!(!footprint.fits(11, 8));
        assert!(!footprint.fits(12, 2));
        assert!(!StorageFootprint::new(2, 4, 4).unwrap().fits(12, 8));
        assert!(StorageFootprint::new(u64::MAX, 1, 1).is_err());
        assert!(StorageFootprint::new(0, 4, 3).is_err());
        assert!(StorageFootprint::new(0, 4, 0).is_err());
    }
}
