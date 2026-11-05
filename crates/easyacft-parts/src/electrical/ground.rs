//! Ground power: a supply behind its cable's impedance, and the external power monitor that only declares it
//! available while its voltage, frequency and phase sequence are good.

use super::bus::{Bus, Islands};
use super::complex::{power, Abc, C, A as OP_A, A2, ZERO, ZERO3};
use super::grid::{Grid, Shunt};
use super::{eval, flag, put, yes, A, B, HZ, N, V, W, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "electrical",
    kind: "ext_power",
    help: "Ground power behind its cable's impedance, with an external power monitor: available only while plugged in with voltage, frequency and phase sequence in limits for its monitor time. Faults: a bad unit's voltage, reversed phase sequence, a lost phase.",
    fields: &[
        req("bus", Kind::Ref(&["electrical.bus"]), "the bus at its output"),
        req("voltage", Kind::Quantity(Dim::VOLTAGE), "supply voltage, phase to neutral for AC"),
        opt("frequency", Kind::Quantity(Dim::FREQUENCY), "0 Hz", "0 for DC"),
        req("rating", Kind::Quantity(Dim::POWER), "supply rating, apparent power"),
        opt("power_factor", Kind::Number, "1", "real power it delivers per VA of rating"),
        opt("connected_when", WHEN, "false", "when the ground unit is plugged in"),
        opt("regulation", Kind::Number, "0.02", "voltage lost across unit and cable at rated load"),
        opt("x_r", Kind::Number, "1", "cable reactance over resistance at 400 Hz"),
        opt("voltage_scale", Kind::Expr(N), "1", "share of its voltage the unit actually delivers, for a bad unit"),
        opt("phase_reversed_when", WHEN, "false", "plugged in with the phase sequence reversed"),
        opt("phase_lost_when", WHEN, "false", "one phase (a) lost"),
        opt("monitor_time", Kind::Quantity(Dim::TIME), "0 s", "how long its quality must be good before it is available"),
    ],
    outputs: OUT,
};

outputs!(OUT, eo { voltage: V, frequency: HZ, current: A, load: W, available: B, failed: B, reactive_power: W, fault: N });

pub struct ExtPower {
    base: usize,
    pub bus: usize,
    sh: Shunt,
    pub ac: bool,
    n: usize,
    v: f64,
    pub frequency: f64,
    pub rating: f64,
    pub power_factor: f64,
    z: C,
    when: Option<Expr>,
    scale: Option<Expr>,
    reversed: Option<Expr>,
    lost: Option<Expr>,
    monitor: f64,
    ok_t: f64,
    pub on: bool,
    pub available: bool,
    is_reversed: bool,
    is_lost: bool,
    vs: f64,
    pub theta: f64,
    y: Abc,
    e: Abc,
}

impl ExtPower {
    pub fn new(p: &CheckedPart, bus: usize, g: &mut Grid, buses: &[Bus]) -> Self {
        let b = &buses[bus];
        let ac = b.kind.ac() && p.num("frequency") > 0.0;
        let n = if ac { b.phases() } else { 1 };
        let term = if ac { b.term } else { b.term.phase(0) };
        let v = p.num("voltage").max(1e-3);
        let rating = p.num("rating").max(1.0);
        let zmag = p.num("regulation").max(1e-4) * v * v / (rating / n as f64);
        let xr = if ac { p.num("x_r").max(0.0) } else { 0.0 };
        let r = zmag / (1.0 + xr * xr).sqrt();
        ExtPower {
            base: p.slot,
            bus,
            sh: Shunt::new(g, term),
            ac,
            n,
            v,
            frequency: p.num("frequency"),
            rating,
            power_factor: p.num("power_factor").clamp(0.05, 1.0),
            z: C::new(r, r * xr),
            when: expr_of(p, "connected_when"),
            scale: expr_of(p, "voltage_scale"),
            reversed: expr_of(p, "phase_reversed_when"),
            lost: expr_of(p, "phase_lost_when"),
            monitor: p.num("monitor_time").max(0.0),
            ok_t: 0.0,
            on: false,
            available: false,
            is_reversed: false,
            is_lost: false,
            vs: 1.0,
            theta: 0.0,
            y: ZERO3,
            e: ZERO3,
        }
    }

    pub fn update(&mut self, slots: &mut [f64], dt: f64) {
        let failed = slots[self.base + eo::failed] != 0.0;
        self.on = !failed && yes(&mut self.when, slots, dt, false);
        self.vs = eval(&mut self.scale, slots, dt, 1.0).max(0.0);
        self.is_reversed = self.ac && self.n == 3 && yes(&mut self.reversed, slots, dt, false);
        self.is_lost = self.ac && self.n == 3 && yes(&mut self.lost, slots, dt, false);
        self.theta = (self.theta + std::f64::consts::TAU * self.frequency * dt).rem_euclid(std::f64::consts::TAU);
        let fault = if !self.on {
            0.0
        } else if self.vs > 1.087 {
            1.0
        } else if self.vs < if self.ac { 0.87 } else { 0.85 } {
            2.0
        } else if self.is_lost {
            8.0
        } else if self.is_reversed {
            9.0
        } else {
            0.0
        };
        let good = self.on && fault == 0.0;
        self.ok_t = if good { self.ok_t + dt } else { 0.0 };
        self.available = good && self.ok_t >= self.monitor;
        flag(slots, self.base + eo::available, self.available);
        put(slots, self.base + eo::fault, fault);
    }

    pub fn offer(&self, isl: &mut Islands, buses: &[Bus]) {
        if !self.on {
            return;
        }
        let i = buses[self.bus].island;
        isl.capacity[i] += self.rating * self.power_factor;
        isl.has_capacity[i] = true;
        isl.source_va[i] += self.rating;
        if self.ac {
            isl.offer_reference(i, self.rating, self.frequency, self.theta);
        }
    }

    pub fn stamp(&mut self, g: &mut Grid, buses: &[Bus], isl: &Islands) {
        self.y = ZERO3;
        self.e = ZERO3;
        if !self.on {
            return;
        }
        let em = self.v * self.vs;
        if self.ac {
            let f = buses[self.bus].freq;
            let y = C::new(self.z.re, self.z.im * f / 400.0).inv();
            let delta = super::wrap(self.theta - isl.theta[buses[self.bus].island]);
            let a = C::polar(em, delta);
            let set: Abc = if self.n == 3 {
                if self.is_reversed {
                    [a, a * OP_A, a * A2]
                } else {
                    [a, a * A2, a * OP_A]
                }
            } else {
                [a, ZERO, ZERO]
            };
            for k in 0..self.n {
                self.y[k] = y;
                self.e[k] = set[k];
            }
            if self.is_lost {
                self.y[0] = ZERO;
                self.e[0] = ZERO;
            }
        } else {
            self.y[0] = self.z.inv();
            self.e[0] = C::real(em);
        }
        self.sh.diag(g, &self.y);
        let mut inj = ZERO3;
        for k in 0..self.n {
            inj[k] = self.y[k] * self.e[k];
        }
        self.sh.inject(g, &inj);
    }

    pub fn post(&mut self, g: &Grid, slots: &mut [f64]) {
        let v = self.sh.v(g);
        let mut i = ZERO3;
        for k in 0..self.n {
            i[k] = self.y[k] * (self.e[k] - v[k]);
        }
        let s = power(&v, &i, self.n);
        let b = self.base;
        let vm = if self.ac { (0..self.n).map(|k| v[k].abs()).sum::<f64>() / self.n as f64 } else { v[0].re.max(0.0) };
        put(slots, b + eo::voltage, if self.on { vm } else { 0.0 });
        put(slots, b + eo::frequency, if self.on { self.frequency } else { 0.0 });
        put(slots, b + eo::current, if self.ac { (0..self.n).map(|k| i[k].abs()).sum::<f64>() / self.n as f64 } else { i[0].re });
        put(slots, b + eo::load, s.re);
        put(slots, b + eo::reactive_power, s.im);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.ok_t, self.theta]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        self.ok_t = it.next().unwrap_or(0.0);
        self.theta = it.next().unwrap_or(0.0);
    }
}
