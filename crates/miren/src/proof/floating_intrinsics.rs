use super::*;
use miren::smt::Rounding;
use rustc_span::Symbol;

impl<'tcx> Engine<'tcx> {
    pub(super) fn floating_intrinsic(
        &mut self,
        callee: DefId,
        signature: ty::FnSig<'tcx>,
        values: &[Value],
    ) -> Result<Option<Value>, String> {
        let operations = [
            (
                "floorf32",
                "floorf64",
                Op::FpRoundToIntegral,
                Rounding::TowardNegative,
                1,
            ),
            (
                "ceilf32",
                "ceilf64",
                Op::FpRoundToIntegral,
                Rounding::TowardPositive,
                1,
            ),
            (
                "truncf32",
                "truncf64",
                Op::FpRoundToIntegral,
                Rounding::TowardZero,
                1,
            ),
            (
                "roundf32",
                "roundf64",
                Op::FpRoundToIntegral,
                Rounding::NearestAway,
                1,
            ),
            (
                "round_ties_even_f32",
                "round_ties_even_f64",
                Op::FpRoundToIntegral,
                Rounding::NearestEven,
                1,
            ),
            ("sqrtf32", "sqrtf64", Op::FpSqrt, Rounding::NearestEven, 1),
            ("fmaf32", "fmaf64", Op::FpFma, Rounding::NearestEven, 3),
        ];
        let Some(bits) = self.float_type(signature.output()) else {
            return Ok(None);
        };
        for (name32, name64, operation, rounding, arity) in operations {
            if !self.tcx.is_intrinsic(
                callee,
                Symbol::intern(if bits == 32 { name32 } else { name64 }),
            ) {
                continue;
            }
            if signature.inputs().len() != arity
                || !signature
                    .inputs()
                    .iter()
                    .all(|input| *input == signature.output())
                || values.len() != arity
            {
                return Err("floating-point intrinsic signature does not match its model".into());
            }
            let mut operands = vec![self.terms.rounding(rounding)];
            for value in values {
                match value {
                    Value::Float {
                        expression,
                        bits: actual,
                        ..
                    } if *actual == bits => operands.push(expression.clone()),
                    _ => {
                        return Err("floating-point intrinsic needs matching float operands".into());
                    }
                }
            }
            let result = Value::Float {
                expression: self.terms.apply(operation, &operands)?,
                bits,
                raw_bits: None,
            };
            self.record_model(
                callee,
                "IEEE floating-point intrinsic with explicit rounding",
            );
            return Ok(Some(self.materialize_float(result)));
        }
        Ok(None)
    }
}
