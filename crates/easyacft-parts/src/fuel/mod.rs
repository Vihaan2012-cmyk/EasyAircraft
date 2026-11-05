//! Fuel domain: tanks, lines, pumps, valves and engine feeds as a liquid network stepped in short substeps, so
//! boost pressure builds and falls with flow and a closing valve is felt down the line. Tanks carry their fuel's
//! grade, temperature, attitude, ullage and water; pumps follow head curves and cavitate; valves travel and
//! leak; feeds suck, heat and ice. Fuel is never created or lost except overboard. A substep whose answer is not
//! finite is thrown away and the domain rolls back.

mod feed;
pub mod props;
mod pump;
mod tank;
mod valve;

use crate::hydraulic::net::Net;
use crate::{Domain, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};
use feed::Feed;
use pump::Pump;
use std::collections::HashMap;
use tank::Tank;
use valve::{Line, Valve};

pub(crate) const M: Type = Type::Num(Dim::MASS);
pub(crate) const MF: Type = Type::Num(Dim::MASS_FLOW);
pub(crate) const P: Type = Type::Num(Dim::PRESSURE);
pub(crate) const L: Type = Type::Num(Dim::LENGTH);
pub(crate) const T: Type = Type::Num(Dim::TEMPERATURE);
pub(crate) const W: Type = Type::Num(Dim::POWER);
pub(crate) const VOL: Type = Type::Num(Dim::VOLUME);
pub(crate) const N: Type = Type::Num(Dim::NONE);
pub(crate) const B: Type = Type::Bool;
pub(crate) const WHEN: Kind = Kind::Expr(Type::Bool);

pub static SPECS: &[PartSpec] = &[tank::SPEC, valve::LINE, pump::SPEC, valve::VALVE, feed::SPEC];

/// Longest substep.
const H_MAX: f64 = 0.005;
/// Conductance holding a tank's node at its head: far stiffer than any valve or pump into it.
const G_TANK: f64 = 1e-3;
const CHECKPOINT_EVERY: u32 = 50;
const T_REF: f64 = 288.15;

pub(crate) fn eval(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: f64) -> f64 {
    e.as_mut().map_or(or, |x| x.eval(slots, dt))
}

pub(crate) fn yes(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: bool) -> bool {
    e.as_mut().map_or(or, |x| x.yes(slots, dt))
}

#[inline]
pub(crate) fn put(slots: &mut [f64], at: usize, v: f64) {
    slots[at] = if v.is_finite() { v } else { 0.0 };
}

#[inline]
pub(crate) fn flag(slots: &mut [f64], at: usize, v: bool) {
    slots[at] = v as u8 as f64;
}

#[derive(Clone, Copy)]
enum NodeOf {
    Tank(usize),
    Line(usize),
}

/// A custom part's port on a fuel line: a Norton source, flow = g (e - p).
struct Port {
    line: usize,
    slot: [usize; 4],
}

/// What moved into a tank or line this step: mass, enthalpy as mass times temperature, water mass, and
/// density weighted by mass.
#[derive(Clone, Copy, Default)]
struct Inflow {
    m: f64,
    mt: f64,
    water: f64,
    rho_m: f64,
}

impl Inflow {
    fn add(&mut self, m: f64, t: f64, water: f64, rho: f64) {
        self.m += m;
        self.mt += m * t;
        self.water += m * water;
        self.rho_m += m * rho;
    }
}

pub struct Fuel {
    net: Net,
    tanks: Vec<Tank>,
    lines: Vec<Line>,
    line_rho: Vec<f64>,
    pumps: Vec<Pump>,
    valves: Vec<Valve>,
    feeds: Vec<Feed>,
    ports: Vec<Port>,
    node_of: HashMap<usize, NodeOf>,
    checkpoint: Vec<f64>,
    since: u32,
    pub rollbacks: u64,
}

impl Fuel {
    pub fn new(c: &Checked) -> Option<Self> {
        let parts: Vec<(usize, &CheckedPart)> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "fuel").collect();
        if parts.is_empty() {
            return None;
        }
        let mut net = Net::default();
        let mut f = Fuel { net: Net::default(), tanks: vec![], lines: vec![], line_rho: vec![], pumps: vec![], valves: vec![], feeds: vec![], ports: vec![], node_of: HashMap::new(), checkpoint: vec![], since: 0, rollbacks: 0 };
        let mut of: HashMap<usize, NodeOf> = HashMap::new();
        for &(i, p) in &parts {
            match p.kind {
                "tank" => {
                    let node = net.node(1e-12);
                    f.node_of.insert(node, NodeOf::Tank(f.tanks.len()));
                    of.insert(i, NodeOf::Tank(f.tanks.len()));
                    f.tanks.push(Tank::new(p, node));
                }
                "line" => {
                    let rho = 800.0;
                    let l = Line::new(p, &mut net, rho);
                    f.node_of.insert(l.node, NodeOf::Line(f.lines.len()));
                    of.insert(i, NodeOf::Line(f.lines.len()));
                    f.lines.push(l);
                    f.line_rho.push(rho);
                }
                _ => {}
            }
        }
        for &(i, p) in parts.iter().filter(|(_, p)| p.kind == "tank") {
            if let (Some(NodeOf::Tank(t)), Some(NodeOf::Tank(to))) = (of.get(&i).copied(), p.part("overflow_to").and_then(|x| of.get(&x)).copied()) {
                if t != to {
                    f.tanks[t].overflow = Some(to);
                }
            }
        }
        let node = |f: &Fuel, i: Option<usize>| match i.and_then(|i| of.get(&i)) {
            Some(NodeOf::Tank(t)) => Some(f.tanks[*t].node),
            Some(NodeOf::Line(l)) => Some(f.lines[*l].node),
            None => None,
        };
        let rho0 = f.tanks.first().map_or(800.0, |t| t.fuel.rho15);
        for &(_, p) in &parts {
            match p.kind {
                "pump" => {
                    let (Some(NodeOf::Tank(t)), Some(out)) = (p.part("tank").and_then(|i| of.get(&i)).copied(), node(&f, p.part("outlet"))) else { continue };
                    f.pumps.push(Pump::new(p, t, out));
                }
                "valve" => {
                    let e = p.parts("between");
                    let (Some(a), Some(b)) = (node(&f, e.first().copied()), node(&f, e.get(1).copied())) else { continue };
                    f.valves.push(Valve::new(p, &mut net, a, b, rho0));
                }
                "feed" => {
                    let Some(NodeOf::Line(l)) = p.part("line").and_then(|i| of.get(&i)).copied() else { continue };
                    let Some(demand) = crate::expr_of(p, "demand") else { continue };
                    f.feeds.push(Feed::new(p, l, f.lines[l].node, demand));
                }
                _ => {}
            }
        }
        let slot = |id: &str, n: &str, k: &str| c.slots.index.get(&format!("{id}.{n}_{k}")).copied();
        for p in c.parts.iter().filter(|p| p.domain == "custom") {
            for port in crate::custom::ports_of(p.kind).into_iter().filter(|x| x.domain == "fuel" && !x.link) {
                let n = port.name;
                let Some(NodeOf::Line(l)) = p.part(n).and_then(|x| of.get(&x)).copied() else { continue };
                let (Some(se), Some(sg), Some(sv), Some(sq)) = (slot(&p.id, n, "e"), slot(&p.id, n, "g"), slot(&p.id, n, "v"), slot(&p.id, n, "q")) else { continue };
                f.ports.push(Port { line: l, slot: [se, sg, sv, sq] });
            }
        }
        f.net = net;
        Some(f)
    }

    fn node_props(&self, node: usize) -> (f64, f64, f64) {
        match self.node_of.get(&node) {
            Some(NodeOf::Tank(t)) => {
                let t = &self.tanks[*t];
                (t.temp, t.water + t.water_carry(), t.density())
            }
            Some(NodeOf::Line(l)) => (self.lines[*l].temp, self.lines[*l].water, self.line_rho[*l]),
            None => (T_REF, 0.0, 800.0),
        }
    }

    fn substep(&mut self, slots: &[f64], h: f64, tank_in: &mut [Inflow], tank_out: &mut [f64], line_in: &mut [Inflow]) -> bool {
        self.net.begin();
        for t in &self.tanks {
            self.net.stamp(t.node, G_TANK, G_TANK * t.head);
        }
        for l in &self.lines {
            l.stamp(&mut self.net);
        }
        for v in &self.valves {
            let dry = v.ends.map(|n| matches!(self.node_of.get(&n), Some(NodeOf::Tank(t)) if self.tanks[*t].quantity <= self.tanks[*t].unusable));
            v.stamp(&mut self.net, dry);
        }
        for p in &mut self.pumps {
            p.stamp(&mut self.net, &self.tanks[p.tank]);
        }
        for fd in &mut self.feeds {
            fd.stamp(&mut self.net);
        }
        for port in &self.ports {
            let g = slots[port.slot[1]].max(0.0);
            if g > 0.0 && g.is_finite() {
                self.net.stamp(self.lines[port.line].node, g, g * slots[port.slot[0]]);
            }
        }
        if !self.net.step(h) {
            return false;
        }
        for k in 0..self.pumps.len() {
            let ti = self.pumps[k].tank;
            let (temp, water, rho) = self.node_props(self.tanks[ti].node);
            self.pumps[k].after(&self.net, rho);
            let m = self.pumps[k].q * rho * h;
            tank_out[ti] += m;
            let carry = self.tanks[ti].water_carry();
            if carry > 0.0 {
                self.tanks[ti].free_water = (self.tanks[ti].free_water - carry * m).max(0.0);
            }
            match self.node_of.get(&self.pumps[k].outlet) {
                Some(NodeOf::Tank(t)) => tank_in[*t].add(m, temp, water, rho),
                Some(NodeOf::Line(l)) => line_in[*l].add(m, temp, water, rho),
                None => {}
            }
        }
        for k in 0..self.valves.len() {
            self.valves[k].after(&self.net);
            let q = self.net.links[self.valves[k].link].q;
            if q == 0.0 {
                continue;
            }
            let [a, b] = self.valves[k].ends;
            let (from, to) = if q > 0.0 { (a, b) } else { (b, a) };
            let (temp, water, rho) = self.node_props(from);
            let m = q.abs() * rho * h;
            if let Some(NodeOf::Tank(t)) = self.node_of.get(&from) {
                tank_out[*t] += m;
            }
            match self.node_of.get(&to) {
                Some(NodeOf::Tank(t)) => tank_in[*t].add(m, temp, water, rho),
                Some(NodeOf::Line(l)) => line_in[*l].add(m, temp, water, rho),
                None => {}
            }
        }
        for l in &mut self.lines {
            l.after(&self.net);
        }
        for fd in &mut self.feeds {
            fd.after(&self.net);
        }
        true
    }
}

impl Domain for Fuel {
    fn name(&self) -> &'static str {
        "fuel"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        if self.since == 0 || self.checkpoint.is_empty() {
            self.checkpoint = self.state();
        }
        self.since = (self.since + 1) % CHECKPOINT_EVERY;
        for t in &mut self.tanks {
            t.begin(slots, dt);
        }
        for l in &mut self.lines {
            l.begin(slots, dt);
        }
        for p in &mut self.pumps {
            p.begin(slots, dt, &self.tanks[p.tank]);
        }
        for v in &mut self.valves {
            let rho = self.tanks.first().map_or(800.0, |t| t.density());
            v.begin(slots, dt, rho);
        }
        let vapour = self.tanks.iter().map(|t| t.fuel.vapour(t.temp)).fold(0.0, f64::max);
        for fd in &mut self.feeds {
            // A line an engine has just sucked dry sits at zero, not below it.
            self.net.p[fd.node] = self.net.p[fd.node].max(-30_000.0);
            let p_line = self.net.p[fd.node];
            let rho = self.line_rho[fd.line];
            fd.begin(slots, dt, p_line, vapour, rho);
        }
        let nt = self.tanks.len();
        let nl = self.lines.len();
        let mut tank_in = vec![Inflow::default(); nt];
        let mut tank_out = vec![0.0; nt];
        let mut line_in = vec![Inflow::default(); nl];
        let n = (dt / H_MAX).ceil().max(1.0);
        let h = dt / n;
        for _ in 0..n as usize {
            if !self.substep(slots, h, &mut tank_in, &mut tank_out, &mut line_in) {
                let cp = std::mem::take(&mut self.checkpoint);
                self.restore(&cp);
                self.checkpoint = cp;
                self.rollbacks += 1;
                return;
            }
        }
        for (k, l) in self.lines.iter_mut().enumerate() {
            let i = line_in[k];
            if i.m > 1e-9 {
                l.temp = i.mt / i.m;
                l.water = i.water / i.m;
                self.line_rho[k] = i.rho_m / i.m;
            }
            let amb = 101_325.0;
            l.finish(&self.net, slots, self.line_rho[k], vapour, amb);
        }
        for fd in &mut self.feeds {
            let l = &self.lines[fd.line];
            let cp = self.tanks.first().map_or(2000.0, |t| t.fuel.cp);
            fd.finish(&self.net, slots, dt, self.line_rho[fd.line], l.temp, l.water, cp);
        }
        for p in &self.pumps {
            p.finish(slots, &self.tanks[p.tank]);
        }
        for v in &self.valves {
            v.finish(slots);
        }
        for port in &self.ports {
            let p = self.net.p[self.lines[port.line].node];
            slots[port.slot[2]] = p;
            slots[port.slot[3]] = slots[port.slot[1]].max(0.0) * (slots[port.slot[0]] - p);
        }
        let mut overflow = vec![0.0; nt];
        for k in 0..nt {
            let i = tank_in[k];
            let net_out = (tank_out[k] - i.m) / dt;
            let water_out = tank_out[k] / dt * self.tanks[k].water;
            overflow[k] = self.tanks[k].finish(slots, dt, net_out, i.m / dt, i.mt / dt, i.water / dt, water_out);
        }
        for k in 0..nt {
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
    }

    fn state(&self) -> Vec<f64> {
        let mut v = Vec::new();
        self.net.state(&mut v);
        for x in &self.tanks {
            x.state(&mut v);
        }
        for x in &self.lines {
            x.state(&mut v);
        }
        v.extend_from_slice(&self.line_rho);
        for x in &self.pumps {
            x.state(&mut v);
        }
        for x in &self.valves {
            x.state(&mut v);
        }
        for x in &self.feeds {
            x.state(&mut v);
        }
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let mut it = s.iter().copied();
        self.net.restore(&mut it);
        for x in &mut self.tanks {
            x.restore(&mut it);
        }
        for x in &mut self.lines {
            x.restore(&mut it);
        }
        for r in &mut self.line_rho {
            *r = it.next().unwrap_or(800.0);
        }
        for x in &mut self.pumps {
            x.restore(&mut it);
        }
        for x in &mut self.valves {
            x.restore(&mut it);
        }
        for x in &mut self.feeds {
            x.restore(&mut it);
        }
    }
}

pub fn init(c: &Checked, slots: &mut [f64]) {
    for p in c.parts.iter().filter(|p| p.domain == "fuel" && p.kind == "tank") {
        let q = p.num("quantity").min(p.num("capacity"));
        slots[crate::catalog::out(p, "quantity")] = q;
        slots[crate::catalog::out(p, "mass")] = q;
        slots[crate::catalog::out(p, "indicated")] = q;
        slots[crate::catalog::out(p, "arm")] = p.num("arm");
        slots[crate::catalog::out(p, "temperature")] = p.num("temperature");
    }
}
