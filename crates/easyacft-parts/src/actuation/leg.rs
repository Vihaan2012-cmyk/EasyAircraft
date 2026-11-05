//! Landing gear as mechanisms: a leg swinging about its trunnion under its retraction actuator, gravity (with load
//! factor and attitude), drag that helps or fights extension, and over-centre lock springs; an uplock hook and a
//! downlock brace, each released by its own hydraulic unlock actuator; doors that open first, close last and
//! hang open by gravity in free fall; an oleo-pneumatic shock strut whose gas and metering orifice set its stroke
//! under the wheel load; dual proximity sensors for weight on wheels. Free fall releases the uplocks and floats the
//! actuators to return, so the gear falls at whatever speed gravity, drag and the actuators' fluid allow.

use super::linear::{self, Io, Passive};
use super::mechanism::{Linkage, Rotary};
use super::ode::{Outcome, Stepper};
use super::valve::Centre;
use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};
use std::collections::HashMap;

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const WHEN: Kind = Kind::Expr(Type::Bool);
const AIR: &[&str] = &["environment.air"];
const G: f64 = 9.80665;
const DEG: f64 = std::f64::consts::PI / 180.0;
const PSI: f64 = 6_894.757;

pub static LEG: PartSpec = PartSpec {
    domain: "landing_gear",
    kind: "leg",
    help: "A landing gear leg on its trunnion: retraction actuator through its linkage, gravity, drag, lock springs, uplock and downlock with their unlock actuators, door sequencing, free fall, an oleo-pneumatic strut and dual weight-on-wheels sensors.",
    fields: &[
        req("actuator", Kind::Ref(&["actuator.linear"]), "the retraction actuator, a selector-valve actuator"),
        opt("down_when", WHEN, "true", "gear selected down"),
        opt("free_fall_when", WHEN, "false", "alternate extension: uplocks released, actuators floated to return"),
        opt("neutral_when", WHEN, "false", "selector to neutral: lines to return, as with the gear up and locked in cruise"),
        opt("doors", Kind::Refs(&["landing_gear.door"]), "", "doors this leg sequences: open before it moves, close after it locks"),
        opt("doors_stay_open", Kind::Bool, "false", "doors stay open with the gear down"),
        opt("air", Kind::Ref(AIR), "", "for dynamic pressure, attitude and on ground"),
        opt("travel", Kind::Quantity(Dim::NONE), "95 deg", "swing from up to down"),
        opt("mass", Kind::Quantity(Dim::MASS), "600 kg", "leg, wheels and brakes"),
        opt("cg", Kind::Quantity(Dim::LENGTH), "1.2 m", "trunnion to the leg's centre of gravity"),
        opt("retracts", Kind::Text, "inboard", "forward (drag helps extension), aft (drag fights extension) or inboard/outboard"),
        opt("drag_area", Kind::Number, "1.2", "drag coefficient times frontal area down, m2"),
        opt("arm", Kind::Quantity(Dim::LENGTH), "0.35 m", "actuator attachment radius on the leg"),
        opt("anchor", Kind::Quantity(Dim::LENGTH), "1.4 m", "trunnion to actuator anchor on the structure"),
        opt("actuator_extends", Kind::Text, "down", "down: the actuator extends to lower the gear; up: it extends to raise it"),
        opt("lock_spring", Kind::Number, "0.08", "downlock spring moment near full extension, share of the leg's weight moment"),
        opt("unlock_time", Kind::Quantity(Dim::TIME), "0.6 s", "time a pressurised unlock actuator takes to release a lock"),
        opt("free_fall_release", Kind::Quantity(Dim::TIME), "2 s", "time the alternate release takes to open the uplock"),
        opt("initial_down", Kind::Bool, "true", "down and locked at start"),
        opt("load", Kind::Expr(Type::Num(Dim::FORCE)), "0 N", "vertical wheel load; 0 uses static_load on the ground"),
        opt("static_load", Kind::Quantity(Dim::FORCE), "300 kN", "wheel load at rest on the ground"),
        opt("strut_stroke", Kind::Quantity(Dim::LENGTH), "0.45 m", "full shock strut stroke"),
        opt("static_stroke", Kind::Number, "0.8", "share of stroke compressed at static load"),
        opt("static_pressure", Kind::Quantity(Dim::PRESSURE), "1500 psi", "strut gas pressure at static load"),
        opt("wow_stroke", Kind::Quantity(Dim::LENGTH), "0.03 m", "compression the proximity sensors call weight on wheels"),
        opt("sensor_1_failed_when", WHEN, "false", "proximity sensor 1 reads target far"),
        opt("sensor_2_failed_when", WHEN, "false", "proximity sensor 2 reads target far"),
        opt("uplock_jam_when", WHEN, "false", "uplock hook seized: the leg stays up, even in free fall"),
        opt("downlock_failed_when", WHEN, "false", "downlock brace does not lock"),
        opt("jam_when", WHEN, "false", "leg seized in its bay"),
        opt("strut_leak_when", WHEN, "false", "strut gas leaking: it sinks"),
        opt("bogie", Kind::Ref(&["landing_gear.bogie"]), "", "bogie that must be trailed before the leg retracts"),
        opt("hard_landing_rate", Kind::Quantity(Dim::SPEED), "3 m/s", "strut closing rate at touchdown that counts as a hard landing"),
        opt("blowdown_when", WHEN, "false", "emergency extension from a nitrogen bottle: it vents the up side and blows the jack down"),
        opt("bottle_pressure", Kind::Quantity(Dim::PRESSURE), "3000 psi", "nitrogen bottle charge"),
        opt("bottle_volume", Kind::Quantity(Dim::VOLUME), "1.5 L", "nitrogen bottle volume"),
        opt("crank", Kind::Expr(N), "0", "gravity extension crank turns so far"),
        opt("release_turns", Kind::Number, "0", "turns for the crank to isolate the hydraulics (first third), release the doors (second) and the leg (all); 0 uses free_fall_when alone"),
        opt("limit_speed", Kind::Quantity(Dim::SPEED), "0 kt", "gear extended speed limit (VLE), equivalent airspeed; 0 none"),
    ],
    outputs: &[
        ("position", N),
        ("angle", N),
        ("down_locked", B),
        ("up_locked", B),
        ("downlock_engaged", B),
        ("uplock_engaged", B),
        ("in_transit", B),
        ("disagree", B),
        ("doors_closed", B),
        ("doors_open", B),
        ("strut_stroke", Type::Num(Dim::LENGTH)),
        ("strut_pressure", Type::Num(Dim::PRESSURE)),
        ("compressed", B),
        ("wow_1", B),
        ("wow_2", B),
        ("weight_on_wheels", B),
        ("selector", N),
        ("strut_rate", Type::Num(Dim::SPEED)),
        ("peak_load", Type::Num(Dim::FORCE)),
        ("bottomed", B),
        ("hard_landing", B),
        ("retract_inhibited", B),
        ("bottle_pressure", Type::Num(Dim::PRESSURE)),
        ("overspeed", B),
        ("overstressed", B),
        ("failed", B),
    ],
};

pub static BOGIE: PartSpec = PartSpec {
    domain: "landing_gear",
    kind: "bogie",
    help: "A main gear bogie on its pivot: a trimmer actuator holds it at its trail angle once the wheels leave the ground, the ground levels it on landing, and its leg will not retract until it is trailed.",
    fields: &[
        req("actuator", Kind::Ref(&["actuator.linear"]), "the pitch trimmer actuator"),
        opt("on_ground_when", WHEN, "false", "the wheels are on the ground, for example \"body_gear_l.compressed\""),
        opt("trail", Kind::Quantity(Dim::NONE), "12 deg", "pitch the trimmer holds in the air, front wheels up"),
        opt("range", Kind::Quantity(Dim::NONE), "20 deg", "pivot travel each way"),
        opt("mass", Kind::Quantity(Dim::MASS), "900 kg", "bogie beam, wheels and brakes"),
        opt("offset", Kind::Quantity(Dim::LENGTH), "0.1 m", "centre of gravity aft of the pivot"),
        opt("wheelbase", Kind::Quantity(Dim::LENGTH), "1.7 m", "front to rear axle"),
        opt("arm", Kind::Quantity(Dim::LENGTH), "0.25 m", "trimmer attachment radius"),
        opt("anchor", Kind::Quantity(Dim::LENGTH), "0.9 m", "pivot to trimmer anchor on the leg"),
        opt("ground_load", Kind::Quantity(Dim::FORCE), "400 kN", "load that flattens it on the ground"),
        opt("jam_when", WHEN, "false", "pivot seized"),
    ],
    outputs: &[("angle", N), ("trailed", B), ("level", B), ("failed", B)],
};

/// A bogie beam pitching on its pivot.
pub struct Bogie {
    pub rot: Rotary,
    e: [Option<Expr>; 2],
    trail: f64,
    mass: f64,
    offset: f64,
    wheelbase: f64,
    ground_load: f64,
    s: Slots,
}

impl Bogie {
    pub fn new(c: &Checked, p: &CheckedPart, acts: &mut Vec<linear::Part>, act_index: &HashMap<usize, usize>) -> Self {
        let range = p.num("range").max(1.0 * DEG);
        let mut links = vec![];
        let mut idx = vec![];
        let mass = p.num("mass").max(1.0);
        let wheelbase = p.num("wheelbase").max(0.2);
        let mut m_ref = mass * G * wheelbase;
        if let Some((r, &k)) = p.part("actuator").and_then(|r| act_index.get(&r).map(|k| (r, k))) {
            let (l, stroke) = Linkage::new(p.num("arm"), p.num("anchor"), -range, range, true);
            if acts[k].act.stroke < 0.98 * stroke || acts[k].act.stroke > 1.5 * stroke {
                acts[k].resize(&c.parts[r], stroke);
            }
            m_ref += acts[k].act.stall * l.r;
            links.push(l);
            idx.push(k);
        }
        let inertia = mass * wheelbase * wheelbase / 12.0;
        let mut rot = Rotary::new(links, idx, inertia, -range, range, 0.0, m_ref);
        rot.init_actuators(acts, 101_325.0 + 50.0 * PSI);
        Bogie {
            rot,
            e: ["on_ground_when", "jam_when"].map(|k| expr_of(p, k)),
            trail: p.num("trail"),
            mass,
            offset: p.num("offset"),
            wheelbase,
            ground_load: p.num("ground_load").max(1.0),
            s: Slots::of(p, &["angle", "trailed", "level", "failed"]),
        }
    }

    pub fn trailed(&self) -> bool {
        (self.rot.angle() - self.trail).abs() < 2.0 * DEG
    }

    pub fn step(&mut self, acts: &mut [linear::Part], slots: &mut [f64], dt: f64) {
        let ground = self.e[0].as_mut().is_some_and(|x| x.yes(slots, dt));
        let jam = self.e[1].as_mut().is_some_and(|x| x.yes(slots, dt)) || self.s.on(slots, "failed");
        let t = self.rot.angle();
        if jam && self.rot.seized.is_none() {
            self.rot.seized = Some(t);
        } else if !jam {
            self.rot.seized = None;
        }
        let mut ios: Vec<Io> = vec![];
        for k in 0..self.rot.acts.len() {
            let share = self.rot.command_for(acts, k, self.trail);
            let ai = self.rot.acts[k];
            let mut io = acts[ai].prepare(slots, dt, Some(share));
            // On the ground the trimmer is bypassed so the bogie follows the runway.
            if ground {
                io.mode = 0.0;
            }
            ios.push(io);
        }
        self.rot.update_jams(acts, &ios);
        let (m, off, wb, gl) = (self.mass, self.offset, self.wheelbase, self.ground_load);
        let load = move |t: f64, w: f64| {
            let g = -m * G * off * t.cos();
            // The runway pushes whichever axle is low back up, levelling the beam.
            let flatten = if ground { -gl * 0.5 * wb * t.sin() - 0.2 * gl * wb * wb * w / 10.0 } else { 0.0 };
            g + flatten - 0.05 * m * wb * wb * w
        };
        self.rot.step(acts, &ios, dt, &load);
        for k in 0..self.rot.acts.len() {
            let ai = self.rot.acts[k];
            let (x, v) = self.rot.actuator_motion(k);
            let st = self.rot.actuator_state(k).to_vec();
            let out = self.rot.outs[k];
            acts[ai].finish(slots, &ios[k], x, v, &st, &out, dt);
        }
        let a = self.rot.angle();
        let s = &self.s;
        s.put(slots, "angle", a);
        s.flag(slots, "trailed", self.trailed());
        s.flag(slots, "level", a.abs() < 1.5 * DEG);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        self.rot.state(v);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.rot.restore(it);
    }
}

pub static DOOR: PartSpec = PartSpec {
    domain: "landing_gear",
    kind: "door",
    help: "A gear door on its hinge: actuator, closed-position uplock, gravity and airload; a leg that lists it sequences it.",
    fields: &[
        opt("actuator", Kind::Ref(&["actuator.linear"]), "", "door actuator, a selector-valve actuator; none for a door linked to its leg"),
        opt("linked", Kind::Bool, "false", "mechanically linked to the leg that lists it: opens as the leg leaves its uplock, rides on it down"),
        opt("open_when", WHEN, "false", "open, when no leg sequences it"),
        opt("air", Kind::Ref(AIR), "", "for dynamic pressure"),
        opt("travel", Kind::Quantity(Dim::NONE), "85 deg", "closed to open"),
        opt("mass", Kind::Quantity(Dim::MASS), "60 kg", "door mass"),
        opt("cg", Kind::Quantity(Dim::LENGTH), "0.5 m", "hinge to centre of gravity"),
        opt("hangs", Kind::Text, "open", "open: gravity opens it once unlocked (belly doors); closed: side-hinged, gravity neutral"),
        opt("area", Kind::Number, "1.5", "door area, m2"),
        opt("airload", Kind::Text, "closes", "closes or opens: which way the slipstream pushes it"),
        opt("arm", Kind::Quantity(Dim::LENGTH), "0.12 m", "actuator attachment radius"),
        opt("anchor", Kind::Quantity(Dim::LENGTH), "0.6 m", "hinge to actuator anchor"),
        opt("unlock_time", Kind::Quantity(Dim::TIME), "0.4 s", "uplock release time under pressure"),
        opt("free_fall_release", Kind::Quantity(Dim::TIME), "1.5 s", "alternate release time"),
        opt("free_fall_when", WHEN, "false", "alternate release, when no leg sequences it"),
        opt("jam_when", WHEN, "false", "door seized"),
        opt("limit_speed", Kind::Quantity(Dim::SPEED), "0 kt", "speed limit with the door open, equivalent airspeed; 0 none"),
    ],
    outputs: &[("position", N), ("angle", N), ("closed", B), ("open", B), ("locked", B), ("in_transit", B), ("overstressed", B), ("failed", B)],
};

/// A hook or brace lock: engages by itself at its angle, releases after its unlock actuator has had pressure long
/// enough or the alternate release has run.
#[derive(Clone, Debug, Default)]
pub struct Lock {
    pub engaged: bool,
    pub release: f64,
}

impl Lock {
    /// `pressing` is the share of unlock pressure the unlock actuator has, `alternate` the free-fall release.
    pub fn update(&mut self, at_lock: bool, can_engage: bool, pressing: f64, alternate: bool, unlock_t: f64, alt_t: f64, dt: f64) {
        let rate = if alternate { dt / alt_t.max(0.05) } else { pressing.clamp(0.0, 1.0).sqrt() * dt / unlock_t.max(0.05) };
        if self.engaged {
            if pressing > 0.3 || alternate {
                self.release += rate;
                if self.release >= 1.0 {
                    self.engaged = false;
                    self.release = 0.0;
                }
            } else {
                self.release = (self.release - dt).max(0.0);
            }
        } else if at_lock && can_engage && pressing < 0.3 && !alternate {
            self.engaged = true;
            self.release = 0.0;
        }
    }
}

pub struct Door {
    pub rot: Rotary,
    lock: Lock,
    e: [Option<Expr>; 3],
    q: Option<usize>,
    mass: f64,
    cg: f64,
    hangs_open: bool,
    area: f64,
    air_opens: bool,
    travel: f64,
    unlock_t: f64,
    alt_t: f64,
    /// Command a leg set for this step: Some(open) and whether in free fall.
    pub sequenced: Option<(bool, bool)>,
    pub linked: bool,
    /// Leg position a linked door follows this step.
    pub follow: Option<f64>,
    eas: Option<usize>,
    limit: f64,
    over_t: f64,
    overstressed: bool,
    s: Slots,
}

fn slot(c: &Checked, p: &CheckedPart, field: &str, out: &str) -> Option<usize> {
    p.part(field).and_then(|i| c.slots.index.get(&format!("{}.{out}", c.parts[i].id)).copied())
}

fn floated(acts: &mut [linear::Part], k: usize) {
    let a = &mut acts[k].act;
    if a.control == linear::Control::Selector {
        a.passive = Passive::Float;
        a.selector.centre = Centre::Float;
    }
}

impl Door {
    pub fn new(c: &Checked, p: &CheckedPart, acts: &mut Vec<linear::Part>, act_index: &HashMap<usize, usize>) -> Self {
        let travel = p.num("travel").max(1.0 * DEG);
        let mass = p.num("mass").max(1.0);
        let cg = p.num("cg").max(0.01);
        let mut links = vec![];
        let mut idx = vec![];
        let mut m_ref = mass * G * cg;
        if let Some(&k) = p.part("actuator").and_then(|r| act_index.get(&r)) {
            let (l, stroke) = Linkage::new(p.num("arm"), p.num("anchor"), 0.0, travel, true);
            if acts[k].act.stroke < 0.98 * stroke || acts[k].act.stroke > 1.5 * stroke {
                let r = p.part("actuator").unwrap_or(0);
                acts[k].resize(&c.parts[r], stroke);
            }
            floated(acts, k);
            m_ref += acts[k].act.stall * l.r;
            links.push(l);
            idx.push(k);
        }
        let mut rot = Rotary::new(links, idx, mass * cg * cg * 1.3, 0.0, travel, 0.0, m_ref);
        rot.init_actuators(acts, 101_325.0 + 50.0 * PSI);
        rot.lock = Some(0.0);
        Door {
            rot,
            lock: Lock { engaged: true, release: 0.0 },
            e: ["open_when", "free_fall_when", "jam_when"].map(|k| expr_of(p, k)),
            q: slot(c, p, "air", "dynamic_pressure"),
            mass,
            cg,
            hangs_open: !p.text("hangs").eq_ignore_ascii_case("closed"),
            area: p.num("area").max(0.01),
            air_opens: p.text("airload").eq_ignore_ascii_case("opens"),
            travel,
            unlock_t: p.num("unlock_time"),
            alt_t: p.num("free_fall_release"),
            sequenced: None,
            linked: p.flag("linked"),
            follow: None,
            eas: slot(c, p, "air", "eas"),
            limit: p.num("limit_speed").max(0.0),
            over_t: 0.0,
            overstressed: false,
            s: Slots::of(p, &["position", "angle", "closed", "open", "locked", "in_transit", "overstressed", "failed"]),
        }
    }

    pub fn position(&self) -> f64 {
        self.rot.angle() / self.travel
    }

    pub fn step(&mut self, acts: &mut [linear::Part], slots: &mut [f64], dt: f64) {
        let own_open = self.e[0].as_mut().is_some_and(|x| x.yes(slots, dt));
        let own_ff = self.e[1].as_mut().is_some_and(|x| x.yes(slots, dt));
        let jam = self.e[2].as_mut().is_some_and(|x| x.yes(slots, dt)) || self.s.on(slots, "failed");
        let (open, ff) = self.sequenced.unwrap_or((own_open, own_ff));
        if let (true, Some(leg)) = (self.linked, self.follow) {
            // Linkage rods drive it: closed on the uplock, fully open a fifth of the way down.
            let target = self.travel * (leg / 0.2).clamp(0.0, 1.0);
            self.rot.x[0] = target;
            self.rot.x[1] = 0.0;
            self.lock.engaged = target < 0.5 * DEG;
            self.rot.lock = self.lock.engaged.then_some(0.0);
            let pos = self.position();
            let s = &self.s;
            s.put(slots, "position", pos);
            s.put(slots, "angle", target);
            s.flag(slots, "closed", self.lock.engaged);
            s.flag(slots, "open", pos > 0.97);
            s.flag(slots, "locked", self.lock.engaged);
            s.flag(slots, "in_transit", !self.lock.engaged && pos <= 0.97);
            return;
        }
        let t = self.rot.angle();
        let mut ios: Vec<Io> = vec![];
        let mut pressing = 0.0;
        for k in 0..self.rot.acts.len() {
            let ai = self.rot.acts[k];
            let cmd = if ff { 0.0 } else if open { 1.0 } else { -1.0 };
            let mut io = acts[ai].prepare(slots, dt, Some(cmd));
            if ff {
                io.mode = 0.0;
            }
            // The uplock's unlock actuator sits on the open line.
            if open && !ff {
                pressing = ((io.ps - io.pr) / acts[ai].act.nominal).clamp(0.0, 1.0) * io.mode;
            }
            ios.push(io);
        }
        self.lock.update(t < 0.5 * DEG, !open && !ff, pressing, ff, self.unlock_t, self.alt_t, dt);
        self.rot.lock = self.lock.engaged.then_some(0.0);
        if jam && self.rot.seized.is_none() {
            self.rot.seized = Some(t);
        } else if !jam {
            self.rot.seized = None;
        }
        self.rot.update_jams(acts, &ios);
        let q = self.q.map_or(0.0, |i| slots[i]).max(0.0);
        let (mass, cg, area, travel) = (self.mass, self.cg, self.area, self.travel);
        let g_sign = if self.hangs_open { 1.0 } else { 0.0 };
        let air_sign = if self.air_opens { 1.0 } else { -1.0 };
        let load = move |t: f64, w: f64| {
            let g = g_sign * mass * G * cg * t.cos().max(0.0);
            let a = air_sign * q * area * 0.6 * cg * (t / travel).clamp(0.0, 1.0).sin();
            g + a - 0.5 * mass * cg * cg * w
        };
        self.rot.step(acts, &ios, dt, &load);
        for k in 0..self.rot.acts.len() {
            let ai = self.rot.acts[k];
            let (x, v) = self.rot.actuator_motion(k);
            let st = self.rot.actuator_state(k).to_vec();
            let out = self.rot.outs[k];
            acts[ai].finish(slots, &ios[k], x, v, &st, &out, dt);
        }
        let pos = self.position();
        let s = &self.s;
        s.put(slots, "position", pos);
        s.put(slots, "angle", self.rot.angle());
        s.flag(slots, "closed", self.lock.engaged);
        s.flag(slots, "open", pos > 0.97);
        s.flag(slots, "locked", self.lock.engaged);
        s.flag(slots, "in_transit", !self.lock.engaged && pos <= 0.97);
        let eas = self.eas.map_or(0.0, |i| slots[i]).abs();
        self.over_t = if self.limit > 0.0 && pos > 0.1 && eas > 1.25 * self.limit { self.over_t + dt } else { 0.0 };
        if self.over_t > 2.0 {
            self.overstressed = true;
        }
        s.flag(slots, "overstressed", self.overstressed);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        self.rot.state(v);
        v.extend([self.lock.engaged as u8 as f64, self.lock.release, self.overstressed as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.rot.restore(it);
        self.lock.engaged = it.next().unwrap_or(1.0) != 0.0;
        self.lock.release = it.next().unwrap_or(0.0);
        self.overstressed = it.next().unwrap_or(0.0) != 0.0;
    }
}

/// The oleo-pneumatic shock strut: polytropic gas spring, metering orifice damping (stiffer on rebound), seal
/// friction and stroke ends. States: stroke, stroke rate.
#[derive(Clone, Debug)]
pub struct Strut {
    pub area: f64,
    pub stroke: f64,
    p0: f64,
    v0: f64,
    c_comp: f64,
    c_reb: f64,
    m_u: f64,
    pub x: [f64; 2],
    stepper: Stepper,
    pub gas: f64,
    /// Largest strut force since the wheels last left the ground.
    pub peak: f64,
    /// Closing rate when the strut started to compress.
    pub touchdown_rate: f64,
    pub bottomed: bool,
    airborne: bool,
    /// Gas temperature share of its 15 C servicing temperature: cold struts sit low.
    pub t_ratio: f64,
}

const GAS_N: f64 = 1.3;
const P_ATM: f64 = 101_325.0;

impl Strut {
    pub fn new(static_load: f64, stroke: f64, static_share: f64, static_p: f64, unsprung: f64) -> Self {
        let stroke = stroke.max(0.05);
        let p_st = static_p.max(2e5);
        let area = static_load.max(100.0) / p_st;
        let s_st = (static_share.clamp(0.2, 0.95)) * stroke;
        // Extended pressure a quarter of static: gas volume from the compression ratio at the static stroke.
        let k = 4f64.powf(1.0 / GAS_N);
        let v0 = area * s_st * k / (k - 1.0);
        let p0 = p_st / 4.0 + P_ATM;
        // Metering sized so a 3 m/s touchdown takes about the static load in damping.
        let c_comp = static_load.max(100.0) * 1.5 / 9.0;
        Strut { area, stroke, p0, v0, c_comp, c_reb: 4.0 * c_comp, m_u: unsprung.max(10.0), x: [0.0, 0.0], stepper: Stepper::new(2), gas: 1.0, peak: 0.0, touchdown_rate: 0.0, bottomed: false, airborne: true, t_ratio: 1.0 }
    }

    pub fn gas_force(&self, s: f64) -> f64 {
        let vol = (self.v0 - self.area * s.clamp(0.0, self.stroke)).max(0.02 * self.v0);
        let p = self.p0 * self.gas * self.t_ratio * (self.v0 / vol).powf(GAS_N);
        (p - P_ATM) * self.area
    }

    /// Tracks touchdowns: closing rate as the strut leaves full extension, peak force, bottoming.
    pub fn track(&mut self, before: [f64; 2]) {
        let force = self.gas_force(self.x[0]);
        if self.x[0] <= 0.002 {
            if !self.airborne && self.x[1] <= 0.0 {
                self.airborne = true;
            }
        } else if self.airborne && before[0] <= 0.002 {
            self.airborne = false;
            self.touchdown_rate = self.x[1].max(before[1]);
            self.peak = 0.0;
            self.bottomed = false;
        }
        self.peak = self.peak.max(force);
        if self.x[0] > 0.98 * self.stroke && self.x[1] > 0.05 {
            self.bottomed = true;
        }
    }

    pub fn pressure(&self) -> f64 {
        self.gas_force(self.x[0]) / self.area
    }

    pub fn step(&mut self, load: f64, dt: f64) {
        let (c1, c2, m, stroke) = (self.c_comp, self.c_reb, self.m_u, self.stroke);
        let mut stepper = std::mem::take(&mut self.stepper);
        let me = &*self;
        let f = |s: &[f64], d: &mut [f64]| {
            let (x, v) = (s[0], s[1]);
            let gas = me.gas_force(x);
            // Metering pin: its taper closes the orifice as the strut compresses, stiffening the damping.
            let pin = 1.0 + 2.0 * (x / stroke).clamp(0.0, 1.0).powi(2);
            let damp = if v > 0.0 { c1 * pin * v * v.abs() } else { c2 * v * v.abs() } + 0.05 * c1 * v;
            let fric = 0.01 * gas * super::ode::soft_sign(v, 0.01);
            let mut stop = 0.0;
            if x < 0.0 {
                stop = -x * 1e9 * me.area - v * 1e5;
            } else if x > stroke {
                stop = -(x - stroke) * 1e9 * me.area - v * 1e5;
            }
            d[0] = v;
            d[1] = (load - gas - damp - fric + stop) / m;
        };
        let mut x = self.x;
        let scale = [1e-4, 1e-2];
        let restored = stepper.step(&mut x, dt, &scale, 1.0, f) == Outcome::Restored;
        self.stepper = stepper;
        if restored {
            x = [x[0].clamp(0.0, self.stroke), 0.0];
        }
        self.x = x;
    }
}

pub struct Leg {
    pub rot: Rotary,
    up_lock: Lock,
    down_lock: Lock,
    pub strut: Strut,
    pub doors: Vec<usize>,
    e: [Option<Expr>; 10],
    air: [Option<usize>; 3],
    mass: f64,
    cg: f64,
    travel: f64,
    drag_area: f64,
    drag_sign: f64,
    spring: f64,
    unlock_t: f64,
    alt_t: f64,
    static_load: f64,
    wow_stroke: f64,
    doors_stay: bool,
    extends_down: bool,
    selector: f64,
    pub bogie: Option<usize>,
    hard_rate: f64,
    hard: bool,
    blow: Option<Expr>,
    bottle: (f64, f64),
    fired_at: Option<f64>,
    crank: Option<Expr>,
    turns: f64,
    vle: f64,
    over_t: f64,
    overstressed: bool,
    temp: Option<usize>,
    eas: Option<usize>,
    s: Slots,
}

pub const LEG_EXPRS: [&str; 10] = ["down_when", "free_fall_when", "neutral_when", "load", "sensor_1_failed_when", "sensor_2_failed_when", "uplock_jam_when", "downlock_failed_when", "jam_when", "strut_leak_when"];

impl Leg {
    pub fn new(c: &Checked, p: &CheckedPart, acts: &mut Vec<linear::Part>, act_index: &HashMap<usize, usize>, door_index: &HashMap<usize, usize>, bogie_index: &HashMap<usize, usize>) -> Self {
        let travel = p.num("travel").max(10.0 * DEG);
        let mass = p.num("mass").max(1.0);
        let cg = p.num("cg").max(0.05);
        let extends_down = !p.text("actuator_extends").eq_ignore_ascii_case("up");
        let mut links = vec![];
        let mut idx = vec![];
        let mut m_ref = mass * G * cg;
        if let Some(&k) = p.part("actuator").and_then(|r| act_index.get(&r)) {
            let (l, stroke) = Linkage::new(p.num("arm"), p.num("anchor"), 0.0, travel, extends_down);
            if acts[k].act.stroke < 0.98 * stroke || acts[k].act.stroke > 1.5 * stroke {
                let r = p.part("actuator").unwrap_or(0);
                acts[k].resize(&c.parts[r], stroke);
            }
            floated(acts, k);
            m_ref += acts[k].act.stall * l.r;
            links.push(l);
            idx.push(k);
        }
        let down = p.flag("initial_down");
        let start = if down { travel } else { 0.0 };
        let mut rot = Rotary::new(links, idx, mass * cg * cg * 1.4, 0.0, travel, start, m_ref);
        rot.init_actuators(acts, 101_325.0 + 50.0 * PSI);
        rot.lock = Some(start);
        let drag_sign = match p.text("retracts").to_ascii_lowercase().as_str() {
            "forward" => 1.0,
            "aft" => -1.0,
            _ => 0.15,
        };
        let static_load = p.num("static_load");
        let mut strut = Strut::new(static_load, p.num("strut_stroke"), p.num("static_stroke"), p.num("static_pressure"), 0.4 * mass);
        if down {
            // Start at rest on the ground under the static load.
            strut.x[0] = p.num("static_stroke").clamp(0.0, 1.0) * strut.stroke;
        }
        Leg {
            rot,
            up_lock: Lock { engaged: !down, release: 0.0 },
            down_lock: Lock { engaged: down, release: 0.0 },
            strut,
            doors: p.parts("doors").iter().filter_map(|r| door_index.get(r).copied()).collect(),
            e: LEG_EXPRS.map(|k| expr_of(p, k)),
            air: [slot(c, p, "air", "dynamic_pressure"), slot(c, p, "air", "pitch"), slot(c, p, "air", "on_ground")],
            mass,
            cg,
            travel,
            drag_area: p.num("drag_area").max(0.0),
            drag_sign,
            spring: p.num("lock_spring").max(0.0) * mass * G * cg,
            unlock_t: p.num("unlock_time"),
            alt_t: p.num("free_fall_release"),
            static_load,
            wow_stroke: p.num("wow_stroke").max(1e-3),
            doors_stay: p.flag("doors_stay_open"),
            extends_down,
            selector: if down { 1.0 } else { -1.0 },
            bogie: p.part("bogie").and_then(|r| bogie_index.get(&r).copied()),
            hard_rate: p.num("hard_landing_rate").max(0.1),
            hard: false,
            blow: expr_of(p, "blowdown_when"),
            bottle: (p.num("bottle_pressure").max(0.0), p.num("bottle_volume").max(1e-5)),
            fired_at: None,
            crank: expr_of(p, "crank"),
            turns: p.num("release_turns").max(0.0),
            vle: p.num("limit_speed").max(0.0),
            over_t: 0.0,
            overstressed: false,
            temp: slot(c, p, "air", "temperature"),
            eas: slot(c, p, "air", "eas"),
            s: Slots::of(p, &["position", "angle", "down_locked", "up_locked", "downlock_engaged", "uplock_engaged", "in_transit", "disagree", "doors_closed", "doors_open", "strut_stroke", "strut_pressure", "compressed", "wow_1", "wow_2", "weight_on_wheels", "selector", "strut_rate", "peak_load", "bottomed", "hard_landing", "retract_inhibited", "bottle_pressure", "overspeed", "overstressed", "failed"]),
        }
    }

    pub fn position(&self) -> f64 {
        self.rot.angle() / self.travel
    }

    pub fn step(&mut self, acts: &mut [linear::Part], doors: &mut [Door], bogies: &[Bogie], slots: &mut [f64], dt: f64) {
        let mut b = |k: usize| self.e[k].as_mut().is_some_and(|x| x.yes(slots, dt));
        let want_down = b(0);
        let ff_cmd = b(1);
        let neutral_cmd = b(2);
        let (s1_bad, s2_bad, uplock_jam, downlock_bad, jam, strut_leak) = (b(4), b(5), b(6), b(7), b(8), b(9));
        let load_in = self.e[3].as_mut().map_or(0.0, |x| x.eval(slots, dt));
        let blow = self.blow.as_mut().is_some_and(|x| x.yes(slots, dt));
        if blow && self.fired_at.is_none() {
            self.fired_at = Some(self.rot.actuator_motion(0).0);
        }
        let blown = self.fired_at.is_some();
        // The crank works in stages: hydraulics isolated, then the doors released, then the leg.
        let turns = self.crank.as_mut().map_or(0.0, |x| x.eval(slots, dt));
        let staged = |share: f64| self.turns > 0.0 && turns >= share * self.turns;
        let neutral = neutral_cmd || staged(1.0 / 3.0);
        let doors_free = ff_cmd || staged(2.0 / 3.0);
        let ff = ff_cmd || staged(1.0);
        let failed = self.s.on(slots, "failed");
        let up_locked = self.up_lock.engaged;
        let down_locked = self.down_lock.engaged;
        let target_reached = (want_down && down_locked) || (!want_down && up_locked);
        // Doors: open while the leg has somewhere to go, close once it is locked where it was sent.
        let doors_open = self.doors.iter().all(|&d| doors[d].linked || doors[d].position() > 0.97);
        let doors_wanted = ff || !target_reached || (want_down && self.doors_stay);
        for &d in &self.doors {
            doors[d].sequenced = Some((doors_wanted || doors_free, doors_free));
            if doors[d].linked {
                doors[d].follow = Some(self.position().clamp(0.0, 1.0));
            }
        }
        // A bogie still level, or a leg still on its strut's weight, keeps the downlock in.
        let bogie_ok = self.bogie.is_none_or(|b| bogies[b].trailed());
        let inhibited = !want_down && self.down_lock.engaged && !bogie_ok;
        let may_move = (self.doors.is_empty() || doors_open) && !inhibited;
        let dir = if want_down { 1.0 } else { -1.0 };
        let sel = if blown {
            if self.extends_down { 1.0 } else { -1.0 }
        } else if ff || neutral || (target_reached && !want_down) {
            0.0
        } else if may_move || target_reached {
            dir
        } else {
            0.0
        };
        self.selector = sel;
        let act_cmd = if self.extends_down { sel } else { -sel };
        let mut ios: Vec<Io> = vec![];
        let mut up_line = 0.0;
        let mut down_line = 0.0;
        for k in 0..self.rot.acts.len() {
            let ai = self.rot.acts[k];
            let mut io = acts[ai].prepare(slots, dt, Some(act_cmd));
            if ff || (neutral && self.turns > 0.0 && turns >= self.turns / 3.0) {
                io.mode = 0.0;
            }
            if let Some(x0) = self.fired_at {
                // Bottle gas expands into the extend side as the jack strokes; the other side vents overboard.
                let a = &acts[ai].act;
                let (x, _) = self.rot.actuator_motion(k);
                let swept = if self.extends_down { a.area_a * (x - x0).max(0.0) } else { a.area_b * (x0 - x).max(0.0) };
                let p = (self.bottle.0 + 101_325.0) * (self.bottle.1 / (self.bottle.1 + swept)).powf(1.4);
                io.ps = p.max(101_325.0);
                io.pr = 101_325.0;
                io.mode = 1.0;
            }
            let p = ((io.ps - io.pr) / acts[ai].act.nominal).clamp(0.0, 1.0) * io.mode;
            if sel > 0.5 {
                down_line = p;
            } else if sel < -0.5 {
                up_line = p;
            }
            ios.push(io);
        }
        // The uplock's unlock actuator is on the down line, the downlock's on the up line.
        let t = self.rot.angle();
        let alternate = (ff || blown) && !uplock_jam;
        self.up_lock.update(t < 0.5 * DEG, !want_down && !ff && !blown, if uplock_jam { 0.0 } else { down_line }, alternate, self.unlock_t, if blown { 0.3 } else { self.alt_t }, dt);
        self.down_lock.update(t > self.travel - 0.5 * DEG, !downlock_bad, up_line, false, self.unlock_t, self.alt_t, dt);
        self.rot.lock = if self.up_lock.engaged {
            Some(0.0)
        } else if self.down_lock.engaged {
            Some(self.travel)
        } else {
            None
        };
        if (jam || failed) && self.rot.seized.is_none() {
            self.rot.seized = Some(t);
        } else if !(jam || failed) {
            self.rot.seized = None;
        }
        self.rot.update_jams(acts, &ios);
        let q = self.air[0].map_or(0.0, |i| slots[i]).max(0.0);
        let pitch = self.air[1].map_or(0.0, |i| slots[i]);
        let on_ground = self.air[2].is_some_and(|i| slots[i] != 0.0);
        let (mass, cg, travel, cda, ds, spring) = (self.mass, self.cg, self.travel, self.drag_area, self.drag_sign, self.spring);
        let fore_aft = ds.abs() > 0.5;
        let load = move |t: f64, w: f64| {
            // Gravity pulls the leg toward hanging straight down; a fore-aft swinging leg feels the pitch attitude.
            let tilt = if fore_aft { pitch * ds.signum() } else { 0.0 };
            let g = mass * G * cg * (t + tilt).cos();
            let drag = ds * q * cda * 0.6 * cg * t.sin().powi(2);
            let near = ((t - (travel - 10.0 * DEG)) / (10.0 * DEG)).clamp(0.0, 1.0);
            g + drag + spring * near - 0.3 * mass * cg * cg * w
        };
        self.rot.step(acts, &ios, dt, &load);
        for k in 0..self.rot.acts.len() {
            let ai = self.rot.acts[k];
            let (x, v) = self.rot.actuator_motion(k);
            let st = self.rot.actuator_state(k).to_vec();
            let out = self.rot.outs[k];
            acts[ai].finish(slots, &ios[k], x, v, &st, &out, dt);
        }
        // Strut: wheel load when down; extends fully in the air.
        let wheel = if !self.down_lock.engaged {
            0.0
        } else if self.e[3].is_some() && load_in > 0.0 {
            load_in
        } else if on_ground {
            self.static_load
        } else {
            0.0
        };
        if strut_leak {
            self.strut.gas = (self.strut.gas - dt / 600.0).max(0.2);
        }
        self.strut.t_ratio = self.temp.map_or(1.0, |i| (slots[i].max(180.0)) / 288.15);
        let before = self.strut.x;
        self.strut.step(wheel, dt);
        self.strut.track(before);
        if self.strut.touchdown_rate > self.hard_rate {
            self.hard = true;
        }
        let compressed = self.strut.x[0] > self.wow_stroke;
        let wow1 = compressed && !s1_bad;
        let wow2 = compressed && !s2_bad;
        let pos = self.position().clamp(0.0, 1.0);
        let doors_closed = self.doors.iter().all(|&d| doors[d].position() < 0.01);
        let s = &self.s;
        s.put(slots, "position", pos);
        s.put(slots, "angle", self.rot.angle());
        s.flag(slots, "down_locked", self.down_lock.engaged);
        s.flag(slots, "up_locked", self.up_lock.engaged);
        s.flag(slots, "downlock_engaged", self.down_lock.engaged);
        s.flag(slots, "uplock_engaged", self.up_lock.engaged);
        s.flag(slots, "in_transit", !self.down_lock.engaged && !self.up_lock.engaged);
        s.flag(slots, "disagree", (want_down && !self.down_lock.engaged) || (!want_down && !self.up_lock.engaged));
        s.flag(slots, "doors_closed", doors_closed);
        s.flag(slots, "doors_open", self.doors.iter().all(|&d| doors[d].position() > 0.97));
        s.put(slots, "strut_stroke", self.strut.x[0]);
        s.put(slots, "strut_pressure", self.strut.pressure());
        s.flag(slots, "compressed", compressed);
        s.flag(slots, "wow_1", wow1);
        s.flag(slots, "wow_2", wow2);
        s.flag(slots, "weight_on_wheels", wow1 || wow2);
        s.put(slots, "selector", self.selector);
        s.put(slots, "strut_rate", self.strut.x[1]);
        s.put(slots, "peak_load", self.strut.peak);
        s.flag(slots, "bottomed", self.strut.bottomed);
        s.flag(slots, "hard_landing", self.hard);
        s.flag(slots, "retract_inhibited", inhibited);
        let bottle = match (self.fired_at, self.rot.acts.first()) {
            (Some(x0), Some(&ai)) => {
                let a = &acts[ai].act;
                let x = self.rot.actuator_motion(0).0;
                let swept = if self.extends_down { a.area_a * (x - x0).max(0.0) } else { a.area_b * (x0 - x).max(0.0) };
                (self.bottle.0 + 101_325.0) * (self.bottle.1 / (self.bottle.1 + swept)).powf(1.4) - 101_325.0
            }
            _ => self.bottle.0,
        };
        s.put(slots, "bottle_pressure", bottle);
        let eas = self.eas.map_or(0.0, |i| slots[i]).abs();
        let out = !self.up_lock.engaged;
        let over = self.vle > 0.0 && out && eas > self.vle;
        self.over_t = if over && eas > 1.25 * self.vle { self.over_t + dt } else { 0.0 };
        if self.over_t > 2.0 {
            self.overstressed = true;
        }
        s.flag(slots, "overspeed", over);
        s.flag(slots, "overstressed", self.overstressed);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        self.rot.state(v);
        v.extend([self.up_lock.engaged as u8 as f64, self.up_lock.release, self.down_lock.engaged as u8 as f64, self.down_lock.release]);
        v.extend(self.strut.x);
        v.extend([self.strut.gas, self.strut.peak, self.strut.touchdown_rate, self.strut.bottomed as u8 as f64, self.hard as u8 as f64, self.fired_at.unwrap_or(f64::NAN), self.overstressed as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.rot.restore(it);
        self.up_lock.engaged = it.next().unwrap_or(0.0) != 0.0;
        self.up_lock.release = it.next().unwrap_or(0.0);
        self.down_lock.engaged = it.next().unwrap_or(1.0) != 0.0;
        self.down_lock.release = it.next().unwrap_or(0.0);
        for v in &mut self.strut.x {
            *v = it.next().unwrap_or(*v);
        }
        self.strut.gas = it.next().unwrap_or(1.0);
        self.strut.peak = it.next().unwrap_or(0.0);
        self.strut.touchdown_rate = it.next().unwrap_or(0.0);
        self.strut.bottomed = it.next().unwrap_or(0.0) != 0.0;
        self.hard = it.next().unwrap_or(0.0) != 0.0;
        self.fired_at = it.next().filter(|x| x.is_finite());
        self.overstressed = it.next().unwrap_or(0.0) != 0.0;
    }
}
