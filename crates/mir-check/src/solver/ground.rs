use std::collections::BTreeSet;

const MAX_BYTES: usize = 200_000;
const MAX_TOKENS: usize = 16_384;
const MAX_DEPTH: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Value {
    Bool(bool),
    BitVec { value: u128, bits: u32 },
}

struct Parser<'a> {
    tokens: Vec<&'a str>,
    position: usize,
}

pub(super) fn feasible(query: &str) -> Option<bool> {
    let mut parser = Parser::new(query)?;
    parser.expect("(")?;
    parser.expect("set-logic")?;
    parser.expect("ALL")?;
    parser.expect(")")?;
    let mut declarations = BTreeSet::new();
    let mut options = 0;
    let mut assertions_started = false;
    let mut answer = true;
    loop {
        parser.expect("(")?;
        match parser.next()? {
            "set-option" if !assertions_started && declarations.is_empty() => {
                let option = match parser.next()? {
                    ":timeout" => {
                        decimal(parser.next()?)?
                            .try_into()
                            .ok()
                            .filter(|v: &u32| *v > 0)?;
                        1
                    }
                    ":pp.bv-literals" => {
                        parser.expect("false")?;
                        2
                    }
                    _ => return None,
                };
                if options & option != 0 {
                    return None;
                }
                options |= option;
            }
            command @ ("declare-fun" | "declare-const") if !assertions_started && options == 3 => {
                let name = parser.next()?;
                if !identifier(name) || !declarations.insert(name) {
                    return None;
                }
                if command == "declare-fun" {
                    parser.expect("(")?;
                    parser.expect(")")?;
                }
                parser.sort()?;
            }
            "assert" if options == 3 => {
                assertions_started = true;
                let Value::Bool(value) = parser.expression(0)? else {
                    return None;
                };
                answer &= value;
            }
            "check-sat" if options == 3 => {
                parser.expect(")")?;
                return (parser.position == parser.tokens.len()).then_some(answer);
            }
            _ => return None,
        }
        parser.expect(")")?;
    }
}

fn identifier(value: &str) -> bool {
    value.strip_prefix('v').is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn decimal(value: &str) -> Option<u128> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse().ok()
}

fn width(value: &str) -> Option<u32> {
    let bits: u32 = decimal(value)?.try_into().ok()?;
    (1..=128).contains(&bits).then_some(bits)
}

fn mask(bits: u32) -> u128 {
    u128::MAX >> (128 - bits)
}

impl<'a> Parser<'a> {
    fn new(query: &'a str) -> Option<Self> {
        if query.len() > MAX_BYTES {
            return None;
        }
        let mut tokens = Vec::new();
        let mut start = None;
        for (position, character) in query.char_indices() {
            if matches!(character, ' ' | '\t' | '\r' | '\n') || matches!(character, '(' | ')') {
                if let Some(start) = start.take() {
                    tokens.push(&query[start..position]);
                }
                if matches!(character, '(' | ')') {
                    tokens.push(&query[position..position + 1]);
                }
            } else if matches!(character, ';' | '"' | '|') {
                return None;
            } else {
                start.get_or_insert(position);
            }
            if tokens.len() > MAX_TOKENS {
                return None;
            }
        }
        if let Some(start) = start {
            tokens.push(&query[start..]);
        }
        (tokens.len() <= MAX_TOKENS).then_some(Self {
            tokens,
            position: 0,
        })
    }

    fn next(&mut self) -> Option<&'a str> {
        let token = self.tokens.get(self.position).copied()?;
        self.position += 1;
        Some(token)
    }

    fn expect(&mut self, expected: &str) -> Option<()> {
        (self.next()? == expected).then_some(())
    }

    fn sort(&mut self) -> Option<()> {
        match self.next()? {
            "Bool" => Some(()),
            "(" => {
                self.expect("_")?;
                self.expect("BitVec")?;
                width(self.next()?)?;
                self.expect(")")
            }
            _ => None,
        }
    }

    fn expression(&mut self, depth: usize) -> Option<Value> {
        if depth >= MAX_DEPTH {
            return None;
        }
        match self.next()? {
            "true" => Some(Value::Bool(true)),
            "false" => Some(Value::Bool(false)),
            "(" => {
                let operator = self.next()?;
                let value = if operator == "(" {
                    self.expect("_")?;
                    let extension = self.next()?;
                    if !matches!(extension, "sign_extend" | "zero_extend") {
                        return None;
                    }
                    let extra: u32 = decimal(self.next()?)?.try_into().ok()?;
                    self.expect(")")?;
                    let Value::BitVec { value, bits } = self.expression(depth + 1)? else {
                        return None;
                    };
                    let extended = bits.checked_add(extra).filter(|bits| *bits <= 128)?;
                    let value = if extension == "sign_extend" && value & (1_u128 << (bits - 1)) != 0
                    {
                        value | (mask(extended) ^ mask(bits))
                    } else {
                        value
                    };
                    Value::BitVec {
                        value,
                        bits: extended,
                    }
                } else if operator == "_" {
                    let value = decimal(self.next()?.strip_prefix("bv")?)?;
                    let bits = width(self.next()?)?;
                    if value > mask(bits) {
                        return None;
                    }
                    Value::BitVec { value, bits }
                } else {
                    let left = self.expression(depth + 1)?;
                    match operator {
                        "not" => match left {
                            Value::Bool(value) => Value::Bool(!value),
                            Value::BitVec { .. } => return None,
                        },
                        "bvnot" | "bvneg" => {
                            let Value::BitVec { value, bits } = left else {
                                return None;
                            };
                            let value = if operator == "bvnot" {
                                !value
                            } else {
                                0_u128.wrapping_sub(value)
                            } & mask(bits);
                            Value::BitVec { value, bits }
                        }
                        _ => {
                            let right = self.expression(depth + 1)?;
                            binary(operator, left, right)?
                        }
                    }
                };
                self.expect(")")?;
                Some(value)
            }
            _ => None,
        }
    }
}

fn binary(operator: &str, left: Value, right: Value) -> Option<Value> {
    match (left, right) {
        (Value::Bool(left), Value::Bool(right)) => match operator {
            "=" => Some(Value::Bool(left == right)),
            "and" => Some(Value::Bool(left && right)),
            "or" => Some(Value::Bool(left || right)),
            _ => None,
        },
        (
            Value::BitVec { value: left, bits },
            Value::BitVec {
                value: right,
                bits: right_bits,
            },
        ) if bits == right_bits => {
            let comparison = match operator {
                "=" => Some(left == right),
                "bvult" => Some(left < right),
                "bvule" => Some(left <= right),
                "bvugt" => Some(left > right),
                "bvuge" => Some(left >= right),
                "bvslt" | "bvsle" | "bvsgt" | "bvsge" => {
                    let sign = 1_u128 << (bits - 1);
                    let left = left ^ sign;
                    let right = right ^ sign;
                    Some(match operator {
                        "bvslt" => left < right,
                        "bvsle" => left <= right,
                        "bvsgt" => left > right,
                        "bvsge" => left >= right,
                        _ => return None,
                    })
                }
                _ => None,
            };
            if let Some(value) = comparison {
                return Some(Value::Bool(value));
            }
            let value = match operator {
                "bvadd" => left.wrapping_add(right),
                "bvsub" => left.wrapping_sub(right),
                "bvmul" => left.wrapping_mul(right),
                "bvand" => left & right,
                "bvor" => left | right,
                "bvxor" => left ^ right,
                _ => return None,
            } & mask(bits);
            Some(Value::BitVec { value, bits })
        }
        (Value::Bool(_), Value::BitVec { .. })
        | (Value::BitVec { .. }, Value::Bool(_))
        | (Value::BitVec { .. }, Value::BitVec { .. }) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_DEPTH, MAX_TOKENS, feasible, mask};
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    fn query(assertions: &str) -> String {
        format!(
            "(set-logic ALL)\n(set-option :timeout 5000)\n\
             (set-option :pp.bv-literals false)\n{assertions}\n(check-sat)\n"
        )
    }

    #[test]
    fn unsupported_or_malformed_scripts_never_decide_a_query() {
        for assertion in [
            "(assert v0)",
            "(declare-fun v0 () Bool) (assert v0)",
            "(declare-fun v0 () Bool) (declare-fun v0 () Bool) (assert true)",
            "(declare-fun v0 () (_ FloatingPoint 8 24)) (assert true)",
            "(declare-fun v0 () (Array Bool Bool)) (assert true)",
            "(assert (= (_ bv256 8) (_ bv0 8)))",
            "(assert (= (_ bv1 0) (_ bv1 0)))",
            "(assert (= (_ bv1 129) (_ bv1 129)))",
            "(assert (= (_ bv01 8) (_ bv1 8)))",
            "(assert (= (_ bv1 8) (_ bv1 16)))",
            "(assert (bvadd true false))",
            "(assert (not (_ bv1 8)))",
            "(assert (bvneg false))",
            "(assert (and false v0))",
            "(assert (or true v0))",
            "(assert false) (assert (unsupported true))",
            "(assert (and true true true))",
            "(assert (not true false))",
            "(assert true) (push 1)",
            "(assert true) (check-sat) (assert false)",
            "(assert true) (declare-fun v0 () Bool)",
            "(assert true) ; comment",
            "(declare-fun not () Bool) (assert true)",
            "(assert true) (set-logic ALL)",
            "(assert (= (_ bv340282366920938463463374607431768211456 128) (_ bv0 128)))",
            "(assert true) (get-model)",
            "(assert #b1)",
            "(assert (= ((_ sign_extend 1) (_ bv1 128)) (_ bv1 128)))",
            "(assert (= ((_ zero_extend 4294967296) (_ bv0 8)) (_ bv0 8)))",
            "(assert (= ((_ sign_extend 1) true) true))",
            "(assert (= ((_ extract 0 0) (_ bv1 8)) (_ bv1 1)))",
            "(assert (fp.isNaN v0))",
            "(assert true",
            "(assert true))",
        ] {
            assert_eq!(feasible(&query(assertion)), None, "{assertion}");
        }
        assert_eq!(
            feasible(&query("(assert true)").replace("ALL", "QF_BV")),
            None
        );
        assert_eq!(
            feasible(&query("(assert true)").replace(":timeout", ":unknown")),
            None
        );
        assert_eq!(
            feasible(&query("(assert true)").replace("5000", "-1")),
            None
        );
        assert_eq!(
            feasible(&query("(assert true)").replace("5000", "4294967296")),
            None
        );
        assert_eq!(
            feasible(&query("(assert true)").replace(" ", "\u{000b}")),
            None
        );
        assert_eq!(
            feasible(&query("(assert true)").replace("(check-sat)", "")),
            None
        );
        assert_eq!(
            feasible(&query("(assert true)").replace("(check-sat)", "(check-sat) x")),
            None
        );
    }

    #[test]
    fn declarations_and_all_assertions_are_checked_before_deciding() {
        assert_eq!(feasible(&query("")), Some(true));
        assert_eq!(
            feasible(&query("(declare-const v0 (_ BitVec 8)) (assert true)")),
            Some(true)
        );
        assert_eq!(
            feasible(&query("(assert true) (assert false)")),
            Some(false)
        );
        assert_eq!(
            feasible(&query("(declare-fun v0 () Bool) (assert true)")),
            Some(true)
        );
        assert_eq!(
            feasible(&query(
                "(declare-fun v0 () (_ BitVec 128)) (assert (not false))"
            )),
            Some(true)
        );
        assert_eq!(
            feasible(&query("(assert (= (and true false) (or false false)))")),
            Some(true)
        );
    }

    #[test]
    fn excessive_nesting_and_token_counts_fall_back_to_the_solver() {
        let expression = format!("{}true{}", "(not ".repeat(MAX_DEPTH), ")".repeat(MAX_DEPTH));
        assert_eq!(feasible(&query(&format!("(assert {expression})"))), None);
        let assertions = "(assert true) ".repeat(MAX_TOKENS / 4 + 1);
        assert_eq!(feasible(&query(&assertions)), None);
    }

    #[test]
    fn ground_bit_vectors_agree_with_z3_at_every_supported_boundary_width() {
        let mut queries = Vec::new();
        for bits in [1, 8, 16, 32, 64, 128] {
            let maximum = mask(bits);
            let sign = 1_u128 << (bits - 1);
            let pairs = [
                (0, 0),
                (maximum, 1),
                (0, maximum),
                (sign, sign - 1),
                (sign, maximum),
            ];
            for extension in ["sign_extend", "zero_extend"] {
                for extra in [0, 128 - bits] {
                    for value in [0, 1, sign, maximum] {
                        for expected in [0, 1, mask(bits + extra)] {
                            queries.push(query(&format!(
                                "(assert (= ((_ {extension} {extra}) \
                                 (_ bv{value} {bits})) (_ bv{expected} {})))",
                                bits + extra,
                            )));
                        }
                    }
                }
            }
            for (left, right) in pairs {
                let left = format!("(_ bv{left} {bits})");
                let right = format!("(_ bv{right} {bits})");
                for operator in [
                    "=", "bvult", "bvule", "bvugt", "bvuge", "bvslt", "bvsle", "bvsgt", "bvsge",
                ] {
                    queries.push(query(&format!("(assert ({operator} {left} {right}))")));
                }
                for operator in ["bvadd", "bvsub", "bvmul", "bvand", "bvor", "bvxor"] {
                    for expected in [0, 1, maximum] {
                        queries.push(query(&format!(
                            "(assert (= ({operator} {left} {right}) (_ bv{expected} {bits})))"
                        )));
                    }
                }
                for operator in ["bvnot", "bvneg"] {
                    queries.push(query(&format!("(assert (= ({operator} {left}) {right}))")));
                }
            }
        }
        let solver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.venv/bin/z3");
        let mut child = Command::new(if solver.is_file() {
            solver
        } else {
            "z3".into()
        })
        .args(["-in", "-smt2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ground differential tests require Z3");
        let mut input = child.stdin.take().unwrap();
        for query in &queries {
            write!(input, "(reset)\n{query}").unwrap();
        }
        drop(input);
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let answers = String::from_utf8(output.stdout).unwrap();
        assert_eq!(answers.lines().count(), queries.len(), "{answers}");
        for (query, answer) in queries.iter().zip(answers.lines()) {
            let expected = match answer {
                "sat" => true,
                "unsat" => false,
                _ => panic!("Z3 did not decide a ground query: {answer}"),
            };
            assert_eq!(feasible(query), Some(expected), "{query}");
        }
    }
}
