use super::*;
use rustc_abi::FieldIdx;
use rustc_const_eval::const_eval::{CompileTimeInterpCx, mk_eval_cx_for_const_val};
use rustc_const_eval::interpret::{OpTy, Projectable};
use rustc_middle::mir::Const;

const MAX_CONSTANT_VALUES: usize = 256;
const MAX_CONSTANT_ELEMENTS: u64 = 128;
const MAX_CONSTANT_DEPTH: usize = 8;

impl<'tcx> Engine<'tcx> {
    pub(super) fn constant(
        &self,
        id: DefId,
        constant: Const<'tcx>,
        span: Span,
    ) -> Result<Value, String> {
        let typing_env = ty::TypingEnv::post_analysis(self.tcx, id);
        let value = constant
            .eval(self.tcx, typing_env, span)
            .map_err(|error| format!("unsupported MIR constant evaluation: {error:?}"))?;
        if let ty::Ref(_, element, mutability) = constant.ty().kind()
            && !mutability.is_mut()
            && let Some(value) = self.atomic_container(*element, 0)
        {
            return Ok(value);
        }
        if let Some(value) = self.atomic_shape(constant.ty()) {
            return Ok(value);
        }
        let (ecx, operand) =
            mk_eval_cx_for_const_val(self.tcx.at(span), typing_env, value, constant.ty())
                .ok_or("unsupported constant layout")?;
        self.constant_value(&ecx, &operand, 0, &mut 0)
    }

    fn constant_value(
        &self,
        ecx: &CompileTimeInterpCx<'tcx>,
        operand: &OpTy<'tcx>,
        depth: usize,
        values: &mut usize,
    ) -> Result<Value, String> {
        if depth >= MAX_CONSTANT_DEPTH || *values >= MAX_CONSTANT_VALUES {
            return Err("unsupported constant shape exceeds depth or value budget".to_owned());
        }
        *values += 1;
        let ty = match operand.layout.ty.kind() {
            ty::Pat(base, _) => *base,
            _ => operand.layout.ty,
        };
        if let Some(value) = self.atomic_shape(ty) {
            return Ok(value);
        }
        if matches!(ty.kind(), ty::Ref(_, _, mutability) if !mutability.is_mut()) {
            let pointee = ecx
                .deref_pointer(operand)
                .discard_err()
                .ok_or("unsupported constant reference")?;
            if !ecx.type_is_freeze(pointee.layout.ty) {
                return Err("unsupported constant reference to interior mutable storage".to_owned());
            }
            return self.constant_value(ecx, &pointee.into(), depth + 1, values);
        }
        if self.thin_raw_pointer(ty) {
            let address = ecx
                .read_scalar(operand)
                .discard_err()
                .and_then(|scalar| scalar.to_bits(operand.layout.size).discard_err())
                .ok_or("pointer constant has unsupported allocation provenance")?;
            let bits = u32::from(self.tcx.sess.target.pointer_width);
            return Ok(Value::RawPointer {
                address: self.terms.bit_vector(address, bits)?,
                bits,
            });
        }
        if ty.is_bool() || self.float_type(ty).is_some() || self.integer_type(ty).is_some() {
            let scalar = ecx
                .read_scalar(operand)
                .discard_err()
                .ok_or("unsupported constant scalar read; memory may be uninitialized")?;
            if ty.is_bool() {
                return scalar
                    .to_bool()
                    .discard_err()
                    .map(|value| Value::Bool(self.terms.boolean(value)))
                    .ok_or_else(|| "unsupported constant boolean".to_owned());
            }
            let bits = scalar
                .to_bits(operand.layout.size)
                .discard_err()
                .ok_or("unsupported constant scalar bits")?;
            if let Some(width) = self.float_type(ty) {
                return Ok(symbolic::float(&self.terms, bits, width));
            }
            let (width, signed) = self.integer_type(ty).ok_or("unsupported constant type")?;
            return Ok(symbolic::integer(&self.terms, bits, width, signed));
        }
        match ty.kind() {
            ty::Closure(id, args) => {
                if !args.as_closure().upvar_tys().is_empty() {
                    return Err("unsupported constant closure with captures".to_owned());
                }
                if self.tcx.def_kind(*id) != DefKind::Closure
                    || operand.layout.fields.count() != 0
                    || operand.layout.size.bytes() != 0
                {
                    return Err("unsupported constant closure layout".to_owned());
                }
                Ok(Value::Adt {
                    name: self.tcx.def_path_str(*id),
                    variant: 0,
                    is_option: false,
                    discriminant: 0,
                    fields: Vec::new(),
                })
            }
            ty::Adt(def, _) if def.is_struct() || def.is_enum() => {
                let variant = ecx
                    .read_discriminant(operand)
                    .discard_err()
                    .ok_or("unsupported constant discriminant")?;
                let down = ecx
                    .project_downcast(operand, variant)
                    .discard_err()
                    .ok_or("unsupported constant variant projection")?;
                let fields = (0..def.variant(variant).fields.len())
                    .map(|index| {
                        let field = ecx
                            .project_field(&down, FieldIdx::from_usize(index))
                            .discard_err()
                            .ok_or("unsupported constant field projection")?;
                        self.constant_value(ecx, &field, depth + 1, values)
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                self.constructed(ty, variant.as_usize(), fields)
            }
            ty::Tuple(fields) => {
                if fields.is_empty() {
                    return Ok(Value::Unit);
                }
                Ok(Value::Tuple(
                    (0..fields.len())
                        .map(|index| {
                            let field = ecx
                                .project_field(operand, FieldIdx::from_usize(index))
                                .discard_err()
                                .ok_or("unsupported constant tuple projection")?;
                            self.constant_value(ecx, &field, depth + 1, values)
                        })
                        .collect::<Result<_, String>>()?,
                ))
            }
            ty::Array(element, _) | ty::Slice(element) => {
                let count = operand
                    .len(ecx)
                    .discard_err()
                    .ok_or("unsupported constant array length")?;
                let bytes = *element == self.tcx.types.u8;
                if count > MAX_CONSTANT_ELEMENTS {
                    return Err("unsupported constant array exceeds element budget".to_owned());
                }
                let elements = (0..count)
                    .map(|index| {
                        let element = ecx
                            .project_index(operand, index)
                            .discard_err()
                            .ok_or("unsupported constant element projection")?;
                        self.constant_value(ecx, &element, depth + 1, values)
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                if !bytes {
                    return Ok(Value::Elements(elements));
                }
                let width = u32::from(self.tcx.sess.target.pointer_width);
                let mut data = self.terms.apply(
                    Op::ConstArray { index_bits: width },
                    &[self.terms.bit_vector(0, 8)?],
                )?;
                for (index, element) in elements.iter().enumerate() {
                    let (expression, _, _) = element.integer()?;
                    data = self.terms.apply(
                        Op::Store,
                        &[
                            data.clone(),
                            self.terms.bit_vector(index as u128, width)?,
                            expression.clone(),
                        ],
                    )?;
                }
                Ok(Value::Bytes {
                    length: Box::new(symbolic::integer(
                        &self.terms,
                        u128::from(count),
                        width,
                        false,
                    )),
                    data,
                })
            }
            _ => Err(format!("unsupported constant type {ty}")),
        }
    }
}
