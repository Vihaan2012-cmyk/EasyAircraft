//! Ice on a surface from a Messinger heat balance: water caught at the local liquid water content and speed,
//! the share that freezes from convective and kinetic heating, thermal anti-icing or de-icing boots, melting
//! and shedding, and the lift and drag the ice costs.

use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const LEN: Type = Type::Num(Dim::LENGTH);
const WHEN: Kind = Kind::Expr(Type::Bool);
const AIR: &[&str] = &["environment.air"];

pub static SPECS: &[PartSpec] = &[PartSpec {
    domain: "ice",
    kind: "surface",
    help: "A wing, engine inlet, probe or window that ices and can be heated or de-iced.",
    fields: &[
        req("air", Kind::Ref(AIR), "outside air: its icing, temperature and true airspeed"),
        opt("heated_when", WHEN, "false", "when its protection runs"),
        opt("build_time", Kind::Quantity(Dim::TIME), "300 s", "time to full thickness at full icing, 100 kt, with all the caught water freezing"),
        opt("shed_time", Kind::Quantity(Dim::TIME), "30 s", "time for a heated surface to shed its ice once the interface melts"),
        opt("full_thickness", Kind::Quantity(Dim::LENGTH), "10 mm", "thickness counted as fully iced (ice = 1)"),
        opt("lwc", Kind::Number, "1.0", "liquid water content at full icing, g/m3 (FAR 25 appendix C continuous maximum is about 0.8)"),
        opt("method", Kind::Text, "thermal", "thermal (hot air or electric heat), boots (pneumatic de-icing), or none"),
        opt("heat_flux", Kind::Number, "20000", "thermal protection heat when on, W/m2"),
        opt("heat_share", Kind::Expr(N), "1", "share of the heat actually delivered, such as bleed pressure over nominal"),
        opt("film_coefficient", Kind::Number, "150", "convective heat transfer at 100 kt, W/m2/K; it scales with speed to the 0.8"),
        opt("boot_cycle", Kind::Quantity(Dim::TIME), "60 s", "boots: time between inflations"),
        opt("boot_minimum", Kind::Quantity(Dim::LENGTH), "6 mm", "boots: thinner ice does not crack off"),
        opt("protected_share", Kind::Number, "1", "share of the surface the protection covers; the rest takes runback ice"),
        opt("lift_loss", Kind::Number, "0.3", "share of maximum lift lost at full thickness"),
        opt("drag_rise", Kind::Number, "1.0", "share added to drag at full thickness"),
    ],
    outputs: &[
        ("ice", N),
        ("heated", B),
        ("iced", B),
        ("thickness", LEN),
        ("surface_temperature", T),
        ("freezing_fraction", N),
        ("catch_rate", N),
        ("runback", N),
        ("lift_loss", N),
        ("drag_rise", N),
        ("failed", B),
    ],
}];

const C_WATER: f64 = 4186.0;
const L_FUSION: f64 = 334_000.0;
const RHO_RIME: f64 = 880.0;
const RHO_GLAZE: f64 = 917.0;
const V_REF: f64 = 51.44;
const FALL_SPEED: f64 = 6.0;
const RECOVERY: f64 = 0.89;

#[derive(Clone, Copy, PartialEq)]
enum Method {
    Thermal,
    Boots,
    None,
}

pub struct Ice {
    air: [usize; 3],
    heated: Option<Expr>,
    share: Option<Expr>,
    method: Method,
    efficiency: f64,
    full: f64,
    lwc: f64,
    flux: f64,
    film: f64,
    shed: f64,
    cycle: f64,
    boot_min: f64,
    covered: f64,
    lift: f64,
    drag: f64,
    h: f64,
    runback: f64,
    boot_t: f64,
    shedding: bool,
    s: Slots,
}

fn slot(c: &Checked, p: &CheckedPart, field: &str, out: &str) -> Option<usize> {
    p.part(field).and_then(|i| c.slots.index.get(&format!("{}.{out}", c.parts[i].id)).copied())
}

impl Ice {
    pub fn new(c: &Checked, p: &CheckedPart) -> Option<Self> {
        let air = [slot(c, p, "air", "icing")?, slot(c, p, "air", "temperature")?, slot(c, p, "air", "airspeed")?];
        let full = p.num("full_thickness").max(1e-5);
        let lwc = p.num("lwc").max(1e-3) * 1e-3;
        let build = p.num("build_time").max(1.0);
        Some(Ice {
            air,
            heated: expr_of(p, "heated_when"),
            share: expr_of(p, "heat_share"),
            method: match p.text("method") {
                "boots" => Method::Boots,
                "none" => Method::None,
                _ => Method::Thermal,
            },
            // Collection efficiency that grows full thickness in build_time at the reference condition.
            efficiency: full * RHO_RIME / build / (lwc * V_REF),
            full,
            lwc,
            flux: p.num("heat_flux").max(0.0),
            film: p.num("film_coefficient").max(1.0),
            shed: p.num("shed_time").max(0.1),
            cycle: p.num("boot_cycle").max(1.0),
            boot_min: p.num("boot_minimum").max(0.0),
            covered: p.num("protected_share").clamp(0.0, 1.0),
            lift: p.num("lift_loss").max(0.0),
            drag: p.num("drag_rise").max(0.0),
            h: 0.0,
            runback: 0.0,
            boot_t: 0.0,
            shedding: false,
            s: Slots::of(p, &["ice", "heated", "iced", "thickness", "surface_temperature", "freezing_fraction", "catch_rate", "runback", "lift_loss", "drag_rise", "failed"]),
        })
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let failed = self.s.on(slots, "failed");
        let on = !failed && self.method != Method::None && self.heated.as_mut().is_some_and(|e| e.yes(slots, dt));
        let share = if on { self.share.as_mut().map_or(1.0, |e| e.eval(slots, dt)).clamp(0.0, 1.5) } else { 0.0 };
        let icing = slots[self.air[0]].clamp(0.0, 1.0);
        let t = slots[self.air[1]].max(150.0);
        let v = slots[self.air[2]].abs();
        let t_rec = t + RECOVERY * v * v / (2.0 * 1005.0);
        let film = (self.film * (v.max(5.0) / V_REF).powf(0.8)).max(10.0);
        // Water caught per square metre per second; precipitation still falls on a slow or parked surface.
        let caught = if t < 273.15 + 1.0 { self.efficiency * self.lwc * icing * v.max(FALL_SPEED) } else { 0.0 };
        let heat = if on && self.method == Method::Thermal { self.flux * share } else { 0.0 };
        // Heat the surface can shed while holding 0 C: convection to the recovery temperature, warming the caught water.
        let removable = film * (273.15 - t_rec) + caught * C_WATER * (273.15 - t) - heat;
        let (freeze, surface) = if removable <= 0.0 {
            (0.0, 273.15 - removable / (film + caught * C_WATER))
        } else if caught <= 0.0 {
            (0.0, t_rec - heat / film)
        } else {
            let n = (removable / (caught * L_FUSION)).min(1.0);
            let surf = if n < 1.0 { 273.15 } else { 273.15 - (removable - caught * L_FUSION) / (film + caught * C_WATER) };
            (n, surf)
        };
        let rho = if freeze >= 1.0 && surface < 263.15 { RHO_RIME } else { RHO_GLAZE };
        let protected_heat = heat > 0.0;
        let grow = freeze * caught / rho;
        // Protected area: no accretion while heated above freezing; unprotected area and runback water still ice.
        let unprotected = 1.0 - if protected_heat || on { self.covered } else { 0.0 };
        let water_left = if protected_heat && removable <= 0.0 { caught * self.covered } else { 0.0 };
        self.runback = (self.runback + (water_left / RHO_GLAZE - self.runback * 0.05) * dt).max(0.0);
        let runback_ice = if t < 273.15 { water_left / RHO_GLAZE * 0.3 * (1.0 - self.covered).max(0.1) } else { 0.0 };
        let mut dh = grow * unprotected + runback_ice;
        // Whatever warms the surface past freezing melts the ice; a heated surface also sheds it once the interface lets go.
        if removable < 0.0 && self.h > 0.0 {
            dh -= -removable / (RHO_GLAZE * L_FUSION);
            if protected_heat {
                self.shedding = true;
                dh -= self.full / self.shed;
            }
        } else if self.h <= 0.0 {
            self.shedding = false;
        }
        self.h = (self.h + dh * dt).max(0.0);
        if self.method == Method::Boots && on {
            self.boot_t += dt;
            if self.boot_t >= self.cycle {
                self.boot_t = 0.0;
                if self.h >= self.boot_min {
                    self.h *= 0.1;
                }
            }
        } else {
            self.boot_t = 0.0;
        }
        let ice = (self.h / self.full).min(2.0);
        let s = &self.s;
        s.put(slots, "ice", ice.min(1.0));
        s.flag(slots, "heated", on);
        s.flag(slots, "iced", ice > 0.2);
        s.put(slots, "thickness", self.h);
        s.put(slots, "surface_temperature", surface);
        s.put(slots, "freezing_fraction", freeze);
        s.put(slots, "catch_rate", caught);
        s.put(slots, "runback", (self.runback / self.full).min(1.0));
        s.put(slots, "lift_loss", self.lift * ice.min(1.0).sqrt());
        s.put(slots, "drag_rise", self.drag * ice.min(1.5));
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.h, self.runback, self.boot_t, self.shedding as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.h = it.next().unwrap_or(0.0);
        self.runback = it.next().unwrap_or(0.0);
        self.boot_t = it.next().unwrap_or(0.0);
        self.shedding = it.next().unwrap_or(0.0) != 0.0;
    }
}
