//! A rotary mechanism: a body turning about a hinge (control surface, gear leg, door, steering collar) driven by
//! linear actuators through triangle linkages whose moment arm changes with angle, against a load the owner
//! supplies, with travel stops, jams and mechanical locks as stiff springs. Its angle, rate and every actuator's
//! chamber and valve states are stepped together by the implicit stepper.

use super::linear::{self, Io, Out, STATES};
use super::ode::{soft_pos, Outcome, Stepper};

/// A compliant attachment between each piston and its horn: rod and bearing stiffness, damping and the free play
/// worn rod-end bearings open up. With it the pistons get their own mass and motion.
#[derive(Clone, Copy, Debug)]
pub struct Attach {
    pub stiffness: f64,
    pub damping: f64,
    pub gap: f64,
    pub piston_mass: f64,
}

impl Attach {
    /// Force the piston puts on the horn with the piston `d` beyond where the horn is, closing at `dv`.
    pub fn force(&self, d: f64, dv: f64) -> f64 {
        let w = 0.1 * self.gap.max(1e-6);
        let dz = soft_pos(d - self.gap, w) - soft_pos(-d - self.gap, w);
        self.stiffness * dz + self.damping * dv * (dz.abs() / (dz.abs() + w)).max(0.05)
    }
}

/// The triangle a linear actuator forms with a hinge: horn of radius `r` on the moving body, anchor `d` from the
/// hinge on the structure. Extension follows the law of cosines.
#[derive(Clone, Copy, Debug)]
pub struct Linkage {
    pub r: f64,
    pub d: f64,
    pub phi: f64,
    /// Actuator length at zero extension.
    pub l0: f64,
}

impl Linkage {
    /// A linkage perpendicular at mid travel; `positive` means extending the actuator increases the angle.
    pub fn new(r: f64, d: f64, lo: f64, hi: f64, positive: bool) -> (Self, f64) {
        let r = r.max(1e-3);
        let d = d.max(1.5 * r);
        let gamma = (r / d).acos();
        let mid = 0.5 * (lo + hi);
        let phi = if positive { gamma - mid } else { -gamma - mid };
        let len = |t: f64| (r * r + d * d - 2.0 * r * d * (t + phi).cos()).sqrt();
        let (a, b) = (len(lo), len(hi));
        let range = (a - b).abs();
        let margin = 0.03 * range + 1e-3;
        let l0 = a.min(b) - margin;
        (Linkage { r, d, phi, l0 }, range + 2.0 * margin)
    }

    pub fn length(&self, t: f64) -> f64 {
        (self.r * self.r + self.d * self.d - 2.0 * self.r * self.d * (t + self.phi).cos()).sqrt()
    }

    pub fn extension(&self, t: f64) -> f64 {
        self.length(t) - self.l0
    }

    /// Moment arm: extension per radian.
    pub fn arm(&self, t: f64) -> f64 {
        self.r * self.d * (t + self.phi).sin() / self.length(t).max(1e-6)
    }

    /// Angle in [lo, hi] giving extension `x`, by bisection.
    pub fn angle_for(&self, x: f64, lo: f64, hi: f64) -> f64 {
        let (mut a, mut b) = (lo, hi);
        let rising = self.extension(hi) > self.extension(lo);
        for _ in 0..50 {
            let m = 0.5 * (a + b);
            if (self.extension(m) < x) == rising {
                a = m;
            } else {
                b = m;
            }
        }
        0.5 * (a + b)
    }
}

/// The load the owner puts on the body: moment at angle and rate, positive in the direction of increasing angle.
pub trait Load {
    fn moment(&self, angle: f64, rate: f64) -> f64;
}

impl<F: Fn(f64, f64) -> f64> Load for F {
    fn moment(&self, angle: f64, rate: f64) -> f64 {
        self(angle, rate)
    }
}

#[derive(Clone, Debug)]
pub struct Rotary {
    pub links: Vec<Linkage>,
    /// Indices of the actuator parts that drive it.
    pub acts: Vec<usize>,
    pub x: Vec<f64>,
    scale: Vec<f64>,
    stepper: Stepper,
    pub inertia: f64,
    pub lo: f64,
    pub hi: f64,
    /// Moment the stops and locks are scaled from.
    pub moment_ref: f64,
    /// Where each actuator seized, if it has.
    pub jam: Vec<Option<f64>>,
    /// A mechanical lock holding the body at this angle.
    pub lock: Option<f64>,
    /// The whole surface seized at this angle.
    pub seized: Option<f64>,
    pub ios: Vec<Io>,
    pub outs: Vec<Out>,
    pub restored: u32,
    pub attach: Option<Attach>,
}

impl Rotary {
    pub fn new(links: Vec<Linkage>, acts: Vec<usize>, inertia: f64, lo: f64, hi: f64, start: f64, moment_ref: f64) -> Self {
        let n = 2 + STATES * acts.len();
        let mut x = vec![0.0; n];
        x[0] = start.clamp(lo, hi);
        let mut scale = vec![1.0; n];
        scale[0] = 2e-4;
        scale[1] = 5e-3;
        let k = acts.len();
        Rotary { links, acts, x, scale, stepper: Stepper::new(n), inertia: inertia.max(1e-6), lo, hi, moment_ref: moment_ref.max(1.0), jam: vec![None; k], lock: None, seized: None, ios: vec![], outs: vec![Out::default(); k], restored: 0, attach: None }
    }

    /// States each actuator carries: its chambers and valves, and with a compliant attachment its piston.
    pub fn per(&self) -> usize {
        STATES + if self.attach.is_some() { 2 } else { 0 }
    }

    /// Gives every actuator a compliant attachment; call before `init_actuators`.
    pub fn with_attachment(&mut self, a: Attach) {
        self.attach = Some(a);
        let n = 2 + self.per() * self.acts.len();
        self.x.resize(n, 0.0);
        self.scale.resize(n, 1.0);
        self.stepper = Stepper::new(n);
    }

    /// Puts each actuator's chambers at rest at return pressure and records its state scales.
    pub fn init_actuators(&mut self, parts: &[linear::Part], pr: f64) {
        let per = self.per();
        for (k, &ai) in self.acts.iter().enumerate() {
            let a = &parts[ai].act;
            let o = 2 + per * k;
            a.initial(pr, &mut self.x[o..o + STATES]);
            a.scales(&mut self.scale[o..o + STATES]);
            if self.attach.is_some() {
                self.x[o + STATES] = self.links[k].extension(self.x[0]);
                self.x[o + STATES + 1] = 0.0;
                self.scale[o + STATES] = 1e-6 * a.stroke.max(1e-3);
                self.scale[o + STATES + 1] = 1e-3 * a.rate;
            }
        }
    }

    pub fn angle(&self) -> f64 {
        self.x[0]
    }

    pub fn rate(&self) -> f64 {
        self.x[1]
    }

    /// Steps the body with actuator inputs `ios` (one per actuator), returning each actuator's final output.
    pub fn step(&mut self, parts: &[linear::Part], ios: &[Io], dt: f64, load: &dyn Load) {
        let links = &self.links;
        let acts = &self.acts;
        let jam = &self.jam;
        let (lo, hi, inertia, lock, seized) = (self.lo, self.hi, self.inertia, self.lock, self.seized);
        let k_stop = 400.0 * self.moment_ref;
        let c_stop = 2.0 * (k_stop * inertia).sqrt();
        let k_lock = 4000.0 * self.moment_ref;
        let c_lock = 2.0 * (k_lock * inertia).sqrt();
        let attach = self.attach;
        let per = self.per();
        let f = |s: &[f64], d: &mut [f64]| {
            let (t, w) = (s[0], s[1]);
            let mut m = load.moment(t, w);
            for (k, &ai) in acts.iter().enumerate() {
                let a = &parts[ai].act;
                let l = &links[k];
                let xl = l.extension(t);
                let arm = l.arm(t);
                let vl = arm * w;
                let o = 2 + per * k;
                let (x, v) = if attach.is_some() { (s[o + STATES], s[o + STATES + 1]) } else { (xl, vl) };
                let out = a.eval(&ios[k], x, v, &s[o..o + STATES], &mut d[o..o + STATES]);
                // Piston stroke ends: the actuator bottoms out before the body stops if it is too short.
                let m_eff = attach.map_or(inertia / (arm * arm).max(1e-6), |at| at.piston_mass);
                let mut force = out.force + a.stroke_stop(x, v, m_eff);
                if let Some(xj) = jam[k] {
                    let kj = 200.0 * a.stall / a.stroke.max(1e-3);
                    force -= kj * (x - xj) + 1.4 * (kj * m_eff).sqrt() * v;
                }
                if let Some(at) = attach {
                    // The piston drives the horn only through its attachment; its own mass sits between.
                    let link = if ios[k].disconnected { 0.0 } else { at.force(x - xl, v - vl) };
                    d[o + STATES] = v;
                    d[o + STATES + 1] = (force - link) / at.piston_mass;
                    m += link * arm;
                } else {
                    m += force * arm;
                }
            }
            let wd = 1e-4 * (hi - lo).abs().max(1e-3);
            let over = soft_pos(t - hi, wd) - soft_pos(lo - t, wd);
            let touch = super::ode::smooth_step(t - hi, 0.0, 2.0 * wd) + super::ode::smooth_step(lo - t, 0.0, 2.0 * wd);
            m -= k_stop * over + c_stop * w * touch;
            if let Some(l) = lock.or(seized) {
                m -= k_lock * (t - l) + c_lock * w;
            }
            d[0] = w;
            d[1] = m / inertia;
        };
        if self.stepper.step(&mut self.x, dt, &self.scale, 1.0, f) == Outcome::Restored {
            self.restored += 1;
            self.x[1] = 0.0;
        }
        // Final outputs at the accepted state.
        let mut scratch = vec![0.0; STATES];
        for (k, &ai) in self.acts.iter().enumerate() {
            let o = 2 + per * k;
            for v in &mut self.x[o..o + 2] {
                *v = v.max(parts[ai].act.fluid.vapour());
            }
            let (x, v) = self.actuator_motion(k);
            self.outs[k] = parts[ai].act.eval(&ios[k], x, v, &self.x[o..o + STATES], &mut scratch);
        }
        self.ios = ios.to_vec();
    }

    /// Extension and velocity of actuator `k` now.
    pub fn actuator_motion(&self, k: usize) -> (f64, f64) {
        if self.attach.is_some() {
            let o = 2 + self.per() * k + STATES;
            return (self.x[o], self.x[o + 1]);
        }
        let l = &self.links[k];
        (l.extension(self.x[0]), l.arm(self.x[0]) * self.x[1])
    }

    pub fn actuator_state(&self, k: usize) -> &[f64] {
        let o = 2 + self.per() * k;
        &self.x[o..o + STATES]
    }

    /// Piston beyond where its horn is, for free-play wear and load monitoring.
    pub fn attachment_slack(&self, k: usize) -> f64 {
        if self.attach.is_none() {
            return 0.0;
        }
        self.actuator_motion(k).0 - self.links[k].extension(self.x[0])
    }

    /// Actuator share of stroke that puts the body at `angle`.
    pub fn command_for(&self, parts: &[linear::Part], k: usize, angle: f64) -> f64 {
        let a = &parts[self.acts[k]].act;
        self.links[k].extension(angle.clamp(self.lo, self.hi)) / a.stroke.max(1e-6)
    }

    /// Records a jam the first step it appears and clears it when it goes.
    pub fn update_jams(&mut self, parts: &[linear::Part], ios: &[Io]) {
        let _ = parts;
        for k in 0..self.acts.len() {
            let jammed = ios[k].jammed;
            if jammed && self.jam[k].is_none() {
                self.jam[k] = Some(self.actuator_motion(k).0);
            } else if !jammed {
                self.jam[k] = None;
            }
        }
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend_from_slice(&self.x);
        v.push(self.lock.unwrap_or(f64::NAN));
        v.push(self.seized.unwrap_or(f64::NAN));
        v.extend(self.jam.iter().map(|j| j.unwrap_or(f64::NAN)));
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        for v in &mut self.x {
            *v = it.next().unwrap_or(*v);
        }
        let opt = |v: Option<f64>| v.filter(|x| x.is_finite());
        self.lock = opt(it.next());
        self.seized = opt(it.next());
        for j in &mut self.jam {
            *j = opt(it.next());
        }
    }

    /// True when every state is finite and inside sane bounds.
    pub fn healthy(&self) -> bool {
        self.x.iter().all(|v| v.is_finite()) && self.x[0].abs() < 10.0 && self.x[1].abs() < 1e3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linkage_arm_is_the_horn_at_mid_travel_and_inverts() {
        let lo = -0.5;
        let hi = 0.5;
        let (l, stroke) = Linkage::new(0.1, 0.8, lo, hi, true);
        assert!((l.arm(0.0) - 0.1).abs() < 1e-3, "{}", l.arm(0.0));
        assert!(l.extension(lo) > 0.0 && l.extension(hi) < stroke);
        let x = l.extension(0.2);
        assert!((l.angle_for(x, lo, hi) - 0.2).abs() < 1e-9);
        let (n, _) = Linkage::new(0.1, 0.8, lo, hi, false);
        assert!(n.arm(0.0) < 0.0);
    }
}
