//! Outside air: bound from X-Plane, or a standard atmosphere from altitude with an ISA deviation; either way the
//! derived values every system wants.

use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, Kind, PartSpec};
use easyacft_lang::{CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const LEN: Type = Type::Num(Dim::LENGTH);
const SPD: Type = Type::Num(Dim::SPEED);

pub static SPECS: &[PartSpec] = &[PartSpec {
    domain: "environment",
    kind: "air",
    help: "Outside air and flight state: bind these from X-Plane, or let it compute a standard atmosphere from altitude; density, speed of sound, dynamic and total pressure, EAS and CAS follow either way.",
    fields: &[
        opt("standard", Kind::Bool, "false", "compute pressure, temperature and Mach from altitude and airspeed with the ISA (to 32 km)"),
        opt("isa_deviation", Kind::Expr(Type::Num(Dim::TEMPERATURE)), "0 K", "standard atmosphere: temperature above ISA"),
        opt("qnh", Kind::Expr(P), "1013.25 hPa", "standard atmosphere: sea-level pressure"),
        opt("wind_speed", Kind::Expr(SPD), "0 m/s", "headwind component, for airspeed from groundspeed when airspeed is not bound"),
    ],
    outputs: &[
        ("pressure", P),
        ("temperature", T),
        ("altitude", LEN),
        ("airspeed", SPD),
        ("groundspeed", SPD),
        ("vertical_speed", SPD),
        ("mach", N),
        ("on_ground", B),
        ("icing", N),
        ("rain", N),
        ("aoa", N),
        ("sideslip", N),
        ("heading", N),
        ("pitch", N),
        ("roll", N),
        ("latitude", N),
        ("longitude", N),
        ("density", Type::Num(Dim::DENSITY)),
        ("speed_of_sound", SPD),
        ("dynamic_pressure", P),
        ("total_pressure", P),
        ("total_temperature", T),
        ("pressure_altitude", LEN),
        ("density_altitude", LEN),
        ("eas", SPD),
        ("cas", SPD),
    ],
}];

const R: f64 = 287.052_87;
const G0: f64 = 9.806_65;
const GAMMA: f64 = 1.4;
const P0: f64 = 101_325.0;
const T0: f64 = 288.15;
const RHO0: f64 = 1.225;

/// ISA temperature and pressure at a geopotential altitude, three layers to 32 km.
pub fn isa(h: f64, p_sl: f64) -> (f64, f64) {
    let layers = [(0.0, T0, -0.0065), (11_000.0, 216.65, 0.0), (20_000.0, 216.65, 0.001), (32_000.0, 228.65, 0.0028)];
    let h = h.clamp(-1000.0, 47_000.0);
    let mut p = p_sl;
    for (k, &(h0, t_base, lapse)) in layers.iter().enumerate() {
        let top = layers.get(k + 1).map_or(f64::INFINITY, |l| l.0);
        let dh = h.min(top) - h0;
        let t = t_base + lapse * dh;
        p *= if lapse == 0.0 { (-G0 * dh / (R * t_base)).exp() } else { (t / t_base).powf(-G0 / (lapse * R)) };
        if h <= top {
            return (t, p);
        }
    }
    (layers[3].1, p)
}

/// Pressure altitude of a static pressure, inverting the ISA layer by layer.
pub fn pressure_altitude(p: f64) -> f64 {
    let (mut lo, mut hi) = (-1000.0, 47_000.0);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if isa(mid, P0).1 > p { lo = mid } else { hi = mid }
    }
    0.5 * (lo + hi)
}

pub struct Air {
    standard: bool,
    e: [Option<Expr>; 3],
    wrote: Option<f64>,
    s: Slots,
}

impl Air {
    pub fn new(p: &CheckedPart) -> Self {
        Air {
            standard: p.flag("standard"),
            e: ["isa_deviation", "qnh", "wind_speed"].map(|k| expr_of(p, k)),
            wrote: None,
            s: Slots::of(p, &["pressure", "temperature", "altitude", "airspeed", "groundspeed", "mach", "density", "speed_of_sound", "dynamic_pressure", "total_pressure", "total_temperature", "pressure_altitude", "density_altitude", "eas", "cas"]),
        }
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let ev = |e: &mut Option<Expr>, s: &[f64]| e.as_mut().map_or(0.0, |x| x.eval(s, dt));
        let s = &self.s;
        if self.standard {
            let dev = ev(&mut self.e[0], slots);
            let qnh = { let q = ev(&mut self.e[1], slots); if q > 1000.0 { q } else { P0 } };
            let (t, p) = isa(s.get(slots, "altitude"), qnh);
            s.put(slots, "pressure", p);
            s.put(slots, "temperature", t + dev);
            let wind = ev(&mut self.e[2], slots);
            let v = s.get(slots, "airspeed");
            if v == 0.0 || self.wrote == Some(v) {
                let v = s.get(slots, "groundspeed") + wind;
                s.put(slots, "airspeed", v);
                self.wrote = Some(v);
            }
        }
        let (p, t, v) = (s.get(slots, "pressure").max(1.0), s.get(slots, "temperature").max(100.0), s.get(slots, "airspeed"));
        let rho = p / (R * t);
        let a = (GAMMA * R * t).sqrt();
        let m = v.abs() / a;
        if self.standard {
            s.put(slots, "mach", m);
        }
        let q = 0.5 * rho * v * v;
        let pt = p * (1.0 + 0.2 * m * m).powf(3.5);
        let qc = pt - p;
        let cas = (2.0 * GAMMA / (GAMMA - 1.0) * P0 / RHO0 * ((qc / P0 + 1.0).powf((GAMMA - 1.0) / GAMMA) - 1.0)).max(0.0).sqrt();
        let pa = pressure_altitude(p);
        // Density altitude: the ISA height whose density this is.
        let da = {
            let (mut lo, mut hi) = (-2000.0, 47_000.0);
            for _ in 0..50 {
                let mid = 0.5 * (lo + hi);
                let (ti, pi) = isa(mid, P0);
                if pi / (R * ti) > rho { lo = mid } else { hi = mid }
            }
            0.5 * (lo + hi)
        };
        s.put(slots, "density", rho);
        s.put(slots, "speed_of_sound", a);
        s.put(slots, "dynamic_pressure", q);
        s.put(slots, "total_pressure", pt);
        s.put(slots, "total_temperature", t * (1.0 + 0.2 * m * m));
        s.put(slots, "pressure_altitude", pa);
        s.put(slots, "density_altitude", da);
        s.put(slots, "eas", v * (rho / RHO0).sqrt());
        s.put(slots, "cas", cas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_atmosphere_matches_its_tables() {
        let (t, p) = isa(11_000.0, P0);
        assert!((t - 216.65).abs() < 0.01 && (p - 22_632.0).abs() < 5.0, "tropopause {t} {p}");
        let (t, p) = isa(20_000.0, P0);
        assert!((t - 216.65).abs() < 0.01 && (p - 5_474.9).abs() < 3.0, "{t} {p}");
        let (t, p) = isa(32_000.0, P0);
        assert!((t - 228.65).abs() < 0.01 && (p - 868.0).abs() < 2.0, "{t} {p}");
        assert!((pressure_altitude(22_632.0) - 11_000.0).abs() < 2.0);
        let (_, p) = isa(10_668.0, P0);
        assert!((p - 23_842.0).abs() < 20.0, "FL350: {p}");
    }
}
