//! A flight control surface on its hinge: aileron, elevator, rudder or spoiler, driven by one or more linear
//! actuators in active/active or active/standby. Aerodynamic hinge moment from dynamic pressure, local angle of
//! attack and deflection; aerodynamic damping that turns negative past the flutter speed, so a surface with no
//! actuator damping left flutters; gravity droop on the ground; turbulence; blowdown when the aero load beats the
//! actuators' stall force; hinge overstress.

use super::linear::{self, Io};
use super::mechanism::{Linkage, Rotary};
use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const WHEN: Kind = Kind::Expr(Type::Bool);
const AIR: &[&str] = &["environment.air"];
const DEG: f64 = std::f64::consts::PI / 180.0;
const G: f64 = 9.80665;

pub static SPEC: PartSpec = PartSpec {
    domain: "flight_control",
    kind: "surface",
    help: "A control surface on its hinge driven by its actuators through horn linkages: aero hinge moments, damping and flutter, gravity droop, turbulence, blowdown, hinge overstress.",
    fields: &[
        req("actuators", Kind::Refs(&["actuator.linear"]), "the actuators that drive it"),
        opt("command", Kind::Expr(N), "0 deg", "deflection the computers ask for"),
        opt("air", Kind::Ref(AIR), "", "for dynamic pressure, airspeed and attitude"),
        opt("min", Kind::Quantity(Dim::NONE), "-25 deg", "travel stop"),
        opt("max", Kind::Quantity(Dim::NONE), "25 deg", "travel stop"),
        opt("area", Kind::Number, "2", "surface area, m2"),
        opt("chord", Kind::Quantity(Dim::LENGTH), "0.6 m", "mean chord aft of the hinge"),
        opt("mass", Kind::Quantity(Dim::MASS), "0 kg", "surface mass; 0 means 20 kg per m2"),
        opt("cg_aft", Kind::Number, "0.3", "centre of gravity aft of the hinge, share of chord; near 0 when mass balanced"),
        opt("orientation", Kind::Text, "horizontal", "horizontal (gravity droops it) or vertical (fin, rudder)"),
        opt("deflects", Kind::Text, "down", "down: positive moves the trailing edge down (ailerons, elevators, flaps); up: positive raises it (spoilers)"),
        opt("ch0", Kind::Number, "0", "hinge moment coefficient at zero angle and deflection; spoilers about -0.1"),
        opt("ch_alpha", Kind::Number, "-0.25", "hinge moment coefficient per radian of local angle of attack"),
        opt("ch_delta", Kind::Number, "-0.55", "hinge moment coefficient per radian of deflection"),
        opt("ch_rate", Kind::Number, "-1.2", "aerodynamic damping coefficient"),
        opt("local_alpha", Kind::Expr(N), "0 deg", "local flow angle at the surface; defaults to none"),
        opt("flutter_speed", Kind::Quantity(Dim::SPEED), "0 kt", "equivalent airspeed past which the surface needs actuator damping to stay stable; 0 never"),
        opt("hinge_limit", Kind::Quantity(Dim::TORQUE), "0 N*m", "hinge moment that overstresses the fittings; 0 means twice the actuators' stall moment"),
        opt("horn", Kind::Quantity(Dim::LENGTH), "0 m", "actuator horn radius; 0 means 18 % of chord"),
        opt("anchor", Kind::Quantity(Dim::LENGTH), "0 m", "hinge to actuator anchor; 0 means six horns"),
        opt("turbulence", Kind::Expr(N), "0", "gust intensity, 0 calm to 1 severe"),
        opt("jam_when", WHEN, "false", "surface seized on its hinge"),
        opt("separated_when", WHEN, "false", "surface gone: no load, no motion"),
        opt("tab", Kind::Expr(N), "0 deg", "tab deflection"),
        opt("ch_tab", Kind::Number, "-0.35", "hinge moment coefficient per radian of tab"),
        opt("sensor_1_fault", Kind::Expr(N), "0", "surface position transducer 1: 0 healthy, 1 open primary, 2 open secondary, 3 detached, 4 drifting"),
        opt("sensor_2_fault", Kind::Expr(N), "0", "transducer 2, the same codes"),
        opt("free_play", Kind::Quantity(Dim::LENGTH), "0 m", "free play at each actuator's rod ends, worn bearings; gives the pistons their own motion"),
        opt("attachment_stiffness", Kind::Number, "0", "rod and bearing stiffness, N per m; 0 rigid unless free play is given"),
        opt("piston_mass", Kind::Quantity(Dim::MASS), "6 kg", "piston and rod mass, with a compliant attachment"),
        opt("force_equalization", Kind::Bool, "false", "active/active actuators trim their commands against each other's chamber pressure so their transducer offsets do not set them fighting"),
    ],
    outputs: &[
        ("angle", N),
        ("rate", N),
        ("command", N),
        ("hinge_moment", Type::Num(Dim::TORQUE)),
        ("error", N),
        ("blowdown", B),
        ("flutter", B),
        ("oscillation", N),
        ("overstress", B),
        ("active_actuators", N),
        ("drooped", B),
        ("sensor_1", N),
        ("sensor_2", N),
        ("sensor_disagree", B),
        ("slack", Type::Num(Dim::LENGTH)),
        ("force_fight", Type::Num(Dim::FORCE)),
        ("failed", B),
    ],
};

pub struct Surface {
    pub rot: Rotary,
    air: [Option<usize>; 4],
    e: [Option<Expr>; 5],
    area: f64,
    chord: f64,
    mass: f64,
    cg: f64,
    gravity: bool,
    up: bool,
    ch: [f64; 4],
    flutter_v: f64,
    hinge_limit: f64,
    gust: f64,
    rng: u64,
    osc: f64,
    osc_mean: f64,
    overstressed: bool,
    rvdt: super::sensors::Dual,
    extra: [Option<Expr>; 3],
    ch_tab: f64,
    equalize: bool,
    trims: Vec<f64>,
    s: Slots,
}

fn slot(c: &Checked, p: &CheckedPart, field: &str, out: &str) -> Option<usize> {
    p.part(field).and_then(|i| c.slots.index.get(&format!("{}.{out}", c.parts[i].id)).copied())
}

impl Surface {
    /// `act_index` maps a part index to its slot in the actuator list; `parts` lets sizing see each actuator.
    pub fn new(c: &Checked, p: &CheckedPart, acts: &mut Vec<linear::Part>, act_index: &std::collections::HashMap<usize, usize>) -> Self {
        let lo = p.num("min");
        let hi = p.num("max").max(lo + 1e-3);
        let chord = p.num("chord").max(0.05);
        let area = p.num("area").max(0.01);
        let horn = if p.num("horn") > 0.0 { p.num("horn") } else { 0.18 * chord };
        let anchor = if p.num("anchor") > 0.0 { p.num("anchor") } else { 6.0 * horn };
        let mass = if p.num("mass") > 0.0 { p.num("mass") } else { 20.0 * area };
        let cg = p.num("cg_aft") * chord;
        // Plate about its leading-edge hinge, plus the offset of its mass.
        let inertia = mass * chord * chord / 3.0 + mass * cg * cg * 0.2;
        let mut links = Vec::new();
        let mut idx = Vec::new();
        let mut m_ref = 0.0;
        for r in p.parts("actuators") {
            let Some(&k) = act_index.get(&r) else { continue };
            let (l, stroke) = Linkage::new(horn, anchor, lo, hi, true);
            let a = &mut acts[k];
            if a.act.stroke < stroke * 0.98 || (a.act.stroke - stroke).abs() > 0.5 * stroke {
                a.resize(&c.parts[r], stroke);
            }
            m_ref += a.act.stall * horn;
            links.push(l);
            idx.push(k);
        }
        let start = 0.0f64.clamp(lo, hi);
        let mut rot = Rotary::new(links, idx, inertia, lo, hi, start, m_ref.max(10.0));
        let gap = p.num("free_play").max(0.0);
        let k_att = if p.num("attachment_stiffness") > 0.0 { p.num("attachment_stiffness") } else if gap > 0.0 { 1e8 } else { 0.0 };
        if k_att > 0.0 {
            let m_p = p.num("piston_mass").max(0.1);
            rot.with_attachment(super::mechanism::Attach { stiffness: k_att, damping: 0.1 * (k_att * m_p).sqrt(), gap, piston_mass: m_p });
        }
        rot.init_actuators(acts, 101_325.0 + 50.0 * 6894.757);
        let given = p.num("hinge_limit");
        Surface {
            rot,
            air: [slot(c, p, "air", "dynamic_pressure"), slot(c, p, "air", "eas"), slot(c, p, "air", "airspeed"), slot(c, p, "air", "pitch")],
            e: ["command", "local_alpha", "turbulence", "jam_when", "separated_when"].map(|k| expr_of(p, k)),
            area,
            chord,
            mass,
            cg,
            gravity: !p.text("orientation").eq_ignore_ascii_case("vertical"),
            up: p.text("deflects").eq_ignore_ascii_case("up"),
            ch: [p.num("ch0"), p.num("ch_alpha"), p.num("ch_delta"), p.num("ch_rate")],
            flutter_v: p.num("flutter_speed"),
            hinge_limit: if given > 0.0 { given } else { 2.0 * m_ref.max(10.0) },
            gust: 0.0,
            rng: 0x9e37_79b9_7f4a_7c15 ^ (p.slot as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9),
            osc: 0.0,
            osc_mean: 0.0,
            overstressed: false,
            rvdt: super::sensors::Dual::new(lo, hi, (p.slot as u64) ^ 0xa076_1d64_78bd_642f),
            extra: ["tab", "sensor_1_fault", "sensor_2_fault"].map(|k| expr_of(p, k)),
            ch_tab: p.num("ch_tab"),
            equalize: p.flag("force_equalization"),
            trims: vec![0.0; p.parts("actuators").len()],
            s: Slots::of(p, &["angle", "rate", "command", "hinge_moment", "error", "blowdown", "flutter", "oscillation", "overstress", "active_actuators", "drooped", "sensor_1", "sensor_2", "sensor_disagree", "slack", "force_fight", "failed"]),
        }
    }

    fn noise(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }

    pub fn step(&mut self, acts: &mut [linear::Part], slots: &mut [f64], dt: f64) {
        let ev = |e: &mut Option<Expr>, s: &[f64]| e.as_mut().map_or(0.0, |x| x.eval(s, dt));
        let cmd = ev(&mut self.e[0], slots).clamp(self.rot.lo, self.rot.hi);
        let alpha = ev(&mut self.e[1], slots);
        let turb = ev(&mut self.e[2], slots).clamp(0.0, 1.0);
        let jam = ev(&mut self.e[3], slots) != 0.0 || self.s.on(slots, "failed");
        let gone = ev(&mut self.e[4], slots) != 0.0;
        let tab = ev(&mut self.extra[0], slots);
        let f1 = super::sensors::fault_code(ev(&mut self.extra[1], slots));
        let f2 = super::sensors::fault_code(ev(&mut self.extra[2], slots));
        let ch_tab = self.ch_tab;
        let q = self.air[0].map_or(0.0, |i| slots[i]).max(0.0);
        let eas = self.air[1].map_or(0.0, |i| slots[i]).abs();
        let v = self.air[2].map_or(0.0, |i| slots[i]).abs().max(eas).max(1.0);
        let pitch = self.air[3].map_or(0.0, |i| slots[i]);
        // Gust: first-order filtered noise, a fraction of a second correlation, a few degrees in severe turbulence.
        let tau = (self.chord * 20.0 / v).clamp(0.05, 2.0);
        let white = self.noise();
        self.gust += (white * (2.0 * tau / dt).sqrt() * 4.0 * DEG * turb - self.gust) * (dt / tau).min(1.0);
        let gust = self.gust;
        if jam && self.rot.seized.is_none() {
            self.rot.seized = Some(self.rot.angle());
        } else if !jam {
            self.rot.seized = None;
        }
        let mut ios: Vec<Io> = Vec::with_capacity(self.rot.acts.len());
        for k in 0..self.rot.acts.len() {
            let share = self.rot.command_for(acts, k, cmd) + self.trims.get(k).copied().unwrap_or(0.0);
            let ai = self.rot.acts[k];
            ios.push(acts[ai].prepare(slots, dt, Some(share)));
        }
        self.rot.update_jams(acts, &ios);
        let (area, chord, mass, cg) = (self.area, self.chord, if gone { 1e-3 } else { self.mass }, self.cg);
        let [ch0, cha, chd, chq] = self.ch;
        let sign = if self.up { -1.0 } else { 1.0 };
        let qsc = if gone { 0.0 } else { q * area * chord };
        let flutter = self.flutter_v > 0.0 && eas > self.flutter_v;
        let excess = if flutter { 15.0 * (eas / self.flutter_v - 1.0).min(1.0) } else { 0.0 };
        let gravity = self.gravity;
        let load = move |t: f64, w: f64| {
            // Hinge moment in the surface's own sense.
            let aero = qsc * (ch0 + cha * (alpha + gust) + chd * t + ch_tab * tab);
            let damp = qsc * chord / (2.0 * v) * (chq + excess) * w;
            let g = if gravity { mass * G * cg * (t * sign + pitch).cos() * sign } else { 0.0 };
            aero + damp + g - 0.02 * mass * chord * chord * w
        };
        self.rot.step(acts, &ios, dt, &load);
        let t = self.rot.angle();
        let w = self.rot.rate();
        let hinge = load(t, 0.0);
        // Oscillation: how far the rate swings about its mean, filtered over a second.
        self.osc_mean += (w - self.osc_mean) * (dt / 1.0).min(1.0);
        self.osc += ((w - self.osc_mean).abs() - self.osc) * (dt / 0.5).min(1.0);
        let mut active = 0.0;
        for k in 0..self.rot.acts.len() {
            let ai = self.rot.acts[k];
            let (x, xv) = self.rot.actuator_motion(k);
            let st: Vec<f64> = self.rot.actuator_state(k).to_vec();
            let out = self.rot.outs[k];
            acts[ai].finish(slots, &ios[k], x, xv, &st, &out, dt);
            if acts[ai].mode.share > 0.5 && acts[ai].mode.mode != linear::Mode::Passive {
                active += 1.0;
            }
        }
        // Force fight between active actuators: each one's share of the total against the average.
        let forces: Vec<(usize, f64)> = (0..self.rot.acts.len()).filter(|&k| acts[self.rot.acts[k]].mode.mode != linear::Mode::Passive).map(|k| (k, self.rot.outs[k].force)).collect();
        let fight = if forces.len() >= 2 {
            let hi = forces.iter().map(|f| f.1).fold(f64::MIN, f64::max);
            let lo = forces.iter().map(|f| f.1).fold(f64::MAX, f64::min);
            hi - lo
        } else {
            0.0
        };
        if self.equalize && forces.len() >= 2 {
            let mean = forces.iter().map(|f| f.1).sum::<f64>() / forces.len() as f64;
            for &(k, f) in &forces {
                let a = &acts[self.rot.acts[k]].act;
                // Integral trim: pushing harder than the average pulls this actuator's command back a little.
                if let Some(tr) = self.trims.get_mut(k) {
                    *tr = (*tr - 2.0 * (f - mean) / a.stall * dt * 0.02).clamp(-0.02, 0.02);
                }
            }
        } else {
            self.trims.iter_mut().for_each(|t| *t *= (1.0 - dt).max(0.0));
        }
        let stall: f64 = self.rot.acts.iter().zip(&self.rot.links).filter(|(&ai, _)| acts[ai].mode.mode != linear::Mode::Passive).map(|(&ai, l)| acts[ai].act.stall * l.arm(t).abs()).sum();
        let err = cmd - t;
        let blowdown = active > 0.0 && err.abs() > 1.0 * DEG && hinge.abs() > 0.8 * stall && err * hinge < 0.0;
        if hinge.abs() > self.hinge_limit {
            self.overstressed = true;
        }
        let (r1, r2, _, rd) = self.rvdt.read(t, f1, f2, dt);
        let s = &self.s;
        s.put(slots, "sensor_1", r1);
        s.put(slots, "sensor_2", r2);
        s.flag(slots, "sensor_disagree", rd);
        let slack = (0..self.rot.acts.len()).map(|k| self.rot.attachment_slack(k)).fold(0.0f64, |a, b| if b.abs() > a.abs() { b } else { a });
        s.put(slots, "slack", slack);
        s.put(slots, "force_fight", fight);
        s.put(slots, "angle", t);
        s.put(slots, "rate", w);
        s.put(slots, "command", cmd);
        s.put(slots, "hinge_moment", hinge);
        s.put(slots, "error", err);
        s.flag(slots, "blowdown", blowdown);
        s.flag(slots, "flutter", self.osc > 0.5);
        s.put(slots, "oscillation", self.osc);
        s.flag(slots, "overstress", self.overstressed);
        s.put(slots, "active_actuators", active);
        s.flag(slots, "drooped", active == 0.0 && t * sign > 2.0 * DEG);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        self.rot.state(v);
        v.extend([self.gust, self.osc, self.osc_mean, self.overstressed as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.rot.restore(it);
        self.gust = it.next().unwrap_or(0.0);
        self.osc = it.next().unwrap_or(0.0);
        self.osc_mean = it.next().unwrap_or(0.0);
        self.overstressed = it.next().unwrap_or(0.0) != 0.0;
    }
}
