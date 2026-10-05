use super::contracts;
use super::solver::{self, Answer};
use super::symbolic::{self, Value};
use mir_check::{Contract, ContractKind, Obligation, ObligationKind, Proof, ProofStatus};
use rustc_attr_ir::{HasAttrs, LangItem};
use rustc_hir::def::DefKind;
use rustc_middle::mir::{
    AggregateKind, BinOp, Body, BorrowKind, CastKind, Operand, Place, ProjectionElem, Rvalue,
    START_BLOCK, StatementKind, TerminatorKind, UnOp, VarDebugInfoContents,
};
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::Span;
use rustc_span::def_id::DefId;
use std::collections::{BTreeMap, VecDeque};

const MAX_STEPS: usize = 256;
const MAX_CALL_DEPTH: usize = 8;
const MAX_QUERY_BYTES: usize = 200_000;

mod aggregates;
mod builtins;
mod library;

#[derive(Clone)]
struct State {
    locals: Vec<Option<Value>>,
    conditions: Vec<String>,
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
            inputs: BTreeMap::new(),
            models: Vec::new(),
            analyzed_bodies: Vec::new(),
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
        let bindings = self.bindings(body, &arguments)?;
        for (name, value) in &bindings {
            self.input_binding(name, value)?;
        }
        for contract in self.contracts(id) {
            if matches!(contract.kind, ContractKind::Requires) {
                let text = contract
                    .predicate
                    .as_deref()
                    .ok_or("missing precondition")?;
                conditions.push(self.predicate(text, &bindings)?);
                self.proof.assumptions.push(text.to_owned());
            }
        }
        if !self.feasible(&conditions)? {
            return Err(
                "entry preconditions are inconsistent; refusing a vacuous proof".to_owned(),
            );
        }
        let instance = ty::Instance::new_raw(id, ty::GenericArgs::identity_for_item(self.tcx, id));
        self.execute(instance, arguments, conditions, &[])?;
        Ok(())
    }

    fn input_binding(&mut self, name: &str, value: &Value) -> Result<(), String> {
        let description = match value {
            Value::Int { expression, .. } | Value::Bool(expression) => expression.clone(),
            Value::Bytes { length, data } => format!("len={}, data={data}", length.integer()?.0),
            Value::Adt { fields, .. } => {
                for (field, value) in fields {
                    self.input_binding(&format!("{name}.{field}"), value)?;
                }
                return Ok(());
            }
            Value::Unit => "()".to_owned(),
            Value::Elements(elements) => {
                for (index, element) in elements.iter().enumerate() {
                    self.input_binding(&format!("{name}[{index}]"), element)?;
                }
                return Ok(());
            }
            Value::Tuple(_)
            | Value::MutableBytes { .. }
            | Value::StaticText
            | Value::FormatArguments
            | Value::Function => {
                return Err("argument binding is unsupported".to_owned());
            }
        };
        self.proof.inputs.insert(name.to_owned(), description);
        Ok(())
    }

    fn contracts(&self, id: DefId) -> Vec<Contract> {
        id.get_attrs(&self.tcx)
            .iter()
            .filter_map(|attribute| attribute.doc_str())
            .filter_map(|doc| super::parse_contract(doc.as_str()))
            .collect()
    }

    fn bindings(
        &self,
        body: &Body<'tcx>,
        arguments: &[Value],
    ) -> Result<BTreeMap<String, Value>, String> {
        let mut bindings = BTreeMap::new();
        let mut bound_locals = BTreeMap::new();
        for debug in &body.var_debug_info {
            if let VarDebugInfoContents::Place(place) = debug.value
                && place.projection.is_empty()
                && place.local.as_usize() > 0
                && place.local.as_usize() <= body.arg_count
            {
                let name = debug.name.as_str().to_owned();
                if bound_locals
                    .insert(name.clone(), place.local)
                    .is_some_and(|local| local != place.local)
                {
                    return Err("ambiguous argument name in contract bindings".to_owned());
                }
                bindings.insert(name, arguments[place.local.as_usize() - 1].clone());
            }
        }
        Ok(bindings)
    }

    fn predicate(&self, text: &str, bindings: &BTreeMap<String, Value>) -> Result<String, String> {
        contracts::predicate(
            text,
            bindings,
            u32::from(self.tcx.sess.target.pointer_width),
        )
        .map_err(|reason| format!("contract `{text}`: {reason}"))
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
                let local_struct = match element.kind() {
                    ty::Adt(def, _) => def.is_struct() && def.did().is_local(),
                    _ => false,
                };
                if local_struct {
                    self.struct_input(id, *element, conditions)
                } else if matches!(element.kind(), ty::Array(..))
                    || self.integer_type(*element).is_some()
                    || element.is_bool()
                {
                    self.argument(id, *element, conditions)
                } else {
                    self.byte_input(id, *element, conditions)
                }
            }
            ty::Array(element, _) if *element == self.tcx.types.u8 => {
                self.byte_input(id, ty, conditions)
            }
            ty::Array(..) => self.element_input(id, ty, conditions),
            ty::Adt(def, _) if def.is_struct() && def.did().is_local() => {
                self.struct_input(id, ty, conditions)
            }
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

    fn instantiated_body(&self, instance: ty::Instance<'tcx>) -> Result<Body<'tcx>, String> {
        if !matches!(instance.def, ty::InstanceKind::Item(_)) {
            return Err(format!("unmodeled call adapter {:?}", instance.def));
        }
        if !self.tcx.is_mir_available(instance.def_id()) {
            return Err(format!(
                "MIR body unavailable for {}",
                self.tcx.def_path_str(instance.def_id())
            ));
        }
        instance
            .try_instantiate_mir_and_normalize_erasing_regions(
                self.tcx,
                ty::TypingEnv::fully_monomorphized(),
                ty::EarlyBinder::bind(self.tcx, self.tcx.instance_mir(instance.def).clone()),
            )
            .map_err(|error| format!("MIR substitution failed: {error:?}"))
    }

    fn execute(
        &mut self,
        instance: ty::Instance<'tcx>,
        arguments: Vec<Value>,
        conditions: Vec<String>,
        stack: &[DefId],
    ) -> Result<Vec<Return>, String> {
        let id = instance.def_id();
        if stack.contains(&id) || stack.len() >= MAX_CALL_DEPTH {
            return Err("recursion or call-depth limit requires an invariant".to_owned());
        }
        let mut stack = stack.to_vec();
        stack.push(id);
        let owned_body = self.instantiated_body(instance)?;
        let body = &owned_body;
        let name = format!("{} {:?}", self.tcx.def_path_str(id), instance.args);
        if !self.proof.analyzed_bodies.contains(&name) {
            self.proof.analyzed_bodies.push(name);
        }
        if arguments.len() != body.arg_count {
            return Err("call arguments do not match the MIR body".to_owned());
        }
        if arguments.iter().any(Value::contains_mutable) {
            return Err("mutable local borrows cannot cross an unmodeled call boundary".to_owned());
        }
        let bindings = self.bindings(body, &arguments)?;
        let contracts = self.contracts(id);
        if bindings.contains_key("result")
            && contracts
                .iter()
                .any(|contract| matches!(contract.kind, ContractKind::Ensures))
        {
            return Err("result is reserved for the postcondition return value".to_owned());
        }
        let mut state = State {
            locals: vec![None; body.local_decls.len()],
            conditions,
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
                    if value.contains_mutable() {
                        return Err("mutable local borrows cannot escape their frame".to_owned());
                    }
                    let mut post_bindings = bindings.clone();
                    post_bindings.insert("result".to_owned(), value.clone());
                    for contract in &contracts {
                        if matches!(contract.kind, ContractKind::Ensures) {
                            let text = contract
                                .predicate
                                .as_deref()
                                .ok_or("missing postcondition")?;
                            let safe = self.predicate(text, &post_bindings)?;
                            self.require(
                                id,
                                terminator.source_info.span,
                                &state.conditions,
                                &safe,
                                ObligationKind::Postcondition,
                                text.to_owned(),
                            )?;
                        }
                    }
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
                    let mut values = args
                        .iter()
                        .map(|arg| self.operand(id, body, &state, &arg.node))
                        .collect::<Result<Vec<_>, _>>()?;
                    let instance = ty::Instance::try_resolve(
                        self.tcx,
                        ty::TypingEnv::fully_monomorphized(),
                        callee,
                        generic_args.skip_binder(),
                    )
                    .map_err(|_| "call instance resolution failed".to_owned())?
                    .ok_or_else(|| {
                        format!("unresolved call to {}", self.tcx.def_path_str(callee))
                    })?;
                    let fn_trait = [LangItem::Fn, LangItem::FnMut, LangItem::FnOnce]
                        .iter()
                        .any(|item| {
                            self.tcx.lang_items().get(*item) == Some(self.tcx.parent(callee))
                        });
                    let instance = if fn_trait
                        && let ty::FnDef(id, args) = generic_args.skip_binder().type_at(0).kind()
                    {
                        let [Value::Function, Value::Tuple(parameters)] = values.as_slice() else {
                            return Err(
                                "function-item call arguments are not a Rust-call tuple".to_owned()
                            );
                        };
                        values = parameters.clone();
                        ty::Instance::new_raw(*id, args.skip_binder())
                    } else {
                        instance
                    };
                    let instance = if let ty::InstanceKind::Shim(ty::ShimKind::ClosureOnce {
                        closure,
                        ..
                    }) = instance.def
                    {
                        let ty::Closure(id, args) = instance.args.type_at(0).kind() else {
                            return Err("closure adapter receiver is not a closure".to_owned());
                        };
                        if *id != closure {
                            return Err("closure adapter identity mismatch".to_owned());
                        }
                        ty::Instance::new_raw(closure, args)
                    } else {
                        instance
                    };
                    let callee = instance.def_id();
                    if let Some(results) = self.library_call(
                        instance,
                        &values,
                        &state,
                        &stack,
                        (id, terminator.source_info.span),
                    )? {
                        let target = target.ok_or("modeled call has no return edge")?;
                        for result in results {
                            let mut continuation = state.clone();
                            continuation.conditions = result.conditions;
                            self.write(&mut continuation, *destination, result.value)?;
                            queue.push_back((target, continuation));
                        }
                        continue;
                    }
                    if let Some(value) = self.builtin(
                        body,
                        callee,
                        instance.args,
                        &values,
                        &mut state,
                        terminator.source_info.span,
                    )? {
                        self.write(&mut state, *destination, value)?;
                        let target = target.ok_or("modeled call has no return edge")?;
                        queue.push_back((target, state));
                        continue;
                    }
                    if matches!(instance.def, ty::InstanceKind::Item(_))
                        && self.tcx.def_kind(callee) == DefKind::Closure
                    {
                        let [closure, Value::Tuple(parameters)] = values.as_slice() else {
                            return Err(
                                "closure call arguments are not a Rust-call tuple".to_owned()
                            );
                        };
                        let mut flattened = vec![closure.clone()];
                        flattened.extend(parameters.iter().cloned());
                        values = flattened;
                    }
                    let results = self.call_instance(
                        instance,
                        values,
                        state.conditions.clone(),
                        &stack,
                        (id, terminator.source_info.span),
                    )?;
                    let target = target.ok_or("local call has no return edge")?;
                    for result in results {
                        let mut continuation = state.clone();
                        continuation.conditions = result.conditions;
                        self.write(&mut continuation, *destination, result.value)?;
                        queue.push_back((target, continuation));
                    }
                }
                TerminatorKind::Drop { place, target, .. } => {
                    if place
                        .ty(&body.local_decls, self.tcx)
                        .ty
                        .needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized())
                    {
                        return Err("destructor behavior is unmodeled".to_owned());
                    }
                    queue.push_back((*target, state));
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
        &mut self,
        id: DefId,
        body: &Body<'tcx>,
        state: &mut State,
        statement: &StatementKind<'tcx>,
    ) -> Result<(), String> {
        match statement {
            StatementKind::Intrinsic(intrinsic) => match intrinsic.as_ref() {
                rustc_middle::mir::NonDivergingIntrinsic::Assume(operand) => {
                    let safe = self.operand(id, body, state, operand)?.boolean()?;
                    self.require(
                        id,
                        self.tcx.def_span(id),
                        &state.conditions,
                        &safe,
                        ObligationKind::Validity,
                        "MIR assume must follow from the current path".to_owned(),
                    )?;
                    state.conditions.push(safe);
                    Ok(())
                }
                rustc_middle::mir::NonDivergingIntrinsic::CopyNonOverlapping(_) => {
                    Err("unmodeled pointer copy intrinsic".to_owned())
                }
            },
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
                (
                    ProjectionElem::Deref,
                    value @ (Value::Bytes { .. }
                    | Value::Adt { .. }
                    | Value::MutableBytes { .. }
                    | Value::Elements(_)
                    | Value::Int { .. }
                    | Value::Bool(_)),
                ) => value,
                (ProjectionElem::Field(field, _), Value::Tuple(fields)) => fields
                    .get(field.as_usize())
                    .cloned()
                    .ok_or("tuple field missing")?,
                (ProjectionElem::Field(field, _), Value::Adt { fields, .. }) => fields
                    .get(field.as_usize())
                    .map(|(_, value)| value.clone())
                    .ok_or("ADT field missing")?,
                (ProjectionElem::Downcast(_, expected), value @ Value::Adt { .. }) => {
                    let Value::Adt { variant, .. } = &value else {
                        unreachable!()
                    };
                    if *variant != expected.as_usize() {
                        return Err("enum downcast does not match the modeled variant".to_owned());
                    }
                    value
                }
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
                (ProjectionElem::Index(index), Value::Elements(elements)) => {
                    let index = state.locals[index.as_usize()]
                        .as_ref()
                        .ok_or("index is unavailable")?;
                    self.fixed_element(&elements, index, &state.conditions)?
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
                if matches!(ty.kind(), ty::FnDef(..)) {
                    return Ok(Value::Function);
                }
                if matches!(ty.kind(), ty::Tuple(fields) if fields.is_empty()) {
                    return Ok(Value::Unit);
                }
                if matches!(
                    constant.const_,
                    rustc_middle::mir::Const::Val(rustc_middle::mir::ConstValue::ZeroSized, _)
                ) && matches!(ty.kind(), ty::Adt(def, args)
                        if self.tcx.lang_items().get(LangItem::Option) == Some(def.did())
                            && args.type_at(0).is_never())
                {
                    return self.constructed(ty, 0, Vec::new());
                }
                if matches!(ty.kind(), ty::Ref(_, element, mutability)
                    if element.is_str() && !mutability.is_mut())
                    && matches!(
                        constant.const_,
                        rustc_middle::mir::Const::Val(
                            rustc_middle::mir::ConstValue::Slice { .. },
                            _
                        )
                    )
                {
                    return Ok(Value::StaticText);
                }
                let bits = constant
                    .const_
                    .try_eval_bits(self.tcx, ty::TypingEnv::post_analysis(self.tcx, id))
                    .ok_or_else(|| format!("unsupported MIR constant {:?}", constant.const_))?;
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
                if matches!(
                    value,
                    Value::Bytes { .. }
                        | Value::Adt { .. }
                        | Value::Elements(_)
                        | Value::Int { .. }
                        | Value::Bool(_)
                ) {
                    Ok(value)
                } else {
                    Err("only byte-array and slice reborrows are modeled".to_owned())
                }
            }
            Rvalue::Ref(_, BorrowKind::Mut { .. }, place)
                if matches!(
                    place.ty(&body.local_decls, self.tcx).ty.kind(),
                    ty::Closure(..)
                ) =>
            {
                self.place(state, *place)
            }
            Rvalue::Ref(_, BorrowKind::Mut { .. }, place) => self.mutable_bytes(state, *place),
            Rvalue::Repeat(operand, length) => {
                self.repeated_bytes(id, body, state, operand, *length)
            }
            Rvalue::Discriminant(place) => {
                let Value::Adt { discriminant, .. } = self.place(state, *place)? else {
                    return Err("unmodeled discriminant".to_owned());
                };
                let (bits, signed) = self
                    .integer_type(value.ty(&body.local_decls, self.tcx))
                    .ok_or("unsupported discriminant type")?;
                Ok(symbolic::integer(discriminant, bits, signed))
            }
            Rvalue::BinaryOp(operation, operands) => {
                let left = self.operand(id, body, state, &operands.0)?;
                let right = self.operand(id, body, state, &operands.1)?;
                let operation = match operation {
                    BinOp::Shl => return symbolic::shift(true, left, right),
                    BinOp::Shr => return symbolic::shift(false, left, right),
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
                    (UnOp::PtrMetadata, Value::Elements(elements)) => Ok(symbolic::integer(
                        elements.len() as u128,
                        u32::from(self.tcx.sess.target.pointer_width),
                        false,
                    )),
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
            Rvalue::Cast(CastKind::PointerCoercion(..), operand, target) => {
                let ty::Ref(_, element, mutability) = target.kind() else {
                    return Err("unsupported pointer coercion".to_owned());
                };
                let ty::Slice(element) = element.kind() else {
                    return Err("only slice coercions are modeled".to_owned());
                };
                let value = self.operand(id, body, state, operand)?;
                if !mutability.is_mut() && matches!(value, Value::Elements(_)) {
                    return Ok(value);
                }
                if *element != self.tcx.types.u8 {
                    return Err("only byte-slice coercions are modeled".to_owned());
                }
                if matches!(
                    (&value, mutability.is_mut()),
                    (Value::Bytes { .. }, false) | (Value::MutableBytes { .. }, true)
                ) {
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
            Rvalue::Aggregate(kind, fields) => self.aggregate(id, body, state, kind, fields),
            other => Err(format!("unsupported rvalue {other:?}")),
        }
    }
}
