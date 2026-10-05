use super::solver::{self, Answer};
use super::symbolic::{self, Value};
use mir_checker::{Obligation, ObligationKind, Proof, ProofStatus};
use rustc_attr_ir::LangItem;
use rustc_hir::def::DefKind;
use rustc_middle::mir::{
    AggregateKind, BinOp, Body, BorrowKind, CastKind, Operand, Place, ProjectionElem, Rvalue,
    START_BLOCK, StatementKind, TerminatorKind, UnOp,
};
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::Span;
use rustc_span::def_id::DefId;
use std::collections::{BTreeSet, VecDeque};

const MAX_STEPS: usize = 256;
const MAX_CALL_DEPTH: usize = 8;
const MAX_QUERY_BYTES: usize = 200_000;

#[derive(Clone)]
struct State {
    locals: Vec<Option<Value>>,
    conditions: Vec<String>,
    visited: BTreeSet<usize>,
}

struct Return {
    value: Value,
    conditions: Vec<String>,
}

struct Engine<'tcx> {
    tcx: TyCtxt<'tcx>,
    declarations: Vec<String>,
    steps: usize,
    proof: Proof,
}

pub fn verify(tcx: TyCtxt<'_>, id: DefId) -> Proof {
    let mut engine = Engine {
        tcx,
        declarations: Vec::new(),
        steps: 0,
        proof: Proof {
            status: ProofStatus::Proved,
            assumptions: Vec::new(),
            obligations: Vec::new(),
        },
    };
    let result = engine.root(id);
    if let Err(reason) = result {
        engine.unknown(id, tcx.def_span(id), reason);
    }
    engine.proof.status = if engine
        .proof
        .obligations
        .iter()
        .any(|o| o.status == ProofStatus::Refuted)
    {
        ProofStatus::Refuted
    } else if engine
        .proof
        .obligations
        .iter()
        .any(|o| o.status == ProofStatus::Unknown)
    {
        ProofStatus::Unknown
    } else {
        ProofStatus::Proved
    };
    engine.proof
}

impl<'tcx> Engine<'tcx> {
    fn root(&mut self, id: DefId) -> Result<(), String> {
        let body = self.tcx.optimized_mir(id);
        let mut conditions = Vec::new();
        let mut arguments = Vec::new();
        for local in body.args_iter() {
            arguments.push(self.argument(id, body.local_decls[local].ty, &mut conditions)?);
        }
        self.execute(id, arguments, conditions, &[])?;
        Ok(())
    }

    fn fresh(&mut self, sort: &str) -> String {
        let symbol = format!("v{}", self.declarations.len());
        self.declarations
            .push(format!("(declare-const {symbol} {sort})"));
        symbol
    }

    fn argument(
        &mut self,
        id: DefId,
        ty: Ty<'tcx>,
        conditions: &mut Vec<String>,
    ) -> Result<Value, String> {
        if let Some((bits, signed)) = self.integer_type(ty) {
            return Ok(Value::Int {
                expression: self.fresh(&format!("(_ BitVec {bits})")),
                bits,
                signed,
            });
        }
        match ty.kind() {
            ty::Bool => Ok(Value::Bool(self.fresh("Bool"))),
            ty::Tuple(fields) if fields.is_empty() => Ok(Value::Unit),
            ty::Ref(_, element, mutability) if !mutability.is_mut() => {
                self.byte_input(id, *element, conditions)
            }
            ty::Array(..) => self.byte_input(id, ty, conditions),
            _ => Err(format!("unsupported argument type {ty:?}")),
        }
    }

    fn byte_input(
        &mut self,
        id: DefId,
        ty: Ty<'tcx>,
        conditions: &mut Vec<String>,
    ) -> Result<Value, String> {
        let bits = u32::from(self.tcx.sess.target.pointer_width);
        let length = match ty.kind() {
            ty::Slice(element) if *element == self.tcx.types.u8 => {
                let expression = self.fresh(&format!("(_ BitVec {bits})"));
                // Valid non-ZST byte slices occupy at most isize::MAX bytes.
                let max = (1_u128 << (bits - 1)) - 1;
                conditions.push(format!("(bvule {expression} (_ bv{max} {bits}))"));
                Value::Int {
                    expression,
                    bits,
                    signed: false,
                }
            }
            ty::Array(element, length) if *element == self.tcx.types.u8 => {
                let length = length.try_to_target_usize(self.tcx).ok_or_else(|| {
                    format!("unevaluated array length in {}", self.tcx.def_path_str(id))
                })?;
                symbolic::integer(u128::from(length), bits, false)
            }
            _ => {
                return Err(format!(
                    "only read-only byte slices and byte arrays are modeled: {ty:?}"
                ));
            }
        };
        let data = self.fresh(&format!("(Array (_ BitVec {bits}) (_ BitVec 8))"));
        Ok(Value::Bytes {
            length: Box::new(length),
            data,
        })
    }

    fn integer_type(&self, ty: Ty<'tcx>) -> Option<(u32, bool)> {
        let pointer_bits = u64::from(self.tcx.sess.target.pointer_width);
        match ty.kind() {
            ty::Int(kind) => Some((kind.bit_width().unwrap_or(pointer_bits) as u32, true)),
            ty::Uint(kind) => Some((kind.bit_width().unwrap_or(pointer_bits) as u32, false)),
            _ => None,
        }
    }

    fn query(&self, conditions: &[String], failure: &str) -> Result<String, String> {
        let mut query = String::from(
            "(set-logic QF_AUFBV)\n(set-option :timeout 5000)\n\
             (set-option :pp.bv-literals false)\n",
        );
        for declaration in &self.declarations {
            query.push_str(declaration);
            query.push('\n');
        }
        for condition in conditions
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(failure))
        {
            query.push_str(&format!("(assert {condition})\n"));
        }
        query.push_str("(check-sat)\n");
        if query.len() > MAX_QUERY_BYTES {
            return Err("symbolic query size limit reached".to_owned());
        }
        Ok(query)
    }

    fn feasible(&self, conditions: &[String]) -> Result<bool, String> {
        match solver::check(&self.query(conditions, "true")?) {
            Answer::Unsat => Ok(false),
            Answer::Sat(_) => Ok(true),
            Answer::Unknown(reason) => Err(reason),
        }
    }

    fn require(
        &mut self,
        id: DefId,
        span: Span,
        conditions: &[String],
        safe: &str,
        kind: ObligationKind,
        detail: String,
    ) -> Result<(), String> {
        let query = self.query(conditions, &symbolic::not(safe))?;
        let (status, model, detail) = match solver::check(&query) {
            Answer::Unsat => (ProofStatus::Proved, None, detail),
            Answer::Sat(model) => (ProofStatus::Refuted, Some(model), detail),
            Answer::Unknown(reason) => (ProofStatus::Unknown, None, format!("{detail}: {reason}")),
        };
        self.proof.obligations.push(Obligation {
            function: self.tcx.def_path_str(id),
            source: super::source(self.tcx, span.source_callsite()),
            kind,
            detail,
            status,
            query: Some(query),
            model,
        });
        if status == ProofStatus::Unknown {
            return Err("solver could not discharge an obligation".to_owned());
        }
        Ok(())
    }

    fn unknown(&mut self, id: DefId, span: Span, detail: String) {
        self.proof.obligations.push(Obligation {
            function: self.tcx.def_path_str(id),
            source: super::source(self.tcx, span.source_callsite()),
            kind: ObligationKind::Unsupported,
            detail,
            status: ProofStatus::Unknown,
            query: None,
            model: None,
        });
    }

    fn execute(
        &mut self,
        id: DefId,
        arguments: Vec<Value>,
        conditions: Vec<String>,
        stack: &[DefId],
    ) -> Result<Vec<Return>, String> {
        if stack.contains(&id) || stack.len() >= MAX_CALL_DEPTH {
            return Err("recursion or call-depth limit requires an invariant".to_owned());
        }
        let mut stack = stack.to_vec();
        stack.push(id);
        let body = self.tcx.optimized_mir(id);
        if arguments.len() != body.arg_count {
            return Err("call arguments do not match the MIR body".to_owned());
        }
        let mut state = State {
            locals: vec![None; body.local_decls.len()],
            conditions,
            visited: BTreeSet::new(),
        };
        for (local, argument) in body.args_iter().zip(arguments) {
            state.locals[local.as_usize()] = Some(argument);
        }
        let mut queue = VecDeque::from([(START_BLOCK, state)]);
        let mut returns = Vec::new();
        while let Some((block, mut state)) = queue.pop_front() {
            self.steps += 1;
            if self.steps > MAX_STEPS {
                return Err("symbolic execution step limit reached".to_owned());
            }
            if !self.feasible(&state.conditions)? {
                continue;
            }
            if !state.visited.insert(block.as_usize()) {
                return Err("a reachable loop requires an invariant".to_owned());
            }
            for statement in &body.basic_blocks[block].statements {
                self.statement(id, body, &mut state, &statement.kind)
                    .map_err(|reason| {
                        format!(
                            "{} bb{}: {reason}",
                            self.tcx.def_path_str(id),
                            block.as_usize()
                        )
                    })?;
            }
            let terminator = body.basic_blocks[block].terminator();
            match &terminator.kind {
                TerminatorKind::Goto { target } => queue.push_back((*target, state)),
                TerminatorKind::SwitchInt { discr, targets } => {
                    let value = self.operand(id, body, &state, discr)?;
                    let mut excluded = Vec::new();
                    for (number, target) in targets.iter() {
                        let condition = match &value {
                            Value::Bool(expression) => match number {
                                0 => symbolic::not(expression),
                                1 => expression.clone(),
                                _ => return Err("invalid boolean switch value".to_owned()),
                            },
                            Value::Int { bits, signed, .. } => symbolic::binary(
                                "eq",
                                value.clone(),
                                symbolic::integer(number, *bits, *signed),
                            )?
                            .boolean()?,
                            _ => return Err("unsupported switch discriminant".to_owned()),
                        };
                        let mut branch = state.clone();
                        branch.conditions.push(condition.clone());
                        queue.push_back((target, branch));
                        excluded.push(symbolic::not(&condition));
                    }
                    state.conditions.extend(excluded);
                    queue.push_back((targets.otherwise(), state));
                }
                TerminatorKind::Assert {
                    cond,
                    expected,
                    msg,
                    target,
                    ..
                } => {
                    if !msg.is_optional_overflow_check() || self.tcx.sess.overflow_checks() {
                        let condition = self.operand(id, body, &state, cond)?.boolean()?;
                        let safe = if *expected {
                            condition
                        } else {
                            symbolic::not(&condition)
                        };
                        self.require(
                            id,
                            terminator.source_info.span,
                            &state.conditions,
                            &safe,
                            ObligationKind::PanicSafety,
                            format!("{msg:?}"),
                        )?;
                        state.conditions.push(safe);
                    }
                    queue.push_back((*target, state));
                }
                TerminatorKind::Return => {
                    let value = state.locals[0]
                        .clone()
                        .ok_or("return value is not modeled")?;
                    returns.push(Return {
                        value,
                        conditions: state.conditions,
                    });
                }
                TerminatorKind::Call {
                    func,
                    args,
                    destination,
                    target,
                    ..
                } => {
                    let ty::FnDef(callee, generic_args) =
                        *func.ty(&body.local_decls, self.tcx).kind()
                    else {
                        return Err("unresolved indirect call".to_owned());
                    };
                    if self
                        .tcx
                        .lang_items()
                        .from_def_id(callee)
                        .is_some_and(|item| {
                            item.name().as_str().starts_with("panic")
                                || matches!(item, LangItem::BeginPanic | LangItem::ConstPanicFmt)
                        })
                    {
                        self.require(
                            id,
                            terminator.source_info.span,
                            &state.conditions,
                            "false",
                            ObligationKind::PanicSafety,
                            "panic entry point is reachable".to_owned(),
                        )?;
                        continue;
                    }
                    let values = args
                        .iter()
                        .map(|arg| self.operand(id, body, &state, &arg.node))
                        .collect::<Result<Vec<_>, _>>()?;
                    if self.tcx.lang_items().get(LangItem::SliceLen) == Some(callee) {
                        let [Value::Bytes { length, .. }] = values.as_slice() else {
                            return Err("slice len receiver is not modeled".to_owned());
                        };
                        self.write(&mut state, *destination, (**length).clone())?;
                        let target = target.ok_or("slice len has no return edge")?;
                        queue.push_back((target, state));
                        continue;
                    }
                    if !callee.is_local()
                        || !self.tcx.is_mir_available(callee)
                        || self.tcx.def_kind(self.tcx.parent(callee)) == DefKind::Trait
                        || generic_args.iter().any(|arg| {
                            matches!(arg.skip_binder().kind(), ty::GenericArgKind::Type(_))
                        })
                    {
                        return Err(format!(
                            "unmodeled call to {}",
                            self.tcx.def_path_str(callee)
                        ));
                    }
                    let results = self.execute(callee, values, state.conditions.clone(), &stack)?;
                    let target = target.ok_or("local call has no return edge")?;
                    for result in results {
                        let mut continuation = state.clone();
                        continuation.conditions = result.conditions;
                        self.write(&mut continuation, *destination, result.value)?;
                        queue.push_back((target, continuation));
                    }
                }
                TerminatorKind::Drop { .. } => {
                    return Err("destructor behavior is unmodeled".to_owned());
                }
                TerminatorKind::Unreachable => {
                    return Err("reachable MIR unreachable terminator".to_owned());
                }
                other => return Err(format!("unsupported terminator {other:?}")),
            }
        }
        Ok(returns)
    }

    fn statement(
        &self,
        id: DefId,
        body: &Body<'tcx>,
        state: &mut State,
        statement: &StatementKind<'tcx>,
    ) -> Result<(), String> {
        match statement {
            StatementKind::Assign(assignment) => {
                let (place, value) = assignment.as_ref();
                let value = self.rvalue(id, body, state, value)?;
                self.write(state, *place, value)
            }
            StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                state.locals[local.as_usize()] = None;
                Ok(())
            }
            StatementKind::Nop
            | StatementKind::ConstEvalCounter
            | StatementKind::Coverage(_)
            | StatementKind::PlaceMention(_)
            | StatementKind::BackwardIncompatibleDropHint { .. } => Ok(()),
            other => Err(format!("unsupported statement {other:?}")),
        }
    }

    fn write(&self, state: &mut State, place: Place<'tcx>, value: Value) -> Result<(), String> {
        if place.projection.is_empty() {
            state.locals[place.local.as_usize()] = Some(value);
            return Ok(());
        }
        Err("writes through references or projected places are unmodeled".to_owned())
    }

    fn place(&self, state: &State, place: Place<'tcx>) -> Result<Value, String> {
        let mut value = state.locals[place.local.as_usize()]
            .clone()
            .ok_or_else(|| format!("uninitialized or unsupported local {:?}", place.local))?;
        for projection in place.projection {
            value = match (projection, value) {
                (ProjectionElem::Deref, value @ Value::Bytes { .. }) => value,
                (ProjectionElem::Field(field, _), Value::Tuple(fields)) => fields
                    .get(field.as_usize())
                    .cloned()
                    .ok_or("tuple field missing")?,
                (ProjectionElem::Index(index), Value::Bytes { data, length }) => {
                    let index = state.locals[index.as_usize()]
                        .as_ref()
                        .ok_or("index is unavailable")?;
                    let (expression, bits, signed) = index.integer()?;
                    let (length_expression, length_bits, _) = length.integer()?;
                    if signed || bits != length_bits {
                        return Err("byte index type mismatch".to_owned());
                    }
                    let outside = format!("(bvuge {expression} {length_expression})");
                    if self.feasible(&[state.conditions.clone(), vec![outside]].concat())? {
                        return Err("byte read lacks a proven bounds check".to_owned());
                    }
                    Value::Int {
                        expression: format!("(select {data} {expression})"),
                        bits: 8,
                        signed: false,
                    }
                }
                _ => return Err(format!("unsupported place projection {projection:?}")),
            };
        }
        Ok(value)
    }

    fn operand(
        &self,
        id: DefId,
        _body: &Body<'tcx>,
        state: &State,
        operand: &Operand<'tcx>,
    ) -> Result<Value, String> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => self.place(state, *place),
            Operand::Constant(constant) => {
                let ty = constant.const_.ty();
                if matches!(ty.kind(), ty::Tuple(fields) if fields.is_empty()) {
                    return Ok(Value::Unit);
                }
                let bits = constant
                    .const_
                    .try_eval_bits(self.tcx, ty::TypingEnv::post_analysis(self.tcx, id))
                    .ok_or("unsupported MIR constant")?;
                if ty.is_bool() {
                    return Ok(Value::Bool((bits != 0).to_string()));
                }
                let (width, signed) = self.integer_type(ty).ok_or("unsupported constant type")?;
                Ok(symbolic::integer(bits, width, signed))
            }
            other => Err(format!("unsupported operand {other:?}")),
        }
    }

    fn rvalue(
        &self,
        id: DefId,
        body: &Body<'tcx>,
        state: &State,
        value: &Rvalue<'tcx>,
    ) -> Result<Value, String> {
        match value {
            Rvalue::Use(operand, _) => self.operand(id, body, state, operand),
            Rvalue::Ref(_, BorrowKind::Shared, place) => {
                let value = self.place(state, *place)?;
                if matches!(value, Value::Bytes { .. }) {
                    Ok(value)
                } else {
                    Err("only byte-array and slice reborrows are modeled".to_owned())
                }
            }
            Rvalue::BinaryOp(operation, operands) => {
                let left = self.operand(id, body, state, &operands.0)?;
                let right = self.operand(id, body, state, &operands.1)?;
                let operation = match operation {
                    BinOp::Add => "add",
                    BinOp::Sub => "sub",
                    BinOp::Mul => "mul",
                    BinOp::AddWithOverflow => "checked_add",
                    BinOp::SubWithOverflow => "checked_sub",
                    BinOp::MulWithOverflow => "checked_mul",
                    BinOp::Div => "div",
                    BinOp::Rem => "rem",
                    BinOp::BitAnd => "and",
                    BinOp::BitOr => "or",
                    BinOp::BitXor => "xor",
                    BinOp::Eq => "eq",
                    BinOp::Ne => "ne",
                    BinOp::Lt => "lt",
                    BinOp::Le => "le",
                    BinOp::Gt => "gt",
                    BinOp::Ge => "ge",
                    other => return Err(format!("unsupported binary operation {other:?}")),
                };
                symbolic::binary(operation, left, right)
            }
            Rvalue::UnaryOp(operation, operand) => {
                let value = self.operand(id, body, state, operand)?;
                match (operation, value) {
                    (UnOp::Not, Value::Bool(expression)) => {
                        Ok(Value::Bool(symbolic::not(&expression)))
                    }
                    (
                        UnOp::Not,
                        Value::Int {
                            expression,
                            bits,
                            signed,
                        },
                    ) => Ok(Value::Int {
                        expression: format!("(bvnot {expression})"),
                        bits,
                        signed,
                    }),
                    (
                        UnOp::Neg,
                        Value::Int {
                            expression,
                            bits,
                            signed: true,
                        },
                    ) => Ok(Value::Int {
                        expression: format!("(bvneg {expression})"),
                        bits,
                        signed: true,
                    }),
                    (UnOp::PtrMetadata, Value::Bytes { length, .. }) => Ok(*length),
                    _ => Err("unsupported unary operation".to_owned()),
                }
            }
            Rvalue::Cast(CastKind::IntToInt, operand, target) => {
                let value = self.operand(id, body, state, operand)?;
                let (bits, signed) = self
                    .integer_type(*target)
                    .ok_or("unsupported cast target")?;
                symbolic::cast(value, bits, signed)
            }
            Rvalue::Cast(CastKind::PointerCoercion(..), operand, target) if matches!(target.kind(), ty::Ref(_, element, mutability) if !mutability.is_mut() && matches!(element.kind(), ty::Slice(element) if *element == self.tcx.types.u8)) =>
            {
                let value = self.operand(id, body, state, operand)?;
                if matches!(value, Value::Bytes { .. }) {
                    Ok(value)
                } else {
                    Err("unsupported pointer coercion".to_owned())
                }
            }
            Rvalue::Aggregate(kind, fields) if matches!(kind.as_ref(), AggregateKind::Tuple) => {
                Ok(Value::Tuple(
                    fields
                        .iter()
                        .map(|field| self.operand(id, body, state, field))
                        .collect::<Result<_, _>>()?,
                ))
            }
            other => Err(format!("unsupported rvalue {other:?}")),
        }
    }
}
