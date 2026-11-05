//! Feeder cables between buses: resistance from gauge, length and conductor temperature, inductive reactance
//! at the island's frequency, I²R heating against the bay, insulation breakdown, chafing to structure and a
//! broken conductor.

use super::bus::Bus;
use super::complex::{C, ZERO};
use super::grid::{Grid, Shunt};
use super::switchgear::Path;
use super::{eval, flag, put, yes, A, B, T, V, W, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "electrical",
    kind: "feeder",
    help: "A feeder cable between two buses: resistance from its gauge, length, conductor count and temperature, inductive reactance at the island's frequency, I²R heating, insulation that breaks down when cooked, chafing through to structure and a broken conductor.",
    fields: &[
        req("between", Kind::Refs(&["electrical.bus"]), "two bus ids"),
        req("length", Kind::Quantity(Dim::LENGTH), "route length"),
        opt("gauge", Kind::Text, "8", "AWG size: 22 to 4/0"),
        opt("area", Kind::Number, "0", "conductor cross-section in square millimetres, instead of a gauge"),
        opt("conductors", Kind::Number, "1", "conductors in parallel per phase"),
        opt("material", Kind::Text, "copper", "copper or aluminium"),
        opt("ambient", Kind::Expr(T), "313.15 K", "temperature of the zone it runs through"),
        opt("insulation_limit", Kind::Quantity(Dim::TEMPERATURE), "473.15 K", "conductor temperature its insulation survives; PTFE is about 200 C"),
        opt("open_when", WHEN, "false", "a conductor broken"),
        opt("chafed_when", WHEN, "false", "insulation chafed through: its far end touches structure"),
        opt("fault_resistance", Kind::Quantity(Dim::RESISTANCE), "0.02 ohm", "resistance of the chafe to structure"),
        opt("phase", Kind::Text, "a", "phase a single-phase bus takes from a three-phase one"),
    ],
    outputs: OUT,
};

outputs!(OUT, fo { current: A, temperature: T, voltage_drop: V, power_loss: W, overheat: B, insulation_failed: B, failed: B });

/// Inductance of a single conductor run over structure, henry per metre.
const L_PER_M: f64 = 0.6e-6;

fn awg_mm2(g: &str) -> f64 {
    match g.trim() {
        "22" => 0.326,
        "20" => 0.518,
        "18" => 0.823,
        "16" => 1.31,
        "14" => 2.08,
        "12" => 3.31,
        "10" => 5.26,
        "6" => 13.3,
        "4" => 21.2,
        "2" => 33.6,
        "1" => 42.4,
        "0" | "1/0" => 53.5,
        "2/0" => 67.4,
        "3/0" => 85.0,
        "4/0" => 107.0,
        _ => 8.37,
    }
}

pub struct Feeder {
    pub path: Path,
    base: usize,
    length: f64,
    /// Resistance per metre at 20 C of all conductors of one phase together.
    r20: f64,
    alpha: f64,
    /// Heat capacity per metre and loss to the zone per metre per kelvin.
    cap: f64,
    cool: f64,
    ambient: Option<Expr>,
    amb: f64,
    limit: f64,
    open: Option<Expr>,
    chafed: Option<Expr>,
    chafe_g: f64,
    chafe: Shunt,
    is_chafed: bool,
    temp: f64,
    insulation_gone: bool,
    r_now: f64,
}

impl Feeder {
    pub fn new(p: &CheckedPart, path: Path, g: &mut Grid, buses: &[Bus]) -> Self {
        let area = if p.num("area") > 0.0 { p.num("area") } else { awg_mm2(p.text("gauge")) } * 1e-6;
        let n = p.num("conductors").max(1.0);
        let alu = p.text("material").starts_with("alu");
        let (rho, alpha, density, c) = if alu { (2.82e-8, 0.00403, 2700.0, 900.0) } else { (1.72e-8, 0.00393, 8960.0, 385.0) };
        let d = (4.0 * area / std::f64::consts::PI).sqrt() + 1.0e-3;
        let chafe = Shunt::new(g, buses[path.b].term);
        Feeder {
            path,
            base: p.slot,
            length: p.num("length").max(0.01),
            r20: rho / (area * n),
            alpha,
            cap: area * n * density * c * 1.15,
            cool: 12.0 * std::f64::consts::PI * d * n.sqrt(),
            ambient: expr_of(p, "ambient"),
            amb: 313.15,
            limit: p.num("insulation_limit"),
            open: expr_of(p, "open_when"),
            chafed: expr_of(p, "chafed_when"),
            chafe_g: 1.0 / p.num("fault_resistance").max(1e-6),
            chafe,
            is_chafed: false,
            temp: f64::NAN,
            insulation_gone: false,
            r_now: rho / (area * n),
        }
    }

    pub fn update(&mut self, slots: &mut [f64], dt: f64) {
        self.amb = eval(&mut self.ambient, slots, dt, 313.15);
        if self.temp.is_nan() {
            self.temp = self.amb;
        }
        let failed = slots[self.base + fo::failed] != 0.0;
        let open = failed || yes(&mut self.open, slots, dt, false);
        self.is_chafed = !failed && (yes(&mut self.chafed, slots, dt, false) || self.insulation_gone);
        self.r_now = self.r20 * (1.0 + self.alpha * (self.temp - 293.15)) * self.length;
        if open {
            self.path.open();
        } else {
            self.path.set(1.0);
        }
    }

    pub fn stamp(&mut self, g: &mut Grid, freq: f64) {
        if self.path.conducts() {
            let x = if self.path.ac { std::f64::consts::TAU * freq * L_PER_M * self.length } else { 0.0 };
            let y = C::new(self.r_now, x).inv();
            for k in 0..self.path.phases() {
                self.path.y[k] = y;
            }
            self.path.stamp(g);
        }
        if self.is_chafed {
            self.chafe.each(g, C::real(self.chafe_g));
        }
    }

    pub fn post(&mut self, g: &Grid, slots: &mut [f64], dt: f64) {
        self.path.read(g);
        let amps = self.path.amps();
        let n = self.path.phases();
        let i2: f64 = self.path.i[..n].iter().map(|i| i.abs2()).sum::<f64>() / n as f64;
        let loss_per_m = i2 * self.r_now / self.length;
        // Implicit step of the conductor temperature, so a fault current cannot overshoot it.
        let k = self.cool / self.cap;
        self.temp = (self.temp + dt * (loss_per_m / self.cap + k * self.amb)) / (1.0 + dt * k);
        if self.temp > self.limit {
            self.insulation_gone = true;
        }
        let drop = if self.path.conducts() { self.path.across(g) } else { 0.0 };
        put(slots, self.base + fo::current, amps);
        put(slots, self.base + fo::temperature, self.temp);
        put(slots, self.base + fo::voltage_drop, drop);
        put(slots, self.base + fo::power_loss, i2 * self.r_now * n as f64);
        flag(slots, self.base + fo::overheat, self.temp > self.limit - 40.0);
        flag(slots, self.base + fo::insulation_failed, self.insulation_gone);
        if !self.path.conducts() {
            self.path.i = [ZERO; 3];
        }
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.temp, self.insulation_gone as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        self.temp = it.next().unwrap_or(f64::NAN);
        self.insulation_gone = it.next().unwrap_or(0.0) != 0.0;
    }
}
