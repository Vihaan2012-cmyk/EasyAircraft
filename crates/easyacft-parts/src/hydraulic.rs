//! Hydraulic domain: circuits, pumps, accumulators, consumers, PTUs, reservoirs.

use crate::util::{Packer, Slots};
use crate::{expr_of, Domain, Expr};
use easyacft_core::{Dim, Network};
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, Type};
use std::collections::HashMap;

const P: Type = Type::Num(Dim::PRESSURE);
const Q: Type = Type::Num(Dim::VOLUME_FLOW);
const VOL: Type = Type::Num(Dim::VOLUME);
const B: Type = Type::Bool;
const WHEN: Kind = Kind::Expr(Type::Bool);
const CIRCUIT: &[&str] = &["hydraulic.circuit"];
/// Pump outlet pressure drop at full flow, share of nominal.
const DROOP: f64 = 0.15;

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "hydraulic",
        kind: "circuit",
        help: "One hydraulic system with its reservoir.",
        fields: &[
            req("nominal", Kind::Quantity(Dim::PRESSURE), "for example \"3000 psi\""),
            req("fluid", Kind::Quantity(Dim::VOLUME), "reservoir quantity at start, for example \"20 L\""),
            opt("low_pressure", Kind::Quantity(Dim::PRESSURE), "1500 psi", "below this the circuit counts as lost"),
            opt("compliance", Kind::Number, "1e-10", "volume per pascal the lines and hoses store"),
            opt("leak", Kind::Quantity(Dim::VOLUME_FLOW), "0.05 L/min", "internal leakage at nominal pressure"),
        ],
        outputs: &[("pressure", P), ("fluid", VOL), ("available", B), ("low", B), ("failed", B)],
    },
    PartSpec {
        domain: "hydraulic",
        kind: "pump",
        help: "An engine-driven, electric or wind-driven pump.",
        fields: &[
            req("circuit", Kind::Ref(CIRCUIT), "the circuit it feeds"),
            req("pressure", Kind::Quantity(Dim::PRESSURE), "outlet pressure at no flow"),
            req("flow", Kind::Quantity(Dim::VOLUME_FLOW), "flow it delivers at full output, for example \"140 L/min\""),
            opt("running_when", WHEN, "true", "when it turns"),
        ],
        outputs: &[("running", B), ("flow", Q), ("pressure", P), ("failed", B)],
    },
    PartSpec {
        domain: "hydraulic",
        kind: "accumulator",
        help: "A gas-charged accumulator.",
        fields: &[req("circuit", Kind::Ref(CIRCUIT), "the circuit it is on"), req("volume", Kind::Quantity(Dim::VOLUME), "gas volume"), req("precharge", Kind::Quantity(Dim::PRESSURE), "gas precharge pressure")],
        outputs: &[("pressure", P), ("failed", B)],
    },
    PartSpec {
        domain: "hydraulic",
        kind: "consumer",
        help: "Actuators or a motor that use flow while they move.",
        fields: &[
            req("circuit", Kind::Ref(CIRCUIT), "the circuit it uses"),
            req("flow", Kind::Quantity(Dim::VOLUME_FLOW), "flow while moving at full rate"),
            opt("moving_when", WHEN, "false", "when it is moving"),
            opt("minimum", Kind::Quantity(Dim::PRESSURE), "1000 psi", "lowest pressure it works on"),
        ],
        outputs: &[("powered", B), ("flow", Q), ("failed", B)],
    },
    PartSpec {
        domain: "hydraulic",
        kind: "ptu",
        help: "A power transfer unit between two circuits.",
        fields: &[
            req("between", Kind::Refs(CIRCUIT), "two circuit ids"),
            opt("threshold", Kind::Quantity(Dim::PRESSURE), "500 psi", "pressure difference that starts it"),
            req("flow", Kind::Quantity(Dim::VOLUME_FLOW), "flow it can move"),
            opt("enabled_when", WHEN, "true", "when it is armed"),
        ],
        outputs: &[("running", B), ("flow", Q), ("failed", B)],
    },
];

struct Circuit {
    node: usize,
    nominal: f64,
    low: f64,
    fluid: f64,
    leak_g: f64,
    s: Slots,
}

struct Pump {
    circuit: usize,
    g: f64,
    pressure: f64,
    when: Option<Expr>,
    s: Slots,
}

struct Consumer {
    circuit: usize,
    flow: f64,
    minimum: f64,
    when: Option<Expr>,
    s: Slots,
}

struct Ptu {
    a: usize,
    b: usize,
    link: usize,
    threshold: f64,
    when: Option<Expr>,
    on: bool,
    s: Slots,
}

pub struct Hydraulic {
    net: Network,
    circuits: Vec<Circuit>,
    pumps: Vec<Pump>,
    accumulators: Vec<(usize, f64, f64, Slots)>,
    consumers: Vec<Consumer>,
    ptus: Vec<Ptu>,
}

impl Hydraulic {
    pub fn new(c: &Checked) -> Option<Self> {
        let parts: Vec<(usize, &easyacft_lang::CheckedPart)> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "hydraulic").collect();
        if parts.is_empty() {
            return None;
        }
        let mut net = Network::new();
        let mut of: HashMap<usize, usize> = HashMap::new();
        let mut h = Hydraulic { net: Network::new(), circuits: vec![], pumps: vec![], accumulators: vec![], consumers: vec![], ptus: vec![] };
        for &(i, p) in parts.iter().filter(|(_, p)| p.kind == "circuit") {
            let mut cap = p.num("compliance").max(0.0);
            for &(_, a) in parts.iter().filter(|(_, a)| a.kind == "accumulator" && a.part("circuit") == Some(i)) {
                cap += a.num("volume") / (1.4 * p.num("nominal").max(1.0));
            }
            of.insert(i, h.circuits.len());
            h.circuits.push(Circuit {
                node: net.add_node(cap),
                nominal: p.num("nominal"),
                low: p.num("low_pressure"),
                fluid: p.num("fluid"),
                leak_g: p.num("leak") / p.num("nominal").max(1.0),
                s: Slots::of(p, &["pressure", "fluid", "available", "low", "failed"]),
            });
        }
        let circ = |p: &easyacft_lang::CheckedPart| p.part("circuit").and_then(|x| of.get(&x).copied());
        for &(_, p) in &parts {
            match p.kind {
                "pump" => {
                    let Some(ci) = circ(p) else { continue };
                    let pr = p.num("pressure").max(1.0);
                    h.pumps.push(Pump { circuit: ci, g: p.num("flow") / (DROOP * pr), pressure: pr, when: expr_of(p, "running_when"), s: Slots::of(p, &["running", "flow", "pressure", "failed"]) });
                }
                "accumulator" => {
                    if let Some(ci) = circ(p) {
                        h.accumulators.push((ci, p.num("precharge"), p.num("volume"), Slots::of(p, &["pressure", "failed"])));
                    }
                }
                "consumer" => {
                    let Some(ci) = circ(p) else { continue };
                    h.consumers.push(Consumer { circuit: ci, flow: p.num("flow"), minimum: p.num("minimum"), when: expr_of(p, "moving_when"), s: Slots::of(p, &["powered", "flow", "failed"]) });
                }
                "ptu" => {
                    let ends = p.parts("between");
                    let (Some(&a), Some(&b)) = (ends.first().and_then(|x| of.get(x)), ends.get(1).and_then(|x| of.get(x))) else { continue };
                    let g = p.num("flow") / p.num("threshold").max(1.0);
                    let link = net.add_link(h.circuits[a].node, h.circuits[b].node, 1.0 / g.max(1e-15), 0.0);
                    net.set_closed(link, false);
                    h.ptus.push(Ptu { a, b, link, threshold: p.num("threshold"), when: expr_of(p, "enabled_when"), on: false, s: Slots::of(p, &["running", "flow", "failed"]) });
                }
                _ => {}
            }
        }
        h.net = net;
        Some(h)
    }
}

impl Domain for Hydraulic {
    fn name(&self) -> &'static str {
        "hydraulic"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        self.net.begin_step();
        for c in &self.circuits {
            let g = if c.s.on(slots, "failed") { c.leak_g * 1e4 } else { c.leak_g };
            self.net.stamp(c.node, g, 0.0);
        }
        let mut running = vec![false; self.pumps.len()];
        for (k, p) in self.pumps.iter_mut().enumerate() {
            let fluid = self.circuits[p.circuit].fluid > 0.0;
            running[k] = !p.s.on(slots, "failed") && fluid && p.when.as_mut().is_none_or(|w| w.yes(slots, dt));
            p.s.flag(slots, "running", running[k]);
            if running[k] {
                self.net.stamp(self.circuits[p.circuit].node, p.g, p.g * p.pressure);
            }
        }
        let mut demand = vec![0.0; self.consumers.len()];
        for (k, u) in self.consumers.iter_mut().enumerate() {
            let c = &self.circuits[u.circuit];
            let p = c.s.get(slots, "pressure");
            let moving = !u.s.on(slots, "failed") && u.when.as_mut().is_some_and(|w| w.yes(slots, dt));
            if moving && p > 0.0 {
                demand[k] = u.flow;
                self.net.stamp(c.node, u.flow / c.nominal.max(1.0), 0.0);
            }
        }
        for t in &mut self.ptus {
            let (pa, pb) = (self.circuits[t.a].s.get(slots, "pressure"), self.circuits[t.b].s.get(slots, "pressure"));
            let armed = !t.s.on(slots, "failed") && t.when.as_mut().is_none_or(|w| w.yes(slots, dt));
            let dp = (pa - pb).abs();
            t.on = armed && (dp > t.threshold || t.on && dp > 0.3 * t.threshold);
            self.net.set_closed(t.link, t.on);
            t.s.flag(slots, "running", t.on);
        }
        self.net.step(dt);
        for c in &mut self.circuits {
            let p = self.net.potential(c.node).max(0.0);
            c.s.put(slots, "pressure", p);
            c.s.flag(slots, "available", p >= c.low);
            c.s.flag(slots, "low", p < c.low);
            if c.s.on(slots, "failed") {
                c.fluid = (c.fluid - c.leak_g * 1e4 * p * dt).max(0.0);
            }
            c.s.put(slots, "fluid", c.fluid);
        }
        for (k, p) in self.pumps.iter().enumerate() {
            let c = &self.circuits[p.circuit];
            let pc = self.net.potential(c.node);
            p.s.put(slots, "flow", if running[k] { (p.g * (p.pressure - pc)).max(0.0) } else { 0.0 });
            p.s.put(slots, "pressure", if running[k] { pc.max(0.0) } else { 0.0 });
        }
        for (k, u) in self.consumers.iter().enumerate() {
            let p = self.circuits[u.circuit].s.get(slots, "pressure");
            u.s.flag(slots, "powered", p >= u.minimum && !u.s.on(slots, "failed"));
            u.s.put(slots, "flow", demand[k] * (p / self.circuits[u.circuit].nominal.max(1.0)).min(1.0));
        }
        for t in &self.ptus {
            t.s.put(slots, "flow", self.net.flow(t.link).abs());
        }
        for (ci, pre, _, s) in &self.accumulators {
            let p = self.circuits[*ci].s.get(slots, "pressure");
            s.put(slots, "pressure", p.max(*pre));
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut v = self.net.state();
        v.extend(self.circuits.iter().map(|c| c.fluid));
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let n = self.net.state().len();
        if s.len() < n {
            return;
        }
        self.net.restore(&s[..n]);
        let mut p = Packer(s[n..].to_vec(), 0);
        for c in &mut self.circuits {
            c.fluid = p.take();
        }
    }
}
