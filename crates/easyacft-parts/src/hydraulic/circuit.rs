//! A hydraulic circuit's own parts: the pressure manifold the lines and hoses make, the reservoir that
//! pressurises the pump inlets (bleed air, bootstrap or vented), the fluid with its temperature, viscosity,
//! thermal expansion, entrained air and contamination, the return and pressure filters, the system relief
//! valve, internal and external leakage, the heat exchanger, and the transmitters the crew sees.

use super::net::Net;
use super::{eval, flag, put, yes, B, N, P, Q, T, VOL, W, WHEN};
use crate::actuation::fluid::{Fluid, P_ATM};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "hydraulic",
    kind: "circuit",
    help: "One hydraulic system with its reservoir: lines and hoses that store fluid under pressure, a reservoir pressurised by bleed air, by system pressure through a bootstrap piston, or vented; fluid whose viscosity, volume and stiffness follow its temperature and entrained air; filters that clog and bypass; a relief valve; leaks; a heat exchanger; pressure and quantity transmitters.",
    fields: &[
        req("nominal", Kind::Quantity(Dim::PRESSURE), "for example \"3000 psi\""),
        req("fluid", Kind::Quantity(Dim::VOLUME), "reservoir quantity at start, for example \"20 L\""),
        opt("low_pressure", Kind::Quantity(Dim::PRESSURE), "1500 psi", "below this the circuit counts as lost"),
        opt("compliance", Kind::Number, "1e-10", "volume per pascal the lines and hoses store"),
        opt("leak", Kind::Quantity(Dim::VOLUME_FLOW), "0.05 L/min", "internal leakage back to the reservoir at nominal pressure and 40 C"),
        opt("leak_when", WHEN, "false", "an external leak: fluid goes overboard"),
        opt("leak_rate", Kind::Quantity(Dim::VOLUME_FLOW), "2 L/min", "that leak at nominal pressure"),
        opt("low_level", Kind::Quantity(Dim::VOLUME), "0 L", "reservoir low-level warning; 0 L means a quarter of the starting fluid"),
        opt("reservoir_pressure", Kind::Expr(P), "50 psi", "air pressurisation above ambient, for an air-pressurised reservoir"),
        opt("ambient", Kind::Expr(P), "101325 Pa", "outside pressure, for an unpressurised reservoir"),
        opt("cavitation_pressure", Kind::Quantity(Dim::PRESSURE), "15 psi", "absolute pump inlet pressure needed at full flow"),
        opt("cooling_temperature", Kind::Expr(T), "288.15 K", "what the heat exchanger dumps heat into, usually fuel"),
        opt("heat_exchanger", Kind::Number, "0", "heat exchanger conductance, W per K; 0 sizes it from the reservoir"),
        opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "368.15 K", "fluid overheat warning"),
        opt("reservoir", Kind::Text, "air", "air (bleed-air pressurised), bootstrap (a piston from system pressure) or vented"),
        opt("bootstrap_ratio", Kind::Number, "50", "bootstrap reservoir: system pressure over reservoir pressure"),
        opt("reservoir_failed_when", WHEN, "false", "reservoir pressurisation lost: regulator failed or a leaking air line"),
        opt("fluid_type", Kind::Text, "phosphate_ester", "phosphate_ester (Skydrol), mineral (MIL-PRF-5606), synthetic (MIL-PRF-83282) or low_temperature"),
        opt("relief", Kind::Quantity(Dim::PRESSURE), "0 psi", "system relief valve cracking pressure; 0 sets 110 % of nominal"),
        opt("ambient_temperature", Kind::Expr(T), "288.15 K", "temperature of the bays the lines run through"),
        opt("return_filter_dp", Kind::Quantity(Dim::PRESSURE), "15 psi", "clean return filter pressure drop at full return flow and 40 C"),
        opt("filter_clog_when", WHEN, "false", "a contaminated system: filters load up fast"),
        opt("rated_flow", Kind::Quantity(Dim::VOLUME_FLOW), "0 L/min", "return flow the filters and lines are sized for; 0 sums the pumps"),
    ],
    outputs: OUT,
};

outputs!(OUT, co {
    pressure: P, fluid: VOL, temperature: T, available: B, low: B, low_level: B, overheat: B, return_pressure: P, failed: B,
    reservoir_pressure: P, quantity: N, viscosity: N, air: N, contamination: N, filter_dp: P, filter_bypass: B, filter_clogged: B,
    relief_flow: Q, leak_flow: Q, transmitter: P, low_pressure_switch: B, heat: W, overboard: VOL
});

/// Fluid density and specific heat, with the pipes and components counted as twice the fluid's heat capacity.
const FLUID_HEAT: f64 = 870.0 * 1900.0 * 3.0;
const EXCHANGER_PER_LITRE: f64 = 15.0;
/// Viscosity at the reference the leak and filter ratings are given at, 40 C.
const T_RATED: f64 = 313.15;
/// Relief valve: full flow 10 % above cracking.
const RELIEF_SPAN: f64 = 0.1;
const LINE_LOSS_PER_LITRE: f64 = 0.6;
/// Entrained air: how fast it comes out in the reservoir, s.
const DEAERATE_S: f64 = 40.0;
/// Pressure transmitter time constant and low-pressure switch band.
const TRANSMITTER_TAU: f64 = 0.15;

#[derive(Clone, Copy, PartialEq)]
pub enum Reservoir {
    Air,
    Bootstrap,
    Vented,
}

pub struct Circuit {
    base: usize,
    pub node: usize,
    pub nominal: f64,
    low: f64,
    compliance: f64,
    fluid0: f64,
    /// Fluid in the circuit at 15 C, less what has gone overboard.
    pub fluid: f64,
    pub level: f64,
    low_level: f64,
    leak_g: f64,
    ext_g: f64,
    leak_when: Option<Expr>,
    res_pressure: Option<Expr>,
    ambient: Option<Expr>,
    res_failed: Option<Expr>,
    pub cavitation: f64,
    cooling: Option<Expr>,
    exchanger: f64,
    heat_capacity: f64,
    overheat: f64,
    pub reservoir: Reservoir,
    bootstrap: f64,
    pub fl: Fluid,
    relief: f64,
    amb_t: Option<Expr>,
    filter_dp0: f64,
    clog: Option<Expr>,
    rated_flow: f64,
    pub temp: f64,
    /// Heat into the fluid this step, W.
    pub heat: f64,
    /// Absolute pump inlet pressure the reservoir gives this step.
    pub inlet: f64,
    pub air: f64,
    pub contamination: f64,
    pub failed: bool,
    ext_on: bool,
    relief_q: f64,
    leak_q: f64,
    return_q: f64,
    overboard: f64,
    transmitter: f64,
    switch_low: bool,
    filter_dp: f64,
    bypass: bool,
    pub return_pressure: f64,
    /// Viscosity ratio to the rated 40 C one.
    pub visc: f64,
    cool_t: f64,
    amb: f64,
    heat_last: f64,
}

impl Circuit {
    pub fn new(p: &CheckedPart, net: &mut Net) -> Self {
        let nominal = p.num("nominal").max(1.0);
        let fluid = p.num("fluid");
        let low_level = p.num("low_level");
        let exchanger = p.num("heat_exchanger");
        let cap = p.num("compliance").max(1e-14);
        let relief = p.num("relief");
        let fl = Fluid::parse(p.text("fluid_type"));
        Circuit {
            base: p.slot,
            node: net.node(cap),
            nominal,
            low: p.num("low_pressure"),
            compliance: cap,
            fluid0: fluid.max(1e-6),
            fluid,
            level: fluid,
            low_level: if low_level > 0.0 { low_level } else { 0.25 * fluid },
            leak_g: p.num("leak") / nominal,
            ext_g: p.num("leak_rate") / nominal,
            leak_when: expr_of(p, "leak_when"),
            res_pressure: expr_of(p, "reservoir_pressure"),
            ambient: expr_of(p, "ambient"),
            res_failed: expr_of(p, "reservoir_failed_when"),
            cavitation: p.num("cavitation_pressure"),
            cooling: expr_of(p, "cooling_temperature"),
            exchanger: if exchanger > 0.0 { exchanger } else { EXCHANGER_PER_LITRE * fluid * 1000.0 },
            heat_capacity: (fluid * FLUID_HEAT).max(1.0),
            overheat: p.num("overheat_temperature"),
            reservoir: match p.text("reservoir") {
                "bootstrap" => Reservoir::Bootstrap,
                "vented" => Reservoir::Vented,
                _ => Reservoir::Air,
            },
            bootstrap: p.num("bootstrap_ratio").max(1.0),
            fl,
            relief: if relief > 0.0 { relief } else { 1.1 * nominal },
            amb_t: expr_of(p, "ambient_temperature"),
            filter_dp0: p.num("return_filter_dp").max(0.0),
            clog: expr_of(p, "filter_clog_when"),
            rated_flow: p.num("rated_flow"),
            temp: f64::NAN,
            heat: 0.0,
            inlet: P_ATM,
            air: 0.0,
            contamination: 0.0,
            failed: false,
            ext_on: false,
            relief_q: 0.0,
            leak_q: 0.0,
            return_q: 0.0,
            overboard: 0.0,
            transmitter: 0.0,
            switch_low: true,
            filter_dp: 0.0,
            bypass: false,
            return_pressure: P_ATM,
            visc: 1.0,
            cool_t: 288.15,
            amb: 288.15,
            heat_last: 0.0,
        }
    }

    /// Sizes the filters from the pumps when no rated flow was given.
    pub fn set_rated_flow(&mut self, pumps: f64) {
        if self.rated_flow <= 0.0 {
            self.rated_flow = pumps.max(1e-5);
        }
    }

    /// Evaluates the circuit's own conditions for this domain step.
    pub fn begin(&mut self, slots: &[f64], dt: f64, net: &Net) {
        self.failed = slots[self.base + co::failed] != 0.0;
        self.ext_on = yes(&mut self.leak_when, slots, dt, false);
        self.cool_t = eval(&mut self.cooling, slots, dt, 288.15);
        self.amb = eval(&mut self.amb_t, slots, dt, 288.15);
        if self.temp.is_nan() {
            self.temp = self.cool_t;
        }
        let ambient = eval(&mut self.ambient, slots, dt, P_ATM).max(0.0);
        let res_lost = yes(&mut self.res_failed, slots, dt, false);
        let p = net.p[self.node].max(0.0);
        let over = match self.reservoir {
            _ if res_lost => 0.0,
            Reservoir::Air => eval(&mut self.res_pressure, slots, dt, 0.0).max(0.0),
            Reservoir::Bootstrap => p / self.bootstrap,
            Reservoir::Vented => 0.0,
        };
        // A nearly empty reservoir also loses head as the outlet uncovers.
        let head = ((self.level / (0.05 * self.fluid0)).clamp(0.0, 1.0)).sqrt();
        self.inlet = (ambient + over) * head;
        let mu = self.fl.dynamic(self.temp, P_ATM);
        let mu_r = self.fl.dynamic(T_RATED, P_ATM);
        self.visc = (mu / mu_r).max(1e-3);
        if yes(&mut self.clog, slots, dt, false) {
            self.contamination = (self.contamination + dt / 600.0).min(2.0);
        }
    }

    /// Compliance of the manifold this substep: fluid stiffness falls with temperature and entrained air.
    pub fn compliance_now(&self, net: &Net) -> f64 {
        let p = net.p[self.node].max(0.0) + P_ATM;
        let beta = self.fl.bulk(self.temp, p);
        let beta_ref = self.fl.bulk(T_RATED, 0.5 * self.nominal + P_ATM);
        let air = 1.0 + 40.0 * self.air * (P_ATM / p).min(1.0);
        self.compliance * (beta_ref / beta.max(1e7)) * air
    }

    /// Leaks, relief valve and failure as stamps on the manifold for one substep.
    pub fn stamp(&mut self, net: &mut Net) {
        let p = net.p[self.node];
        net.cap[self.node] = self.compliance_now(net);
        // Laminar internal leakage thins with heat.
        let g_int = self.leak_g / self.visc;
        let g_ext = if self.failed { self.leak_g * 1e4 + self.ext_g } else if self.ext_on { self.ext_g } else { 0.0 };
        // Relief valve: a poppet opening over 10 % above cracking, rated to pass every pump's full flow.
        let span = RELIEF_SPAN * self.relief;
        let g_rel = if p > self.relief { self.rated_flow / span * ((p - self.relief) / span).min(1.0) } else { 0.0 };
        net.stamp(self.node, g_int + g_ext + g_rel, g_rel * self.relief);
        self.leak_q = g_int * p.max(0.0);
        self.relief_q = (g_rel * (p - self.relief)).max(0.0);
        self.return_q = 0.0;
    }

    /// Flow summed over the substeps that came back through the return filter from users and case drains.
    pub fn add_return(&mut self, q: f64) {
        self.return_q += q;
    }

    /// After a substep: fluid lost overboard, leaks and relief flow back to the reservoir.
    pub fn after(&mut self, net: &Net, h: f64) {
        let p = net.p[self.node].max(0.0);
        let g_ext = if self.failed { self.leak_g * 1e4 + self.ext_g } else if self.ext_on { self.ext_g } else { 0.0 };
        let lost = g_ext * p * h;
        self.fluid = (self.fluid - lost).max(0.0);
        self.overboard += lost;
        self.return_q += self.leak_q + self.relief_q;
        self.heat += (self.leak_q + self.relief_q) * p;
    }

    /// Fluid level, temperature, filters, transmitters and outputs, once per domain step.
    #[allow(clippy::too_many_arguments)]
    pub fn finish(&mut self, net: &Net, slots: &mut [f64], dt: f64, substeps: f64, held: f64, cavitating: f64, pump_wear: f64) {
        let p = net.p[self.node].max(0.0);
        let rq = self.return_q / substeps.max(1.0);
        // Temperature: heat in, heat exchanger and line losses out.
        let lines = LINE_LOSS_PER_LITRE * self.fluid0 * 1000.0;
        let k = (self.exchanger + lines) / self.heat_capacity;
        self.temp = (self.temp + dt * (self.heat / substeps.max(1.0) / self.heat_capacity + (self.exchanger * self.cool_t + lines * self.amb) / self.heat_capacity)) / (1.0 + dt * k);
        self.heat_last = self.heat / substeps.max(1.0);
        self.heat = 0.0;
        // Entrained air comes in when the pumps cavitate or the level is low, and settles out in the reservoir.
        let starving = (1.0 - self.level / (0.1 * self.fluid0)).clamp(0.0, 1.0);
        let source = 0.002 * cavitating + 0.001 * starving;
        self.air = ((self.air + source * dt) / (1.0 + dt / DEAERATE_S)).min(0.1);
        // Contamination: pump wear makes it, the return filter takes it out until it bypasses.
        let make = 1e-6 * pump_wear + 2e-8 * cavitating;
        let clean = if self.bypass { 0.0 } else { 2e-7 * rq / self.rated_flow.max(1e-6) };
        self.contamination = (self.contamination + (make - clean.min(self.contamination * 0.01)) * dt).clamp(0.0, 2.0);
        let load = 1.0 + 9.0 * self.contamination * self.contamination;
        self.filter_dp = self.filter_dp0 * (rq / self.rated_flow.max(1e-6)).min(2.0) * self.visc * load;
        let bypass_at = 100.0 * 6894.757;
        self.bypass = self.filter_dp > bypass_at;
        let dp_return = self.filter_dp.min(bypass_at);
        self.return_pressure = self.inlet + dp_return;
        // The fluid swells with heat; what the lines, accumulators and users hold comes out of the reservoir.
        let swell = self.fl.density(288.15, P_ATM) / self.fl.density(self.temp, P_ATM);
        let in_lines = self.compliance * p;
        self.level = (self.fluid * swell - in_lines - held).max(0.0);
        let a = (dt / TRANSMITTER_TAU).min(1.0);
        self.transmitter += (p - self.transmitter) * a;
        self.switch_low = if self.switch_low { self.transmitter < self.low * 1.05 } else { self.transmitter < self.low * 0.95 };
        let b = self.base;
        put(slots, b + co::pressure, p);
        put(slots, b + co::fluid, self.level);
        put(slots, b + co::temperature, self.temp);
        flag(slots, b + co::available, p >= self.low);
        flag(slots, b + co::low, p < self.low);
        flag(slots, b + co::low_level, self.level < self.low_level);
        flag(slots, b + co::overheat, self.temp > self.overheat);
        put(slots, b + co::return_pressure, self.return_pressure);
        put(slots, b + co::reservoir_pressure, self.inlet);
        put(slots, b + co::quantity, self.level / self.fluid0);
        put(slots, b + co::viscosity, self.visc);
        put(slots, b + co::air, self.air);
        put(slots, b + co::contamination, self.contamination);
        put(slots, b + co::filter_dp, self.filter_dp);
        flag(slots, b + co::filter_bypass, self.bypass);
        flag(slots, b + co::filter_clogged, self.filter_dp > 0.7 * bypass_at);
        put(slots, b + co::relief_flow, self.relief_q);
        put(slots, b + co::leak_flow, self.leak_q);
        put(slots, b + co::transmitter, self.transmitter);
        flag(slots, b + co::low_pressure_switch, self.switch_low);
        put(slots, b + co::heat, self.heat_last);
        put(slots, b + co::overboard, self.overboard);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.fluid, self.level, self.temp, self.air, self.contamination, self.overboard, self.transmitter, self.switch_low as u8 as f64, self.bypass as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.fluid = n();
        self.level = n();
        self.temp = n();
        self.air = n();
        self.contamination = n();
        self.overboard = n();
        self.transmitter = n();
        self.switch_low = n() != 0.0;
        self.bypass = n() != 0.0;
    }
}
