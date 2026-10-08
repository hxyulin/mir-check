use super::*;

#[derive(Clone, Copy)]
pub(super) enum AtomicRmw {
    Add,
    Subtract,
    And,
    Or,
    Xor,
    Nand,
    Minimum,
    Maximum,
}

impl AtomicRmw {
    pub(super) fn from_method(name: &str) -> Option<Self> {
        match name {
            "fetch_add" => Some(Self::Add),
            "fetch_sub" => Some(Self::Subtract),
            "fetch_and" => Some(Self::And),
            "fetch_or" => Some(Self::Or),
            "fetch_xor" => Some(Self::Xor),
            "fetch_nand" => Some(Self::Nand),
            "fetch_min" => Some(Self::Minimum),
            "fetch_max" => Some(Self::Maximum),
            _ => None,
        }
    }

    pub(super) fn replacement(
        self,
        context: &Context,
        old: &Value,
        operand: &Value,
        bits: u32,
        signed: bool,
    ) -> Result<Value, String> {
        let (old, old_bits, old_signed) = old.integer()?;
        let (operand, operand_bits, operand_signed) = operand.integer()?;
        if (old_bits, old_signed) != (bits, signed)
            || (operand_bits, operand_signed) != (bits, signed)
            || old.sort() != &Sort::BitVec(bits)
            || operand.sort() != &Sort::BitVec(bits)
        {
            return Err("atomic RMW operands have inconsistent integer types".into());
        }
        let expression = match self {
            Self::Add => context.apply(Op::BvAdd, &[old, operand])?,
            Self::Subtract => context.apply(Op::BvSub, &[old, operand])?,
            Self::And => context.apply(Op::BvAnd, &[old, operand])?,
            Self::Or => context.apply(Op::BvOr, &[old, operand])?,
            Self::Xor => context.apply(Op::BvXor, &[old, operand])?,
            Self::Nand => {
                let conjunction = context.apply(Op::BvAnd, &[old, operand])?;
                context.apply(Op::BvNot, &[conjunction])?
            }
            Self::Minimum | Self::Maximum => {
                let comparison = if signed {
                    Op::BvSignedLt
                } else {
                    Op::BvUnsignedLt
                };
                let less = context.apply(comparison, &[old.clone(), operand.clone()])?;
                let (then_value, else_value) = match self {
                    Self::Minimum => (old, operand),
                    Self::Maximum => (operand, old),
                    Self::Add | Self::Subtract | Self::And | Self::Or | Self::Xor | Self::Nand => {
                        return Err("atomic extremum operation mismatch".into());
                    }
                };
                context.apply(Op::Ite, &[less, then_value, else_value])?
            }
        };
        Ok(Value::Int {
            expression,
            bits,
            signed,
        })
    }
}

impl<'tcx> Engine<'tcx> {
    pub(super) fn validate_atomic_rmw_signature(
        &self,
        receiver_ty: Ty<'tcx>,
        signature: ty::FnSig<'tcx>,
        bits: u32,
        signed: bool,
    ) -> Result<(), String> {
        let ty::Adt(_, arguments) = receiver_ty.kind() else {
            return Err("integer atomic RMW receiver is not an ADT".into());
        };
        let ([reference, operand, _], output) = (signature.inputs(), signature.output()) else {
            return Err("integer atomic RMW signature mismatch".into());
        };
        if *operand != arguments.type_at(0)
            || output != *operand
            || self.integer_type(*operand) != Some((bits, signed))
            || !matches!(reference.kind(), ty::Ref(_, element, mutability)
                if *element == receiver_ty && !mutability.is_mut())
        {
            return Err("integer atomic RMW requires its exact primitive signature".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_modify_write_rejects_mismatched_integer_operands() {
        let context = Context::default();
        let old = symbolic::integer(&context, 1, 8, false);
        let signed = symbolic::integer(&context, 1, 8, true);
        let wide = symbolic::integer(&context, 1, 16, false);
        for operation in [
            AtomicRmw::Add,
            AtomicRmw::Subtract,
            AtomicRmw::And,
            AtomicRmw::Or,
            AtomicRmw::Xor,
            AtomicRmw::Nand,
            AtomicRmw::Minimum,
            AtomicRmw::Maximum,
        ] {
            for operand in [&signed, &wide, &Value::Bool(context.boolean(true))] {
                assert!(
                    operation
                        .replacement(&context, &old, operand, 8, false)
                        .is_err()
                );
            }
            let invalid_term = Value::Int {
                expression: context.bit_vector(1, 16).unwrap(),
                bits: 8,
                signed: false,
            };
            assert!(
                operation
                    .replacement(&context, &old, &invalid_term, 8, false)
                    .is_err()
            );
        }
    }
}
