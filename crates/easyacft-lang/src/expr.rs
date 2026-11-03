//! Typed expression language compiled to bytecode.

use easyacft_core::units::{parse_unit, Dim};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Bool,
    Num(Dim),
}

impl Type {
    pub fn name(self) -> String {
        match self {
            Type::Bool => "true/false".into(),
            Type::Num(d) => d.name(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExprError {
    pub span: Range<usize>,
    pub message: String,
    pub help: Option<String>,
}

fn err<T>(span: Range<usize>, message: impl Into<String>) -> Result<T, ExprError> {
    Err(ExprError { span, message: message.into(), help: None })
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Unit(String),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
}

fn lex(src: &str) -> Result<Vec<(Tok, Range<usize>)>, ExprError> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if c.is_ascii_digit() || c == '.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
            while i < b.len() && ((b[i] as char).is_ascii_digit() || b[i] == b'.' || (b[i] == b'e' || b[i] == b'E') && b.get(i + 1).is_some_and(|d| d.is_ascii_digit() || *d == b'-')) {
                i += if b[i] == b'e' || b[i] == b'E' { 2 } else { 1 };
            }
            let v: f64 = src[start..i].parse().map_err(|_| ExprError { span: start..i, message: "not a number".into(), help: None })?;
            out.push((Tok::Num(v), start..i));
            continue;
        }
        if c == '[' {
            let end = src[i..].find(']').map(|e| i + e).ok_or_else(|| ExprError { span: i..src.len(), message: "unit is missing its `]`".into(), help: None })?;
            out.push((Tok::Unit(src[i + 1..end].trim().to_owned()), i..end + 1));
            i = end + 1;
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '%' || c == '°' {
            let mut j = i;
            for (k, ch) in src[i..].char_indices() {
                if ch.is_alphanumeric() || ch == '_' || ch == '.' || ch == '%' || ch == '°' {
                    j = i + k + ch.len_utf8();
                } else {
                    break;
                }
            }
            out.push((Tok::Ident(src[i..j].to_owned()), i..j));
            i = j;
            continue;
        }
        let two = src.get(i..i + 2).unwrap_or("");
        let op = match two {
            "&&" | "||" | "==" | "!=" | "<=" | ">=" => Some(two),
            _ => None,
        };
        if let Some(op) = op {
            out.push((Tok::Op(["&&", "||", "==", "!=", "<=", ">="].into_iter().find(|o| *o == op).unwrap()), i..i + 2));
            i += 2;
            continue;
        }
        let tok = match c {
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            ',' => Tok::Comma,
            '!' => Tok::Op("!"),
            '<' => Tok::Op("<"),
            '>' => Tok::Op(">"),
            '+' => Tok::Op("+"),
            '-' => Tok::Op("-"),
            '*' => Tok::Op("*"),
            '/' => Tok::Op("/"),
            _ => return err(i..i + c.len_utf8(), format!("unexpected `{c}`")),
        };
        out.push((tok, i..i + 1));
        i += 1;
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ast {
    Num(f64, Dim, Range<usize>),
    Bool(bool, Range<usize>),
    Name(String, Range<usize>),
    Unary(&'static str, Box<Ast>, Range<usize>),
    Binary(&'static str, Box<Ast>, Box<Ast>, Range<usize>),
    Call(String, Vec<Ast>, Range<usize>),
}

impl Ast {
    pub fn span(&self) -> Range<usize> {
        match self {
            Ast::Num(_, _, s) | Ast::Bool(_, s) | Ast::Name(_, s) | Ast::Unary(_, _, s) | Ast::Binary(_, _, _, s) | Ast::Call(_, _, s) => s.clone(),
        }
    }
}

struct Parser {
    toks: Vec<(Tok, Range<usize>)>,
    at: usize,
    end: usize,
}

fn precedence(op: &str) -> Option<u8> {
    Some(match op {
        "||" => 1,
        "&&" => 2,
        "==" | "!=" | "<" | "<=" | ">" | ">=" => 3,
        "+" | "-" => 4,
        "*" | "/" => 5,
        _ => return None,
    })
}

impl Parser {
    fn peek(&self) -> Option<&(Tok, Range<usize>)> {
        self.toks.get(self.at)
    }

    fn next(&mut self) -> Option<(Tok, Range<usize>)> {
        let t = self.toks.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn expr(&mut self, min: u8) -> Result<Ast, ExprError> {
        let mut lhs = self.unary()?;
        loop {
            let Some((Tok::Op(op), _)) = self.peek().cloned() else { break };
            let Some(p) = precedence(op) else { break };
            if p < min {
                break;
            }
            self.next();
            let rhs = self.expr(p + 1)?;
            let span = lhs.span().start..rhs.span().end;
            lhs = Ast::Binary(op, Box::new(lhs), Box::new(rhs), span);
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Ast, ExprError> {
        if let Some((Tok::Op(op @ ("!" | "-")), span)) = self.peek().cloned() {
            self.next();
            let inner = self.unary()?;
            let s = span.start..inner.span().end;
            return Ok(Ast::Unary(op, Box::new(inner), s));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Ast, ExprError> {
        let Some((tok, span)) = self.next() else { return err(self.end..self.end, "the expression ends too early") };
        match tok {
            Tok::Num(v) => {
                let unit = match self.peek() {
                    Some((Tok::Unit(u), s)) => Some((u.clone(), s.clone())),
                    Some((Tok::Ident(u), s)) if parse_unit(u).is_ok() && !matches!(self.toks.get(self.at + 1), Some((Tok::LParen, _))) => Some((u.clone(), s.clone())),
                    _ => None,
                };
                match unit {
                    Some((mut u, mut us)) => {
                        self.next();
                        while let (Some((Tok::Op(op @ ("/" | "*")), _)), Some((Tok::Ident(more), ms))) = (self.toks.get(self.at).cloned(), self.toks.get(self.at + 1).cloned()) {
                            let joined = format!("{u}{op}{more}");
                            if parse_unit(&more).is_err() || parse_unit(&joined).is_err() {
                                break;
                            }
                            u = joined;
                            us = us.start..ms.end;
                            self.at += 2;
                        }
                        let (f, off, d) = parse_unit(&u).map_err(|e| ExprError { span: us.clone(), message: e.to_string(), help: None })?;
                        Ok(Ast::Num(v * f + off, d, span.start..us.end))
                    }
                    None => Ok(Ast::Num(v, Dim::NONE, span)),
                }
            }
            Tok::Ident(name) if name == "true" || name == "false" => Ok(Ast::Bool(name == "true", span)),
            Tok::Ident(name) => {
                if matches!(self.peek(), Some((Tok::LParen, _))) {
                    self.next();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some((Tok::RParen, _))) {
                        loop {
                            args.push(self.expr(0)?);
                            match self.next() {
                                Some((Tok::Comma, _)) => continue,
                                Some((Tok::RParen, e)) => return Ok(Ast::Call(name, args, span.start..e.end)),
                                Some((_, s)) => return err(s, "expected `,` or `)`"),
                                None => return err(self.end..self.end, "missing `)`"),
                            }
                        }
                    }
                    let (_, e) = self.next().unwrap();
                    return Ok(Ast::Call(name, args, span.start..e.end));
                }
                Ok(Ast::Name(name, span))
            }
            Tok::LParen => {
                let inner = self.expr(0)?;
                match self.next() {
                    Some((Tok::RParen, _)) => Ok(inner),
                    Some((_, s)) => err(s, "expected `)`"),
                    None => err(self.end..self.end, "missing `)`"),
                }
            }
            Tok::Unit(_) => err(span, "a unit needs a number in front of it"),
            _ => err(span, "expected a value, a name or `(`"),
        }
    }
}

pub fn parse(src: &str) -> Result<Ast, ExprError> {
    let toks = lex(src)?;
    let mut p = Parser { toks, at: 0, end: src.len() };
    let ast = p.expr(0)?;
    if let Some((_, s)) = p.peek() {
        return err(s.clone(), "unexpected text after the expression");
    }
    Ok(ast)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Push(f64),
    Load(usize),
    Not,
    Neg,
    Add,
    Sub,
    Mul,
    Div,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
    Min,
    Max,
    Abs,
    If,
    Delay(usize),
    OffDelay(usize),
    Latch(usize),
    Rising(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub ops: Vec<Op>,
    pub states: usize,
    pub ty: Type,
}

/// Resolves expression names to value slots.
pub trait Names {
    fn lookup(&self, name: &str) -> Option<(usize, Type)>;
    fn suggest(&self, _name: &str) -> Option<String> {
        None
    }
}

struct Compiler<'a> {
    names: &'a dyn Names,
    ops: Vec<Op>,
    states: usize,
}

fn need(span: Range<usize>, got: Type, want: &str) -> ExprError {
    ExprError { span, message: format!("expected {want}, found {}", got.name()), help: None }
}

impl Compiler<'_> {
    fn bool(&mut self, a: &Ast) -> Result<(), ExprError> {
        match self.node(a)? {
            Type::Bool => Ok(()),
            t => Err(need(a.span(), t, "true/false")),
        }
    }

    fn node(&mut self, a: &Ast) -> Result<Type, ExprError> {
        Ok(match a {
            Ast::Num(v, d, _) => {
                self.ops.push(Op::Push(*v));
                Type::Num(*d)
            }
            Ast::Bool(b, _) => {
                self.ops.push(Op::Push(*b as u8 as f64));
                Type::Bool
            }
            Ast::Name(n, s) => match self.names.lookup(n) {
                Some((slot, t)) => {
                    self.ops.push(Op::Load(slot));
                    t
                }
                None => {
                    return Err(ExprError {
                        span: s.clone(),
                        message: format!("nothing called `{n}`"),
                        help: self.names.suggest(n).map(|s| format!("did you mean `{s}`?")),
                    })
                }
            },
            Ast::Unary("!", x, _) => {
                self.bool(x)?;
                self.ops.push(Op::Not);
                Type::Bool
            }
            Ast::Unary(_, x, _) => match self.node(x)? {
                Type::Num(d) => {
                    self.ops.push(Op::Neg);
                    Type::Num(d)
                }
                t => return Err(need(x.span(), t, "a number")),
            },
            Ast::Binary(op @ ("&&" | "||"), l, r, _) => {
                self.bool(l)?;
                self.bool(r)?;
                self.ops.push(if *op == "&&" { Op::And } else { Op::Or });
                Type::Bool
            }
            Ast::Binary(op, l, r, s) => {
                let (lt, rt) = (self.node(l)?, self.node(r)?);
                let (Type::Num(ld), Type::Num(rd)) = (lt, rt) else {
                    if matches!(*op, "==" | "!=") && lt == rt {
                        self.ops.push(if *op == "==" { Op::Eq } else { Op::Ne });
                        return Ok(Type::Bool);
                    }
                    let bad = if lt == Type::Bool { l } else { r };
                    return Err(need(bad.span(), Type::Bool, "a number"));
                };
                match *op {
                    "*" | "/" => {
                        self.ops.push(if *op == "*" { Op::Mul } else { Op::Div });
                        Type::Num(if *op == "*" { ld.mul(rd) } else { ld.mul(rd.pow(-1)) })
                    }
                    _ => {
                        if ld != rd && ld != Dim::NONE && rd != Dim::NONE || (ld == Dim::NONE) != (rd == Dim::NONE) && !matches!(&**r, Ast::Num(v, Dim::NONE, _) if *v == 0.0) && !matches!(&**l, Ast::Num(v, Dim::NONE, _) if *v == 0.0) {
                            return Err(ExprError { span: s.clone(), message: format!("cannot compare or add {} and {}", ld.name(), rd.name()), help: Some("both sides need the same kind of quantity, with units such as `25 V`".into()) });
                        }
                        let dim = if ld == Dim::NONE { rd } else { ld };
                        self.ops.push(match *op {
                            "+" => Op::Add,
                            "-" => Op::Sub,
                            "<" => Op::Lt,
                            "<=" => Op::Le,
                            ">" => Op::Gt,
                            ">=" => Op::Ge,
                            "==" => Op::Eq,
                            _ => Op::Ne,
                        });
                        if matches!(*op, "+" | "-") {
                            Type::Num(dim)
                        } else {
                            Type::Bool
                        }
                    }
                }
            }
            Ast::Call(f, args, s) => {
                let want = |n: usize| -> Result<(), ExprError> {
                    if args.len() != n {
                        return err(s.clone(), format!("`{f}` takes {n} value{}", if n == 1 { "" } else { "s" }));
                    }
                    Ok(())
                };
                match f.as_str() {
                    "delay" | "off_delay" => {
                        want(2)?;
                        self.bool(&args[0])?;
                        match self.node(&args[1])? {
                            Type::Num(d) if d == Dim::TIME => {}
                            t => return Err(need(args[1].span(), t, "a time such as `3 s`")),
                        }
                        let k = self.states;
                        self.states += 1;
                        self.ops.push(if f == "delay" { Op::Delay(k) } else { Op::OffDelay(k) });
                        Type::Bool
                    }
                    "latch" => {
                        want(2)?;
                        self.bool(&args[0])?;
                        self.bool(&args[1])?;
                        let k = self.states;
                        self.states += 1;
                        self.ops.push(Op::Latch(k));
                        Type::Bool
                    }
                    "rising" => {
                        want(1)?;
                        self.bool(&args[0])?;
                        let k = self.states;
                        self.states += 1;
                        self.ops.push(Op::Rising(k));
                        Type::Bool
                    }
                    "min" | "max" => {
                        want(2)?;
                        let (a, b) = (self.node(&args[0])?, self.node(&args[1])?);
                        if a != b || a == Type::Bool {
                            return err(s.clone(), format!("`{f}` needs two numbers of the same kind"));
                        }
                        self.ops.push(if f == "min" { Op::Min } else { Op::Max });
                        a
                    }
                    "abs" => {
                        want(1)?;
                        let t = self.node(&args[0])?;
                        if t == Type::Bool {
                            return Err(need(args[0].span(), t, "a number"));
                        }
                        self.ops.push(Op::Abs);
                        t
                    }
                    "if" => {
                        want(3)?;
                        self.bool(&args[0])?;
                        let (a, b) = (self.node(&args[1])?, self.node(&args[2])?);
                        if a != b {
                            return err(s.clone(), format!("both branches of `if` must be the same kind, found {} and {}", a.name(), b.name()));
                        }
                        self.ops.push(Op::If);
                        a
                    }
                    _ => {
                        return Err(ExprError {
                            span: s.start..s.start + f.len(),
                            message: format!("unknown function `{f}`"),
                            help: Some("available: delay, off_delay, latch, rising, min, max, abs, if".into()),
                        })
                    }
                }
            }
        })
    }
}

pub fn compile(src: &str, names: &dyn Names) -> Result<Program, ExprError> {
    let ast = parse(src)?;
    let mut c = Compiler { names, ops: Vec::new(), states: 0 };
    let ty = c.node(&ast)?;
    Ok(Program { ops: c.ops, states: c.states, ty })
}

impl Program {
    /// One evaluation; `state` holds timers and latches.
    pub fn eval(&self, slots: &[f64], state: &mut [f64], dt: f64) -> f64 {
        let mut st: Vec<f64> = Vec::with_capacity(8);
        let b = |v: f64| v != 0.0;
        for op in &self.ops {
            match *op {
                Op::Push(v) => st.push(v),
                Op::Load(i) => st.push(slots.get(i).copied().unwrap_or(0.0)),
                Op::Not => {
                    let v = st.pop().unwrap_or(0.0);
                    st.push(!b(v) as u8 as f64)
                }
                Op::Neg => {
                    let v = st.pop().unwrap_or(0.0);
                    st.push(-v)
                }
                Op::Abs => {
                    let v = st.pop().unwrap_or(0.0);
                    st.push(v.abs())
                }
                Op::Delay(k) | Op::OffDelay(k) => {
                    let t = st.pop().unwrap_or(0.0);
                    let c = b(st.pop().unwrap_or(0.0));
                    let timer = &mut state[k];
                    let out = if matches!(op, Op::Delay(_)) {
                        *timer = if c { *timer + dt } else { 0.0 };
                        c && *timer >= t - 1e-9
                    } else {
                        *timer = if c { 0.0 } else { *timer + dt };
                        c || *timer < t - 1e-9
                    };
                    st.push(out as u8 as f64)
                }
                Op::Latch(k) => {
                    let reset = b(st.pop().unwrap_or(0.0));
                    let set = b(st.pop().unwrap_or(0.0));
                    let held = if reset { false } else { set || b(state[k]) };
                    state[k] = held as u8 as f64;
                    st.push(state[k])
                }
                Op::Rising(k) => {
                    let c = b(st.pop().unwrap_or(0.0));
                    let out = c && !b(state[k]);
                    state[k] = c as u8 as f64;
                    st.push(out as u8 as f64)
                }
                Op::If => {
                    let e = st.pop().unwrap_or(0.0);
                    let t = st.pop().unwrap_or(0.0);
                    let c = st.pop().unwrap_or(0.0);
                    st.push(if b(c) { t } else { e })
                }
                _ => {
                    let r = st.pop().unwrap_or(0.0);
                    let l = st.pop().unwrap_or(0.0);
                    st.push(match op {
                        Op::Add => l + r,
                        Op::Sub => l - r,
                        Op::Mul => l * r,
                        Op::Div => if r == 0.0 { 0.0 } else { l / r },
                        Op::Lt => (l < r) as u8 as f64,
                        Op::Le => (l <= r) as u8 as f64,
                        Op::Gt => (l > r) as u8 as f64,
                        Op::Ge => (l >= r) as u8 as f64,
                        Op::Eq => (l == r) as u8 as f64,
                        Op::Ne => (l != r) as u8 as f64,
                        Op::And => (b(l) && b(r)) as u8 as f64,
                        Op::Or => (b(l) || b(r)) as u8 as f64,
                        Op::Min => l.min(r),
                        Op::Max => l.max(r),
                        _ => 0.0,
                    })
                }
            }
        }
        st.pop().unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Map(Vec<(&'static str, Type)>);
    impl Names for Map {
        fn lookup(&self, n: &str) -> Option<(usize, Type)> {
            self.0.iter().position(|(k, _)| *k == n).map(|i| (i, self.0[i].1))
        }
        fn suggest(&self, n: &str) -> Option<String> {
            self.0.iter().map(|(k, _)| *k).find(|k| k.starts_with(&n[..n.len().min(4)])).map(str::to_owned)
        }
    }

    fn names() -> Map {
        Map(vec![("gen1.ready", Type::Bool), ("bus_tie.off", Type::Bool), ("dc1.voltage", Type::Num(Dim::VOLTAGE)), ("dc1.current", Type::Num(Dim::CURRENT))])
    }

    fn run(src: &str, slots: &[f64]) -> f64 {
        let p = compile(src, &names()).unwrap();
        p.eval(slots, &mut vec![0.0; p.states], 0.1)
    }

    #[test]
    fn logic_and_comparisons_with_units() {
        assert_eq!(run("gen1.ready && !bus_tie.off", &[1.0, 0.0, 28.0, 0.0]), 1.0);
        assert_eq!(run("dc1.voltage < 25 V", &[0.0, 0.0, 24.0, 0.0]), 1.0);
        assert_eq!(run("dc1.voltage * dc1.current > 1 kW", &[0.0, 0.0, 28.0, 40.0]), 1.0);
        assert_eq!(run("if(gen1.ready, 115 V, 0 V)", &[1.0, 0.0, 0.0, 0.0]), 115.0);
        assert_eq!(run("min(dc1.voltage, 28 V) + 1 V", &[0.0, 0.0, 30.0, 0.0]), 29.0);
        assert_eq!(compile("dc1.current / dc1.voltage", &names()).unwrap().ty, Type::Num(Dim::RESISTANCE.pow(-1)));
        assert_eq!(compile("0.8 kg/s", &names()).unwrap().ty, Type::Num(Dim::MASS_FLOW));
        assert_eq!(compile("2 m/s2 * 1 s", &names()).unwrap().ty, Type::Num(Dim::SPEED));
    }

    #[test]
    fn timers_and_latches_keep_their_state() {
        let p = compile("delay(dc1.voltage < 25 V, 0.3 s)", &names()).unwrap();
        let mut st = vec![0.0; p.states];
        let low = [0.0, 0.0, 20.0, 0.0];
        assert_eq!(p.eval(&low, &mut st, 0.1), 0.0);
        assert_eq!(p.eval(&low, &mut st, 0.1), 0.0);
        assert_eq!(p.eval(&low, &mut st, 0.1), 1.0);
        assert_eq!(p.eval(&[0.0, 0.0, 28.0, 0.0], &mut st, 0.1), 0.0);
        let l = compile("latch(gen1.ready, bus_tie.off)", &names()).unwrap();
        let mut ls = vec![0.0];
        assert_eq!(l.eval(&[1.0, 0.0, 0.0, 0.0], &mut ls, 0.1), 1.0);
        assert_eq!(l.eval(&[0.0, 0.0, 0.0, 0.0], &mut ls, 0.1), 1.0);
        assert_eq!(l.eval(&[0.0, 1.0, 0.0, 0.0], &mut ls, 0.1), 0.0);
        let r = compile("rising(gen1.ready)", &names()).unwrap();
        let mut rs = vec![0.0];
        assert_eq!(r.eval(&[1.0, 0.0, 0.0, 0.0], &mut rs, 0.1), 1.0);
        assert_eq!(r.eval(&[1.0, 0.0, 0.0, 0.0], &mut rs, 0.1), 0.0);
    }

    #[test]
    fn mistakes_point_at_the_right_text() {
        let e = compile("dc1.voltage < 25 psi", &names()).unwrap_err();
        assert_eq!(e.span, 0..20);
        assert!(e.message.contains("voltage") && e.message.contains("pressure"), "{}", e.message);
        let e = compile("gen1.redy && true", &names()).unwrap_err();
        assert_eq!((e.span.clone(), e.help.as_deref()), (0..9, Some("did you mean `gen1.ready`?")));
        let e = compile("delay(gen1.ready, 3 V)", &names()).unwrap_err();
        assert!(e.message.contains("time"), "{}", e.message);
        let e = compile("dc1.voltage && gen1.ready", &names()).unwrap_err();
        assert_eq!(e.span, 0..11);
        assert!(compile("(gen1.ready", &names()).is_err());
        assert!(compile("gen1.ready gen1.ready", &names()).is_err());
        let e = compile("blink(gen1.ready)", &names()).unwrap_err();
        assert!(e.help.unwrap().contains("delay"));
    }
}
