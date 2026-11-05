//! Switching and protection between buses: contactors with coils, travel time, latching, wear and welding;
//! thermal-magnetic breakers and fuses with ambient compensation and arc-fault detection; solid-state power
//! controllers; diodes with a forward drop.

use super::bus::{Bus, Islands};
use super::complex::{Abc, C, ZERO, ZERO3};
use super::grid::{Grid, Series, Shunt, Term};
use super::{eval, flag, put, yes, A, B, N, T, W, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

const BUSES: &[&str] = &["electrical.bus"];

pub const SPECS: [PartSpec; 4] = [
    PartSpec {
        domain: "electrical",
        kind: "contactor",
        help: "An electromechanical contactor between two buses: an optional coil fed from a bus with pull-in and drop-out voltages, travel time, latching, contact wear from breaking current, welding when closed into a fault, and one pole failing open.",
        fields: &[
            req("between", Kind::Refs(BUSES), "two bus ids"),
            opt("close_when", WHEN, "true", "when it closes; for a latching contactor, the set coil"),
            opt("close_delay", Kind::Quantity(Dim::TIME), "0 s", "how long close_when must hold before it closes; it opens at once"),
            opt("welded_when", WHEN, "false", "its contacts are welded shut"),
            opt("coil", Kind::Ref(BUSES), "", "bus its coil is fed from; none means the coil always has power"),
            opt("coil_power", Kind::Quantity(Dim::POWER), "12 W", "coil draw while energised"),
            opt("pull_in", Kind::Number, "0.6", "share of coil bus nominal that pulls it in"),
            opt("drop_out", Kind::Number, "0.3", "share of coil bus nominal below which it drops out"),
            opt("operate_time", Kind::Quantity(Dim::TIME), "0.02 s", "from coil energised to contacts made"),
            opt("release_time", Kind::Quantity(Dim::TIME), "0.01 s", "from coil dropped to contacts parted"),
            opt("latching", Kind::Bool, "false", "magnetically latched: close_when sets it, trip_when resets it, and it holds without coil power"),
            opt("trip_when", WHEN, "false", "latching contactor's reset coil"),
            opt("resistance", Kind::Quantity(Dim::RESISTANCE), "0.1 mohm", "contact resistance per pole when new"),
            opt("rating", Kind::Quantity(Dim::CURRENT), "0 A", "rated current per pole, for wear and welding; 0 skips both"),
            opt("life", Kind::Number, "50000", "breaks at rated current before the contacts are worn out"),
            opt("open_phase_when", WHEN, "false", "one pole (phase a) failed open"),
            opt("stuck_open_when", WHEN, "false", "jammed open: it will not close"),
            opt("phase", Kind::Text, "a", "phase a single-phase bus takes from a three-phase one"),
            opt("sync_check", Kind::Bool, "false", "AC paralleling: it only closes onto two live sides once their voltage, frequency and phase angle match"),
        ],
        outputs: CONTACTOR_OUT,
    },
    PartSpec {
        domain: "electrical",
        kind: "breaker",
        help: "A thermal-magnetic circuit breaker: carries 115 % of its rating indefinitely, trips on its heat curve or at once above its instant multiple, compensates for the bay temperature; pull it to reset. A fuse blows instead and stays open until replaced. An arc-fault breaker also trips on a series arc.",
        fields: &[
            req("between", Kind::Refs(BUSES), "two bus ids"),
            req("rating", Kind::Quantity(Dim::CURRENT), "for example \"10 A\""),
            opt("trip_time", Kind::Quantity(Dim::TIME), "3 s", "time to trip at twice its rating, from cold"),
            opt("instant", Kind::Number, "10", "multiple of its rating that trips it at once"),
            opt("fuse", Kind::Bool, "false", "a fuse: pulling it does not reset it"),
            opt("replaced_when", WHEN, "false", "a blown fuse is replaced on the rising edge of this"),
            opt("ambient", Kind::Expr(T), "298.15 K", "temperature around it: a cold breaker carries more before it trips"),
            opt("arc_fault", Kind::Bool, "false", "arc-fault circuit breaker: trips on a series arc's signature"),
            opt("arcing_when", WHEN, "false", "a series arc in the circuit it protects, from chafed or loose wiring"),
            opt("phase", Kind::Text, "a", "phase a single-phase bus takes from a three-phase one"),
        ],
        outputs: BREAKER_OUT,
    },
    PartSpec {
        domain: "electrical",
        kind: "diode",
        help: "Lets current through one way only, from the first bus to the second, less its forward drop.",
        fields: &[
            req("between", Kind::Refs(BUSES), "from bus, to bus"),
            opt("shorted_when", WHEN, "false", "failed short: conducts both ways"),
            opt("open_when", WHEN, "false", "failed open: conducts neither way"),
            opt("forward_drop", Kind::Quantity(Dim::VOLTAGE), "0.7 V", "voltage lost across it when it conducts"),
        ],
        outputs: DIODE_OUT,
    },
    PartSpec {
        domain: "electrical",
        kind: "sspc",
        help: "Solid-state power controller: a switched, current-limiting electronic breaker with an I²t trip curve, an instant trip, optional arc-fault detection and remote reset by cycling its command.",
        fields: &[
            req("between", Kind::Refs(BUSES), "two bus ids"),
            req("rating", Kind::Quantity(Dim::CURRENT), "for example \"5 A\""),
            opt("on_when", WHEN, "true", "its command; a trip resets when the command goes off"),
            opt("trip_time", Kind::Quantity(Dim::TIME), "0.5 s", "time to trip at twice its rating"),
            opt("instant", Kind::Number, "10", "multiple of rating that trips it inside a millisecond"),
            opt("current_limit", Kind::Number, "0", "multiple of rating it limits to while its trip curve runs; 0 for none"),
            opt("drop", Kind::Quantity(Dim::VOLTAGE), "0.1 V", "voltage lost across it at rated current"),
            opt("arc_fault", Kind::Bool, "false", "trips on a series arc's signature"),
            opt("arcing_when", WHEN, "false", "a series arc in its circuit"),
            opt("failed_closed_when", WHEN, "false", "its switch failed short: on whatever the command"),
            opt("failed_open_when", WHEN, "false", "its switch failed open"),
            opt("phase", Kind::Text, "a", "phase a single-phase bus takes from a three-phase one"),
        ],
        outputs: SSPC_OUT,
    },
];

outputs!(CONTACTOR_OUT, co { closed: B, current: A, failed: B, aux: B, wear: N, coil_powered: B, welded: B, in_sync: B, phase_difference: N });
outputs!(BREAKER_OUT, bo { closed: B, tripped: B, pulled: B, current: A, failed: B, heat: N, arc: B });
outputs!(DIODE_OUT, dio { conducting: B, current: A, failed: B, power: W });
outputs!(SSPC_OUT, so { closed: B, tripped: B, current: A, failed: B, power: W, status: N });

/// Thermal breakers carry this share of their rating indefinitely.
const BREAKER_HOLD: f64 = 1.15;
/// Bimetal opening temperature for ambient compensation.
const BIMETAL_K: f64 = 403.15;
const REF_K: f64 = 298.15;
/// Diode on-resistance and reverse leakage at each end.
const DIODE_OHM: f64 = 5e-3;
pub const DIODE_LEAK: f64 = 1e-3;
/// Voltage a series arc holds.
const ARC_V: f64 = 25.0;
/// Time for arc-fault detection to recognise an arc.
const ARC_DETECT_S: f64 = 0.1;

fn phase_index(s: &str) -> usize {
    match s {
        "b" => 1,
        "c" => 2,
        _ => 0,
    }
}

/// The conducting path of a link: per-phase admittance and emf between its two buses, and what flowed.
pub struct Path {
    pub a: usize,
    pub b: usize,
    pub ser: Series,
    pub ac: bool,
    pub y: Abc,
    pub e: Abc,
    pub i: Abc,
}

impl Path {
    /// None when the two buses cannot be joined (one DC, one AC).
    pub fn new(g: &mut Grid, buses: &[Bus], a: usize, b: usize, phase: &str) -> Option<Path> {
        let (ka, kb) = (buses[a].kind, buses[b].kind);
        if ka.ac() != kb.ac() {
            return None;
        }
        let ph = phase_index(phase);
        let (mut ta, mut tb): (Term, Term) = (buses[a].term, buses[b].term);
        if ta.phases() != tb.phases() {
            if ta.phases() == 3 {
                ta = ta.phase(ph);
            } else {
                tb = tb.phase(ph);
            }
        }
        Some(Path { a, b, ser: Series::new(g, ta, tb), ac: ka.ac(), y: ZERO3, e: ZERO3, i: ZERO3 })
    }

    pub fn from_part(g: &mut Grid, buses: &[Bus], bus_of: &dyn Fn(usize) -> Option<usize>, p: &CheckedPart) -> Option<Path> {
        let ends = p.parts("between");
        let (a, b) = (ends.first().and_then(|&x| bus_of(x))?, ends.get(1).and_then(|&x| bus_of(x))?);
        Path::new(g, buses, a, b, p.text("phase"))
    }

    pub fn phases(&self) -> usize {
        self.ser.phases()
    }

    pub fn conducts(&self) -> bool {
        self.y[..self.phases()].iter().any(|y| !y.is_zero())
    }

    pub fn set(&mut self, g: f64) {
        let n = self.phases();
        for k in 0..3 {
            self.y[k] = if k < n { C::real(g) } else { ZERO };
            self.e[k] = ZERO;
        }
    }

    pub fn open(&mut self) {
        self.y = ZERO3;
        self.e = ZERO3;
    }

    pub fn stamp(&self, g: &mut Grid) {
        if !self.conducts() {
            return;
        }
        self.ser.series(g, &self.y);
        if self.e.iter().any(|e| !e.is_zero()) {
            self.ser.emf(g, &self.y, &self.e);
        }
    }

    pub fn read(&mut self, g: &Grid) {
        self.i = if self.conducts() { self.ser.current(g, &self.y, &self.e) } else { ZERO3 };
    }

    /// Largest phase current; signed for DC.
    pub fn amps(&self) -> f64 {
        if self.ac {
            self.i[..self.phases()].iter().map(|c| c.abs()).fold(0.0, f64::max)
        } else {
            self.i[0].re
        }
    }

    /// Voltage across it, largest phase.
    pub fn across(&self, g: &Grid) -> f64 {
        let (va, vb) = (self.ser.va(g), self.ser.vb(g));
        (0..self.phases()).map(|k| (va[k] - vb[k]).abs()).fold(0.0, f64::max)
    }
}

pub struct Contactor {
    pub path: Path,
    base: usize,
    when: Option<Expr>,
    welded_e: Option<Expr>,
    trip: Option<Expr>,
    open_phase: Option<Expr>,
    stuck: Option<Expr>,
    delay: f64,
    held: f64,
    coil: Option<(usize, Shunt)>,
    coil_power: f64,
    pull_in: f64,
    drop_out: f64,
    operate: f64,
    release: f64,
    travel: f64,
    latching: bool,
    latched: bool,
    pub closed: bool,
    coil_on: bool,
    r0: f64,
    rating: f64,
    life: f64,
    wear: f64,
    welded: bool,
    just_closed: bool,
    last_amps: f64,
    sync: bool,
}

/// Sync-check limits: voltage within this share, slip under this many hertz, phase within this many degrees.
const SYNC_DV: f64 = 0.08;
const SYNC_DF: f64 = 0.5;
const SYNC_DEG: f64 = 12.0;

/// Whether two sides may be paralleled, and their phase difference in degrees.
fn sync_ok(g: &Grid, path: &Path, buses: &[Bus], isl: &Islands) -> (bool, f64) {
    let (ba, bb) = (&buses[path.a], &buses[path.b]);
    let live = |b: &Bus| b.vmag > 0.3 * b.nominal;
    if !path.ac || !live(ba) || !live(bb) || ba.island == bb.island {
        return (true, 0.0);
    }
    let (va, vb) = (path.ser.va(g)[0], path.ser.vb(g)[0]);
    let ta = isl.theta.get(ba.island).copied().unwrap_or(0.0);
    let tb = isl.theta.get(bb.island).copied().unwrap_or(0.0);
    let d = super::wrap(va.arg() + ta - vb.arg() - tb).to_degrees();
    let dv = (va.abs() - vb.abs()).abs() / ba.nominal.max(1e-9);
    let df = (isl.freq.get(ba.island).copied().unwrap_or(0.0) - isl.freq.get(bb.island).copied().unwrap_or(0.0)).abs();
    (dv < SYNC_DV && df < SYNC_DF && d.abs() < SYNC_DEG, d)
}

impl Contactor {
    pub fn new(p: &CheckedPart, path: Path, coil: Option<(usize, Shunt)>) -> Self {
        Contactor {
            path,
            base: p.slot,
            when: expr_of(p, "close_when"),
            welded_e: expr_of(p, "welded_when"),
            trip: expr_of(p, "trip_when"),
            open_phase: expr_of(p, "open_phase_when"),
            stuck: expr_of(p, "stuck_open_when"),
            delay: p.num("close_delay"),
            held: 0.0,
            coil,
            coil_power: p.num("coil_power").max(0.0),
            pull_in: p.num("pull_in"),
            drop_out: p.num("drop_out"),
            operate: p.num("operate_time").max(0.0),
            release: p.num("release_time").max(0.0),
            travel: 0.0,
            latching: p.flag("latching"),
            latched: false,
            closed: false,
            coil_on: false,
            r0: p.num("resistance").max(1e-7),
            rating: p.num("rating").max(0.0),
            life: p.num("life").max(1.0),
            wear: 0.0,
            welded: false,
            just_closed: false,
            last_amps: 0.0,
            sync: p.flag("sync_check"),
        }
    }

    pub fn update(&mut self, slots: &mut [f64], dt: f64, buses: &[Bus], g: &Grid, isl: &Islands) {
        let failed = slots[self.base + co::failed] != 0.0;
        let stuck = yes(&mut self.stuck, slots, dt, false);
        let coil_ok = match &self.coil {
            None => true,
            Some((b, _)) => {
                let bus = &buses[*b];
                let v = bus.vmag / bus.nominal.max(1e-9);
                if self.coil_on {
                    v >= self.drop_out
                } else {
                    v >= self.pull_in
                }
            }
        };
        let cmd = yes(&mut self.when, slots, dt, true);
        let want = if self.latching {
            let reset = yes(&mut self.trip, slots, dt, false);
            if coil_ok && reset {
                self.latched = false;
            } else if coil_ok && cmd {
                self.latched = true;
            }
            self.coil_on = coil_ok && (cmd || reset);
            self.latched
        } else {
            self.coil_on = coil_ok && cmd && !failed;
            self.coil_on
        };
        let want = want && !failed && !stuck;
        self.held = if want { self.held + dt } else { 0.0 };
        let (in_sync, diff) = sync_ok(g, &self.path, buses, isl);
        let target = want && self.held >= self.delay && (self.closed || !self.sync || in_sync);
        flag(slots, self.base + co::in_sync, in_sync);
        put(slots, self.base + co::phase_difference, diff);
        let was = self.closed;
        if target != self.closed {
            self.travel += dt;
            let need = if target { self.operate } else { self.release };
            if self.travel + 1e-9 >= need {
                self.closed = target;
                self.travel = 0.0;
            }
        } else {
            self.travel = 0.0;
        }
        if was && !self.closed && self.rating > 0.0 {
            // Each break under load burns the contacts a little; heavier current, much faster.
            self.wear = (self.wear + (self.last_amps / self.rating).powf(1.5) / self.life).min(1.0);
        }
        self.just_closed = !was && self.closed;
        if yes(&mut self.welded_e, slots, dt, false) {
            self.welded = true;
        }
        let shut = self.closed || self.welded;
        let r = self.r0 * (1.0 + 4.0 * self.wear);
        if shut {
            self.path.set(1.0 / r);
            if yes(&mut self.open_phase, slots, dt, false) {
                self.path.y[0] = ZERO;
            }
        } else {
            self.path.open();
        }
        flag(slots, self.base + co::closed, shut);
        flag(slots, self.base + co::aux, self.closed || self.welded);
    }

    pub fn stamp(&self, g: &mut Grid, buses: &[Bus]) {
        self.path.stamp(g);
        if let Some((b, sh)) = &self.coil {
            if self.coil_on && self.coil_power > 0.0 {
                let bus = &buses[*b];
                let n = bus.phases() as f64;
                sh.each(g, C::real(self.coil_power / n / (bus.nominal * bus.nominal).max(1e-9)));
            }
        }
    }

    pub fn post(&mut self, g: &Grid, slots: &mut [f64]) {
        self.path.read(g);
        let amps = self.path.amps();
        if self.just_closed && self.rating > 0.0 && amps.abs() > 12.0 * self.rating {
            // Closing into a fault draws an arc across bouncing contacts that can weld them.
            self.welded = true;
        }
        self.last_amps = amps.abs();
        put(slots, self.base + co::current, amps);
        put(slots, self.base + co::wear, self.wear);
        flag(slots, self.base + co::coil_powered, self.coil_on);
        flag(slots, self.base + co::welded, self.welded);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        let b = |x: bool| x as u8 as f64;
        v.extend([self.held, self.travel, b(self.latched), b(self.closed), b(self.coil_on), self.wear, b(self.welded), self.last_amps]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.held = n();
        self.travel = n();
        self.latched = n() != 0.0;
        self.closed = n() != 0.0;
        self.coil_on = n() != 0.0;
        self.wear = n();
        self.welded = n() != 0.0;
        self.last_amps = n();
    }
}

/// Thermal and arc state shared by breakers and power controllers.
#[derive(Clone, Copy, Default)]
struct Trip {
    heat: f64,
    arc_t: f64,
}

impl Trip {
    /// Integrates the I²t heat; true when it trips.
    fn heat(&mut self, amps: f64, hold: f64, trip_heat: f64, dt: f64) -> bool {
        let r = (amps / hold.max(1e-9)).powi(2);
        self.heat = (self.heat + (r - 1.0) * dt).max(0.0);
        self.heat > trip_heat
    }

    fn arc(&mut self, arcing: bool, detect: bool, dt: f64) -> bool {
        self.arc_t = if arcing { self.arc_t + dt } else { 0.0 };
        detect && self.arc_t >= ARC_DETECT_S
    }
}

/// Series arc: extra resistance holding roughly ARC_V at the current that flows.
fn arc_admittance(y: f64, amps: f64) -> f64 {
    if amps < 0.5 {
        return y;
    }
    1.0 / (1.0 / y + ARC_V / amps)
}

pub struct Breaker {
    pub path: Path,
    base: usize,
    rating: f64,
    trip_heat: f64,
    instant: f64,
    fuse: bool,
    replaced: Option<Expr>,
    replaced_was: bool,
    ambient: Option<Expr>,
    arc_fault: bool,
    arcing_e: Option<Expr>,
    arcing: bool,
    t: Trip,
    pub tripped: bool,
    hold: f64,
    last_amps: f64,
}

impl Breaker {
    pub fn new(p: &CheckedPart, path: Path) -> Self {
        Breaker {
            path,
            base: p.slot,
            rating: p.num("rating").max(1e-6),
            trip_heat: ((2.0 / BREAKER_HOLD).powi(2) - 1.0) * p.num("trip_time").max(0.01),
            instant: p.num("instant").max(1.0),
            fuse: p.flag("fuse"),
            replaced: expr_of(p, "replaced_when"),
            replaced_was: false,
            ambient: expr_of(p, "ambient"),
            arc_fault: p.flag("arc_fault"),
            arcing_e: expr_of(p, "arcing_when"),
            arcing: false,
            t: Trip::default(),
            tripped: false,
            hold: BREAKER_HOLD * p.num("rating").max(1e-6),
            last_amps: 0.0,
        }
    }

    pub fn update(&mut self, slots: &mut [f64], dt: f64) {
        let failed = slots[self.base + bo::failed] != 0.0;
        let replaced = yes(&mut self.replaced, slots, dt, false);
        if self.fuse && replaced && !self.replaced_was {
            self.tripped = false;
            self.t = Trip::default();
        }
        self.replaced_was = replaced;
        let amb = eval(&mut self.ambient, slots, dt, REF_K);
        // A bimetal set for 25 C bends later in the cold and sooner in a hot bay.
        let comp = ((BIMETAL_K - amb).max(10.0) / (BIMETAL_K - REF_K)).sqrt();
        self.hold = BREAKER_HOLD * self.rating * if self.fuse { 1.0 } else { comp };
        self.arcing = yes(&mut self.arcing_e, slots, dt, false);
        let pulled = slots[self.base + bo::pulled] != 0.0;
        let closed = !failed && !self.tripped && !pulled;
        if closed {
            let g = 1e4;
            let g = if self.arcing { arc_admittance(g, self.last_amps) } else { g };
            self.path.set(g);
        } else {
            self.path.open();
        }
        flag(slots, self.base + bo::closed, closed);
    }

    pub fn post(&mut self, g: &Grid, slots: &mut [f64], dt: f64) {
        self.path.read(g);
        let i = self.path.amps();
        let amps = i.abs();
        self.last_amps = amps;
        if self.t.heat(amps, self.hold, self.trip_heat, dt) || amps > self.instant * self.rating {
            self.tripped = true;
        }
        let arc_seen = self.t.arc(self.arcing && amps > 0.5, self.arc_fault, dt);
        if arc_seen {
            self.tripped = true;
        }
        if self.tripped && !self.fuse && slots[self.base + bo::pulled] != 0.0 {
            self.tripped = false;
            self.t = Trip::default();
        }
        put(slots, self.base + bo::current, i);
        flag(slots, self.base + bo::tripped, self.tripped);
        put(slots, self.base + bo::heat, (self.t.heat / self.trip_heat).min(1.0));
        flag(slots, self.base + bo::arc, arc_seen || (self.arcing && self.arc_fault && self.tripped));
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        let b = |x: bool| x as u8 as f64;
        v.extend([self.t.heat, self.t.arc_t, b(self.tripped), b(self.replaced_was), self.last_amps]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.t.heat = n();
        self.t.arc_t = n();
        self.tripped = n() != 0.0;
        self.replaced_was = n() != 0.0;
        self.last_amps = n();
    }
}

pub struct Diode {
    pub path: Path,
    base: usize,
    shorted: Option<Expr>,
    open_e: Option<Expr>,
    drop: f64,
    conducting: bool,
    leak: [Shunt; 2],
}

impl Diode {
    pub fn new(p: &CheckedPart, path: Path, g: &mut Grid, buses: &[Bus]) -> Self {
        let leak = [Shunt::new(g, buses[path.a].term), Shunt::new(g, buses[path.b].term)];
        Diode { path, base: p.slot, shorted: expr_of(p, "shorted_when"), open_e: expr_of(p, "open_when"), drop: p.num("forward_drop").max(0.0), conducting: false, leak }
    }

    pub fn update(&mut self, g: &Grid, slots: &mut [f64], dt: f64) {
        let failed = slots[self.base + dio::failed] != 0.0;
        let short = yes(&mut self.shorted, slots, dt, false);
        let open = yes(&mut self.open_e, slots, dt, false);
        let (va, vb) = (self.path.ser.va(g)[0].re, self.path.ser.vb(g)[0].re);
        let forward = va - vb;
        let ok = !failed && !open;
        self.conducting = ok && if self.conducting { self.path.i[0].re >= -1e-5 } else { forward > self.drop + 1e-3 };
        if short {
            self.path.set(1.0 / DIODE_OHM);
        } else if self.conducting {
            self.path.set(1.0 / DIODE_OHM);
            self.path.e[0] = C::real(self.drop);
        } else {
            self.path.open();
        }
        flag(slots, self.base + dio::conducting, self.conducting || short);
    }

    pub fn stamp(&self, g: &mut Grid) {
        for l in &self.leak {
            l.each(g, C::real(DIODE_LEAK));
        }
        self.path.stamp(g);
    }

    pub fn post(&mut self, g: &Grid, slots: &mut [f64]) {
        self.path.read(g);
        let i = self.path.amps();
        put(slots, self.base + dio::current, i);
        put(slots, self.base + dio::power, (i * self.path.across(g)).abs());
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.push(self.conducting as u8 as f64);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        self.conducting = it.next().unwrap_or(0.0) != 0.0;
    }
}

pub struct Sspc {
    pub path: Path,
    base: usize,
    rating: f64,
    trip_heat: f64,
    instant: f64,
    limit: f64,
    g_on: f64,
    on: Option<Expr>,
    arc_fault: bool,
    arcing_e: Option<Expr>,
    failed_closed: Option<Expr>,
    failed_open: Option<Expr>,
    t: Trip,
    tripped: bool,
    cmd_was: bool,
    g_now: f64,
    status: f64,
    arcing: bool,
    last_amps: f64,
}

impl Sspc {
    pub fn new(p: &CheckedPart, path: Path) -> Self {
        let rating = p.num("rating").max(1e-6);
        let g_on = rating / p.num("drop").max(1e-4);
        Sspc {
            path,
            base: p.slot,
            rating,
            trip_heat: 3.0 * p.num("trip_time").max(1e-3),
            instant: p.num("instant").max(1.0),
            limit: p.num("current_limit").max(0.0),
            g_on,
            on: expr_of(p, "on_when"),
            arc_fault: p.flag("arc_fault"),
            arcing_e: expr_of(p, "arcing_when"),
            failed_closed: expr_of(p, "failed_closed_when"),
            failed_open: expr_of(p, "failed_open_when"),
            t: Trip::default(),
            tripped: false,
            cmd_was: false,
            g_now: g_on,
            status: 0.0,
            arcing: false,
            last_amps: 0.0,
        }
    }

    pub fn update(&mut self, g: &Grid, slots: &mut [f64], dt: f64) {
        let failed = slots[self.base + so::failed] != 0.0;
        let cmd = yes(&mut self.on, slots, dt, true);
        if !cmd && self.cmd_was {
            self.tripped = false;
            self.t = Trip::default();
        }
        self.cmd_was = cmd;
        let shorted = yes(&mut self.failed_closed, slots, dt, false);
        let open = yes(&mut self.failed_open, slots, dt, false);
        self.arcing = yes(&mut self.arcing_e, slots, dt, false);
        let closed = shorted || (cmd && !self.tripped && !failed && !open);
        if closed {
            // In current limit the switch runs linear, its resistance rising to hold the limit.
            let across = self.path.across(g);
            let cap = if self.limit > 0.0 && !shorted && across > 1e-3 { self.limit * self.rating / across } else { f64::INFINITY };
            self.g_now = self.g_on.min(cap.max(1e-3));
            let y = if self.arcing { arc_admittance(self.g_now, self.last_amps) } else { self.g_now };
            self.path.set(y);
        } else {
            self.path.open();
        }
        self.status = if shorted || failed || open {
            3.0
        } else if self.tripped {
            2.0
        } else if closed {
            1.0
        } else {
            0.0
        };
        flag(slots, self.base + so::closed, closed);
    }

    /// After a solve: over its limit, it raises its resistance to hold the limit. True when it changed.
    pub fn limit(&mut self, g: &Grid) -> bool {
        if self.limit <= 0.0 || self.status != 1.0 || !self.path.conducts() {
            return false;
        }
        let i = self.path.ser.current(g, &self.path.y, &self.path.e);
        let amps = (0..self.path.phases()).map(|k| i[k].abs()).fold(0.0, f64::max);
        let cap = self.limit * self.rating;
        if amps <= cap * 1.01 {
            return false;
        }
        let across = amps / self.g_now.max(1e-9);
        self.g_now = (cap / across.max(1e-6)).min(self.g_on);
        let y = if self.arcing { arc_admittance(self.g_now, self.last_amps) } else { self.g_now };
        self.path.set(y);
        true
    }

    pub fn post(&mut self, g: &Grid, slots: &mut [f64], dt: f64) {
        self.path.read(g);
        let i = self.path.amps();
        let amps = i.abs();
        self.last_amps = amps;
        let shorted = self.status == 3.0 && self.path.conducts();
        if !shorted && self.path.conducts() {
            if self.t.heat(amps, self.rating, self.trip_heat, dt) || amps > self.instant * self.rating {
                self.tripped = true;
            }
            if self.t.arc(self.arcing && amps > 0.2, self.arc_fault, dt) {
                self.tripped = true;
            }
        }
        let loss = if self.path.conducts() { amps * amps / self.g_now.max(1e-9) } else { 0.0 };
        put(slots, self.base + so::current, i);
        flag(slots, self.base + so::tripped, self.tripped);
        put(slots, self.base + so::power, loss);
        put(slots, self.base + so::status, self.status);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        let b = |x: bool| x as u8 as f64;
        v.extend([self.t.heat, self.t.arc_t, b(self.tripped), b(self.cmd_was), self.g_now, self.last_amps]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.t.heat = n();
        self.t.arc_t = n();
        self.tripped = n() != 0.0;
        self.cmd_was = n() != 0.0;
        self.g_now = n();
        self.last_amps = n();
    }
}
