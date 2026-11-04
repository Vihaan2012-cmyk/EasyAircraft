//! Bleedless air supply and duct humidification: an electrically driven cabin air compressor that takes ram air
//! and feeds a pack in place of engine bleed, with its surge valve and motor load, and a humidifier that sprays
//! water into a zone's supply duct, cooling it as the water evaporates.

use super::thermo::{self, compress, lag, Confirm, Pi, CP, LATENT, P_SL};
use super::{eval, slot_of, yes};
use crate::util::Slots;
use crate::Expr;
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const MF: Type = Type::Num(Dim::MASS_FLOW);
const W: Type = Type::Num(Dim::POWER);
const M: Type = Type::Num(Dim::MASS);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub const COMPRESSOR: PartSpec = PartSpec {
    domain: "aircon",
    kind: "cabin_air_compressor",
    help: "A motor-driven cabin air compressor on ram air, feeding a pack instead of engine bleed: point a pack's duct at it.",
    fields: &[
        req("air", Kind::Ref(&["environment.air"]), "outside air and flight state"),
        opt("demand", Kind::Expr(MF), "1 kg/s", "flow its pack asks for, such as \"pack1.bleed_demand\""),
        opt("flow", Kind::Quantity(Dim::MASS_FLOW), "1.3 kg/s", "corrected flow at full speed"),
        opt("pressure_ratio", Kind::Number, "3.6", "pressure ratio at full speed"),
        opt("delivery", Kind::Quantity(Dim::PRESSURE), "45 psi", "delivery pressure the motor controller aims for"),
        opt("efficiency", Kind::Number, "0.8", "compressor isentropic efficiency"),
        opt("motor_efficiency", Kind::Number, "0.95", "motor and controller efficiency"),
        opt("spool_time", Kind::Quantity(Dim::TIME), "10 s", "time to full speed"),
        opt("on_when", WHEN, "true", "selected on"),
        opt("powered_when", WHEN, "true", "its motor controller has power"),
        opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "480 K", "delivery temperature that trips it"),
        opt("ram_recovery", Kind::Number, "0.8", "share of the dynamic pressure its inlet recovers"),
    ],
    outputs: &[
        ("pressure", P),
        ("temperature", T),
        ("flow", MF),
        ("power", W),
        ("speed", N),
        ("surge_valve", N),
        ("operating", B),
        ("overheat", B),
        ("fault", B),
        ("failed", B),
    ],
};

pub const HUMIDIFIER: PartSpec = PartSpec {
    domain: "aircon",
    kind: "humidifier",
    help: "A duct humidifier for a zone: sprays potable water into its supply air to hold a humidity, backing off when the skin starts to condense.",
    fields: &[
        req("zone", Kind::Ref(&["aircon.zone"]), "the zone whose duct it serves"),
        opt("capacity", Kind::Quantity(Dim::MASS_FLOW), "4 kg/h", "most water it evaporates"),
        opt("target", Kind::Number, "0.2", "relative humidity it aims for"),
        opt("on_when", WHEN, "true", "selected on"),
        opt("water_when", WHEN, "true", "potable water is available"),
        opt("power", Kind::Quantity(Dim::POWER), "150 W", "pump and heater power while it runs"),
    ],
    outputs: &[("running", B), ("water_flow", MF), ("water_used", M), ("power", W), ("fault", B), ("failed", B)],
};

pub struct Compressor {
    air: [usize; 4],
    demand: Option<Expr>,
    rated: f64,
    pr_rated: f64,
    delivery: f64,
    eta: f64,
    motor_eta: f64,
    spool: f64,
    on: Option<Expr>,
    powered: Option<Expr>,
    hot_limit: f64,
    recovery: f64,
    speed: f64,
    hot: Confirm,
    tripped: bool,
    s: Slots,
}

impl Compressor {
    pub fn new(c: &Checked, p: &CheckedPart) -> Option<Self> {
        Some(Compressor {
            air: [slot_of(c, p, "air", "pressure")?, slot_of(c, p, "air", "temperature")?, slot_of(c, p, "air", "mach")?, slot_of(c, p, "air", "airspeed")?],
            demand: crate::expr_of(p, "demand"),
            rated: p.num("flow").max(0.01),
            pr_rated: p.num("pressure_ratio").max(1.1),
            delivery: p.num("delivery").max(1.0),
            eta: p.num("efficiency").clamp(0.3, 0.95),
            motor_eta: p.num("motor_efficiency").clamp(0.3, 1.0),
            spool: p.num("spool_time").max(0.1),
            on: crate::expr_of(p, "on_when"),
            powered: crate::expr_of(p, "powered_when"),
            hot_limit: p.num("overheat_temperature"),
            recovery: p.num("ram_recovery").clamp(0.0, 1.0),
            speed: 0.0,
            hot: Confirm::default(),
            tripped: false,
            s: Slots::of(p, &["pressure", "temperature", "flow", "power", "speed", "surge_valve", "operating", "overheat", "fault", "failed"]),
        })
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let failed = self.s.on(slots, "failed");
        let (pa, ta, mach, tas) = (slots[self.air[0]].max(100.0), slots[self.air[1]].max(150.0), slots[self.air[2]].max(0.0), slots[self.air[3]].max(0.0));
        let selected = yes(&mut self.on, slots, dt, true);
        if !selected {
            self.tripped = false;
        }
        let run = !failed && !self.tripped && selected && yes(&mut self.powered, slots, dt, true);
        let p_in = thermo::ram_pressure(pa, mach, self.recovery);
        let t_in = thermo::recovery_temperature(ta, tas, 1.0);
        // The motor controller sets the speed for the delivery pressure; thin air needs a higher ratio.
        let ratio_needed = (self.delivery / p_in).clamp(1.0, self.pr_rated);
        let target = if run { ((ratio_needed - 1.0) / (self.pr_rated - 1.0)).max(0.0).sqrt().max(0.35) } else { 0.0 };
        self.speed = lag(self.speed, target, self.spool / 3.0, dt);
        let ratio = 1.0 + (self.pr_rated - 1.0) * self.speed * self.speed;
        let t_out = compress(t_in, ratio, self.eta);
        let corrected = (p_in / P_SL) * (288.15 / t_in).sqrt();
        let capacity = self.rated * self.speed * corrected;
        let demand = eval(&mut self.demand, slots, dt, 1.0).max(0.0);
        let delivered = demand.min(capacity);
        // Below half its capacity the surge valve dumps the difference so the compressor stays out of surge.
        let surge_flow = (0.5 * capacity - delivered).max(0.0);
        let surge = if capacity > 1e-6 { surge_flow / capacity } else { 0.0 };
        let power = if run { (delivered + surge_flow) * CP * (t_out - t_in) / self.motor_eta } else { 0.0 };
        let over = self.hot.hold(t_out > self.hot_limit, 2.0, dt);
        if over {
            self.tripped = true;
        }
        let p_out = if self.speed > 0.05 { (p_in * ratio).min(self.delivery * 1.1) } else { pa };
        let s = &self.s;
        s.put(slots, "pressure", p_out);
        s.put(slots, "temperature", if self.speed > 0.05 { t_out } else { ta });
        s.put(slots, "flow", delivered);
        s.put(slots, "power", power);
        s.put(slots, "speed", self.speed);
        s.put(slots, "surge_valve", surge);
        s.flag(slots, "operating", run && self.speed > 0.3);
        s.flag(slots, "overheat", self.tripped);
        s.flag(slots, "fault", self.tripped || (selected && !run));
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.speed, self.hot.elapsed, self.tripped as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.speed = n();
        self.hot.elapsed = n();
        self.tripped = n() != 0.0;
    }
}

pub struct Humidifier {
    pub zone: usize,
    capacity: f64,
    target: f64,
    on: Option<Expr>,
    water: Option<Expr>,
    power: f64,
    ctl: Pi,
    used: f64,
    /// Water it puts into its zone's duct this step, kg/s.
    pub rate: f64,
    s: Slots,
}

impl Humidifier {
    pub fn new(p: &CheckedPart, zone: usize) -> Self {
        Humidifier {
            zone,
            capacity: p.num("capacity").max(0.0),
            target: p.num("target").clamp(0.0, 0.6),
            on: crate::expr_of(p, "on_when"),
            water: crate::expr_of(p, "water_when"),
            power: p.num("power").max(0.0),
            ctl: Pi::new(4.0, 0.02, 0.0, 1.0),
            used: 0.0,
            rate: 0.0,
            s: Slots::of(p, &["running", "water_flow", "water_used", "power", "fault", "failed"]),
        }
    }

    /// `rh` is the zone's humidity now and `condensing` whether its skin is collecting water.
    pub fn step(&mut self, slots: &mut [f64], dt: f64, rh: f64, condensing: bool) {
        let failed = self.s.on(slots, "failed");
        let selected = yes(&mut self.on, slots, dt, true);
        let water = yes(&mut self.water, slots, dt, true);
        let run = !failed && selected && water;
        let demand = if run && !condensing { self.ctl.step(self.target - rh, dt) } else { self.ctl.reset(0.0); 0.0 };
        self.rate = self.capacity * demand;
        self.used += self.rate * dt;
        let s = &self.s;
        s.flag(slots, "running", self.rate > 1e-7);
        s.put(slots, "water_flow", self.rate);
        s.put(slots, "water_used", self.used);
        s.put(slots, "power", if self.rate > 1e-7 { self.power } else { 0.0 });
        s.flag(slots, "fault", selected && !water);
    }

    /// Cool and wet a duct stream by the water evaporated into it.
    pub fn apply(rate: f64, duct: thermo::Stream) -> thermo::Stream {
        if rate <= 0.0 || duct.flow <= 1e-6 {
            return duct;
        }
        let dw = rate / duct.flow;
        let dt_k = rate * LATENT / (duct.flow * CP);
        thermo::Stream { w: duct.w + dw, t: duct.t - dt_k, ..duct }
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.ctl.integral, self.used, self.rate]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.ctl.integral = n();
        self.used = n();
        self.rate = n();
    }
}
