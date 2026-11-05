//! EasyAcft part catalogue and the aircraft runtime.

pub mod actuation;
pub mod aircon;
pub mod apu;
pub mod avionics;
pub mod cabin;
pub mod catalog;
pub mod control;
pub mod custom;
pub mod electrical;
pub mod engine;
pub mod environment;
pub mod extras;
mod files;
pub mod fuel;
pub mod gear;
pub mod hydraulic;
pub mod ice;
pub mod logic;
pub mod oxygen;
pub mod pneumatic;
pub mod ports;
pub mod scenario;
pub mod systems;
pub mod util;

use easyacft_core::{Rate, Snapshot};
use easyacft_lang::{check, Checked, CheckedPart, Diagnostic, Model, Program, Severity, Sources};
use std::path::Path;

/// A compiled expression with its own timer and latch state.
#[derive(Clone, Debug)]
pub struct Expr {
    pub program: Program,
    pub state: Vec<f64>,
    constant: Option<f64>,
}

impl Expr {
    pub fn new(p: &Program) -> Self {
        Self { program: p.clone(), state: vec![0.0; p.states], constant: p.constant() }
    }
    pub fn eval(&mut self, slots: &[f64], dt: f64) -> f64 {
        if let Some(c) = self.constant {
            return c;
        }
        self.program.eval(slots, &mut self.state, dt)
    }
    pub fn yes(&mut self, slots: &[f64], dt: f64) -> bool {
        self.eval(slots, dt) != 0.0
    }
}

pub fn expr_of(p: &CheckedPart, key: &str) -> Option<Expr> {
    p.expr(key).map(Expr::new)
}

/// Time one domain has spent stepping.
#[derive(Clone, Debug, Default)]
pub struct DomainTime {
    pub name: &'static str,
    pub seconds: f64,
    pub steps: u64,
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

/// A maintenance log line: when a part failed or a fault came up.
#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub time: f64,
    pub what: String,
    pub on_ground: bool,
}

pub struct Aircraft {
    pub log: Vec<LogEntry>,
    watched: Vec<(usize, String, bool)>,
    ground: Option<usize>,
    pub name: String,
    pub files: Vec<String>,
    pub bindings: Vec<easyacft_lang::model::Binding>,
    pub checked: Checked,
    pub slots: Vec<f64>,
    domains: Vec<(Rate, Box<dyn Domain>)>,
    timing_on: bool,
    timing: Option<Vec<DomainTime>>,
    custom: (Rate, custom::Custom),
    pub time: f64,
}

fn rate_for(rates: &[(String, f64)], name: &str, default: f64) -> Rate {
    let period = rates.iter().find(|(n, _)| n == name).map_or(default, |(_, p)| *p);
    Rate::new(period, (1.0 / period).ceil() as usize)
}

impl Aircraft {
    pub fn from_model(model: Model, diags: &mut Vec<Diagnostic>) -> Result<(Self, Report), Report> {
        let own = custom::specs(&model.kinds, diags);
        catalog::register(&own);
        let cat = catalog::With::new(&own);
        let (checked, more) = check(&model, &cat);
        diags.extend(more);
        let report = Report { diagnostics: std::mem::take(diags), sources: model.sources.clone() };
        if report.errors() > 0 {
            return Err(report);
        }
        let base = model.sources.files.first().and_then(|(n, _)| Path::new(n).parent().map(|p| p.to_path_buf())).unwrap_or_default();
        let files = model.sources.files.iter().map(|(n, _)| n.clone()).collect();
        Ok((Self::from_checked(checked, &model.name, &model.rates, model.bindings.clone(), files, &base), report))
    }

    /// Builds a checked aircraft; `base` is the folder table files are read from.
    pub fn from_checked(checked: Checked, name: &str, rates: &[(String, f64)], bindings: Vec<easyacft_lang::model::Binding>, files: Vec<String>, base: &Path) -> Self {
        let mut slots = vec![0.0; checked.slots.names.len()];
        logic::init(&checked, &mut slots);
        fuel::init(&checked, &mut slots);
        systems::init(&checked, &mut slots);
        avionics::init(&checked, &mut slots);
        let mut domains: Vec<(Rate, Box<dyn Domain>)> = Vec::new();
        domains.push((rate_for(rates, "logic", 0.01), Box::new(logic::Logic::new(&checked))));
        if let Some(e) = electrical::Electrical::new(&checked) {
            domains.push((rate_for(rates, "electrical", 0.01), Box::new(e)));
        }
        if let Some(d) = hydraulic::Hydraulic::new(&checked) {
            domains.push((rate_for(rates, "hydraulic", 0.005), Box::new(d)));
        }
        if let Some(d) = pneumatic::Pneumatic::new(&checked) {
            domains.push((rate_for(rates, "pneumatic", 0.02), Box::new(d)));
        }
        if let Some(d) = fuel::Fuel::new(&checked) {
            domains.push((rate_for(rates, "fuel", 0.05), Box::new(d)));
        }
        if let Some(d) = engine::Engines::new(&checked, base) {
            domains.push((rate_for(rates, "engines", 0.01), Box::new(d)));
        }
        if let Some(d) = control::Control::new(&checked, base) {
            domains.push((rate_for(rates, "control", 0.02), Box::new(d)));
        }
        if let Some(d) = avionics::Avionics::new(&checked) {
            domains.push((rate_for(rates, "avionics", 0.02), Box::new(d)));
        }
        if let Some(d) = extras::Extras::new(&checked) {
            domains.push((rate_for(rates, "extras", 0.02), Box::new(d)));
        }
        if let Some(d) = systems::Systems::new(&checked) {
            domains.push((rate_for(rates, "systems", 0.02), Box::new(d)));
        }
        if let Some(d) = actuation::Actuation::new(&checked) {
            domains.push((rate_for(rates, "actuation", 0.005), Box::new(d)));
        }
        if let Some(d) = aircon::AirCon::new(&checked) {
            domains.push((rate_for(rates, "aircon", 0.02), Box::new(d)));
        }
        let custom = (rate_for(rates, "custom", 0.01), custom::Custom::new(&checked));
        let watched = checked.slots.names.iter().enumerate().filter(|(_, (n, _))| [".failed", ".fault", ".aborted", ".shutdown", ".tripped", ".detected"].iter().any(|s| n.ends_with(s))).map(|(i, (n, _))| (i, n.clone(), false)).collect();
        let ground = checked.parts.iter().find(|p| p.domain == "environment").and_then(|p| checked.slots.index.get(&format!("{}.on_ground", p.id)).copied());
        Self { log: Vec::new(), watched, ground, name: name.to_owned(), files, bindings, checked, slots, domains, timing_on: false, timing: None, custom, time: 0.0 }
    }

    /// Steps every `[[custom.<kind>]]` part with `f`; returns how many there are.
    pub fn on_kind(&mut self, kind: &str, f: impl FnMut(&mut custom::Part<'_>) + 'static) -> usize {
        self.custom.1.on_kind(kind, Box::new(f))
    }

    pub fn step(&mut self, dt: f64) {
        self.time += dt;
        let mut timing = self.timing.take();
        let mut clock = |k: usize, name: &'static str, t: Option<std::time::Instant>, n: usize| {
            if let (Some(tm), Some(t)) = (timing.as_mut(), t) {
                if tm.len() <= k {
                    tm.resize(k + 1, DomainTime::default());
                }
                tm[k].name = name;
                tm[k].seconds += t.elapsed().as_secs_f64();
                tm[k].steps += n as u64;
            }
        };
        let now = || self.timing_on.then(std::time::Instant::now);
        let (rate, c) = &mut self.custom;
        let t = now();
        let n = rate.due(dt);
        for _ in 0..n {
            c.step(&mut self.slots, rate.period());
        }
        clock(0, "custom", t, n);
        for (k, (rate, d)) in self.domains.iter_mut().enumerate() {
            let t = now();
            let n = rate.due(dt);
            for _ in 0..n {
                d.step(&mut self.slots, rate.period());
            }
            clock(k + 1, d.name(), t, n);
        }
        self.timing = timing;
        self.watch();
    }

    /// Starts timing every domain's steps from zero.
    pub fn start_profile(&mut self) {
        self.timing_on = true;
        self.timing = Some(Vec::new());
    }

    /// Wall-clock time spent in each domain since `start_profile`.
    pub fn profile(&self) -> &[DomainTime] {
        self.timing.as_deref().unwrap_or(&[])
    }

    fn watch(&mut self) {
        let on_ground = self.ground.is_some_and(|g| self.slots[g] != 0.0);
        for (i, name, last) in &mut self.watched {
            let now = self.slots[*i] != 0.0;
            if now && !*last {
                self.log.push(LogEntry { time: self.time, what: name.clone(), on_ground });
            }
            *last = now;
        }
    }

    /// Wear counters as `name value` lines, for keeping between flights.
    pub fn persistent(&self) -> String {
        self.checked.parts.iter().filter(|p| p.domain == "wear").filter_map(|p| self.get(&format!("{}.value", p.id)).map(|v| format!("{}.value {v}
", p.id))).collect()
    }

    pub fn restore_persistent(&mut self, text: &str) -> usize {
        let mut n = 0;
        for line in text.lines() {
            if let Some((k, v)) = line.split_once(' ') {
                if let Ok(v) = v.trim().parse::<f64>() {
                    if k.ends_with(".value") && self.set(k, v) {
                        n += 1;
                    }
                }
            }
        }
        n
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
