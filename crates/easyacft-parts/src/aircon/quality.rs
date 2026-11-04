//! Cabin air quality and comfort: atmospheric ozone by altitude and latitude, ozone in the cabin against the
//! certification limits, Fanger's predicted mean vote and percentage dissatisfied (ISO 7730), and the lavatory and
//! galley extract fans that pull odour and smoke out of their zones.

use super::thermo::{lag, p_sat, P_SL};
use super::yes;
use crate::util::Slots;
use crate::Expr;
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, Kind, PartSpec};
use easyacft_lang::{CheckedPart, Type};

const B: Type = Type::Bool;
const MF: Type = Type::Num(Dim::MASS_FLOW);
const W: Type = Type::Num(Dim::POWER);
const N: Type = Type::Num(Dim::NONE);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub const EXTRACT_FAN: PartSpec = PartSpec {
    domain: "aircon",
    kind: "extract_fan",
    help: "A lavatory or galley extract fan: pulls air from its zones overboard through the outflow, taking odour and smoke with it.",
    fields: &[
        opt("zones", Kind::Refs(&["aircon.zone"]), "", "zones it extracts from"),
        opt("flow", Kind::Quantity(Dim::MASS_FLOW), "0.08 kg/s", "extract flow"),
        opt("power", Kind::Quantity(Dim::POWER), "250 W", "motor power"),
        opt("on_when", WHEN, "true", "running"),
        opt("powered_when", WHEN, "true", "its motor has power"),
    ],
    outputs: &[("running", B), ("flow", MF), ("power", W), ("speed", N), ("fault", B), ("failed", B)],
};

/// Ozone in outside air, ppb by volume, from pressure altitude (m) and latitude (degrees). Tropospheric background
/// below the tropopause; above it, the ozone layer's lower edge, highest toward the poles in spring.
pub fn ambient_ozone(altitude: f64, latitude: f64) -> f64 {
    let lat = latitude.abs().min(90.0);
    let tropopause = 16_500.0 - 8_000.0 * (lat / 90.0).powf(0.6);
    let surface = 40.0;
    if altitude < tropopause - 1500.0 {
        return surface + 25.0 * (altitude.max(0.0) / tropopause).min(1.0);
    }
    let above = altitude - (tropopause - 1500.0);
    let rate = 0.12 + 0.10 * (lat / 90.0);
    (surface + 25.0 + rate * above).min(3000.0)
}

/// Fanger's predicted mean vote and predicted percentage dissatisfied (ISO 7730). Air and mean radiant temperature in
/// kelvin, air speed m/s, relative humidity 0 to 1, metabolic rate in met, clothing in clo.
pub fn pmv(ta: f64, tr: f64, vel: f64, rh: f64, met: f64, clo: f64) -> (f64, f64) {
    let (ta, tr) = (ta - 273.15, tr - 273.15);
    let pa = rh.clamp(0.0, 1.0) * p_sat(ta + 273.15);
    let icl = 0.155 * clo;
    let m = met * 58.15;
    let w = 0.0;
    let mw = m - w;
    let fcl = if icl <= 0.078 { 1.0 + 1.29 * icl } else { 1.05 + 0.645 * icl };
    let hcf = 12.1 * vel.max(0.0).sqrt();
    let taa = ta + 273.0;
    let tra = tr + 273.0;
    let mut tcla = taa + (35.5 - ta) / (3.5 * icl + 0.1);
    let p1 = icl * fcl;
    let p2 = p1 * 3.96;
    let p3 = p1 * 100.0;
    let p4 = p1 * taa;
    let p5 = 308.7 - 0.028 * mw + p2 * (tra / 100.0).powi(4);
    let mut xn = tcla / 100.0;
    let mut xf = xn;
    let mut hc = hcf;
    for _ in 0..150 {
        xf = (xf + xn) / 2.0;
        let hcn = 2.38 * (100.0 * xf - taa).abs().powf(0.25);
        hc = hcf.max(hcn);
        xn = (p5 + p4 * hc - p2 * xf.powi(4)) / (100.0 + p3 * hc);
        if (xn - xf).abs() < 1.5e-5 {
            break;
        }
    }
    tcla = 100.0 * xn;
    let tcl = tcla - 273.0;
    let hl1 = 3.05e-3 * (5733.0 - 6.99 * mw - pa);
    let hl2 = if mw > 58.15 { 0.42 * (mw - 58.15) } else { 0.0 };
    let hl3 = 1.7e-5 * m * (5867.0 - pa);
    let hl4 = 0.0014 * m * (34.0 - ta);
    let hl5 = 3.96 * fcl * (xn.powi(4) - (tra / 100.0).powi(4));
    let hl6 = fcl * hc * (tcl - ta);
    let ts = 0.303 * (-0.036 * m).exp() + 0.028;
    let vote = (ts * (mw - hl1 - hl2 - hl3 - hl4 - hl5 - hl6)).clamp(-3.0, 3.0);
    let ppd = 100.0 - 95.0 * (-0.03353 * vote.powi(4) - 0.2179 * vote.powi(2)).exp();
    (vote, ppd)
}

/// Ozone a zone holds, ppb, with the certification averages: the peak and the three-hour time-weighted average,
/// both as sea-level equivalents.
#[derive(Clone, Copy, Debug)]
pub struct Ozone {
    pub ppb: f64,
    pub twa: f64,
}

/// Ozone reacts on cabin surfaces and with occupants, per second.
const OZONE_SURFACE: f64 = 2.5 / 3600.0;
const OZONE_PER_PERSON: f64 = 0.02 / 3600.0;
const TWA_WINDOW: f64 = 3.0 * 3600.0;
/// Certification limits: 0.25 ppm peak above FL320 and 0.1 ppm averaged over three hours above FL270.
pub const OZONE_PEAK: f64 = 250.0;
pub const OZONE_TWA: f64 = 100.0;

impl Ozone {
    pub fn new() -> Self {
        Ozone { ppb: 30.0, twa: 30.0 }
    }

    pub fn step(&mut self, supply_ppb: f64, supply_flow: f64, air_mass: f64, occupants: f64, cabin_p: f64, dt: f64) {
        let removal = OZONE_SURFACE + OZONE_PER_PERSON * occupants;
        let m = air_mass.max(0.01);
        self.ppb = (m * self.ppb + dt * supply_flow * supply_ppb) / (m + dt * (supply_flow + removal * m));
        let sea_level = self.ppb * cabin_p / P_SL;
        self.twa = lag(self.twa, sea_level, TWA_WINDOW, dt);
    }

    pub fn sea_level(&self, cabin_p: f64) -> f64 {
        self.ppb * cabin_p / P_SL
    }
}

impl Default for Ozone {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ExtractFan {
    pub zones: Vec<usize>,
    flow: f64,
    power: f64,
    on: Option<Expr>,
    powered: Option<Expr>,
    pub speed: f64,
    s: Slots,
}

impl ExtractFan {
    pub fn new(p: &CheckedPart, zones: Vec<usize>) -> Self {
        ExtractFan {
            zones,
            flow: p.num("flow").max(0.0),
            power: p.num("power").max(0.0),
            on: crate::expr_of(p, "on_when"),
            powered: crate::expr_of(p, "powered_when"),
            speed: 0.0,
            s: Slots::of(p, &["running", "flow", "power", "speed", "fault", "failed"]),
        }
    }

    /// Run the fan; returns the flow it takes from each of its zones.
    pub fn step(&mut self, slots: &mut [f64], dt: f64) -> f64 {
        let failed = self.s.on(slots, "failed");
        let commanded = yes(&mut self.on, slots, dt, true);
        let run = !failed && commanded && yes(&mut self.powered, slots, dt, true);
        self.speed = lag(self.speed, if run { 1.0 } else { 0.0 }, 2.0, dt);
        let flow = self.flow * self.speed;
        let s = &self.s;
        s.flag(slots, "running", run && self.speed > 0.8);
        s.put(slots, "flow", flow);
        s.put(slots, "power", if run { self.power * self.speed.powi(3).max(0.2) } else { 0.0 });
        s.put(slots, "speed", self.speed);
        s.flag(slots, "fault", commanded && !run);
        flow / self.zones.len().max(1) as f64
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.push(self.speed);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.speed = it.next().unwrap_or(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmv_matches_the_iso_7730_reference_cases() {
        let (v, p) = pmv(295.15, 295.15, 0.1, 0.6, 1.2, 0.5);
        assert!((v - (-0.75)).abs() < 0.1 && (p - 17.0).abs() < 3.0, "{v:.2} {p:.1}");
        let (v, _) = pmv(300.15, 300.15, 0.1, 0.5, 1.2, 0.5);
        assert!(v > 0.6 && v < 1.2, "{v:.2}");
    }

    #[test]
    fn ozone_rises_above_the_tropopause_and_toward_the_poles() {
        assert!(ambient_ozone(0.0, 45.0) < 70.0);
        let mid = ambient_ozone(11_000.0, 45.0);
        let polar = ambient_ozone(11_000.0, 70.0);
        assert!(mid > 150.0 && polar > mid, "{mid:.0} {polar:.0}");
    }
}
