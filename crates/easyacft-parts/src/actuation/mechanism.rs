//! A rotary mechanism: a body turning about a hinge (control surface, gear leg, door, steering collar) driven by
//! linear actuators through triangle linkages whose moment arm changes with angle, against a load the owner
//! supplies, with travel stops, jams and mechanical locks as stiff springs. Its angle, rate and every actuator's
//! chamber and valve states are stepped together by the implicit stepper.

use super::linear::{self, Io, Out, STATES};
use super::ode::{Outcome, Stepper};

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
        Rotary { links, acts, x, scale, stepper: Stepper::new(n), inertia: inertia.max(1e-6), lo, hi, moment_ref: moment_ref.max(1.0), jam: vec![None; k], lock: None, seized: None, ios: vec![], outs: vec![Out::default(); k], restored: 0 }
    }

    /// Puts each actuator's chambers at rest at return pressure and records its state scales.
    pub fn init_actuators(&mut self, parts: &[linear::Part], pr: f64) {
        for (k, &ai) in self.acts.iter().enumerate() {
            let a = &parts[ai].act;
            let o = 2 + STATES * k;
            a.initial(pr, &mut self.x[o..o + STATES]);
            a.scales(&mut self.scale[o..o + STATES]);
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
        let f = |s: &[f64], d: &mut [f64]| {
            let (t, w) = (s[0], s[1]);
            let mut m = load.moment(t, w);
            for (k, &ai) in acts.iter().enumerate() {
                let a = &parts[ai].act;
                let l = &links[k];
                let x = l.extension(t);
                let arm = l.arm(t);
                let v = arm * w;
                let o = 2 + STATES * k;
                let out = a.eval(&ios[k], x, v, &s[o..o + STATES], &mut d[o..o + STATES]);
                let mut force = out.force;
                // Piston stroke ends: the actuator bottoms out before the surface stops if it is too short.
                if x < 0.0 {
                    force += -x * 50.0 * a.stall / a.stroke.max(1e-3) - v * 5.0 * a.stall / a.rate;
                } else if x > a.stroke {
                    force -= (x - a.stroke) * 50.0 * a.stall / a.stroke.max(1e-3) + v * 5.0 * a.stall / a.rate;
                }
                if let Some(xj) = jam[k] {
                    force -= (x - xj) * 200.0 * a.stall / a.stroke.max(1e-3) + v * 20.0 * a.stall / a.rate;
                }
                m += force * arm;
            }
            if t > hi {
                m -= k_stop * (t - hi) + c_stop * w;
            } else if t < lo {
                m -= k_stop * (t - lo) + c_stop * w;
            }
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
        let t = self.x[0];
        let w = self.x[1];
        for (k, &ai) in self.acts.iter().enumerate() {
            let l = &self.links[k];
            let o = 2 + STATES * k;
            for v in &mut self.x[o..o + 2] {
                *v = v.max(parts[ai].act.fluid.vapour());
            }
            self.outs[k] = parts[ai].act.eval(&ios[k], l.extension(t), l.arm(t) * w, &self.x[o..o + STATES], &mut scratch);
        }
        self.ios = ios.to_vec();
    }

    /// Extension and velocity of actuator `k` now.
    pub fn actuator_motion(&self, k: usize) -> (f64, f64) {
        let l = &self.links[k];
        (l.extension(self.x[0]), l.arm(self.x[0]) * self.x[1])
    }

    pub fn actuator_state(&self, k: usize) -> &[f64] {
        let o = 2 + STATES * k;
        &self.x[o..o + STATES]
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
                self.jam[k] = Some(self.links[k].extension(self.x[0]));
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
