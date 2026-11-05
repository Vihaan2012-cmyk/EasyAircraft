//! Loads: constant-power electronics with hold-up capacitors and reboot, resistive heaters, lamps whose cold
//! filaments draw an inrush, constant-current loads, simple motors with a starting surge, and induction motors
//! from their equivalent circuit, accelerating against their load and stalling when the voltage sags. AC loads
//! sit on one phase or all three and draw reactive power from their power factor.

use super::bus::{Bus, BusKind};
use super::complex::{Abc, C, ZERO, ZERO3};
use super::grid::{Grid, Shunt};
use super::{flag, put, yes, A, B, E, N, SEC, T, V, W, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "electrical",
    kind: "load",
    help: "Something that draws power: constant power (electronics, with hold-up and reboot), resistive (heaters), lamp (cold filament inrush), constant current, motor (a starting surge) or induction_motor (equivalent circuit: inrush, acceleration, stall, winding heat). AC loads take one phase or all three and draw reactive power from their power factor.",
    fields: &[
        req("bus", Kind::Ref(&["electrical.bus"]), "the bus it draws from"),
        req("power", Kind::Expr(W), "for example \"250 W\", or an expression such as \"heater.value * 1 kW\"; resistive loads draw this at nominal voltage; an induction motor's shaft power at rated speed"),
        opt("on_when", WHEN, "true", "when it is switched on"),
        opt("shed_priority", Kind::Number, "0", "0 never shed; higher numbers are shed first"),
        opt("min_voltage", Kind::Quantity(Dim::VOLTAGE), "0 V", "below this it switches itself off, back on 0.5 s after the bus is 10 % above it; 0 V means half the bus nominal"),
        opt("type", Kind::Text, "constant_power", "constant_power, electronic, resistive, lamp, constant_current, motor or induction_motor"),
        opt("inrush", Kind::Number, "5", "motor: starting power as a multiple of running power"),
        opt("start_time", Kind::Quantity(Dim::TIME), "1 s", "motor: how long the inrush takes to die away; induction motor: its mechanical starting time"),
        opt("power_factor", Kind::Number, "1", "real per apparent power; lagging"),
        opt("phase", Kind::Text, "abc", "AC loads: abc for all three phases, a, b or c, or auto for the phase with the fewest single-phase loads so far"),
        opt("holdup", Kind::Quantity(Dim::TIME), "0.05 s", "electronic: how long its capacitors carry it through an interruption"),
        opt("boot_time", Kind::Quantity(Dim::TIME), "0 s", "electronic: restart time after it lost power for longer than its hold-up"),
        opt("harmonics", Kind::Number, "0", "share of its current that is harmonic, for supply distortion; rectifier front ends about 0.3"),
        opt("rated_frequency", Kind::Quantity(Dim::FREQUENCY), "400 Hz", "induction motor: frequency its speed and reactances are rated at"),
        opt("rated_power", Kind::Quantity(Dim::POWER), "0 W", "induction motor: its rated shaft power; 0 takes the first power it is asked for, and power then sets its load"),
    ],
    outputs: OUT,
};

outputs!(OUT, lo { powered: B, current: A, power: W, shed: B, failed: B, voltage: V, reactive_power: W, speed: N, resetting: B, temperature: T, overheat: B, interrupted: SEC, energy: E, phase: N });

/// A dropped-out load comes back once its bus is this far above its minimum voltage for `RECOVER_S`.
const RECOVER_RATIO: f64 = 1.1;
const RECOVER_S: f64 = 0.5;
/// Constant-power loads are linearised at last step's voltage, never below this share of nominal: a lagged
/// constant-power load diverges once its conductance passes the source's. Loads drop out at their minimum
/// voltage above it, so the floor only holds through a transient.
const LINEARISE_FLOOR: f64 = 0.5;
const FILAMENT_TAU: f64 = 0.06;
/// Cold filament resistance as a share of hot.
const FILAMENT_COLD: f64 = 0.08;
const AMBIENT_K: f64 = 313.15;

#[derive(Clone, Copy, PartialEq)]
pub enum LoadType {
    Power,
    Electronic,
    Resistive,
    Lamp,
    Current,
    Motor,
    Induction,
}

/// Per-unit equivalent circuit of an induction motor at rated frequency.
#[derive(Clone, Copy)]
struct Induction {
    rs: f64,
    xs: f64,
    xm: f64,
    rr: f64,
    xr: f64,
    slip_rated: f64,
    /// Air-gap power at rated slip, per unit of the circuit's own base.
    pag_rated: f64,
    f_rated: f64,
    /// Ohms of one per unit, per phase.
    zbase: f64,
    t_mech: f64,
    speed: f64,
    temp: f64,
    hot: bool,
    cu_rated: f64,
    rated: f64,
}

impl Induction {
    fn new(power_w: f64, phases: f64, v_phase: f64, f_rated: f64, t_mech: f64) -> Self {
        let mut m = Induction { rs: 0.03, xs: 0.09, xm: 2.5, rr: 0.03, xr: 0.09, slip_rated: 0.035, pag_rated: 1.0, f_rated, zbase: 1.0, t_mech: t_mech.max(0.05), speed: 0.0, temp: AMBIENT_K, hot: false, cu_rated: 1.0, rated: power_w };
        let (_, ir, is) = m.circuit(m.slip_rated, 1.0);
        m.pag_rated = ir.abs2() * m.rr / m.slip_rated;
        let pmech = m.pag_rated * (1.0 - m.slip_rated);
        m.cu_rated = is.abs2() * m.rs + ir.abs2() * m.rr;
        // Base chosen so the motor delivers its rated shaft power at rated slip on nominal voltage.
        m.zbase = v_phase * v_phase * pmech * phases / power_w.max(1.0);
        m
    }

    /// Input impedance, rotor and stator current per unit at slip `s` and frequency share `fr`.
    fn circuit(&self, s: f64, fr: f64) -> (C, C, C) {
        let s = if s.abs() < 1e-4 { 1e-4f64.copysign(if s == 0.0 { 1.0 } else { s }) } else { s };
        let fr = fr.max(0.02);
        let zm = C::new(0.0, self.xm * fr);
        let zr = C::new(self.rr / s, self.xr * fr);
        let par = zm * zr / (zm + zr);
        let z = C::new(self.rs, self.xs * fr) + par;
        let is = z.inv();
        let ir = is * zm / (zm + zr);
        (z, ir, is)
    }

    fn slip(&self, fr: f64) -> f64 {
        if fr <= 0.0 {
            1.0
        } else {
            (fr - self.speed) / fr
        }
    }

    /// Electrical torque per unit of rated, at voltage share `vr`.
    fn torque(&self, speed: f64, fr: f64, vr: f64) -> f64 {
        if fr <= 0.02 {
            return 0.0;
        }
        let s = (fr - speed) / fr;
        let s = if s.abs() < 1e-4 { 1e-4 } else { s };
        let (_, ir, _) = self.circuit(s, fr);
        let pag = ir.abs2() * self.rr / s;
        vr * vr * pag / fr / self.pag_rated
    }
}

pub struct Load {
    pub bus: usize,
    base: usize,
    sh: Shunt,
    phases: [bool; 3],
    n_act: f64,
    ac: bool,
    nominal: f64,
    pub power: f64,
    power_e: Option<Expr>,
    when: Option<Expr>,
    pub priority: f64,
    pub shed: bool,
    min_voltage: f64,
    dropped: bool,
    recover: f64,
    pub kind: LoadType,
    inrush: f64,
    start_time: f64,
    started: f64,
    pub power_factor: f64,
    pub harmonics: f64,
    holdup: f64,
    hold_t: f64,
    boot_time: f64,
    boot_t: f64,
    resetting: bool,
    filament: f64,
    im: Option<Induction>,
    pub wanted: bool,
    on: bool,
    y: Abc,
    interrupted: f64,
    pub drawn: f64,
    energy: f64,
}

impl Load {
    pub fn new(p: &CheckedPart, bus: usize, g: &mut Grid, buses: &[Bus], spread: &mut [usize; 3]) -> Self {
        let b = &buses[bus];
        let min = p.num("min_voltage");
        let kind = match p.text("type") {
            "resistive" | "heater" => LoadType::Resistive,
            "motor" => LoadType::Motor,
            "lamp" => LoadType::Lamp,
            "electronic" => LoadType::Electronic,
            "constant_current" => LoadType::Current,
            "induction_motor" if b.kind.ac() => LoadType::Induction,
            "induction_motor" => LoadType::Motor,
            _ => LoadType::Power,
        };
        let phases = if b.kind == BusKind::Ac3 {
            match p.text("phase") {
                "a" => [true, false, false],
                "b" => [false, true, false],
                "c" => [false, false, true],
                "auto" => {
                    let k = (0..3).min_by_key(|&k| (spread[k], k)).unwrap_or(0);
                    let mut ph = [false; 3];
                    ph[k] = true;
                    ph
                }
                _ => [true; 3],
            }
        } else {
            [true, false, false]
        };
        let n_act = phases.iter().filter(|&&x| x).count() as f64;
        if n_act == 1.0 && b.kind == BusKind::Ac3 {
            spread[phases.iter().position(|&x| x).unwrap_or(0)] += 1;
        }
        let mut l = Load {
            bus,
            base: p.slot,
            sh: Shunt::new(g, b.term),
            phases,
            n_act,
            ac: b.kind.ac(),
            nominal: b.nominal,
            power: 0.0,
            power_e: expr_of(p, "power"),
            when: expr_of(p, "on_when"),
            priority: p.num("shed_priority"),
            shed: false,
            min_voltage: if min > 0.0 { min } else { 0.5 * b.nominal },
            dropped: false,
            recover: 0.0,
            kind,
            inrush: p.num("inrush").max(1.0),
            start_time: p.num("start_time").max(0.01),
            started: 0.0,
            power_factor: p.num("power_factor").clamp(0.05, 1.0),
            harmonics: p.num("harmonics").clamp(0.0, 1.0),
            holdup: if kind == LoadType::Electronic { p.num("holdup").max(0.0) } else { 0.0 },
            hold_t: 0.0,
            boot_time: if kind == LoadType::Electronic { p.num("boot_time").max(0.0) } else { 0.0 },
            boot_t: 0.0,
            resetting: false,
            filament: 0.0,
            im: None,
            wanted: false,
            on: false,
            y: ZERO3,
            interrupted: 0.0,
            drawn: 0.0,
            energy: 0.0,
        };
        if kind == LoadType::Induction {
            l.im = Some(Induction::new(p.num("rated_power"), n_act, b.nominal, p.num("rated_frequency").max(1.0), l.start_time));
        }
        l
    }

    /// Apparent power it asks for, for load shedding.
    pub fn va(&self) -> f64 {
        self.power / self.power_factor
    }

    /// Evaluates its demand and whether it wants power this step.
    pub fn demand(&mut self, slots: &[f64], dt: f64, buses: &[Bus]) {
        self.power = self.power_e.as_mut().map_or(0.0, |e| e.eval(slots, dt).max(0.0));
        let tripped_hot = self.im.is_some_and(|m| m.hot);
        self.on = slots[self.base + lo::failed] == 0.0 && yes(&mut self.when, slots, dt, true) && !tripped_hot;
        self.wanted = self.on && buses[self.bus].powered;
        if let Some(m) = &mut self.im {
            if m.rated <= 0.0 && self.power > 0.0 {
                let mut fresh = Induction::new(self.power, self.n_act, self.nominal, m.f_rated, m.t_mech);
                fresh.speed = m.speed;
                fresh.temp = m.temp;
                *m = fresh;
            }
        }
    }

    pub fn active(&self) -> bool {
        self.wanted && !self.shed && !self.dropped && !(self.kind == LoadType::Electronic && self.hold_t > 0.0) && self.im.is_none_or(|m| m.rated > 0.0)
    }

    /// `first` is false when the network is solved again within a step, so timers do not count twice.
    pub fn stamp(&mut self, g: &mut Grid, buses: &[Bus], dt: f64, first: bool) {
        self.y = ZERO3;
        let b = &buses[self.bus];
        let running = self.active();
        let dt = if first { dt } else { 0.0 };
        if !running {
            self.started = 0.0;
            if !self.wanted {
                self.filament = 0.0;
            }
            if let Some(m) = &mut self.im {
                if !self.wanted || self.shed || self.dropped {
                    m.speed = (m.speed - dt / (4.0 * m.t_mech)).max(0.0);
                }
            }
            return;
        }
        let n = self.n_act;
        let p = self.power / n;
        let q = if self.ac { p * (1.0 / (self.power_factor * self.power_factor) - 1.0).max(0.0).sqrt() } else { 0.0 };
        let vn2 = (b.nominal * b.nominal).max(1e-9);
        for k in 0..b.phases() {
            if !self.phases[k] {
                continue;
            }
            let vk = if b.kind.ac() { b.v[k].abs() } else { b.v[k].re };
            let v = vk.max(LINEARISE_FLOOR * b.nominal);
            self.y[k] = match self.kind {
                LoadType::Resistive => C::new(p, -q) / vn2,
                LoadType::Power | LoadType::Electronic => C::new(p, -q) / (v * v),
                LoadType::Current => C::new(p, -q) / (b.nominal * v),
                LoadType::Lamp => C::real(p / vn2 / (FILAMENT_COLD + (1.0 - FILAMENT_COLD) * self.filament)),
                LoadType::Motor => {
                    let starting = (1.0 - self.started / self.start_time).max(0.0);
                    C::new(p, -q) * (1.0 + (self.inrush - 1.0) * starting) / (v * v)
                }
                LoadType::Induction => {
                    let m = self.im.as_ref().expect("induction motor");
                    let fr = b.freq / m.f_rated;
                    let (z, _, _) = m.circuit(m.slip(fr), fr);
                    (z.scale(m.zbase)).inv()
                }
            };
        }
        if self.kind == LoadType::Motor {
            self.started += dt;
        }
        self.sh.diag(g, &self.y);
    }

    pub fn post(&mut self, g: &Grid, slots: &mut [f64], dt: f64, buses: &[Bus]) {
        let bus = &buses[self.bus];
        let v = g.v(&self.sh.t);
        let n = bus.phases();
        let mut s = ZERO;
        let mut amps = 0.0f64;
        let mut vsum = 0.0;
        for k in 0..n {
            if !self.phases[k] {
                continue;
            }
            let i = self.y[k] * v[k];
            s += v[k] * i.conj();
            amps = amps.max(i.abs());
            vsum += if bus.kind.ac() { v[k].abs() } else { v[k].re.max(0.0) };
        }
        let vm = vsum / self.n_act;
        let wanted = self.wanted && !self.shed;
        let electronic = self.kind == LoadType::Electronic && self.on && !self.shed;
        if electronic {
            self.electronic(vm, dt);
        } else if !wanted {
            self.dropped = false;
            self.recover = 0.0;
            self.hold_t = 0.0;
            self.resetting = false;
            self.boot_t = 0.0;
        } else if !self.dropped && vm < self.min_voltage {
            self.dropped = true;
            self.recover = 0.0;
        } else if self.dropped {
            self.recover = if vm >= self.min_voltage * RECOVER_RATIO { self.recover + dt } else { 0.0 };
            if self.recover >= RECOVER_S {
                self.dropped = false;
            }
        }
        if self.kind == LoadType::Lamp {
            let target = if self.active() { (vm / self.nominal.max(1e-9)).clamp(0.0, 1.3) } else { 0.0 };
            self.filament += (target - self.filament) * (dt / FILAMENT_TAU).min(1.0);
        }
        let mut speed = 0.0;
        let mut temp = AMBIENT_K;
        let mut hot = false;
        if let Some(m) = &mut self.im {
            let fr = bus.freq / m.f_rated;
            let vr = vm / self.nominal.max(1e-9);
            if self.y.iter().any(|y| !y.is_zero()) {
                // Implicit speed step: torque falls steeply near synchronous speed, so explicit steps ring.
                let share = self.power / m.rated.max(1.0);
                let load = |w: f64| share * w.max(0.0).powi(2);
                let mut w = m.speed;
                for _ in 0..3 {
                    let f = |x: f64| m.speed + dt / m.t_mech * (m.torque(x, fr, vr) - load(x)) - x;
                    let h = 1e-5;
                    let d = (f(w + h) - f(w - h)) / (2.0 * h);
                    let fw = f(w);
                    if d.abs() > 1e-12 {
                        w -= fw / d;
                    }
                    w = w.clamp(0.0, fr.max(0.0) * 1.02);
                }
                m.speed = w;
                let (_, ir, is) = m.circuit(m.slip(fr), fr.max(0.02));
                let cu = (is.abs2() * m.rs + ir.abs2() * m.rr) * vr * vr;
                let rise = 80.0 * cu / m.cu_rated;
                m.temp += (AMBIENT_K + rise - m.temp) * (dt / 240.0).min(1.0);
            } else {
                m.temp += (AMBIENT_K - m.temp) * (dt / 600.0).min(1.0);
            }
            // Class F windings: the thermal switch opens at 155 C and closes 30 K lower.
            m.hot = if m.hot { m.temp > 398.15 } else { m.temp > 428.15 };
            speed = m.speed;
            temp = m.temp;
            hot = m.hot;
        }
        self.interrupted = if wanted && vm < self.min_voltage { self.interrupted + dt } else { 0.0 };
        self.drawn = s.re.max(0.0);
        self.energy += self.drawn * dt;
        let b = self.base;
        let powered = (wanted || electronic) && !self.dropped && !self.resetting;
        flag(slots, b + lo::powered, powered);
        flag(slots, b + lo::shed, self.shed && self.wanted);
        put(slots, b + lo::power, self.drawn);
        put(slots, b + lo::current, amps);
        put(slots, b + lo::voltage, vm);
        put(slots, b + lo::reactive_power, s.im);
        put(slots, b + lo::speed, speed);
        flag(slots, b + lo::resetting, self.resetting);
        put(slots, b + lo::temperature, temp);
        flag(slots, b + lo::overheat, hot);
        put(slots, b + lo::interrupted, self.interrupted);
        put(slots, b + lo::energy, self.energy);
        let ph = if self.n_act == 1.0 && self.ac { self.phases.iter().position(|&x| x).map_or(0.0, |k| k as f64 + 1.0) } else { 0.0 };
        put(slots, b + lo::phase, ph);
    }

    fn electronic(&mut self, vm: f64, dt: f64) {
        let holding = self.hold_t > 0.0;
        if vm < self.min_voltage || (holding && vm < self.min_voltage * RECOVER_RATIO) {
            self.hold_t += dt;
            if self.hold_t > self.holdup && !self.dropped {
                self.dropped = true;
            }
        } else if self.dropped {
            // Its capacitors recharge before it restarts; a reboot follows if it has one.
            self.recover += dt;
            if self.recover >= RECOVER_S.min(0.1) {
                self.dropped = false;
                self.recover = 0.0;
                self.hold_t = 0.0;
                self.resetting = self.boot_time > 0.0;
                self.boot_t = 0.0;
            }
        } else {
            // A brief dip recharges the hold-up slowly, so a load sagging its own bus cannot chatter forever.
            self.hold_t = (self.hold_t - 0.5 * dt).max(0.0);
            if self.resetting {
                self.boot_t += dt;
                if self.boot_t >= self.boot_time {
                    self.resetting = false;
                }
            }
        }
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        let b = |x: bool| x as u8 as f64;
        let (sp, tp, hot) = self.im.map_or((0.0, AMBIENT_K, false), |m| (m.speed, m.temp, m.hot));
        v.extend([b(self.shed), b(self.dropped), self.recover, self.started, self.hold_t, self.boot_t, b(self.resetting), self.filament, sp, tp, b(hot), self.interrupted, self.energy]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.shed = n() != 0.0;
        self.dropped = n() != 0.0;
        self.recover = n();
        self.started = n();
        self.hold_t = n();
        self.boot_t = n();
        self.resetting = n() != 0.0;
        self.filament = n();
        let (sp, tp, hot) = (n(), n(), n() != 0.0);
        if let Some(m) = &mut self.im {
            m.speed = sp;
            m.temp = tp;
            m.hot = hot;
        }
        self.interrupted = n();
        self.energy = n();
    }
}
