//! Hydraulic domain: each circuit is a small liquid network stepped in millisecond substeps, so pressure rises,
//! sags and rings the way a real system does. Circuits carry their reservoir, fluid temperature, viscosity,
//! entrained air and contamination; pumps have compensators, case drains and suction; accumulators have gas
//! that heats and cools; consumers, actuators and brakes take flow; PTUs move power between circuits. A substep
//! whose answer is not finite is thrown away and the domain rolls back.

mod accumulator;
mod circuit;
pub mod net;
mod ptu;
mod pump;
mod users;

use crate::{Domain, Expr};
use accumulator::Accumulator;
use circuit::Circuit;
use easyacft_core::Dim;
use easyacft_lang::schema::{Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};
use net::Net;
use ptu::Ptu;
use pump::Pump;
use std::collections::HashMap;
use users::{Consumer, Draw, Port};

pub(crate) const P: Type = Type::Num(Dim::PRESSURE);
pub(crate) const Q: Type = Type::Num(Dim::VOLUME_FLOW);
pub(crate) const VOL: Type = Type::Num(Dim::VOLUME);
pub(crate) const T: Type = Type::Num(Dim::TEMPERATURE);
pub(crate) const W: Type = Type::Num(Dim::POWER);
pub(crate) const N: Type = Type::Num(Dim::NONE);
pub(crate) const B: Type = Type::Bool;
pub(crate) const TQ: Type = Type::Num(Dim::TORQUE);
pub(crate) const WHEN: Kind = Kind::Expr(Type::Bool);

pub static SPECS: &[PartSpec] = &[circuit::SPEC, pump::SPEC, accumulator::SPEC, users::SPEC, ptu::SPEC];

/// Longest substep: a millisecond resolves the pump compensators and the pressure waves in the lines.
const H_MAX: f64 = 1e-3;
const CHECKPOINT_EVERY: u32 = 50;

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

/// A custom part's link between two circuits: a conductance set each step.
struct CustomLink {
    link: usize,
    g: usize,
    q: usize,
}

pub struct Hydraulic {
    net: Net,
    circuits: Vec<Circuit>,
    pumps: Vec<Pump>,
    accumulators: Vec<Accumulator>,
    consumers: Vec<Consumer>,
    ptus: Vec<Ptu>,
    draws: Vec<Draw>,
    ports: Vec<Port>,
    links: Vec<CustomLink>,
    checkpoint: Vec<f64>,
    since: u32,
    pub rollbacks: u64,
}

impl Hydraulic {
    pub fn new(c: &Checked) -> Option<Self> {
        let parts: Vec<(usize, &CheckedPart)> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "hydraulic").collect();
        if parts.is_empty() {
            return None;
        }
        let mut net = Net::default();
        let mut of: HashMap<usize, usize> = HashMap::new();
        let mut h = Hydraulic { net: Net::default(), circuits: vec![], pumps: vec![], accumulators: vec![], consumers: vec![], ptus: vec![], draws: vec![], ports: vec![], links: vec![], checkpoint: vec![], since: 0, rollbacks: 0 };
        for &(i, p) in parts.iter().filter(|(_, p)| p.kind == "circuit") {
            of.insert(i, h.circuits.len());
            h.circuits.push(Circuit::new(p, &mut net));
        }
        let circ = |p: &CheckedPart| p.part("circuit").and_then(|x| of.get(&x).copied());
        for &(_, p) in &parts {
            match p.kind {
                "pump" => {
                    let Some(ci) = circ(p) else { continue };
                    let pm = Pump::new(p, ci, &h.circuits[ci], &mut net);
                    h.pumps.push(pm);
                }
                "accumulator" => {
                    let Some(ci) = circ(p) else { continue };
                    let a = Accumulator::new(p, ci, &h.circuits[ci], &mut net);
                    h.accumulators.push(a);
                }
                "consumer" => {
                    let Some(ci) = circ(p) else { continue };
                    let u = Consumer::new(p, ci, &h.circuits[ci]);
                    h.consumers.push(u);
                }
                "ptu" => {
                    let ends = p.parts("between");
                    let (Some(&a), Some(&b)) = (ends.first().and_then(|x| of.get(x)), ends.get(1).and_then(|x| of.get(x))) else { continue };
                    h.ptus.push(Ptu::new(p, a, b));
                }
                _ => {}
            }
        }
        for p in c.parts.iter().filter(|p| p.domain != "hydraulic") {
            let Some(spec) = crate::catalog::find(p.domain, p.kind) else { continue };
            for f in spec.fields {
                let Kind::Ref(kinds) = f.kind else { continue };
                if !kinds.contains(&"hydraulic.circuit") {
                    continue;
                }
                let Some(ci) = p.part(f.key).and_then(|x| of.get(&x).copied()) else { continue };
                let out = |o: String| spec.outputs.iter().position(|(n, _)| *n == o).map(|k| p.slot + k);
                if let Some(flow) = out(format!("{}_flow", f.key)) {
                    h.draws.push(Draw::new(ci, flow, out(format!("{}_held", f.key))));
                }
            }
        }
        let slot = |id: &str, n: &str, k: &str| c.slots.index.get(&format!("{id}.{n}_{k}")).copied();
        for p in c.parts.iter().filter(|p| p.domain == "custom") {
            for port in crate::custom::ports_of(p.kind).into_iter().filter(|x| x.domain == "hydraulic") {
                let n = port.name;
                if port.link {
                    let ends = p.parts(n);
                    let (Some(&a), Some(&b)) = (ends.first().and_then(|x| of.get(x)), ends.get(1).and_then(|x| of.get(x))) else { continue };
                    let (Some(g), Some(q)) = (slot(&p.id, n, "g"), slot(&p.id, n, "q")) else { continue };
                    let link = net.link(h.circuits[a].node, h.circuits[b].node, 1e16, 0.0, false);
                    h.links.push(CustomLink { link, g, q });
                } else {
                    let Some(ci) = p.part(n).and_then(|x| of.get(&x).copied()) else { continue };
                    let (Some(se), Some(sg), Some(sv), Some(sq)) = (slot(&p.id, n, "e"), slot(&p.id, n, "g"), slot(&p.id, n, "v"), slot(&p.id, n, "q")) else { continue };
                    h.ports.push(Port { circuit: ci, slot: [se, sg, sv, sq] });
                }
            }
        }
        for (k, ci) in h.circuits.iter_mut().enumerate() {
            let total: f64 = h.pumps.iter().filter(|p| p.circuit == k).map(|p| p.full_flow).sum();
            ci.set_rated_flow(total);
        }
        h.net = net;
        Some(h)
    }

    /// Pumps size the filters from the flow they first deliver, as their flow is an expression.
    fn size_filters(&mut self) {
        for (k, c) in self.circuits.iter_mut().enumerate() {
            let total: f64 = self.pumps.iter().filter(|p| p.circuit == k).map(|p| p.full_flow).sum();
            if total > 0.0 {
                c.set_rated_flow(total);
            }
        }
    }

    fn substep(&mut self, slots: &[f64], h: f64) -> bool {
        self.net.begin();
        for c in &mut self.circuits {
            c.stamp(&mut self.net);
        }
        for pm in &mut self.pumps {
            pm.stamp(&mut self.net, &self.circuits[pm.circuit], h);
        }
        for a in &mut self.accumulators {
            a.stamp(&mut self.net);
        }
        for u in &mut self.consumers {
            u.stamp(&mut self.net, &self.circuits[u.circuit]);
        }
        for d in &self.draws {
            d.stamp(&mut self.net, &self.circuits[d.circuit]);
        }
        for t in &mut self.ptus {
            t.stamp(&mut self.net, &self.circuits[t.a], &self.circuits[t.b], h);
        }
        for port in &self.ports {
            port.stamp(&mut self.net, &self.circuits[port.circuit], slots);
        }
        for l in &self.links {
            let g = slots[l.g];
            let link = &mut self.net.links[l.link];
            link.open = g > 0.0 && g.is_finite();
            link.r = if link.open { 1.0 / g } else { 1e16 };
        }
        if !self.net.step(h) {
            return false;
        }
        for a in &mut self.accumulators {
            a.after(&self.net, h);
        }
        for c in &mut self.circuits {
            c.after(&self.net, h);
        }
        true
    }
}

impl Domain for Hydraulic {
    fn name(&self) -> &'static str {
        "hydraulic"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        if self.since == 0 || self.checkpoint.is_empty() {
            self.checkpoint = self.state();
        }
        self.since = (self.since + 1) % CHECKPOINT_EVERY;
        for c in &mut self.circuits {
            c.begin(slots, dt, &self.net);
        }
        for pm in &mut self.pumps {
            pm.begin(slots, dt, &self.circuits[pm.circuit]);
        }
        self.size_filters();
        for a in &mut self.accumulators {
            a.begin(slots, dt);
        }
        for u in &mut self.consumers {
            u.begin(slots, dt);
        }
        for d in &mut self.draws {
            d.begin(slots);
        }
        for t in &mut self.ptus {
            t.begin(slots, dt);
        }
        let n = (dt / H_MAX).ceil().max(1.0);
        let h = dt / n;
        for _ in 0..n as usize {
            if !self.substep(slots, h) {
                let cp = std::mem::take(&mut self.checkpoint);
                self.restore(&cp);
                self.checkpoint = cp;
                self.rollbacks += 1;
                return;
            }
        }
        let nc = self.circuits.len();
        let mut held = vec![0.0; nc];
        let mut cav = vec![0.0; nc];
        let mut wear = vec![0.0; nc];
        for a in &self.accumulators {
            held[a.circuit] += a.fluid;
            a.finish(&self.net, slots);
        }
        for d in &self.draws {
            held[d.circuit] += d.held(slots);
            let c = &mut self.circuits[d.circuit];
            let p = self.net.p[c.node].max(0.0);
            c.heat += d.flow() * p * n;
            c.add_return(d.flow() * n);
        }
        for pm in &mut self.pumps {
            let (drain, loss) = pm.finish(&self.net, slots, dt, &self.circuits[pm.circuit]);
            self.circuits[pm.circuit].heat += loss * n;
            self.circuits[pm.circuit].add_return(drain * n);
            cav[pm.circuit] += pm.cavitation();
            wear[pm.circuit] += pm.wear;
        }
        for u in &self.consumers {
            let c = &self.circuits[u.circuit];
            let p = self.net.p[c.node];
            let (q, heat) = u.returned(p);
            u.finish(slots, c, &self.net);
            self.circuits[u.circuit].heat += heat * n;
            self.circuits[u.circuit].add_return(q * n);
        }
        for t in &mut self.ptus {
            t.finish(slots);
            self.circuits[t.a].heat += t.heat_a;
            self.circuits[t.b].heat += t.heat_b;
        }
        for port in &self.ports {
            port.read(&self.net, &self.circuits[port.circuit], slots);
        }
        for l in &self.links {
            let link = &self.net.links[l.link];
            slots[l.q] = if link.open { link.q } else { 0.0 };
        }
        for (k, c) in self.circuits.iter_mut().enumerate() {
            c.finish(&self.net, slots, dt, n, held[k], cav[k], wear[k]);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut v = Vec::new();
        self.net.state(&mut v);
        for x in &self.circuits {
            x.state(&mut v);
        }
        for x in &self.pumps {
            x.state(&mut v);
        }
        for x in &self.accumulators {
            x.state(&mut v);
        }
        for x in &self.consumers {
            x.state(&mut v);
        }
        for x in &self.ptus {
            x.state(&mut v);
        }
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let mut it = s.iter().copied();
        self.net.restore(&mut it);
        for x in &mut self.circuits {
            x.restore(&mut it);
        }
        for x in &mut self.pumps {
            x.restore(&mut it);
        }
        for x in &mut self.accumulators {
            x.restore(&mut it);
        }
        for x in &mut self.consumers {
            x.restore(&mut it);
        }
        for x in &mut self.ptus {
            x.restore(&mut it);
        }
    }
}
