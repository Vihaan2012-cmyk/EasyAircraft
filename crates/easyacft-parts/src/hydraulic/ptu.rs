//! Power transfer unit: a hydraulic motor and pump on one shaft between two circuits. The shaft turns when the
//! torque one side's pressure makes beats the other's plus breakout friction; its inertia and stiction make it
//! run in bursts when it cannot keep up, the bark a PTU makes on the ground. It moves power, never fluid.

use super::circuit::Circuit;
use super::net::Net;
use super::{flag, put, yes, B, N, Q, TQ, W, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "hydraulic",
    kind: "ptu",
    help: "A power transfer unit: a motor and a pump on one shaft between two circuits; the higher side drives the lower once its torque beats breakout friction, the shaft's inertia and stiction make it run in bursts when it cannot keep up. It moves power, never fluid.",
    fields: &[
        req("between", Kind::Refs(&["hydraulic.circuit"]), "two circuit ids"),
        opt("threshold", Kind::Quantity(Dim::PRESSURE), "500 psi", "pressure difference that starts it: its breakout friction"),
        req("flow", Kind::Quantity(Dim::VOLUME_FLOW), "most flow it delivers"),
        opt("efficiency", Kind::Number, "0.8", "power out per power in"),
        opt("enabled_when", WHEN, "true", "when it is armed"),
        opt("rated_rpm", Kind::Quantity(Dim::FREQUENCY), "3000 rpm", "shaft speed at its most flow"),
        opt("inertia", Kind::Number, "0.002", "shaft and rotating group inertia, kg m2"),
        opt("displacement_ratio", Kind::Number, "1", "second circuit's displacement over the first's"),
    ],
    outputs: OUT,
};

outputs!(OUT, to { running: B, flow: Q, power: W, failed: B, speed: N, torque: TQ, starts: N });

pub struct Ptu {
    base: usize,
    pub a: usize,
    pub b: usize,
    when: Option<Expr>,
    /// Displacement per radian of each side, m3/rad.
    da: f64,
    db: f64,
    breakout: f64,
    w_max: f64,
    j: f64,
    eta: f64,
    w: f64,
    armed: bool,
    starts: f64,
    was: bool,
    sum_flow: f64,
    sum_power: f64,
    sum_torque: f64,
    n_sub: f64,
    pub heat_a: f64,
    pub heat_b: f64,
}

impl Ptu {
    pub fn new(p: &CheckedPart, a: usize, b: usize) -> Self {
        let w_max = (p.num("rated_rpm") * std::f64::consts::TAU).max(1.0);
        let da = p.num("flow").max(1e-7) / w_max;
        let db = da * p.num("displacement_ratio").max(0.05);
        Ptu {
            base: p.slot,
            a,
            b,
            when: expr_of(p, "enabled_when"),
            da,
            db,
            breakout: da * p.num("threshold").max(1.0),
            w_max,
            j: p.num("inertia").max(1e-5),
            eta: p.num("efficiency").clamp(0.05, 1.0).sqrt(),
            w: 0.0,
            armed: false,
            starts: 0.0,
            was: false,
            sum_flow: 0.0,
            sum_power: 0.0,
            sum_torque: 0.0,
            n_sub: 0.0,
            heat_a: 0.0,
            heat_b: 0.0,
        }
    }

    pub fn begin(&mut self, slots: &[f64], dt: f64) {
        self.armed = slots[self.base + to::failed] == 0.0 && yes(&mut self.when, slots, dt, true);
        self.sum_flow = 0.0;
        self.sum_power = 0.0;
        self.sum_torque = 0.0;
        self.n_sub = 0.0;
        self.heat_a = 0.0;
        self.heat_b = 0.0;
    }

    /// One substep: shaft torque balance, then the flows each side sees.
    pub fn stamp(&mut self, net: &mut Net, ca: &Circuit, cb: &Circuit, h: f64) {
        let (pa, pb) = (net.p[ca.node].max(0.0), net.p[cb.node].max(0.0));
        // Positive speed: side a motoring, side b pumping.
        let drive = self.da * pa - self.db * pb;
        let moving = self.w.abs() > 1e-3;
        // Churning and seal drag, rising steeply past rated speed so an unloaded pump side cannot run away.
        let over = (self.w.abs() / self.w_max - 1.0).max(0.0);
        let viscous = 2e-4 * self.w * ca.visc.max(cb.visc).sqrt() + self.breakout * 20.0 * over * self.w.signum();
        let torque = if !self.armed {
            -self.w * self.j / h.max(1e-6)
        } else if moving {
            // Running friction is lower than breakout, as with any seal.
            drive - 0.6 * self.breakout * self.w.signum() - viscous
        } else if drive.abs() > self.breakout {
            drive - self.breakout * drive.signum()
        } else {
            0.0
        };
        let w_new = self.w + torque / self.j * h;
        // Stiction catches it as it passes through zero.
        self.w = if moving && w_new.signum() != self.w.signum() { 0.0 } else { w_new.clamp(-1.2 * self.w_max, 1.2 * self.w_max) };
        let w = self.w;
        // Motor side takes more than its displacement, pump side gives less: the losses.
        let (qa, qb) = if w >= 0.0 { (self.da * w / self.eta, -self.db * w * self.eta) } else { (self.da * w * self.eta, -self.db * w / self.eta) };
        net.stamp(ca.node, 0.0, -qa);
        net.stamp(cb.node, 0.0, -qb);
        let power_in = (qa * pa).max(0.0) + (qb * pb).max(0.0);
        let power_out = (-qa * pa).max(0.0) + (-qb * pb).max(0.0);
        self.sum_flow += if w >= 0.0 { qb.abs() } else { qa.abs() };
        self.sum_power += power_out;
        self.sum_torque += torque;
        let loss = (power_in - power_out).max(0.0);
        self.heat_a += 0.5 * loss;
        self.heat_b += 0.5 * loss;
        self.n_sub += 1.0;
    }

    pub fn finish(&mut self, slots: &mut [f64]) {
        let n = self.n_sub.max(1.0);
        let running = self.w.abs() > 0.02 * self.w_max;
        if running && !self.was {
            self.starts += 1.0;
        }
        self.was = running;
        let b = self.base;
        flag(slots, b + to::running, running);
        put(slots, b + to::flow, self.sum_flow / n);
        put(slots, b + to::power, self.sum_power / n);
        put(slots, b + to::speed, self.w / self.w_max);
        put(slots, b + to::torque, self.sum_torque / n);
        put(slots, b + to::starts, self.starts);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.w, self.starts, self.was as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.w = n();
        self.starts = n();
        self.was = n() != 0.0;
    }
}
