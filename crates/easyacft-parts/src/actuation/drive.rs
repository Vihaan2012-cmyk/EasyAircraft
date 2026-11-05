//! A motor-driven output: one or two hydraulic motors (or a hydraulic and an electric motor) into a speed-summing
//! differential, then a gear train or ball screw to the output, against the load the owner gives. Shared by the
//! stabiliser trim actuator and the flap and slat power control units. States: output angle, input 1 speed,
//! input 2 speed.

use super::motor::{self, differential, MotorIo};
use super::ode::{soft_sign, Outcome, Stepper};

/// An electric motor on the second input: torque falls linearly with speed; its brake holds when off.
#[derive(Clone, Copy, Debug)]
pub struct Electric {
    pub stall: f64,
    pub w_free: f64,
    pub inertia: f64,
    pub eff: f64,
}

impl Electric {
    pub fn torque(&self, cmd: f64, w: f64, braked: bool) -> f64 {
        if braked {
            return -1.5 * self.stall * soft_sign(w, 0.5);
        }
        self.stall * cmd.clamp(-1.0, 1.0) - self.stall / self.w_free * w
    }

    pub fn power(&self, cmd: f64, w: f64) -> f64 {
        if cmd.abs() < 1e-3 {
            return 0.0;
        }
        (self.torque(cmd, w, false) * w).abs() / self.eff + 0.05 * self.stall * self.w_free * cmd.abs()
    }
}

/// What drives each differential input this step.
#[derive(Clone, Copy, Debug)]
pub enum Input {
    Hydraulic(usize, MotorIo),
    Electric(Electric, f64, bool),
    /// Nothing on this input: held by the gearbox.
    Locked,
}

#[derive(Clone, Debug)]
pub struct Drive {
    /// Output radians per input radian, averaged over both inputs.
    pub ratio: f64,
    pub out_inertia: f64,
    pub lo: f64,
    pub hi: f64,
    pub x: [f64; 3],
    stepper: Stepper,
    scale: [f64; 3],
    /// Output held: wing-tip brake, torque limiter, jam.
    pub held: Option<f64>,
    pub moment_ref: f64,
    pub j1: f64,
    pub j2: f64,
    pub restored: u32,
}

impl Drive {
    pub fn new(ratio: f64, out_inertia: f64, lo: f64, hi: f64, start: f64, moment_ref: f64, j1: f64, j2: f64) -> Self {
        Drive {
            ratio: ratio.max(1e-9),
            out_inertia: out_inertia.max(1e-6),
            lo,
            hi,
            x: [start.clamp(lo, hi), 0.0, 0.0],
            stepper: Stepper::new(3),
            scale: [1e-5 * (hi - lo).abs().max(1e-3), 0.5, 0.5],
            held: None,
            moment_ref: moment_ref.max(1.0),
            j1: j1.max(1e-6),
            j2: j2.max(1e-6),
            restored: 0,
        }
    }

    pub fn angle(&self) -> f64 {
        self.x[0]
    }

    pub fn rate(&self) -> f64 {
        self.ratio * (self.x[1] + self.x[2]) / 2.0
    }

    fn input_torque(motors: &[motor::Part], input: &Input, w: f64) -> (f64, f64) {
        match *input {
            Input::Hydraulic(k, io) => motors[k].motor.torque(&io, w),
            Input::Electric(e, cmd, braked) => (e.torque(cmd, w, braked), 0.0),
            Input::Locked => (-1e3 * w, 0.0),
        }
    }

    /// Steps under `load(angle, rate)`, the moment resisting increasing angle. Returns each input's torque and flow.
    pub fn step(&mut self, motors: &[motor::Part], inputs: [Input; 2], dt: f64, load: &dyn Fn(f64, f64) -> f64) -> [(f64, f64); 2] {
        let (r, io, lo, hi, held) = (self.ratio, self.out_inertia, self.lo, self.hi, self.held);
        let (j1, j2) = (self.j1, self.j2);
        let k_stop = 300.0 * self.moment_ref / (hi - lo).abs().max(1e-3);
        let c_stop = 2.0 * (k_stop * io).sqrt();
        let f = |s: &[f64], d: &mut [f64]| {
            let (t, w1, w2) = (s[0], s[1], s[2]);
            let rate = r * (w1 + w2) / 2.0;
            let (t1, _) = Self::input_torque(motors, &inputs[0], w1);
            let (t2, _) = Self::input_torque(motors, &inputs[1], w2);
            let mut l = load(t, rate);
            if t > hi {
                l += k_stop * (t - hi) + c_stop * rate;
            } else if t < lo {
                l += k_stop * (t - lo) + c_stop * rate;
            }
            if let Some(h) = held {
                l += 20.0 * k_stop * (t - h) + 20.0 * c_stop * rate;
            }
            // Unequal input inertias: split the differential with each side's own.
            let j = 0.5 * (j1 + j2);
            let (_, a1, a2) = differential(t1, t2, j, r, io, l);
            d[0] = rate;
            d[1] = a1;
            d[2] = a2;
        };
        if self.stepper.step(&mut self.x, dt, &self.scale, 1.0, f) == Outcome::Restored {
            self.restored += 1;
            self.x[1] = 0.0;
            self.x[2] = 0.0;
        }
        [Self::input_torque(motors, &inputs[0], self.x[1]), Self::input_torque(motors, &inputs[1], self.x[2])]
    }

    /// Output moment the load puts on the gearing at the current state.
    pub fn healthy(&self) -> bool {
        self.x.iter().all(|v| v.is_finite())
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend(self.x);
        v.push(self.held.unwrap_or(f64::NAN));
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        for v in &mut self.x {
            *v = it.next().unwrap_or(*v);
        }
        self.held = it.next().filter(|x| x.is_finite());
    }
}
