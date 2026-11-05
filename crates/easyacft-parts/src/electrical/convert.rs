//! Power conversion: transformer-rectifier units, static inverters, battery chargers and three-phase
//! transformers. Converters couple islands that are solved together: a TRU's output emf follows the AC it
//! rectifies in the same step, while what it draws on its input lags its output by one step.

use super::bus::{Bus, Islands};
use super::complex::{balanced, Abc, C, ZERO, ZERO3};
use super::grid::{Grid, Series, Shunt};
use super::switchgear::Path;
use super::{flag, put, yes, A, B, N, T, V, W, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

const BUSES: &[&str] = &["electrical.bus"];

pub const SPECS: [PartSpec; 4] = [
    PartSpec {
        domain: "electrical",
        kind: "tru",
        help: "Transformer-rectifier unit: its DC output follows the AC it rectifies and droops with load, more at higher input frequency as commutation overlap grows. Output diodes stop it sinking current. Constant-current limiting, an efficiency curve, harmonic current on its AC side and overheat.",
        fields: &[
            req("input", Kind::Ref(BUSES), "AC bus"),
            req("output", Kind::Ref(BUSES), "DC bus"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "DC output voltage off load at nominal AC input"),
            req("rating", Kind::Quantity(Dim::POWER), "output rating"),
            opt("efficiency", Kind::Number, "0.9", "at rated load, 0 to 1"),
            opt("on_when", WHEN, "true", "when it runs"),
            opt("current_limit", Kind::Number, "1.5", "share of rating it holds as a constant current when overloaded"),
            opt("cooled_when", WHEN, "true", "its cooling fan or airflow works"),
            opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "393.15 K", "it shuts down above this and restarts 20 K below"),
            opt("regulation", Kind::Number, "0.04", "output droop from no load to rated load at nominal input frequency"),
            opt("regulated", Kind::Bool, "false", "a regulated TRU holds its output whatever the input, down to 75 % input voltage"),
            opt("pulses", Kind::Number, "12", "rectifier pulse number: 6, 12 or 18; fewer pulses, more harmonic current"),
            opt("input_power_factor", Kind::Number, "0.95", "displacement power factor of its AC draw"),
            opt("nominal_frequency", Kind::Quantity(Dim::FREQUENCY), "400 Hz", "input frequency its regulation is quoted at"),
        ],
        outputs: CONVERTER_OUT,
    },
    PartSpec {
        domain: "electrical",
        kind: "inverter",
        help: "Static inverter: DC in, regulated AC out on its own oscillator, current limiting, an input undervoltage cut-off, an efficiency curve and overheat.",
        fields: &[
            req("input", Kind::Ref(BUSES), "DC bus"),
            req("output", Kind::Ref(BUSES), "AC bus"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "AC output voltage"),
            opt("frequency", Kind::Quantity(Dim::FREQUENCY), "400 Hz", "output frequency"),
            req("rating", Kind::Quantity(Dim::POWER), "output rating"),
            opt("efficiency", Kind::Number, "0.85", "at rated load, 0 to 1"),
            opt("on_when", WHEN, "true", "when it runs"),
            opt("current_limit", Kind::Number, "1.5", "share of rating it holds as a constant current when overloaded"),
            opt("cooled_when", WHEN, "true", "its cooling works"),
            opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "373.15 K", "it shuts down above this and restarts 20 K below"),
            opt("min_input", Kind::Number, "0.6", "share of input bus nominal below which it shuts down"),
        ],
        outputs: CONVERTER_OUT,
    },
    PartSpec {
        domain: "electrical",
        kind: "charger",
        help: "Battery charger: constant current up to its float voltage, then constant voltage, compensated for the battery's temperature and stopped when the battery overheats.",
        fields: &[
            req("input", Kind::Ref(BUSES), "the bus it draws from"),
            req("output", Kind::Ref(BUSES), "the battery's bus"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "float voltage at 25 C"),
            req("current", Kind::Quantity(Dim::CURRENT), "constant-current charge rate"),
            opt("efficiency", Kind::Number, "0.85", "0 to 1"),
            opt("on_when", WHEN, "true", "when it charges"),
            opt("cooled_when", WHEN, "true", "its cooling works"),
            opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "373.15 K", "it shuts down above this and restarts 20 K below"),
            opt("battery", Kind::Ref(&["electrical.battery"]), "", "battery it watches for temperature"),
            opt("temperature_compensation", Kind::Number, "0", "float voltage change per kelvin of battery temperature above 25 C, volts; negative for nickel-cadmium and lead-acid"),
        ],
        outputs: CONVERTER_OUT,
    },
    PartSpec {
        domain: "electrical",
        kind: "transformer",
        help: "Three-phase transformer or autotransformer between two AC buses: turns ratio, leakage impedance, magnetising current, copper and core loss heating.",
        fields: &[
            req("between", Kind::Refs(BUSES), "primary bus, secondary bus"),
            req("ratio", Kind::Number, "primary voltage over secondary voltage"),
            req("rating", Kind::Quantity(Dim::POWER), "apparent power rating"),
            opt("impedance", Kind::Number, "0.05", "leakage impedance, per unit on its rating"),
            opt("x_r", Kind::Number, "5", "reactance over resistance of that impedance"),
            opt("magnetizing", Kind::Number, "0.02", "magnetising current, per unit"),
            opt("cooled_when", WHEN, "true", "its cooling works"),
            opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "423.15 K", "overheat warning"),
            opt("phase", Kind::Text, "a", "phase a single-phase bus takes from a three-phase one"),
        ],
        outputs: TRANSFORMER_OUT,
    },
];

outputs!(CONVERTER_OUT, cvo { current: A, power: W, active: B, temperature: T, overheat: B, limiting: B, failed: B, input_power: W, voltage: V, efficiency: N });
outputs!(TRANSFORMER_OUT, tfo { current: A, power: W, temperature: T, overheat: B, failed: B });

const CONVERTER_TAU: f64 = 240.0;
const CONVERTER_BAY: f64 = 303.15;
/// Droop of an inverter's or charger's output stage at full load.
const STAGE_REGULATION: f64 = 0.02;
/// Constant-power draws are linearised at last step's voltage, never below this share of nominal.
const FLOOR: f64 = 0.5;
const DIODE_DROP_PAIR: f64 = 1.6;

#[derive(Clone, Copy, PartialEq)]
pub enum ConvKind {
    Tru,
    Inverter,
    Charger,
}

pub struct Converter {
    pub kind: ConvKind,
    base: usize,
    pub input: usize,
    pub output: usize,
    inp: Shunt,
    out: Shunt,
    voltage: f64,
    pub frequency: f64,
    pub rating: f64,
    amps: f64,
    p0: f64,
    k2: f64,
    when: Option<Expr>,
    cooled: Option<Expr>,
    limit: f64,
    overheat_at: f64,
    regulation: f64,
    regulated: bool,
    harmonic: f64,
    pf: f64,
    f_nom: f64,
    min_input: f64,
    comp: f64,
    out_phases: usize,
    /// Per-phase output conductance this step.
    g: f64,
    /// Emf this step, before limiting.
    e_src: f64,
    /// Fold-back emf the current limit holds it to.
    e_lim: f64,
    pub drawn: f64,
    pub input_w: f64,
    temp: f64,
    hot: bool,
    pub active: bool,
    feeding: bool,
    /// Oscillator phase of an inverter.
    pub theta: f64,
    pub out_i: f64,
}

impl Converter {
    pub fn new(p: &CheckedPart, kind: ConvKind, g: &mut Grid, buses: &[Bus], input: usize, output: usize) -> Self {
        let v = p.num("voltage");
        let charger = kind == ConvKind::Charger;
        let amps = if charger { p.num("current").max(0.01) } else { 0.0 };
        let rating = if charger { amps * v } else { p.num("rating").max(1.0) };
        let eff = p.num("efficiency").clamp(0.05, 1.0);
        let loss_rated = rating * (1.0 / eff - 1.0);
        let p0 = (0.03 * rating).min(0.4 * loss_rated);
        let k2 = ((loss_rated - p0) / (rating * rating)).max(0.0);
        let pulses = p.num("pulses");
        let out_phases = buses[output].phases();
        Converter {
            kind,
            base: p.slot,
            input,
            output,
            inp: Shunt::new(g, buses[input].term),
            out: Shunt::new(g, buses[output].term),
            voltage: v,
            frequency: if kind == ConvKind::Inverter { p.num("frequency") } else { 0.0 },
            rating,
            amps,
            p0,
            k2,
            when: expr_of(p, "on_when"),
            cooled: expr_of(p, "cooled_when"),
            limit: if charger { f64::INFINITY } else { p.num("current_limit").max(0.1) },
            overheat_at: p.num("overheat_temperature"),
            regulation: if kind == ConvKind::Tru { p.num("regulation").clamp(0.001, 0.5) } else { STAGE_REGULATION },
            regulated: kind == ConvKind::Tru && p.flag("regulated"),
            harmonic: match kind {
                ConvKind::Tru if pulses <= 6.0 => 0.3,
                ConvKind::Tru if pulses <= 12.0 => 0.12,
                ConvKind::Tru => 0.06,
                _ => 0.05,
            },
            pf: if kind == ConvKind::Tru { p.num("input_power_factor").clamp(0.3, 1.0) } else { 0.98 },
            f_nom: if kind == ConvKind::Tru { p.num("nominal_frequency").max(1.0) } else { 400.0 },
            min_input: if kind == ConvKind::Inverter { p.num("min_input") } else { 0.0 },
            comp: if charger { p.num("temperature_compensation") } else { 0.0 },
            out_phases,
            g: 0.0,
            e_src: v,
            e_lim: v,
            drawn: 0.0,
            input_w: 0.0,
            temp: CONVERTER_BAY,
            hot: false,
            active: false,
            feeding: false,
            theta: 0.0,
            out_i: 0.0,
        }
    }

    pub fn failed(&self, slots: &[f64]) -> bool {
        slots[self.base + cvo::failed] != 0.0
    }

    /// Decides whether it runs this step and steps its temperature.
    pub fn update(&mut self, slots: &mut [f64], dt: f64, buses: &[Bus], battery_temp: Option<(f64, bool)>) {
        let cooled = yes(&mut self.cooled, slots, dt, true);
        let share = self.drawn / self.rating;
        let target = CONVERTER_BAY + 50.0 * share * share * if cooled { 1.0 } else { 4.0 };
        self.temp += (target - self.temp) * (dt / CONVERTER_TAU).min(1.0);
        self.hot = if self.hot { self.temp > self.overheat_at - 20.0 } else { self.temp > self.overheat_at };
        let inb = &buses[self.input];
        let input_ok = inb.powered && inb.vmag >= self.min_input * inb.nominal;
        let bat_hot = battery_temp.is_some_and(|(_, hot)| hot);
        self.active = !self.failed(slots) && !self.hot && input_ok && !bat_hot && yes(&mut self.when, slots, dt, true);
        if self.kind == ConvKind::Inverter {
            self.theta = (self.theta + std::f64::consts::TAU * self.frequency * dt).rem_euclid(std::f64::consts::TAU);
        }
        let vset = match self.kind {
            ConvKind::Charger => self.voltage + self.comp * (battery_temp.map_or(298.15, |b| b.0) - 298.15),
            _ => self.voltage,
        };
        self.e_src = if self.kind == ConvKind::Tru { self.tru_emf(buses) } else { vset };
        let n = self.out_phases as f64;
        self.g = match self.kind {
            ConvKind::Tru => {
                let f = buses[self.input].freq;
                let r_rated = self.regulation * self.voltage * self.voltage / self.rating;
                let r = r_rated * (0.5 + 0.5 * f / self.f_nom);
                1.0 / r.max(1e-6)
            }
            _ => self.rating / n / (STAGE_REGULATION * vset * vset).max(1e-9),
        };
        if !self.active {
            self.e_lim = self.e_src;
        }
        flag(slots, self.base + cvo::active, self.active);
        put(slots, self.base + cvo::temperature, self.temp);
        flag(slots, self.base + cvo::overheat, self.hot);
    }

    fn tru_emf(&self, buses: &[Bus]) -> f64 {
        let inb = &buses[self.input];
        let ratio = inb.vmag / inb.nominal.max(1e-9);
        if self.regulated {
            if ratio >= 0.75 {
                self.voltage
            } else {
                self.voltage * ratio / 0.75
            }
        } else {
            // Diode pair drop is fixed; the rest scales with the AC it rectifies.
            ((self.voltage + DIODE_DROP_PAIR) * ratio - DIODE_DROP_PAIR).max(0.0)
        }
    }

    /// Output emf after current limiting.
    fn emf(&self) -> f64 {
        self.e_src.min(self.e_lim)
    }

    pub fn stamp(&mut self, g: &mut Grid, buses: &[Bus], isl: &Islands) {
        let ob = &buses[self.output];
        let e = self.emf();
        self.feeding = false;
        if self.active && ob.vmag < e + 0.02 * self.voltage {
            self.feeding = true;
            let gy = C::real(self.g);
            self.out.each(g, gy);
            let ph = if ob.kind.ac() { balanced(C::polar(e, self.theta - isl.theta[ob.island])) } else { [C::real(e), ZERO, ZERO] };
            let mut inj = ZERO3;
            for k in 0..ob.phases() {
                inj[k] = ph[k] * gy;
            }
            self.out.inject(g, &inj);
        }
        if self.active {
            let inb = &buses[self.input];
            let p = self.input_w;
            let n = inb.phases() as f64;
            let v = inb.vmag.max(FLOOR * inb.nominal);
            let y = if inb.kind.ac() {
                let q = p * (1.0 / (self.pf * self.pf) - 1.0).max(0.0).sqrt();
                C::new(p / n, -q / n) / (v * v)
            } else {
                C::real(p / (v * v))
            };
            self.inp.each(g, y);
        }
    }

    /// The DC side of a TRU takes the AC solved this step: the emf moves by `delta`, injected without refactoring.
    pub fn refresh(&mut self, g: &mut Grid, buses: &[Bus]) -> bool {
        if self.kind != ConvKind::Tru || !self.feeding {
            return false;
        }
        let before = self.emf();
        let inb = &buses[self.input];
        let v_now = g.v(&inb.term);
        let ratio = super::complex::mean_abs(&v_now, inb.phases()) / inb.nominal.max(1e-9);
        let fresh = if self.regulated {
            if ratio >= 0.75 {
                self.voltage
            } else {
                self.voltage * ratio / 0.75
            }
        } else {
            ((self.voltage + DIODE_DROP_PAIR) * ratio - DIODE_DROP_PAIR).max(0.0)
        };
        self.e_src = fresh;
        let after = self.emf();
        if (after - before).abs() < 1e-9 {
            return false;
        }
        g.inject(self.out.t.u[0], C::real(self.g * (after - before)));
        true
    }

    pub fn post(&mut self, g: &Grid, slots: &mut [f64], buses: &[Bus], isl: &mut Islands) {
        let ob = &buses[self.output];
        let vout = g.v(&ob.term);
        let e = self.emf();
        let n = ob.phases();
        let (i, v) = if self.feeding {
            if ob.kind.ac() {
                let ph = balanced(C::polar(e, self.theta - isl.theta[ob.island]));
                let mut amps = 0.0f64;
                let mut p = 0.0;
                for k in 0..n {
                    let ik = (ph[k] - vout[k]).scale(self.g);
                    amps = amps.max(ik.abs());
                    p += (vout[k] * ik.conj()).re;
                }
                let vm = super::complex::mean_abs(&vout, n);
                self.drawn = p.max(0.0);
                (amps, vm)
            } else {
                let v = vout[0].re;
                let i = (self.g * (e - v)).max(0.0);
                self.drawn = (i * v).max(0.0);
                (i, v)
            }
        } else {
            self.drawn = 0.0;
            (0.0, if ob.kind.ac() { super::complex::mean_abs(&vout, n) } else { vout[0].re })
        };
        if self.active {
            let i_lim = if self.amps > 0.0 { self.amps } else { self.limit * self.rating / n as f64 / v.max(0.3 * self.voltage) };
            self.e_lim = (self.e_lim + 0.5 * (i_lim - i) / self.g).clamp(0.0, self.e_src);
        } else {
            self.e_lim = self.e_src;
        }
        let losses = if self.active { self.p0 + self.k2 * self.drawn * self.drawn } else { 0.0 };
        self.input_w = if self.active { self.drawn + losses } else { 0.0 };
        if self.active && buses[self.input].kind.ac() {
            isl.nonlinear[buses[self.input].island] += self.harmonic * self.input_w;
        }
        self.out_i = i;
        put(slots, self.base + cvo::current, i);
        put(slots, self.base + cvo::power, self.drawn);
        flag(slots, self.base + cvo::limiting, self.active && self.e_lim < 0.999 * self.e_src);
        put(slots, self.base + cvo::input_power, self.input_w);
        put(slots, self.base + cvo::voltage, if self.feeding { v.max(0.0) } else { 0.0 });
        put(slots, self.base + cvo::efficiency, if self.input_w > 1.0 { self.drawn / self.input_w } else { 0.0 });
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.drawn, self.e_lim, self.temp, self.hot as u8 as f64, self.input_w, self.theta]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.drawn = n();
        self.e_lim = n();
        self.temp = n();
        self.hot = n() != 0.0;
        self.input_w = n();
        self.theta = n();
    }

    /// What it adds to its output island's supply capacity, for load shedding.
    pub fn capacity(&self) -> f64 {
        if self.active && self.kind != ConvKind::Charger {
            self.rating
        } else {
            0.0
        }
    }
}

pub struct Transformer {
    pub path: Path,
    base: usize,
    ratio: f64,
    z: C,
    mag: Shunt,
    y_mag: f64,
    cooled: Option<Expr>,
    overheat_at: f64,
    rating: f64,
    loss_rated: f64,
    temp: f64,
    pub i_sec: Abc,
}

impl Transformer {
    pub fn new(p: &CheckedPart, g: &mut Grid, buses: &[Bus], a: usize, b: usize) -> Option<Self> {
        if !buses[a].kind.ac() || !buses[b].kind.ac() {
            return None;
        }
        let path = Path::new(g, buses, a, b, p.text("phase"))?;
        let rating = p.num("rating").max(1.0);
        let vs = buses[b].nominal.max(1e-6);
        let n = path.phases() as f64;
        let zbase = vs * vs / (rating / n);
        let zpu = p.num("impedance").max(1e-4);
        let xr = p.num("x_r").max(0.0);
        let r = zpu / (1.0 + xr * xr).sqrt();
        let z = C::new(r, r * xr).scale(zbase);
        let vp = buses[a].nominal.max(1e-6);
        let ip_base = rating / n / vp;
        let y_mag = p.num("magnetizing").max(0.0) * ip_base / vp;
        let mag = Shunt::new(g, path.ser.a);
        Some(Transformer {
            path,
            base: p.slot,
            ratio: p.num("ratio").max(1e-6),
            z,
            mag,
            y_mag,
            cooled: expr_of(p, "cooled_when"),
            overheat_at: p.num("overheat_temperature"),
            rating,
            loss_rated: rating * r.max(0.002) + 0.003 * rating,
            temp: CONVERTER_BAY,
            i_sec: ZERO3,
        })
    }

    pub fn update(&mut self, slots: &[f64]) {
        if slots[self.base + tfo::failed] != 0.0 {
            self.path.open();
        } else {
            self.path.set(1.0);
        }
    }

    pub fn stamp(&self, g: &mut Grid, freq: f64) {
        if !self.path.conducts() {
            return;
        }
        // Leakage reactance was given at 400 Hz; it scales with the island's frequency.
        let z = C::new(self.z.re, self.z.im * freq / 400.0);
        let y = z.inv();
        let n = self.ratio;
        for k in 0..self.path.phases() {
            self.path.ser.stamp(g, k, y.scale(1.0 / (n * n)), -y.scale(1.0 / n), -y.scale(1.0 / n), y);
        }
        let ym = C::new(0.05 * self.y_mag, -self.y_mag * 400.0 / freq.max(1.0));
        self.mag.each(g, ym);
    }

    pub fn post(&mut self, g: &Grid, slots: &mut [f64], dt: f64, freq: f64) {
        let ser: &Series = &self.path.ser;
        let (va, vb) = (ser.va(g), ser.vb(g));
        let z = C::new(self.z.re, self.z.im * freq / 400.0);
        let y = z.inv();
        let mut amps = 0.0f64;
        let mut p = 0.0;
        let mut cu = 0.0;
        for k in 0..self.path.phases() {
            let i = if self.path.conducts() { y * (va[k].scale(1.0 / self.ratio) - vb[k]) } else { ZERO };
            self.i_sec[k] = i;
            amps = amps.max(i.abs());
            p += (vb[k] * i.conj()).re;
            cu += i.abs2() * self.z.re;
        }
        let cooled = yes(&mut self.cooled, slots, dt, true);
        let loss = cu + if self.path.conducts() { 0.003 * self.rating } else { 0.0 };
        let target = CONVERTER_BAY + 70.0 * loss / self.loss_rated.max(1.0) * if cooled { 1.0 } else { 3.0 };
        self.temp += (target - self.temp) * (dt / 600.0).min(1.0);
        put(slots, self.base + tfo::current, amps);
        put(slots, self.base + tfo::power, p);
        put(slots, self.base + tfo::temperature, self.temp);
        flag(slots, self.base + tfo::overheat, self.temp > self.overheat_at);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.push(self.temp);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        self.temp = it.next().unwrap_or(CONVERTER_BAY);
    }
}
