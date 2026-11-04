//! An APU and its controller: inlet door, starter, light-off, starter cut-out and governing; start faults
//! (no light-off, hung start, start over-temperature, outside its start envelope); EGT from load, outside
//! temperature and altitude; oil pressure and temperature; bleed falling with altitude; protective shutdowns
//! with their reason; cool-down after bleed use.

use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const P: Type = Type::Num(Dim::PRESSURE);
const MF: Type = Type::Num(Dim::MASS_FLOW);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub static SPECS: &[PartSpec] = &[PartSpec {
    domain: "apu",
    kind: "unit",
    help: "An auxiliary power unit with its controller: inlet door, starter, light-off, governing, start faults, protective shutdowns, oil, cool-down.",
    fields: &[
        opt("master", WHEN, "false", "APU master switch"),
        opt("start", WHEN, "false", "start button"),
        opt("start_time", Kind::Quantity(Dim::TIME), "45 s", "time from starter engaged to full speed"),
        opt("bleed_load", Kind::Expr(MF), "0 kg/s", "bleed taken"),
        opt("bleed_rating", Kind::Quantity(Dim::MASS_FLOW), "1.2 kg/s", "most bleed at full speed, sea level"),
        opt("generator_load", Kind::Expr(Type::Num(Dim::POWER)), "0 W", "generator load"),
        opt("generator_rating", Kind::Quantity(Dim::POWER), "90 kW", "generator rating"),
        opt("egt_limit", Kind::Quantity(Dim::TEMPERATURE), "950 K", "protective shutdown EGT running"),
        opt("start_egt_limit", Kind::Quantity(Dim::TEMPERATURE), "1100 K", "start aborts above this EGT"),
        opt("shutdown_when", WHEN, "false", "other protective shutdowns: fire, overspeed"),
        opt("fuel_flow", Kind::Quantity(Dim::MASS_FLOW), "0.035 kg/s", "fuel at full load"),
        opt("cooldown", Kind::Quantity(Dim::TIME), "60 s", "run-on after master off with bleed used"),
        opt("air", Kind::Ref(&["environment.air"]), "", "outside air: temperature and pressure"),
        opt("door_time", Kind::Quantity(Dim::TIME), "0 s", "air inlet door opening time before the starter engages"),
        opt("light_off", Kind::Number, "7", "speed for ignition and fuel, percent"),
        opt("starter_cutout", Kind::Number, "55", "speed the starter drops out at, percent"),
        opt("light_off_time", Kind::Quantity(Dim::TIME), "15 s", "no light-off this long after fuel aborts the start"),
        opt("hung_time", Kind::Quantity(Dim::TIME), "0 s", "not at starter cut-out this long after engaging aborts; 0 means start time plus 30 s"),
        opt("starter_when", WHEN, "true", "the starter has its power, such as its battery's voltage"),
        opt("start_ceiling", Kind::Quantity(Dim::LENGTH), "0 ft", "highest pressure altitude it can start at; 0 for no limit"),
        opt("oil_failed_when", WHEN, "false", "oil lost: low oil pressure shuts it down"),
    ],
    outputs: &[
        ("n", N),
        ("egt", T),
        ("starting", B),
        ("running", B),
        ("available", B),
        ("shutdown", B),
        ("fuel_flow", MF),
        ("failed", B),
        ("door", N),
        ("starter", B),
        ("ignition", B),
        ("oil_pressure", P),
        ("oil_temperature", T),
        ("start_fault", B),
        ("reason", N),
        ("bleed_available", N),
    ],
}];

const PSI: f64 = 6894.757;

pub struct Apu {
    e: [Option<Expr>; 8],
    air: [Option<usize>; 2],
    start_time: f64,
    bleed_rating: f64,
    gen_rating: f64,
    egt_limit: f64,
    start_egt: f64,
    fuel: f64,
    cooldown: f64,
    door_time: f64,
    light_off: f64,
    cutout: f64,
    light_time: f64,
    hung_time: f64,
    ceiling: f64,
    n: f64,
    egt: f64,
    oil_t: f64,
    door: f64,
    running: bool,
    lit: bool,
    shutdown: bool,
    reason: f64,
    timer: f64,
    fuel_timer: f64,
    cool: f64,
    s: Slots,
}

fn slot(c: &Checked, p: &CheckedPart, field: &str, out: &str) -> Option<usize> {
    p.part(field).and_then(|i| c.slots.index.get(&format!("{}.{out}", c.parts[i].id)).copied())
}

impl Apu {
    pub fn new(c: &Checked, p: &CheckedPart) -> Self {
        let start_time = p.num("start_time").max(1.0);
        let hung = p.num("hung_time");
        Apu {
            e: ["master", "start", "bleed_load", "generator_load", "shutdown_when", "starter_when", "oil_failed_when", "start"].map(|k| expr_of(p, k)),
            air: [slot(c, p, "air", "temperature"), slot(c, p, "air", "pressure")],
            start_time,
            bleed_rating: p.num("bleed_rating").max(1e-3),
            gen_rating: p.num("generator_rating").max(1.0),
            egt_limit: p.num("egt_limit"),
            start_egt: p.num("start_egt_limit"),
            fuel: p.num("fuel_flow"),
            cooldown: p.num("cooldown"),
            door_time: p.num("door_time").max(0.0),
            light_off: p.num("light_off") / 100.0,
            cutout: p.num("starter_cutout") / 100.0,
            light_time: p.num("light_off_time").max(1.0),
            hung_time: if hung > 0.0 { hung } else { start_time + 30.0 },
            ceiling: p.num("start_ceiling"),
            n: 0.0,
            egt: 288.15,
            oil_t: 288.15,
            door: 0.0,
            running: false,
            lit: false,
            shutdown: false,
            reason: 0.0,
            timer: 0.0,
            fuel_timer: 0.0,
            cool: 0.0,
            s: Slots::of(p, &["n", "egt", "starting", "running", "available", "shutdown", "fuel_flow", "failed", "door", "starter", "ignition", "oil_pressure", "oil_temperature", "start_fault", "reason", "bleed_available"]),
        }
    }

    fn trip(&mut self, why: f64) {
        self.shutdown = true;
        self.running = false;
        self.reason = why;
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let ev = |e: &mut Option<Expr>, s: &[f64]| e.as_mut().map_or(0.0, |x| x.eval(s, dt));
        let failed = self.s.on(slots, "failed");
        let master = ev(&mut self.e[0], slots) != 0.0 && !failed;
        let start = ev(&mut self.e[1], slots) != 0.0;
        let bleed_raw = ev(&mut self.e[2], slots).max(0.0);
        let gen = ev(&mut self.e[3], slots).max(0.0) / self.gen_rating;
        let protect = ev(&mut self.e[4], slots) != 0.0;
        let starter_ok = self.e[5].as_mut().is_none_or(|x| x.yes(slots, dt));
        let oil_lost = ev(&mut self.e[6], slots) != 0.0;
        let t_amb = self.air[0].map_or(288.15, |t| slots[t]).max(150.0);
        let p_amb = self.air[1].map_or(101_325.0, |p| slots[p]).max(1000.0);
        let alt = 44_330.8 * (1.0 - (p_amb / 101_325.0).powf(0.190_263));
        let density = p_amb / 101_325.0;
        let bleed = bleed_raw / (self.bleed_rating * density.max(0.05));
        if master && start && !self.shutdown && !self.running {
            if self.ceiling > 0.0 && alt > self.ceiling {
                self.trip(7.0);
            } else {
                self.running = true;
                self.timer = 0.0;
                self.fuel_timer = 0.0;
                self.lit = false;
            }
        }
        if !master {
            if self.running && bleed > 0.0 {
                self.cool += dt;
            }
            if !(self.running && bleed > 0.0 && self.cool < self.cooldown) {
                self.running = false;
                self.cool = 0.0;
            }
            self.shutdown = false;
        }
        if self.running && (protect || self.egt > self.egt_limit && self.n >= 0.95) {
            self.trip(if protect { 2.0 } else { 1.0 });
        }
        // Inlet door first, then the starter cranks until the core is self-sustaining.
        self.door = if self.running || self.n > 0.0 { (self.door + dt / self.door_time.max(1e-6)).min(1.0) } else { (self.door - dt / self.door_time.max(1e-6)).max(0.0) };
        let door_open = self.door >= 1.0;
        let starter = self.running && door_open && self.n < self.cutout;
        if self.running && door_open && self.n < 0.95 {
            self.timer += dt;
        }
        let rate = 1.0 / self.start_time;
        if self.running && door_open {
            if self.n < self.cutout && !starter_ok {
                self.n = (self.n - 0.05 * dt).max(0.0);
            } else if self.n < 0.95 || self.lit {
                self.n = (self.n + rate * dt).min(1.0);
            }
        } else {
            self.n = (self.n - dt / 20.0).max(0.0);
        }
        if self.running && self.n >= self.light_off {
            self.fuel_timer += dt;
            let rise = self.egt - t_amb;
            if !self.lit && rise > 100.0 {
                self.lit = true;
            }
            if !self.lit && self.fuel_timer > self.light_time {
                self.trip(3.0);
            }
        }
        if self.running && self.n < self.cutout && self.timer > self.hung_time {
            self.trip(4.0);
        }
        let oil_p = if oil_lost { 0.0 } else { 60.0 * PSI * self.n * self.n };
        if self.running && self.n >= 0.95 && oil_p < 25.0 * PSI {
            self.trip(5.0);
        }
        let burning = self.running && self.n >= self.light_off;
        // Start EGT peaks around 40 % on the light-off fuel, then falls back to the governed value, which climbs with load,
        // a hot day and thin air.
        let governed = 680.0 + 120.0 * bleed.min(1.5) + 80.0 * gen.min(1.5) + (t_amb - 288.15) + 60.0 * (1.0 - density);
        let peak = 900.0 + 1.5 * (t_amb - 288.15) + 100.0 * (1.0 - density);
        let goal = if burning {
            if self.n < 0.95 { t_amb + (peak - t_amb) * (self.n / 0.4).min(1.0) - (peak - governed) * ((self.n - 0.4) / 0.55).clamp(0.0, 1.0) } else { governed }
        } else {
            t_amb.max(288.15).min(t_amb)
        };
        self.egt += (goal - self.egt) * (1.0 - (-dt / 3.0).exp());
        if self.running && self.n < 0.95 && self.egt > self.start_egt {
            self.trip(6.0);
        }
        self.oil_t += ((if burning { 360.0 + 30.0 * (bleed + gen).min(2.0) } else { t_amb }) - self.oil_t) * (1.0 - (-dt / 300.0).exp());
        let s = &self.s;
        s.put(slots, "n", self.n * 100.0);
        s.put(slots, "egt", self.egt);
        s.flag(slots, "starting", self.running && self.n < 0.95);
        s.flag(slots, "running", self.running);
        s.flag(slots, "available", self.running && self.n >= 0.95);
        s.flag(slots, "shutdown", self.shutdown);
        s.put(slots, "fuel_flow", if burning { self.fuel * (0.4 + 0.4 * bleed + 0.2 * gen) * self.n.max(0.3) } else { 0.0 });
        s.put(slots, "door", self.door);
        s.flag(slots, "starter", starter);
        s.flag(slots, "ignition", burning && self.n < self.cutout);
        s.put(slots, "oil_pressure", oil_p);
        s.put(slots, "oil_temperature", self.oil_t);
        s.flag(slots, "start_fault", self.shutdown && self.reason >= 3.0);
        s.put(slots, "reason", if self.shutdown { self.reason } else { 0.0 });
        s.put(slots, "bleed_available", if self.running && self.n >= 0.95 { density.min(1.0) } else { 0.0 });
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.n, self.egt, self.oil_t, self.door, self.running as u8 as f64, self.lit as u8 as f64, self.shutdown as u8 as f64, self.reason, self.timer, self.fuel_timer, self.cool]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut next = || it.next().unwrap_or(0.0);
        self.n = next();
        self.egt = next();
        self.oil_t = next();
        self.door = next();
        self.running = next() != 0.0;
        self.lit = next() != 0.0;
        self.shutdown = next() != 0.0;
        self.reason = next();
        self.timer = next();
        self.fuel_timer = next();
        self.cool = next();
    }
}
