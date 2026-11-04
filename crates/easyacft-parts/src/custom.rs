//! Developers' own part kinds: declared in `[kinds.<name>]`, placed as `[[custom.<name>]]`, stepped by their code.
//!
//! ```toml
//! [kinds.turbo_pump]
//! help = "A turbine-driven pump"
//! settings.rated_flow = "L/min"
//! settings.drive = { type = "expr bool", default = "false" }
//! settings.circuit = "part hydraulic.circuit"
//! values.flow = "L/min"
//! values.running = "bool"
//! ```

use crate::Expr;
use easyacft_core::units::parse_unit;
use easyacft_core::Dim;
use easyacft_lang::model::KindDecl;
use easyacft_lang::schema::{FieldSpec, Kind, PartSpec, Value};
use easyacft_lang::{Checked, Diagnostic, Type};
use std::collections::HashMap;

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

fn value_type(ty: &str) -> Result<Type, String> {
    match ty.trim() {
        "bool" => Ok(Type::Bool),
        "number" => Ok(Type::Num(Dim::NONE)),
        u => parse_unit(u).map(|(_, _, d)| Type::Num(d)).map_err(|e| format!("`{u}` is not a type: use bool, number or a unit such as \"L/min\" ({e})")),
    }
}

fn setting_kind(ty: &str) -> Result<Kind, String> {
    let t = ty.trim();
    if let Some(rest) = t.strip_prefix("expr ") {
        return value_type(rest).map(Kind::Expr);
    }
    let refs = |r: &str| -> &'static [&'static str] { Box::leak(r.split(',').map(|x| leak(x.trim())).collect::<Vec<_>>().into_boxed_slice()) };
    if let Some(r) = t.strip_prefix("parts ") {
        return Ok(Kind::Refs(refs(r)));
    }
    if let Some(r) = t.strip_prefix("part ") {
        return Ok(Kind::Ref(refs(r)));
    }
    match t {
        "bool" => Ok(Kind::Bool),
        "number" => Ok(Kind::Number),
        "text" => Ok(Kind::Text),
        u => parse_unit(u).map(|(_, _, d)| Kind::Quantity(d)).map_err(|e| format!("`{u}` is not a setting type: use bool, number, text, a unit, \"expr <type>\" or \"part <domain.kind>\" ({e})")),
    }
}

/// Turns declarations into part kinds for the checker; problems come back as diagnostics.
pub fn specs(kinds: &[KindDecl], diags: &mut Vec<Diagnostic>) -> Vec<PartSpec> {
    let mut out = Vec::new();
    for k in kinds {
        let mut fields = Vec::new();
        let mut outputs: Vec<(&'static str, Type)> = Vec::new();
        let mut ok = true;
        for s in &k.settings {
            match setting_kind(&s.ty) {
                Ok(kind) => {
                    if let Kind::Expr(t) = kind {
                        outputs.push((leak(&s.key), t));
                    }
                    let default = s.default.as_deref().map(leak);
                    fields.push(FieldSpec { key: leak(&s.key), kind, default, help: leak(&s.help) });
                }
                Err(e) => {
                    diags.push(Diagnostic::error("E0013", e, k.file, s.span.clone()));
                    ok = false;
                }
            }
        }
        for v in &k.values {
            if outputs.iter().any(|(n, _)| *n == v.key) {
                diags.push(Diagnostic::error("E0013", format!("`{}` is both a setting and a value", v.key), k.file, v.span.clone()));
                ok = false;
                continue;
            }
            match value_type(&v.ty) {
                Ok(t) => outputs.push((leak(&v.key), t)),
                Err(e) => {
                    diags.push(Diagnostic::error("E0013", e, k.file, v.span.clone()));
                    ok = false;
                }
            }
        }
        if !outputs.iter().any(|(n, _)| *n == "failed") {
            outputs.push(("failed", Type::Bool));
        }
        if ok {
            out.push(PartSpec {
                domain: "custom",
                kind: leak(&k.name),
                help: leak(if k.help.is_empty() { "A part kind of this aircraft's own." } else { &k.help }),
                fields: Box::leak(fields.into_boxed_slice()),
                outputs: Box::leak(outputs.into_boxed_slice()),
            });
        }
    }
    out
}

struct CustomPart {
    id: String,
    kind: &'static str,
    own: Vec<(&'static str, usize)>,
    nums: Vec<(&'static str, f64)>,
    texts: Vec<(&'static str, String)>,
    refs: Vec<(&'static str, Vec<String>)>,
    exprs: Vec<(usize, Expr)>,
}

/// One custom part as its code sees it.
pub struct Part<'a> {
    p: &'a CustomPart,
    slots: &'a mut [f64],
    index: &'a HashMap<String, usize>,
    pub dt: f64,
}

impl Part<'_> {
    pub fn id(&self) -> &str {
        &self.p.id
    }
    pub fn kind(&self) -> &str {
        self.p.kind
    }
    /// One of this part's values, such as `flow`; expression settings are values too.
    pub fn get(&self, name: &str) -> f64 {
        self.p.own.iter().find(|(n, _)| *n == name).map_or(0.0, |&(_, s)| self.slots[s])
    }
    pub fn set(&mut self, name: &str, v: f64) -> bool {
        match self.p.own.iter().find(|(n, _)| *n == name) {
            Some(&(_, s)) => {
                self.slots[s] = v;
                true
            }
            None => false,
        }
    }
    /// Any value of the aircraft, such as `ac1.voltage`.
    pub fn read(&self, full: &str) -> Option<f64> {
        self.index.get(full).map(|&s| self.slots[s])
    }
    pub fn write(&mut self, full: &str, v: f64) -> bool {
        match self.index.get(full) {
            Some(&s) => {
                self.slots[s] = v;
                true
            }
            None => false,
        }
    }
    /// A number, quantity (SI) or true/false setting.
    pub fn setting(&self, key: &str) -> f64 {
        self.p.nums.iter().find(|(n, _)| *n == key).map_or(0.0, |x| x.1)
    }
    pub fn text(&self, key: &str) -> &str {
        self.p.texts.iter().find(|(n, _)| *n == key).map_or("", |x| x.1.as_str())
    }
    /// Ids of the parts a `part` or `parts` setting names.
    pub fn parts(&self, key: &str) -> &[String] {
        self.p.refs.iter().find(|(n, _)| *n == key).map_or(&[], |x| x.1.as_slice())
    }
}

pub type KindFn = Box<dyn FnMut(&mut Part<'_>)>;

#[derive(Default)]
pub struct Custom {
    parts: Vec<CustomPart>,
    handlers: Vec<(String, KindFn)>,
    index: HashMap<String, usize>,
}

impl Custom {
    pub fn new(c: &Checked) -> Self {
        let mut parts = Vec::new();
        for p in c.parts.iter().filter(|p| p.domain == "custom") {
            let Some(spec) = crate::catalog::find(p.domain, p.kind) else { continue };
            let own = spec.outputs.iter().enumerate().map(|(i, (n, _))| (*n, p.slot + i)).collect::<Vec<_>>();
            let mut cp = CustomPart { id: p.id.clone(), kind: p.kind, own, nums: vec![], texts: vec![], refs: vec![], exprs: vec![] };
            for (k, v) in &p.values {
                match v {
                    Value::Num(x) => cp.nums.push((k, *x)),
                    Value::Bool(b) => cp.nums.push((k, *b as u8 as f64)),
                    Value::Text(t) => cp.texts.push((k, t.clone())),
                    Value::Ref(i) => cp.refs.push((k, vec![c.parts[*i].id.clone()])),
                    Value::Refs(r) => cp.refs.push((k, r.iter().map(|i| c.parts[*i].id.clone()).collect())),
                    Value::Expr(prog) => {
                        if let Some(&(_, s)) = cp.own.iter().find(|(n, _)| n == k) {
                            cp.exprs.push((s, Expr::new(prog)));
                        }
                    }
                }
            }
            parts.push(cp);
        }
        let index = c.slots.index.clone();
        Custom { parts, handlers: Vec::new(), index }
    }

    pub fn count(&self, kind: &str) -> usize {
        self.parts.iter().filter(|p| p.kind == kind).count()
    }

    pub fn on_kind(&mut self, kind: &str, f: KindFn) -> usize {
        self.handlers.push((kind.to_owned(), f));
        self.count(kind)
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        for p in &mut self.parts {
            for (s, e) in &mut p.exprs {
                let v = e.eval(slots, dt);
                slots[*s] = v;
            }
        }
        for (kind, f) in &mut self.handlers {
            for p in self.parts.iter().filter(|p| p.kind == kind.as_str()) {
                let failed = p.own.iter().find(|(n, _)| *n == "failed").is_some_and(|&(_, s)| slots[s] != 0.0);
                if failed {
                    continue;
                }
                let mut part = Part { p, slots, index: &self.index, dt };
                f(&mut part);
            }
        }
    }
}
