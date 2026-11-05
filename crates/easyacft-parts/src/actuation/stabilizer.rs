//! The trimmable horizontal stabiliser actuator: a ball screw turned through a differential by two hydraulic motors,
//! or a hydraulic and an electric motor, each with its pressure-off brake; a no-back that stops aerodynamic load
//! back-driving the screw; a torque limiter that locks it under an overload; aero load from the tailplane's pitching
//! moment.

use super::drive::{Drive, Electric, Input};
use super::motor;
use super::ode::{smooth_step, soft_sign};
use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};
use std::collections::HashMap;

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const WHEN: Kind = Kind::Expr(Type::Bool);
const DEG: f64 = std::f64::consts::PI / 180.0;

pub static SPEC: PartSpec = PartSpec {
    domain: "flight_control",
    kind: "stabilizer",
    help: "Trimmable horizontal stabiliser: ball screw, differential, hydraulic motors with pressure-off brakes, optional electric motor, no-back, torque limiter, tailplane aero load.",
    fields: &[
        req("motors", Kind::Refs(&["actuator.motor"]), "one or two hydraulic motors"),
        opt("command", Kind::Expr(N), "0 deg", "trim angle the computers ask for"),
        opt("air", Kind::Ref(&["environment.air"]), "", "for dynamic pressure"),
        opt("min", Kind::Quantity(Dim::NONE), "-13 deg", "nose-down stop, trailing edge up"),
        opt("max", Kind::Quantity(Dim::NONE), "3 deg", "nose-up stop"),
        opt("initial", Kind::Quantity(Dim::NONE), "0 deg", "angle at start"),
        opt("rate", Kind::Quantity(Dim::NONE), "0.5 deg", "trim rate per second with both hydraulic motors"),
        opt("electric_motor", Kind::Bool, "false", "an electric motor on the second input"),
        opt("electric_when", WHEN, "false", "the electric motor drives"),
        opt("electric_rate", Kind::Quantity(Dim::NONE), "0.2 deg", "trim rate per second on the electric motor alone"),
        opt("area", Kind::Number, "200", "tailplane area, m2"),
        opt("chord", Kind::Quantity(Dim::LENGTH), "6 m", "tailplane mean chord"),
        opt("cm_alpha", Kind::Number, "0.08", "pitching moment about the pivot per radian of tail angle of attack, share of q S c"),
        opt("tail_alpha", Kind::Expr(N), "0 deg", "tail angle of attack before trim"),
        opt("torque_limit", Kind::Number, "2.5", "torque limiter trip, share of the motors' combined stall torque at the screw"),
        opt("deadband", Kind::Quantity(Dim::NONE), "0.05 deg", "error below which the motor valves close"),
        opt("jam_when", WHEN, "false", "screw seized"),
        opt("no_back_failed_when", WHEN, "false", "no-back gone: aero load can back-drive the screw"),
        opt("manual", Kind::Expr(N), "0", "pilots' trim wheels, -1 to 1: cables to the motors' control valves, overriding the electrical command"),
        opt("wheel_ratio", Kind::Number, "6", "cockpit trim wheel turns per degree of stabiliser"),
    ],
    outputs: &[
        ("angle", N),
        ("rate", N),
        ("command", N),
        ("load", Type::Num(Dim::TORQUE)),
        ("jammed", B),
        ("torque_limited", B),
        ("hydraulic_motors", N),
        ("electric_power", Type::Num(Dim::POWER)),
        ("motor_temperature", Type::Num(Dim::TEMPERATURE)),
        ("runaway", B),
        ("sensor_1", N),
        ("sensor_2", N),
        ("wheel_turns", N),
        ("failed", B),
    ],
};

pub struct Stabilizer {
    pub drive: Drive,
    pub motors: Vec<usize>,
    electric: Option<Electric>,
    q: Option<usize>,
    e: [Option<Expr>; 6],
    area: f64,
    chord: f64,
    cm: f64,
    limit: f64,
    deadband: f64,
    tripped: bool,
    power: f64,
    motor_t: f64,
    motor_out: bool,
    rvdt: super::sensors::Dual,
    wheel_ratio: f64,
    s: Slots,
}

impl Stabilizer {
    pub fn new(c: &Checked, p: &CheckedPart, motors: &[motor::Part], index: &HashMap<usize, usize>) -> Self {
        let mi: Vec<usize> = p.parts("motors").iter().filter_map(|r| index.get(r).copied()).take(2).collect();
        let lo = p.num("min");
        let hi = p.num("max").max(lo + DEG);
        let rate = p.num("rate").max(1e-3);
        let w = mi.first().map_or(400.0, |&k| motors[k].motor.w_rated);
        let both = mi.len() >= 2;
        // With two motors summed, full rate needs both at rated speed; with one, the other input is held.
        let ratio = if both || p.flag("electric_motor") { rate / w } else { 2.0 * rate / w };
        let stall: f64 = mi.iter().map(|&k| motors[k].motor.stall_torque()).sum::<f64>().max(1.0);
        let electric = p.flag("electric_motor").then(|| {
            let we = 2.0 * p.num("electric_rate").max(1e-3) / ratio;
            let t = stall / mi.len().max(1) as f64 * 0.6;
            Electric { stall: t, w_free: we * 1.3, inertia: 3e-4, eff: 0.85 }
        });
        let j = mi.first().map_or(1e-3, |&k| motors[k].motor.inertia) + 2e-4;
        let area = p.num("area").max(1.0);
        let chord = p.num("chord").max(0.1);
        Stabilizer {
            drive: Drive::new(ratio, 0.02 * area * area * chord, lo, hi, p.num("initial"), stall / ratio, j, j),
            motors: mi,
            electric,
            q: p.part("air").and_then(|i| c.slots.index.get(&format!("{}.dynamic_pressure", c.parts[i].id)).copied()),
            e: ["command", "electric_when", "tail_alpha", "jam_when", "no_back_failed_when", "manual"].map(|k| expr_of(p, k)),
            area,
            chord,
            cm: p.num("cm_alpha"),
            limit: p.num("torque_limit").max(1.0) * stall / ratio,
            deadband: p.num("deadband").max(1e-5),
            tripped: false,
            power: 0.0,
            motor_t: f64::NAN,
            motor_out: false,
            rvdt: super::sensors::Dual::new(lo, hi, (p.slot as u64) ^ 0x2545_f491_4f6c_dd1d),
            wheel_ratio: p.num("wheel_ratio"),
            s: Slots::of(p, &["angle", "rate", "command", "load", "jammed", "torque_limited", "hydraulic_motors", "electric_power", "motor_temperature", "runaway", "sensor_1", "sensor_2", "wheel_turns", "failed"]),
        }
    }

    pub fn step(&mut self, motors: &mut [motor::Part], slots: &mut [f64], dt: f64) {
        let ev = |e: &mut Option<Expr>, s: &[f64]| e.as_mut().map_or(0.0, |x| x.eval(s, dt));
        let cmd = ev(&mut self.e[0], slots).clamp(self.drive.lo, self.drive.hi);
        let electric_on = ev(&mut self.e[1], slots) != 0.0;
        let alpha = ev(&mut self.e[2], slots);
        let jam = ev(&mut self.e[3], slots) != 0.0 || self.s.on(slots, "failed");
        let no_back_failed = ev(&mut self.e[4], slots) != 0.0;
        let manual = ev(&mut self.e[5], slots).clamp(-1.0, 1.0);
        let t = self.drive.angle();
        let err = cmd - t;
        // Valves open fully beyond a few tenths of a degree of error and close inside the dead band.
        let valve = if manual.abs() > 0.05 {
            manual
        } else if err.abs() < self.deadband {
            0.0
        } else {
            (err / (0.3 * DEG)).clamp(-1.0, 1.0)
        };
        let mut inputs = [Input::Locked, Input::Locked];
        let mut hyd = 0.0;
        for (slot, &k) in self.motors.iter().enumerate() {
            let io = motors[k].prepare(slots, dt, valve);
            if io.ps - io.pr > 0.5 * motors[k].motor.nominal {
                hyd += 1.0;
            }
            inputs[slot] = Input::Hydraulic(k, io);
        }
        let e_cmd = if electric_on && !self.motor_out { valve } else { 0.0 };
        if let Some(e) = self.electric {
            if self.motors.len() < 2 {
                inputs[1] = Input::Electric(e, e_cmd, e_cmd.abs() < 1e-3);
            }
        }
        if jam || self.tripped {
            self.drive.held.get_or_insert(t);
        } else {
            self.drive.held = None;
        }
        let q = self.q.map_or(0.0, |i| slots[i]).max(0.0);
        let qsc = q * self.area * self.chord;
        let cm = self.cm;
        let no_back = !no_back_failed;
        let load = move |t: f64, rate: f64| {
            // Aero moment on the tailplane about its pivot, resisting positive trim when positive.
            let m = qsc * cm * (alpha + t);
            let dir = soft_sign(rate, 1e-4);
            // No-back: a load that would drive the screw meets a brake a little stronger than itself.
            let aiding = smooth_step(-m * dir, 0.0, 0.05 * m.abs() + 1.0);
            let nb = if no_back { 1.15 * m.abs() * dir * aiding } else { 0.0 };
            m + nb + 0.02 * m.abs() * dir
        };
        let [(t1, q1), (t2, q2)] = self.drive.step(motors, inputs, dt, &load);
        let screw = load(self.drive.angle(), self.drive.rate());
        if screw.abs() > self.limit && !self.tripped {
            self.tripped = true;
        }
        for (slot, &k) in self.motors.iter().enumerate() {
            let Input::Hydraulic(_, io) = inputs[slot] else { continue };
            let (w, tq, qq) = if slot == 0 { (self.drive.x[1], t1, q1) } else { (self.drive.x[2], t2, q2) };
            motors[k].finish(slots, &io, w, tq, qq);
        }
        self.power = self.electric.filter(|_| self.motors.len() < 2).map_or(0.0, |e| e.power(e_cmd, self.drive.x[2]));
        self.motor_heat(dt);
        let rate = self.drive.rate();
        let (r1, r2, _, _) = self.rvdt.read(self.drive.angle(), super::sensors::Fault::None, super::sensors::Fault::None, dt);
        let s = &self.s;
        s.put(slots, "sensor_1", r1);
        s.put(slots, "sensor_2", r2);
        // The trim wheels are geared to the screw, so they turn with it whatever drives it.
        s.put(slots, "wheel_turns", self.drive.angle() / DEG * self.wheel_ratio);
        s.put(slots, "angle", self.drive.angle());
        s.put(slots, "rate", rate);
        s.put(slots, "command", cmd);
        s.put(slots, "load", screw);
        s.flag(slots, "jammed", jam || self.tripped);
        s.flag(slots, "torque_limited", self.tripped);
        s.put(slots, "hydraulic_motors", hyd);
        s.put(slots, "electric_power", self.power);
        s.put(slots, "motor_temperature", self.motor_t);
        s.flag(slots, "runaway", rate.abs() > 0.1 * DEG && rate * err < 0.0 && err.abs() > 0.5 * DEG);
    }

    /// Motor windings warm with the losses of its duty and cut out at 160 C until they cool to 120 C.
    fn motor_heat(&mut self, dt: f64) {
        if !self.motor_t.is_finite() {
            self.motor_t = 288.15;
        }
        let loss = self.electric.map_or(0.0, |e| self.power * (1.0 - e.eff));
        let cap = 4000.0;
        self.motor_t += (loss - cap / 900.0 * (self.motor_t - 288.15)) / cap * dt;
        if self.motor_t > 433.15 {
            self.motor_out = true;
        } else if self.motor_t < 393.15 {
            self.motor_out = false;
        }
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        self.drive.state(v);
        v.extend([self.tripped as u8 as f64, self.motor_t, self.motor_out as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.drive.restore(it);
        self.tripped = it.next().unwrap_or(0.0) != 0.0;
        self.motor_t = it.next().unwrap_or(f64::NAN);
        self.motor_out = it.next().unwrap_or(0.0) != 0.0;
    }
}
