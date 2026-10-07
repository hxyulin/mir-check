use super::*;
use rustc_abi::Size;
use startup_memory::StaticAtomicLocation;

const MAX_STARTUP_ATOMICS: usize = 128;

#[derive(Clone, Copy)]
pub(super) enum AtomicStorage {
    Tracked(usize),
    Startup(StaticAtomicLocation),
}

impl<'tcx> Engine<'tcx> {
    pub(super) fn startup_atomic_access(
        &mut self,
        receiver: &Value,
        state: &mut State,
        bits: u32,
        signed: bool,
    ) -> Result<(Value, Option<AtomicStorage>), String> {
        let location = self.startup_atomic_location(receiver, state, bits, signed)?;
        if state.memory.startup_invalidated {
            return Ok((self.opaque_atomic_value(bits, signed), None));
        }
        if let Some(Value::Int {
            expression,
            bits: width,
            ..
        }) = state.memory.startup_atomics.get(&location)
        {
            if bits != *width {
                return Err("startup atomic history has an inconsistent width".into());
            }
            return Ok((
                Value::Int {
                    expression: expression.clone(),
                    bits,
                    signed,
                },
                Some(AtomicStorage::Startup(location)),
            ));
        }
        if state
            .memory
            .startup_atomics
            .keys()
            .any(|existing| location.overlaps(*existing))
        {
            state.memory.invalidate_startup();
            self.record_model(
                location.definition,
                "overlapping startup atomic views invalidate exact static history",
            );
            return Ok((self.opaque_atomic_value(bits, signed), None));
        }
        if state.memory.startup_atomics.len() >= MAX_STARTUP_ATOMICS {
            return Err("startup atomic history exceeds the 128-location budget".into());
        }
        let value = self.startup_atomic_initializer(location, bits, signed)?;
        state.memory.startup_atomics.insert(location, value.clone());
        self.record_model(
            location.definition,
            "fresh-startup static integer atomic; initializer and path-local updates",
        );
        Ok((value, Some(AtomicStorage::Startup(location))))
    }

    pub(super) fn opaque_atomic_value(&mut self, bits: u32, signed: bool) -> Value {
        Value::Int {
            expression: self.fresh_abstraction(
                Sort::BitVec(bits),
                "shared atomic reads allow arbitrary old values; exclusivity is unverified",
            ),
            bits,
            signed,
        }
    }

    pub(super) fn store_atomic(&self, state: &mut State, storage: AtomicStorage, value: Value) {
        match storage {
            AtomicStorage::Tracked(allocation) => state.memory[allocation] = Some(value),
            AtomicStorage::Startup(location) => {
                state.memory.startup_atomics.insert(location, value);
            }
        }
    }

    fn startup_atomic_initializer(
        &self,
        location: StaticAtomicLocation,
        bits: u32,
        signed: bool,
    ) -> Result<Value, String> {
        let initializer = self
            .tcx
            .eval_static_initializer(location.definition)
            .map_err(|error| format!("startup static initializer evaluation failed: {error:?}"))?;
        if location.bytes != u64::from(bits / 8)
            || location
                .offset
                .checked_add(location.bytes)
                .is_none_or(|end| end > initializer.inner().size().bytes())
        {
            return Err("startup atomic initializer access exceeds its allocation".into());
        }
        let range = rustc_middle::mir::interpret::AllocRange {
            start: Size::from_bytes(location.offset),
            size: Size::from_bytes(location.bytes),
        };
        let scalar = initializer
            .inner()
            .read_scalar(&self.tcx, range, false)
            .map_err(|error| format!("startup atomic initializer scalar read failed: {error:?}"))?;
        let value = scalar
            .to_bits(range.size)
            .discard_err()
            .ok_or("startup atomic initializer cannot contain pointer provenance")?;
        Ok(symbolic::integer(&self.terms, value, bits, signed))
    }
}
