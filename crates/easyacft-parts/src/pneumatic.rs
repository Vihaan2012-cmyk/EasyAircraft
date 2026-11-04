//! Pneumatic domain: bleed sources, ducts, regulating valves, precoolers, consumers.

use crate::util::Slots;
use crate::{expr_of, Domain, Expr};
use easyacft_core::{Dim, Network};
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};
use std::collections::HashMap;

const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const MF: Type = Type::Num(Dim::MASS_FLOW);
const B: Type = Type::Bool;
const WHEN: Kind = Kind::Expr(Type::Bool);
const DUCT: &[&str] = &["pneumatic.duct"];
const R_AIR: f64 = 287.05;
const AMBIENT_K: f64 = 288.15;

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "pneumatic",
        kind: "duct",
        help: "A bleed duct or manifold.",
        fields: &[opt("volume", Kind::Quantity(Dim::VOLUME), "0.05 m3", "air volume it holds")],
        outputs: &[("pressure", P), ("temperature", T), ("failed", B)],
    },
    PartSpec {
        domain: "pneumatic",
        kind: "source",
        help: "An engine bleed port, APU load compressor or ground air cart.",
        fields: &[
            req("duct", Kind::Ref(DUCT), "the duct it feeds"),
            req("pressure", Kind::Quantity(Dim::PRESSURE), "delivery pressure at no flow"),
            req("temperature", Kind::Quantity(Dim::TEMPERATURE), "delivery temperature"),
            req("flow", Kind::Expr(MF), "flow at full output, or an expression"),
            opt("available_when", WHEN, "true", "when it can supply"),
        ],
        outputs: &[("available", B), ("flow", MF), ("failed", B)],
    },
    PartSpec {
        domain: "pneumatic",
        kind: "valve",
        help: "A shutoff, crossbleed or pressure-regulating valve, with an optional precooler.",
        fields: &[
            req("between", Kind::Refs(DUCT), "upstream and downstream duct"),
            opt("open_when", WHEN, "true", "when it is open"),
            opt("flow", Kind::Quantity(Dim::MASS_FLOW), "2 kg/s", "flow at 1 bar across it"),
            opt("regulate", Kind::Quantity(Dim::PRESSURE), "0 Pa", "downstream pressure it holds; 0 for a plain valve"),
            opt("cool_to", Kind::Quantity(Dim::TEMPERATURE), "0 K", "precooler outlet temperature; 0 for none"),
        ],
        outputs: &[("open", B), ("flow", MF), ("failed", B)],
    },
    PartSpec {
        domain: "pneumatic",
        kind: "consumer",
        help: "A pack, anti-ice valve, engine starter or other user of bleed air.",
        fields: &[
            req("duct", Kind::Ref(DUCT), "the duct it draws from"),
            req("flow", Kind::Expr(MF), "flow it takes at nominal pressure, or an expression"),
            opt("nominal", Kind::Quantity(Dim::PRESSURE), "45 psi", "pressure its flow is given at"),
            opt("minimum", Kind::Quantity(Dim::PRESSURE), "15 psi", "below this it is not supplied"),
            opt("on_when", WHEN, "true", "when it draws"),
        ],
        outputs: &[("supplied", B), ("flow", MF), ("temperature", T), ("failed", B)],
    },
];

struct Duct {
    node: usize,
    temp: f64,
    s: Slots,
}

struct Source {
    duct: usize,
    g: f64,
    flow_e: Option<Expr>,
    pressure: f64,
    temp: f64,
    when: Option<Expr>,
    s: Slots,
}

struct Valve {
    link: usize,
    ends: [usize; 2],
    r_open: f64,
    regulate: f64,
    cool_to: f64,
    opening: f64,
    when: Option<Expr>,
    s: Slots,
}

struct Consumer {
    duct: usize,
    flow: f64,
    flow_e: Option<Expr>,
    nominal: f64,
    minimum: f64,
    when: Option<Expr>,
    s: Slots,
}

pub struct Pneumatic {
    net: Network,
    ducts: Vec<Duct>,
    sources: Vec<Source>,
    valves: Vec<Valve>,
    consumers: Vec<Consumer>,
}

impl Pneumatic {
    pub fn new(c: &Checked) -> Option<Self> {
        let parts: Vec<(usize, &CheckedPart)> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "pneumatic").collect();
        if parts.is_empty() {
            return None;
        }
        let mut net = Network::new();
        let mut of: HashMap<usize, usize> = HashMap::new();
        let mut n = Pneumatic { net: Network::new(), ducts: vec![], sources: vec![], valves: vec![], consumers: vec![] };
        for &(i, p) in parts.iter().filter(|(_, p)| p.kind == "duct") {
            of.insert(i, n.ducts.len());
            n.ducts.push(Duct { node: net.add_node(p.num("volume") / (R_AIR * 450.0)), temp: AMBIENT_K, s: Slots::of(p, &["pressure", "temperature", "failed"]) });
        }
        let duct = |p: &CheckedPart| p.part("duct").and_then(|x| of.get(&x).copied());
        for &(_, p) in &parts {
            match p.kind {
                "source" => {
                    let Some(d) = duct(p) else { continue };
                    let pr = p.num("pressure").max(1.0);
                    n.sources.push(Source { duct: d, g: 0.0, flow_e: expr_of(p, "flow"), pressure: pr, temp: p.num("temperature"), when: expr_of(p, "available_when"), s: Slots::of(p, &["available", "flow", "failed"]) });
                }
                "valve" => {
                    let e = p.parts("between");
                    let (Some(&a), Some(&b)) = (e.first().and_then(|x| of.get(x)), e.get(1).and_then(|x| of.get(x))) else { continue };
                    let r = 1e5 / p.num("flow").max(1e-9);
                    let link = net.add_link(n.ducts[a].node, n.ducts[b].node, r, 0.0);
                    n.valves.push(Valve { link, ends: [a, b], r_open: r, regulate: p.num("regulate"), cool_to: p.num("cool_to"), opening: 1.0, when: expr_of(p, "open_when"), s: Slots::of(p, &["open", "flow", "failed"]) });
                }
                "consumer" => {
                    let Some(d) = duct(p) else { continue };
                    n.consumers.push(Consumer { duct: d, flow: 0.0, flow_e: expr_of(p, "flow"), nominal: p.num("nominal").max(1.0), minimum: p.num("minimum"), when: expr_of(p, "on_when"), s: Slots::of(p, &["supplied", "flow", "temperature", "failed"]) });
                }
                _ => {}
            }
        }
        n.net = net;
        Some(n)
    }
}

impl Domain for Pneumatic {
    fn name(&self) -> &'static str {
        "pneumatic"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        self.net.begin_step();
        let mut on = vec![false; self.sources.len()];
        for (k, s) in self.sources.iter_mut().enumerate() {
            s.g = s.flow_e.as_mut().map_or(0.0, |e| e.eval(slots, dt).max(0.0)) / (0.3 * s.pressure);
            on[k] = !s.s.on(slots, "failed") && s.when.as_mut().is_none_or(|w| w.yes(slots, dt));
            s.s.flag(slots, "available", on[k]);
            if on[k] {
                self.net.stamp(self.ducts[s.duct].node, s.g, s.g * s.pressure);
            }
        }
        for v in &mut self.valves {
            let open = !v.s.on(slots, "failed") && v.when.as_mut().is_none_or(|w| w.yes(slots, dt));
            if v.regulate > 0.0 && open {
                let down = self.ducts[v.ends[1]].s.get(slots, "pressure");
                let err = (v.regulate - down) / v.regulate;
                v.opening = (v.opening + err * 4.0 * dt).clamp(0.02, 1.0);
            } else {
                v.opening = 1.0;
            }
            self.net.set_closed(v.link, open);
            v.s.flag(slots, "open", open);
        }
        let mut drawing = vec![false; self.consumers.len()];
        for (k, c) in self.consumers.iter_mut().enumerate() {
            c.flow = c.flow_e.as_mut().map_or(0.0, |e| e.eval(slots, dt).max(0.0));
            drawing[k] = !c.s.on(slots, "failed") && c.when.as_mut().is_none_or(|w| w.yes(slots, dt));
            if drawing[k] {
                self.net.stamp(self.ducts[c.duct].node, c.flow / c.nominal, 0.0);
            }
        }
        for d in &self.ducts {
            if d.s.on(slots, "failed") {
                self.net.stamp(d.node, 1e-4, 0.0);
            }
        }
        self.apply_openings();
        self.net.step(dt);
        let mut heat_in = vec![(0.0, 0.0); self.ducts.len()];
        for (k, s) in self.sources.iter().enumerate() {
            let p = self.net.potential(self.ducts[s.duct].node);
            let q = if on[k] { (s.g * (s.pressure - p)).max(0.0) } else { 0.0 };
            s.s.put(slots, "flow", q);
            heat_in[s.duct].0 += q;
            heat_in[s.duct].1 += q * s.temp;
        }
        for v in &self.valves {
            let q = self.net.flow(v.link);
            v.s.put(slots, "flow", q.abs());
            let (from, to) = if q >= 0.0 { (v.ends[0], v.ends[1]) } else { (v.ends[1], v.ends[0]) };
            let mut t = self.ducts[from].temp;
            if v.cool_to > 0.0 {
                t = t.min(v.cool_to);
            }
            heat_in[to].0 += q.abs();
            heat_in[to].1 += q.abs() * t;
        }
        for (k, d) in self.ducts.iter_mut().enumerate() {
            let p = self.net.potential(d.node).max(0.0);
            let (m, h) = heat_in[k];
            let target = if m > 1e-6 { h / m } else { AMBIENT_K };
            let tau = if m > 1e-6 { 0.5 } else { 30.0 };
            d.temp += (target - d.temp) * (1.0 - (-dt / tau).exp());
            d.s.put(slots, "pressure", p);
            d.s.put(slots, "temperature", d.temp);
        }
        for (k, c) in self.consumers.iter().enumerate() {
            let p = self.ducts[c.duct].s.get(slots, "pressure");
            let q = if drawing[k] { c.flow * p / c.nominal } else { 0.0 };
            c.s.flag(slots, "supplied", drawing[k] && p >= c.minimum);
            c.s.put(slots, "flow", q);
            c.s.put(slots, "temperature", self.ducts[c.duct].temp);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut v = self.net.state();
        v.extend(self.ducts.iter().map(|d| d.temp));
        v.extend(self.valves.iter().map(|x| x.opening));
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let n = self.net.state().len();
        if s.len() < n + self.ducts.len() + self.valves.len() {
            return;
        }
        self.net.restore(&s[..n]);
        for (k, d) in self.ducts.iter_mut().enumerate() {
            d.temp = s[n + k];
        }
        let m = n + self.ducts.len();
        for (k, v) in self.valves.iter_mut().enumerate() {
            v.opening = s[m + k];
        }
    }
}

impl Pneumatic {
    fn apply_openings(&mut self) {
        for v in &self.valves {
            self.net.set_resistance(v.link, v.r_open / v.opening.max(1e-3));
        }
    }
}
