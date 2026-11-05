//! Flaps or slats: a power control unit of two hydraulic motors (or one and an electric motor) through a
//! differential and torque shafts to the panels' rotary or ball-screw actuators, which have no-backs. Panel aero
//! load grows with deflection and dynamic pressure; a torque limiter locks the shaft on a jam; wing-tip brakes
//! hold it when commanded and stay on until reset on the ground; a shaft break leaves the outboard panels where
//! they were while the other side keeps moving, which the position pick-offs see as an asymmetry.

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

pub static SPEC: PartSpec = PartSpec {
    domain: "flight_control",
    kind: "high_lift",
    help: "Flaps or slats: power control unit, differential, torque shafts, no-back panel actuators, panel aero load, torque limiter, wing-tip brakes, shaft break and asymmetry, position pick-offs.",
    fields: &[
        req("motors", Kind::Refs(&["actuator.motor"]), "one or two hydraulic motors of the power control unit"),
        opt("command", Kind::Expr(N), "0", "wanted position, 0 retracted to 1 fully extended"),
        opt("air", Kind::Ref(&["environment.air"]), "", "for dynamic pressure"),
        opt("travel_time", Kind::Quantity(Dim::TIME), "40 s", "full travel with both motors, no load"),
        opt("travel", Kind::Quantity(Dim::NONE), "40 deg", "panel deflection at full extension"),
        opt("electric_motor", Kind::Bool, "false", "an electric motor on the second input, as A380 slats"),
        opt("electric_when", WHEN, "false", "the electric motor drives"),
        opt("area", Kind::Number, "40", "panel area per wing, m2"),
        opt("cn", Kind::Number, "1.1", "panel normal force coefficient at full extension"),
        opt("arm", Kind::Quantity(Dim::LENGTH), "0.3 m", "panel load's moment arm about its hinge"),
        opt("torque_limit", Kind::Number, "2", "torque limiter trip, share of the motors' combined stall torque"),
        opt("deadband", Kind::Number, "0.002", "position error the valves close inside"),
        opt("wtb_when", WHEN, "false", "wing-tip brakes on: the computers saw an asymmetry, runaway or overspeed"),
        opt("wtb_reset_when", WHEN, "false", "maintenance reset of the wing-tip brakes"),
        opt("shaft_break_left_when", WHEN, "false", "left torque shaft broken outboard of the PCU"),
        opt("shaft_break_right_when", WHEN, "false", "right torque shaft broken"),
        opt("jam_when", WHEN, "false", "a panel actuator seized"),
    ],
    outputs: &[
        ("position", N),
        ("angle", N),
        ("position_left", N),
        ("position_right", N),
        ("asymmetry", N),
        ("rate", N),
        ("command", N),
        ("in_transit", B),
        ("torque", Type::Num(Dim::TORQUE)),
        ("wtb_on", B),
        ("torque_limited", B),
        ("hydraulic_motors", N),
        ("electric_power", Type::Num(Dim::POWER)),
        ("failed", B),
    ],
};

pub struct HighLift {
    pub drive: Drive,
    pub motors: Vec<usize>,
    electric: Option<Electric>,
    q: Option<usize>,
    e: [Option<Expr>; 7],
    travel: f64,
    area: f64,
    cn: f64,
    arm: f64,
    limit: f64,
    deadband: f64,
    wtb: bool,
    tripped: bool,
    broken: [Option<f64>; 2],
    power: f64,
    s: Slots,
}

impl HighLift {
    pub fn new(c: &Checked, p: &CheckedPart, motors: &[motor::Part], index: &HashMap<usize, usize>) -> Self {
        let mi: Vec<usize> = p.parts("motors").iter().filter_map(|r| index.get(r).copied()).take(2).collect();
        let w = mi.first().map_or(400.0, |&k| motors[k].motor.w_rated);
        let rate = 1.0 / p.num("travel_time").max(1.0);
        let two = mi.len() >= 2 || p.flag("electric_motor");
        let ratio = if two { rate / w } else { 2.0 * rate / w };
        let stall: f64 = mi.iter().map(|&k| motors[k].motor.stall_torque()).sum::<f64>().max(1.0);
        let electric = p.flag("electric_motor").then(|| Electric { stall: 0.6 * stall / mi.len().max(1) as f64, w_free: 1.3 * w, inertia: 3e-4, eff: 0.85 });
        let j = mi.first().map_or(1e-3, |&k| motors[k].motor.inertia) + 5e-4;
        let area = p.num("area").max(0.1);
        let travel = p.num("travel").max(1e-3);
        // Output inertia: the panels and the shafting seen at the position output.
        let out_inertia = 2.0 * 30.0 * area * 0.25 * travel * travel + 1.0;
        HighLift {
            drive: Drive::new(ratio, out_inertia, 0.0, 1.0, 0.0, stall / ratio, j, j),
            motors: mi,
            electric,
            q: p.part("air").and_then(|i| c.slots.index.get(&format!("{}.dynamic_pressure", c.parts[i].id)).copied()),
            e: ["command", "electric_when", "wtb_when", "wtb_reset_when", "shaft_break_left_when", "shaft_break_right_when", "jam_when"].map(|k| expr_of(p, k)),
            travel,
            area,
            cn: p.num("cn"),
            arm: p.num("arm"),
            limit: p.num("torque_limit").max(1.0) * stall,
            deadband: p.num("deadband").max(1e-5),
            wtb: false,
            tripped: false,
            broken: [None, None],
            power: 0.0,
            s: Slots::of(p, &["position", "angle", "position_left", "position_right", "asymmetry", "rate", "command", "in_transit", "torque", "wtb_on", "torque_limited", "hydraulic_motors", "electric_power", "failed"]),
        }
    }

    pub fn step(&mut self, motors: &mut [motor::Part], slots: &mut [f64], dt: f64) {
        let mut b = |k: usize| self.e[k].as_mut().is_some_and(|x| x.yes(slots, dt));
        let electric_on = b(1);
        let wtb_cmd = b(2);
        let reset = b(3);
        let breaks = [b(4), b(5)];
        let jam = b(6) || self.s.on(slots, "failed");
        let cmd = self.e[0].as_mut().map_or(0.0, |x| x.eval(slots, dt)).clamp(0.0, 1.0);
        if wtb_cmd {
            self.wtb = true;
        }
        if reset {
            self.wtb = false;
            self.tripped = false;
        }
        let pos = self.drive.angle();
        for (k, br) in breaks.iter().enumerate() {
            if *br && self.broken[k].is_none() {
                self.broken[k] = Some(pos);
            }
        }
        let err = cmd - pos;
        let valve = if err.abs() < self.deadband || self.wtb { 0.0 } else { (err / 0.02).clamp(-1.0, 1.0) };
        let mut inputs = [Input::Locked, Input::Locked];
        let mut hyd = 0.0;
        for (slot, &k) in self.motors.iter().enumerate() {
            let io = motors[k].prepare(slots, dt, valve);
            if io.ps - io.pr > 0.5 * motors[k].motor.nominal {
                hyd += 1.0;
            }
            inputs[slot] = Input::Hydraulic(k, io);
        }
        let e_cmd = if electric_on && !self.wtb { valve } else { 0.0 };
        if let (Some(e), true) = (self.electric, self.motors.len() < 2) {
            inputs[1] = Input::Electric(e, e_cmd, e_cmd.abs() < 1e-3);
        }
        if self.wtb || self.tripped || jam {
            self.drive.held.get_or_insert(pos);
        } else {
            self.drive.held = None;
        }
        let q = self.q.map_or(0.0, |i| slots[i]).max(0.0);
        let (area, cn, arm, travel) = (self.area, self.cn, self.arm, self.travel);
        let sides = 2.0 - self.broken.iter().filter(|b| b.is_some()).count() as f64;
        let load = move |x: f64, rate: f64| {
            // Panel hinge moment seen at the position output; no-back panel actuators turn an aiding load into friction.
            let hinge = q * area * cn * (x.clamp(0.0, 1.0) * std::f64::consts::FRAC_PI_2).sin() * arm * sides;
            let m = hinge * travel;
            let dir = soft_sign(rate, 1e-5);
            let aiding = smooth_step(-m * dir, 0.0, 0.05 * m.abs() + 1e-3);
            m + 1.1 * m.abs() * dir * aiding + 0.03 * m.abs() * dir
        };
        let [(t1, q1), (t2, q2)] = self.drive.step(motors, inputs, dt, &load);
        let pos = self.drive.angle();
        let shaft = load(pos, self.drive.rate()) * self.drive.ratio * 0.5;
        if shaft.abs() > self.limit {
            self.tripped = true;
        }
        for (slot, &k) in self.motors.iter().enumerate() {
            let Input::Hydraulic(_, io) = inputs[slot] else { continue };
            let (w, tq, qq) = if slot == 0 { (self.drive.x[1], t1, q1) } else { (self.drive.x[2], t2, q2) };
            motors[k].finish(slots, &io, w, tq, qq);
        }
        self.power = self.electric.filter(|_| self.motors.len() < 2).map_or(0.0, |e| e.power(e_cmd, self.drive.x[2]));
        let left = self.broken[0].unwrap_or(pos);
        let right = self.broken[1].unwrap_or(pos);
        let s = &self.s;
        s.put(slots, "position", pos);
        s.put(slots, "angle", pos * self.travel);
        s.put(slots, "position_left", left);
        s.put(slots, "position_right", right);
        s.put(slots, "asymmetry", left - right);
        s.put(slots, "rate", self.drive.rate());
        s.put(slots, "command", cmd);
        s.flag(slots, "in_transit", self.drive.rate().abs() > 1e-4);
        s.put(slots, "torque", shaft);
        s.flag(slots, "wtb_on", self.wtb);
        s.flag(slots, "torque_limited", self.tripped);
        s.put(slots, "hydraulic_motors", hyd);
        s.put(slots, "electric_power", self.power);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        self.drive.state(v);
        v.extend([self.wtb as u8 as f64, self.tripped as u8 as f64, self.broken[0].unwrap_or(f64::NAN), self.broken[1].unwrap_or(f64::NAN)]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.drive.restore(it);
        self.wtb = it.next().unwrap_or(0.0) != 0.0;
        self.tripped = it.next().unwrap_or(0.0) != 0.0;
        for b in &mut self.broken {
            *b = it.next().filter(|x| x.is_finite());
        }
    }
}
