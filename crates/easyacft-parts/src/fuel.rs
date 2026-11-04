//! Fuel domain: tanks with fuel temperature, freezing, density, unusable fuel and overflow; pumps; lines that can
//! leak; motorised and check valves; engine feeds with suction feed; quantity indication.

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
const T: Type = Type::Num(Dim::TEMPERATURE);
const W: Type = Type::Num(Dim::POWER);
const VOL: Type = Type::Num(Dim::VOLUME);
const N: Type = Type::Num(Dim::NONE);
const B: Type = Type::Bool;
const WHEN: Kind = Kind::Expr(Type::Bool);
const NODES: &[&str] = &["fuel.tank", "fuel.line"];
const TANK: &[&str] = &["fuel.tank"];
const DROOP: f64 = 0.3;
/// Pressure a full tank's head gives a line through an open gravity path.
const HEAD: f64 = 5_000.0;
const FUEL_CP: f64 = 2000.0;
/// Tank structure counted as fuel when the tank is nearly dry, kg.
const STRUCTURE_KG: f64 = 50.0;
/// Fuel waxes over this many kelvin above its freezing point, and pumps lose flow as it does.
const WAX_BAND: f64 = 4.0;
/// Fuel density change per kelvin, share of its density at 15 °C.
const EXPANSION: f64 = 0.00099;
const T_REF: f64 = 288.15;
/// Suction an engine-driven pump pulls on its line when it feeds without boost pressure.
const SUCTION: f64 = 30_000.0;

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "fuel",
        kind: "tank",
        help: "A fuel tank: fuel temperature from the skin and from heat dumped into it, freezing, density and volume, unusable fuel and overflow to a surge tank.",
        fields: &[
            req("capacity", Kind::Quantity(Dim::MASS), "usable fuel when full, for example \"2400 kg\""),
            opt("quantity", Kind::Quantity(Dim::MASS), "0 kg", "fuel at start"),
            opt("arm", Kind::Quantity(Dim::LENGTH), "0 m", "for weight and balance"),
            opt("gauge_error", Kind::Number, "0", "share the gauge reads high (negative: low)"),
            opt("unusable", Kind::Quantity(Dim::MASS), "0 kg", "fuel the pumps cannot reach"),
            opt("overflow_to", Kind::Ref(TANK), "", "surge or vent tank that takes what overflows; without one it spills overboard"),
            opt("temperature", Kind::Quantity(Dim::TEMPERATURE), "288.15 K", "fuel temperature at start"),
            opt("skin_temperature", Kind::Expr(T), "288.15 K", "temperature of the skin and air around the tank"),
            opt("heat", Kind::Expr(W), "0 W", "heat put into the fuel, such as from hydraulic and drive oil coolers"),
            opt("cooling", Kind::Number, "0", "heat loss to the skin, W per K; 0 means 0.1 W/K per kg of capacity"),
            opt("freeze_point", Kind::Quantity(Dim::TEMPERATURE), "233.15 K", "fuel freezing point; -40 °C for Jet A, -47 °C for Jet A-1"),
            opt("density", Kind::Number, "804", "fuel density at 15 °C, kg/m³"),
        ],
        outputs: &[("quantity", M), ("indicated", M), ("mass", M), ("arm", L), ("volume", VOL), ("temperature", T), ("low", B), ("empty", B), ("frozen", B), ("spilled", M), ("failed", B)],
    },
    PartSpec {
        domain: "fuel",
        kind: "line",
        help: "A fuel line or manifold.",
        fields: &[opt("leak_when", WHEN, "false", "a leak overboard"), opt("leak_rate", Kind::Quantity(Dim::MASS_FLOW), "0.2 kg/s", "that leak at 1 bar")],
        outputs: &[("pressure", P), ("temperature", T), ("leak_flow", MF), ("failed", B)],
    },
    PartSpec {
        domain: "fuel",
        kind: "pump",
        help: "A boost or transfer pump from a tank into a line or another tank.",
        fields: &[
            req("tank", Kind::Ref(TANK), "the tank it draws from"),
            req("outlet", Kind::Ref(NODES), "the line or tank it pushes into"),
            opt("pressure", Kind::Quantity(Dim::PRESSURE), "25 psi", "outlet pressure at no flow"),
            req("flow", Kind::Expr(MF), "flow at full output, for example \"1.5 kg/s\", or an expression"),
            opt("running_when", WHEN, "true", "when it runs"),
            opt("efficiency", Kind::Number, "0.3", "hydraulic power out per electrical power in"),
            opt("inlet_uncovered_when", WHEN, "false", "its inlet is out of the fuel, as in a steep attitude"),
        ],
        outputs: &[("running", B), ("flow", MF), ("power", W), ("low_pressure", B), ("failed", B)],
    },
    PartSpec {
        domain: "fuel",
        kind: "valve",
        help: "A crossfeed, transfer, shutoff or gravity valve; a tank end gives gravity feed.",
        fields: &[
            req("between", Kind::Refs(NODES), "two line or tank ids"),
            opt("open_when", WHEN, "true", "when it is commanded open"),
            opt("flow", Kind::Quantity(Dim::MASS_FLOW), "5 kg/s", "flow at 1 bar across it"),
            opt("open_time", Kind::Quantity(Dim::TIME), "0 s", "motor travel time"),
            opt("stuck_when", WHEN, "false", "its motor has failed where it is"),
            opt("check", Kind::Bool, "false", "a check valve: flow only from the first to the second"),
        ],
        outputs: &[("open", B), ("position", N), ("flow", MF), ("failed", B)],
    },
    PartSpec {
        domain: "fuel",
        kind: "feed",
        help: "An engine or APU fuel inlet.",
        fields: &[
            req("line", Kind::Ref(&["fuel.line"]), "the line it draws from"),
            req("demand", Kind::Expr(MF), "fuel it asks for, for example \"eng1.fuel_flow\""),
            opt("minimum", Kind::Quantity(Dim::PRESSURE), "1 psi", "below this the inlet starves"),
            opt("suction_when", WHEN, "false", "the engine pump can suck fuel with the boost pumps off, as at low altitude"),
        ],
        outputs: &[("flow", MF), ("starved", B), ("temperature", T), ("failed", B)],
    },
];

fn eval(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: f64) -> f64 {
    e.as_mut().map_or(or, |x| x.eval(slots, dt))
}

fn yes(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: bool) -> bool {
    e.as_mut().map_or(or, |x| x.yes(slots, dt))
}

enum NodeOf {
    Tank(usize),
    Line(usize),
}

struct Tank {
    node: usize,
    capacity: f64,
    quantity: f64,
    error: f64,
    unusable: f64,
    overflow: Option<usize>,
    spilled: f64,
    temp: f64,
    skin: Option<Expr>,
    heat: Option<Expr>,
    cooling: f64,
    freeze: f64,
    density: f64,
    s: Slots,
}

struct Line {
    node: usize,
    leak_when: Option<Expr>,
    leak_g: f64,
    leaking: bool,
    temp: f64,
    s: Slots,
}

struct Pump {
    tank: usize,
    outlet: usize,
    g: f64,
    flow_e: Option<Expr>,
    pressure: f64,
    eff: f64,
    uncovered: Option<Expr>,
    when: Option<Expr>,
    on: bool,
    flow: f64,
    s: Slots,
}

struct Valve {
    link: usize,
    ends: [usize; 2],
    r_open: f64,
    when: Option<Expr>,
    open_time: f64,
    stuck: Option<Expr>,
    check: bool,
    blocked: bool,
    position: f64,
    s: Slots,
}

struct Feed {
    line: usize,
    node: usize,
    demand: Expr,
    minimum: f64,
    suction: Option<Expr>,
    sucking: bool,
    g: f64,
    s: Slots,
}

pub struct Fuel {
    net: Network,
    ports: crate::ports::Ports,
    tanks: Vec<Tank>,
    lines: Vec<Line>,
    pumps: Vec<Pump>,
    valves: Vec<Valve>,
    feeds: Vec<Feed>,
    node_tank: HashMap<usize, usize>,
    node_line: HashMap<usize, usize>,
}

impl Fuel {
    pub fn new(c: &Checked) -> Option<Self> {
        let parts: Vec<(usize, &CheckedPart)> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "fuel").collect();
        if parts.is_empty() {
            return None;
        }
        let mut net = Network::new();
        let mut f = Fuel { net: Network::new(), ports: Default::default(), tanks: vec![], lines: vec![], pumps: vec![], valves: vec![], feeds: vec![], node_tank: HashMap::new(), node_line: HashMap::new() };
        let mut of: HashMap<usize, NodeOf> = HashMap::new();
        for &(i, p) in &parts {
            match p.kind {
                "tank" => {
                    let node = net.add_node(0.0);
                    f.node_tank.insert(node, f.tanks.len());
                    of.insert(i, NodeOf::Tank(f.tanks.len()));
                    let capacity = p.num("capacity").max(1e-6);
                    let cooling = p.num("cooling");
                    f.tanks.push(Tank {
                        node,
                        capacity,
                        quantity: p.num("quantity").min(capacity),
                        error: p.num("gauge_error"),
                        unusable: p.num("unusable").max(0.0),
                        overflow: None,
                        spilled: 0.0,
                        temp: p.num("temperature"),
                        skin: expr_of(p, "skin_temperature"),
                        heat: expr_of(p, "heat"),
                        cooling: if cooling > 0.0 { cooling } else { 0.1 * capacity },
                        freeze: p.num("freeze_point"),
                        density: p.num("density").max(1.0),
                        s: Slots::of(p, &["quantity", "indicated", "mass", "arm", "volume", "temperature", "low", "empty", "frozen", "spilled", "failed"]),
                    });
                }
                "line" => {
                    let node = net.add_node(0.0);
                    f.node_line.insert(node, f.lines.len());
                    of.insert(i, NodeOf::Line(f.lines.len()));
                    f.lines.push(Line { node, leak_when: expr_of(p, "leak_when"), leak_g: p.num("leak_rate") / 1e5, leaking: false, temp: T_REF, s: Slots::of(p, &["pressure", "temperature", "leak_flow", "failed"]) });
                }
                _ => {}
            }
        }
        for &(i, p) in parts.iter().filter(|(_, p)| p.kind == "tank") {
            if let (Some(NodeOf::Tank(t)), Some(NodeOf::Tank(to))) = (of.get(&i), p.part("overflow_to").and_then(|x| of.get(&x))) {
                if t != to {
                    f.tanks[*t].overflow = Some(*to);
                }
            }
        }
        let node = |f: &Fuel, i: Option<usize>| match i.and_then(|i| of.get(&i)) {
            Some(NodeOf::Tank(t)) => Some(f.tanks[*t].node),
            Some(NodeOf::Line(l)) => Some(f.lines[*l].node),
            None => None,
        };
        for &(_, p) in &parts {
            match p.kind {
                "pump" => {
                    let (Some(NodeOf::Tank(t)), Some(out)) = (p.part("tank").and_then(|i| of.get(&i)), node(&f, p.part("outlet"))) else { continue };
                    f.pumps.push(Pump {
                        tank: *t,
                        outlet: out,
                        g: 0.0,
                        flow_e: expr_of(p, "flow"),
                        pressure: p.num("pressure").max(1.0),
                        eff: p.num("efficiency").clamp(0.01, 1.0),
                        uncovered: expr_of(p, "inlet_uncovered_when"),
                        when: expr_of(p, "running_when"),
                        on: false,
                        flow: 0.0,
                        s: Slots::of(p, &["running", "flow", "power", "low_pressure", "failed"]),
                    });
                }
                "valve" => {
                    let e = p.parts("between");
                    let (Some(a), Some(b)) = (node(&f, e.first().copied()), node(&f, e.get(1).copied())) else { continue };
                    let r = 1e5 / p.num("flow").max(1e-12);
                    let link = net.add_link(a, b, r, 0.0);
                    f.valves.push(Valve {
                        link,
                        ends: [a, b],
                        r_open: r,
                        when: expr_of(p, "open_when"),
                        open_time: p.num("open_time").max(0.0),
                        stuck: expr_of(p, "stuck_when"),
                        check: p.flag("check"),
                        blocked: false,
                        position: 0.0,
                        s: Slots::of(p, &["open", "position", "flow", "failed"]),
                    });
                }
                "feed" => {
                    let Some(NodeOf::Line(l)) = p.part("line").and_then(|i| of.get(&i)) else { continue };
                    let Some(demand) = expr_of(p, "demand") else { continue };
                    f.feeds.push(Feed { line: *l, node: f.lines[*l].node, demand, minimum: p.num("minimum"), suction: expr_of(p, "suction_when"), sucking: false, g: 0.0, s: Slots::of(p, &["flow", "starved", "temperature", "failed"]) });
                }
                _ => {}
            }
        }
        f.ports = crate::ports::Ports::new(c, "fuel", &mut net, |i| match of.get(&i) {
            Some(NodeOf::Line(l)) => Some(f.lines[*l].node),
            _ => None,
        });
        f.net = net;
        Some(f)
    }

    fn node_temp(&self, node: usize) -> f64 {
        if let Some(&t) = self.node_tank.get(&node) {
            self.tanks[t].temp
        } else if let Some(&l) = self.node_line.get(&node) {
            self.lines[l].temp
        } else {
            T_REF
        }
    }
}

impl Domain for Fuel {
    fn name(&self) -> &'static str {
        "fuel"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        self.net.begin_step();
        for t in &self.tanks {
            let head = if t.quantity > t.unusable { HEAD * (0.2 + 0.8 * t.quantity / t.capacity) } else { 0.0 };
            self.net.stamp(t.node, easyacft_core::STIFF * 1e-9, easyacft_core::STIFF * 1e-9 * head);
        }
        for l in &mut self.lines {
            l.leaking = yes(&mut l.leak_when, slots, dt, false);
            if l.leaking {
                self.net.stamp(l.node, l.leak_g, 0.0);
            }
        }
        for v in &mut self.valves {
            let commanded = !v.s.on(slots, "failed") && yes(&mut v.when, slots, dt, true);
            if !yes(&mut v.stuck, slots, dt, false) {
                let target = if commanded { 1.0 } else { 0.0 };
                v.position = if v.open_time > 0.0 { v.position + (target - v.position).clamp(-dt / v.open_time, dt / v.open_time) } else { target };
            }
            if v.check {
                let forward = self.net.potential(v.ends[0]) - self.net.potential(v.ends[1]);
                v.blocked = if v.blocked { forward <= 0.0 } else { self.net.flow(v.link) < 0.0 };
            }
            let open = v.position > 1e-3 && !v.blocked;
            self.net.set_closed(v.link, open);
            self.net.set_resistance(v.link, v.r_open / v.position.max(1e-3));
            v.s.flag(slots, "open", open);
            v.s.put(slots, "position", v.position);
        }
        for p in &mut self.pumps {
            let t = &self.tanks[p.tank];
            let wax = ((t.temp - t.freeze) / WAX_BAND).clamp(0.0, 1.0);
            p.g = eval(&mut p.flow_e, slots, dt, 0.0).max(0.0) * wax / (DROOP * p.pressure);
            p.on = !p.s.on(slots, "failed") && t.quantity > t.unusable && !yes(&mut p.uncovered, slots, dt, false) && yes(&mut p.when, slots, dt, true);
            p.s.flag(slots, "running", p.on);
            if p.on {
                self.net.stamp(p.outlet, p.g, p.g * p.pressure);
            }
        }
        let mut want = vec![0.0; self.feeds.len()];
        for (k, fd) in self.feeds.iter_mut().enumerate() {
            want[k] = if fd.s.on(slots, "failed") { 0.0 } else { fd.demand.eval(slots, dt).max(0.0) };
            let p = self.lines[fd.line].s.get(slots, "pressure");
            fd.sucking = want[k] > 0.0 && p < fd.minimum && yes(&mut fd.suction, slots, dt, false);
            if fd.sucking {
                fd.g = want[k] / SUCTION;
                self.net.stamp(fd.node, fd.g, -fd.g * SUCTION);
            } else if want[k] > 0.0 {
                fd.g = want[k] / p.max(fd.minimum);
                self.net.stamp(fd.node, fd.g, 0.0);
            }
        }
        self.ports.stamp(&mut self.net, slots);
        self.net.step(dt);
        self.ports.read(&self.net, slots);
        let mut out = vec![0.0; self.tanks.len()];
        let mut inflow = vec![(0.0, 0.0); self.tanks.len()];
        let mut line_in = vec![(0.0, 0.0); self.lines.len()];
        for p in &mut self.pumps {
            let pout = self.net.potential(p.outlet);
            p.flow = if p.on { (p.g * (p.pressure - pout)).max(0.0) } else { 0.0 };
            out[p.tank] += p.flow;
            let t_src = self.tanks[p.tank].temp;
            if let Some(&t) = self.node_tank.get(&p.outlet) {
                out[t] -= p.flow;
                inflow[t].0 += p.flow;
                inflow[t].1 += p.flow * t_src;
            } else if let Some(&l) = self.node_line.get(&p.outlet) {
                line_in[l].0 += p.flow;
                line_in[l].1 += p.flow * t_src;
            }
            let density = self.tanks[p.tank].density;
            p.s.put(slots, "flow", p.flow);
            let rated = p.g * DROOP * p.pressure * p.pressure / density / p.eff;
            p.s.put(slots, "power", if p.on { 0.3 * rated + p.flow * pout.max(0.0) / density / p.eff } else { 0.0 });
            p.s.flag(slots, "low_pressure", p.on && pout < 0.5 * p.pressure);
        }
        for v in &self.valves {
            let q = if v.position > 1e-3 && !v.blocked { self.net.flow(v.link) } else { 0.0 };
            v.s.put(slots, "flow", q.abs());
            let (from, to) = if q >= 0.0 { (v.ends[0], v.ends[1]) } else { (v.ends[1], v.ends[0]) };
            let t_src = self.node_temp(from);
            if let Some(&t) = self.node_tank.get(&from) {
                out[t] += q.abs();
            }
            if let Some(&t) = self.node_tank.get(&to) {
                out[t] -= q.abs();
                inflow[t].0 += q.abs();
                inflow[t].1 += q.abs() * t_src;
            } else if let Some(&l) = self.node_line.get(&to) {
                line_in[l].0 += q.abs();
                line_in[l].1 += q.abs() * t_src;
            }
        }
        for (k, l) in self.lines.iter_mut().enumerate() {
            let p = self.net.potential(l.node).max(0.0);
            let (m, h) = line_in[k];
            if m > 1e-9 {
                l.temp = h / m;
            }
            l.s.put(slots, "pressure", p);
            l.s.put(slots, "temperature", l.temp);
            l.s.put(slots, "leak_flow", if l.leaking { l.leak_g * p } else { 0.0 });
        }
        for (k, fd) in self.feeds.iter_mut().enumerate() {
            let p = self.net.potential(fd.node);
            let drawn = if fd.sucking { (fd.g * (p + SUCTION)).clamp(0.0, want[k]) } else { want[k] };
            let starved = if fd.sucking { drawn < 0.8 * want[k] } else { p < fd.minimum };
            fd.s.flag(slots, "starved", starved);
            fd.s.put(slots, "flow", if starved { 0.0 } else { drawn });
            fd.s.put(slots, "temperature", self.lines[fd.line].temp);
        }
        let mut overflow = vec![0.0; self.tanks.len()];
        for k in 0..self.tanks.len() {
            let skin = eval(&mut self.tanks[k].skin, slots, dt, T_REF);
            let heat = eval(&mut self.tanks[k].heat, slots, dt, 0.0);
            let t = &mut self.tanks[k];
            let (m_in, h_in) = inflow[k];
            let mass = t.quantity + STRUCTURE_KG;
            t.temp = (mass * FUEL_CP * t.temp + dt * (FUEL_CP * h_in + heat + t.cooling * skin)) / (mass * FUEL_CP + dt * (FUEL_CP * m_in + t.cooling));
            let q = t.quantity - out[k] * dt;
            if q > t.capacity {
                overflow[k] = q - t.capacity;
            }
            t.quantity = q.clamp(0.0, t.capacity);
        }
        for k in 0..self.tanks.len() {
            let mut excess = overflow[k];
            let mut at = self.tanks[k].overflow;
            let mut hops = 0;
            while excess > 0.0 && hops < 4 {
                let Some(to) = at else { break };
                let room = self.tanks[to].capacity - self.tanks[to].quantity;
                let moved = excess.min(room);
                self.tanks[to].quantity += moved;
                excess -= moved;
                at = self.tanks[to].overflow;
                hops += 1;
            }
            self.tanks[k].spilled += excess;
        }
        for t in &mut self.tanks {
            let density = t.density * (1.0 - EXPANSION * (t.temp - T_REF));
            t.s.put(slots, "quantity", t.quantity);
            t.s.put(slots, "mass", t.quantity);
            t.s.put(slots, "volume", t.quantity / density);
            t.s.put(slots, "temperature", t.temp);
            t.s.put(slots, "indicated", if t.s.on(slots, "failed") { 0.0 } else { t.quantity * (1.0 + t.error) });
            t.s.flag(slots, "low", t.quantity < 0.1 * t.capacity);
            t.s.flag(slots, "empty", t.quantity <= t.unusable);
            t.s.flag(slots, "frozen", t.temp < t.freeze + WAX_BAND);
            t.s.put(slots, "spilled", t.spilled);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut p = Packer(self.net.state(), 0);
        for t in &self.tanks {
            [t.quantity, t.temp, t.spilled].into_iter().for_each(|v| p.put(v));
        }
        self.lines.iter().for_each(|l| p.put(l.temp));
        for v in &self.valves {
            p.put(v.position);
            p.put(v.blocked as u8 as f64);
        }
        p.0
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
            t.temp = p.take();
            t.spilled = p.take();
        }
        self.lines.iter_mut().for_each(|l| l.temp = p.take());
        for v in &mut self.valves {
            v.position = p.take();
            v.blocked = p.take() != 0.0;
        }
    }
}

pub fn init(c: &Checked, slots: &mut [f64]) {
    for p in c.parts.iter().filter(|p| p.domain == "fuel" && p.kind == "tank") {
        let q = p.num("quantity").min(p.num("capacity"));
        slots[crate::catalog::out(p, "quantity")] = q;
        slots[crate::catalog::out(p, "mass")] = q;
        slots[crate::catalog::out(p, "arm")] = p.num("arm");
        slots[crate::catalog::out(p, "temperature")] = p.num("temperature");
    }
}
