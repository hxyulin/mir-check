//! Typed Horn clauses. Relations and universal bindings are printed at this boundary.

use super::{Context, Sort, Term, printer};
use std::collections::BTreeSet;

#[derive(Clone)]
pub struct Atom {
    pub relation: usize,
    pub arguments: Vec<Term>,
}

pub struct Clause {
    pub premise: Option<Atom>,
    pub conditions: Vec<Term>,
    /// A missing conclusion excludes the premise: it denotes a safety failure.
    pub conclusion: Option<Atom>,
}

pub struct System {
    pub relations: Vec<Vec<Sort>>,
    pub clauses: Vec<Clause>,
}

impl System {
    pub fn smt(&self, context: &Context, max_bytes: usize) -> Result<String, String> {
        let mut output = String::new();
        append(
            &mut output,
            "(set-logic HORN)\n(set-option :fp.engine spacer)\n\
             (set-option :fp.xform.bit_blast false)\n(set-option :pp.max_indent 0)\n\
             (set-option :timeout 5000)\n",
            max_bytes,
        )?;
        for (index, sorts) in self.relations.iter().enumerate() {
            append(&mut output, &format!("(declare-fun b{index} ("), max_bytes)?;
            for sort in sorts {
                sort.validate()?;
                append(&mut output, &format!("{} ", printer::sort(sort)), max_bytes)?;
            }
            append(&mut output, ") Bool)\n", max_bytes)?;
        }
        for clause in &self.clauses {
            let mut symbols = BTreeSet::new();
            for condition in &clause.conditions {
                if condition.sort() != &Sort::Bool || !condition.belongs_to(context) {
                    return Err("Horn conditions require Boolean terms in this context".into());
                }
                symbols.extend(condition.symbols());
            }
            for atom in clause.premise.iter().chain(&clause.conclusion) {
                let sorts = self
                    .relations
                    .get(atom.relation)
                    .ok_or("Horn clause uses an undeclared relation")?;
                if sorts.len() != atom.arguments.len() {
                    return Err("Horn relation arity mismatch".into());
                }
                for (sort, argument) in sorts.iter().zip(&atom.arguments) {
                    if argument.sort() != sort || !argument.belongs_to(context) {
                        return Err("Horn relation argument type or context mismatch".into());
                    }
                    symbols.extend(argument.symbols());
                }
            }
            append(&mut output, "(assert ", max_bytes)?;
            if !symbols.is_empty() {
                append(&mut output, "(forall (", max_bytes)?;
                for symbol in &symbols {
                    let pool = context.0.borrow();
                    let sort = pool.symbols.get(symbol).ok_or("unbound Horn symbol")?;
                    append(
                        &mut output,
                        &format!("(v{symbol} {})", printer::sort(sort)),
                        max_bytes,
                    )?;
                }
                append(&mut output, ") ", max_bytes)?;
            }
            append(&mut output, "(=> (and true", max_bytes)?;
            if let Some(atom) = &clause.premise {
                append(&mut output, " ", max_bytes)?;
                self.atom(&mut output, atom, max_bytes)?;
            }
            for condition in &clause.conditions {
                append(&mut output, " ", max_bytes)?;
                term(&mut output, condition, max_bytes)?;
            }
            append(&mut output, ") ", max_bytes)?;
            if let Some(atom) = &clause.conclusion {
                self.atom(&mut output, atom, max_bytes)?;
            } else {
                append(&mut output, "false", max_bytes)?;
            }
            append(&mut output, ")", max_bytes)?;
            if !symbols.is_empty() {
                append(&mut output, ")", max_bytes)?;
            }
            append(&mut output, ")\n", max_bytes)?;
        }
        append(&mut output, "(check-sat)\n", max_bytes)?;
        Ok(output)
    }

    fn atom(&self, output: &mut String, atom: &Atom, max_bytes: usize) -> Result<(), String> {
        if atom.arguments.is_empty() {
            return append(output, &format!("b{}", atom.relation), max_bytes);
        }
        append(output, &format!("(b{}", atom.relation), max_bytes)?;
        for argument in &atom.arguments {
            append(output, " ", max_bytes)?;
            term(output, argument, max_bytes)?;
        }
        append(output, ")", max_bytes)
    }
}

fn term(output: &mut String, value: &Term, max_bytes: usize) -> Result<(), String> {
    let printed = value.smt(max_bytes.saturating_sub(output.len()))?;
    append(output, &printed, max_bytes)
}

fn append(output: &mut String, text: &str, max_bytes: usize) -> Result<(), String> {
    if text.len() > max_bytes.saturating_sub(output.len()) {
        return Err("Horn query size limit reached".into());
    }
    output.push_str(text);
    Ok(())
}
