//! Accumulators: a nitrogen charge behind a piston. The gas heats as it is squeezed fast and cools back toward
//! its surroundings, so a quickly charged accumulator reads high and sags; the piston's seal friction holds a
//! small difference between fluid and gas; a check valve keeps a brake accumulator's fluid for its own users;
//! the precharge can leak away.

use super::circuit::Circuit;
use super::net::Net;
use super::{eval, put, yes, P, Q, T, VOL, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "hydraulic",
    kind: "accumulator",
    help: "A nitrogen-charged piston accumulator: its gas heats when squeezed fast and cools back, its piston seal holds a little friction, it fills from and gives back to its circuit, or behind a check valve only fills for its own users; the precharge can leak.",
    fields: &[
        req("circuit", Kind::Ref(&["hydraulic.circuit"]), "the circuit it is on"),
        req("volume", Kind::Quantity(Dim::VOLUME), "gas volume"),
        req("precharge", Kind::Quantity(Dim::PRESSURE), "gas precharge pressure at 15 C"),
        opt("check_valve", Kind::Bool, "false", "a check valve keeps its fluid for its own users, such as the brakes"),
        opt("draw", Kind::Expr(Q), "0 L/min", "flow its own users take from it"),
        opt("precharge_lost_when", WHEN, "false", "the nitrogen has leaked away"),
        opt("port", Kind::Quantity(Dim::LENGTH), "8 mm", "bore of the line joining it to its circuit"),
        opt("ambient", Kind::Expr(T), "288.15 K", "temperature around it"),
        opt("thermal_time", Kind::Quantity(Dim::TIME), "60 s", "how fast its gas comes back to the temperature around it"),
        opt("piston_friction", Kind::Quantity(Dim::PRESSURE), "15 psi", "pressure difference its piston seal holds"),
    ],
    outputs: OUT,
};

outputs!(OUT, ao { pressure: P, fluid: VOL, failed: super::B, gas_temperature: T, gauge: P, flow: Q, precharge_now: P });

/// Isentropic exponent of nitrogen; the gas runs between this and isothermal as heat flows.
const GAMMA: f64 = 1.4;
const T_REF: f64 = 288.15;

pub struct Accumulator {
    base: usize,
    pub circuit: usize,
    pub node: usize,
    pub link: usize,
    volume: f64,
    precharge: f64,
    check: bool,
    draw_e: Option<Expr>,
    lost_e: Option<Expr>,
    ambient: Option<Expr>,
    tau: f64,
    friction: f64,
    pub fluid: f64,
    gas_t: f64,
    p_gas: f64,
    pre: f64,
    draw: f64,
    amb: f64,
    failed: bool,
    sum_q: f64,
    n_sub: f64,
}

impl Accumulator {
    pub fn new(p: &CheckedPart, circuit: usize, c: &Circuit, net: &mut Net) -> Self {
        let volume = p.num("volume").max(1e-6);
        let pre = p.num("precharge").max(1.0);
        let node = net.node(volume / (GAMMA * pre));
        let bore = p.num("port").max(1e-3);
        let area = std::f64::consts::PI * bore * bore / 4.0;
        let link = net.link(c.node, node, 1e8, 870.0 * 1.0 / area, p.flag("check_valve"));
        net.p[node] = pre;
        Accumulator {
            base: p.slot,
            circuit,
            node,
            link,
            volume,
            precharge: pre,
            check: p.flag("check_valve"),
            draw_e: expr_of(p, "draw"),
            lost_e: expr_of(p, "precharge_lost_when"),
            ambient: expr_of(p, "ambient"),
            tau: p.num("thermal_time").max(1.0),
            friction: p.num("piston_friction").max(0.0),
            fluid: 0.0,
            gas_t: f64::NAN,
            p_gas: pre,
            pre,
            draw: 0.0,
            amb: T_REF,
            failed: false,
            sum_q: 0.0,
            n_sub: 0.0,
        }
    }

    fn gas_pressure(&self) -> f64 {
        let gas = (self.volume - self.fluid).max(0.02 * self.volume);
        // Precharge is given at 15 C; the gas law carries both volume and temperature.
        self.pre * (self.volume / gas) * (self.gas_t / T_REF)
    }

    pub fn begin(&mut self, slots: &[f64], dt: f64) {
        self.failed = slots[self.base + ao::failed] != 0.0;
        self.pre = if yes(&mut self.lost_e, slots, dt, false) { 0.05 * self.precharge } else { self.precharge };
        self.draw = eval(&mut self.draw_e, slots, dt, 0.0).max(0.0);
        self.amb = eval(&mut self.ambient, slots, dt, T_REF);
        if self.gas_t.is_nan() {
            self.gas_t = self.amb;
        }
        self.sum_q = 0.0;
        self.n_sub = 0.0;
    }

    /// Sets the node to the gas pressure and its compliance to the gas spring's, for one substep.
    pub fn stamp(&mut self, net: &mut Net) {
        self.p_gas = self.gas_pressure();
        let gas = (self.volume - self.fluid).max(0.02 * self.volume);
        // Fluid side sits within the seal's friction of the gas.
        let p_fluid = net.p[self.node];
        let lo = self.p_gas - self.friction;
        let hi = self.p_gas + self.friction;
        net.p[self.node] = if self.fluid <= 1e-9 { self.p_gas } else { p_fluid.clamp(lo, hi) };
        net.cap[self.node] = gas / (GAMMA * self.p_gas.max(1e3)) + 1e-13;
        let link = &mut net.links[self.link];
        link.check = self.check || self.fluid <= 1e-9;
        link.r = if self.failed { 1e16 } else { 5e7 + 2e10 * link.q.abs() };
        // Its own users draw from the fluid side.
        let draw = if self.fluid > 1e-9 { self.draw } else { 0.0 };
        net.stamp(self.node, 0.0, -draw);
    }

    /// After a substep: fluid in from the circuit, gas heated by compression and cooled toward ambient.
    pub fn after(&mut self, net: &Net, h: f64) {
        let q = net.links[self.link].q;
        let draw = if self.fluid > 1e-9 { self.draw } else { 0.0 };
        let before = (self.volume - self.fluid).max(0.02 * self.volume);
        self.fluid = (self.fluid + (q - draw) * h).clamp(0.0, 0.98 * self.volume);
        let after = (self.volume - self.fluid).max(0.02 * self.volume);
        // Adiabatic compression heats the gas, T V^(gamma-1) constant over the substep, then it relaxes.
        self.gas_t *= (before / after).powf(GAMMA - 1.0);
        self.gas_t = (self.gas_t + h / self.tau * self.amb) / (1.0 + h / self.tau);
        self.sum_q += q;
        self.n_sub += 1.0;
    }

    pub fn finish(&self, net: &Net, slots: &mut [f64]) {
        let b = self.base;
        let p = if self.fluid > 0.0 { self.gas_pressure() } else { self.pre.min(net.p[self.node].max(self.pre)) };
        put(slots, b + ao::pressure, p);
        put(slots, b + ao::fluid, self.fluid);
        put(slots, b + ao::gas_temperature, self.gas_t);
        put(slots, b + ao::gauge, self.gas_pressure());
        put(slots, b + ao::flow, self.sum_q / self.n_sub.max(1.0));
        put(slots, b + ao::precharge_now, self.pre * self.amb / T_REF);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.fluid, self.gas_t]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        self.fluid = it.next().unwrap_or(0.0);
        self.gas_t = it.next().unwrap_or(T_REF);
    }
}
