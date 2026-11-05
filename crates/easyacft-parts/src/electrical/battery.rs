//! Batteries as an equivalent circuit: open-circuit voltage from charge and temperature behind an ohmic
//! resistance and two polarisation RC pairs, so the voltage sags under a step load and recovers after it.
//! Usable capacity falls with cold and with rate (Peukert), with cycling and age; charge acceptance falls near
//! full as the cells gas; heat from all of it against the bay drives overheat and thermal runaway. Lithium
//! packs carry a battery management system; lead-acid electrolyte freezes when cold and flat.

use super::bus::Bus;
use super::complex::{C, ZERO};
use super::grid::{Grid, Shunt};
use super::{eval, flag, put, yes, A, B, N, Q, T, V, W, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "electrical",
    kind: "battery",
    help: "A battery: open-circuit voltage from charge and temperature behind internal and polarisation resistance, usable capacity that falls with cold, high current, cycling and age, charge acceptance that falls as it gasses near full, self-discharge, I²R and gassing heat against its bay, thermal runaway, a lithium battery management system and lead-acid freezing.",
    fields: &[
        req("bus", Kind::Ref(&["electrical.bus"]), "the bus it feeds"),
        req("voltage", Kind::Quantity(Dim::VOLTAGE), "nominal voltage, for example \"24 V\""),
        req("capacity", Kind::Quantity(Dim::CHARGE), "for example \"44 Ah\""),
        opt("resistance", Kind::Quantity(Dim::RESISTANCE), "0.02 ohm", "internal resistance at 15 °C"),
        opt("charge", Kind::Number, "1.0", "state of charge at start, 0 to 1"),
        opt("charge_limit", Kind::Quantity(Dim::CURRENT), "40 A", "most current it takes when charging"),
        opt("chemistry", Kind::Text, "nicd", "nicd, lead_acid or lithium: sets the voltage curve, Peukert exponent, charge efficiency, cold loss and runaway temperature"),
        opt("ambient", Kind::Expr(T), "288.15 K", "temperature of the bay it sits in"),
        opt("mass", Kind::Quantity(Dim::MASS), "25 kg", "for its heat capacity"),
        opt("cooling", Kind::Number, "10", "heat loss to the bay, W per K"),
        opt("health", Kind::Number, "1", "share of its rated capacity left at start, from age"),
        opt("cycle_life", Kind::Number, "0", "full cycles to 80 % health; 0 takes the chemistry's"),
        opt("polarization", Kind::Number, "0.6", "polarisation resistance as a share of the internal resistance"),
        opt("recovery_time", Kind::Quantity(Dim::TIME), "40 s", "slow polarisation time constant: how long it takes to recover after a heavy load"),
        opt("cell_failed_when", WHEN, "false", "one cell shorted: a cell's voltage lost, more resistance and heat"),
        opt("heater_when", WHEN, "false", "its heater is on"),
        opt("heater_power", Kind::Quantity(Dim::POWER), "0 W", "heater power, taken from its own bus"),
        opt("bms", Kind::Bool, "true", "lithium only: battery management system that opens on cell over- or undervoltage, overtemperature, or charging below freezing"),
    ],
    outputs: OUT,
};

outputs!(OUT, bo { voltage: V, current: A, charge: N, remaining: Q, temperature: T, overheat: B, runaway: B, failed: B, ocv: V, power: W, health: N, cell_voltage: V, bms_open: B, frozen: B, heat: W });

/// Below this share of charge a battery's open-circuit voltage falls away to nothing.
pub const KNEE: f64 = 0.1;
const T_REF: f64 = 288.15;
/// Specific heat of cells and case, J/kg/K.
const BATTERY_C: f64 = 1100.0;
/// A cell this far past its runaway temperature has vented and gone open circuit.
const DESTROYED_ABOVE: f64 = 60.0;
const SECONDS_PER_MONTH: f64 = 30.0 * 86400.0;

#[derive(Clone, Copy)]
pub struct Chemistry {
    base: f64,
    slope: f64,
    peukert: f64,
    charge_eff: f64,
    cold_per_k: f64,
    pub overheat: f64,
    runaway: f64,
    cell: f64,
    /// Open-circuit voltage change per kelvin, as a share.
    ocv_tc: f64,
    self_discharge: f64,
    cycles: f64,
    gasses: bool,
    lithium: bool,
    lead: bool,
}

fn chemistry(name: &str) -> Chemistry {
    match name {
        "lead_acid" => Chemistry { base: 0.87, slope: 0.18, peukert: 1.2, charge_eff: 0.85, cold_per_k: 0.012, overheat: 323.15, runaway: 343.15, cell: 2.0, ocv_tc: 1.0e-4, self_discharge: 0.04, cycles: 500.0, gasses: true, lithium: false, lead: true },
        "lithium" => Chemistry { base: 0.84, slope: 0.21, peukert: 1.03, charge_eff: 0.98, cold_per_k: 0.008, overheat: 343.15, runaway: 403.15, cell: 3.6, ocv_tc: 0.0, self_discharge: 0.02, cycles: 3000.0, gasses: false, lithium: true, lead: false },
        _ => Chemistry { base: 0.88, slope: 0.17, peukert: 1.05, charge_eff: 0.83, cold_per_k: 0.009, overheat: 333.15, runaway: 353.15, cell: 1.2, ocv_tc: -3.0e-4, self_discharge: 0.1, cycles: 2000.0, gasses: true, lithium: false, lead: false },
    }
}

/// Open-circuit voltage at a state of charge: a gentle slope over the useful range, a lift near full, then
/// collapse below the knee.
fn ocv(c: &Chemistry, nominal: f64, charge: f64, temp: f64) -> f64 {
    let at_knee = c.base + c.slope * KNEE;
    let top = 0.02 * ((charge - 0.9) / 0.1).clamp(0.0, 1.0).powi(2);
    let shape = if charge >= KNEE { c.base + c.slope * charge + top } else { at_knee * charge.max(0.0) / KNEE };
    nominal * shape * (1.0 + c.ocv_tc * (temp - 298.15)).max(0.5)
}

pub struct Battery {
    pub bus: usize,
    base: usize,
    sh: Shunt,
    nominal: f64,
    capacity: f64,
    g: f64,
    limit: f64,
    g_now: f64,
    pub charge: f64,
    pub chem: Chemistry,
    ambient: Option<Expr>,
    heat_capacity: f64,
    cooling: f64,
    pub temp: f64,
    amb: f64,
    destroyed: bool,
    cells: f64,
    health: f64,
    cycle_life: f64,
    pol: f64,
    tau1: f64,
    v1: f64,
    v2: f64,
    cell_failed: Option<Expr>,
    cell_short: bool,
    heater: Option<Expr>,
    heater_power: f64,
    heater_on: bool,
    heater_sh: Shunt,
    bms: bool,
    bms_open: bool,
    frozen: bool,
    e_now: f64,
    i: f64,
}

impl Battery {
    pub fn new(p: &CheckedPart, bus: usize, g: &mut Grid, buses: &[Bus]) -> Self {
        let chem = chemistry(p.text("chemistry"));
        let nominal = p.num("voltage");
        let g0 = 1.0 / p.num("resistance").max(1e-6);
        let term = buses[bus].term.phase(0);
        Battery {
            bus,
            base: p.slot,
            sh: Shunt::new(g, term),
            nominal,
            capacity: p.num("capacity").max(1.0),
            g: g0,
            limit: p.num("charge_limit").max(0.1),
            g_now: g0,
            charge: p.num("charge").clamp(0.0, 1.0),
            chem,
            ambient: expr_of(p, "ambient"),
            heat_capacity: p.num("mass").max(0.1) * BATTERY_C,
            cooling: p.num("cooling").max(0.0),
            temp: f64::NAN,
            amb: T_REF,
            destroyed: false,
            cells: (nominal / chem.cell).round().max(1.0),
            health: p.num("health").clamp(0.05, 1.0),
            cycle_life: if p.num("cycle_life") > 0.0 { p.num("cycle_life") } else { chem.cycles },
            pol: p.num("polarization").max(0.0),
            tau1: p.num("recovery_time").max(1.0),
            v1: 0.0,
            v2: 0.0,
            cell_failed: expr_of(p, "cell_failed_when"),
            cell_short: false,
            heater: expr_of(p, "heater_when"),
            heater_power: p.num("heater_power").max(0.0),
            heater_on: false,
            heater_sh: Shunt::new(g, term),
            bms: chem.lithium && p.flag("bms"),
            bms_open: false,
            frozen: false,
            e_now: 0.0,
            i: 0.0,
        }
    }

    fn r0(&self) -> f64 {
        let base = (0.02 * (T_REF - self.temp)).exp().clamp(0.3, 8.0) / self.g;
        let aged = base * (1.0 + 0.8 * (1.0 - self.health));
        let shorted = if self.cell_short { 1.3 } else { 1.0 };
        let frozen = if self.frozen { 30.0 } else { 1.0 };
        aged * shorted * frozen
    }

    fn ocv(&self) -> f64 {
        let v = ocv(&self.chem, self.nominal, self.charge, self.temp);
        if self.cell_short {
            v * (self.cells - 1.0) / self.cells
        } else {
            v
        }
    }

    fn live(&self, slots: &[f64]) -> bool {
        slots[self.base + bo::failed] == 0.0 && !self.destroyed && !self.bms_open
    }

    pub fn stamp(&mut self, g: &mut Grid, slots: &[f64], dt: f64, bus_v: f64) {
        self.amb = eval(&mut self.ambient, slots, dt, T_REF);
        if self.temp.is_nan() {
            self.temp = self.amb;
        }
        self.cell_short = yes(&mut self.cell_failed, slots, dt, false);
        self.heater_on = self.heater_power > 0.0 && yes(&mut self.heater, slots, dt, false);
        if self.chem.lead {
            let freeze = 273.15 - (8.0 + 52.0 * self.charge * self.charge);
            self.frozen = if self.frozen { self.temp < freeze + 2.0 } else { self.temp < freeze };
        }
        if self.heater_on {
            let v = bus_v.max(0.5 * self.nominal);
            self.heater_sh.each(g, C::real(self.heater_power / (v * v)));
        }
        self.e_now = 0.0;
        if !self.live(slots) {
            return;
        }
        let ocv = self.ocv();
        // The polarisation pairs sit in series with the ohmic resistance; their voltage is carried as emf.
        let e = ocv - self.v1 - self.v2;
        let g0 = 1.0 / self.r0();
        let over = bus_v - e;
        let mut gn = if over > 0.0 { g0.min(self.limit / over) } else { g0 };
        if self.frozen && over > 0.0 {
            gn = gn.min(1e-3);
        }
        if self.bms && over > 0.0 && self.temp < 273.15 {
            gn = gn.min(1e-4);
        }
        self.g_now = gn;
        self.e_now = e;
        self.sh.each(g, C::real(gn));
        self.sh.inject(g, &[C::real(gn * e), ZERO, ZERO]);
    }

    pub fn post(&mut self, g: &Grid, slots: &mut [f64], dt: f64) {
        let v = g.v1(self.sh.t.u[0]).re;
        let live = self.live(slots);
        let i = if live && self.e_now != 0.0 { self.g_now * (self.e_now - v) } else { 0.0 };
        self.i = i;
        let r0 = self.r0();
        let ocv = self.ocv();
        // Polarisation: a fast charge-transfer pair and a slow diffusion pair, stepped exactly for this current.
        let (r1, r2) = (0.6 * self.pol * r0, 0.4 * self.pol * r0);
        let (t1, t2) = (self.tau1, 1.5);
        let a1 = (-dt / t1).exp();
        let a2 = (-dt / t2).exp();
        self.v1 = i * r1 + (self.v1 - i * r1) * a1;
        self.v2 = i * r2 + (self.v2 - i * r2) * a2;
        let usable = self.capacity * self.health * (1.0 - self.chem.cold_per_k * (T_REF - self.temp).max(0.0)).max(0.3);
        let one_c = self.capacity / 3600.0;
        let full = ((self.charge - 0.9) / 0.1).clamp(0.0, 1.0);
        let accept = if self.chem.gasses { self.chem.charge_eff * (1.0 - 0.9 * full * full) } else { self.chem.charge_eff };
        let drain = if i > 0.0 { i * (i / one_c).max(1.0).powf(self.chem.peukert - 1.0) } else { i * accept };
        let leak = self.chem.self_discharge / SECONDS_PER_MONTH * self.charge;
        self.charge = (self.charge - drain * dt / usable - leak * dt).clamp(0.0, 1.0);
        // Ageing: equivalent full cycles wear the capacity toward 80 % at the cycle life.
        self.health = (self.health - 0.2 * (i.abs() * dt / (2.0 * self.capacity)) / self.cycle_life).max(0.05);
        let gassing = if i < 0.0 { (-i) * ocv * (self.chem.charge_eff - accept).max(0.0) } else { 0.0 };
        let joule = i * i * r0 + if r1 > 0.0 { self.v1 * self.v1 / r1 } else { 0.0 } + if r2 > 0.0 { self.v2 * self.v2 / r2 } else { 0.0 };
        let short_heat = if self.cell_short && live { 0.02 * self.capacity / 3600.0 * self.nominal } else { 0.0 };
        let runaway = self.temp > self.chem.runaway;
        let exo = if runaway { self.heat_capacity * 0.01 * (self.temp - self.chem.runaway + 1.0) } else { 0.0 };
        let heater = if self.heater_on { self.heater_power } else { 0.0 };
        let heat = joule + gassing + short_heat + exo + heater;
        // Implicit in the loss to the bay so a light battery in a big cooling flow cannot overshoot.
        let k = self.cooling / self.heat_capacity;
        self.temp = (self.temp + dt * (heat / self.heat_capacity + k * self.amb)) / (1.0 + dt * k);
        if self.temp > self.chem.runaway + DESTROYED_ABOVE {
            self.destroyed = true;
        }
        if self.bms {
            let cell = v / self.cells;
            let (hi, lo) = (self.chem.cell * 1.17, self.chem.cell * 0.69);
            if cell > hi || (cell < lo && i > 0.0) || self.temp > 343.15 {
                self.bms_open = true;
            } else if self.bms_open && cell < hi * 0.97 && self.temp < 318.15 && self.charge > 0.05 {
                self.bms_open = false;
            }
        }
        let b = self.base;
        put(slots, b + bo::voltage, v.max(0.0));
        put(slots, b + bo::current, i);
        put(slots, b + bo::charge, self.charge);
        put(slots, b + bo::remaining, self.charge * usable);
        put(slots, b + bo::temperature, self.temp);
        flag(slots, b + bo::overheat, self.temp > self.chem.overheat);
        flag(slots, b + bo::runaway, runaway || self.destroyed);
        put(slots, b + bo::ocv, if self.destroyed { 0.0 } else { ocv });
        put(slots, b + bo::power, i * v.max(0.0));
        put(slots, b + bo::health, self.health);
        put(slots, b + bo::cell_voltage, v.max(0.0) / self.cells);
        flag(slots, b + bo::bms_open, self.bms_open);
        flag(slots, b + bo::frozen, self.frozen);
        put(slots, b + bo::heat, heat);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        let b = |x: bool| x as u8 as f64;
        v.extend([self.charge, self.temp, b(self.destroyed), self.v1, self.v2, self.health, b(self.bms_open), b(self.frozen), self.i]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.charge = n();
        self.temp = n();
        self.destroyed = n() != 0.0;
        self.v1 = n();
        self.v2 = n();
        self.health = n();
        self.bms_open = n() != 0.0;
        self.frozen = n() != 0.0;
        self.i = n();
    }
}
