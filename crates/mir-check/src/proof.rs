use super::contracts;
use super::solver::{Answer, Query, Solver};
use super::symbolic::{self, Context, Op, Sort, Term, Value};
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

const MAX_STEPS: usize = 8192;
const MAX_CALL_DEPTH: usize = 16;
const MAX_QUERY_BYTES: usize = 200_000;
const MAX_INPUT_DEPTH: usize = 16;
const MAX_INPUT_VALUES: usize = 512;
const MAX_ROOT_SECONDS: u64 = 30;

mod aggregates;
mod builtins;
mod constants;
mod external;
mod floating_intrinsics;
mod induction;
mod inputs;
mod integer_intrinsics;
mod interior;
mod iterators;
mod library;
mod memory;
mod owned_iterators;

#[derive(Clone)]
struct State {
    locals: Vec<Option<Value>>,
    conditions: Vec<Term>,
    addresses: Vec<Option<usize>>,
    memory: Vec<Option<Value>>,
}

struct Return {
    value: Value,
    conditions: Vec<Term>,
    memory: Vec<Option<Value>>,
}

struct Engine<'tcx> {
    tcx: TyCtxt<'tcx>,
    terms: Context,
    next_symbol: u32,
    float_encodings: BTreeMap<u32, Term>,
    steps: usize,
    input_depth: usize,
    input_values: usize,
    building_mutable_input: bool,
    started: std::time::Instant,
    proof: Proof,
    config: mir_check::ContractConfig,
    all_failures: bool,
    induction: bool,
    resolved_contracts: BTreeMap<String, DefId>,
    solver: std::cell::RefCell<Solver>,
    bodies:
        std::cell::RefCell<std::collections::HashMap<ty::Instance<'tcx>, std::rc::Rc<Body<'tcx>>>>,
}

pub fn verify(
    tcx: TyCtxt<'_>,
    id: DefId,
    config: &mir_check::ContractConfig,
    all_failures: bool,
    induction: bool,
) -> Proof {
    let mut engine = Engine {
        tcx,
        terms: Context::default(),
        next_symbol: 0,
        float_encodings: BTreeMap::new(),
        steps: 0,
        input_depth: 0,
        input_values: 0,
        building_mutable_input: false,
        started: std::time::Instant::now(),
        config: config.clone(),
        all_failures,
        induction,
        resolved_contracts: BTreeMap::new(),
        solver: std::cell::RefCell::new(Solver::default()),
        bodies: std::cell::RefCell::new(std::collections::HashMap::new()),
        proof: Proof {
            status: ProofStatus::Proved,
            assumptions: Vec::new(),
            inputs: BTreeMap::new(),
            models: Vec::new(),
            invariants: Vec::new(),
            analyzed_bodies: Vec::new(),
            obligations: Vec::new(),
            trusted_calls: Vec::new(),
            matched_contracts: Vec::new(),
            stopped_after_counterexample: false,
        },
    };
    let result = engine.root(id);
    if let Err(reason) = result
        && !engine.proof.stopped_after_counterexample
    {
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
    } else if !engine.proof.trusted_calls.is_empty() {
        ProofStatus::ProvedWithAssumptions
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
        let mut memory = Vec::new();
        for local in body.args_iter() {
            let ty = body.local_decls[local].ty;
            if let ty::Ref(_, element, mutability) = ty.kind()
                && mutability.is_mut()
            {
                if !memory.is_empty() {
                    return Err("only one mutable root reference is supported".to_owned());
                }
                self.building_mutable_input = true;
                let value = if matches!(element.kind(), ty::Slice(_)) {
                    self.byte_input(id, *element, &mut conditions)?
                } else {
                    self.argument(id, *element, &mut conditions)?
                };
                self.building_mutable_input = false;
                memory.push(Some(value));
                arguments.push(Value::Reference {
                    allocation: 0,
                    projection: Vec::new(),
                    mutable: true,
                });
            } else if let ty::Ref(_, element, _) = ty.kind()
                && let Some(inner) = self.cell_element(*element)
            {
                if !memory.is_empty() {
                    return Err(
                        "multiple interior/mutable root locations need alias constraints"
                            .to_owned(),
                    );
                }
                let value = self.argument(id, inner, &mut conditions)?;
                memory.push(Some(value));
                arguments.push(Value::Cell { allocation: 0 });
            } else {
                arguments.push(self.argument(id, ty, &mut conditions)?);
            }
        }
        let snapshots = self.snapshots(&arguments, &memory, &conditions)?;
        let instance = ty::Instance::new_raw(id, ty::GenericArgs::identity_for_item(self.tcx, id));
        let bindings = self.configured_bindings(body, &snapshots, instance)?;
        for (name, value) in &bindings {
            self.input_binding(name, value)?;
        }
        for contract in self.configured_contracts(instance)? {
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
        if self.induction && self.needs_induction(instance) {
            return self.inductive_root(instance, arguments, conditions, memory);
        }
        self.execute(instance, arguments, conditions, memory, &[])?;
        Ok(())
    }

    fn input_binding(&mut self, name: &str, value: &Value) -> Result<(), String> {
        let description = match value {
            Value::Int { expression, .. }
            | Value::Float { expression, .. }
            | Value::Bool(expression) => expression.smt(MAX_QUERY_BYTES)?,
            Value::Bytes { length, data } => format!(
                "len={}, data={}",
                length.integer()?.0.smt(MAX_QUERY_BYTES)?,
                data.smt(MAX_QUERY_BYTES)?
            ),
            Value::Adt { fields, .. } => {
                for (field, value) in fields {
                    self.input_binding(&format!("{name}.{field}"), value)?;
                }
                return Ok(());
            }
            Value::Enum {
                discriminant,
                variants,
                ..
            } => {
                self.input_binding(&format!("{name}.discriminant"), discriminant)?;
                for (index, variant) in variants.iter().enumerate() {
                    self.input_binding(&format!("{name}.variant{index}"), variant)?;
                }
                return Ok(());
            }
            Value::Cell { allocation } => format!("Cell allocation {allocation}; mutable contents"),
            Value::Atomic { bits, signed } => {
                format!("shared atomic {bits}-bit signed={signed}; arbitrary per access")
            }
            Value::Unit => "()".to_owned(),
            Value::Elements(elements) => {
                for (index, element) in elements.iter().enumerate() {
                    self.input_binding(&format!("{name}[{index}]"), element)?;
                }
                return Ok(());
            }
            Value::Tuple(fields) => {
                for (index, field) in fields.iter().enumerate() {
                    self.input_binding(&format!("{name}.{index}"), field)?;
                }
                return Ok(());
            }
            Value::Reference { .. }
            | Value::SliceIterator { .. }
            | Value::MetadataPointer(_)
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

    fn predicate(&self, text: &str, bindings: &BTreeMap<String, Value>) -> Result<Term, String> {
        contracts::predicate(
            &self.terms,
            text,
            bindings,
            u32::from(self.tcx.sess.target.pointer_width),
        )
        .map_err(|reason| format!("contract `{text}`: {reason}"))
    }

    fn fresh(&mut self, sort: Sort) -> Term {
        let symbol = self
            .terms
            .symbol(self.next_symbol, sort)
            .expect("modeled MIR sort");
        self.next_symbol += 1;
        symbol
    }

    fn argument(
        &mut self,
        id: DefId,
        ty: Ty<'tcx>,
        conditions: &mut Vec<Term>,
    ) -> Result<Value, String> {
        if self.input_depth >= MAX_INPUT_DEPTH {
            return Err("input shape exceeds 16 levels of nesting".to_owned());
        }
        if self.input_values >= MAX_INPUT_VALUES {
            return Err("input shape exceeds the 512-value budget".to_owned());
        }
        self.input_depth += 1;
        self.input_values += 1;
        let result = self.argument_value(id, ty, conditions);
        self.input_depth -= 1;
        result
    }

    fn argument_value(
        &mut self,
        id: DefId,
        ty: Ty<'tcx>,
        conditions: &mut Vec<Term>,
    ) -> Result<Value, String> {
        if let ty::Pat(base, pattern) = ty.kind() {
            let value = self.argument_value(id, *base, conditions)?;
            conditions.push(self.input_pattern(*pattern, &value, 0)?);
            return Ok(value);
        }
        if ty.is_char() {
            let value = Value::Int {
                expression: self.fresh(Sort::BitVec(32)),
                bits: 32,
                signed: false,
            };
            let within = symbolic::binary(
                &self.terms,
                "le",
                value.clone(),
                symbolic::integer(&self.terms, 0x10ffff, 32, false),
            )?
            .boolean()?;
            let below = symbolic::binary(
                &self.terms,
                "lt",
                value.clone(),
                symbolic::integer(&self.terms, 0xd800, 32, false),
            )?
            .boolean()?;
            let above = symbolic::binary(
                &self.terms,
                "gt",
                value.clone(),
                symbolic::integer(&self.terms, 0xdfff, 32, false),
            )?
            .boolean()?;
            conditions.push(self.terms.apply(
                Op::And,
                &[within, self.terms.apply(Op::Or, &[below, above])?],
            )?);
            return Ok(value);
        }
        if let Some(value) = self.atomic_shape(ty) {
            return Ok(value);
        }
        if let Some((bits, signed)) = self.integer_type(ty) {
            return Ok(Value::Int {
                expression: self.fresh(Sort::BitVec(bits)),
                bits,
                signed,
            });
        }
        if let Some(bits) = self.float_type(ty) {
            let raw = self.fresh(Sort::BitVec(bits));
            return Ok(symbolic::float_from_bits(&self.terms, raw, bits));
        }
        match ty.kind() {
            ty::Bool => Ok(Value::Bool(self.fresh(Sort::Bool))),
            ty::Tuple(fields) if fields.is_empty() => Ok(Value::Unit),
            ty::Tuple(fields) => Ok(Value::Tuple(
                fields
                    .iter()
                    .map(|field| self.argument(id, field, conditions))
                    .collect::<Result<_, _>>()?,
            )),
            ty::Ref(_, element, mutability) if !mutability.is_mut() => {
                if self.building_mutable_input {
                    return Err("mutable root pointees cannot contain reference fields".to_owned());
                }
                if matches!(element.kind(), ty::Slice(_)) {
                    self.byte_input(id, *element, conditions)
                } else {
                    self.argument(id, *element, conditions)
                }
            }
            ty::Array(element, _) if *element == self.tcx.types.u8 => {
                self.byte_input(id, ty, conditions)
            }
            ty::Array(..) => self.element_input(id, ty, conditions),
            ty::Adt(def, _) if def.is_struct() => self.struct_input(id, ty, conditions),
            ty::Adt(def, _) if def.is_enum() => self.enum_input(id, ty, conditions),
            _ => Err(format!("unsupported argument type {ty:?}")),
        }
    }

    fn byte_input(
        &mut self,
        id: DefId,
        ty: Ty<'tcx>,
        conditions: &mut Vec<Term>,
    ) -> Result<Value, String> {
        let bits = u32::from(self.tcx.sess.target.pointer_width);
        let length = match ty.kind() {
            ty::Slice(element) if *element == self.tcx.types.u8 => {
                let expression = self.fresh(Sort::BitVec(bits));
                // Valid non-ZST byte slices occupy at most isize::MAX bytes.
                let max = (1_u128 << (bits - 1)) - 1;
                conditions.push(self.terms.apply(
                    Op::BvUnsignedLe,
                    &[expression.clone(), self.terms.bit_vector(max, bits)?],
                )?);
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
                symbolic::integer(&self.terms, u128::from(length), bits, false)
            }
            _ => {
                return Err(format!(
                    "only read-only byte slices and byte arrays are modeled: {ty:?}"
                ));
            }
        };
        let data = self.fresh(Sort::Array(
            Box::new(Sort::BitVec(bits)),
            Box::new(Sort::BitVec(8)),
        ));
        Ok(Value::Bytes {
            length: Box::new(length),
            data,
        })
    }

    fn integer_type(&self, ty: Ty<'tcx>) -> Option<(u32, bool)> {
        let pointer_bits = u64::from(self.tcx.sess.target.pointer_width);
        match ty.kind() {
            ty::Pat(base, _) => self.integer_type(*base),
            ty::Char => Some((32, false)),
            ty::Int(kind) => Some((kind.bit_width().unwrap_or(pointer_bits) as u32, true)),
            ty::Uint(kind) => Some((kind.bit_width().unwrap_or(pointer_bits) as u32, false)),
            _ => None,
        }
    }

    fn float_type(&self, ty: Ty<'tcx>) -> Option<u32> {
        match ty.kind() {
            ty::Float(ty::FloatTy::F32) => Some(32),
            ty::Float(ty::FloatTy::F64) => Some(64),
            _ => None,
        }
    }

    fn query(&self, conditions: &[Term], failure: &Term) -> Result<Query, String> {
        if self.started.elapsed().as_secs() >= MAX_ROOT_SECONDS {
            return Err("symbolic root exceeded the 30-second execution budget".to_owned());
        }
        let query = Query::from_terms(&self.terms, conditions, failure, &self.float_encodings)?;
        if query.text().len() > MAX_QUERY_BYTES {
            return Err("symbolic query size limit reached".to_owned());
        }
        Ok(query)
    }

    fn feasible(&self, conditions: &[Term]) -> Result<bool, String> {
        self.solver
            .borrow_mut()
            .feasible_query(&self.query(conditions, &self.terms.boolean(true))?)
    }

    fn require(
        &mut self,
        id: DefId,
        span: Span,
        conditions: &[Term],
        safe: &Term,
        kind: ObligationKind,
        detail: String,
    ) -> Result<(), String> {
        let query = self.query(conditions, &symbolic::not(safe))?;
        let (status, model, detail) = match self.solver.borrow_mut().check_query(&query) {
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
            query: Some(query.text().to_owned()),
            model,
        });
        if status == ProofStatus::Unknown {
            return Err("solver could not discharge an obligation".to_owned());
        }
        if status == ProofStatus::Refuted && !self.all_failures {
            self.proof.stopped_after_counterexample = true;
            return Err("root stopped after its first counterexample".to_owned());
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

    fn instantiated_body(
        &self,
        instance: ty::Instance<'tcx>,
    ) -> Result<std::rc::Rc<Body<'tcx>>, String> {
        if let Some(body) = self.bodies.borrow().get(&instance) {
            return Ok(body.clone());
        }
        if !matches!(instance.def, ty::InstanceKind::Item(_)) {
            return Err(format!("unmodeled call adapter {:?}", instance.def));
        }
        if !self.tcx.is_mir_available(instance.def_id()) {
            return Err(self.missing_body_reason(instance.def_id()));
        }
        let body = instance
            .try_instantiate_mir_and_normalize_erasing_regions(
                self.tcx,
                ty::TypingEnv::fully_monomorphized(),
                ty::EarlyBinder::bind(self.tcx, self.tcx.instance_mir(instance.def).clone()),
            )
            .map_err(|error| format!("MIR substitution failed: {error:?}"))?;
        let body = std::rc::Rc::new(body);
        let mut bodies = self.bodies.borrow_mut();
        if bodies.len() >= 128 {
            bodies.clear();
        }
        bodies.insert(instance, body.clone());
        Ok(body)
    }

    fn execute(
        &mut self,
        instance: ty::Instance<'tcx>,
        arguments: Vec<Value>,
        conditions: Vec<Term>,
        memory: Vec<Option<Value>>,
        stack: &[DefId],
    ) -> Result<Vec<Return>, String> {
        let id = instance.def_id();
        if stack.len() >= MAX_CALL_DEPTH {
            return Err(
                "16-frame call-depth limit reached; recursion may require an invariant".to_owned(),
            );
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
        let snapshots = self.snapshots(&arguments, &memory, &conditions)?;
        let contracts = self.configured_contracts(instance)?;
        let binds_contract_names = contracts
            .iter()
            .any(|contract| !matches!(contract.kind, ContractKind::NoPanic))
            || self
                .specification(instance)?
                .is_some_and(|spec| !spec.arguments.is_empty());
        let bindings = if binds_contract_names {
            self.configured_bindings(body, &snapshots, instance)?
        } else {
            BTreeMap::new()
        };
        if bindings.keys().any(|name| name.starts_with("final_"))
            && contracts
                .iter()
                .any(|contract| matches!(contract.kind, ContractKind::Ensures))
        {
            return Err("final_ argument names are reserved for post-state bindings".to_owned());
        }
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
            addresses: vec![None; body.local_decls.len()],
            memory,
        };
        let incoming_allocations = state.memory.len();
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
            state.conditions.retain(|condition| {
                condition.constant() != Some(mir_check::smt::Constant::Bool(true))
            });
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
                                &self.terms,
                                "eq",
                                value.clone(),
                                symbolic::integer(&self.terms, number, *bits, *signed),
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
                    let value = if body.return_ty().is_unit() {
                        Value::Unit
                    } else {
                        self.local(&state, 0)?
                    };
                    self.validate_frame_escape(&value, &state, incoming_allocations)?;
                    let mut post_bindings = bindings.clone();
                    let uses_post_state = contracts
                        .iter()
                        .filter(|contract| matches!(contract.kind, ContractKind::Ensures))
                        .filter_map(|contract| contract.predicate.as_deref())
                        .try_fold(false, |found, predicate| {
                            Ok::<_, String>(found | contracts::uses_post_state(predicate)?)
                        })?;
                    if uses_post_state {
                        let final_arguments = body
                            .args_iter()
                            .map(|local| self.local(&state, local.as_usize()))
                            .collect::<Result<Vec<_>, _>>()?;
                        let final_snapshots =
                            self.snapshots(&final_arguments, &state.memory, &state.conditions)?;
                        for (name, value) in
                            self.configured_bindings(body, &final_snapshots, instance)?
                        {
                            post_bindings.insert(format!("final_{name}"), value);
                        }
                    }
                    if contracts
                        .iter()
                        .any(|contract| matches!(contract.kind, ContractKind::Ensures))
                    {
                        post_bindings.insert(
                            "result".to_owned(),
                            self.snapshot(&value, &state.memory, &state.conditions, 0)?,
                        );
                    }
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
                    let value = self.return_value(value, &state, incoming_allocations)?;
                    for allocation in state.addresses.iter().flatten() {
                        state.memory[*allocation] = None;
                    }
                    returns.push(Return {
                        value,
                        conditions: state.conditions,
                        memory: state.memory,
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
                    if super::identity::is_panic_call(self.tcx, callee)
                        || self.is_core_panic_helper(callee)
                    {
                        self.require(
                            id,
                            terminator.source_info.span,
                            &state.conditions,
                            &self.terms.boolean(false),
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
                        self.resolve_function_item(*id, args.skip_binder())?
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
                    if let Some(results) = self.trusted_call(
                        instance,
                        &values,
                        &state,
                        (id, terminator.source_info.span),
                    )? {
                        let target = target.ok_or("trusted call has no return edge")?;
                        for result in results {
                            let mut continuation = state.clone();
                            continuation.conditions = result.conditions;
                            continuation.memory = result.memory;
                            self.write(&mut continuation, *destination, result.value)?;
                            queue.push_back((target, continuation));
                        }
                        continue;
                    }
                    let configured = self.specification(instance)?.is_some();
                    if !configured
                        && let Some(results) = self.owned_iterator_call(
                            instance,
                            &values,
                            &mut state,
                            &stack,
                            (id, terminator.source_info.span),
                        )?
                    {
                        let target = target.ok_or("owned iterator call has no return edge")?;
                        for result in results {
                            let mut continuation = state.clone();
                            continuation.conditions = result.conditions;
                            continuation.memory = result.memory;
                            self.write(&mut continuation, *destination, result.value)?;
                            queue.push_back((target, continuation));
                        }
                        continue;
                    }
                    if !configured
                        && let Some(results) = self.iterator_call(
                            instance,
                            &values,
                            &mut state,
                            &stack,
                            (id, terminator.source_info.span),
                        )?
                    {
                        let target = target.ok_or("iterator call has no return edge")?;
                        for result in results {
                            let mut continuation = state.clone();
                            continuation.conditions = result.conditions;
                            continuation.memory = result.memory;
                            self.write(&mut continuation, *destination, result.value)?;
                            queue.push_back((target, continuation));
                        }
                        continue;
                    }
                    let modeled_values =
                        self.snapshots(&values, &state.memory, &state.conditions)?;
                    if !configured
                        && let Some(value) = self.interior_call(
                            instance,
                            &modeled_values,
                            &mut state,
                            (id, terminator.source_info.span),
                        )?
                    {
                        self.write(&mut state, *destination, value)?;
                        let target = target.ok_or("interior call has no return edge")?;
                        queue.push_back((target, state));
                        continue;
                    }
                    if !configured
                        && let Some(results) = self.library_call(
                            instance,
                            &modeled_values,
                            &values,
                            &state,
                            &stack,
                            (id, terminator.source_info.span),
                        )?
                    {
                        let target = target.ok_or("modeled call has no return edge")?;
                        for result in results {
                            let mut continuation = state.clone();
                            continuation.conditions = result.conditions;
                            continuation.memory = result.memory;
                            self.write(&mut continuation, *destination, result.value)?;
                            queue.push_back((target, continuation));
                        }
                        continue;
                    }
                    if !configured
                        && let Some(value) = self.builtin(
                            body,
                            instance,
                            &modeled_values,
                            &values,
                            &mut state,
                            terminator.source_info.span,
                        )?
                    {
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
                        state.memory.clone(),
                        &stack,
                        (id, terminator.source_info.span),
                    )?;
                    let target = target.ok_or("local call has no return edge")?;
                    for result in results {
                        let mut continuation = state.clone();
                        continuation.conditions = result.conditions;
                        continuation.memory = result.memory;
                        self.write(&mut continuation, *destination, result.value)?;
                        queue.push_back((target, continuation));
                    }
                }
                TerminatorKind::Drop { place, target, .. } => {
                    let ty = place.ty(&body.local_decls, self.tcx).ty;
                    if ty.needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized())
                        && !self.is_owned_no_drop_iterator(ty)
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
                if let Some(allocation) = state.addresses[local.as_usize()].take() {
                    state.memory[allocation] = None;
                }
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

    fn place(&self, state: &State, place: Place<'tcx>) -> Result<Value, String> {
        let mut value = self.local(state, place.local.as_usize())?;
        for projection in place.projection {
            value = match (projection, value) {
                (ProjectionElem::Deref, value @ Value::Reference { .. }) => {
                    self.reference_value(&value, &state.memory, &state.conditions)?
                }
                (
                    ProjectionElem::Deref,
                    value @ (Value::Bytes { .. }
                    | Value::Adt { .. }
                    | Value::Enum { .. }
                    | Value::Elements(_)
                    | Value::Tuple(_)
                    | Value::Unit
                    | Value::Int { .. }
                    | Value::Float { .. }
                    | Value::Bool(_)
                    | Value::StaticText
                    | Value::Cell { .. }
                    | Value::Atomic { .. }
                    | Value::SliceIterator { .. }),
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
                (
                    ProjectionElem::Downcast(_, expected),
                    Value::Enum {
                        discriminant,
                        variants,
                        ..
                    },
                ) => {
                    let value = variants
                        .get(expected.as_usize())
                        .ok_or("enum variant missing")?;
                    let Value::Adt {
                        discriminant: tag, ..
                    } = value
                    else {
                        return Err("enum payload is not an ADT".to_owned());
                    };
                    let (_, bits, signed) = discriminant.integer()?;
                    let equal = symbolic::binary(
                        &self.terms,
                        "eq",
                        *discriminant,
                        symbolic::integer(&self.terms, *tag, bits, signed),
                    )?
                    .boolean()?;
                    let mut wrong = state.conditions.clone();
                    wrong.push(symbolic::not(&equal));
                    if self.feasible(&wrong)? {
                        return Err("enum downcast lacks a proven variant check".to_owned());
                    }
                    value.clone()
                }
                (ProjectionElem::Index(index), Value::Bytes { data, length }) => {
                    let index = self.local(state, index.as_usize())?;
                    let (expression, bits, signed) = index.integer()?;
                    let (length_expression, length_bits, _) = length.integer()?;
                    if signed || bits != length_bits {
                        return Err("byte index type mismatch".to_owned());
                    }
                    let outside = self
                        .terms
                        .apply(Op::BvUnsignedGe, &[expression.clone(), length_expression])?;
                    if self.feasible(&[state.conditions.clone(), vec![outside]].concat())? {
                        return Err("byte read lacks a proven bounds check".to_owned());
                    }
                    Value::Int {
                        expression: self
                            .terms
                            .apply(Op::Select, &[data.clone(), expression.clone()])?,
                        bits: 8,
                        signed: false,
                    }
                }
                (ProjectionElem::Index(index), Value::Elements(elements)) => {
                    let index = self.local(state, index.as_usize())?;
                    self.fixed_element(&elements, &index, &state.conditions)?
                }
                (
                    ProjectionElem::ConstantIndex {
                        offset,
                        min_length,
                        from_end,
                    },
                    value @ (Value::Bytes { .. } | Value::Elements(_)),
                ) => self.constant_element(state, value, offset, min_length, from_end)?,
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
            Operand::RuntimeChecks(checks) => {
                Ok(Value::Bool(self.terms.boolean(checks.value(self.tcx.sess))))
            }
            Operand::Constant(constant) => {
                let ty = constant.const_.ty();
                if matches!(ty.kind(), ty::FnDef(..)) {
                    return Ok(Value::Function);
                }
                if matches!(ty.kind(), ty::Tuple(fields) if fields.is_empty()) {
                    return Ok(Value::Unit);
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
                self.constant(id, constant.const_, constant.span)
            }
        }
    }

    fn rvalue(
        &mut self,
        id: DefId,
        body: &Body<'tcx>,
        state: &mut State,
        value: &Rvalue<'tcx>,
    ) -> Result<Value, String> {
        match value {
            Rvalue::Use(operand, _) => self.operand(id, body, state, operand),
            Rvalue::Ref(_, BorrowKind::Shared, place) => self.borrow(state, *place, false),
            Rvalue::Ref(_, BorrowKind::Mut { .. }, place) => self.borrow(state, *place, true),
            Rvalue::RawPtr(rustc_middle::mir::RawPtrKind::FakeForPtrMetadata, place) => {
                let value = self.place(state, *place)?;
                let length = match value {
                    Value::Bytes { length, .. } => *length,
                    Value::Elements(elements) => symbolic::integer(
                        &self.terms,
                        elements.len() as u128,
                        u32::from(self.tcx.sess.target.pointer_width),
                        false,
                    ),
                    _ => {
                        return Err("metadata-only pointer needs array or slice storage".to_owned());
                    }
                };
                Ok(Value::MetadataPointer(Box::new(length)))
            }
            Rvalue::Repeat(operand, length) => {
                self.repeated_array(id, body, state, operand, *length)
            }
            Rvalue::Discriminant(place) => {
                let (bits, signed) = self
                    .integer_type(value.ty(&body.local_decls, self.tcx))
                    .ok_or("unsupported discriminant type")?;
                let ty = place.ty(&body.local_decls, self.tcx).ty;
                if let ty::Adt(def, _) = ty.kind()
                    && def.is_enum()
                    && let Ok(layout) = self
                        .tcx
                        .layout_of(ty::TypingEnv::fully_monomorphized().as_query_input(ty))
                    && let rustc_abi::Variants::Single { index } = layout.variants
                    && index.as_usize() < def.variants().len()
                {
                    return Ok(symbolic::integer(
                        &self.terms,
                        def.discriminant_for_variant(self.tcx, index).val,
                        bits,
                        signed,
                    ));
                }
                let modeled = self.place(state, *place)?;
                match modeled {
                    Value::Adt { discriminant, .. } => {
                        Ok(symbolic::integer(&self.terms, discriminant, bits, signed))
                    }
                    Value::Enum { discriminant, .. } => {
                        let (_, width, sign) = discriminant.integer()?;
                        if (width, sign) != (bits, signed) {
                            return Err("enum discriminant type mismatch".to_owned());
                        }
                        Ok(*discriminant)
                    }
                    _ => Err("unmodeled discriminant".to_owned()),
                }
            }
            Rvalue::BinaryOp(operation, operands) => {
                let left = self.operand(id, body, state, &operands.0)?;
                let right = self.operand(id, body, state, &operands.1)?;
                let operation = match operation {
                    BinOp::AddUnchecked | BinOp::SubUnchecked | BinOp::MulUnchecked => {
                        let checked = match operation {
                            BinOp::AddUnchecked => "checked_add",
                            BinOp::SubUnchecked => "checked_sub",
                            BinOp::MulUnchecked => "checked_mul",
                            _ => unreachable!(),
                        };
                        let Value::Tuple(mut fields) =
                            symbolic::binary(&self.terms, checked, left, right)?
                        else {
                            return Err("unchecked arithmetic needs integer operands".to_owned());
                        };
                        let overflow = fields.pop().ok_or("missing overflow result")?.boolean()?;
                        let safe = symbolic::not(&overflow);
                        self.require(
                            id,
                            self.tcx.def_span(id),
                            &state.conditions,
                            &safe,
                            ObligationKind::Validity,
                            format!("MIR {operation:?} must not overflow"),
                        )?;
                        state.conditions.push(safe);
                        return fields
                            .pop()
                            .ok_or_else(|| "missing arithmetic result".to_owned());
                    }
                    BinOp::Shl => return symbolic::shift(&self.terms, true, left, right),
                    BinOp::Shr => return symbolic::shift(&self.terms, false, left, right),
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
                let value = symbolic::binary(&self.terms, operation, left, right)?;
                Ok(self.materialize_float(value))
            }
            Rvalue::UnaryOp(operation, operand) => {
                let value = self.operand(id, body, state, operand)?;
                let value = if matches!(operation, UnOp::PtrMetadata) {
                    self.snapshot(&value, &state.memory, &state.conditions, 0)?
                } else {
                    value
                };
                match (operation, value) {
                    (UnOp::Neg, value @ Value::Float { .. }) => {
                        symbolic::float_negate(&self.terms, value)
                    }
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
                        expression: self.terms.apply(Op::BvNot, &[expression])?,
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
                        expression: self.terms.apply(Op::BvNeg, &[expression])?,
                        bits,
                        signed: true,
                    }),
                    (UnOp::PtrMetadata, Value::MetadataPointer(length)) => Ok(*length),
                    (UnOp::PtrMetadata, Value::Bytes { length, .. }) => Ok(*length),
                    (UnOp::PtrMetadata, Value::Elements(elements)) => Ok(symbolic::integer(
                        &self.terms,
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
                symbolic::cast(&self.terms, value, bits, signed)
            }
            Rvalue::Cast(CastKind::IntToFloat | CastKind::FloatToFloat, operand, target) => {
                let value = symbolic::float_cast(
                    &self.terms,
                    self.operand(id, body, state, operand)?,
                    self.float_type(*target)
                        .ok_or("unsupported float cast target")?,
                )?;
                Ok(self.materialize_float(value))
            }
            Rvalue::Cast(CastKind::Transmute, operand, target) => {
                let source = operand.ty(&body.local_decls, self.tcx);
                let value = self.operand(id, body, state, operand)?;
                if let Some(bits) = self.float_type(source)
                    && let Some((target_bits, signed)) = self.integer_type(*target)
                    && target_bits == bits
                {
                    return self.float_to_bits(value, bits, signed);
                }
                if let Some((source_bits, _)) = self.integer_type(source)
                    && let Some(bits) = self.float_type(*target)
                    && bits == source_bits
                {
                    return Ok(symbolic::float_from_bits(
                        &self.terms,
                        value.integer()?.0,
                        bits,
                    ));
                }
                Err(format!(
                    "unsupported transmute from {source:?} to {target:?}"
                ))
            }
            Rvalue::Cast(CastKind::FloatToInt, operand, target) => {
                let (bits, signed) = self
                    .integer_type(*target)
                    .ok_or("unsupported cast target")?;
                symbolic::cast(
                    &self.terms,
                    self.operand(id, body, state, operand)?,
                    bits,
                    signed,
                )
            }
            Rvalue::Cast(CastKind::PointerCoercion(..), operand, target) => {
                let ty::Ref(_, element, mutability) = target.kind() else {
                    return Err("unsupported pointer coercion".to_owned());
                };
                let ty::Slice(element) = element.kind() else {
                    return Err("only slice coercions are modeled".to_owned());
                };
                let value = self.operand(id, body, state, operand)?;
                if let Value::Reference {
                    allocation,
                    projection,
                    mutable,
                } = &value
                {
                    let inner = self.reference_value(&value, &state.memory, &state.conditions)?;
                    if matches!(inner, Value::Bytes { .. } | Value::Elements(_))
                        && (!mutability.is_mut() || *mutable)
                    {
                        return Ok(Value::Reference {
                            allocation: *allocation,
                            projection: projection.clone(),
                            mutable: mutability.is_mut(),
                        });
                    }
                }
                if !mutability.is_mut() && matches!(value, Value::Elements(_)) {
                    return Ok(value);
                }
                if *element != self.tcx.types.u8 {
                    return Err("only byte-slice coercions are modeled".to_owned());
                }
                if matches!((&value, mutability.is_mut()), (Value::Bytes { .. }, false)) {
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
