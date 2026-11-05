//! Buses, the faults that can sit on them, their power quality, and the islands closed links join them into.

use super::complex::{mean_abs, to_sequence, Abc, C, ZERO3};
use super::grid::{Grid, Shunt, Term};
use super::{flag, put, yes, A, B, HZ, N, SEC, V, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "electrical",
    kind: "bus",
    help: "A busbar. DC, three-phase AC (phase to neutral) or single-phase AC; an AC bus takes the frequency of the source feeding it. Faults to structure or between phases, bolted or arcing.",
    fields: &[
        req("nominal", Kind::Quantity(Dim::VOLTAGE), "for example \"28 V\", or \"115 V\" phase to neutral"),
        opt("powered_at", Kind::Number, "0.8", "share of nominal counted as powered"),
        opt("type", Kind::Text, "auto", "auto, dc, ac or ac_single; auto is AC when an AC source, inverter, transformer or TRU input can reach it"),
        opt("short_when", WHEN, "false", "a short: fault current flows until something upstream trips"),
        opt("fault_resistance", Kind::Quantity(Dim::RESISTANCE), "0.01 ohm", "resistance of that short"),
        opt("fault_type", Kind::Text, "abc", "abc (all phases to structure), a, b or c (one phase), ab, bc or ca (between phases)"),
        opt("fault_arcing", Kind::Bool, "false", "an arcing fault: about 30 V of arc in the fault path, so less current than a bolted one"),
    ],
    outputs: OUT,
};

outputs!(OUT, o { voltage: V, frequency: HZ, powered: B, failed: B, voltage_a: V, voltage_b: V, voltage_c: V, unbalance: N, thd: N, outage: SEC, fault_current: A });

/// Arc voltage of an arcing fault.
const ARC_V: f64 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BusKind {
    Dc,
    Ac3,
    Ac1,
}

impl BusKind {
    pub fn ac(self) -> bool {
        self != BusKind::Dc
    }
    pub fn phases(self) -> usize {
        if self == BusKind::Ac3 {
            3
        } else {
            1
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum FaultType {
    Ground([bool; 3]),
    Between(usize, usize),
}

pub struct Bus {
    pub base: usize,
    pub nominal: f64,
    pub powered_at: f64,
    pub kind: BusKind,
    pub term: Term,
    shunt: Shunt,
    short: Option<Expr>,
    fault_g: f64,
    fault: FaultType,
    arcing: bool,
    pub shorted: bool,
    fault_amps: f64,
    pub v: Abc,
    pub vmag: f64,
    pub powered: bool,
    pub failed: bool,
    pub outage: f64,
    pub island: usize,
    /// Frequency reactances on it see this step.
    pub freq: f64,
}

pub fn declared_kind(p: &CheckedPart) -> Option<BusKind> {
    match p.text("type") {
        "dc" => Some(BusKind::Dc),
        "ac" => Some(BusKind::Ac3),
        "ac_single" => Some(BusKind::Ac1),
        _ => None,
    }
}

impl Bus {
    pub fn new(p: &CheckedPart, kind: BusKind, g: &mut Grid) -> Self {
        let term = g.unknowns(kind.phases());
        let fault = match p.text("fault_type") {
            "a" => FaultType::Ground([true, false, false]),
            "b" => FaultType::Ground([false, true, false]),
            "c" => FaultType::Ground([false, false, true]),
            "ab" => FaultType::Between(0, 1),
            "bc" => FaultType::Between(1, 2),
            "ca" => FaultType::Between(2, 0),
            _ => FaultType::Ground([true; 3]),
        };
        Bus {
            base: p.slot,
            nominal: p.num("nominal"),
            powered_at: p.num("powered_at"),
            kind,
            term,
            shunt: Shunt::new(g, term),
            short: expr_of(p, "short_when"),
            fault_g: 1.0 / p.num("fault_resistance").max(1e-6),
            fault,
            arcing: p.flag("fault_arcing"),
            shorted: false,
            fault_amps: 0.0,
            v: ZERO3,
            vmag: 0.0,
            powered: false,
            failed: false,
            outage: 0.0,
            island: 0,
            freq: 400.0,
        }
    }

    pub fn phases(&self) -> usize {
        self.kind.phases()
    }

    pub fn pre(&mut self, slots: &[f64], dt: f64) {
        self.failed = slots[self.base + o::failed] != 0.0;
        self.shorted = !self.failed && yes(&mut self.short, slots, dt, false);
    }

    fn fault_y(&self) -> f64 {
        if !self.arcing || self.fault_amps < 1.0 {
            return self.fault_g;
        }
        // The arc holds roughly constant voltage: it adds ARC_V / I of resistance at the current that flowed.
        1.0 / (1.0 / self.fault_g + ARC_V / self.fault_amps)
    }

    pub fn stamp(&self, g: &mut Grid) {
        if self.failed {
            // A failed bus is grounded hard: nothing beyond it is fed and it carries no voltage.
            self.shunt.each(g, C::real(easyacft_core::STIFF * 1e-3));
            return;
        }
        if !self.shorted {
            return;
        }
        let y = C::real(self.fault_y());
        match (self.fault, self.kind) {
            (_, BusKind::Dc) | (_, BusKind::Ac1) => self.shunt.each(g, y),
            (FaultType::Ground(ph), _) => {
                let mut d = ZERO3;
                for k in 0..3 {
                    if ph[k] {
                        d[k] = y;
                    }
                }
                self.shunt.diag(g, &d);
            }
            (FaultType::Between(a, b), _) => self.shunt.across(g, a, b, y),
        }
    }

    /// Reads the solution and writes the bus's outputs; `freq` and `thd` come from its island.
    pub fn post(&mut self, g: &Grid, slots: &mut [f64], dt: f64, freq: f64, thd: f64) {
        self.v = g.v(&self.term);
        let n = self.phases();
        let mut mag = [0.0; 3];
        for k in 0..n {
            mag[k] = if self.kind.ac() { self.v[k].abs() } else { self.v[k].re.max(0.0) };
        }
        self.vmag = mag[..n].iter().sum::<f64>() / n as f64;
        if !self.vmag.is_finite() {
            self.vmag = 0.0;
        }
        // Voltage sensing drops out 3 % below where it picks up, so a load sagging its bus cannot chatter.
        let at = self.powered_at * self.nominal * if self.powered { 0.97 } else { 1.0 };
        let powered = !self.failed && self.vmag >= at;
        if powered {
            if !self.powered {
                self.outage = 0.0;
            }
        } else {
            self.outage += dt;
        }
        self.powered = powered;
        self.fault_amps = if self.shorted {
            let y = self.fault_y();
            match (self.fault, self.kind) {
                (FaultType::Between(a, b), BusKind::Ac3) => (self.v[a] - self.v[b]).abs() * y,
                (FaultType::Ground(ph), BusKind::Ac3) => (0..3).filter(|&k| ph[k]).map(|k| self.v[k].abs() * y).fold(0.0, f64::max),
                _ => mean_abs(&self.v, n) * y,
            }
        } else {
            0.0
        };
        let unbalance = if self.kind == BusKind::Ac3 && self.vmag > 1.0 {
            let s = to_sequence(&self.v);
            s[2].abs() / s[1].abs().max(1e-9)
        } else {
            0.0
        };
        let b = self.base;
        put(slots, b + o::voltage, self.vmag);
        put(slots, b + o::frequency, if powered && self.kind.ac() { freq } else { 0.0 });
        flag(slots, b + o::powered, powered);
        put(slots, b + o::voltage_a, mag[0]);
        put(slots, b + o::voltage_b, if n == 3 { mag[1] } else { 0.0 });
        put(slots, b + o::voltage_c, if n == 3 { mag[2] } else { 0.0 });
        put(slots, b + o::unbalance, unbalance);
        put(slots, b + o::thd, if powered && self.kind.ac() { thd } else { 0.0 });
        put(slots, b + o::outage, self.outage);
        put(slots, b + o::fault_current, self.fault_amps);
    }
}

/// Union-find islands of buses joined by conducting links.
#[derive(Default)]
pub struct Islands {
    pub of: Vec<usize>,
    pub count: usize,
    pub ac: Vec<bool>,
    pub freq: Vec<f64>,
    /// Absolute phase of the island's reference source, which every AC phasor in the island is measured from.
    pub theta: Vec<f64>,
    pub ref_rating: Vec<f64>,
    pub capacity: Vec<f64>,
    pub demand: Vec<f64>,
    pub has_capacity: Vec<bool>,
    pub nonlinear: Vec<f64>,
    pub source_va: Vec<f64>,
    /// Real load and rating of the generators on line in each island, for load sharing.
    pub gen_p: Vec<f64>,
    pub gen_s: Vec<f64>,
    pub gen_n: Vec<f64>,
    parent: Vec<usize>,
}

impl Islands {
    pub fn rebuild(&mut self, buses: &[Bus], links: &[(usize, usize)]) {
        let n = buses.len();
        self.parent.clear();
        self.parent.extend(0..n);
        fn find(p: &mut [usize], mut x: usize) -> usize {
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        for &(a, b) in links {
            let (ra, rb) = (find(&mut self.parent, a), find(&mut self.parent, b));
            if ra != rb {
                self.parent[ra.max(rb)] = ra.min(rb);
            }
        }
        let mut id = vec![usize::MAX; n];
        self.of.clear();
        let mut count = 0;
        for i in 0..n {
            let r = find(&mut self.parent, i);
            if id[r] == usize::MAX {
                id[r] = count;
                count += 1;
            }
            self.of.push(id[r]);
        }
        self.count = count;
        self.ac.clear();
        self.ac.resize(count, false);
        for (i, b) in buses.iter().enumerate() {
            if b.kind.ac() {
                self.ac[self.of[i]] = true;
            }
        }
    }

    /// Clears the per-step sums.
    pub fn begin(&mut self) {
        let c = self.count;
        for v in [&mut self.freq, &mut self.theta, &mut self.ref_rating, &mut self.capacity, &mut self.demand, &mut self.nonlinear, &mut self.source_va, &mut self.gen_p, &mut self.gen_s, &mut self.gen_n] {
            v.clear();
            v.resize(c, 0.0);
        }
        self.has_capacity.clear();
        self.has_capacity.resize(c, false);
    }

    /// An AC source joins its island: the largest becomes the island's reference.
    pub fn offer_reference(&mut self, isl: usize, rating: f64, freq: f64, theta: f64) {
        if rating > self.ref_rating[isl] {
            self.ref_rating[isl] = rating;
            self.freq[isl] = freq;
            self.theta[isl] = theta;
        }
    }

    /// Frequency for reactances in an island, falling back to 400 Hz in a dead one.
    pub fn reactance_freq(&self, isl: usize) -> f64 {
        let f = self.freq.get(isl).copied().unwrap_or(0.0);
        if f > 1.0 {
            f
        } else {
            400.0
        }
    }

    /// Rough voltage distortion: nonlinear load against the source capacity feeding it.
    pub fn thd(&self, isl: usize) -> f64 {
        let s = self.source_va.get(isl).copied().unwrap_or(0.0);
        if s <= 0.0 {
            0.0
        } else {
            (0.25 * self.nonlinear[isl] / s).min(0.4)
        }
    }
}
