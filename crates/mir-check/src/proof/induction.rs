use super::*;
use mir_check::smt::horn::{Atom, Clause, System};
use rustc_middle::mir::BasicBlock;
use std::collections::BTreeSet;

mod enums;
mod ranges;
mod slices;
mod storage;
mod templates;
mod writes;
use storage::same_shape;

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
        let mut system = System {
            relations: Vec::new(),
            clauses: Vec::new(),
        };
        let (arguments, memory) = self.loop_root_storage(instance, arguments, memory)?;
        let root = self.loop_frame(instance, Vec::new(), None, &mut system, &arguments, &memory)?;
        let initial = self.loop_entry_atom(&root, &arguments, &memory, Vec::new())?;
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
            if self.started.elapsed().as_secs() >= self.limits.root_timeout_secs {
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
            self.loop_normalize(&frame, &mut state)?;
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
                    for local in body.args_iter() {
                        let value = self.local(&state, local.as_usize())?;
                        self.loop_reference_bounds(&mut state, &value, &mut system, &premise)?;
                    }
                    self.loop_reference_bounds(&mut state, &result, &mut system, &premise)?;
                    self.loop_postconditions(&frame, &mut state, &result, &mut system, &premise)?;
                    if let Some(resume) = &frame.resume {
                        let target = resume
                            .target
                            .ok_or("a diverging call unexpectedly returned")?;
                        let caller = &frames[resume.caller];
                        self.validate_frame_escape(&result, &state, caller.state.memory.len())?;
                        let mut restored = caller.state.clone();
                        restored
                            .memory
                            .clone_from_slice(&state.memory[..caller.state.memory.len()]);
                        restored.conditions = state.conditions;
                        self.loop_place_bounds(
                            &mut restored,
                            resume.destination,
                            &mut system,
                            &premise,
                        )?;
                        self.loop_write(&mut restored, resume.destination, result)?;
                        self.loop_normalize(caller, &mut restored)?;
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
                    if depth >= self.limits.max_call_depth {
                        return Err(format!(
                            "inductive call graph exceeds the {}-frame depth limit",
                            self.limits.max_call_depth
                        ));
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
                    for value in &values {
                        self.loop_reference_bounds(&mut state, value, &mut system, &premise)?;
                    }
                    let modeled =
                        self.loop_slice_call(callee, &values, &mut state, &mut system, &premise)?;
                    let modeled = match modeled {
                        Some(results) => Some(results),
                        None => self.loop_range_call(callee, &values, &state)?,
                    };
                    if let Some(results) = modeled {
                        let target = target.ok_or("inductive iterator call has no return edge")?;
                        for (mut continuation, value) in results {
                            self.loop_place_bounds(
                                &mut continuation,
                                *destination,
                                &mut system,
                                &premise,
                            )?;
                            self.loop_write(&mut continuation, *destination, value)?;
                            edges.push((target, continuation));
                        }
                    } else {
                        let inherited = frame.captured(&frame.state)?;
                        let child = self.loop_frame(
                            callee,
                            inherited,
                            Some(Resume {
                                caller: frame_index,
                                target: *target,
                                destination: *destination,
                            }),
                            &mut system,
                            &values,
                            &state.memory,
                        )?;
                        self.loop_preconditions(
                            &child,
                            &values,
                            &mut state,
                            &mut system,
                            &premise,
                        )?;
                        let captured = frame.captured(&state)?;
                        let entry =
                            self.loop_entry_atom(&child, &values, &state.memory, captured)?;
                        system.clauses.push(Clause {
                            premise: Some(premise.clone()),
                            conditions: state.conditions,
                            conclusion: Some(entry),
                        });
                        let child_index = frames.len();
                        frames.push(child);
                        pending.push((child_index, START_BLOCK));
                    }
                }
                TerminatorKind::Unreachable => exclude_failure(
                    &mut system,
                    &premise,
                    &state.conditions,
                    &self.terms.boolean(false),
                ),
                other => return Err(format!("loop induction unsupported terminator {other:?}")),
            }
            for (target, mut state) in edges {
                self.loop_normalize(&frame, &mut state)?;
                system.clauses.push(Clause {
                    premise: Some(premise.clone()),
                    conditions: state.conditions.clone(),
                    conclusion: Some(frame.atom(target, &state)?),
                });
                pending.push((frame_index, target));
            }
        }
        let query = system.smt_with_timeout(
            &self.terms,
            self.limits.max_query_bytes,
            self.limits.solver_timeout_ms,
        )?;
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
        arguments: &[Value],
        incoming_memory: &[Option<Value>],
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
        self.input_values = 0;
        let state = self
            .loop_storage(&body, arguments, incoming_memory)
            .map_err(|reason| {
                format!(
                    "loop induction {}: {reason}",
                    self.tcx.def_path_str(instance.def_id())
                )
            })?;
        let mut entry = Vec::new();
        if contracts
            .iter()
            .any(|contract| matches!(contract.kind, ContractKind::Ensures))
        {
            let values = body
                .args_iter()
                .map(|local| self.local(&state, local.as_usize()))
                .collect::<Result<Vec<_>, _>>()?;
            let snapshots = values
                .iter()
                .map(|value| self.loop_template_snapshot(value, &state))
                .collect::<Result<Vec<_>, _>>()?;
            for snapshot in &snapshots {
                entry.push(self.loop_fresh_value(snapshot)?);
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
        let snapshots = arguments
            .iter()
            .map(|value| self.loop_template_snapshot(value, &frame.state))
            .collect::<Result<Vec<_>, _>>()?;
        let bindings = self.configured_bindings(&frame.body, &snapshots, frame.instance)?;
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
            let result = self.local(&frame.state, 0)?;
            post_bindings.insert(
                "result".into(),
                self.loop_template_snapshot(&result, &frame.state)?,
            );
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
        let snapshots = arguments
            .iter()
            .map(|value| self.loop_template_snapshot(value, state))
            .collect::<Result<Vec<_>, _>>()?;
        let bindings = self.configured_bindings(&frame.body, &snapshots, frame.instance)?;
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
        let snapshots = final_arguments
            .iter()
            .map(|value| self.loop_template_snapshot(value, state))
            .collect::<Result<Vec<_>, _>>()?;
        for (name, value) in self.configured_bindings(&frame.body, &snapshots, frame.instance)? {
            bindings.insert(format!("final_{name}"), value);
        }
        bindings.insert("result".into(), self.loop_template_snapshot(result, state)?);
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
        self.input_values += 1;
        if self.input_values > MAX_STATE_VALUES {
            return Err("loop state exceeds its 512-node type shape budget".into());
        }
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
            ty::Array(element, count) => {
                let count = count
                    .try_to_target_usize(self.tcx)
                    .ok_or("unknown inductive scalar array length")?;
                if count > 16 {
                    return Err("inductive scalar arrays exceed 16 elements".into());
                }
                if self.integer_type(*element).is_none() && !element.is_bool() {
                    return Err("inductive element arrays require integers or Booleans".into());
                }
                Ok(Value::Elements(
                    (0..count)
                        .map(|_| self.loop_input(*element, depth + 1))
                        .collect::<Result<_, _>>()?,
                ))
            }
            ty::Adt(def, args)
                if def.is_struct()
                    && ty.is_freeze(self.tcx, ty::TypingEnv::fully_monomorphized())
                    && !ty.needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized()) =>
            {
                let fields = def
                    .non_enum_variant()
                    .fields
                    .iter()
                    .map(|field| {
                        let field_ty = self
                            .tcx
                            .try_normalize_erasing_regions(
                                ty::TypingEnv::fully_monomorphized(),
                                field.ty(self.tcx, args),
                            )
                            .map_err(|error| {
                                format!("loop field normalization failed: {error:?}")
                            })?;
                        self.loop_input(field_ty, depth + 1)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.constructed(ty, 0, fields)
            }
            ty::Adt(def, _)
                if def.is_enum()
                    && ty.is_freeze(self.tcx, ty::TypingEnv::fully_monomorphized())
                    && !ty.needs_drop(self.tcx, ty::TypingEnv::fully_monomorphized()) =>
            {
                self.loop_enum(ty, depth)
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
                ProjectionElem::Deref => {
                    let prefix = Place {
                        local: place.local,
                        projection: self.tcx.mk_place_elems(&place.projection[..position]),
                    };
                    let reference = self.place(state, prefix)?;
                    if let Value::Reference {
                        allocation,
                        projection,
                        ..
                    } = reference
                    {
                        self.loop_reference_bounds(
                            state,
                            &Value::Reference {
                                allocation,
                                projection,
                                mutable: false,
                            },
                            system,
                            premise,
                        )?;
                    }
                }
                ProjectionElem::Field(..) | ProjectionElem::ConstantIndex { .. } => {}
                ProjectionElem::Downcast(_, expected) => {
                    let prefix = Place {
                        local: place.local,
                        projection: self.tcx.mk_place_elems(&place.projection[..position]),
                    };
                    let value = self.place(state, prefix)?;
                    if let Value::Enum {
                        discriminant,
                        variants,
                        ..
                    } = value
                    {
                        let Value::Adt {
                            discriminant: tag, ..
                        } = variants
                            .get(expected.as_usize())
                            .ok_or("loop enum payload missing")?
                        else {
                            return Err("loop enum payload needs an ADT".into());
                        };
                        let (_, bits, signed) = discriminant.integer()?;
                        let safe = symbolic::binary(
                            &self.terms,
                            "eq",
                            *discriminant,
                            symbolic::integer(&self.terms, *tag, bits, signed),
                        )?
                        .boolean()?;
                        exclude_failure(system, premise, &state.conditions, &safe);
                        state.conditions.push(safe);
                    }
                }
                ProjectionElem::Index(index) => {
                    let prefix = Place {
                        local: place.local,
                        projection: self.tcx.mk_place_elems(&place.projection[..position]),
                    };
                    let storage = self.place(state, prefix)?;
                    let length = self.loop_slice_length(&storage)?;
                    let safe = symbolic::binary(
                        &self.terms,
                        "lt",
                        self.local(state, index.as_usize())?,
                        length,
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
                    Rvalue::Ref(_, BorrowKind::Shared | BorrowKind::Mut { .. }, borrowed) => {
                        self.loop_place_bounds(state, *borrowed, system, premise)?;
                        let mutable = matches!(value, Rvalue::Ref(_, BorrowKind::Mut { .. }, _));
                        let reference = self
                            .loop_alias(state, *borrowed, mutable)?
                            .ok_or("inductive borrow has no stable target")?;
                        return self.loop_write(state, *place, reference);
                    }
                    Rvalue::RawPtr(rustc_middle::mir::RawPtrKind::FakeForPtrMetadata, borrowed) => {
                        self.loop_place_bounds(state, *borrowed, system, premise)?;
                    }
                    Rvalue::Cast(CastKind::PointerCoercion(..), operand, _) => {
                        self.loop_operand_bounds(state, operand, system, premise)?;
                    }
                    Rvalue::Use(operand, _)
                    | Rvalue::Cast(CastKind::IntToInt, operand, _)
                    | Rvalue::UnaryOp(UnOp::Not | UnOp::Neg | UnOp::PtrMetadata, operand)
                    | Rvalue::Repeat(operand, _) => {
                        self.loop_operand_bounds(state, operand, system, premise)?;
                    }
                    Rvalue::Discriminant(place) => {
                        self.loop_place_bounds(state, *place, system, premise)?;
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
                let value = self.rvalue(id, body, state, value)?;
                self.loop_write(state, *place, value)
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
    fn captured(&self, state: &State) -> Result<Vec<Term>, String> {
        self.check_shape(state)?;
        let mut values = self.ghosts.clone();
        values.extend(flatten(state)?);
        Ok(values)
    }

    fn parameters(&self, state: &State) -> Result<Vec<Term>, String> {
        let mut values = self.captured(state)?;
        for cell in &state.memory {
            flatten_value(
                cell.as_ref().ok_or("unavailable inductive memory cell")?,
                &mut values,
            )?;
        }
        Ok(values)
    }

    fn check_shape(&self, state: &State) -> Result<(), String> {
        if state.addresses != self.state.addresses || state.memory.len() != self.state.memory.len()
        {
            return Err("inductive allocation layout changed".into());
        }
        for (expected, value) in self.state.locals.iter().zip(&state.locals) {
            same_shape(expected.as_ref(), value.as_ref())?;
        }
        for (expected, value) in self.state.memory.iter().zip(&state.memory) {
            same_shape(expected.as_ref(), value.as_ref())?;
        }
        Ok(())
    }

    fn atom(&self, block: BasicBlock, state: &State) -> Result<Atom, String> {
        Ok(Atom {
            relation: self.base + block.as_usize(),
            arguments: self.parameters(state)?,
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
    for (index, local) in state.locals.iter().enumerate() {
        if state.addresses[index].is_none() {
            flatten_value(
                local.as_ref().ok_or("uninitialized inductive state")?,
                &mut result,
            )?;
        }
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
        Value::Enum {
            discriminant,
            variants,
            ..
        } => {
            flatten_value(discriminant, result)?;
            for variant in variants {
                flatten_value(variant, result)?;
            }
        }
        Value::Adt { fields, .. } => {
            for (_, field) in fields {
                flatten_value(field, result)?;
            }
        }
        Value::Elements(fields) | Value::Tuple(fields) => {
            for field in fields {
                flatten_value(field, result)?;
            }
        }
        Value::Reference { projection, .. } => {
            for part in projection {
                match part {
                    symbolic::MemoryProjection::Field(_)
                    | symbolic::MemoryProjection::Variant(_) => {}
                    symbolic::MemoryProjection::Index(index) => flatten_value(index, result)?,
                    symbolic::MemoryProjection::Slice { .. }
                    | symbolic::MemoryProjection::Chunks { .. } => {
                        return Err("inductive reference projection is not a static field".into());
                    }
                }
            }
        }
        Value::SliceIterator {
            source,
            front,
            back,
            ..
        } => {
            flatten_value(source, result)?;
            flatten_value(front, result)?;
            flatten_value(back, result)?;
        }
        Value::MetadataPointer(length) => flatten_value(length, result)?,
        Value::Unit => {}
        Value::Input(_) => {
            return Err("unmaterialized lazy input is unsupported by induction".into());
        }
        other @ (Value::Float { .. }
        | Value::Cell { .. }
        | Value::Atomic { .. }
        | Value::StaticText
        | Value::FormatArguments
        | Value::Function) => return Err(format!("unsupported inductive value {other:?}")),
    }
    Ok(())
}
