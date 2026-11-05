//! Pneumatic domain: bleed ducts as volumes of compressible air with their mass and energy, joined by valves
//! whose flow chokes, fed by sources behind check valves and drawn by packs, anti-ice and starters. Stepped in
//! short substeps on the same implicit network as the liquids, each orifice linearised at the pressures across
//! it, so a manifold fills, regulates and empties the way a real one does. A substep whose answer is not finite
//! is thrown away and the domain rolls back.

mod duct;
mod ends;
mod valve;

use crate::hydraulic::net::Net;
use crate::{Domain, Expr};
use duct::Duct;
use easyacft_core::Dim;
use easyacft_lang::schema::{Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};
use ends::{Consumer, Source};
use std::collections::HashMap;
use valve::Valve;

pub(crate) const P: Type = Type::Num(Dim::PRESSURE);
pub(crate) const T: Type = Type::Num(Dim::TEMPERATURE);
pub(crate) const MF: Type = Type::Num(Dim::MASS_FLOW);
pub(crate) const W: Type = Type::Num(Dim::POWER);
pub(crate) const N: Type = Type::Num(Dim::NONE);
pub(crate) const B: Type = Type::Bool;
pub(crate) const WHEN: Kind = Kind::Expr(Type::Bool);

pub static SPECS: &[PartSpec] = &[duct::SPEC, ends::SOURCE, valve::SPEC, ends::CONSUMER];

const H_MAX: f64 = 0.002;
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

/// A custom part's port on a duct: a Norton source of mass flow, q = g (e - p).
struct Port {
    duct: usize,
    slot: [usize; 4],
}

pub struct Pneumatic {
    net: Net,
    ducts: Vec<Duct>,
    sources: Vec<Source>,
    valves: Vec<Valve>,
    consumers: Vec<Consumer>,
    ports: Vec<Port>,
    checkpoint: Vec<f64>,
    since: u32,
    started: bool,
    pub rollbacks: u64,
}

impl Pneumatic {
    pub fn new(c: &Checked) -> Option<Self> {
        let parts: Vec<(usize, &CheckedPart)> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "pneumatic").collect();
        if parts.is_empty() {
            return None;
        }
        let mut net = Net::default();
        let mut of: HashMap<usize, usize> = HashMap::new();
        let mut n = Pneumatic { net: Net::default(), ducts: vec![], sources: vec![], valves: vec![], consumers: vec![], ports: vec![], checkpoint: vec![], since: 0, started: false, rollbacks: 0 };
        for &(i, p) in parts.iter().filter(|(_, p)| p.kind == "duct") {
            of.insert(i, n.ducts.len());
            let node = net.node(1e-6);
            n.ducts.push(Duct::new(p, node));
        }
        let duct = |p: &CheckedPart| p.part("duct").and_then(|x| of.get(&x).copied());
        for &(_, p) in &parts {
            match p.kind {
                "source" => {
                    let Some(d) = duct(p) else { continue };
                    n.sources.push(Source::new(p, d));
                }
                "valve" => {
                    let e = p.parts("between");
                    let (Some(&a), Some(&b)) = (e.first().and_then(|x| of.get(x)), e.get(1).and_then(|x| of.get(x))) else { continue };
                    let (na, nb) = (n.ducts[a].node, n.ducts[b].node);
                    n.valves.push(Valve::new(p, &mut net, a, b, na, nb));
                }
                "consumer" => {
                    let Some(d) = duct(p) else { continue };
                    n.consumers.push(Consumer::new(p, d));
                }
                _ => {}
            }
        }
        let slot = |id: &str, nm: &str, k: &str| c.slots.index.get(&format!("{id}.{nm}_{k}")).copied();
        for p in c.parts.iter().filter(|p| p.domain == "custom") {
            for port in crate::custom::ports_of(p.kind).into_iter().filter(|x| x.domain == "pneumatic" && !x.link) {
                let nm = port.name;
                let Some(d) = p.part(nm).and_then(|x| of.get(&x).copied()) else { continue };
                let (Some(se), Some(sg), Some(sv), Some(sq)) = (slot(&p.id, nm, "e"), slot(&p.id, nm, "g"), slot(&p.id, nm, "v"), slot(&p.id, nm, "q")) else { continue };
                n.ports.push(Port { duct: d, slot: [se, sg, sv, sq] });
            }
        }
        n.net = net;
        Some(n)
    }

    fn substep(&mut self, slots: &[f64], h: f64, m_in: &mut [f64], h_in: &mut [f64]) -> bool {
        self.net.begin();
        for d in &mut self.ducts {
            d.stamp(&mut self.net);
        }
        for s in &mut self.sources {
            s.stamp(&mut self.net, &self.ducts[s.duct]);
        }
        for c in &mut self.consumers {
            c.stamp(&mut self.net, &self.ducts[c.duct]);
        }
        for v in &mut self.valves {
            let [a, b] = v.ends;
            let (na, nb) = (self.ducts[a].node, self.ducts[b].node);
            v.stamp(&mut self.net, &self.ducts[a], &self.ducts[b], na, nb);
        }
        for port in &self.ports {
            let g = slots[port.slot[1]].max(0.0);
            if g > 0.0 && g.is_finite() {
                self.net.stamp(self.ducts[port.duct].node, g, g * slots[port.slot[0]]);
            }
        }
        if !self.net.step(h) {
            return false;
        }
        for s in &mut self.sources {
            let q = s.after(&self.net, &self.ducts[s.duct]);
            m_in[s.duct] += q * h;
            h_in[s.duct] += q * h * s.temp;
        }
        for v in &mut self.valves {
            let q = v.after(&self.net);
            let [a, b] = v.ends;
            let (from, to) = if q >= 0.0 { (a, b) } else { (b, a) };
            let t_out = {
                let t_in = self.ducts[from].temp;
                v.cool(t_in, q);
                v.t_out
            };
            m_in[to] += q.abs() * h;
            h_in[to] += q.abs() * h * t_out;
        }
        for c in &mut self.consumers {
            c.after(&self.net, &self.ducts[c.duct]);
        }
        for d in &mut self.ducts {
            d.after(&self.net);
        }
        for port in &self.ports {
            let g = slots[port.slot[1]].max(0.0);
            let q = g * (slots[port.slot[0]] - self.net.p[self.ducts[port.duct].node]);
            if q > 0.0 {
                m_in[port.duct] += q * h;
                h_in[port.duct] += q * h * self.ducts[port.duct].temp;
            }
        }
        true
    }
}

impl Domain for Pneumatic {
    fn name(&self) -> &'static str {
        "pneumatic"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        if self.since == 0 || self.checkpoint.is_empty() {
            self.checkpoint = self.state();
        }
        self.since = (self.since + 1) % CHECKPOINT_EVERY;
        for d in &mut self.ducts {
            let p = self.net.p[d.node];
            d.begin(slots, dt, p);
        }
        if !self.started {
            // Ducts start at the pressure around them.
            for d in &self.ducts {
                self.net.p[d.node] = d.p_amb;
            }
            self.started = true;
        }
        for s in &mut self.sources {
            s.begin(slots, dt);
        }
        for c in &mut self.consumers {
            c.begin(slots, dt);
        }
        for v in &mut self.valves {
            let [a, b] = v.ends;
            let (pa, pb) = (self.net.p[self.ducts[a].node], self.net.p[self.ducts[b].node]);
            v.begin(slots, dt, pa, pb, self.ducts[a].p_amb);
        }
        let nd = self.ducts.len();
        let mut m_in = vec![0.0; nd];
        let mut h_in = vec![0.0; nd];
        let n = (dt / H_MAX).ceil().max(1.0);
        let h = dt / n;
        for _ in 0..n as usize {
            if !self.substep(slots, h, &mut m_in, &mut h_in) {
                let cp = std::mem::take(&mut self.checkpoint);
                self.restore(&cp);
                self.checkpoint = cp;
                self.rollbacks += 1;
                return;
            }
        }
        for (k, d) in self.ducts.iter_mut().enumerate() {
            d.finish(&self.net, slots, dt, m_in[k] / dt, h_in[k] / dt);
        }
        for s in &self.sources {
            s.finish(slots);
        }
        for v in &self.valves {
            v.finish(slots);
        }
        for c in &self.consumers {
            c.finish(&self.net, slots, &self.ducts[c.duct]);
        }
        for port in &self.ports {
            let p = self.net.p[self.ducts[port.duct].node];
            slots[port.slot[2]] = p;
            slots[port.slot[3]] = slots[port.slot[1]].max(0.0) * (slots[port.slot[0]] - p);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut v = Vec::new();
        self.net.state(&mut v);
        for x in &self.ducts {
            x.state(&mut v);
        }
        for x in &self.sources {
            x.state(&mut v);
        }
        for x in &self.valves {
            x.state(&mut v);
        }
        v.push(self.started as u8 as f64);
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let mut it = s.iter().copied();
        self.net.restore(&mut it);
        for x in &mut self.ducts {
            x.restore(&mut it);
        }
        for x in &mut self.sources {
            x.restore(&mut it);
        }
        for x in &mut self.valves {
            x.restore(&mut it);
        }
        self.started = it.next().unwrap_or(1.0) != 0.0;
    }
}
