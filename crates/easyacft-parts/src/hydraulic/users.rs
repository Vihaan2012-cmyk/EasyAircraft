//! What takes flow from a circuit: consumers with servo valves whose flow follows the square root of the pressure
//! they see, behind priority valves that close smoothly as the circuit sags, leaking past their spools; parts
//! from other domains, such as actuators and brakes, through their flow outputs; and custom parts' ports.

use super::circuit::Circuit;
use super::net::Net;
use super::{eval, flag, put, yes, B, N, Q, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "hydraulic",
    kind: "consumer",
    help: "Actuators or a motor that use flow while they move: flow through its valves follows the square root of the pressure across them, a priority valve throttles it as the circuit sags, and its spools leak a little all the time.",
    fields: &[
        req("circuit", Kind::Ref(&["hydraulic.circuit"]), "the circuit it uses"),
        req("flow", Kind::Expr(Q), "flow while moving at full rate on nominal pressure, or an expression"),
        opt("moving_when", WHEN, "false", "when it is moving"),
        opt("minimum", Kind::Quantity(Dim::PRESSURE), "1000 psi", "lowest pressure it works on"),
        opt("priority_below", Kind::Quantity(Dim::PRESSURE), "0 psi", "a priority valve throttles it to hold the circuit at this, shut 5 % below"),
        opt("leak", Kind::Quantity(Dim::VOLUME_FLOW), "0 L/min", "spool and seal leakage at nominal pressure and 40 C"),
        opt("load", Kind::Expr(N), "0", "load on it as a share of what it can push: flow falls as the load takes the pressure"),
        opt("leak_when", WHEN, "false", "its supply line leaks: fluid goes overboard"),
        opt("leak_rate", Kind::Quantity(Dim::VOLUME_FLOW), "2 L/min", "that leak at nominal pressure"),
    ],
    outputs: OUT,
};

outputs!(OUT, uo { powered: B, flow: Q, rate: N, failed: B, leak_flow: Q, priority_open: N, stalled: B, overboard_flow: Q });

pub struct Consumer {
    base: usize,
    pub circuit: usize,
    flow_e: Option<Expr>,
    when: Option<Expr>,
    load_e: Option<Expr>,
    minimum: f64,
    priority: f64,
    leak_g: f64,
    ext_g: f64,
    ext_when: Option<Expr>,
    ext_on: bool,
    sum_lost: f64,
    flow: f64,
    load: f64,
    moving: bool,
    failed: bool,
    share: f64,
    g: f64,
    sum_q: f64,
    sum_leak: f64,
    n_sub: f64,
}

impl Consumer {
    pub fn new(p: &CheckedPart, circuit: usize, c: &Circuit) -> Self {
        Consumer {
            base: p.slot,
            circuit,
            flow_e: expr_of(p, "flow"),
            when: expr_of(p, "moving_when"),
            load_e: expr_of(p, "load"),
            minimum: p.num("minimum"),
            priority: p.num("priority_below"),
            leak_g: p.num("leak") / c.nominal,
            ext_g: p.num("leak_rate") / c.nominal,
            ext_when: expr_of(p, "leak_when"),
            ext_on: false,
            sum_lost: 0.0,
            flow: 0.0,
            load: 0.0,
            moving: false,
            failed: false,
            share: 1.0,
            g: 0.0,
            sum_q: 0.0,
            sum_leak: 0.0,
            n_sub: 0.0,
        }
    }

    pub fn begin(&mut self, slots: &[f64], dt: f64) {
        self.failed = slots[self.base + uo::failed] != 0.0;
        self.flow = eval(&mut self.flow_e, slots, dt, 0.0).max(0.0);
        self.load = eval(&mut self.load_e, slots, dt, 0.0).clamp(0.0, 1.0);
        self.moving = !self.failed && yes(&mut self.when, slots, dt, false);
        self.ext_on = yes(&mut self.ext_when, slots, dt, false);
        self.sum_lost = 0.0;
        self.sum_q = 0.0;
        self.sum_leak = 0.0;
        self.n_sub = 0.0;
    }

    pub fn stamp(&mut self, net: &mut Net, c: &Circuit) {
        let p = net.p[c.node];
        // Priority valve: a spool that closes over the 5 % below its setting.
        let target = if self.priority > 0.0 { ((p - self.priority) / (0.05 * self.priority)).clamp(0.0, 1.0) } else { 1.0 };
        self.share += (target - self.share) * 0.3;
        // Valve flow goes as the square root of the pressure left after the load takes its share.
        let dp_ref = c.nominal * (1.0 - self.load).max(0.02);
        let dp = (p * (1.0 - self.load) - 0.0).max(0.02 * c.nominal);
        let q = if self.moving && p > 0.0 { self.flow * self.share * (dp / dp_ref).sqrt().min(1.3) } else { 0.0 };
        self.g = q / p.max(0.02 * c.nominal);
        let leak = if self.failed { 0.0 } else { self.leak_g / c.visc };
        let ext = if self.ext_on { self.ext_g } else { 0.0 };
        net.stamp(c.node, self.g + leak + ext, 0.0);
        self.sum_lost += ext * p.max(0.0);
        self.sum_q += self.g * p.max(0.0);
        self.sum_leak += leak * p.max(0.0);
        self.n_sub += 1.0;
    }

    /// Held off by a shut valve ahead of it.
    pub fn hold(&mut self) {
        self.moving = false;
    }

    /// Fluid its leaking line put overboard over a step.
    pub fn lost(&self, dt: f64) -> f64 {
        self.sum_lost / self.n_sub.max(1.0) * dt
    }

    /// Flow returned, for the filters, and the heat its throttling makes.
    pub fn returned(&self, c_p: f64) -> (f64, f64) {
        let n = self.n_sub.max(1.0);
        let q = (self.sum_q + self.sum_leak) / n;
        (q, q * c_p.max(0.0))
    }

    pub fn finish(&self, slots: &mut [f64], c: &Circuit, net: &Net) {
        let p = net.p[c.node].max(0.0);
        let n = self.n_sub.max(1.0);
        let q = self.sum_q / n;
        let rate = if self.flow > 0.0 && self.moving { (q / self.flow).min(1.0) } else { 0.0 };
        let b = self.base;
        flag(slots, b + uo::powered, p >= self.minimum && self.share > 0.0 && !self.failed);
        put(slots, b + uo::flow, q);
        put(slots, b + uo::rate, rate);
        put(slots, b + uo::leak_flow, self.sum_leak / n);
        put(slots, b + uo::priority_open, self.share);
        put(slots, b + uo::overboard_flow, self.sum_lost / n);
        flag(slots, b + uo::stalled, self.moving && (p * (1.0 - self.load)) < self.minimum);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.push(self.share);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        self.share = it.next().unwrap_or(1.0);
    }
}

/// A part outside this domain drawing from a circuit through a `<field>_flow` output, and the fluid it holds.
pub struct Draw {
    pub circuit: usize,
    pub flow: usize,
    pub held: Option<usize>,
    q: f64,
}

impl Draw {
    pub fn new(circuit: usize, flow: usize, held: Option<usize>) -> Self {
        Draw { circuit, flow, held, q: 0.0 }
    }

    pub fn begin(&mut self, slots: &[f64]) {
        let q = slots[self.flow];
        self.q = if q.is_finite() { q.max(0.0) } else { 0.0 };
    }

    /// Last step's flow as a conductance at the pressure it was drawn at.
    pub fn stamp(&self, net: &mut Net, c: &Circuit) {
        if self.q > 0.0 {
            let p = net.p[c.node].max(0.05 * c.nominal);
            net.stamp(c.node, self.q / p, 0.0);
        }
    }

    pub fn flow(&self) -> f64 {
        self.q
    }

    pub fn held(&self, slots: &[f64]) -> f64 {
        self.held.map_or(0.0, |h| if slots[h].is_finite() { slots[h] } else { 0.0 })
    }
}

/// A custom part's port on a circuit: a Norton source, flow = g (e - p).
pub struct Port {
    pub circuit: usize,
    pub slot: [usize; 4],
}

impl Port {
    pub fn stamp(&self, net: &mut Net, c: &Circuit, slots: &[f64]) {
        let g = slots[self.slot[1]].max(0.0);
        if g > 0.0 && g.is_finite() {
            net.stamp(c.node, g, g * slots[self.slot[0]]);
        }
    }

    pub fn read(&self, net: &Net, c: &Circuit, slots: &mut [f64]) {
        let p = net.p[c.node];
        slots[self.slot[2]] = p;
        slots[self.slot[3]] = slots[self.slot[1]].max(0.0) * (slots[self.slot[0]] - p);
    }
}
