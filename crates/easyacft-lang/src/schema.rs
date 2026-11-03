//! Part schemas and the checker that holds a model against them.

use crate::diag::Diagnostic;
use crate::expr::{self, Names, Program, Type};
use crate::model::{Field, Model, PartDecl};
use easyacft_core::units::{parse_quantity, Dim};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Quantity(Dim),
    Number,
    Bool,
    Text,
    /// A part of one of these kinds (`"electrical.bus"`, or `"electrical.*"`).
    Ref(&'static [&'static str]),
    Refs(&'static [&'static str]),
    Expr(Type),
}

#[derive(Clone, Copy, Debug)]
pub struct FieldSpec {
    pub key: &'static str,
    pub kind: Kind,
    pub default: Option<&'static str>,
    pub help: &'static str,
}

pub const fn req(key: &'static str, kind: Kind, help: &'static str) -> FieldSpec {
    FieldSpec { key, kind, default: None, help }
}

pub const fn opt(key: &'static str, kind: Kind, default: &'static str, help: &'static str) -> FieldSpec {
    FieldSpec { key, kind, default: Some(default), help }
}

#[derive(Clone, Copy, Debug)]
pub struct PartSpec {
    pub domain: &'static str,
    pub kind: &'static str,
    pub help: &'static str,
    pub fields: &'static [FieldSpec],
    /// Values every part of this kind exposes as `<id>.<name>`.
    pub outputs: &'static [(&'static str, Type)],
}

pub trait Catalog {
    fn specs(&self) -> &[PartSpec];
    fn spec(&self, domain: &str, kind: &str) -> Option<&PartSpec> {
        self.specs().iter().find(|s| s.domain == domain && s.kind == kind)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(f64),
    Bool(bool),
    Text(String),
    Ref(usize),
    Refs(Vec<usize>),
    Expr(Program),
}

#[derive(Clone, Debug)]
pub struct CheckedPart {
    pub domain: &'static str,
    pub kind: &'static str,
    pub id: String,
    pub values: Vec<(&'static str, Value)>,
    pub slot: usize,
}

impl CheckedPart {
    pub fn num(&self, k: &str) -> f64 {
        self.values.iter().find(|(n, _)| *n == k).and_then(|(_, v)| if let Value::Num(x) = v { Some(*x) } else { None }).unwrap_or(0.0)
    }
    pub fn flag(&self, k: &str) -> bool {
        self.values.iter().any(|(n, v)| *n == k && *v == Value::Bool(true))
    }
    pub fn text(&self, k: &str) -> &str {
        self.values.iter().find(|(n, _)| *n == k).and_then(|(_, v)| if let Value::Text(x) = v { Some(x.as_str()) } else { None }).unwrap_or("")
    }
    pub fn part(&self, k: &str) -> Option<usize> {
        self.values.iter().find(|(n, _)| *n == k).and_then(|(_, v)| if let Value::Ref(i) = v { Some(*i) } else { None })
    }
    pub fn parts(&self, k: &str) -> Vec<usize> {
        self.values.iter().find(|(n, _)| *n == k).map(|(_, v)| match v { Value::Refs(r) => r.clone(), Value::Ref(i) => vec![*i], _ => vec![] }).unwrap_or_default()
    }
    pub fn expr(&self, k: &str) -> Option<&Program> {
        self.values.iter().find(|(n, _)| *n == k).and_then(|(_, v)| if let Value::Expr(p) = v { Some(p) } else { None })
    }
}

/// Every name expressions can read: `<id>.<output>` to a slot.
#[derive(Clone, Debug, Default)]
pub struct Slots {
    pub names: Vec<(String, Type)>,
    pub index: HashMap<String, usize>,
}

impl Slots {
    pub fn add(&mut self, name: String, ty: Type) -> usize {
        if let Some(&i) = self.index.get(&name) {
            return i;
        }
        self.index.insert(name.clone(), self.names.len());
        self.names.push((name, ty));
        self.names.len() - 1
    }
}

impl Names for Slots {
    fn lookup(&self, name: &str) -> Option<(usize, Type)> {
        self.index.get(name).map(|&i| (i, self.names[i].1))
    }
    fn suggest(&self, name: &str) -> Option<String> {
        closest(name, self.names.iter().map(|(n, _)| n.as_str()))
    }
}

pub fn closest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        let d = edit_distance(name, c);
        if d <= (name.len() / 3).max(2) && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c.to_owned())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + (a[i - 1] != b[j - 1]) as usize);
        }
        prev = cur;
    }
    prev[b.len()]
}

#[derive(Clone, Debug, Default)]
pub struct Checked {
    pub parts: Vec<CheckedPart>,
    pub slots: Slots,
}

impl Checked {
    pub fn find(&self, id: &str) -> Option<usize> {
        self.parts.iter().position(|p| p.id == id)
    }
}

fn matches(allowed: &[&str], decl: &PartDecl) -> bool {
    allowed.iter().any(|a| {
        let (d, k) = a.split_once('.').unwrap_or((a, "*"));
        d == decl.domain && (k == "*" || k == decl.kind)
    })
}

pub fn check(model: &Model, catalog: &dyn Catalog) -> (Checked, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let mut out = Checked::default();
    let mut specs = Vec::new();
    for p in &model.parts {
        match catalog.spec(&p.domain, &p.kind) {
            Some(s) => specs.push(Some(*s)),
            None => {
                let all = catalog.specs().iter().map(|s| format!("{}.{}", s.domain, s.kind)).collect::<Vec<_>>();
                let near = closest(&format!("{}.{}", p.domain, p.kind), all.iter().map(String::as_str));
                let mut d = Diagnostic::error("E0101", format!("there is no part kind `{}.{}`", p.domain, p.kind), p.file, p.span.clone());
                if let Some(n) = near {
                    d = d.help(format!("did you mean `{n}`?"));
                }
                diags.push(d);
                specs.push(None);
            }
        }
    }
    for (i, p) in model.parts.iter().enumerate() {
        let Some(s) = specs[i] else { continue };
        let slot = out.slots.names.len();
        for (name, ty) in s.outputs {
            out.slots.add(format!("{}.{name}", p.id), *ty);
        }
        out.parts.push(CheckedPart { domain: s.domain, kind: s.kind, id: p.id.clone(), values: Vec::new(), slot });
    }
    let ids: Vec<String> = out.parts.iter().map(|p| p.id.clone()).collect();
    let index: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, p)| (p.as_str(), i)).collect();
    let mut all_values = vec![Vec::new(); out.parts.len()];
    let decls: Vec<(&PartDecl, PartSpec)> = model.parts.iter().zip(&specs).filter_map(|(p, s)| s.map(|s| (p, s))).collect();
    for (pi, (p, s)) in decls.iter().enumerate() {
        for f in &p.fields {
            if !s.fields.iter().any(|x| x.key == f.key) {
                let near = closest(&f.key, s.fields.iter().map(|x| x.key));
                let mut d = Diagnostic::error("E0102", format!("{}.{} has no setting `{}`", s.domain, s.kind, f.key), f.file, f.span.clone());
                if let Some(n) = near {
                    d = d.help(format!("did you mean `{n}`?"));
                }
                diags.push(d);
            }
        }
        let mut values = Vec::new();
        for spec in s.fields {
            let decl = p.get(spec.key);
            let raw = match (decl, spec.default) {
                (Some(d), _) => d.value.clone(),
                (None, Some(def)) => match spec.kind {
                    Kind::Bool => Field::Bool(def == "true"),
                    Kind::Number => Field::Num(def.parse().unwrap_or(0.0)),
                    Kind::Refs(_) if def.is_empty() => Field::List(vec![]),
                    _ => Field::Str(def.to_owned()),
                },
                (None, None) => {
                    diags.push(Diagnostic::error("E0105", format!("{} `{}` needs `{}`", s.kind, p.id, spec.key), p.file, p.span.clone()).help(spec.help));
                    continue;
                }
            };
            let (file, span) = decl.map_or((p.file, p.span.clone()), |d| (d.file, d.span.clone()));
            let bad = |what: &str| Diagnostic::error("E0103", format!("`{}` must be {what}", spec.key), file, span.clone()).help(spec.help);
            let v = match (spec.kind, &raw) {
                (Kind::Quantity(dim), Field::Str(t)) => match parse_quantity(t) {
                    Ok(q) if q.dim == dim => Value::Num(q.si),
                    Ok(q) => {
                        diags.push(bad(&dim.name()).label(format!("this is {}", q.dim.name())));
                        continue;
                    }
                    Err(e) => {
                        diags.push(bad(&dim.name()).label(e.to_string()));
                        continue;
                    }
                },
                (Kind::Quantity(dim), Field::Num(x)) if dim == Dim::NONE => Value::Num(*x),
                (Kind::Quantity(dim), _) => {
                    diags.push(bad(&format!("{} with a unit, in quotes", dim.name())));
                    continue;
                }
                (Kind::Number, Field::Num(x)) => Value::Num(*x),
                (Kind::Bool, Field::Bool(b)) => Value::Bool(*b),
                (Kind::Text, Field::Str(t)) => Value::Text(t.clone()),
                (Kind::Ref(allowed), Field::Str(t)) => match index.get(t.as_str()) {
                    Some(&i) if matches(allowed, decls[i].0) => Value::Ref(i),
                    Some(&i) => {
                        diags.push(Diagnostic::error("E0106", format!("`{t}` is a {}.{}, `{}` needs {}", decls[i].0.domain, decls[i].0.kind, spec.key, allowed.join(" or ")), file, span.clone()));
                        continue;
                    }
                    None => {
                        let mut d = Diagnostic::error("E0106", format!("there is no part `{t}`"), file, span.clone());
                        if let Some(n) = closest(t, index.keys().copied()) {
                            d = d.help(format!("did you mean `{n}`?"));
                        }
                        diags.push(d);
                        continue;
                    }
                },
                (Kind::Refs(allowed), Field::List(items)) => {
                    let mut ok = Vec::new();
                    for t in items {
                        match index.get(t.as_str()) {
                            Some(&i) if matches(allowed, decls[i].0) => ok.push(i),
                            Some(&i) => diags.push(Diagnostic::error("E0106", format!("`{t}` is a {}.{}, `{}` takes {}", decls[i].0.domain, decls[i].0.kind, spec.key, allowed.join(" or ")), file, span.clone())),
                            None => {
                                let mut d = Diagnostic::error("E0106", format!("there is no part `{t}`"), file, span.clone());
                                if let Some(n) = closest(t, index.keys().copied()) {
                                    d = d.help(format!("did you mean `{n}`?"));
                                }
                                diags.push(d);
                            }
                        }
                    }
                    Value::Refs(ok)
                }
                (Kind::Expr(want), Field::Str(src)) => match expr::compile(src, &out.slots) {
                    Ok(prog) if prog.ty == want => Value::Expr(prog),
                    Ok(prog) => {
                        diags.push(bad(&format!("{} (this expression gives {})", want.name(), prog.ty.name())));
                        continue;
                    }
                    Err(e) => {
                        let at = span.start + 1 + e.span.start;
                        let mut d = Diagnostic::error("E0201", e.message, file, at..span.start + 1 + e.span.end.max(e.span.start + 1));
                        if let Some(h) = e.help {
                            d = d.help(h);
                        }
                        diags.push(d);
                        continue;
                    }
                },
                (Kind::Expr(Type::Bool), Field::Bool(b)) => Value::Expr(expr::compile(if *b { "true" } else { "false" }, &out.slots).unwrap()),
                (k, _) => {
                    let what = match k {
                        Kind::Number => "a number",
                        Kind::Bool => "true or false",
                        Kind::Text => "text in quotes",
                        Kind::Ref(_) => "the id of a part, in quotes",
                        Kind::Refs(_) => "a list of part ids",
                        _ => "an expression in quotes",
                    };
                    diags.push(bad(what));
                    continue;
                }
            };
            values.push((spec.key, v));
        }
        all_values[pi] = values;
    }
    for (p, v) in out.parts.iter_mut().zip(all_values) {
        p.values = v;
    }
    (out, diags)
}
