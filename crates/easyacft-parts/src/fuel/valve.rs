//! Fuel lines and valves. A line is a manifold node holding a little fuel, with its temperature and the water it
//! carries, a leak overboard and vapour that forms when its pressure falls below the fuel's vapour pressure. A
//! valve is a motorised or check valve whose drop goes as flow squared, with travel time, a motor that needs
//! power, a stuck motor, seat leakage when closed, and position switches that disagree with the command.

use super::{flag, put, yes, B, MF, N, P, T, WHEN};
use crate::hydraulic::net::Net;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const LINE: PartSpec = PartSpec {
    domain: "fuel",
    kind: "line",
    help: "A fuel line or manifold: the fuel in it, its temperature and water, a leak overboard, and vapour when its pressure falls below the fuel's vapour pressure.",
    fields: &[
        opt("leak_when", WHEN, "false", "a leak overboard"),
        opt("leak_rate", Kind::Quantity(Dim::MASS_FLOW), "0.2 kg/s", "that leak at 1 bar"),
        opt("volume", Kind::Quantity(Dim::VOLUME), "5 L", "fuel the line and manifold hold"),
        opt("ambient", Kind::Expr(P), "101325 Pa", "outside pressure, the reference for the line's pressure"),
    ],
    outputs: LINE_OUT,
};

pub const VALVE: PartSpec = PartSpec {
    domain: "fuel",
    kind: "valve",
    help: "A crossfeed, transfer, shutoff or gravity valve; a tank end gives gravity feed. Its drop goes as flow squared; a motorised valve travels, needs power, can stick and leak past its seat, and its position switches can disagree with its command.",
    fields: &[
        req("between", Kind::Refs(&["fuel.tank", "fuel.line"]), "two line or tank ids"),
        opt("open_when", WHEN, "true", "when it is commanded open"),
        opt("flow", Kind::Quantity(Dim::MASS_FLOW), "5 kg/s", "flow at 1 bar across it"),
        opt("open_time", Kind::Quantity(Dim::TIME), "0 s", "motor travel time"),
        opt("stuck_when", WHEN, "false", "its motor has failed where it is"),
        opt("check", Kind::Bool, "false", "a check valve: flow only from the first to the second"),
        opt("powered_when", WHEN, "true", "its motor has power; without it, it stays where it is"),
        opt("leak_when", WHEN, "false", "worn seat: it passes fuel when shut"),
    ],
    outputs: VALVE_OUT,
};

outputs!(LINE_OUT, lo { pressure: P, temperature: T, leak_flow: MF, failed: B, vapour: B, water: N });
outputs!(VALVE_OUT, vo { open: B, position: N, flow: MF, failed: B, closed: B, disagree: B, in_transit: B });

/// Bulk modulus of jet fuel, Pa.
const FUEL_BULK: f64 = 1.4e9;
const BAR: f64 = 1e5;

pub struct Line {
    base: usize,
    pub node: usize,
    leak_e: Option<Expr>,
    leak_g: f64,
    pub leaking: bool,
    pub temp: f64,
    pub water: f64,
    sum_leak: f64,
    n_sub: f64,
    pub vapour: bool,
}

impl Line {
    pub fn new(p: &CheckedPart, net: &mut Net, density: f64) -> Self {
        let node = net.node(p.num("volume").max(1e-5) / FUEL_BULK);
        Line { base: p.slot, node, leak_e: expr_of(p, "leak_when"), leak_g: p.num("leak_rate") / density / BAR, leaking: false, temp: 288.15, water: 0.0, sum_leak: 0.0, n_sub: 0.0, vapour: false }
    }

    pub fn begin(&mut self, slots: &[f64], dt: f64) {
        self.leaking = slots[self.base + lo::failed] != 0.0 || yes(&mut self.leak_e, slots, dt, false);
        self.sum_leak = 0.0;
        self.n_sub = 0.0;
    }

    pub fn stamp(&self, net: &mut Net) {
        if self.leaking {
            net.stamp(self.node, self.leak_g, 0.0);
        }
    }

    /// Volume flow lost overboard this substep.
    pub fn after(&mut self, net: &Net) -> f64 {
        let q = if self.leaking { self.leak_g * net.p[self.node].max(0.0) } else { 0.0 };
        self.sum_leak += q;
        self.n_sub += 1.0;
        q
    }

    pub fn finish(&mut self, net: &Net, slots: &mut [f64], density: f64, vapour_p: f64, ambient: f64) {
        let p = net.p[self.node];
        self.vapour = p + ambient < vapour_p;
        let b = self.base;
        put(slots, b + lo::pressure, p.max(0.0));
        put(slots, b + lo::temperature, self.temp);
        put(slots, b + lo::leak_flow, self.sum_leak / self.n_sub.max(1.0) * density);
        flag(slots, b + lo::vapour, self.vapour);
        put(slots, b + lo::water, self.water * 1e6);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.temp, self.water]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        self.temp = it.next().unwrap_or(288.15);
        self.water = it.next().unwrap_or(0.0);
    }
}

pub struct Valve {
    base: usize,
    pub link: usize,
    pub ends: [usize; 2],
    /// Volume flow at 1 bar when fully open.
    q_rated: f64,
    when: Option<Expr>,
    open_time: f64,
    stuck: Option<Expr>,
    powered: Option<Expr>,
    seat_leak: Option<Expr>,
    pub check: bool,
    pub position: f64,
    commanded: bool,
    leaking: bool,
    disagree_t: f64,
    sum_q: f64,
    n_sub: f64,
    rho: f64,
}

impl Valve {
    pub fn new(p: &CheckedPart, net: &mut Net, a: usize, b: usize, density: f64) -> Self {
        let q_rated = p.num("flow").max(1e-9) / density;
        let check = p.flag("check");
        let link = net.link(a, b, BAR / q_rated, 1e6, check);
        Valve {
            base: p.slot,
            link,
            ends: [a, b],
            q_rated,
            when: expr_of(p, "open_when"),
            open_time: p.num("open_time").max(0.0),
            stuck: expr_of(p, "stuck_when"),
            powered: expr_of(p, "powered_when"),
            seat_leak: expr_of(p, "leak_when"),
            check,
            position: 0.0,
            commanded: false,
            leaking: false,
            disagree_t: 0.0,
            sum_q: 0.0,
            n_sub: 0.0,
            rho: density,
        }
    }

    pub fn begin(&mut self, slots: &mut [f64], dt: f64, rho: f64) {
        self.rho = rho;
        let failed = slots[self.base + vo::failed] != 0.0;
        self.commanded = !failed && yes(&mut self.when, slots, dt, true);
        let stuck = yes(&mut self.stuck, slots, dt, false);
        let powered = yes(&mut self.powered, slots, dt, true);
        self.leaking = yes(&mut self.seat_leak, slots, dt, false);
        if !stuck && powered {
            let target = if self.commanded { 1.0 } else { 0.0 };
            self.position = if self.open_time > 0.0 { self.position + (target - self.position).clamp(-dt / self.open_time, dt / self.open_time) } else { target };
        }
        let at = if self.commanded { self.position > 0.98 } else { self.position < 0.02 };
        self.disagree_t = if at { 0.0 } else { self.disagree_t + dt };
        self.sum_q = 0.0;
        self.n_sub = 0.0;
    }

    /// Flow coefficient this substep: drop as flow squared, linearised at last flow; a shut valve may leak.
    /// `dry` says whether each end is a tank down to its unusable fuel, which nothing can draw from.
    pub fn stamp(&self, net: &mut Net, dry: [bool; 2]) {
        let [a, b] = self.ends;
        let dp = net.p[a] - net.p[b];
        if (dry[0] && dp >= 0.0) || (dry[1] && dp <= 0.0) {
            let l = &mut net.links[self.link];
            l.open = false;
            l.q = 0.0;
            return;
        }
        let open_share = if self.position > 1e-3 { self.position } else if self.leaking { 0.02 } else { 0.0 };
        let l = &mut net.links[self.link];
        if open_share <= 0.0 {
            l.open = false;
            l.q = 0.0;
            return;
        }
        if !self.check {
            l.open = true;
        }
        // Area goes with position; at rated flow the drop is a bar, and it goes as flow squared.
        let q_full = self.q_rated * open_share;
        let qn = l.q.abs().max(0.05 * q_full);
        l.r = BAR * qn / (q_full * q_full);
    }

    pub fn after(&mut self, net: &Net) {
        self.sum_q += net.links[self.link].q;
        self.n_sub += 1.0;
    }

    /// Mean volume flow from the first end to the second.
    pub fn flow(&self) -> f64 {
        self.sum_q / self.n_sub.max(1.0)
    }

    pub fn finish(&self, slots: &mut [f64]) {
        let b = self.base;
        let open = self.position > 1e-3;
        flag(slots, b + vo::open, open && (!self.check || self.flow() > 1e-9));
        put(slots, b + vo::position, self.position);
        put(slots, b + vo::flow, self.flow().abs() * self.rho);
        flag(slots, b + vo::closed, self.position < 0.02);
        flag(slots, b + vo::disagree, self.disagree_t > self.open_time.max(0.5) * 2.0 + 1.0);
        flag(slots, b + vo::in_transit, self.position > 0.02 && self.position < 0.98);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.position, self.disagree_t]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        self.position = it.next().unwrap_or(0.0);
        self.disagree_t = it.next().unwrap_or(0.0);
    }
}
