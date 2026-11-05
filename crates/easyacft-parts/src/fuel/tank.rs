//! Fuel tanks: fuel mass, temperature from the skin, from heat dumped into it and from what flows in, density and
//! volume with temperature, wax and freezing; the fuel surface tilting with attitude and sloshing with
//! acceleration so a low tank uncovers its pump inlets; the ullage vented to outside or blocked; dissolved
//! water coming out as it cools, settling in the sump and freezing; capacitance probes that read volume and
//! a densitometer, fooled by water; overflow to a surge tank.

use super::props::Fuel as Props;
use super::{eval, flag, put, yes, B, L, M, N, P, T, VOL, W, WHEN};
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "fuel",
    kind: "tank",
    help: "A fuel tank: temperature from the skin, from heat dumped into it and from what flows in; density, volume, wax and freezing by fuel grade; a fuel surface that tilts with attitude and sloshes with acceleration and uncovers pump inlets; a vented or blocked ullage; dissolved water that comes out as the fuel cools, settles, freezes and can be drained; capacitance probes fooled by water; unusable fuel and overflow to a surge tank.",
    fields: &[
        req("capacity", Kind::Quantity(Dim::MASS), "usable fuel when full, for example \"2400 kg\""),
        opt("quantity", Kind::Quantity(Dim::MASS), "0 kg", "fuel at start"),
        opt("arm", Kind::Quantity(Dim::LENGTH), "0 m", "for weight and balance"),
        opt("gauge_error", Kind::Number, "0", "share the gauge reads high (negative: low)"),
        opt("unusable", Kind::Quantity(Dim::MASS), "0 kg", "fuel the pumps cannot reach in level flight"),
        opt("overflow_to", Kind::Ref(&["fuel.tank"]), "", "surge or vent tank that takes what overflows; without one it spills overboard"),
        opt("temperature", Kind::Quantity(Dim::TEMPERATURE), "288.15 K", "fuel temperature at start"),
        opt("skin_temperature", Kind::Expr(T), "288.15 K", "temperature of the skin and air around the tank"),
        opt("heat", Kind::Expr(W), "0 W", "heat put into the fuel, such as from hydraulic and drive oil coolers"),
        opt("cooling", Kind::Number, "0", "heat loss to the skin, W per K; 0 means 0.1 W/K per kg of capacity"),
        opt("freeze_point", Kind::Quantity(Dim::TEMPERATURE), "0 K", "fuel freezing point; 0 K takes the grade's"),
        opt("density", Kind::Number, "0", "fuel density at 15 C, kg/m3; 0 takes the grade's"),
        opt("grade", Kind::Text, "jet_a", "jet_a, jet_a1, jp8, jp4 or avgas"),
        opt("depth", Kind::Quantity(Dim::LENGTH), "0.8 m", "tank depth at its pump inlets"),
        opt("inlet_x", Kind::Quantity(Dim::LENGTH), "0 m", "pump inlets ahead of the tank's middle"),
        opt("inlet_y", Kind::Quantity(Dim::LENGTH), "0 m", "pump inlets outboard (right) of the tank's middle"),
        opt("length", Kind::Quantity(Dim::LENGTH), "4 m", "tank length fore and aft, for how far the surface can tilt"),
        opt("span", Kind::Quantity(Dim::LENGTH), "6 m", "tank span, for how far the surface can tilt"),
        opt("pitch", Kind::Expr(N), "0", "aircraft pitch, for the fuel surface"),
        opt("roll", Kind::Expr(N), "0", "aircraft roll"),
        opt("accel_x", Kind::Expr(ACC), "0 m/s2", "longitudinal acceleration, forward positive"),
        opt("accel_y", Kind::Expr(ACC), "0 m/s2", "lateral acceleration, right positive"),
        opt("ambient", Kind::Expr(P), "101325 Pa", "outside static pressure at the vent"),
        opt("vent_pressure", Kind::Expr(P), "0 Pa", "ram pressure the vent scoop adds"),
        opt("vent_blocked_when", WHEN, "false", "vent iced or blocked: the ullage stops following outside"),
        opt("water", Kind::Number, "40", "dissolved water at start, parts per million by mass"),
        opt("drain_when", WHEN, "false", "sump drain open: free water runs out"),
        opt("probe_failed_when", WHEN, "false", "quantity probes failed: the gauge holds its last reading"),
    ],
    outputs: OUT,
};

const ACC: easyacft_lang::Type = easyacft_lang::Type::Num(Dim([0, 1, -2, 0, 0]));

outputs!(OUT, ko {
    quantity: M, indicated: M, mass: M, arm: L, volume: VOL, temperature: T, low: B, empty: B, frozen: B, spilled: M, failed: B,
    level: N, inlet_depth: L, inlet_uncovered: B, ullage_pressure: P, head: P, boiling: B, free_water: M, dissolved: N, ice: B,
    density: N, viscosity: N, vapour_pressure: P, surface_tilt: N
});

const T_REF: f64 = 288.15;
const G0: f64 = 9.80665;
const STRUCTURE_KG: f64 = 50.0;
/// Slosh: natural frequency and damping of the fuel surface.
const SLOSH_W: f64 = 2.0;
const SLOSH_Z: f64 = 0.25;

pub struct Tank {
    base: usize,
    pub node: usize,
    pub capacity: f64,
    pub quantity: f64,
    error: f64,
    pub unusable: f64,
    pub overflow: Option<usize>,
    pub spilled: f64,
    pub temp: f64,
    skin: Option<Expr>,
    heat_e: Option<Expr>,
    cooling: f64,
    pub fuel: Props,
    depth: f64,
    inlet: (f64, f64),
    size: (f64, f64),
    att: [Option<Expr>; 4],
    ambient: Option<Expr>,
    vent: Option<Expr>,
    vent_blocked: Option<Expr>,
    drain: Option<Expr>,
    probe_failed: Option<Expr>,
    /// Surface tilt about the pitch and roll axes, rad, and their rates, for slosh.
    tilt: [f64; 2],
    tilt_rate: [f64; 2],
    pub ullage: f64,
    ullage_set: bool,
    pub water: f64,
    pub free_water: f64,
    indicated: f64,
    pub head: f64,
    pub inlet_depth: f64,
    pub failed: bool,
    amb: f64,
}

impl Tank {
    pub fn new(p: &CheckedPart, node: usize) -> Self {
        let capacity = p.num("capacity").max(1e-6);
        let cooling = p.num("cooling");
        let fz = p.num("freeze_point");
        let rho = p.num("density");
        let fuel = Props::parse(p.text("grade"), (fz > 0.0).then_some(fz), (rho > 0.0).then_some(rho));
        Tank {
            base: p.slot,
            node,
            capacity,
            quantity: p.num("quantity").min(capacity),
            error: p.num("gauge_error"),
            unusable: p.num("unusable").max(0.0),
            overflow: None,
            spilled: 0.0,
            temp: p.num("temperature"),
            skin: expr_of(p, "skin_temperature"),
            heat_e: expr_of(p, "heat"),
            cooling: if cooling > 0.0 { cooling } else { 0.1 * capacity },
            fuel,
            depth: p.num("depth").max(0.05),
            inlet: (p.num("inlet_x"), p.num("inlet_y")),
            size: (p.num("length").max(0.1), p.num("span").max(0.1)),
            att: ["pitch", "roll", "accel_x", "accel_y"].map(|k| expr_of(p, k)),
            ambient: expr_of(p, "ambient"),
            vent: expr_of(p, "vent_pressure"),
            vent_blocked: expr_of(p, "vent_blocked_when"),
            drain: expr_of(p, "drain_when"),
            probe_failed: expr_of(p, "probe_failed_when"),
            tilt: [0.0; 2],
            tilt_rate: [0.0; 2],
            ullage: 101_325.0,
            ullage_set: false,
            water: p.num("water").max(0.0) * 1e-6,
            free_water: 0.0,
            indicated: p.num("quantity").min(capacity),
            head: 0.0,
            inlet_depth: 0.0,
            failed: false,
            amb: 101_325.0,
        }
    }

    pub fn density(&self) -> f64 {
        self.fuel.density(self.temp)
    }

    /// Share of the tank's height the fuel stands at, level.
    fn fill(&self) -> f64 {
        (self.quantity / self.capacity).clamp(0.0, 1.0)
    }

    /// Attitude, slosh, vent and the head the fuel gives its inlets, for this domain step.
    pub fn begin(&mut self, slots: &[f64], dt: f64) {
        self.failed = slots[self.base + ko::failed] != 0.0;
        let [pitch, roll, ax, ay] = [0, 1, 2, 3].map(|k| eval(&mut self.att[k], slots, dt, 0.0));
        // The surface lies normal to apparent gravity: attitude plus what acceleration adds, through a damped slosh.
        let want = [pitch + (ax / G0).atan(), roll - (ay / G0).atan()];
        for k in 0..2 {
            let acc = SLOSH_W * SLOSH_W * (want[k] - self.tilt[k]) - 2.0 * SLOSH_Z * SLOSH_W * self.tilt_rate[k];
            self.tilt_rate[k] += acc * dt;
            self.tilt[k] += self.tilt_rate[k] * dt;
        }
        self.amb = eval(&mut self.ambient, slots, dt, 101_325.0).max(0.0);
        let vent = eval(&mut self.vent, slots, dt, 0.0);
        if !self.ullage_set {
            self.ullage = self.amb + vent;
            self.ullage_set = true;
        }
        if !yes(&mut self.vent_blocked, slots, dt, false) {
            self.ullage += (self.amb + vent - self.ullage) * (dt / 2.0).min(1.0);
        }
        // Fuel standing over the inlets, the tilt lifting or burying them; unusable fuel sits below them.
        let usable_fill = ((self.quantity - self.unusable) / (self.capacity - self.unusable).max(1e-6)).clamp(0.0, 1.0);
        // Nose up runs the fuel aft, away from forward inlets; right wing down runs it right.
        let lift = -self.inlet.0 * self.tilt[0].tan() + self.inlet.1 * self.tilt[1].tan();
        let slope = 0.5 * (self.size.0 * self.tilt[0].tan().abs() + self.size.1 * self.tilt[1].tan().abs()) * (1.0 - usable_fill);
        self.inlet_depth = (usable_fill * self.depth + lift - 0.3 * slope).max(0.0);
        let rho = self.density();
        let wet = if self.quantity > self.unusable { 1.0 } else { 0.0 };
        self.head = (self.ullage - self.amb) + rho * G0 * (self.inlet_depth + 0.02 * wet);
    }

    pub fn uncovered(&self) -> bool {
        self.inlet_depth <= 1e-4 || self.quantity <= self.unusable
    }

    /// Mixes in what flowed in, then heat to and from the skin, water coming out of solution and settling.
    #[allow(clippy::too_many_arguments)]
    pub fn finish(&mut self, slots: &mut [f64], dt: f64, net_out: f64, m_in: f64, h_in: f64, water_in: f64, water_out: f64) -> f64 {
        let skin = eval(&mut self.skin, slots, dt, T_REF);
        let heat = eval(&mut self.heat_e, slots, dt, 0.0);
        let cp = self.fuel.cp;
        let mass = self.quantity + STRUCTURE_KG;
        self.temp = (mass * cp * self.temp + dt * (cp * h_in + heat + self.cooling * skin)) / (mass * cp + dt * (cp * m_in + self.cooling));
        let q = self.quantity - net_out * dt;
        let overflow = (q - self.capacity).max(0.0);
        self.quantity = q.clamp(0.0, self.capacity);
        // Water: dissolved water above saturation drops out as free water, which sinks to the sump.
        self.water = (self.water * (self.quantity + 1.0) + water_in * dt - water_out * dt).max(0.0) / (self.quantity + 1.0);
        let sat = self.fuel.water_saturation(self.temp);
        if self.water > sat {
            let drop = (self.water - sat) * self.quantity * (dt / 300.0).min(1.0);
            self.free_water += drop;
            self.water -= drop / self.quantity.max(1.0);
        } else if self.free_water > 0.0 && self.temp > 273.15 {
            let back = ((sat - self.water) * self.quantity).min(self.free_water) * (dt / 3600.0).min(1.0);
            self.free_water -= back;
            self.water += back / self.quantity.max(1.0);
        }
        if yes(&mut self.drain, slots, dt, false) {
            self.free_water = (self.free_water - 0.05 * dt).max(0.0);
        }
        let rho = self.density();
        let probe_failed = yes(&mut self.probe_failed, slots, dt, false);
        if !probe_failed {
            // Probes measure the wetted height, so volume, and a densitometer gives mass; free water reads as fuel.
            let water_reads = self.free_water * 6.0;
            self.indicated = (self.quantity + water_reads) * (1.0 + self.error);
        }
        let b = self.base;
        let level = self.fill();
        put(slots, b + ko::quantity, self.quantity);
        put(slots, b + ko::mass, self.quantity);
        put(slots, b + ko::volume, self.quantity / rho);
        put(slots, b + ko::temperature, self.temp);
        put(slots, b + ko::indicated, if self.failed { 0.0 } else { self.indicated });
        flag(slots, b + ko::low, self.quantity < 0.1 * self.capacity);
        flag(slots, b + ko::empty, self.quantity <= self.unusable);
        flag(slots, b + ko::frozen, self.temp < self.fuel.cloud());
        put(slots, b + ko::spilled, self.spilled);
        put(slots, b + ko::level, level);
        put(slots, b + ko::inlet_depth, self.inlet_depth);
        flag(slots, b + ko::inlet_uncovered, self.uncovered());
        put(slots, b + ko::ullage_pressure, self.ullage);
        put(slots, b + ko::head, self.head);
        flag(slots, b + ko::boiling, self.fuel.vapour(self.temp) > self.ullage);
        put(slots, b + ko::free_water, self.free_water);
        put(slots, b + ko::dissolved, self.water * 1e6);
        flag(slots, b + ko::ice, self.free_water > 0.05 && self.temp < 273.15);
        put(slots, b + ko::density, rho);
        put(slots, b + ko::viscosity, self.fuel.kinematic(self.temp) * 1e6);
        put(slots, b + ko::vapour_pressure, self.fuel.vapour(self.temp));
        put(slots, b + ko::surface_tilt, self.tilt[0].hypot(self.tilt[1]));
        overflow
    }

    /// Mass share of free water a pump at the sump pulls in with its fuel.
    pub fn water_carry(&self) -> f64 {
        if self.free_water > 0.5 {
            (self.free_water / (self.quantity + 1.0)).min(0.02)
        } else {
            0.0
        }
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.quantity, self.temp, self.spilled, self.tilt[0], self.tilt[1], self.tilt_rate[0], self.tilt_rate[1], self.ullage, self.water, self.free_water, self.indicated]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.quantity = n();
        self.temp = n();
        self.spilled = n();
        self.tilt = [n(), n()];
        self.tilt_rate = [n(), n()];
        self.ullage = n();
        self.water = n();
        self.free_water = n();
        self.indicated = n();
        self.ullage_set = true;
    }
}
