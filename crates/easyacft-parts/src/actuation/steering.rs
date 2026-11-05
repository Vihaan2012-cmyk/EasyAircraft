//! Nose or body wheel steering as a mechanism: push-pull actuators (or a rack) on a steering collar turning the
//! wheels against tyre scrub at a standstill, self-aligning torque from the slip angle once rolling, and the
//! shimmy the actuators damp. Towing bypasses the actuators so the wheels castor, and swinging them past the
//! towing limit shears the torque link pin.

use super::linear::{self, Io};
use super::mechanism::{Linkage, Rotary};
use super::ode::soft_sign;
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
    domain: "steering",
    kind: "unit",
    help: "Nose or body wheel steering: actuators on a steering collar, tyre scrub torque, self-aligning torque, shimmy, towing bypass and the towing shear pin.",
    fields: &[
        req("actuators", Kind::Refs(&["actuator.linear"]), "the steering actuators"),
        opt("command", Kind::Expr(N), "0 deg", "wheel angle the steering computer asks for"),
        opt("air", Kind::Ref(&["environment.air"]), "", "for groundspeed and on ground"),
        opt("max", Kind::Quantity(Dim::NONE), "70 deg", "steering stop each side"),
        opt("tow_limit", Kind::Quantity(Dim::NONE), "95 deg", "towing angle that shears the pin"),
        opt("opposed", Kind::Bool, "true", "push-pull pair: successive actuators push opposite ways"),
        opt("arm", Kind::Quantity(Dim::LENGTH), "0.15 m", "actuator attachment radius on the collar"),
        opt("anchor", Kind::Quantity(Dim::LENGTH), "0.5 m", "steering axis to actuator anchor"),
        opt("load", Kind::Expr(Type::Num(Dim::FORCE)), "0 N", "vertical load on the steered wheels; 0 uses static_load on the ground"),
        opt("static_load", Kind::Quantity(Dim::FORCE), "200 kN", "steered wheels' load at rest"),
        opt("contact", Kind::Quantity(Dim::LENGTH), "0.25 m", "tyre footprint radius about the steering axis"),
        opt("trail", Kind::Quantity(Dim::LENGTH), "0.1 m", "mechanical trail plus pneumatic trail"),
        opt("cornering", Kind::Number, "8", "tyre cornering stiffness, times the load per radian"),
        opt("wheelbase", Kind::Quantity(Dim::LENGTH), "30 m", "steered wheels to the turn centre's line"),
        opt("yaw_rate", Kind::Expr(N), "0", "aircraft yaw rate, rad/s"),
        opt("inertia", Kind::Number, "40", "wheels and collar about the steering axis, kg m2"),
        opt("towing_when", WHEN, "false", "towing: bypass valve open, wheels castor"),
        opt("jam_when", WHEN, "false", "steering seized"),
        opt("centring_when", WHEN, "false", "the strut's centring cams engage, as it extends after lift-off"),
    ],
    outputs: &[("angle", N), ("rate", N), ("command", N), ("error", N), ("castoring", B), ("towing", B), ("overtow", B), ("scrub", Type::Num(Dim::TORQUE)), ("failed", B)],
};

pub struct Steering {
    pub rot: Rotary,
    e: [Option<Expr>; 6],
    air: [Option<usize>; 2],
    static_load: f64,
    contact: f64,
    trail: f64,
    cornering: f64,
    wheelbase: f64,
    tow_limit: f64,
    steer_max: f64,
    sheared: bool,
    scrub: f64,
    s: Slots,
}

impl Steering {
    pub fn new(c: &Checked, p: &CheckedPart, acts: &mut Vec<linear::Part>, act_index: &HashMap<usize, usize>) -> Self {
        let hi = p.num("max").max(1.0 * DEG);
        let tow = p.num("tow_limit").max(hi);
        let opposed = p.flag("opposed");
        let mut links = vec![];
        let mut idx = vec![];
        let mut m_ref = 100.0;
        for (n, r) in p.parts("actuators").into_iter().enumerate() {
            let Some(&k) = act_index.get(&r) else { continue };
            let positive = !opposed || n % 2 == 0;
            let (l, stroke) = Linkage::new(p.num("arm"), p.num("anchor"), -hi, hi, positive);
            if acts[k].act.stroke < 0.98 * stroke || acts[k].act.stroke > 1.5 * stroke {
                acts[k].resize(&c.parts[r], stroke);
            }
            m_ref += acts[k].act.stall * l.r;
            links.push(l);
            idx.push(k);
        }
        // Towing swings it past the steering stops, up to the shear pin's limit.
        let mut rot = Rotary::new(links, idx, p.num("inertia").max(0.5), -tow, tow, 0.0, m_ref);
        rot.init_actuators(acts, 101_325.0 + 50.0 * 6894.757);
        Steering {
            rot,
            e: ["command", "load", "yaw_rate", "towing_when", "jam_when", "centring_when"].map(|k| expr_of(p, k)),
            air: [
                p.part("air").and_then(|i| c.slots.index.get(&format!("{}.groundspeed", c.parts[i].id)).copied()),
                p.part("air").and_then(|i| c.slots.index.get(&format!("{}.on_ground", c.parts[i].id)).copied()),
            ],
            static_load: p.num("static_load"),
            contact: p.num("contact").max(0.01),
            trail: p.num("trail"),
            cornering: p.num("cornering").max(0.1),
            wheelbase: p.num("wheelbase").max(0.5),
            tow_limit: tow,
            steer_max: hi,
            sheared: false,
            scrub: 0.0,
            s: Slots::of(p, &["angle", "rate", "command", "error", "castoring", "towing", "overtow", "scrub", "failed"]),
        }
    }

    pub fn step(&mut self, acts: &mut [linear::Part], slots: &mut [f64], dt: f64) {
        let steer_max = self.steer_max;
        let ev = |e: &mut Option<Expr>, s: &[f64]| e.as_mut().map_or(0.0, |x| x.eval(s, dt));
        let cmd = ev(&mut self.e[0], slots).clamp(-steer_max, steer_max);
        let load_in = ev(&mut self.e[1], slots);
        let yaw = ev(&mut self.e[2], slots);
        let towing = ev(&mut self.e[3], slots) != 0.0;
        let jam = ev(&mut self.e[4], slots) != 0.0 || self.s.on(slots, "failed");
        let cams = ev(&mut self.e[5], slots) != 0.0;
        let gs = self.air[0].map_or(0.0, |i| slots[i]);
        let on_ground = self.air[1].is_none_or(|i| slots[i] != 0.0);
        let load = if self.e[1].is_some() && load_in > 0.0 { load_in } else if on_ground { self.static_load } else { 0.0 };
        let t = self.rot.angle();
        if towing && t.abs() > self.tow_limit - 0.5 * DEG {
            self.sheared = true;
        }
        if jam && self.rot.seized.is_none() {
            self.rot.seized = Some(t);
        } else if !jam {
            self.rot.seized = None;
        }
        let bypass = towing || self.sheared || cams;
        let cam_k = if cams { 4.0 * self.rot.moment_ref } else { 0.0 };
        let mut ios: Vec<Io> = vec![];
        for k in 0..self.rot.acts.len() {
            let share = self.rot.command_for(acts, k, cmd);
            let ai = self.rot.acts[k];
            let mut io = acts[ai].prepare(slots, dt, Some(share));
            if bypass {
                io.mode = 0.0;
            }
            ios.push(io);
        }
        self.rot.update_jams(acts, &ios);
        // Inside the steering stops for the actuators; the collar itself goes round to the towing limit.
        let (contact, trail, corner, wb) = (self.contact, self.trail, self.cornering, self.wheelbase);
        let v = gs.abs();
        let kinematic = (wb * yaw / v.max(0.5)).atan() * gs.signum();
        let sheared = self.sheared;
        let loadf = move |t: f64, w: f64| {
            // Scrub: the footprint twisting on the ground, strongest at a standstill, fading once it rolls.
            let roll = (-v / 1.5).exp();
            let scrub = 0.8 * load * contact * (2.0 / 3.0) * roll * soft_sign(w, 0.02);
            // Self-aligning: cornering force on the slip angle, acting through the trail.
            let slip = t - kinematic;
            let fy = (corner * load * slip).clamp(-0.8 * load, 0.8 * load);
            let align = fy * trail * (1.0 - roll) * gs.signum();
            let free = if sheared { 0.0 } else { 1.0 };
            -(scrub + align) * free - 2.0 * w - cam_k * t - 0.2 * cam_k * w
        };
        self.scrub = 0.8 * load * contact * (2.0 / 3.0) * (-v / 1.5).exp();
        self.rot.step(acts, &ios, dt, &loadf);
        for k in 0..self.rot.acts.len() {
            let ai = self.rot.acts[k];
            let (x, xv) = self.rot.actuator_motion(k);
            let st = self.rot.actuator_state(k).to_vec();
            let out = self.rot.outs[k];
            acts[ai].finish(slots, &ios[k], x, xv, &st, &out, dt);
        }
        let active = self.rot.acts.iter().any(|&ai| acts[ai].mode.share > 0.5 && acts[ai].mode.mode != linear::Mode::Passive);
        let a = self.rot.angle();
        let s = &self.s;
        s.put(slots, "angle", a);
        s.put(slots, "rate", self.rot.rate());
        s.put(slots, "command", cmd);
        s.put(slots, "error", cmd - a);
        s.flag(slots, "castoring", !active || bypass);
        s.flag(slots, "towing", towing);
        s.flag(slots, "overtow", self.sheared);
        s.put(slots, "scrub", self.scrub);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        self.rot.state(v);
        v.push(self.sheared as u8 as f64);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.rot.restore(it);
        self.sheared = it.next().unwrap_or(0.0) != 0.0;
    }
}
