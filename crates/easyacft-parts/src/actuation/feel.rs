//! An artificial feel and trim unit for a mechanically signalled control: a spring cartridge with breakout and a
//! gradient that can follow dynamic pressure (Q-feel, as elevator feel computers schedule it), a viscous damper, a
//! trim actuator that moves the cartridge's neutral at its motor's rate, and the control circuit's own friction.
//! Gives the force the pilot feels and where the control rests when let go.

use super::ode::{soft_sign, Stepper};
use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const F: Type = Type::Num(Dim::FORCE);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub static SPEC: PartSpec = PartSpec {
    domain: "flight_control",
    kind: "feel",
    help: "Artificial feel and trim: spring cartridge with breakout, gradient optionally scheduled with dynamic pressure, damper, control friction, an electric trim actuator moving the neutral point, and a pilot force input.",
    fields: &[
        opt("pilot_force", Kind::Expr(F), "0 N", "force the pilot applies, positive toward positive travel"),
        opt("trim", Kind::Expr(N), "0", "trim switch: 1 toward positive, -1 toward negative, 0 off"),
        opt("trim_rate", Kind::Number, "0.05", "neutral point travel per second, share of full travel"),
        opt("trim_powered_when", WHEN, "true", "trim motor has power"),
        opt("trim_reset_when", WHEN, "false", "drives the neutral back to centre"),
        opt("air", Kind::Ref(&["environment.air"]), "", "for Q-feel"),
        opt("breakout", Kind::Quantity(Dim::FORCE), "40 N", "force to move it off its neutral"),
        opt("gradient", Kind::Quantity(Dim::FORCE), "300 N", "force at full travel beyond breakout, at the reference dynamic pressure"),
        opt("q_feel", Kind::Bool, "false", "gradient follows dynamic pressure"),
        opt("reference_q", Kind::Quantity(Dim::PRESSURE), "10 kPa", "dynamic pressure the gradient is rated at"),
        opt("min_gradient", Kind::Number, "0.3", "Q-feel floor, share of the rated gradient"),
        opt("max_gradient", Kind::Number, "2.5", "Q-feel ceiling"),
        opt("damping", Kind::Number, "60", "N per unit travel per second"),
        opt("friction", Kind::Quantity(Dim::FORCE), "15 N", "control circuit friction"),
        opt("mass", Kind::Quantity(Dim::MASS), "8 kg", "control and linkage mass at the pilot's hand"),
        opt("travel", Kind::Quantity(Dim::LENGTH), "0.12 m", "full travel each way at the pilot's hand"),
        opt("jam_when", WHEN, "false", "control circuit jammed"),
        opt("trim_runaway_when", WHEN, "false", "trim motor runs on its own toward positive"),
        opt("spring_broken_when", WHEN, "false", "feel spring broken: no centring"),
    ],
    outputs: &[("position", N), ("force", F), ("neutral", N), ("gradient", F), ("trimming", B), ("jammed", B), ("failed", B)],
};

pub struct Feel {
    e: [Option<Expr>; 7],
    q: Option<usize>,
    breakout: f64,
    gradient: f64,
    q_feel: bool,
    q_ref: f64,
    g_min: f64,
    g_max: f64,
    damping: f64,
    friction: f64,
    mass: f64,
    travel: f64,
    trim_rate: f64,
    x: [f64; 2],
    neutral: f64,
    stepper: Stepper,
    s: Slots,
}

impl Feel {
    pub fn new(c: &Checked, p: &CheckedPart) -> Self {
        Feel {
            e: ["pilot_force", "trim", "trim_powered_when", "trim_reset_when", "jam_when", "trim_runaway_when", "spring_broken_when"].map(|k| expr_of(p, k)),
            q: p.part("air").and_then(|i| c.slots.index.get(&format!("{}.dynamic_pressure", c.parts[i].id)).copied()),
            breakout: p.num("breakout").max(0.0),
            gradient: p.num("gradient").max(1.0),
            q_feel: p.flag("q_feel"),
            q_ref: p.num("reference_q").max(100.0),
            g_min: p.num("min_gradient").max(0.01),
            g_max: p.num("max_gradient").max(p.num("min_gradient")),
            damping: p.num("damping").max(0.0),
            friction: p.num("friction").max(0.0),
            mass: p.num("mass").max(0.1),
            travel: p.num("travel").max(1e-3),
            trim_rate: p.num("trim_rate").max(0.0),
            x: [0.0, 0.0],
            neutral: 0.0,
            stepper: Stepper::new(2),
            s: Slots::of(p, &["position", "force", "neutral", "gradient", "trimming", "jammed", "failed"]),
        }
    }

    /// Spring force at travel share `x` about the neutral: breakout first, then the gradient.
    fn spring(&self, x: f64, gradient: f64) -> f64 {
        let d = x - self.neutral;
        let breakout = self.breakout * soft_sign(d, 0.01);
        -(breakout + gradient * d)
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let ev = |e: &mut Option<Expr>, s: &[f64]| e.as_mut().map_or(0.0, |x| x.eval(s, dt));
        let pilot = ev(&mut self.e[0], slots);
        let trim = ev(&mut self.e[1], slots).clamp(-1.0, 1.0);
        let powered = self.e[2].as_mut().is_none_or(|x| x.yes(slots, dt));
        let reset = ev(&mut self.e[3], slots) != 0.0;
        let jam = ev(&mut self.e[4], slots) != 0.0 || self.s.on(slots, "failed");
        let runaway = ev(&mut self.e[5], slots) != 0.0;
        let broken = ev(&mut self.e[6], slots) != 0.0;
        let q = self.q.map_or(self.q_ref, |i| slots[i]).max(0.0);
        let gradient = if self.q_feel { self.gradient * (q / self.q_ref).clamp(self.g_min, self.g_max) } else { self.gradient };
        let drive = if runaway {
            1.0
        } else if reset {
            -self.neutral.signum() * (self.neutral.abs() > 0.005) as u8 as f64
        } else {
            trim
        };
        let trimming = powered && drive != 0.0;
        if trimming {
            self.neutral = (self.neutral + drive * self.trim_rate * dt).clamp(-1.0, 1.0);
        }
        let g = if broken { 0.0 } else { gradient };
        let spring_b = if broken { 0.0 } else { 1.0 };
        let (m, c, fr, travel) = (self.mass, self.damping, self.friction, self.travel);
        let mut stepper = std::mem::take(&mut self.stepper);
        let me = &*self;
        let mut x = self.x;
        let f = |s: &[f64], d: &mut [f64]| {
            let (p, v) = (s[0], s[1]);
            let spring = me.spring(p, g) * spring_b;
            let stop = if p.abs() > 1.0 { -(p.abs() - 1.0) * p.signum() * 50.0 * me.gradient.max(1000.0) - v * 200.0 } else { 0.0 };
            let force = pilot + spring + stop - c * v - fr * soft_sign(v, 0.01);
            d[0] = if jam { 0.0 } else { v };
            d[1] = if jam { -v * 100.0 } else { force / (m * travel) };
        };
        stepper.step(&mut x, dt, &[1e-4, 1e-3], 1.0, f);
        self.stepper = stepper;
        if x.iter().all(|v| v.is_finite()) {
            self.x = x;
        }
        let felt = -self.spring(self.x[0], g) * spring_b;
        let s = &self.s;
        s.put(slots, "position", self.x[0]);
        s.put(slots, "force", felt);
        s.put(slots, "neutral", self.neutral);
        s.put(slots, "gradient", g);
        s.flag(slots, "trimming", trimming);
        s.flag(slots, "jammed", jam);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend(self.x);
        v.push(self.neutral);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        for v in &mut self.x {
            *v = it.next().unwrap_or(*v);
        }
        self.neutral = it.next().unwrap_or(0.0);
    }
}
