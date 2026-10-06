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
        let left = "integer_operand_0";
        let right = "integer_operand_1";
        let expression = match operation {
            IntegerIntrinsic::Min | IntegerIntrinsic::Max => {
                let comparison = if signed { "bvslt" } else { "bvult" };
                let (less, greater) = match operation {
                    IntegerIntrinsic::Min => (left, right),
                    IntegerIntrinsic::Max => (right, left),
                    IntegerIntrinsic::SaturatingAdd
                    | IntegerIntrinsic::SaturatingSub
                    | IntegerIntrinsic::LeadingZeros
                    | IntegerIntrinsic::TrailingZeros
                    | IntegerIntrinsic::SwapBytes
                    | IntegerIntrinsic::ReverseBits => unreachable!(),
                };
                format!("(ite ({comparison} {left} {right}) {less} {greater})")
            }
            IntegerIntrinsic::SaturatingAdd | IntegerIntrinsic::SaturatingSub => {
                saturation(left, right, bits, signed, operation)
            }
            IntegerIntrinsic::LeadingZeros | IntegerIntrinsic::TrailingZeros => {
                let mut result = format!("(_ bv{bits} 32)");
                for offset in 0..bits {
                    let (bit, zeros) = match operation {
                        IntegerIntrinsic::LeadingZeros => (offset, bits - offset - 1),
                        IntegerIntrinsic::TrailingZeros => (bits - offset - 1, bits - offset - 1),
                        IntegerIntrinsic::Min
                        | IntegerIntrinsic::Max
                        | IntegerIntrinsic::SaturatingAdd
                        | IntegerIntrinsic::SaturatingSub
                        | IntegerIntrinsic::SwapBytes
                        | IntegerIntrinsic::ReverseBits => unreachable!(),
                    };
                    result = format!(
                        "(ite (= ((_ extract {bit} {bit}) {left}) (_ bv1 1)) \
                         (_ bv{zeros} 32) {result})"
                    );
                }
                result
            }
            IntegerIntrinsic::SwapBytes | IntegerIntrinsic::ReverseBits => {
                let chunk_bits = match operation {
                    IntegerIntrinsic::SwapBytes => 8,
                    IntegerIntrinsic::ReverseBits => 1,
                    IntegerIntrinsic::Min
                    | IntegerIntrinsic::Max
                    | IntegerIntrinsic::SaturatingAdd
                    | IntegerIntrinsic::SaturatingSub
                    | IntegerIntrinsic::LeadingZeros
                    | IntegerIntrinsic::TrailingZeros => unreachable!(),
                };
                let mut result = format!("((_ extract {} 0) {left})", chunk_bits - 1);
                for low in (chunk_bits..bits).step_by(chunk_bits as usize) {
                    let high = low + chunk_bits - 1;
                    result = format!("(concat {result} ((_ extract {high} {low}) {left}))");
                }
                result
            }
        };
        self.record_model(
            callee,
            "exact primitive integer min/max, saturation or bit transformation",
        );
        let bindings = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| format!("(integer_operand_{index} {argument})"))
            .collect::<Vec<_>>()
            .join(" ");
        Ok(Some(Value::Int {
            expression: format!("(let ({bindings}) {expression})"),
            bits: if count { 32 } else { bits },
            signed: !count && signed,
        }))
    }
}

fn saturation(
    left: &str,
    right: &str,
    bits: u32,
    signed: bool,
    operation: IntegerIntrinsic,
) -> String {
    let operator = match operation {
        IntegerIntrinsic::SaturatingAdd => "bvadd",
        IntegerIntrinsic::SaturatingSub => "bvsub",
        IntegerIntrinsic::Min
        | IntegerIntrinsic::Max
        | IntegerIntrinsic::LeadingZeros
        | IntegerIntrinsic::TrailingZeros
        | IntegerIntrinsic::SwapBytes
        | IntegerIntrinsic::ReverseBits => unreachable!(),
    };
    let extend = if signed { "sign_extend" } else { "zero_extend" };
    let wide = format!("({operator} ((_ {extend} 1) {left}) ((_ {extend} 1) {right}))");
    let result = format!("((_ extract {} 0) {wide})", bits - 1);
    if signed {
        let min = format!("(_ bv{} {bits})", 1_u128 << (bits - 1));
        let max = format!("(_ bv{} {bits})", (1_u128 << (bits - 1)) - 1);
        format!(
            "(ite (bvslt {wide} ((_ sign_extend 1) {min})) {min} \
                 (ite (bvsgt {wide} ((_ sign_extend 1) {max})) {max} {result}))"
        )
    } else if matches!(operation, IntegerIntrinsic::SaturatingSub) {
        format!("(ite (bvult {left} {right}) (_ bv0 {bits}) {result})")
    } else {
        let maximum = if bits == 128 {
            u128::MAX
        } else {
            (1_u128 << bits) - 1
        };
        let max = format!("(_ bv{maximum} {bits})");
        format!("(ite (bvugt {wide} ((_ zero_extend 1) {max})) {max} {result})")
    }
}
