use super::*;
use mir_check::smt::horn::{Atom, Clause, System};
use rustc_middle::mir::BasicBlock;
use std::collections::BTreeSet;

const MAX_BLOCKS: usize = 256;
const MAX_STATE_VALUES: usize = 512;

impl<'tcx> Engine<'tcx> {
    pub(super) fn has_cycle(&self, body: &Body<'tcx>) -> bool {
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
        let id = instance.def_id();
        let body = self.instantiated_body(instance)?;
        if arguments.len() != body.arg_count {
            return Err("inductive root arguments do not match its MIR body".into());
        }
        if !memory.is_empty() {
            return Err("loop induction does not yet model mutable or interior storage".into());
        }
        if self
            .configured_contracts(instance)?
            .iter()
            .any(|contract| matches!(contract.kind, ContractKind::Ensures))
        {
            return Err("loop induction does not yet check function postconditions".into());
        }
        if body.basic_blocks.len() > MAX_BLOCKS {
            return Err("loop induction exceeds its 256-block translation budget".into());
        }
        let mut locals = Vec::new();
        for declaration in &body.local_decls {
            locals.push(Some(self.loop_input(declaration.ty, 0)?));
        }
        let canonical = State {
            locals,
            conditions: Vec::new(),
            addresses: vec![None; body.local_decls.len()],
            memory: Vec::new(),
        };
        let parameters = flatten(&canonical)?;
        if parameters.len() > MAX_STATE_VALUES {
            return Err("loop induction exceeds its 512-scalar state budget".into());
        }
        let sorts = parameters
            .iter()
            .map(|term| term.sort().clone())
            .collect::<Vec<_>>();
        let mut system = System {
            relations: vec![sorts; body.basic_blocks.len()],
            clauses: Vec::new(),
        };
        let mut initial = canonical.clone();
        for (local, argument) in body.args_iter().zip(arguments) {
            initial.locals[local.as_usize()] = Some(argument);
        }
        system.clauses.push(Clause {
            premise: None,
            conditions,
            conclusion: Some(atom(START_BLOCK, &initial)?),
        });
        let mut pending = vec![START_BLOCK];
        let mut visited = BTreeSet::new();
        while let Some(block) = pending.pop() {
            if !visited.insert(block) {
                continue;
            }
            if self.started.elapsed().as_secs() >= MAX_ROOT_SECONDS {
                return Err("loop translation exceeded the root time budget".into());
            }
            let mut state = canonical.clone();
            let premise = atom(block, &canonical)?;
            for statement in &body.basic_blocks[block].statements {
                self.loop_statement(
                    id,
                    &body,
                    &mut state,
                    &statement.kind,
                    &mut system,
                    &premise,
                )
                .map_err(|reason| format!("loop induction bb{}: {reason}", block.as_usize()))?;
            }
            let terminator = body.basic_blocks[block].terminator();
            let mut edges = Vec::new();
            match &terminator.kind {
                TerminatorKind::Goto { target } => edges.push((*target, state)),
                TerminatorKind::SwitchInt { discr, targets } => {
                    self.loop_operand_bounds(&mut state, discr, &mut system, &premise)?;
                    let value = self.operand(id, &body, &state, discr)?;
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
                        let condition = self.operand(id, &body, &state, cond)?.boolean()?;
                        let safe = if *expected {
                            condition
                        } else {
                            symbolic::not(&condition)
                        };
                        let mut failure = state.conditions.clone();
                        failure.push(symbolic::not(&safe));
                        system.clauses.push(Clause {
                            premise: Some(premise.clone()),
                            conditions: failure,
                            conclusion: None,
                        });
                        state.conditions.push(safe);
                    }
                    edges.push((*target, state));
                }
                TerminatorKind::Return => {}
                TerminatorKind::Call { func, .. } => {
                    let ty::FnDef(callee, _) = *func.ty(&body.local_decls, self.tcx).kind() else {
                        return Err("loop induction cannot resolve an indirect call".into());
                    };
                    if !super::super::identity::is_panic_call(self.tcx, callee)
                        && !self.is_core_panic_helper(callee)
                    {
                        return Err(format!(
                            "loop induction needs a transition model for call {}",
                            self.tcx.def_path_str(callee)
                        ));
                    }
                    system.clauses.push(Clause {
                        premise: Some(premise.clone()),
                        conditions: state.conditions,
                        conclusion: None,
                    });
                }
                TerminatorKind::Unreachable => system.clauses.push(Clause {
                    premise: Some(premise.clone()),
                    conditions: state.conditions,
                    conclusion: None,
                }),
                other => return Err(format!("loop induction unsupported terminator {other:?}")),
            }
            for (target, state) in edges {
                system.clauses.push(Clause {
                    premise: Some(premise.clone()),
                    conditions: state.conditions.clone(),
                    conclusion: Some(atom(target, &state)?),
                });
                pending.push(target);
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
        self.proof.analyzed_bodies.push(format!(
            "{} {:?}",
            self.tcx.def_path_str(id),
            instance.args
        ));
        self.proof.obligations.push(Obligation {
            function: self.tcx.def_path_str(id),
            source: super::super::source(self.tcx, self.tcx.def_span(id)),
            kind: ObligationKind::Validity,
            detail,
            status,
            query: Some(query),
            model: None,
        });
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

fn atom(block: BasicBlock, state: &State) -> Result<Atom, String> {
    Ok(Atom {
        relation: block.as_usize(),
        arguments: flatten(state)?,
    })
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
        other => return Err(format!("unsupported inductive value {other:?}")),
    }
    Ok(())
}
