use super::*;
use mir_check::smt::horn::{Atom, Clause, System};
use rustc_middle::mir::BasicBlock;
use std::collections::BTreeSet;

const MAX_BLOCKS: usize = 256;
const MAX_STATE_VALUES: usize = 512;

impl<'tcx> Engine<'tcx> {
    pub(super) fn needs_induction(&self, instance: ty::Instance<'tcx>) -> bool {
        let mut pending = vec![(instance, false)];
        let mut active = std::collections::HashSet::new();
        let mut complete = std::collections::HashSet::new();
        while let Some((instance, leaving)) = pending.pop() {
            if leaving {
                active.remove(&instance);
                complete.insert(instance);
                continue;
            }
            if complete.contains(&instance) {
                continue;
            }
            if !active.insert(instance) {
                return true;
            }
            // Failure to discover a cycle retains bounded interpretation; it never
            // licenses a proof or bypasses an unavailable callee.
            if active.len() + complete.len() > 128 {
                return false;
            }
            let Ok(body) = self.instantiated_body(instance) else {
                active.remove(&instance);
                complete.insert(instance);
                continue;
            };
            if self.has_cycle(&body) {
                return true;
            }
            pending.push((instance, true));
            for block in body.basic_blocks.iter() {
                if let TerminatorKind::Call { func, .. } = &block.terminator().kind
                    && let ty::FnDef(id, args) = *func.ty(&body.local_decls, self.tcx).kind()
                    && !super::super::identity::is_panic_call(self.tcx, id)
                    && !self.is_core_panic_helper(id)
                    && let Ok(Some(callee)) = ty::Instance::try_resolve(
                        self.tcx,
                        ty::TypingEnv::fully_monomorphized(),
                        id,
                        args.skip_binder(),
                    )
                {
                    pending.push((callee, false));
                }
            }
        }
        false
    }

    fn has_cycle(&self, body: &Body<'tcx>) -> bool {
        let mut pending = vec![(START_BLOCK, false)];
        let mut active = BTreeSet::new();
        let mut complete = BTreeSet::new();
        while let Some((block, leaving)) = pending.pop() {
            if leaving {
                active.remove(&block);
                complete.insert(block);
            } else if !complete.contains(&block) {
                if !active.insert(block) {
                    return true;
                }
                pending.push((block, true));
                pending.extend(
                    body.basic_blocks[block]
                        .terminator()
                        .successors()
                        .map(|target| (target, false)),
                );
            }
        }
        false
    }

    pub(super) fn inductive_root(
        &mut self,
        instance: ty::Instance<'tcx>,
        arguments: Vec<Value>,
        conditions: Vec<Term>,
        memory: Vec<Option<Value>>,
    ) -> Result<(), String> {
        if !memory.is_empty() {
            return Err("loop induction does not yet model mutable or interior storage".into());
        }
        let mut system = System {
            relations: Vec::new(),
            clauses: Vec::new(),
        };
        let root = self.loop_frame(instance, Vec::new(), None, &mut system)?;
        let initial = root.entry_atom(&arguments, Vec::new())?;
        system.clauses.push(Clause {
            premise: None,
            conditions,
            conclusion: Some(initial),
        });
        let mut frames = vec![root];
        let mut pending = vec![(0, START_BLOCK)];
        let mut visited = BTreeSet::new();
        while let Some((frame_index, block)) = pending.pop() {
            if !visited.insert((frame_index, block)) {
                continue;
            }
            if self.started.elapsed().as_secs() >= MAX_ROOT_SECONDS {
                return Err("loop translation exceeded the root time budget".into());
            }
            let frame = frames[frame_index].clone();
            let id = frame.instance.def_id();
            let body = &frame.body;
            let mut state = frame.state.clone();
            let premise = frame.atom(block, &state)?;
            for statement in &body.basic_blocks[block].statements {
                self.loop_statement(id, body, &mut state, &statement.kind, &mut system, &premise)
                    .map_err(|reason| {
                        format!(
                            "loop induction {} bb{}: {reason}",
                            self.tcx.def_path_str(id),
                            block.as_usize(),
                        )
                    })?;
            }
            let terminator = body.basic_blocks[block].terminator();
            let mut edges = Vec::new();
            match &terminator.kind {
                TerminatorKind::Goto { target } => edges.push((*target, state)),
                TerminatorKind::SwitchInt { discr, targets } => {
                    self.loop_operand_bounds(&mut state, discr, &mut system, &premise)?;
                    let value = self.operand(id, body, &state, discr)?;
                    let mut excluded = Vec::new();
                    for (number, target) in targets.iter() {
                        let condition = match &value {
                            Value::Bool(expression) => match number {
                                0 => symbolic::not(expression),
                                1 => expression.clone(),
                                _ => return Err("invalid loop Boolean switch".into()),
                            },
                            Value::Int { bits, signed, .. } => symbolic::binary(
                                &self.terms,
                                "eq",
                                value.clone(),
                                symbolic::integer(&self.terms, number, *bits, *signed),
                            )?
                            .boolean()?,
                            _ => return Err("loop switch needs an integer or Boolean".into()),
                        };
                        let mut branch = state.clone();
                        branch.conditions.push(condition.clone());
                        edges.push((target, branch));
                        excluded.push(symbolic::not(&condition));
                    }
                    state.conditions.extend(excluded);
                    edges.push((targets.otherwise(), state));
                }
                TerminatorKind::Assert {
                    cond,
                    expected,
                    msg,
                    target,
                    ..
                } => {
                    if !msg.is_optional_overflow_check() || self.tcx.sess.overflow_checks() {
                        self.loop_operand_bounds(&mut state, cond, &mut system, &premise)?;
                        let condition = self.operand(id, body, &state, cond)?.boolean()?;
                        let safe = if *expected {
                            condition
                        } else {
                            symbolic::not(&condition)
                        };
                        exclude_failure(&mut system, &premise, &state.conditions, &safe);
                        state.conditions.push(safe);
                    }
                    edges.push((*target, state));
                }
                TerminatorKind::Return => {
                    if body.return_ty().is_never() {
                        exclude_failure(
                            &mut system,
                            &premise,
                            &state.conditions,
                            &self.terms.boolean(false),
                        );
                        continue;
                    }
                    let result = if body.return_ty().is_unit() {
                        Value::Unit
                    } else {
                        self.local(&state, 0)?
                    };
                    self.loop_postconditions(&frame, &mut state, &result, &mut system, &premise)?;
                    if let Some(resume) = &frame.resume {
                        let target = resume
                            .target
                            .ok_or("a diverging call unexpectedly returned")?;
                        let caller = &frames[resume.caller];
                        let mut restored = caller.state.clone();
                        restored.conditions = state.conditions;
                        self.loop_place_bounds(
                            &mut restored,
                            resume.destination,
                            &mut system,
                            &premise,
                        )?;
                        self.write(&mut restored, resume.destination, result)?;
                        system.clauses.push(Clause {
                            premise: Some(premise.clone()),
                            conditions: restored.conditions.clone(),
                            conclusion: Some(caller.atom(target, &restored)?),
                        });
                        pending.push((resume.caller, target));
                    }
                }
                TerminatorKind::Call {
                    func,
                    args,
                    destination,
                    target,
                    ..
                } => {
                    let ty::FnDef(callee, generics) = *func.ty(&body.local_decls, self.tcx).kind()
                    else {
                        return Err("loop induction cannot resolve an indirect call".into());
                    };
                    if super::super::identity::is_panic_call(self.tcx, callee)
                        || self.is_core_panic_helper(callee)
                    {
                        exclude_failure(
                            &mut system,
                            &premise,
                            &state.conditions,
                            &self.terms.boolean(false),
                        );
                        continue;
                    }
                    let callee = ty::Instance::try_resolve(
                        self.tcx,
                        ty::TypingEnv::fully_monomorphized(),
                        callee,
                        generics.skip_binder(),
                    )
                    .map_err(|error| format!("inductive call resolution failed: {error:?}"))?
                    .ok_or("inductive call has no concrete instance")?;
                    let mut ancestor = Some(frame_index);
                    let mut depth = 0;
                    while let Some(index) = ancestor {
                        if frames[index].instance == callee {
                            return Err(format!(
                                "loop induction does not yet encode recursive call {}",
                                self.tcx.def_path_str(callee.def_id())
                            ));
                        }
                        depth += 1;
                        ancestor = frames[index].resume.as_ref().map(|resume| resume.caller);
                    }
                    if depth >= MAX_CALL_DEPTH {
                        return Err("inductive call graph exceeds the 16-frame depth limit".into());
                    }
                    let mut values = Vec::new();
                    for argument in args {
                        self.loop_operand_bounds(
                            &mut state,
                            &argument.node,
                            &mut system,
                            &premise,
                        )?;
                        values.push(self.operand(id, body, &state, &argument.node)?);
                    }
                    let inherited = frame.parameters(&frame.state)?;
                    let child = self.loop_frame(
                        callee,
                        inherited,
                        Some(Resume {
                            caller: frame_index,
                            target: *target,
                            destination: *destination,
                        }),
                        &mut system,
                    )?;
                    self.loop_preconditions(&child, &values, &mut state, &mut system, &premise)?;
                    let captured = frame.parameters(&state)?;
                    let entry = child.entry_atom(&values, captured)?;
                    system.clauses.push(Clause {
                        premise: Some(premise.clone()),
                        conditions: state.conditions,
                        conclusion: Some(entry),
                    });
                    let child_index = frames.len();
                    frames.push(child);
                    pending.push((child_index, START_BLOCK));
                }
                TerminatorKind::Unreachable => exclude_failure(
                    &mut system,
                    &premise,
                    &state.conditions,
                    &self.terms.boolean(false),
                ),
                other => return Err(format!("loop induction unsupported terminator {other:?}")),
            }
            for (target, state) in edges {
                system.clauses.push(Clause {
                    premise: Some(premise.clone()),
                    conditions: state.conditions.clone(),
                    conclusion: Some(frame.atom(target, &state)?),
                });
                pending.push((frame_index, target));
            }
        }
        let query = system.smt(&self.terms, MAX_QUERY_BYTES)?;
        let result = self.solver.borrow_mut().inductive_model(&query);
        let (status, detail) = match result {
            Ok(model) => {
                self.proof.invariants.push(model);
                (
                    ProofStatus::Proved,
                    "Spacer established unbounded MIR panic safety".to_owned(),
                )
            }
            Err(reason) => (ProofStatus::Unknown, reason),
        };
        self.proof.obligations.push(Obligation {
            function: self.tcx.def_path_str(instance.def_id()),
            source: super::super::source(self.tcx, self.tcx.def_span(instance.def_id())),
            kind: ObligationKind::Validity,
            detail,
            status,
            query: Some(query),
            model: None,
        });
        Ok(())
    }

    fn loop_frame(
        &mut self,
        instance: ty::Instance<'tcx>,
        inherited: Vec<Term>,
        resume: Option<Resume<'tcx>>,
        system: &mut System,
    ) -> Result<Frame<'tcx>, String> {
        if self
            .specification(instance)?
            .is_some_and(|spec| spec.trusted)
        {
            return Err(format!(
                "loop induction does not yet encode trusted boundary {}",
                self.tcx.def_path_str(instance.def_id())
            ));
        }
        let body = self.instantiated_body(instance)?;
        if system.relations.len() + body.basic_blocks.len() > MAX_BLOCKS {
            return Err(
                "loop induction exceeds its 256-block call-graph translation budget".into(),
            );
        }
        let contracts = self.configured_contracts(instance)?;
        let mut locals = Vec::new();
        for declaration in &body.local_decls {
            locals.push(Some(self.loop_input(declaration.ty, 0)?));
        }
        let state = State {
            locals,
            conditions: Vec::new(),
            addresses: vec![None; body.local_decls.len()],
            memory: Vec::new(),
        };
        let mut entry = Vec::new();
        if contracts
            .iter()
            .any(|contract| matches!(contract.kind, ContractKind::Ensures))
        {
            for local in body.args_iter() {
                entry.push(self.loop_input(body.local_decls[local].ty, 0)?);
            }
        }
        let mut ghosts = inherited;
        for value in &entry {
            flatten_value(value, &mut ghosts)?;
        }
        let frame = Frame {
            instance,
            body,
            state,
            entry,
            ghosts,
            contracts: contracts.into(),
            resume,
            base: system.relations.len(),
        };
        self.loop_contract_syntax(&frame)?;
        let parameters = frame.parameters(&frame.state)?;
        if parameters.len() > MAX_STATE_VALUES {
            return Err("loop induction exceeds its 512-scalar call-frame state budget".into());
        }
        let sorts = parameters
            .iter()
            .map(|term| term.sort().clone())
            .collect::<Vec<_>>();
        system
            .relations
            .extend(vec![sorts; frame.body.basic_blocks.len()]);
        let name = format!(
            "{} {:?}",
            self.tcx.def_path_str(instance.def_id()),
            instance.args
        );
        if !self.proof.analyzed_bodies.contains(&name) {
            self.proof.analyzed_bodies.push(name);
        }
        Ok(frame)
    }

    fn loop_contract_syntax(&self, frame: &Frame<'tcx>) -> Result<(), String> {
        let needs_bindings = frame
            .contracts
            .iter()
            .any(|contract| !matches!(contract.kind, ContractKind::NoPanic))
            || self
                .specification(frame.instance)?
                .is_some_and(|spec| !spec.arguments.is_empty());
        if !needs_bindings {
            return Ok(());
        }
        let arguments = frame
            .body
            .args_iter()
            .map(|local| self.local(&frame.state, local.as_usize()))
            .collect::<Result<Vec<_>, _>>()?;
        let bindings = self.configured_bindings(&frame.body, &arguments, frame.instance)?;
        let mut post_bindings = bindings.clone();
        if frame
            .contracts
            .iter()
            .any(|contract| matches!(contract.kind, ContractKind::Ensures))
        {
            check_reserved_names(&bindings)?;
            post_bindings = self.configured_bindings(&frame.body, &frame.entry, frame.instance)?;
            for (name, value) in &bindings {
                post_bindings.insert(format!("final_{name}"), value.clone());
            }
            post_bindings.insert("result".into(), self.local(&frame.state, 0)?);
        }
        for contract in frame.contracts.iter() {
            let values = match contract.kind {
                ContractKind::NoPanic => continue,
                ContractKind::Requires => &bindings,
                ContractKind::Ensures => &post_bindings,
            };
            let predicate = contract
                .predicate
                .as_deref()
                .ok_or("missing induction predicate")?;
            // Validate representation and syntax only; no predicate is assumed here.
            self.predicate(predicate, values)?;
        }
        Ok(())
    }

    fn loop_preconditions(
        &self,
        frame: &Frame<'tcx>,
        arguments: &[Value],
        state: &mut State,
        system: &mut System,
        premise: &Atom,
    ) -> Result<(), String> {
        if arguments.len() != frame.body.arg_count {
            return Err("inductive call arguments do not match its MIR body".into());
        }
        let needs_bindings = frame
            .contracts
            .iter()
            .any(|contract| !matches!(contract.kind, ContractKind::NoPanic))
            || self
                .specification(frame.instance)?
                .is_some_and(|spec| !spec.arguments.is_empty());
        if !needs_bindings {
            return Ok(());
        }
        let bindings = self.configured_bindings(&frame.body, arguments, frame.instance)?;
        if frame
            .contracts
            .iter()
            .any(|contract| matches!(contract.kind, ContractKind::Ensures))
        {
            check_reserved_names(&bindings)?;
        }
        for contract in frame.contracts.iter() {
            if matches!(contract.kind, ContractKind::Requires) {
                let predicate = contract
                    .predicate
                    .as_deref()
                    .ok_or("missing loop precondition")?;
                let safe = self.predicate(predicate, &bindings)?;
                exclude_failure(system, premise, &state.conditions, &safe);
                state.conditions.push(safe);
            }
        }
        Ok(())
    }

    fn loop_postconditions(
        &self,
        frame: &Frame<'tcx>,
        state: &mut State,
        result: &Value,
        system: &mut System,
        premise: &Atom,
    ) -> Result<(), String> {
        if !frame
            .contracts
            .iter()
            .any(|contract| matches!(contract.kind, ContractKind::Ensures))
        {
            return Ok(());
        }
        let mut bindings = self.configured_bindings(&frame.body, &frame.entry, frame.instance)?;
        check_reserved_names(&bindings)?;
        let final_arguments = frame
            .body
            .args_iter()
            .map(|local| self.local(state, local.as_usize()))
            .collect::<Result<Vec<_>, _>>()?;
        for (name, value) in
            self.configured_bindings(&frame.body, &final_arguments, frame.instance)?
        {
            bindings.insert(format!("final_{name}"), value);
        }
        bindings.insert("result".into(), result.clone());
        for contract in frame.contracts.iter() {
            if matches!(contract.kind, ContractKind::Ensures) {
                let predicate = contract
                    .predicate
                    .as_deref()
                    .ok_or("missing loop postcondition")?;
                let safe = self.predicate(predicate, &bindings)?;
                exclude_failure(system, premise, &state.conditions, &safe);
                state.conditions.push(safe);
            }
        }
        Ok(())
    }

    fn loop_input(&mut self, ty: Ty<'tcx>, depth: usize) -> Result<Value, String> {
        if depth >= MAX_INPUT_DEPTH {
            return Err("loop state type nesting limit reached".into());
        }
        match ty.kind() {
            ty::Bool => Ok(Value::Bool(self.fresh(Sort::Bool))),
            ty::Int(_) | ty::Uint(_) => {
                let (bits, signed) = self.integer_type(ty).ok_or("unknown loop integer width")?;
                Ok(Value::Int {
                    expression: self.fresh(Sort::BitVec(bits)),
                    bits,
                    signed,
                })
            }
            ty::Never => Ok(Value::Unit),
            ty::Tuple(fields) if fields.is_empty() => Ok(Value::Unit),
            ty::Tuple(fields) => Ok(Value::Tuple(
                fields
                    .iter()
                    .map(|ty| self.loop_input(ty, depth + 1))
                    .collect::<Result<_, _>>()?,
            )),
            ty::Array(element, count) if *element == self.tcx.types.u8 => {
                let count = count
                    .try_to_target_usize(self.tcx)
                    .ok_or("unknown loop array length")?;
                if count > 128 {
                    return Err("loop byte array exceeds 128 elements".into());
                }
                let bits = u32::from(self.tcx.sess.target.pointer_width);
                Ok(Value::Bytes {
                    length: Box::new(symbolic::integer(&self.terms, count as u128, bits, false)),
                    data: self.fresh(Sort::Array(
                        Box::new(Sort::BitVec(bits)),
                        Box::new(Sort::BitVec(8)),
                    )),
                })
            }
            // Other representations need their own inductive memory/validity model.
            other => Err(format!("loop induction unsupported state type {other:?}")),
        }
    }

    fn loop_operand_bounds(
        &self,
        state: &mut State,
        operand: &Operand<'tcx>,
        system: &mut System,
        premise: &Atom,
    ) -> Result<(), String> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                self.loop_place_bounds(state, *place, system, premise)
            }
            Operand::Constant(_) | Operand::RuntimeChecks(_) => Ok(()),
        }
    }

    fn loop_place_bounds(
        &self,
        state: &mut State,
        place: Place<'tcx>,
        system: &mut System,
        premise: &Atom,
    ) -> Result<(), String> {
        for (position, projection) in place.projection.iter().enumerate() {
            match projection {
                ProjectionElem::Field(..) | ProjectionElem::ConstantIndex { .. } => {}
                ProjectionElem::Index(index) => {
                    let prefix = Place {
                        local: place.local,
                        projection: self.tcx.mk_place_elems(&place.projection[..position]),
                    };
                    let Value::Bytes { length, .. } = self.place(state, prefix)? else {
                        return Err("loop index requires a fixed byte array".into());
                    };
                    let safe = symbolic::binary(
                        &self.terms,
                        "lt",
                        self.local(state, index.as_usize())?,
                        *length,
                    )?
                    .boolean()?;
                    let mut failure = state.conditions.clone();
                    failure.push(symbolic::not(&safe));
                    system.clauses.push(Clause {
                        premise: Some(premise.clone()),
                        conditions: failure,
                        conclusion: None,
                    });
                    state.conditions.push(safe);
                }
                other => return Err(format!("loop induction unsupported place {other:?}")),
            }
        }
        Ok(())
    }

    fn loop_statement(
        &mut self,
        id: DefId,
        body: &Body<'tcx>,
        state: &mut State,
        statement: &StatementKind<'tcx>,
        system: &mut System,
        premise: &Atom,
    ) -> Result<(), String> {
        match statement {
            StatementKind::StorageLive(_) | StatementKind::StorageDead(_) => Ok(()),
            StatementKind::Assign(assignment) => {
                let (place, value) = assignment.as_ref();
                self.loop_place_bounds(state, *place, system, premise)?;
                match value {
                    Rvalue::Use(operand, _)
                    | Rvalue::Cast(CastKind::IntToInt, operand, _)
                    | Rvalue::UnaryOp(UnOp::Not | UnOp::Neg, operand)
                    | Rvalue::Repeat(operand, _) => {
                        self.loop_operand_bounds(state, operand, system, premise)?;
                    }
                    Rvalue::Aggregate(_, operands) => {
                        for operand in operands {
                            self.loop_operand_bounds(state, operand, system, premise)?;
                        }
                    }
                    Rvalue::BinaryOp(op, operands)
                        if !matches!(
                            op,
                            BinOp::AddUnchecked
                                | BinOp::SubUnchecked
                                | BinOp::MulUnchecked
                                | BinOp::ShlUnchecked
                                | BinOp::ShrUnchecked
                                | BinOp::Cmp
                                | BinOp::Offset
                        ) =>
                    {
                        self.loop_operand_bounds(state, &operands.0, system, premise)?;
                        self.loop_operand_bounds(state, &operands.1, system, premise)?;
                    }
                    other => return Err(format!("loop induction unsupported rvalue {other:?}")),
                }
                // Whitelisted scalar operations cannot allocate, assume facts, or emit
                // independent obligations. Panics are represented by terminator clauses.
                self.statement(id, body, state, statement)
            }
            StatementKind::Nop
            | StatementKind::ConstEvalCounter
            | StatementKind::Coverage(_)
            | StatementKind::PlaceMention(_)
            | StatementKind::BackwardIncompatibleDropHint { .. } => Ok(()),
            other => Err(format!("loop induction unsupported statement {other:?}")),
        }
    }
}

#[derive(Clone)]
struct Resume<'tcx> {
    caller: usize,
    target: Option<BasicBlock>,
    destination: Place<'tcx>,
}

#[derive(Clone)]
struct Frame<'tcx> {
    instance: ty::Instance<'tcx>,
    body: std::rc::Rc<Body<'tcx>>,
    state: State,
    entry: Vec<Value>,
    ghosts: Vec<Term>,
    contracts: std::rc::Rc<[Contract]>,
    resume: Option<Resume<'tcx>>,
    base: usize,
}

impl Frame<'_> {
    fn parameters(&self, state: &State) -> Result<Vec<Term>, String> {
        let mut values = self.ghosts.clone();
        values.extend(flatten(state)?);
        Ok(values)
    }

    fn atom(&self, block: BasicBlock, state: &State) -> Result<Atom, String> {
        Ok(Atom {
            relation: self.base + block.as_usize(),
            arguments: self.parameters(state)?,
        })
    }

    fn entry_atom(&self, arguments: &[Value], captured: Vec<Term>) -> Result<Atom, String> {
        if arguments.len() != self.body.arg_count {
            return Err("inductive entry arguments do not match its MIR body".into());
        }
        let mut initial = self.state.clone();
        for (local, value) in self.body.args_iter().zip(arguments) {
            initial.locals[local.as_usize()] = Some(value.clone());
        }
        let mut parameters = captured;
        if self
            .contracts
            .iter()
            .any(|contract| matches!(contract.kind, ContractKind::Ensures))
        {
            for value in arguments {
                flatten_value(value, &mut parameters)?;
            }
        }
        parameters.extend(flatten(&initial)?);
        Ok(Atom {
            relation: self.base + START_BLOCK.as_usize(),
            arguments: parameters,
        })
    }
}

fn exclude_failure(system: &mut System, premise: &Atom, conditions: &[Term], safe: &Term) {
    let mut failure = conditions.to_vec();
    failure.push(symbolic::not(safe));
    system.clauses.push(Clause {
        premise: Some(premise.clone()),
        conditions: failure,
        conclusion: None,
    });
}

fn check_reserved_names(bindings: &BTreeMap<String, Value>) -> Result<(), String> {
    if bindings
        .keys()
        .any(|name| name == "result" || name.starts_with("final_"))
    {
        return Err("result and final_ argument names are reserved for loop postconditions".into());
    }
    Ok(())
}

fn flatten(state: &State) -> Result<Vec<Term>, String> {
    let mut result = Vec::new();
    for local in &state.locals {
        flatten_value(
            local.as_ref().ok_or("uninitialized inductive state")?,
            &mut result,
        )?;
    }
    Ok(result)
}

fn flatten_value(value: &Value, result: &mut Vec<Term>) -> Result<(), String> {
    match value {
        Value::Bool(term)
        | Value::Int {
            expression: term, ..
        } => result.push(term.clone()),
        Value::Bytes { length, data } => {
            flatten_value(length, result)?;
            result.push(data.clone());
        }
        Value::Tuple(fields) => {
            for field in fields {
                flatten_value(field, result)?;
            }
        }
        Value::Unit => {}
        other @ (Value::Float { .. }
        | Value::Adt { .. }
        | Value::Enum { .. }
        | Value::Cell { .. }
        | Value::Atomic { .. }
        | Value::Reference { .. }
        | Value::SliceIterator { .. }
        | Value::Elements(_)
        | Value::MetadataPointer(_)
        | Value::StaticText
        | Value::FormatArguments
        | Value::Function) => return Err(format!("unsupported inductive value {other:?}")),
    }
    Ok(())
}
