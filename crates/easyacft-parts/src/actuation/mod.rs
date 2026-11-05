//! Actuation: hydraulic, electro-hydrostatic and electrical-backup actuators and motors, and the mechanisms they
//! drive: flight control surfaces, the trimmable stabiliser, flaps and slats, landing gear legs and doors, wheel
//! steering, and the braked wheels. Each mechanism's motion and its actuators' chamber pressures and valve
//! positions are integrated together by an implicit stepper, so a 1.5 GPa fluid in a litre of chamber steps at
//! the frame rate; the hydraulic circuits see the flow each actuator draws and the fluid it holds.

pub mod drive;
pub mod feel;
pub mod fluid;
pub mod high_lift;
pub mod leg;
pub mod linear;
pub mod mechanism;
pub mod motor;
pub mod ode;
pub mod sensors;
pub mod stabilizer;
pub mod steering;
pub mod surface;
pub mod valve;
pub mod wheel;

use crate::Domain;
use easyacft_lang::schema::PartSpec;
use easyacft_lang::{Checked, CheckedPart};
use linear::STATES;
use ode::{Outcome, Stepper};
use std::collections::HashMap;

pub static SPECS: &[PartSpec] = &[linear::SPEC, motor::SPEC, surface::SPEC, stabilizer::SPEC, high_lift::SPEC, feel::SPEC, leg::LEG, leg::DOOR, leg::BOGIE, steering::SPEC, wheel::SPEC];

/// Steps between kept healthy states, for rolling back after a blowup.
const CHECKPOINT_EVERY: u32 = 50;

/// An actuator no mechanism lists: it drives a plain mass against its own load expression.
struct Free {
    k: usize,
    x: Vec<f64>,
    scale: Vec<f64>,
    stepper: Stepper,
}

pub struct Actuation {
    acts: Vec<linear::Part>,
    motors: Vec<motor::Part>,
    doors: Vec<leg::Door>,
    bogies: Vec<leg::Bogie>,
    legs: Vec<leg::Leg>,
    surfaces: Vec<surface::Surface>,
    stabilizers: Vec<stabilizer::Stabilizer>,
    high_lift: Vec<high_lift::HighLift>,
    steering: Vec<steering::Steering>,
    wheels: Vec<wheel::Wheel>,
    feel: Vec<feel::Feel>,
    free: Vec<Free>,
    outputs: Vec<(usize, usize)>,
    checkpoint: Vec<f64>,
    slots_checkpoint: Vec<(usize, Vec<f64>)>,
    since: u32,
    pub recoveries: u32,
}

fn ours(p: &CheckedPart) -> bool {
    SPECS.iter().any(|s| s.domain == p.domain && s.kind == p.kind)
}

impl Actuation {
    pub fn new(c: &Checked) -> Option<Self> {
        if !c.parts.iter().any(ours) {
            return None;
        }
        let mut acts = Vec::new();
        let mut act_index = HashMap::new();
        let mut motors = Vec::new();
        let mut motor_index = HashMap::new();
        for (i, p) in c.parts.iter().enumerate() {
            match (p.domain, p.kind) {
                ("actuator", "linear") => {
                    act_index.insert(i, acts.len());
                    acts.push(linear::Part::new(c, p, i, 0.1));
                }
                ("actuator", "motor") => {
                    motor_index.insert(i, motors.len());
                    motors.push(motor::Part::new(c, p));
                }
                _ => {}
            }
        }
        let mut a = Actuation {
            acts,
            motors,
            doors: vec![],
            bogies: vec![],
            legs: vec![],
            surfaces: vec![],
            stabilizers: vec![],
            high_lift: vec![],
            steering: vec![],
            wheels: vec![],
            feel: vec![],
            free: vec![],
            outputs: vec![],
            checkpoint: vec![],
            slots_checkpoint: vec![],
            since: 0,
            recoveries: 0,
        };
        let mut door_index = HashMap::new();
        for (i, p) in c.parts.iter().enumerate() {
            if (p.domain, p.kind) == ("landing_gear", "door") {
                door_index.insert(i, a.doors.len());
                a.doors.push(leg::Door::new(c, p, &mut a.acts, &act_index));
            }
        }
        let mut bogie_index = HashMap::new();
        for (i, p) in c.parts.iter().enumerate() {
            if (p.domain, p.kind) == ("landing_gear", "bogie") {
                bogie_index.insert(i, a.bogies.len());
                a.bogies.push(leg::Bogie::new(c, p, &mut a.acts, &act_index));
            }
        }
        for p in &c.parts {
            match (p.domain, p.kind) {
                ("landing_gear", "leg") => a.legs.push(leg::Leg::new(c, p, &mut a.acts, &act_index, &door_index, &bogie_index)),
                ("flight_control", "surface") => a.surfaces.push(surface::Surface::new(c, p, &mut a.acts, &act_index)),
                ("flight_control", "stabilizer") => a.stabilizers.push(stabilizer::Stabilizer::new(c, p, &a.motors, &motor_index)),
                ("flight_control", "high_lift") => a.high_lift.push(high_lift::HighLift::new(c, p, &a.motors, &motor_index)),
                ("steering", "unit") => a.steering.push(steering::Steering::new(c, p, &mut a.acts, &act_index)),
                ("brakes", "wheel") => a.wheels.push(wheel::Wheel::new(c, p)),
                ("flight_control", "feel") => a.feel.push(feel::Feel::new(c, p)),
                _ => {}
            }
        }
        let mut used = vec![false; a.acts.len()];
        for k in a.doors.iter().flat_map(|d| d.rot.acts.iter()).chain(a.legs.iter().flat_map(|l| l.rot.acts.iter())).chain(a.bogies.iter().flat_map(|b| b.rot.acts.iter())).chain(a.surfaces.iter().flat_map(|s| s.rot.acts.iter())).chain(a.steering.iter().flat_map(|s| s.rot.acts.iter())) {
            used[*k] = true;
        }
        for (k, u) in used.iter().enumerate() {
            if !u {
                let act = &a.acts[k].act;
                let n = 2 + STATES;
                let mut x = vec![0.0; n];
                let mut scale = vec![1.0; n];
                x[0] = 0.5 * act.stroke;
                scale[0] = 1e-4 * act.stroke;
                scale[1] = 1e-3 * act.rate;
                act.initial(101_325.0 + 50.0 * 6894.757, &mut x[2..]);
                act.scales(&mut scale[2..]);
                a.free.push(Free { k, x, scale, stepper: Stepper::new(n) });
            }
        }
        for p in c.parts.iter().filter(|p| ours(p)) {
            if let Some(spec) = SPECS.iter().find(|s| s.domain == p.domain && s.kind == p.kind) {
                a.outputs.push((p.slot, spec.outputs.len()));
            }
        }
        Some(a)
    }

    fn step_free(&mut self, slots: &mut [f64], dt: f64) {
        for f in &mut self.free {
            let part = &mut self.acts[f.k];
            let io = part.prepare(slots, dt, None);
            let load = part.ev(4, slots, dt, 0.0);
            let act = &part.act;
            let mass = act.mass;
            let jam = io.jammed;
            let rhs = |s: &[f64], d: &mut [f64]| {
                let (x, v) = (s[0], s[1]);
                let out = act.eval(&io, x, v, &s[2..], &mut d[2..]);
                let force = out.force - load + act.stroke_stop(x, v, mass);
                d[0] = if jam { 0.0 } else { v };
                d[1] = if jam { -v * 100.0 } else { force / mass };
            };
            if f.stepper.step(&mut f.x, dt, &f.scale, 1.0, rhs) == Outcome::Restored {
                self.recoveries += 1;
            }
            let floor = act.fluid.vapour();
            f.x[2] = f.x[2].max(floor);
            f.x[3] = f.x[3].max(floor);
            let mut scratch = [0.0; STATES];
            let out = act.eval(&io, f.x[0], f.x[1], &f.x[2..], &mut scratch);
            let st = f.x[2..].to_vec();
            part.finish(slots, &io, f.x[0], f.x[1], &st, &out, dt);
        }
    }

    fn all_state(&self) -> Vec<f64> {
        let mut v = Vec::new();
        self.acts.iter().for_each(|a| a.state(&mut v));
        self.motors.iter().for_each(|m| v.push(m.valve));
        self.doors.iter().for_each(|d| d.state(&mut v));
        self.bogies.iter().for_each(|b| b.state(&mut v));
        self.legs.iter().for_each(|l| l.state(&mut v));
        self.surfaces.iter().for_each(|s| s.state(&mut v));
        self.stabilizers.iter().for_each(|s| s.state(&mut v));
        self.high_lift.iter().for_each(|h| h.state(&mut v));
        self.steering.iter().for_each(|s| s.state(&mut v));
        self.wheels.iter().for_each(|w| w.state(&mut v));
        self.feel.iter().for_each(|f| f.state(&mut v));
        for f in &self.free {
            v.extend_from_slice(&f.x);
        }
        v
    }

    fn all_restore(&mut self, s: &[f64]) {
        let mut it = s.iter().copied();
        self.acts.iter_mut().for_each(|a| a.restore(&mut it));
        self.motors.iter_mut().for_each(|m| m.valve = it.next().unwrap_or(0.0));
        self.doors.iter_mut().for_each(|d| d.restore(&mut it));
        self.bogies.iter_mut().for_each(|b| b.restore(&mut it));
        self.legs.iter_mut().for_each(|l| l.restore(&mut it));
        self.surfaces.iter_mut().for_each(|x| x.restore(&mut it));
        self.stabilizers.iter_mut().for_each(|x| x.restore(&mut it));
        self.high_lift.iter_mut().for_each(|x| x.restore(&mut it));
        self.steering.iter_mut().for_each(|x| x.restore(&mut it));
        self.wheels.iter_mut().for_each(|x| x.restore(&mut it));
        self.feel.iter_mut().for_each(|x| x.restore(&mut it));
        for f in &mut self.free {
            for v in &mut f.x {
                *v = it.next().unwrap_or(*v);
            }
        }
    }

    fn healthy(&self, slots: &[f64]) -> bool {
        self.outputs.iter().all(|&(o, n)| slots[o..o + n].iter().all(|v| v.is_finite()))
            && self.surfaces.iter().all(|s| s.rot.healthy())
            && self.legs.iter().all(|l| l.rot.healthy())
            && self.doors.iter().all(|d| d.rot.healthy())
            && self.bogies.iter().all(|b| b.rot.healthy())
            && self.steering.iter().all(|s| s.rot.healthy())
            && self.stabilizers.iter().all(|s| s.drive.healthy())
            && self.high_lift.iter().all(|h| h.drive.healthy())
    }
}

impl Domain for Actuation {
    fn name(&self) -> &'static str {
        "actuation"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        for d in &mut self.doors {
            d.step(&mut self.acts, slots, dt);
        }
        for b in &mut self.bogies {
            b.step(&mut self.acts, slots, dt);
        }
        for l in &mut self.legs {
            l.step(&mut self.acts, &mut self.doors, &self.bogies, slots, dt);
        }
        for s in &mut self.surfaces {
            s.step(&mut self.acts, slots, dt);
        }
        for s in &mut self.stabilizers {
            s.step(&mut self.motors, slots, dt);
        }
        for h in &mut self.high_lift {
            h.step(&mut self.motors, slots, dt);
        }
        for s in &mut self.steering {
            s.step(&mut self.acts, slots, dt);
        }
        self.step_free(slots, dt);
        for w in &mut self.wheels {
            w.step(slots, dt);
        }
        for f in &mut self.feel {
            f.step(slots, dt);
        }
        // Blowup guard: anything non-finite rolls back to the last healthy state, outputs and all.
        if self.healthy(slots) {
            self.since += 1;
            if self.since >= CHECKPOINT_EVERY || self.checkpoint.is_empty() {
                self.since = 0;
                self.checkpoint = self.all_state();
                self.slots_checkpoint = self.outputs.iter().map(|&(o, n)| (o, slots[o..o + n].to_vec())).collect();
            }
        } else {
            self.recoveries += 1;
            let cp = std::mem::take(&mut self.checkpoint);
            self.all_restore(&cp);
            self.checkpoint = cp;
            for (o, v) in &self.slots_checkpoint {
                slots[*o..*o + v.len()].copy_from_slice(v);
            }
        }
    }

    fn state(&self) -> Vec<f64> {
        self.all_state()
    }

    fn restore(&mut self, s: &[f64]) {
        self.all_restore(s);
    }
}
