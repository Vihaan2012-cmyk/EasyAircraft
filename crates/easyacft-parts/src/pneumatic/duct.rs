//! Bleed ducts: air mass and energy in a fixed volume, so pressure and temperature follow what flows in and out
//! and what the duct wall takes; the wall loses heat through its insulation to the bay; a rupture is a choked
//! jet to the bay; overpressure bursts it; the detection loop beside it heats up in the jet and alarms once it
//! has been over its set point for its confirmation time; transducers lag.

use super::{eval, flag, put, yes, B, MF, P, T, WHEN};
use crate::cabin::{orifice, R_AIR};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "pneumatic",
    kind: "duct",
    help: "A bleed duct or manifold: pressure and temperature from the air mass and energy in it, a wall that stores heat and loses it through its insulation, a rupture that jets to the bay, a burst on overpressure, and a leak detection loop that alarms when the jet heats it past its set point.",
    fields: &[
        opt("volume", Kind::Quantity(Dim::VOLUME), "0.05 m3", "air volume it holds"),
        opt("ambient", Kind::Expr(T), "288.15 K", "temperature of the bay around it"),
        opt("ambient_pressure", Kind::Expr(P), "101325 Pa", "pressure around it, for a leak"),
        opt("heat_loss", Kind::Number, "20", "heat lost through its insulation, W per K"),
        opt("leak_when", WHEN, "false", "a duct rupture or leak"),
        opt("leak_area", Kind::Number, "0.002", "effective area of that leak, m2"),
        opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "0 K", "overheat warning; 0 K for none"),
        opt("burst_pressure", Kind::Quantity(Dim::PRESSURE), "0 Pa", "pressure above ambient that bursts it; 0 for never"),
        opt("wall_mass", Kind::Quantity(Dim::MASS), "4 kg", "duct wall, for the heat it stores"),
        opt("loop_temperature", Kind::Quantity(Dim::TEMPERATURE), "397.15 K", "leak detection loop alarm temperature"),
        opt("loop_time", Kind::Quantity(Dim::TIME), "1 s", "how long the loop must be over its set point to alarm"),
        opt("bay_ventilation", Kind::Number, "200", "how well the bay sheds heat, W per K"),
    ],
    outputs: OUT,
};

outputs!(OUT, d { pressure: P, temperature: T, leak_flow: MF, leak_detected: B, overheat: B, failed: B, wall_temperature: T, loop_temperature: T, burst: B, pressure_sensor: P, temperature_sensor: T, mass: super::N });

const CV: f64 = 718.0;
const CP: f64 = 1005.0;
const STEEL_C: f64 = 500.0;
/// Air to wall heat transfer per kg/s of flow through, W/K, plus natural convection.
const WALL_H: f64 = 40.0;
const SENSOR_TAU: f64 = 0.5;
const LOOP_TAU: f64 = 1.0;
/// Leak jet flow at which the loop sits half way to the jet temperature.
const LOOP_JET: f64 = 0.05;

pub struct Duct {
    base: usize,
    pub node: usize,
    volume: f64,
    pub temp: f64,
    ambient: Option<Expr>,
    ambient_p: Option<Expr>,
    pub t_amb: f64,
    pub p_amb: f64,
    heat_loss: f64,
    leak_when: Option<Expr>,
    leak_cda: f64,
    leak_on: bool,
    pub leak_g: f64,
    overheat: f64,
    burst_at: f64,
    burst: bool,
    wall_c: f64,
    wall_t: f64,
    loop_set: f64,
    loop_time: f64,
    vent: f64,
    loop_t: f64,
    loop_timer: f64,
    sensed_p: f64,
    sensed_t: f64,
    pub failed: bool,
    sum_leak: f64,
    n_sub: f64,
    pub through: f64,
}

impl Duct {
    pub fn new(p: &CheckedPart, node: usize) -> Self {
        Duct {
            base: p.slot,
            node,
            volume: p.num("volume").max(1e-6),
            temp: 288.15,
            ambient: expr_of(p, "ambient"),
            ambient_p: expr_of(p, "ambient_pressure"),
            t_amb: 288.15,
            p_amb: 101_325.0,
            heat_loss: p.num("heat_loss").max(0.0),
            leak_when: expr_of(p, "leak_when"),
            leak_cda: 0.8 * p.num("leak_area").max(0.0),
            leak_on: false,
            leak_g: 0.0,
            overheat: p.num("overheat_temperature"),
            burst_at: p.num("burst_pressure").max(0.0),
            burst: false,
            wall_c: p.num("wall_mass").max(0.01) * STEEL_C,
            wall_t: f64::NAN,
            loop_set: p.num("loop_temperature"),
            loop_time: p.num("loop_time").max(0.0),
            vent: p.num("bay_ventilation").max(1.0),
            loop_t: f64::NAN,
            loop_timer: 0.0,
            sensed_p: 0.0,
            sensed_t: 288.15,
            failed: false,
            sum_leak: 0.0,
            n_sub: 0.0,
            through: 0.0,
        }
    }

    /// Air the volume holds per pascal at its temperature, kg/Pa.
    pub fn compliance(&self) -> f64 {
        self.volume / (R_AIR * self.temp.max(150.0))
    }

    pub fn mass(&self, p: f64) -> f64 {
        p.max(0.0) * self.compliance()
    }

    pub fn begin(&mut self, slots: &[f64], dt: f64, p: f64) {
        self.failed = slots[self.base + d::failed] != 0.0;
        self.t_amb = eval(&mut self.ambient, slots, dt, 288.15);
        self.p_amb = eval(&mut self.ambient_p, slots, dt, 101_325.0).max(0.0);
        if self.wall_t.is_nan() {
            self.wall_t = self.t_amb;
            self.loop_t = self.t_amb;
            self.temp = self.t_amb;
        }
        if self.burst_at > 0.0 && p - self.p_amb > self.burst_at {
            self.burst = true;
        }
        self.leak_on = self.burst || yes(&mut self.leak_when, slots, dt, false);
        self.sum_leak = 0.0;
        self.n_sub = 0.0;
        self.through = 0.0;
    }

    /// Leak and failure stamps, the jet linearised at this substep's pressures.
    pub fn stamp(&mut self, net: &mut crate::hydraulic::net::Net) {
        let p = net.p[self.node];
        net.cap[self.node] = self.compliance();
        if self.failed {
            net.stamp(self.node, 1e-4, 1e-4 * self.p_amb);
        }
        let area = self.leak_cda * if self.burst { 3.0 } else { 1.0 };
        self.leak_g = if self.leak_on && p > self.p_amb + 1.0 { orifice(area, p, self.temp, self.p_amb) / (p - self.p_amb) } else { 0.0 };
        if self.leak_g > 0.0 {
            net.stamp(self.node, self.leak_g, self.leak_g * self.p_amb);
        }
    }

    pub fn after(&mut self, net: &crate::hydraulic::net::Net) -> f64 {
        let q = self.leak_g * (net.p[self.node] - self.p_amb).max(0.0);
        self.sum_leak += q;
        self.n_sub += 1.0;
        q
    }

    /// Energy balance for one domain step: mass and enthalpy in, flow out at the duct's own temperature, the
    /// wall and the bay.
    pub fn finish(&mut self, net: &crate::hydraulic::net::Net, slots: &mut [f64], dt: f64, m_in: f64, h_in: f64) {
        let p = net.p[self.node].max(0.0);
        let mass = self.mass(p).max(1e-4);
        let leak = self.sum_leak / self.n_sub.max(1.0);
        // Air: what flows in brings its enthalpy, what flows out takes the duct's, the wall trades heat.
        let h_w = WALL_H * (1.0 + 4.0 * m_in) + 2.0;
        self.temp = (mass * CV * self.temp + dt * (CP * h_in + h_w * self.wall_t)) / (mass * CV + dt * (CP * m_in + h_w));
        self.wall_t = (self.wall_c * self.wall_t + dt * (h_w * self.temp + self.heat_loss * self.t_amb)) / (self.wall_c + dt * (h_w + self.heat_loss));
        // The bay settles a little above outside with the heat through the insulation; a jet heats the loop.
        let bay = self.t_amb + self.heat_loss * (self.wall_t - self.t_amb).max(0.0) / self.vent;
        let jet = leak / (leak + LOOP_JET);
        let target = bay + (self.temp - bay).max(0.0) * jet;
        self.loop_t += (target - self.loop_t) * (dt / LOOP_TAU).min(1.0);
        self.loop_timer = if self.loop_t > self.loop_set { self.loop_timer + dt } else { 0.0 };
        let a = (dt / SENSOR_TAU).min(1.0);
        self.sensed_p += (p - self.sensed_p) * a;
        self.sensed_t += (self.temp - self.sensed_t) * a;
        let b = self.base;
        put(slots, b + d::pressure, p);
        put(slots, b + d::temperature, self.temp);
        put(slots, b + d::leak_flow, leak);
        flag(slots, b + d::leak_detected, self.loop_timer >= self.loop_time && self.loop_t > self.loop_set);
        flag(slots, b + d::overheat, self.overheat > 0.0 && self.temp > self.overheat);
        put(slots, b + d::wall_temperature, self.wall_t);
        put(slots, b + d::loop_temperature, self.loop_t);
        flag(slots, b + d::burst, self.burst);
        put(slots, b + d::pressure_sensor, self.sensed_p);
        put(slots, b + d::temperature_sensor, self.sensed_t);
        put(slots, b + d::mass, mass);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.temp, self.wall_t, self.loop_t, self.loop_timer, self.burst as u8 as f64, self.sensed_p, self.sensed_t]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.temp = n();
        self.wall_t = n();
        self.loop_t = n();
        self.loop_timer = n();
        self.burst = n() != 0.0;
        self.sensed_p = n();
        self.sensed_t = n();
    }
}
