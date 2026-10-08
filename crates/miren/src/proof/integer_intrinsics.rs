use super::*;
use rustc_span::Symbol;

#[derive(Clone, Copy)]
enum IntegerIntrinsic {
    Min,
    Max,
    SaturatingAdd,
    SaturatingSub,
    LeadingZeros,
    TrailingZeros,
    SwapBytes,
    ReverseBits,
}

impl IntegerIntrinsic {
    fn name(self) -> &'static str {
        match self {
            Self::Min => "integer_min",
            Self::Max => "integer_max",
            Self::SaturatingAdd => "saturating_add",
            Self::SaturatingSub => "saturating_sub",
            Self::LeadingZeros => "ctlz",
            Self::TrailingZeros => "cttz",
            Self::SwapBytes => "bswap",
            Self::ReverseBits => "bitreverse",
        }
    }

    fn arity(self) -> usize {
        match self {
            Self::Min | Self::Max | Self::SaturatingAdd | Self::SaturatingSub => 2,
            Self::LeadingZeros | Self::TrailingZeros | Self::SwapBytes | Self::ReverseBits => 1,
        }
    }
}

impl<'tcx> Engine<'tcx> {
    pub(super) fn integer_intrinsic(
        &mut self,
        callee: DefId,
        signature: ty::FnSig<'tcx>,
        values: &[Value],
    ) -> Result<Option<Value>, String> {
        let Some(operation) = [
            IntegerIntrinsic::Min,
            IntegerIntrinsic::Max,
            IntegerIntrinsic::SaturatingAdd,
            IntegerIntrinsic::SaturatingSub,
            IntegerIntrinsic::LeadingZeros,
            IntegerIntrinsic::TrailingZeros,
            IntegerIntrinsic::SwapBytes,
            IntegerIntrinsic::ReverseBits,
        ]
        .into_iter()
        .find(|operation| {
            self.tcx
                .is_intrinsic(callee, Symbol::intern(operation.name()))
        }) else {
            return Ok(None);
        };
        if signature.inputs().len() != operation.arity() || values.len() != operation.arity() {
            return Err("integer intrinsic argument count mismatch".to_owned());
        }
        let input_ty = signature.inputs()[0];
        let (bits, signed) = self
            .integer_type(input_ty)
            .ok_or("integer intrinsic needs a primitive integer input")?;
        if signature.inputs().iter().any(|ty| *ty != input_ty) {
            return Err("integer intrinsic input types do not match".to_owned());
        }
        let count = matches!(
            operation,
            IntegerIntrinsic::LeadingZeros | IntegerIntrinsic::TrailingZeros
        );
        if (count && signature.output() != self.tcx.types.u32)
            || (!count && signature.output() != input_ty)
        {
            return Err("integer intrinsic output type mismatch".to_owned());
        }
        let arguments = values
            .iter()
            .map(|value| {
                let (expression, actual_bits, actual_signed) = value.integer()?;
                if (actual_bits, actual_signed) != (bits, signed) {
                    return Err("integer intrinsic modeled argument type mismatch".to_owned());
                }
                Ok(expression)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let left = &arguments[0];
        let expression = match operation {
            IntegerIntrinsic::Min | IntegerIntrinsic::Max => {
                let right = &arguments[1];
                let comparison = self.terms.apply(
                    if signed {
                        Op::BvSignedLt
                    } else {
                        Op::BvUnsignedLt
                    },
                    &[left.clone(), right.clone()],
                )?;
                let (less, greater) = if matches!(operation, IntegerIntrinsic::Min) {
                    (left, right)
                } else {
                    (right, left)
                };
                self.terms
                    .apply(Op::Ite, &[comparison, less.clone(), greater.clone()])?
            }
            IntegerIntrinsic::SaturatingAdd | IntegerIntrinsic::SaturatingSub => {
                saturation(&self.terms, left, &arguments[1], bits, signed, operation)?
            }
            IntegerIntrinsic::LeadingZeros | IntegerIntrinsic::TrailingZeros => {
                let mut result = self.terms.bit_vector(u128::from(bits), 32)?;
                for offset in 0..bits {
                    let bit = if matches!(operation, IntegerIntrinsic::LeadingZeros) {
                        offset
                    } else {
                        bits - offset - 1
                    };
                    let zeros = bits - offset - 1;
                    let extracted = self.terms.apply(
                        Op::Extract {
                            high: bit,
                            low: bit,
                        },
                        std::slice::from_ref(left),
                    )?;
                    let set = self
                        .terms
                        .apply(Op::Equal, &[extracted, self.terms.bit_vector(1, 1)?])?;
                    result = self.terms.apply(
                        Op::Ite,
                        &[set, self.terms.bit_vector(u128::from(zeros), 32)?, result],
                    )?;
                }
                result
            }
            IntegerIntrinsic::SwapBytes | IntegerIntrinsic::ReverseBits => {
                let chunk_bits = if matches!(operation, IntegerIntrinsic::SwapBytes) {
                    8
                } else {
                    1
                };
                let mut result = self.terms.apply(
                    Op::Extract {
                        high: chunk_bits - 1,
                        low: 0,
                    },
                    std::slice::from_ref(left),
                )?;
                for low in (chunk_bits..bits).step_by(chunk_bits as usize) {
                    let extracted = self.terms.apply(
                        Op::Extract {
                            high: low + chunk_bits - 1,
                            low,
                        },
                        std::slice::from_ref(left),
                    )?;
                    result = self.terms.apply(Op::Concat, &[result, extracted])?;
                }
                result
            }
        };
        self.record_model(
            callee,
            "exact primitive integer min/max, saturation or bit transformation",
        );
        Ok(Some(Value::Int {
            expression,
            bits: if count { 32 } else { bits },
            signed: !count && signed,
        }))
    }
}

fn saturation(
    context: &Context,
    left: &Term,
    right: &Term,
    bits: u32,
    signed: bool,
    operation: IntegerIntrinsic,
) -> Result<Term, String> {
    let op = match operation {
        IntegerIntrinsic::SaturatingAdd => Op::BvAdd,
        IntegerIntrinsic::SaturatingSub => Op::BvSub,
        IntegerIntrinsic::Min
        | IntegerIntrinsic::Max
        | IntegerIntrinsic::LeadingZeros
        | IntegerIntrinsic::TrailingZeros
        | IntegerIntrinsic::SwapBytes
        | IntegerIntrinsic::ReverseBits => unreachable!("only saturating arithmetic is dispatched"),
    };
    let extend = if signed {
        Op::SignExtend(1)
    } else {
        Op::ZeroExtend(1)
    };
    let wide = context.apply(
        op,
        &[
            context.apply(extend, std::slice::from_ref(left))?,
            context.apply(extend, std::slice::from_ref(right))?,
        ],
    )?;
    let result = context.apply(
        Op::Extract {
            high: bits - 1,
            low: 0,
        },
        std::slice::from_ref(&wide),
    )?;
    if signed {
        let min = context.bit_vector(1_u128 << (bits - 1), bits)?;
        let max = context.bit_vector((1_u128 << (bits - 1)) - 1, bits)?;
        let above = context.apply(
            Op::BvSignedGt,
            &[
                wide.clone(),
                context.apply(Op::SignExtend(1), std::slice::from_ref(&max))?,
            ],
        )?;
        let high_result = context.apply(Op::Ite, &[above, max, result])?;
        let below = context.apply(
            Op::BvSignedLt,
            &[
                wide,
                context.apply(Op::SignExtend(1), std::slice::from_ref(&min))?,
            ],
        )?;
        context.apply(Op::Ite, &[below, min, high_result])
    } else if matches!(operation, IntegerIntrinsic::SaturatingSub) {
        let underflow = context.apply(Op::BvUnsignedLt, &[left.clone(), right.clone()])?;
        context.apply(Op::Ite, &[underflow, context.bit_vector(0, bits)?, result])
    } else {
        let maximum = u128::MAX >> (128 - bits);
        let max = context.bit_vector(maximum, bits)?;
        let overflow = context.apply(
            Op::BvUnsignedGt,
            &[
                wide,
                context.apply(Op::ZeroExtend(1), std::slice::from_ref(&max))?,
            ],
        )?;
        context.apply(Op::Ite, &[overflow, max, result])
    }
}
