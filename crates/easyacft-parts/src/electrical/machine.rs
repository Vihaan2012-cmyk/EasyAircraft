//! Generators. An AC generator is a salient-pole synchronous machine in the two-axis form: transient and
//! subtransient flux on the direct axis, transient on the quadrature axis, its internal emf behind the
//! subtransient reactance in the network, separate negative and zero sequence reactances so an unbalanced load
//! or fault sees the right impedance, and saturation. A brushless exciter fed by a permanent-magnet generator
//! drives the field under a PI voltage regulator; the field ceiling sets the sustained fault current.
//! Constant-frequency drives have a governor and rotor inertia, so paralleled machines share load through their
//! rotor angles and an unsynchronised close draws real circulating current. A DC generator is a shunt machine
//! behind its armature resistance with a reverse-current cut-out, and can run as a starter.

use super::bus::{Bus, Islands};
use super::complex::{balanced, from_sequence_diag, mul3, power, to_sequence, Abc, C, M3, ZERO, ZERO3};
use super::gcu::{Gcu, Limits, Measure};
use super::grid::{Grid, Shunt};
use super::{eval, flag, put, yes, A, B, HZ, N, T, TQ, V, W, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "electrical",
    kind: "generator",
    help: "A generator and its control unit. AC: a synchronous machine with transient and subtransient reactances on both axes, saturation, a PMG-fed exciter under a PI voltage regulator, constant or variable frequency, governor and rotor inertia for paralleling. DC: a shunt machine with a reverse-current cut-out that can also crank as a starter. Protection: over/under-voltage, over/under-frequency, overspeed, overcurrent, overload, differential, open phase, unbalance and loss of excitation; drive disconnect and drive oil temperature.",
    fields: &[
        req("bus", Kind::Ref(&["electrical.bus"]), "the bus at its output"),
        req("voltage", Kind::Quantity(Dim::VOLTAGE), "regulated voltage, phase to neutral for AC"),
        opt("frequency", Kind::Quantity(Dim::FREQUENCY), "0 Hz", "0 for DC; for a variable-frequency generator, the frequency at rated drive speed"),
        req("rating", Kind::Quantity(Dim::POWER), "apparent power, for example \"90 kVA\""),
        opt("power_factor", Kind::Number, "1", "real power it delivers per VA of rating; 0.75 to 1 for aircraft AC generators"),
        opt("driven_when", WHEN, "true", "when its drive turns it and its control switch is on"),
        opt("speed", Kind::Expr(N), "1", "drive speed as a share of rated"),
        opt("variable_frequency", Kind::Bool, "false", "frequency follows drive speed, as on a VFG or a starter-generator"),
        opt("min_speed", Kind::Number, "0.5", "below this share of rated speed it goes off line"),
        opt("max_speed", Kind::Number, "1.2", "above this for 1 s it trips for overfrequency"),
        opt("excitation_time", Kind::Quantity(Dim::TIME), "0.2 s", "from turning fast enough to ready, while the field builds"),
        opt("regulator_time", Kind::Quantity(Dim::TIME), "0.5 s", "integral time of its voltage regulator's trim on what remains of the error"),
        opt("regulator_gain", Kind::Number, "4", "field forcing gain: extra field per unit of transient flux short of what the load needs"),
        opt("protection", Kind::Bool, "true", "its control unit trips it on a fault"),
        opt("overload_time", Kind::Quantity(Dim::TIME), "300 s", "how long it carries 150 % before the overload trip"),
        opt("reset_when", WHEN, "false", "a trip clears on the rising edge of this, or when it stops being driven; a differential lockout only on this"),
        opt("disconnect_when", WHEN, "false", "disconnects the drive for good"),
        opt("oil_ambient", Kind::Expr(T), "333.15 K", "drive oil temperature off load, after its cooler"),
        opt("oil_rise", Kind::Quantity(Dim::TEMPERATURE), "60 K", "oil temperature rise at full load"),
        opt("oil_limit", Kind::Quantity(Dim::TEMPERATURE), "458.15 K", "oil overheat warning"),
        opt("oil_cooled_when", WHEN, "true", "its oil cooler works"),
        opt("xd", Kind::Number, "1.2", "direct-axis synchronous reactance, per unit"),
        opt("xd_transient", Kind::Number, "0.2", "direct-axis transient reactance, per unit"),
        opt("xd_subtransient", Kind::Number, "0.12", "subtransient reactance, per unit"),
        opt("xq", Kind::Number, "0.8", "quadrature-axis synchronous reactance, per unit"),
        opt("x2", Kind::Number, "0.14", "negative-sequence reactance, per unit"),
        opt("x0", Kind::Number, "0.06", "zero-sequence reactance, per unit"),
        opt("armature_resistance", Kind::Number, "0.01", "per unit"),
        opt("field_time", Kind::Quantity(Dim::TIME), "0.3 s", "open-circuit direct-axis transient time constant"),
        opt("subtransient_time", Kind::Quantity(Dim::TIME), "0.02 s", "open-circuit subtransient time constant"),
        opt("quadrature_time", Kind::Quantity(Dim::TIME), "0.05 s", "open-circuit quadrature-axis time constant"),
        opt("exciter_ceiling", Kind::Number, "4.5", "most field the exciter can force, per unit of the no-load field"),
        opt("exciter_time", Kind::Quantity(Dim::TIME), "0.015 s", "exciter time constant"),
        opt("reactive_droop", Kind::Number, "0", "voltage set point lowered per unit of reactive load, for sharing between paralleled generators"),
        opt("inertia", Kind::Quantity(Dim::TIME), "0.5 s", "constant-frequency drive: rotor inertia constant H"),
        opt("governor_droop", Kind::Number, "0", "constant-frequency drive: speed droop per unit of load; 0 is isochronous"),
        opt("governor_time", Kind::Quantity(Dim::TIME), "0.05 s", "constant-frequency drive: governor time constant"),
        opt("rated_rpm", Kind::Quantity(Dim::FREQUENCY), "12000 rpm", "shaft speed at rated drive speed, for torque"),
        opt("zone", Kind::Ref(&["electrical.contactor", "electrical.breaker", "electrical.sspc", "electrical.feeder"]), "", "the line contactor or feeder at the end of its differential protection zone"),
        opt("pmg_failed_when", WHEN, "false", "its permanent-magnet generator failed: the control unit and exciter lose power"),
        opt("exciter_failed_when", WHEN, "false", "a rotating rectifier diode open: the exciter delivers less than half its ceiling"),
        opt("regulator_high_when", WHEN, "false", "voltage regulator failed to full field"),
        opt("regulator_low_when", WHEN, "false", "voltage regulator failed to no field"),
        opt("phase_open_when", WHEN, "false", "AC: one stator phase (a) open"),
        opt("winding_short_when", WHEN, "false", "a stator winding shorted to the frame, inside its differential zone"),
        opt("start_when", WHEN, "false", "DC: runs as a starter motor, cranking its drive"),
        opt("start_current", Kind::Quantity(Dim::CURRENT), "1000 A", "DC: starter current limit"),
    ],
    outputs: OUT,
};

outputs!(OUT, go {
    voltage: V, frequency: HZ, current: A, load: W, ready: B, tripped: B, fault: N, disconnected: B, oil_temperature: T, oil_overheat: B, failed: B,
    reactive_power: W, apparent_power: W, power_factor: N, field: N, emf: N, shaft_power: W, torque: TQ, rotor_angle: N,
    current_a: A, current_b: A, current_c: A, negative_sequence: N, speed: N, starter_current: A
});

const OIL_TAU: f64 = 90.0;
/// Remanent flux: the open-circuit voltage a spinning machine shows with no field.
const REMANENCE: f64 = 0.02;

/// A drive's frequency set point tolerance, about 0.06 % either way, fixed per generator.
fn set_point_offset(seed: u64) -> f64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    ((z % 10_000) as f64 / 10_000.0 - 0.5) * 1.25e-3
}

fn saturation(e: f64) -> (f64, f64) {
    if e <= 0.7 {
        return (0.0, 0.0);
    }
    let s = 0.03 * (7.0 * (e - 1.0)).exp();
    (s, 7.0 * s)
}

pub struct Generator {
    base: usize,
    pub bus: usize,
    sh: Shunt,
    pub ac: bool,
    n: usize,
    v_rated: f64,
    f_rated: f64,
    pub rating: f64,
    pub power_factor: f64,
    zb: f64,
    ib: f64,
    xd: f64,
    xd1: f64,
    xd2: f64,
    xq: f64,
    x2: f64,
    x0: f64,
    ra: f64,
    td0p: f64,
    td0pp: f64,
    tq0p: f64,
    ceiling: f64,
    te: f64,
    kp: f64,
    ti: f64,
    q_droop: f64,
    when: Option<Expr>,
    speed_e: Option<Expr>,
    variable: bool,
    min_speed: f64,
    max_speed: f64,
    excitation: f64,
    protect: bool,
    overload_heat: f64,
    reset: Option<Expr>,
    disconnect: Option<Expr>,
    oil_ambient: Option<Expr>,
    oil_rise: f64,
    oil_limit: f64,
    oil_cooled: Option<Expr>,
    faults: [Option<Expr>; 7],
    h: f64,
    droop: f64,
    gov_t: f64,
    w_rated: f64,
    start_limit: f64,
    /// Zone-end link and whether its first bus is this generator's.
    pub zone: Option<(usize, bool)>,
    pub zone_i: Abc,
    eq1: f64,
    eq2: f64,
    ed1: f64,
    efd: f64,
    xi: f64,
    pub theta: f64,
    w: f64,
    pm: f64,
    pref: f64,
    /// Governor set point off rated: no two drives hold exactly the same frequency.
    w_set: f64,
    /// Rotor angle against the island's reference while paralleled and locked to it.
    locked: Option<f64>,
    spin_t: f64,
    pub online: bool,
    pub gcu: Gcu,
    oil: f64,
    disconnected: bool,
    speed: f64,
    fr: f64,
    pub f_now: f64,
    spinning: bool,
    gcr: bool,
    pmg_ok: bool,
    exc_share: f64,
    reg: u8,
    phase_open: bool,
    winding_short: bool,
    starting: bool,
    delta: f64,
    is_ref: bool,
    y: M3,
    e_abc: Abc,
    i_abc: Abc,
    s: C,
    i_d: f64,
    i_q: f64,
    kappa: f64,
    v_sense: f64,
    q_pu: f64,
    fault_y: f64,
    g_dc: f64,
}

impl Generator {
    pub fn new(p: &CheckedPart, bus: usize, g: &mut Grid, buses: &[Bus]) -> Self {
        let b = &buses[bus];
        let ac = b.kind.ac() && p.num("frequency") > 0.0;
        let n = if ac { b.phases() } else { 1 };
        let term = if ac { b.term } else { b.term.phase(0) };
        let v = p.num("voltage").max(1e-3);
        let rating = p.num("rating").max(1.0);
        let ib = rating / n as f64 / v;
        let gen = |k: &str| expr_of(p, k);
        Generator {
            base: p.slot,
            bus,
            sh: Shunt::new(g, term),
            ac,
            n,
            v_rated: v,
            f_rated: p.num("frequency"),
            rating,
            power_factor: p.num("power_factor").clamp(0.05, 1.0),
            zb: v / ib,
            ib,
            xd: p.num("xd").max(0.05),
            xd1: p.num("xd_transient").max(0.02),
            xd2: p.num("xd_subtransient").max(0.01),
            xq: p.num("xq").max(0.02),
            x2: p.num("x2").max(0.01),
            x0: p.num("x0").max(0.005),
            ra: p.num("armature_resistance").max(1e-4),
            td0p: p.num("field_time").max(0.005),
            td0pp: p.num("subtransient_time").max(0.001),
            tq0p: p.num("quadrature_time").max(0.001),
            ceiling: p.num("exciter_ceiling").max(1.0),
            te: p.num("exciter_time").max(0.001),
            kp: p.num("regulator_gain").max(0.1),
            ti: p.num("regulator_time").max(0.01),
            q_droop: p.num("reactive_droop").max(0.0),
            when: gen("driven_when"),
            speed_e: gen("speed"),
            variable: p.flag("variable_frequency"),
            min_speed: p.num("min_speed"),
            max_speed: p.num("max_speed"),
            excitation: p.num("excitation_time"),
            protect: p.flag("protection"),
            overload_heat: p.num("overload_time") * 1.25,
            reset: gen("reset_when"),
            disconnect: gen("disconnect_when"),
            oil_ambient: gen("oil_ambient"),
            oil_rise: p.num("oil_rise"),
            oil_limit: p.num("oil_limit"),
            oil_cooled: gen("oil_cooled_when"),
            faults: ["pmg_failed_when", "exciter_failed_when", "regulator_high_when", "regulator_low_when", "phase_open_when", "winding_short_when", "start_when"].map(gen),
            h: p.num("inertia").max(0.01),
            droop: p.num("governor_droop").max(0.0),
            gov_t: p.num("governor_time").max(0.001),
            w_rated: (p.num("rated_rpm") * std::f64::consts::TAU).max(1.0),
            start_limit: p.num("start_current").max(1.0),
            zone: None,
            zone_i: ZERO3,
            eq1: 0.0,
            eq2: 0.0,
            ed1: 0.0,
            efd: 0.0,
            xi: 0.0,
            theta: 0.0,
            w: 0.0,
            pm: 0.0,
            pref: 0.0,
            w_set: set_point_offset(p.slot as u64),
            locked: None,
            spin_t: 0.0,
            online: false,
            gcu: Gcu::default(),
            oil: 333.15,
            disconnected: false,
            speed: 0.0,
            fr: 0.0,
            f_now: 0.0,
            spinning: false,
            gcr: false,
            pmg_ok: false,
            exc_share: 1.0,
            reg: 0,
            phase_open: false,
            winding_short: false,
            starting: false,
            delta: 0.0,
            is_ref: false,
            y: [[ZERO; 3]; 3],
            e_abc: ZERO3,
            i_abc: ZERO3,
            s: ZERO,
            i_d: 0.0,
            i_q: 0.0,
            kappa: 0.0,
            v_sense: 0.0,
            q_pu: 0.0,
            fault_y: 0.0,
            g_dc: 0.0,
        }
    }

    fn constant_frequency(&self) -> bool {
        self.ac && !self.variable
    }

    /// Drive, control unit logic and line contactor, before the network is solved.
    pub fn drive(&mut self, slots: &mut [f64], dt: f64) {
        let failed = slots[self.base + go::failed] != 0.0;
        if yes(&mut self.disconnect, slots, dt, false) {
            self.disconnected = true;
        }
        self.speed = eval(&mut self.speed_e, slots, dt, 1.0).max(0.0);
        let driven = !failed && !self.disconnected && yes(&mut self.when, slots, dt, true);
        let reset = yes(&mut self.reset, slots, dt, false);
        self.gcu.reset(driven, reset);
        let f = &mut self.faults;
        let pmg_failed = yes(&mut f[0], slots, dt, false);
        let exc_failed = yes(&mut f[1], slots, dt, false);
        let hi = yes(&mut f[2], slots, dt, false);
        let lo = yes(&mut f[3], slots, dt, false);
        self.phase_open = self.ac && yes(&mut f[4], slots, dt, false);
        self.winding_short = yes(&mut f[5], slots, dt, false);
        self.starting = !self.ac && !failed && yes(&mut f[6], slots, dt, false);
        self.reg = if hi { 1 } else if lo { 2 } else { 0 };
        self.exc_share = if exc_failed { 0.4 } else { 1.0 };
        self.spinning = driven && self.speed >= self.min_speed;
        if self.protect {
            self.gcu.overspeed(self.spinning, self.speed, self.max_speed, dt);
        }
        self.pmg_ok = driven && self.speed > 0.05 && !pmg_failed;
        if self.protect && self.spinning && pmg_failed && self.online {
            self.gcu.trip(10.0);
        }
        self.gcr = self.spinning && self.pmg_ok && !self.gcu.tripped;
        self.spin_t = if self.spinning { self.spin_t + dt } else { 0.0 };
        // Drive speed sets frequency on a variable-frequency machine; a constant-speed drive holds rated
        // frequency less what its governor lets slip.
        self.fr = if !self.ac {
            self.speed
        } else if self.variable {
            self.speed
        } else if self.spinning {
            1.0 + self.w
        } else {
            self.speed.min(1.0)
        };
        self.f_now = self.f_rated * self.fr;
        self.theta = (self.theta + std::f64::consts::TAU * self.f_now * dt).rem_euclid(std::f64::consts::TAU);
        if !self.ac && !self.starting && self.online && self.s.re < -0.05 * self.rating {
            // Reverse-current cut-out: a DC generator does not motor off its bus.
            self.online = false;
        }
        let open_v = self.open_circuit();
        let in_band = (0.9..=1.1).contains(&open_v);
        if self.starting {
            self.online = true;
        } else if !self.spinning || self.gcu.tripped || !self.gcr {
            self.online = false;
        } else if !self.online && self.spin_t >= self.excitation && in_band {
            self.online = true;
        }
        let cooled = yes(&mut self.oil_cooled, slots, dt, true);
        let amb = eval(&mut self.oil_ambient, slots, dt, 333.15);
        let share = slots[self.base + go::load] / (self.rating * self.power_factor);
        let target = if self.spinning { amb + self.oil_rise * (0.4 + 0.6 * share.clamp(0.0, 2.0).powi(2)) * if cooled { 1.0 } else { 3.0 } } else { amb };
        self.oil += (target - self.oil) * (dt / OIL_TAU).min(1.0);
        flag(slots, self.base + go::ready, self.online && !self.starting);
        flag(slots, self.base + go::tripped, self.gcu.tripped);
        put(slots, self.base + go::fault, self.gcu.fault);
        flag(slots, self.base + go::disconnected, self.disconnected);
        put(slots, self.base + go::oil_temperature, self.oil);
        flag(slots, self.base + go::oil_overheat, self.oil > self.oil_limit);
    }

    /// Terminal voltage per unit with no load.
    fn open_circuit(&self) -> f64 {
        if self.ac {
            C::new(self.eq2, -self.ed1).abs() * self.fr
        } else {
            self.efd * self.fr
        }
    }

    /// Joins its island's reference contest when on line.
    pub fn offer(&self, isl: &mut Islands, buses: &[Bus]) {
        if !self.online {
            return;
        }
        let i = buses[self.bus].island;
        isl.capacity[i] += self.rating * self.power_factor;
        isl.has_capacity[i] = true;
        isl.source_va[i] += self.rating;
        if self.ac {
            isl.offer_reference(i, self.rating, self.f_now, self.theta);
            isl.gen_p[i] += self.s.re;
            isl.gen_s[i] += self.rating;
            isl.gen_n[i] += 1.0;
        }
    }

    pub fn stamp(&mut self, g: &mut Grid, buses: &[Bus], isl: &Islands) {
        self.y = [[ZERO; 3]; 3];
        self.e_abc = ZERO3;
        let b = &buses[self.bus];
        if self.winding_short {
            self.fault_y = 1.0 / (0.05 * self.zb);
            self.sh.each(g, C::real(self.fault_y));
        } else {
            self.fault_y = 0.0;
        }
        if !self.online {
            return;
        }
        if !self.ac {
            let ra = self.ra * self.zb;
            let e = self.efd * self.fr * self.v_rated;
            let r = if self.starting {
                // The starter controller holds current to its limit until back-emf builds.
                let v = b.vmag.max(0.3 * b.nominal);
                ra.max((v - e).abs() / self.start_limit)
            } else {
                ra
            };
            self.g_dc = 1.0 / r;
            self.y[0][0] = C::real(self.g_dc);
            self.e_abc[0] = C::real(e);
            self.sh.full(g, &self.y);
            self.sh.inject(g, &[C::real(self.g_dc * e), ZERO, ZERO]);
            return;
        }
        let i = b.island;
        if let Some(rel) = self.locked {
            self.theta = (isl.theta[i] + rel).rem_euclid(std::f64::consts::TAU);
        }
        self.delta = super::wrap(self.theta - isl.theta[i]);
        self.is_ref = (isl.ref_rating[i] - self.rating).abs() < 1e-9 && self.delta.abs() < 1e-12;
        let fr = self.fr.max(0.05);
        let z = |x: f64| C::new(self.ra, x * fr).scale(self.zb).inv();
        let (y1, y2, y0) = (z(self.xd2), z(self.x2), z(self.x0));
        let mut y = if self.n == 3 { from_sequence_diag(y0, y1, y2) } else { [[y1, ZERO, ZERO], [ZERO; 3], [ZERO; 3]] };
        if self.phase_open {
            for k in 0..3 {
                y[0][k] = ZERO;
                y[k][0] = ZERO;
            }
        }
        let e = C::new(self.eq2, -self.ed1).scale(fr * self.v_rated) * C::polar(1.0, self.delta);
        let ph = balanced(e);
        let mut eabc = ZERO3;
        eabc[..self.n].copy_from_slice(&ph[..self.n]);
        let inj = mul3(&y, &eabc);
        self.y = y;
        self.e_abc = eabc;
        self.sh.full(g, &self.y);
        self.sh.inject(g, &inj);
    }

    /// Measures, protects, and steps the machine's flux, regulator and rotor for the next step.
    pub fn post(&mut self, g: &Grid, slots: &mut [f64], dt: f64, isl: &Islands, buses: &[Bus]) {
        let v = self.sh.v(g);
        let n = self.n;
        let mut i = ZERO3;
        if self.online {
            let mut d = ZERO3;
            for k in 0..n {
                d[k] = self.e_abc[k] - v[k];
            }
            i = mul3(&self.y, &d);
        }
        self.i_abc = i;
        self.s = power(&v, &i, n);
        let vb = self.v_rated;
        let mag = |x: C| if self.ac { x.abs() } else { x.re };
        let vm: Vec<f64> = (0..n).map(|k| mag(v[k]) / vb).collect();
        let im: Vec<f64> = (0..n).map(|k| i[k].abs() / self.ib).collect();
        let v_avg = vm.iter().sum::<f64>() / n as f64;
        self.v_sense = if self.online { v_avg } else { self.open_circuit() };
        let seq_i = if n == 3 { to_sequence(&i) } else { [ZERO, i[0], ZERO] };
        let fault_i = if self.fault_y > 0.0 { (0..n).map(|k| v[k].abs() * self.fault_y).fold(0.0, f64::max) / self.ib } else { 0.0 };
        let differential = match self.zone {
            Some(_) if self.online => (0..n).map(|k| (i[k] - self.zone_i[k]).abs()).fold(0.0, f64::max) / self.ib + fault_i,
            _ => fault_i,
        };
        let measure = Measure {
            v_max: vm.iter().copied().fold(0.0, f64::max).max(if self.online { 0.0 } else { self.v_sense }),
            v_min: vm.iter().copied().fold(f64::INFINITY, f64::min),
            v_avg,
            freq: if self.f_rated > 0.0 { self.f_now / self.f_rated } else { 1.0 },
            i_max: im.iter().copied().fold(0.0, f64::max),
            i_min: im.iter().copied().fold(f64::INFINITY, f64::min),
            i_neg: seq_i[2].abs() / self.ib,
            differential,
            load_share: self.s.re / (self.rating * self.power_factor),
            reverse: if self.ac { 0.0 } else { (-self.s.re / self.rating).max(0.0) },
            online: self.online && !self.starting,
            excited: self.gcr,
            ac: self.ac,
            constant_frequency: self.constant_frequency(),
            three_phase: n == 3,
        };
        if self.protect {
            let lim = Limits { overload_heat: self.overload_heat, differential: self.zone.is_some() || self.fault_y > 0.0 };
            self.gcu.protect(&measure, &lim, dt);
        }
        if self.ac {
            let seq_v1 = if n == 3 { to_sequence(&v)[1] } else { v[0] };
            self.step_ac(dt, &seq_i, seq_v1, isl, buses);
        } else {
            self.step_dc(dt);
        }
        let p_mech = self.shaft_power();
        let omega = self.w_rated * self.speed.max(0.02);
        let b = self.base;
        let on = self.online;
        put(slots, b + go::voltage, if on { (v_avg * vb).max(0.0) } else { 0.0 });
        put(slots, b + go::frequency, if on && self.ac { self.f_now } else { 0.0 });
        put(slots, b + go::current, if self.ac { im.iter().sum::<f64>() / n as f64 * self.ib } else { i[0].re });
        put(slots, b + go::load, self.s.re);
        put(slots, b + go::reactive_power, self.s.im);
        put(slots, b + go::apparent_power, self.s.abs());
        put(slots, b + go::power_factor, if self.s.abs() > 1.0 { self.s.re / self.s.abs() } else { 1.0 });
        put(slots, b + go::field, self.efd);
        put(slots, b + go::emf, self.open_circuit());
        put(slots, b + go::shaft_power, p_mech);
        put(slots, b + go::torque, p_mech / omega);
        put(slots, b + go::rotor_angle, self.delta.to_degrees());
        put(slots, b + go::current_a, i[0].abs());
        put(slots, b + go::current_b, if n == 3 { i[1].abs() } else { 0.0 });
        put(slots, b + go::current_c, if n == 3 { i[2].abs() } else { 0.0 });
        put(slots, b + go::negative_sequence, seq_i[2].abs() / self.ib);
        put(slots, b + go::speed, self.speed);
        put(slots, b + go::starter_current, if self.starting { (-i[0].re).max(0.0) } else { 0.0 });
        flag(slots, b + go::tripped, self.gcu.tripped);
        put(slots, b + go::fault, self.gcu.fault);
    }

    fn shaft_power(&self) -> f64 {
        if !self.spinning && !self.starting {
            return 0.0;
        }
        let i2: f64 = self.i_abc[..self.n].iter().map(|x| x.abs2()).sum();
        let copper = i2 * self.ra * self.zb;
        let core = 0.012 * self.rating * (self.v_sense * self.v_sense) * self.fr.max(0.0).powf(1.5);
        let windage = 0.006 * self.rating * self.speed.powi(3);
        let drive = if self.constant_frequency() { 0.06 * self.s.re.abs() + 0.01 * self.rating } else { 0.0 };
        self.s.re + copper + core + windage + drive
    }

    fn step_ac(&mut self, dt: f64, seq_i: &[C; 3], seq_v1: C, isl: &Islands, buses: &[Bus]) {
        let ip = if self.online { seq_i[1] / self.ib * C::polar(1.0, -self.delta) } else { ZERO };
        self.i_d = -ip.im;
        self.i_q = ip.re;
        self.kappa = if self.online { (self.i_d / self.eq2.abs().max(0.05)).clamp(0.0, 1.0 / self.xd2) } else { 0.0 };
        self.q_pu = self.s.im / self.rating;
        // Regulator: from the measured current it works out the field that holds the terminal at its set
        // point, forces the transient flux toward it, and trims what that misses with a slow integral.
        let ceiling = if self.pmg_ok { self.ceiling * self.exc_share * self.fr.clamp(0.0, 1.2) } else { 0.0 };
        let ramp = (self.spin_t / (0.6 * self.excitation).max(0.02)).min(1.0);
        let target = ramp * (1.0 - self.q_droop * self.q_pu);
        let err = target - self.v_sense;
        let fr = self.fr.max(0.05);
        let v1 = if self.online { seq_v1.scale(1.0 / self.v_rated) * C::polar(1.0, -self.delta) } else { ZERO };
        let dir = if v1.abs() > 0.05 { v1.unit() } else { C::real(1.0) };
        let e2 = dir.scale(target + self.xi) + C::new(self.ra, self.xd2 * fr) * ip;
        let eq2_need = (e2.re / fr).max(0.0);
        let eq1_need = eq2_need + (self.xd1 - self.xd2) * self.i_d;
        let (s_need, _) = saturation(eq1_need);
        let efd_ss = eq1_need + s_need + (self.xd - self.xd1) * self.i_d;
        let cmd = if !self.gcr {
            self.xi = 0.0;
            0.0
        } else {
            match self.reg {
                1 => ceiling,
                2 => 0.0,
                _ => {
                    let raw = efd_ss + self.kp * (eq1_need - self.eq1);
                    if raw > 0.0 && raw < ceiling && ramp >= 1.0 && err.abs() < 0.02 {
                        self.xi = (self.xi + err * dt / self.ti).clamp(-0.3, 0.3);
                    }
                    raw.clamp(0.0, ceiling)
                }
            }
        };
        self.efd = cmd + (self.efd - cmd) * (-dt / self.te).exp();
        // Flux on both axes, implicit in the field's own decay and in how the stator current answers it.
        let floor = if self.spinning { REMANENCE } else { 0.0 };
        let (s0, ds) = saturation(self.eq1);
        let kd = self.xd - self.xd1;
        let h1 = dt / self.td0p;
        let k = self.kappa;
        self.eq1 = (self.eq1 + h1 * (self.efd - s0 + ds * self.eq1 - kd * (self.i_d - k * self.eq1))) / (1.0 + h1 * (1.0 + ds + kd * k));
        self.eq1 = self.eq1.max(floor);
        let kd2 = self.xd1 - self.xd2;
        let h2 = dt / self.td0pp;
        self.eq2 = (self.eq2 + h2 * (self.eq1 - kd2 * (self.i_d - k * self.eq2))) / (1.0 + h2 * (1.0 + kd2 * k));
        self.eq2 = self.eq2.max(floor);
        let kq = (self.xq - self.xd2).max(0.0);
        let hq = dt / self.tq0p;
        self.ed1 = (self.ed1 + hq * kq * self.i_q) / (1.0 + hq);
        if !self.spinning {
            self.ed1 = 0.0;
        }
        // Rotor of a constant-frequency drive. Alone, or as its island's reference, its governor holds speed
        // against rotor inertia. Paralleled, it is locked to the island's frequency by synchronising torque and
        // its rotor angle moves each step toward an equal share of the island's load, so the stiff angle mode
        // between two machines never has to be stepped explicitly.
        if self.constant_frequency() && self.spinning {
            let pe = if self.online { self.s.re / self.rating } else { 0.0 };
            let i = buses[self.bus].island;
            let w_ref = isl.freq.get(i).copied().unwrap_or(self.f_now) / self.f_rated.max(1e-9) - 1.0;
            let paralleled = self.online && !self.is_ref && isl.gen_n.get(i).copied().unwrap_or(0.0) >= 2.0;
            if paralleled {
                let share = isl.gen_p[i] / isl.gen_s[i].max(1.0);
                // Steady synchronising power through both machines' quadrature reactance.
                let ks = (self.eq2 * self.v_sense / (self.xd2 + self.xq)).max(0.3);
                let step = ((share - pe) / ks).clamp(-0.2, 0.2) * (dt / 0.2).min(1.0);
                self.locked = Some(super::wrap(self.locked.unwrap_or(self.delta) + step));
                self.w = w_ref;
                self.pm = pe;
                self.pref = pe;
            } else {
                self.locked = None;
                // Droop governs on speed error alone; isochronous adds an integral back to its set point.
                let r = if self.droop > 0.0 { self.droop } else { 0.02 };
                let speed_err = self.w - self.w_set;
                if self.droop <= 0.0 {
                    self.pref -= speed_err * dt / (0.5 * r);
                }
                let pm_target = self.pref - speed_err / r;
                self.pm = pm_target + (self.pm - pm_target) * (-dt / self.gov_t).exp();
                self.w = (self.w + dt / (2.0 * self.h) * (self.pm - pe)).clamp(-0.2, 0.2);
            }
        } else if !self.spinning {
            self.locked = None;
            self.w = 0.0;
            self.pm = 0.0;
            self.pref = 0.0;
        }
    }

    fn step_dc(&mut self, dt: f64) {
        let ceiling = if self.pmg_ok || self.starting { 1.5 * self.exc_share } else { 0.0 };
        let ramp = (self.spin_t / (0.6 * self.excitation).max(0.02)).min(1.0);
        let err = ramp - self.v_sense;
        let i_pu = if self.online { self.i_abc[0].re / self.ib } else { 0.0 };
        let need = ((ramp + self.xi + self.ra * i_pu) / self.fr.max(0.05)).max(0.0);
        let cmd = if self.starting {
            1.0
        } else if !self.gcr {
            self.xi = 0.0;
            0.0
        } else {
            match self.reg {
                1 => ceiling,
                2 => 0.0,
                _ => {
                    let raw = need + self.kp * (need - self.efd);
                    if raw > 0.0 && raw < ceiling && ramp >= 1.0 && err.abs() < 0.02 {
                        self.xi = (self.xi + err * dt / self.ti).clamp(-0.3, 0.3);
                    }
                    raw.clamp(0.0, ceiling)
                }
            }
        };
        let floor = if self.spinning { REMANENCE } else { 0.0 };
        self.efd = (cmd + (self.efd - cmd) * (-dt / self.td0p).exp()).max(floor);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        let b = |x: bool| x as u8 as f64;
        v.extend([self.eq1, self.eq2, self.ed1, self.efd, self.xi, self.theta, self.w, self.pm, self.pref, self.spin_t, b(self.online), self.oil, b(self.disconnected), self.i_d, self.i_q, self.kappa, self.v_sense, self.s.re, self.s.im]);
        self.gcu.state(v);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.eq1 = n();
        self.eq2 = n();
        self.ed1 = n();
        self.efd = n();
        self.xi = n();
        self.theta = n();
        self.w = n();
        self.pm = n();
        self.pref = n();
        self.spin_t = n();
        self.online = n() != 0.0;
        self.oil = n();
        self.disconnected = n() != 0.0;
        self.i_d = n();
        self.i_q = n();
        self.kappa = n();
        self.v_sense = n();
        self.s = C::new(n(), n());
        self.gcu.restore(it);
    }
}
