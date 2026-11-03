//! EasyAcft part catalogue and the aircraft runtime.

pub mod catalog;
pub mod electrical;
pub mod logic;

use easyacft_core::{Rate, Snapshot};
use easyacft_lang::{check, load, Checked, CheckedPart, Diagnostic, Model, Program, Severity, Sources};
use std::path::Path;

/// A compiled expression with its own timer and latch state.
#[derive(Clone, Debug)]
pub struct Expr {
    pub program: Program,
    pub state: Vec<f64>,
}

impl Expr {
    pub fn new(p: &Program) -> Self {
        Self { program: p.clone(), state: vec![0.0; p.states] }
    }
    pub fn eval(&mut self, slots: &[f64], dt: f64) -> f64 {
        self.program.eval(slots, &mut self.state, dt)
    }
    pub fn yes(&mut self, slots: &[f64], dt: f64) -> bool {
        self.eval(slots, dt) != 0.0
    }
}

pub fn expr_of(p: &CheckedPart, key: &str) -> Option<Expr> {
    p.expr(key).map(Expr::new)
}

/// One simulated domain stepping at its own rate.
pub trait Domain {
    fn name(&self) -> &'static str;
    fn step(&mut self, slots: &mut [f64], dt: f64);
    fn state(&self) -> Vec<f64> {
        Vec::new()
    }
    fn restore(&mut self, _state: &[f64]) {}
}

pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
    pub sources: Sources,
}

impl Report {
    pub fn errors(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.severity == Severity::Error).count()
    }
    pub fn render(&self) -> String {
        self.diagnostics.iter().map(|d| self.sources.render(d)).collect::<Vec<_>>().join("\n")
    }
}

pub struct Aircraft {
    pub name: String,
    pub files: Vec<String>,
    pub bindings: Vec<easyacft_lang::model::Binding>,
    pub checked: Checked,
    pub slots: Vec<f64>,
    domains: Vec<(Rate, Box<dyn Domain>)>,
    pub time: f64,
}

fn rate_for(model: &Model, name: &str, default: f64) -> Rate {
    let period = model.rates.iter().find(|(n, _)| n == name).map_or(default, |(_, p)| *p);
    Rate::new(period, (1.0 / period).ceil() as usize)
}

impl Aircraft {
    pub fn load(path: &Path) -> Result<(Self, Report), Report> {
        let (model, mut diags) = load(path);
        Self::from_model(model, &mut diags)
    }

    pub fn from_model(model: Model, diags: &mut Vec<Diagnostic>) -> Result<(Self, Report), Report> {
        let cat = catalog::Builtin;
        let (checked, more) = check(&model, &cat);
        diags.extend(more);
        let report = Report { diagnostics: std::mem::take(diags), sources: model.sources.clone() };
        if report.errors() > 0 {
            return Err(report);
        }
        let mut slots = vec![0.0; checked.slots.names.len()];
        logic::init(&checked, &mut slots);
        let mut domains: Vec<(Rate, Box<dyn Domain>)> = Vec::new();
        domains.push((rate_for(&model, "logic", 0.01), Box::new(logic::Logic::new(&checked))));
        if let Some(e) = electrical::Electrical::new(&checked) {
            domains.push((rate_for(&model, "electrical", 0.01), Box::new(e)));
        }
        let files = model.sources.files.iter().map(|(n, _)| n.clone()).collect();
        Ok((Self { name: model.name.clone(), files, bindings: model.bindings.clone(), checked, slots, domains, time: 0.0 }, report))
    }

    pub fn step(&mut self, dt: f64) {
        self.time += dt;
        for (rate, d) in &mut self.domains {
            let n = rate.due(dt);
            for _ in 0..n {
                d.step(&mut self.slots, rate.period());
            }
        }
    }

    pub fn slot(&self, name: &str) -> Option<usize> {
        self.checked.slots.index.get(name).copied()
    }

    pub fn get(&self, name: &str) -> Option<f64> {
        self.slot(name).map(|i| self.slots[i])
    }

    pub fn set(&mut self, name: &str, v: f64) -> bool {
        match self.slot(name) {
            Some(i) => {
                self.slots[i] = v;
                true
            }
            None => false,
        }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.checked.slots.names.iter().map(|(n, _)| n.as_str())
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut s = Snapshot::new();
        s.put("slots", self.slots.clone());
        s.put("time", vec![self.time]);
        for (_, d) in &self.domains {
            s.put(d.name(), d.state());
        }
        s
    }

    pub fn restore(&mut self, s: &Snapshot) -> bool {
        match s.get("slots") {
            Some(v) if v.len() == self.slots.len() => self.slots.copy_from_slice(v),
            _ => return false,
        }
        self.time = s.get("time").and_then(|t| t.first().copied()).unwrap_or(0.0);
        for (_, d) in &mut self.domains {
            if let Some(v) = s.get(d.name()) {
                d.restore(v);
            }
        }
        true
    }
}
