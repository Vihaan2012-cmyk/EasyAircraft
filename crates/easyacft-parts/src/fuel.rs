//! Fuel domain: tanks, pumps, lines, valves, engine feeds, quantity indication.

use crate::util::{Packer, Slots};
use crate::{expr_of, Domain, Expr};
use easyacft_core::{Dim, Network};
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};
use std::collections::HashMap;

const M: Type = Type::Num(Dim::MASS);
const MF: Type = Type::Num(Dim::MASS_FLOW);
const P: Type = Type::Num(Dim::PRESSURE);
const L: Type = Type::Num(Dim::LENGTH);
const B: Type = Type::Bool;
const WHEN: Kind = Kind::Expr(Type::Bool);
const NODES: &[&str] = &["fuel.tank", "fuel.line"];
const DROOP: f64 = 0.3;
/// Pressure a full tank's head gives a line through an open gravity path.
const HEAD: f64 = 5_000.0;

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "fuel",
        kind: "tank",
        help: "A fuel tank.",
        fields: &[
            req("capacity", Kind::Quantity(Dim::MASS), "usable fuel when full, for example \"2400 kg\""),
            opt("quantity", Kind::Quantity(Dim::MASS), "0 kg", "fuel at start"),
            opt("arm", Kind::Quantity(Dim::LENGTH), "0 m", "for weight and balance"),
            opt("gauge_error", Kind::Number, "0", "share the gauge reads high (negative: low)"),
        ],
        outputs: &[("quantity", M), ("indicated", M), ("mass", M), ("arm", L), ("low", B), ("empty", B), ("failed", B)],
    },
    PartSpec {
        domain: "fuel",
        kind: "line",
        help: "A fuel line or manifold.",
        fields: &[],
        outputs: &[("pressure", P), ("failed", B)],
    },
    PartSpec {
        domain: "fuel",
        kind: "pump",
        help: "A boost or transfer pump from a tank into a line or another tank.",
        fields: &[
            req("tank", Kind::Ref(&["fuel.tank"]), "the tank it draws from"),
            req("outlet", Kind::Ref(NODES), "the line or tank it pushes into"),
            opt("pressure", Kind::Quantity(Dim::PRESSURE), "25 psi", "outlet pressure at no flow"),
            req("flow", Kind::Quantity(Dim::MASS_FLOW), "flow at full output, for example \"1.5 kg/s\""),
            opt("running_when", WHEN, "true", "when it runs"),
        ],
        outputs: &[("running", B), ("flow", MF), ("low_pressure", B), ("failed", B)],
    },
    PartSpec {
        domain: "fuel",
        kind: "valve",
        help: "A crossfeed, transfer, shutoff or gravity valve; a tank end gives gravity feed.",
        fields: &[req("between", Kind::Refs(NODES), "two line or tank ids"), opt("open_when", WHEN, "true", "when it is open"), opt("flow", Kind::Quantity(Dim::MASS_FLOW), "5 kg/s", "flow at 1 bar across it")],
        outputs: &[("open", B), ("flow", MF), ("failed", B)],
    },
    PartSpec {
        domain: "fuel",
        kind: "feed",
        help: "An engine or APU fuel inlet.",
        fields: &[req("line", Kind::Ref(&["fuel.line"]), "the line it draws from"), req("demand", Kind::Expr(MF), "fuel it asks for, for example \"eng1.fuel_flow\""), opt("minimum", Kind::Quantity(Dim::PRESSURE), "1 psi", "below this the inlet starves")],
        outputs: &[("flow", MF), ("starved", B), ("failed", B)],
    },
];

enum NodeOf {
    Tank(usize),
    Line(usize),
}

struct Tank {
    node: usize,
    capacity: f64,
    quantity: f64,
    error: f64,
    s: Slots,
}

struct Pump {
    tank: usize,
    outlet: usize,
    g: f64,
    pressure: f64,
    when: Option<Expr>,
    flow: f64,
    s: Slots,
}

struct Valve {
    link: usize,
    ends: [usize; 2],
    when: Option<Expr>,
    s: Slots,
}

struct Feed {
    line: usize,
    node: usize,
    demand: Expr,
    minimum: f64,
    s: Slots,
}

pub struct Fuel {
    net: Network,
    tanks: Vec<Tank>,
    lines: Vec<(usize, Slots)>,
    pumps: Vec<Pump>,
    valves: Vec<Valve>,
    feeds: Vec<Feed>,
    node_tank: HashMap<usize, usize>,
}

impl Fuel {
    pub fn new(c: &Checked) -> Option<Self> {
        let parts: Vec<(usize, &CheckedPart)> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "fuel").collect();
        if parts.is_empty() {
            return None;
        }
        let mut net = Network::new();
        let mut f = Fuel { net: Network::new(), tanks: vec![], lines: vec![], pumps: vec![], valves: vec![], feeds: vec![], node_tank: HashMap::new() };
        let mut of: HashMap<usize, NodeOf> = HashMap::new();
        for &(i, p) in &parts {
            match p.kind {
                "tank" => {
                    let node = net.add_node(0.0);
                    f.node_tank.insert(node, f.tanks.len());
                    of.insert(i, NodeOf::Tank(f.tanks.len()));
                    f.tanks.push(Tank { node, capacity: p.num("capacity").max(1e-6), quantity: p.num("quantity").min(p.num("capacity")), error: p.num("gauge_error"), s: Slots::of(p, &["quantity", "indicated", "mass", "arm", "low", "empty", "failed"]) });
                }
                "line" => {
                    of.insert(i, NodeOf::Line(f.lines.len()));
                    f.lines.push((net.add_node(1e-9), Slots::of(p, &["pressure", "failed"])));
                }
                _ => {}
            }
        }
        let node = |f: &Fuel, i: Option<usize>| match i.and_then(|i| of.get(&i)) {
            Some(NodeOf::Tank(t)) => Some(f.tanks[*t].node),
            Some(NodeOf::Line(l)) => Some(f.lines[*l].0),
            None => None,
        };
        for &(_, p) in &parts {
            match p.kind {
                "pump" => {
                    let (Some(NodeOf::Tank(t)), Some(out)) = (p.part("tank").and_then(|i| of.get(&i)), node(&f, p.part("outlet"))) else { continue };
                    let pr = p.num("pressure").max(1.0);
                    f.pumps.push(Pump { tank: *t, outlet: out, g: p.num("flow") / (DROOP * pr), pressure: pr, when: expr_of(p, "running_when"), flow: 0.0, s: Slots::of(p, &["running", "flow", "low_pressure", "failed"]) });
                }
                "valve" => {
                    let e = p.parts("between");
                    let (Some(a), Some(b)) = (node(&f, e.first().copied()), node(&f, e.get(1).copied())) else { continue };
                    let g = p.num("flow") / 1e5;
                    let link = net.add_link(a, b, 1.0 / g.max(1e-12), 0.0);
                    f.valves.push(Valve { link, ends: [a, b], when: expr_of(p, "open_when"), s: Slots::of(p, &["open", "flow", "failed"]) });
                }
                "feed" => {
                    let Some(NodeOf::Line(l)) = p.part("line").and_then(|i| of.get(&i)) else { continue };
                    let Some(demand) = expr_of(p, "demand") else { continue };
                    f.feeds.push(Feed { line: *l, node: f.lines[*l].0, demand, minimum: p.num("minimum"), s: Slots::of(p, &["flow", "starved", "failed"]) });
                }
                _ => {}
            }
        }
        f.net = net;
        Some(f)
    }
}

impl Domain for Fuel {
    fn name(&self) -> &'static str {
        "fuel"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        self.net.begin_step();
        for t in &self.tanks {
            let head = if t.quantity > 0.0 { HEAD * (0.2 + 0.8 * t.quantity / t.capacity) } else { 0.0 };
            self.net.stamp(t.node, easyacft_core::STIFF * 1e-9, easyacft_core::STIFF * 1e-9 * head);
        }
        for v in &mut self.valves {
            let open = !v.s.on(slots, "failed") && v.when.as_mut().is_none_or(|w| w.yes(slots, dt));
            self.net.set_closed(v.link, open);
            v.s.flag(slots, "open", open);
        }
        for p in &mut self.pumps {
            let on = !p.s.on(slots, "failed") && self.tanks[p.tank].quantity > 0.0 && p.when.as_mut().is_none_or(|w| w.yes(slots, dt));
            p.s.flag(slots, "running", on);
            if on {
                self.net.stamp(p.outlet, p.g, p.g * p.pressure);
            }
        }
        let mut want = vec![0.0; self.feeds.len()];
        for (k, fd) in self.feeds.iter_mut().enumerate() {
            want[k] = if fd.s.on(slots, "failed") { 0.0 } else { fd.demand.eval(slots, dt).max(0.0) };
            let p = self.lines[fd.line].1.get(slots, "pressure").max(fd.minimum);
            if want[k] > 0.0 {
                self.net.stamp(fd.node, want[k] / p, 0.0);
            }
        }
        self.net.step(dt);
        let mut out = vec![0.0; self.tanks.len()];
        for p in &mut self.pumps {
            let pout = self.net.potential(p.outlet);
            p.flow = if p.s.on(slots, "running") { (p.g * (p.pressure - pout)).max(0.0) } else { 0.0 };
            out[p.tank] += p.flow;
            if let Some(&t) = self.node_tank.get(&p.outlet) {
                out[t] -= p.flow;
            }
            p.s.put(slots, "flow", p.flow);
            p.s.flag(slots, "low_pressure", p.s.on(slots, "running") && pout < 0.5 * p.pressure);
        }
        for v in &self.valves {
            let q = self.net.flow(v.link);
            v.s.put(slots, "flow", q.abs());
            for (end, sign) in [(v.ends[0], 1.0), (v.ends[1], -1.0)] {
                if let Some(&t) = self.node_tank.get(&end) {
                    out[t] += sign * q;
                }
            }
        }
        for (node, s) in &self.lines {
            s.put(slots, "pressure", self.net.potential(*node).max(0.0));
        }
        for (k, fd) in self.feeds.iter().enumerate() {
            let p = self.lines[fd.line].1.get(slots, "pressure");
            let starved = want[k] > 0.0 && p < fd.minimum;
            fd.s.flag(slots, "starved", starved);
            fd.s.put(slots, "flow", if starved { 0.0 } else { want[k] });
        }
        for (k, t) in self.tanks.iter_mut().enumerate() {
            t.quantity = (t.quantity - out[k] * dt).clamp(0.0, t.capacity);
            t.s.put(slots, "quantity", t.quantity);
            t.s.put(slots, "mass", t.quantity);
            t.s.put(slots, "indicated", if t.s.on(slots, "failed") { 0.0 } else { t.quantity * (1.0 + t.error) });
            t.s.flag(slots, "low", t.quantity < 0.1 * t.capacity);
            t.s.flag(slots, "empty", t.quantity <= 0.0);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut v = self.net.state();
        v.extend(self.tanks.iter().map(|t| t.quantity));
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let n = self.net.state().len();
        if s.len() < n {
            return;
        }
        self.net.restore(&s[..n]);
        let mut p = Packer(s[n..].to_vec(), 0);
        for t in &mut self.tanks {
            t.quantity = p.take();
        }
    }
}

pub fn init(c: &Checked, slots: &mut [f64]) {
    for p in c.parts.iter().filter(|p| p.domain == "fuel" && p.kind == "tank") {
        let q = p.num("quantity").min(p.num("capacity"));
        slots[crate::catalog::out(p, "quantity")] = q;
        slots[crate::catalog::out(p, "mass")] = q;
        slots[crate::catalog::out(p, "arm")] = p.num("arm");
    }
}
