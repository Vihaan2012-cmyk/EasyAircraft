//! Bleed valves: shutoff, crossbleed and pressure-regulating valves. A butterfly whose compressible flow chokes,
//! driven by its own upstream air against a spring so it shuts when that air is gone; a regulator that trims its
//! opening to hold downstream pressure; overpressure, overtemperature and reverse-flow shutoffs; travel time;
//! sticking; a seat that leaks; and a precooler whose fan-air valve modulates cooling flow through a heat
//! exchanger sized by its effectiveness.

use super::duct::Duct;
use super::{eval, flag, put, yes, B, MF, N, T, W, WHEN};
use crate::cabin::orifice;
use crate::hydraulic::net::Net;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "pneumatic",
    kind: "valve",
    help: "A shutoff, crossbleed or pressure-regulating valve: compressible flow that chokes, travel time, spring-closed pneumatic actuation, a downstream pressure regulator, overpressure, overtemperature and reverse-flow shutoffs, sticking, seat leakage, and an optional precooler whose fan-air valve modulates cooling air through it.",
    fields: &[
        req("between", Kind::Refs(&["pneumatic.duct"]), "upstream and downstream duct"),
        opt("open_when", WHEN, "true", "when it is commanded open"),
        opt("flow", Kind::Quantity(Dim::MASS_FLOW), "2 kg/s", "flow at 1 bar across it, when no area is given"),
        opt("area", Kind::Number, "0", "open flow area, m2; flow is then compressible and chokes"),
        opt("open_time", Kind::Quantity(Dim::TIME), "0 s", "full travel time"),
        opt("actuation_pressure", Kind::Quantity(Dim::PRESSURE), "0 Pa", "upstream pressure it needs to open; it springs shut below; 0 for an electric valve"),
        opt("regulate", Kind::Quantity(Dim::PRESSURE), "0 Pa", "downstream pressure it holds; 0 for a plain valve"),
        opt("overpressure", Kind::Quantity(Dim::PRESSURE), "0 Pa", "it shuts when downstream goes above this, and reopens 10 % below"),
        opt("stuck_when", WHEN, "false", "it stays where it is"),
        opt("cool_to", Kind::Quantity(Dim::TEMPERATURE), "0 K", "precooler outlet temperature its fan-air valve aims for; 0 for none"),
        opt("effectiveness", Kind::Number, "0.8", "precooler effectiveness with full fan air at rated flow"),
        opt("fan_temperature", Kind::Expr(T), "318.15 K", "fan air temperature"),
        opt("fan_air_when", WHEN, "true", "fan air reaches the precooler"),
        opt("fan_flow", Kind::Quantity(Dim::MASS_FLOW), "0 kg/s", "fan air through the precooler with its valve wide open; 0 sizes it to the bleed"),
        opt("overtemperature", Kind::Quantity(Dim::TEMPERATURE), "0 K", "it shuts when its outlet goes above this; 0 for none"),
        opt("reverse_flow_shutoff", Kind::Bool, "false", "it shuts if flow turns back through it"),
        opt("leak_when", WHEN, "false", "its seat leaks when shut"),
        opt("regulator_time", Kind::Quantity(Dim::TIME), "0.25 s", "how fast its regulator trims the opening"),
    ],
    outputs: OUT,
};

outputs!(OUT, vo { open: B, position: N, flow: MF, outlet_temperature: T, failed: B, fan_valve: N, heat_rejected: W, shut_reason: N, regulating: B, choked: B });

const CP: f64 = 1005.0;

pub struct Valve {
    base: usize,
    pub link: usize,
    pub ends: [usize; 2],
    r_open: f64,
    cda: f64,
    open_time: f64,
    actuation: f64,
    regulate: f64,
    overpressure: f64,
    over_t: f64,
    reverse_shut: bool,
    when: Option<Expr>,
    stuck: Option<Expr>,
    leak_e: Option<Expr>,
    cool_to: f64,
    eff: f64,
    fan_t: Option<Expr>,
    fan_when: Option<Expr>,
    fan_flow: f64,
    reg_tau: f64,
    pub position: f64,
    opening: f64,
    fav: f64,
    shut_op: bool,
    shut_ot: bool,
    shut_rev: bool,
    leaking: bool,
    pub t_out: f64,
    fan_t_now: f64,
    fan_ok: bool,
    heat: f64,
    sum_q: f64,
    n_sub: f64,
    choked: bool,
}

impl Valve {
    pub fn new(p: &CheckedPart, net: &mut Net, a: usize, b: usize, na: usize, nb: usize) -> Self {
        let r = 1e5 / p.num("flow").max(1e-9);
        let link = net.link(na, nb, r, 0.0, false);
        Valve {
            base: p.slot,
            link,
            ends: [a, b],
            r_open: r,
            cda: 0.8 * p.num("area").max(0.0),
            open_time: p.num("open_time").max(0.0),
            actuation: p.num("actuation_pressure"),
            regulate: p.num("regulate"),
            overpressure: p.num("overpressure"),
            over_t: p.num("overtemperature"),
            reverse_shut: p.flag("reverse_flow_shutoff"),
            when: expr_of(p, "open_when"),
            stuck: expr_of(p, "stuck_when"),
            leak_e: expr_of(p, "leak_when"),
            cool_to: p.num("cool_to"),
            eff: p.num("effectiveness").clamp(0.0, 0.99),
            fan_t: expr_of(p, "fan_temperature"),
            fan_when: expr_of(p, "fan_air_when"),
            fan_flow: p.num("fan_flow").max(0.0),
            reg_tau: p.num("regulator_time").max(0.01),
            position: 0.0,
            opening: 1.0,
            fav: 0.5,
            shut_op: false,
            shut_ot: false,
            shut_rev: false,
            leaking: false,
            t_out: 288.15,
            fan_t_now: 318.15,
            fan_ok: true,
            heat: 0.0,
            sum_q: 0.0,
            n_sub: 0.0,
            choked: false,
        }
    }

    /// `amb_up` is the pressure around the upstream duct: its actuator works on the difference.
    pub fn begin(&mut self, slots: &mut [f64], dt: f64, p_up: f64, p_down: f64, amb_up: f64) {
        if self.overpressure > 0.0 {
            self.shut_op = if self.shut_op { p_down > 0.9 * self.overpressure } else { p_down > self.overpressure };
        }
        if self.over_t > 0.0 {
            self.shut_ot = if self.shut_ot { self.t_out > self.over_t - 20.0 } else { self.t_out > self.over_t };
        }
        if self.reverse_shut {
            let q = self.sum_q / self.n_sub.max(1.0);
            self.shut_rev = if self.shut_rev { p_down >= p_up } else { q < -1e-3 };
        }
        let failed = slots[self.base + vo::failed] != 0.0;
        let air = self.actuation <= 0.0 || p_up - amb_up >= self.actuation;
        let commanded = !failed && yes(&mut self.when, slots, dt, true) && !self.shut_op && !self.shut_ot && !self.shut_rev && air;
        if !yes(&mut self.stuck, slots, dt, false) {
            let target = if commanded { 1.0 } else { 0.0 };
            self.position = if self.open_time > 0.0 { self.position + (target - self.position).clamp(-dt / self.open_time, dt / self.open_time) } else { target };
        }
        if self.regulate > 0.0 && commanded {
            let err = (self.regulate - p_down) / self.regulate;
            self.opening = (self.opening + err * dt / self.reg_tau).clamp(0.02, 1.0);
        } else {
            self.opening = 1.0;
        }
        self.leaking = yes(&mut self.leak_e, slots, dt, false);
        self.fan_t_now = eval(&mut self.fan_t, slots, dt, 318.15);
        self.fan_ok = yes(&mut self.fan_when, slots, dt, true);
        // Fan-air valve trims cooling flow to hold the precooler outlet at its set point.
        if self.cool_to > 0.0 {
            let err = (self.t_out - self.cool_to) / 100.0;
            self.fav = (self.fav + err * dt / 0.8).clamp(0.0, 1.0);
        }
        self.sum_q = 0.0;
        self.n_sub = 0.0;
    }

    /// Flow area this substep, linearised at its pressures; a compressible orifice when it has an area.
    pub fn stamp(&mut self, net: &mut Net, up: &Duct, down: &Duct, na: usize, nb: usize) {
        let share = (self.position * self.opening).max(if self.leaking { 0.01 } else { 0.0 });
        let l = &mut net.links[self.link];
        if share <= 1e-3 {
            l.open = false;
            l.q = 0.0;
            return;
        }
        l.open = true;
        let (pa, pb) = (net.p[na], net.p[nb]);
        l.r = if self.cda > 0.0 {
            let (hi, lo, t) = if pa >= pb { (pa, pb, up.temp) } else { (pb, pa, down.temp) };
            let hi = hi.max(1e3);
            let dp = (hi - lo).max(0.002 * hi);
            let q = orifice(self.cda * share, hi, t, hi - dp).max(1e-9);
            self.choked = lo / hi < 0.53;
            dp / q
        } else {
            self.r_open / share
        };
    }

    pub fn after(&mut self, net: &Net) -> f64 {
        let q = net.links[self.link].q;
        self.sum_q += q;
        self.n_sub += 1.0;
        q
    }

    /// Outlet temperature after the precooler for flow from `t_in`; returns the heat it rejected to fan air.
    pub fn cool(&mut self, t_in: f64, q: f64) -> f64 {
        if self.cool_to <= 0.0 {
            self.t_out = t_in;
            self.heat = 0.0;
            return 0.0;
        }
        let q = q.abs().max(1e-4);
        let fan_full = if self.fan_flow > 0.0 { self.fan_flow } else { 1.5 * q };
        // Without fan air only a little ram leakage gets through the precooler.
        let fan = fan_full * self.fav.max(0.03) * if self.fan_ok { 1.0 } else { 0.03 };
        // Effectiveness-NTU for a crossflow exchanger, sized from its rated effectiveness with full fan air.
        let ntu_rated = -(1.0 - self.eff).ln();
        let (c_hot, c_cold) = (q * CP, fan * CP);
        let (c_min, c_max) = (c_hot.min(c_cold), c_hot.max(c_cold));
        let cr = c_min / c_max;
        // Conductance scales with the fan air through it, about as its flow to the 0.6.
        let ua = ntu_rated * c_hot * (fan / fan_full).powf(0.6);
        let ntu = ua / c_min.max(1e-6);
        let e = if (1.0 - cr).abs() < 1e-6 { ntu / (1.0 + ntu) } else { (1.0 - (-ntu * (1.0 - cr)).exp()) / (1.0 - cr * (-ntu * (1.0 - cr)).exp()) };
        let qdot = e * c_min * (t_in - self.fan_t_now).max(0.0);
        self.t_out = t_in - qdot / c_hot;
        self.heat = qdot;
        qdot
    }

    pub fn flow(&self) -> f64 {
        self.sum_q / self.n_sub.max(1.0)
    }

    pub fn finish(&self, slots: &mut [f64]) {
        let b = self.base;
        let open = self.position > 1e-3;
        let reason = if self.shut_op {
            1.0
        } else if self.shut_ot {
            2.0
        } else if self.shut_rev {
            3.0
        } else {
            0.0
        };
        flag(slots, b + vo::open, open);
        put(slots, b + vo::position, self.position);
        put(slots, b + vo::flow, self.flow().abs());
        put(slots, b + vo::outlet_temperature, self.t_out);
        put(slots, b + vo::fan_valve, if self.cool_to > 0.0 { self.fav } else { 0.0 });
        put(slots, b + vo::heat_rejected, self.heat);
        put(slots, b + vo::shut_reason, reason);
        flag(slots, b + vo::regulating, self.regulate > 0.0 && open && self.opening < 0.98);
        flag(slots, b + vo::choked, open && self.choked);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        let b = |x: bool| x as u8 as f64;
        v.extend([self.position, self.opening, self.fav, b(self.shut_op), b(self.shut_ot), b(self.shut_rev), self.t_out]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.position = n();
        self.opening = n();
        self.fav = n();
        self.shut_op = n() != 0.0;
        self.shut_ot = n() != 0.0;
        self.shut_rev = n() != 0.0;
        self.t_out = n();
    }
}
