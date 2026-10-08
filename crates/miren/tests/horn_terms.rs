#![forbid(unsafe_code)]

use miren::smt::horn::{Atom, Clause, System};
use miren::smt::{Context, Op, Sort};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn solve(query: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.venv/bin/z3");
    let mut child = Command::new(path)
        .args(["-in", "-smt2", "-T:6"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(query.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn typed_horn_clauses_prove_a_masked_cycle_and_reject_a_changed_transition() {
    let context = Context::default();
    let word = context.symbol(0, Sort::BitVec(8)).unwrap();
    let atom = Atom {
        relation: 0,
        arguments: vec![word.clone()],
    };
    let mask = context.bit_vector(7, 8).unwrap();
    let next = context
        .apply(
            Op::BvAnd,
            &[
                context
                    .apply(
                        Op::BvAdd,
                        &[word.clone(), context.bit_vector(1, 8).unwrap()],
                    )
                    .unwrap(),
                mask.clone(),
            ],
        )
        .unwrap();
    let outside = context
        .apply(Op::BvUnsignedGt, &[word.clone(), mask])
        .unwrap();
    let mut system = System {
        relations: vec![vec![Sort::BitVec(8)]],
        clauses: vec![
            Clause {
                premise: None,
                conditions: vec![],
                conclusion: Some(Atom {
                    relation: 0,
                    arguments: vec![context.bit_vector(0, 8).unwrap()],
                }),
            },
            Clause {
                premise: Some(atom.clone()),
                conditions: vec![],
                conclusion: Some(Atom {
                    relation: 0,
                    arguments: vec![next],
                }),
            },
            Clause {
                premise: Some(atom),
                conditions: vec![outside],
                conclusion: None,
            },
        ],
    };
    let query = system.smt(&context, 200_000).unwrap();
    assert_eq!(solve(&query), "sat");
    assert!(system.smt(&context, query.len()).is_ok());
    assert!(system.smt(&context, query.len() - 1).is_err());
    system.clauses[1].conclusion.as_mut().unwrap().arguments[0] = context
        .apply(Op::BvAdd, &[word, context.bit_vector(1, 8).unwrap()])
        .unwrap();
    assert_eq!(solve(&system.smt(&context, 200_000).unwrap()), "unsat");
}

#[test]
fn horn_bindings_reject_invalid_contexts_arities_and_non_boolean_guards() {
    let context = Context::default();
    let mut system = System {
        relations: vec![vec![Sort::Bool]],
        clauses: vec![Clause {
            premise: None,
            conditions: vec![],
            conclusion: Some(Atom {
                relation: 0,
                arguments: vec![context.boolean(true)],
            }),
        }],
    };
    assert_eq!(solve(&system.smt(&context, 1000).unwrap()), "sat");
    system.clauses[0]
        .conditions
        .push(context.bit_vector(0, 8).unwrap());
    assert!(system.smt(&context, 1000).is_err());
    system.clauses[0].conditions.clear();
    system.clauses[0].conclusion.as_mut().unwrap().arguments[0] = Context::default().boolean(true);
    assert!(system.smt(&context, 1000).is_err());
    system.clauses[0]
        .conclusion
        .as_mut()
        .unwrap()
        .arguments
        .clear();
    assert!(system.smt(&context, 1000).is_err());
    system.clauses[0].conclusion.as_mut().unwrap().relation = 1;
    assert!(system.smt(&context, 1000).is_err());
}
