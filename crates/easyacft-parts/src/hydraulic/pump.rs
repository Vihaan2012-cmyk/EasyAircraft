//! Pumps: variable-displacement piston pumps whose pressure compensator strokes the swashplate back as the
//! outlet nears its set point, with the swashplate's own response time, volumetric loss through the case drain
//! that grows with pressure, heat and wear, suction losses that thicken with cold fluid and starve the inlet
//! into cavitation, a depressurising solenoid, a shaft that shears on seizure, and for electric pumps an
//! induction motor that slips under load and heats up.

use super::circuit::Circuit;
use super::net::Net;
use super::{eval, flag, put, yes, B, N, P, Q, T, TQ, W, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "hydraulic",
    kind: "pump",
    help: "An engine-driven, electric, ram-air or hand pump: variable displacement with a pressure compensator and swashplate response, case drain leakage that grows with pressure, heat and wear, suction losses and cavitation, a depressurising solenoid, shaft shear on seizure, and for an electric pump a motor that slips and heats.",
    fields: &[
        req("circuit", Kind::Ref(&["hydraulic.circuit"]), "the circuit it feeds"),
        req("pressure", Kind::Quantity(Dim::PRESSURE), "outlet pressure at no flow: the compensator set point"),
        req("flow", Kind::Expr(Q), "flow it delivers at full stroke and rated speed, for example \"140 L/min\", or an expression"),
        opt("running_when", WHEN, "true", "when it turns"),
        opt("speed", Kind::Expr(N), "1", "drive speed as a share of rated; flow follows it"),
        opt("efficiency", Kind::Number, "0.85", "hydraulic power out per shaft power in, at rated conditions"),
        opt("type", Kind::Text, "engine", "engine, electric, ram_air or hand"),
        opt("compensator_band", Kind::Number, "0.1", "share of set point over which the compensator strokes from full to none"),
        opt("response_time", Kind::Quantity(Dim::TIME), "0.03 s", "swashplate time constant"),
        opt("case_drain", Kind::Number, "0.03", "case drain flow at set point, as a share of full flow, when new and at 40 C"),
        opt("depressurized_when", WHEN, "false", "its depressurising solenoid: outlet held at standby pressure, as for engine start or EDP OFF"),
        opt("standby", Kind::Quantity(Dim::PRESSURE), "300 psi", "outlet pressure while depressurised"),
        opt("worn_when", WHEN, "false", "worn pistons and port plate: more case drain, less flow, more contamination"),
        opt("seized_when", WHEN, "false", "pump seized: its drive shaft shears"),
        opt("rated_rpm", Kind::Quantity(Dim::FREQUENCY), "3750 rpm", "shaft speed at rated drive speed"),
        opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "393.15 K", "case drain temperature that sets its overheat warning"),
        opt("motor_efficiency", Kind::Number, "0.88", "electric pump: motor efficiency"),
        opt("powered_when", WHEN, "true", "electric pump: its motor has power"),
        opt("check_open_when", WHEN, "false", "its discharge check valve is stuck open: the circuit flows back through it when it stops"),
        opt("check_shut_when", WHEN, "false", "its discharge check valve is stuck shut: it cannot deliver"),
    ],
    outputs: OUT,
};

outputs!(OUT, po {
    running: B, flow: Q, pressure: P, power: W, cavitating: B, failed: B,
    displacement: N, case_drain_flow: Q, case_temperature: T, overheat: B, torque: TQ, shaft_power: W, electric_power: W,
    depressurized: B, inlet_pressure: P, sheared: B, low_pressure: B, speed: N
});

#[derive(Clone, Copy, PartialEq)]
enum PumpType {
    Engine,
    Electric,
    RamAir,
    Hand,
}

/// Outlet volume of the pump and its discharge line, m3, and the discharge check valve's drop at full flow.
const OUTLET_VOLUME: f64 = 0.4e-3;
const CHECK_DP: f64 = 30.0 * 6894.757;
const SUCTION_DP: f64 = 2.0 * 6894.757;

pub struct Pump {
    base: usize,
    pub circuit: usize,
    pub outlet: usize,
    pub link: usize,
    kind: PumpType,
    flow_e: Option<Expr>,
    speed_e: Option<Expr>,
    when: Option<Expr>,
    depress_e: Option<Expr>,
    worn_e: Option<Expr>,
    seized_e: Option<Expr>,
    powered_e: Option<Expr>,
    check_open_e: Option<Expr>,
    check_shut_e: Option<Expr>,
    check_open: bool,
    check_shut: bool,
    set: f64,
    eff: f64,
    band: f64,
    tau: f64,
    drain_share: f64,
    standby: f64,
    w_rated: f64,
    overheat_at: f64,
    motor_eff: f64,
    pub full_flow: f64,
    speed: f64,
    pub running: bool,
    depress: bool,
    worn: bool,
    sheared: bool,
    stroke: f64,
    /// Flow out of the pump body this substep, before the discharge check.
    q: f64,
    q_drain: f64,
    factor: f64,
    case_t: f64,
    motor_t: f64,
    sum_q: f64,
    sum_drain: f64,
    sum_p: f64,
    sum_cav: f64,
    n_sub: f64,
    pub wear: f64,
}

impl Pump {
    pub fn new(p: &CheckedPart, circuit: usize, c: &Circuit, net: &mut Net) -> Self {
        let outlet = net.node(OUTLET_VOLUME / 1.5e9);
        let link = net.link(outlet, c.node, 1.0, 2e7, true);
        Pump {
            base: p.slot,
            circuit,
            outlet,
            link,
            kind: match p.text("type") {
                "electric" => PumpType::Electric,
                "ram_air" | "rat" => PumpType::RamAir,
                "hand" => PumpType::Hand,
                _ => PumpType::Engine,
            },
            flow_e: expr_of(p, "flow"),
            speed_e: expr_of(p, "speed"),
            when: expr_of(p, "running_when"),
            depress_e: expr_of(p, "depressurized_when"),
            worn_e: expr_of(p, "worn_when"),
            seized_e: expr_of(p, "seized_when"),
            powered_e: expr_of(p, "powered_when"),
            check_open_e: expr_of(p, "check_open_when"),
            check_shut_e: expr_of(p, "check_shut_when"),
            check_open: false,
            check_shut: false,
            set: p.num("pressure").max(1.0),
            eff: p.num("efficiency").clamp(0.05, 1.0),
            band: p.num("compensator_band").clamp(0.01, 0.5),
            tau: p.num("response_time").max(1e-3),
            drain_share: p.num("case_drain").clamp(0.0, 0.5),
            standby: p.num("standby").max(0.0),
            w_rated: (p.num("rated_rpm") * std::f64::consts::TAU).max(1.0),
            overheat_at: p.num("overheat_temperature"),
            motor_eff: p.num("motor_efficiency").clamp(0.3, 1.0),
            full_flow: 0.0,
            speed: 0.0,
            running: false,
            depress: false,
            worn: false,
            sheared: false,
            stroke: 1.0,
            q: 0.0,
            q_drain: 0.0,
            factor: 1.0,
            case_t: f64::NAN,
            motor_t: f64::NAN,
            sum_q: 0.0,
            sum_drain: 0.0,
            sum_p: 0.0,
            sum_cav: 0.0,
            n_sub: 0.0,
            wear: 0.0,
        }
    }

    pub fn begin(&mut self, slots: &mut [f64], dt: f64, c: &Circuit) {
        let failed = slots[self.base + po::failed] != 0.0;
        self.full_flow = eval(&mut self.flow_e, slots, dt, 0.0).max(0.0);
        let mut speed = eval(&mut self.speed_e, slots, dt, 1.0).max(0.0);
        if yes(&mut self.seized_e, slots, dt, false) {
            self.sheared = true;
        }
        self.worn = yes(&mut self.worn_e, slots, dt, false);
        self.depress = yes(&mut self.depress_e, slots, dt, false);
        self.check_open = yes(&mut self.check_open_e, slots, dt, false);
        self.check_shut = yes(&mut self.check_shut_e, slots, dt, false);
        let powered = self.kind != PumpType::Electric || yes(&mut self.powered_e, slots, dt, true);
        if self.kind == PumpType::Electric {
            // An induction motor slips a few per cent at full torque.
            let torque_share = (self.sum_p.max(0.0) / self.set).min(1.5) * self.stroke;
            speed *= 1.0 - 0.04 * torque_share;
        }
        self.speed = speed;
        self.running = !failed && !self.sheared && powered && c.level > 0.0 && yes(&mut self.when, slots, dt, true);
        if self.case_t.is_nan() {
            self.case_t = c.temp;
            self.motor_t = c.temp;
        }
        self.sum_q = 0.0;
        self.sum_drain = 0.0;
        self.sum_cav = 0.0;
        self.n_sub = 0.0;
        if self.worn {
            self.wear = (self.wear + dt / 3600.0).min(1.0);
        }
    }

    /// One substep: compensator, suction and the flow it pushes into its outlet.
    pub fn stamp(&mut self, net: &mut Net, c: &Circuit, h: f64) {
        let p_out = net.p[self.outlet].max(0.0);
        let set = if self.depress { self.standby } else { self.set };
        let band = self.band * self.set;
        let target = if self.running { ((set - p_out) / band).clamp(0.0, 1.0) } else { 0.0 };
        self.stroke += (target - self.stroke) * (h / self.tau).min(1.0);
        // Suction: inlet line losses grow with flow and viscosity, and the pump needs more inlet pressure the
        // harder it is stroked.
        let share = self.stroke * self.speed.min(1.5);
        let suction = SUCTION_DP * share * share * c.visc.sqrt();
        let p_in = (c.inlet - suction).max(0.0);
        let need = c.cavitation * share * share;
        let starve = (c.level / (0.02 * c.fluid.max(1e-6))).clamp(0.0, 1.0);
        self.factor = if need > 1.0 { (p_in / need).clamp(0.0, 1.0) } else { 1.0 } * starve;
        let wear_loss = 1.0 - 0.15 * self.wear;
        let geometric = if self.running { self.full_flow * self.speed * self.stroke * self.factor * wear_loss } else { 0.0 };
        // Case drain: laminar leakage past the pistons, thinner fluid and worn parts leak more.
        let drain_g = if self.running || p_out > 0.0 { self.drain_share * self.full_flow.max(1e-6) / self.set / c.visc * (1.0 + 3.0 * self.wear) } else { 0.0 };
        self.q = geometric;
        self.q_drain = drain_g * p_out;
        net.stamp(self.outlet, drain_g, geometric);
        // The discharge check valve's drop, linearised at its last flow; stuck, it no longer checks.
        let link = &mut net.links[self.link];
        if self.check_shut {
            link.check = false;
            link.open = false;
        } else if !link.check || self.check_open {
            link.open = link.open || !link.check || self.check_open;
            link.check = !self.check_open;
        }
        let qn = self.full_flow.max(1e-5);
        link.r = CHECK_DP / qn * (0.2 + 0.8 * (link.q.abs() / qn).min(2.0));
        self.sum_q += link.q.max(0.0);
        self.sum_drain += self.q_drain;
        self.sum_p += 0.0;
        self.sum_cav += (1.0 - self.factor).max(0.0);
        self.n_sub += 1.0;
    }

    /// Outputs and heat at the end of a domain step; returns the case drain flow and heat into the circuit.
    pub fn finish(&mut self, net: &Net, slots: &mut [f64], dt: f64, c: &Circuit) -> (f64, f64) {
        let n = self.n_sub.max(1.0);
        let q = self.sum_q / n;
        let drain = self.sum_drain / n;
        let p_out = net.p[self.outlet].max(0.0);
        self.sum_p = p_out;
        let hydraulic = q * p_out;
        // Mechanical and leakage losses: what the shaft puts in beyond the hydraulic power, worse when worn.
        let base_loss = if self.running { (1.0 / self.eff - 1.0) * self.full_flow * self.speed * self.set * (0.25 + 0.75 * self.stroke) } else { 0.0 };
        let shaft = if self.running { hydraulic + drain * p_out + base_loss * (1.0 + self.wear) } else { 0.0 };
        let loss = (shaft - hydraulic).max(0.0);
        // Case drain carries the losses away; little drain flow and the case runs hot.
        let rho_c = 870.0 * 1900.0;
        let target = c.temp + loss / (rho_c * drain.max(1e-6) + 15.0);
        self.case_t += (target - self.case_t) * (dt / 20.0).min(1.0);
        let omega = self.w_rated * self.speed.max(0.02);
        let electric = if self.kind == PumpType::Electric && self.running { shaft / self.motor_eff } else { 0.0 };
        if self.kind == PumpType::Electric {
            let rise = 80.0 * (electric * (1.0 - self.motor_eff)) / (self.full_flow.max(1e-6) * self.set * 0.15).max(1.0);
            self.motor_t += (c.temp + rise - self.motor_t) * (dt / 300.0).min(1.0);
        }
        let hot = self.case_t > self.overheat_at || (self.kind == PumpType::Electric && self.motor_t > self.overheat_at + 40.0);
        let cav = self.running && self.full_flow > 0.0 && self.sum_cav / n > 0.05;
        let b = self.base;
        flag(slots, b + po::running, self.running);
        put(slots, b + po::flow, q);
        put(slots, b + po::pressure, if self.running { p_out } else { 0.0 });
        put(slots, b + po::power, shaft);
        flag(slots, b + po::cavitating, cav);
        put(slots, b + po::displacement, self.stroke);
        put(slots, b + po::case_drain_flow, drain);
        put(slots, b + po::case_temperature, self.case_t);
        flag(slots, b + po::overheat, hot);
        put(slots, b + po::torque, shaft / omega);
        put(slots, b + po::shaft_power, shaft);
        put(slots, b + po::electric_power, electric);
        flag(slots, b + po::depressurized, self.depress);
        put(slots, b + po::inlet_pressure, c.inlet);
        flag(slots, b + po::sheared, self.sheared);
        flag(slots, b + po::low_pressure, self.running && p_out < 0.6 * self.set);
        put(slots, b + po::speed, self.speed);
        (drain, loss)
    }

    pub fn cavitation(&self) -> f64 {
        if self.running {
            self.sum_cav / self.n_sub.max(1.0)
        } else {
            0.0
        }
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.stroke, self.case_t, self.motor_t, self.wear, self.sheared as u8 as f64, self.sum_p]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.stroke = n();
        self.case_t = n();
        self.motor_t = n();
        self.wear = n();
        self.sheared = n() != 0.0;
        self.sum_p = n();
    }
}
