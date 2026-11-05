//! Where bleed air comes from and goes. Sources are engine bleed ports, APU load compressors or ground carts
//! behind check valves that crack at a small pressure difference. Users draw through their own valves to their
//! exhaust: packs behind a flow control valve and a venturi that chokes, anti-ice valves, and air turbine
//! starters whose shaft power comes from the expansion of the air through them.

use super::duct::Duct;
use super::{eval, flag, put, yes, B, MF, N, P, T, W, WHEN};
use crate::cabin::orifice;
use crate::hydraulic::net::Net;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SOURCE: PartSpec = PartSpec {
    domain: "pneumatic",
    kind: "source",
    help: "An engine bleed port, APU load compressor or ground air cart, behind a check valve that cracks at a small pressure difference.",
    fields: &[
        req("duct", Kind::Ref(&["pneumatic.duct"]), "the duct it feeds"),
        req("pressure", Kind::Expr(P), "delivery pressure at no flow, or an expression such as the engine's compressor pressure"),
        req("temperature", Kind::Expr(T), "delivery temperature, or an expression"),
        req("flow", Kind::Expr(MF), "flow at full output, or an expression"),
        opt("available_when", WHEN, "true", "when it can supply"),
        opt("crack", Kind::Quantity(Dim::PRESSURE), "0.3 psi", "pressure difference that opens its check valve"),
        opt("check_failed_when", WHEN, "false", "its check valve stuck open: air can flow back into it"),
    ],
    outputs: SOURCE_OUT,
};

pub const CONSUMER: PartSpec = PartSpec {
    domain: "pneumatic",
    kind: "consumer",
    help: "A pack, anti-ice valve, engine starter or other user of bleed air, drawing to its exhaust; a flow control valve holds its flow, a venturi chokes it, an air turbine starter turns the air's expansion into shaft power.",
    fields: &[
        req("duct", Kind::Ref(&["pneumatic.duct"]), "the duct it draws from"),
        req("flow", Kind::Expr(MF), "flow it takes at nominal pressure, or an expression"),
        opt("nominal", Kind::Quantity(Dim::PRESSURE), "45 psi", "pressure its flow is given at"),
        opt("minimum", Kind::Quantity(Dim::PRESSURE), "15 psi", "below this it is not supplied"),
        opt("on_when", WHEN, "true", "when it draws"),
        opt("regulated", Kind::Bool, "false", "a flow control valve holds its flow above the minimum pressure"),
        opt("exhaust", Kind::Expr(P), "101325 Pa", "pressure it exhausts to: the cabin, a wing leading edge, outside"),
        opt("venturi_area", Kind::Number, "0", "throat area of a flow-limiting venturi, m2; 0 for none"),
        opt("turbine", Kind::Bool, "false", "an air turbine starter: it gives shaft power"),
        opt("efficiency", Kind::Number, "0.7", "turbine efficiency"),
    ],
    outputs: CONSUMER_OUT,
};

outputs!(SOURCE_OUT, so { available: B, flow: MF, failed: B, check_open: B, reverse_flow: MF });
outputs!(CONSUMER_OUT, co { supplied: B, flow: MF, temperature: T, failed: B, choked: B, shaft_power: W, pressure_ratio: N });

const CP: f64 = 1005.0;
const GAMMA: f64 = 1.4;

pub struct Source {
    base: usize,
    pub duct: usize,
    flow_e: Option<Expr>,
    pressure_e: Option<Expr>,
    temp_e: Option<Expr>,
    when: Option<Expr>,
    check_failed: Option<Expr>,
    crack: f64,
    pub pressure: f64,
    pub temp: f64,
    rated: f64,
    g: f64,
    on: bool,
    check_open: bool,
    leaky: bool,
    sum_q: f64,
    n_sub: f64,
}

impl Source {
    pub fn new(p: &CheckedPart, duct: usize) -> Self {
        Source {
            base: p.slot,
            duct,
            flow_e: expr_of(p, "flow"),
            pressure_e: expr_of(p, "pressure"),
            temp_e: expr_of(p, "temperature"),
            when: expr_of(p, "available_when"),
            check_failed: expr_of(p, "check_failed_when"),
            crack: p.num("crack").max(0.0),
            pressure: 0.0,
            temp: 288.15,
            rated: 0.0,
            g: 0.0,
            on: false,
            check_open: false,
            leaky: false,
            sum_q: 0.0,
            n_sub: 0.0,
        }
    }

    pub fn begin(&mut self, slots: &mut [f64], dt: f64) {
        self.pressure = eval(&mut self.pressure_e, slots, dt, 0.0).max(0.0);
        self.temp = eval(&mut self.temp_e, slots, dt, 288.15);
        self.rated = eval(&mut self.flow_e, slots, dt, 0.0).max(0.0);
        self.leaky = yes(&mut self.check_failed, slots, dt, false);
        self.on = slots[self.base + so::failed] == 0.0 && self.pressure > 0.0 && yes(&mut self.when, slots, dt, true);
        // The port droops with flow: full flow at 70 % of its no-flow pressure.
        self.g = self.rated / (0.3 * self.pressure.max(1e3));
        flag(slots, self.base + so::available, self.on);
        self.sum_q = 0.0;
        self.n_sub = 0.0;
    }

    pub fn stamp(&mut self, net: &mut Net, duct: &Duct) {
        let p = net.p[duct.node];
        // Check valve with a little hysteresis around its cracking pressure.
        let dp = self.pressure - p;
        self.check_open = self.on && if self.check_open { dp > 0.0 } else { dp > self.crack };
        if self.check_open || (self.leaky && self.on) {
            net.stamp(duct.node, self.g, self.g * self.pressure);
        } else if self.leaky {
            // A stuck-open check valve on a dead source lets the manifold blow back out through it.
            net.stamp(duct.node, 0.2 * self.g.max(1e-6), 0.2 * self.g.max(1e-6) * duct.p_amb);
        }
    }

    pub fn after(&mut self, net: &Net, duct: &Duct) -> f64 {
        let p = net.p[duct.node];
        let q = if self.check_open || (self.leaky && self.on) { self.g * (self.pressure - p) } else if self.leaky { 0.2 * self.g.max(1e-6) * (duct.p_amb - p) } else { 0.0 };
        self.sum_q += q;
        self.n_sub += 1.0;
        q.max(0.0)
    }

    pub fn finish(&self, slots: &mut [f64]) {
        let q = self.sum_q / self.n_sub.max(1.0);
        let b = self.base;
        put(slots, b + so::flow, q.max(0.0));
        flag(slots, b + so::check_open, self.check_open);
        put(slots, b + so::reverse_flow, (-q).max(0.0));
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.push(self.check_open as u8 as f64);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        self.check_open = it.next().unwrap_or(0.0) != 0.0;
    }
}

pub struct Consumer {
    base: usize,
    pub duct: usize,
    flow_e: Option<Expr>,
    when: Option<Expr>,
    exhaust_e: Option<Expr>,
    nominal: f64,
    minimum: f64,
    regulated: bool,
    venturi: f64,
    turbine: bool,
    eff: f64,
    flow: f64,
    exhaust: f64,
    drawing: bool,
    g: f64,
    choked: bool,
    sum_q: f64,
    n_sub: f64,
}

impl Consumer {
    pub fn new(p: &CheckedPart, duct: usize) -> Self {
        Consumer {
            base: p.slot,
            duct,
            flow_e: expr_of(p, "flow"),
            when: expr_of(p, "on_when"),
            exhaust_e: expr_of(p, "exhaust"),
            nominal: p.num("nominal").max(1.0),
            minimum: p.num("minimum"),
            regulated: p.flag("regulated"),
            venturi: 0.97 * p.num("venturi_area").max(0.0),
            turbine: p.flag("turbine"),
            eff: p.num("efficiency").clamp(0.05, 1.0),
            flow: 0.0,
            exhaust: 101_325.0,
            drawing: false,
            g: 0.0,
            choked: false,
            sum_q: 0.0,
            n_sub: 0.0,
        }
    }

    pub fn begin(&mut self, slots: &[f64], dt: f64) {
        self.flow = eval(&mut self.flow_e, slots, dt, 0.0).max(0.0);
        self.exhaust = eval(&mut self.exhaust_e, slots, dt, 101_325.0).max(0.0);
        self.drawing = slots[self.base + co::failed] == 0.0 && yes(&mut self.when, slots, dt, true);
        self.sum_q = 0.0;
        self.n_sub = 0.0;
    }

    pub fn stamp(&mut self, net: &mut Net, duct: &Duct) {
        let p = net.p[duct.node];
        let span = (self.nominal - self.exhaust).max(1e3);
        let drive = (p - self.exhaust).max(0.0);
        // A flow control valve holds its flow while it has the pressure to; otherwise flow goes with the
        // pressure across it.
        let mut q = if !self.drawing {
            0.0
        } else if self.regulated && p >= self.minimum {
            self.flow
        } else {
            self.flow * drive / span
        };
        self.choked = false;
        if self.venturi > 0.0 && self.drawing {
            let limit = orifice(self.venturi, p.max(1.0), duct.temp, 0.0);
            if q > limit {
                q = limit;
                self.choked = true;
            }
        }
        self.g = if drive > 1.0 { q / drive } else { 0.0 };
        if self.g > 0.0 {
            net.stamp(duct.node, self.g, self.g * self.exhaust);
        }
    }

    pub fn after(&mut self, net: &Net, duct: &Duct) -> f64 {
        let q = self.g * (net.p[duct.node] - self.exhaust).max(0.0);
        self.sum_q += q;
        self.n_sub += 1.0;
        q
    }

    pub fn finish(&self, net: &Net, slots: &mut [f64], duct: &Duct) {
        let p = net.p[duct.node];
        let q = self.sum_q / self.n_sub.max(1.0);
        let ratio = if self.exhaust > 0.0 { p / self.exhaust } else { 1.0 };
        let shaft = if self.turbine && ratio > 1.0 { self.eff * q * CP * duct.temp * (1.0 - ratio.powf(-(GAMMA - 1.0) / GAMMA)) } else { 0.0 };
        let b = self.base;
        flag(slots, b + co::supplied, self.drawing && p >= self.minimum);
        put(slots, b + co::flow, q);
        put(slots, b + co::temperature, duct.temp);
        flag(slots, b + co::choked, self.choked);
        put(slots, b + co::shaft_power, shaft);
        put(slots, b + co::pressure_ratio, ratio);
    }
}
